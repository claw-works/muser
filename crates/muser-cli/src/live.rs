//! `muser live`：agent 现场演奏。一个常驻进程占着本机声卡，agent 一段一段往后写，音乐不停。
//!
//! - 整场是一个往后长的工程：模板（音轨、效果器、总线）+ 依次提交的"段"。每一段是工程里的一个段落（有自己的小节数、拍号），
//!   片段、自动化的时间相对这一段的开头。生成的工程交给播放器对账（[`muser_engine::Renderer::reconcile`]），不停、不重建。
//! - 不冷场：排好的音乐放完、还没有新的一段时，最后一段整体循环（效果器状态不重置）。新的一段默认在这一遍放完时接上。
//! - 控制接口只在 127.0.0.1：`muser live send|status|wait|autoplay|stop` 与 muser-mcp 的 live 工具都转发到这里。
//! - 整场随时写进 `--record` 的工程文件，离线渲染与现场一致。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use muser_core::{Automation, Beats, Clip, EffectSlot, Project, TimeSignature, Track};
use serde::Deserialize;
use serde_json::{json, Value};

/// 改动最早在当前位置这么多秒之后生效（引擎提前准备、换上新工程）。
const MARGIN_SECONDS: f64 = 1.5;
/// 循环至少往后排这么多秒。
const KEEP_SECONDS: f64 = 20.0;
/// `wait` 单次最多等这么久（免得 agent 的客户端超时）。
const WAIT_MAX: Duration = Duration::from_secs(30);

/// 一次改动（agent 提交的一段）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    /// 这一段叫什么（如 "chorus"、"blues 2"）：录下的工程里段落名带上它，页面上也显示。
    #[serde(default)]
    pub name: Option<String>,
    /// 一句话说明这一段变了什么（"换成 6/8 校园民谣，木吉他分解和弦"）：换段时页面上提示。
    #[serde(default)]
    pub note: Option<String>,
    /// 给整场换个标题（页面最上面，从这一段起）。
    #[serde(default)]
    pub title: Option<String>,
    /// 这一段的歌词原文（页面上卡拉 OK 大字幕显示）。唱的时间来自这一段里第一条音符带 `lyric`（ABC 的 `w:` 行）的轨：
    /// 汉字一个字对一个音节，英文按词；字数和音节数对不上时显示音节本身。
    #[serde(default)]
    pub lyrics: Option<String>,
    /// `"after"`（默认）：排在已经提交的音乐之后（在循环里时：这一遍放完再接）；`"now"`：下一条小节线就换（截断正在放的这段）；
    /// `"patch"`：改正在放的这一段——写的是完整的一段（从它的开头写起），从下一小节起接着这一段的同一位置往下放（曲式不乱）。
    #[serde(default)]
    pub at: At,
    /// 这一段几小节。
    pub bars: u32,
    /// 这一段的速度（BPM）；不写就沿用前一段的。
    #[serde(default)]
    pub tempo: Option<f64>,
    /// 用这一段开头的几小节从前一段的速度渐变过来（渐快、渐慢）；不写或 0 = 直接切换。
    #[serde(default)]
    pub tempo_ramp_bars: Option<u32>,
    /// 这一段的拍号；不写就用模板的。
    #[serde(default)]
    pub time_signature: Option<TimeSignature>,
    #[serde(default)]
    pub tracks: Vec<TrackChange>,
    /// 总线（模板里建好的）：从这一段开头接入 / 撤下效果器、自动化（`gain_db`、`pan`、`effects[i]`）。
    #[serde(default)]
    pub buses: Vec<BusChange>,
    /// （总线和主输出的变化只在这一段第一次放时生效；循环的重复里保持变化后的状态。）
    /// 主输出：效果器（`effects_add` / `effects_off`）与自动化（`master_gain_db`、`master_effects[i]`；不写 `name`）。
    /// 收尾的渐弱：`{ "target": "master_gain_db", "shape": "points", "points": [[0, 0], [16, -40]] }`。
    #[serde(default)]
    pub master: Option<BusChange>,
}

/// 一条总线（或主输出）在这一段里的变化。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusChange {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub effects_add: Vec<EffectSlot>,
    #[serde(default)]
    pub effects_off: Vec<usize>,
    #[serde(default)]
    pub automation: Vec<Automation>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum At {
    #[default]
    After,
    Now,
    Patch,
}

/// 一条轨在这一段里做什么。没写到的轨在这一段里休息。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackChange {
    pub name: String,
    /// 新轨才写：乐器、参数、输出、发送、音量、声像。已有的轨中途不能换乐器（换就新开一条轨），音量、声像用自动化。
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub instrument_params: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub sends: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    pub gain_db: Option<f64>,
    #[serde(default)]
    pub pan: Option<f64>,
    /// 片段，`start` 相对这一段的开头；超出这一段的部分截掉。
    #[serde(default)]
    pub clips: Vec<Clip>,
    /// 从这一段开头接入的效果器（追加在效果链末尾）。
    #[serde(default)]
    pub effects_add: Vec<EffectSlot>,
    /// 从这一段开头撤下的效果器（`effects` 里的序号）。
    #[serde(default)]
    pub effects_off: Vec<usize>,
    /// 自动化，时间相对这一段的开头；从这一段开头起生效（写了 `end` 就到那里为止，否则一直到被后面的覆盖）。
    #[serde(default)]
    pub automation: Vec<Automation>,
}

/// 一段：从第 `start` 拍开始，`bars` 小节，`ts` 拍号。`rep` 是循环出来的重复（不再接入 / 撤下效果器）。
#[derive(Debug, Clone)]
struct Seg {
    start: f64,
    bars: u32,
    ts: TimeSignature,
    change: Arc<Change>,
    rep: bool,
    /// 被后来的 `"now"` 截短了。
    cut: bool,
    /// 从这一段写的内容的第几拍开始放（`"patch"` 从中间接上；平常是 0）。
    offset: f64,
    /// 是 `"patch"` 改出来的：新轨的写法（instrument 等）允许和已有的重复。
    patch: bool,
    /// 这一段的卡拉 OK 字幕（时间是秒，相对这一段写的内容的开头）。
    karaoke: Arc<Vec<muser_engine::lyric::KaraokeLine>>,
}

impl Seg {
    fn len(&self) -> f64 {
        f64::from(self.bars) * self.ts.beats_per_bar()
    }
    fn end(&self) -> f64 {
        self.start + self.len()
    }
    /// 写的这一段完整有多长（循环时一遍的长度）。
    fn full_len(&self) -> f64 {
        f64::from(self.change.bars) * self.ts.beats_per_bar()
    }
}

/// 一次调音量：从第 `beat` 拍起，`ramp` 拍内从 `from` 渐变到 `to`（dB）。`target` 是轨名、总线名，或 "master"。
#[derive(Debug, Clone)]
struct Mix {
    target: String,
    beat: f64,
    ramp: f64,
    from: f64,
    to: f64,
}

impl Mix {
    fn automation(&self, master: bool) -> Automation {
        serde_json::from_value(json!({ "target": if master { "master_gain_db" } else { "gain_db" }, "shape": "points",
            "points": [[self.beat, self.from], [self.beat + self.ramp, self.to]], "start": self.beat })).expect("合法的自动化")
    }
}

/// 提交的回执。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Receipt {
    /// 生效在第几小节的第几拍（都从 1 起；段总是从小节线开始，所以拍是 1）。
    pub bar: usize,
    pub beat_in_bar: f64,
    /// 从整场开头算第几拍（四分音符为一拍，从 0 起）。
    pub absolute_beat: f64,
    /// 离生效还有几秒。
    pub starts_in_seconds: f64,
    /// 已经排到第几小节（这一段的最后一小节）。
    pub queued_until_bar: usize,
    /// 从这一段写的第几小节起放（`"patch"` 接在中间时大于 1）。
    pub section_bar: usize,
}

/// 一场演奏的音乐部分：模板 + 依次的段。
pub struct Session {
    base: Project,
    segs: Vec<Seg>,
    /// 演奏中调的音量（`muser live mix`），按时间写进对应那一段之后。
    mixes: Vec<Mix>,
    /// 整场的标题（`--title`，或某一段的 `title`）。
    title: String,
}

impl Session {
    /// 从模板开始：模板里的片段（编曲层展开后）成为第一段，放完就循环，直到 agent 写下一段。
    pub fn new(template: &Project) -> Result<Session> {
        template.validate().map_err(|e| anyhow!("{e}"))?;
        let mut names = std::collections::HashSet::new();
        for t in &template.tracks {
            if t.name.trim().is_empty() || !names.insert(t.name.as_str()) {
                bail!("模板里每条音轨都要有名字，且不重名（现场改动按名字找轨）");
            }
            if t.instrument.trim() == muser_core::audio::INSTRUMENT {
                bail!("现场演奏不支持音频轨（'{}'）", t.name);
            }
        }
        let arranged = muser_engine::arrange::arranged(template).map_err(|e| anyhow!(e))?.into_owned();
        let ts = template.time_signature;
        let bpb = ts.beats_per_bar();
        let bars = ((arranged.duration_beats().0 / bpb) - 1e-9).ceil().max(1.0) as u32;
        let first = Change {
            name: Some("template".into()),
            tempo: None,
            tempo_ramp_bars: None,
            note: None,
            title: None,
            lyrics: None,
            buses: Vec::new(),
            master: None,
            at: At::After,
            bars,
            time_signature: Some(ts),
            tracks: arranged
                .tracks
                .iter()
                .filter(|t| !t.clips.is_empty())
                .map(|t| TrackChange { name: t.name.clone(), clips: t.clips.clone(), ..empty(&t.name) })
                .collect(),
        };
        let mut base = arranged;
        for t in &mut base.tracks {
            t.clips.clear();
            t.parts.clear();
        }
        base.form.clear();
        base.sections.clear();
        let title = if template.name.is_empty() { "muser live".to_string() } else { template.name.clone() };
        let s = Session { base, title, mixes: Vec::new(), segs: vec![Seg { start: 0.0, bars, ts, change: Arc::new(first), rep: false, cut: false, offset: 0.0, patch: false, karaoke: Default::default() }] };
        s.project()?;
        Ok(s)
    }

    pub fn set_title(&mut self, title: &str) {
        if !title.trim().is_empty() {
            self.title = title.trim().to_string();
        }
    }

    /// 整场的速度表：和生成的工程一致（写了 `tempo` 的段从段首起换，渐变不超过这一段）。
    pub fn tm(&self) -> muser_core::tempo::TempoMap {
        let mut m = muser_core::tempo::TempoMap::constant(self.base.tempo);
        for s in &self.segs {
            if let Some(bpm) = s.change.tempo {
                m.change(s.start, bpm, f64::from(s.change.tempo_ramp_bars.unwrap_or(0).min(s.bars)) * s.ts.beats_per_bar());
            }
        }
        m
    }

    /// 从第 `beat` 拍起再过 `secs` 秒是第几拍。
    pub fn beat_after(&self, beat: f64, secs: f64) -> f64 {
        let m = self.tm();
        m.beat_at(m.secs_at(beat) + secs)
    }

    fn last_real(&self) -> &Seg {
        self.segs.iter().rev().find(|s| !s.rep).expect("至少有模板那一段")
    }

    /// 已经提交的音乐排到第几拍（之后是循环）。
    fn real_end(&self) -> f64 {
        self.last_real().end()
    }

    /// 让循环至少排到第 `until` 拍。有新加的就返回真。
    pub fn extend(&mut self, until: f64) -> bool {
        let src = self.last_real().clone();
        let mut changed = false;
        while self.segs.last().is_some_and(|l| l.end() < until) {
            let start = self.segs.last().unwrap().end();
            // 循环从这一段的开头完整地重复（就算它是从中间 patch 上来的）。
            self.segs.push(Seg { start, rep: true, cut: false, offset: 0.0, patch: false, bars: src.change.bars, ..src.clone() });
            changed = true;
        }
        changed
    }

    /// 提交一段。`now` 是现在演奏到的拍。出错时什么也不改。
    pub fn submit(&mut self, change: Change, now: f64) -> Result<Receipt> {
        if change.bars == 0 || change.bars > 256 {
            bail!("bars 应在 1 到 256 之间");
        }
        let earliest = self.beat_after(now, MARGIN_SECONDS);
        let before = self.segs.clone();
        let mut patch_from: Option<u32> = None;
        let at = match change.at {
            At::After => {
                // 排在已经提交的之后；那里已经过了（在循环里）就等这一遍循环放完。
                let mut b = self.real_end();
                let l = self.last_real().full_len();
                while b < earliest {
                    b += l;
                }
                b
            }
            At::Now | At::Patch => {
                self.extend(earliest + 1.0);
                let s = self.segs.iter().find(|s| s.end() > earliest).expect("刚排过").clone();
                let bpb = s.ts.beats_per_bar();
                let k = ((earliest - s.start) / bpb - 1e-9).ceil().max(0.0) as u32;
                if change.at == At::Patch {
                    // 接着这一段的同一位置：这一段的第几小节（算上它自己从中间开始的部分）。
                    let into = (s.offset / bpb).round() as u32 + k;
                    if into >= change.bars {
                        self.segs = before;
                        bail!("这一段已经放到第 {} 小节，改写的只有 {} 小节：用 \"after\" 排在后面", into + 1, change.bars);
                    }
                    patch_from = Some(into);
                }
                s.start + f64::from(k.min(s.bars)) * bpb
            }
        };
        self.extend(at);
        // 排着还没放的段里接入了效果器：效果链只能往后加，去不掉。
        let adds = |c: &Change| c.tracks.iter().any(|t| !t.effects_add.is_empty()) || c.buses.iter().any(|b| !b.effects_add.is_empty()) || c.master.as_ref().is_some_and(|m| !m.effects_add.is_empty());
        if self.segs.iter().any(|s| s.start >= at - 1e-9 && !s.rep && adds(&s.change)) {
            self.segs = before;
            bail!("后面排着的段接入了效果器，现在截不掉：等它开始放以后再用 \"now\"，或者用 \"after\"");
        }
        // 截在 `at`：之后的（还没演奏的循环）去掉，跨过 `at` 的那段只留前面整小节。
        self.segs.retain(|s| s.start < at - 1e-9);
        if let Some(last) = self.segs.last_mut() {
            if last.end() > at + 1e-9 {
                last.bars = ((at - last.start) / last.ts.beats_per_bar()).round() as u32;
                last.cut = true;
            }
        }
        if let Some(t) = change.title.as_ref().filter(|t| !t.trim().is_empty()) {
            self.title = t.trim().to_string();
        }
        let ts = change.time_signature.unwrap_or(self.base.time_signature);
        let skip = patch_from.unwrap_or(0);
        let offset = f64::from(skip) * ts.beats_per_bar();
        let karaoke = Arc::new(karaoke_of(&change, &self.base, ts));
        self.segs.push(Seg { start: at, bars: change.bars - skip, ts, change: Arc::new(change), rep: false, cut: false, offset, patch: patch_from.is_some(), karaoke });
        match self.project() {
            Ok(p) => {
                let map = p.bar_map();
                let tm = self.tm();
                let (bar, into) = map.bar_at(at);
                Ok(Receipt {
                    section_bar: skip as usize + 1,
                    bar: bar + 1,
                    beat_in_bar: (into * 1000.0).round() / 1000.0 + 1.0,
                    absolute_beat: at,
                    starts_in_seconds: (tm.secs_at(at) - tm.secs_at(now)).max(0.0),
                    queued_until_bar: map.bar_at(self.real_end()).0,
                })
            }
            Err(e) => {
                self.segs = before;
                Err(e)
            }
        }
    }

    /// 生成整场的工程（到目前排好的为止）。
    pub fn project(&self) -> Result<Project> {
        self.project_until(f64::INFINITY)
    }

    /// 只到第 `until` 拍前开始的段（录下来时用：还没演奏的循环不要）。
    pub fn project_until(&self, until: f64) -> Result<Project> {
        let mut p = self.base.clone();
        let mut sections = BTreeMap::new();
        for (k, seg) in self.segs.iter().enumerate().filter(|(_, s)| s.start < until || s.start == 0.0) {
            // 段落名：序号 + agent 起的名字，循环出来的重复、被截短的注明。
            let mut name = format!("{:03}", k + 1);
            if let Some(n) = &seg.change.name {
                name = format!("{name} {n}");
            }
            if seg.rep {
                name.push_str(" (repeat)");
            }
            if seg.cut {
                name.push_str(" (cut)");
            }
            if seg.patch {
                name.push_str(&format!(" (patch from bar {})", (seg.offset / seg.ts.beats_per_bar()).round() as u32 + 1));
            }
            sections.insert(name.clone(), muser_core::arrange::Section { bars: seg.bars, time_signature: Some(seg.ts), chords: None, tempo: seg.change.tempo, tempo_ramp_bars: seg.change.tempo_ramp_bars });
            p.form.push(muser_core::arrange::FormEntry::Name(name));
            apply(&mut p, seg).with_context(|| format!("第 {} 段", k + 1))?;
            // 这一段里调过的音量：写在这一段之后（比它晚的段写的自动化会覆盖它）。
            let end = self.segs.get(k + 1).map_or(f64::INFINITY, |n| n.start);
            for m in self.mixes.iter().filter(|m| m.beat >= seg.start - 1e-9 && m.beat < end && (m.beat < until || seg.start == 0.0)) {
                if m.target == "master" {
                    p.automation.push(m.automation(true));
                } else if let Some(t) = p.tracks.iter_mut().find(|t| t.name == m.target) {
                    t.automation.push(m.automation(false));
                } else if let Some(b) = p.buses.iter_mut().find(|b| b.name == m.target) {
                    b.automation.push(m.automation(false));
                }
            }
        }
        p.sections = sections;
        muser_engine::validate(&p).map_err(|e| anyhow!("{e}"))?;
        Ok(p)
    }

    /// 第 `beat` 拍时某条轨 / 总线 / 主输出的推子值（dB，算上自动化）。
    fn fader_at(p: &Project, target: &str, beat: f64) -> Option<f64> {
        let total = p.duration_beats().0 + 64.0;
        if target == "master" {
            let l = muser_engine::automate::compile_master(p, total).ok()?;
            return Some(muser_engine::automate::scalar(&l.gain, beat, p.master_gain_db));
        }
        if let Some(i) = p.tracks.iter().position(|t| t.name == target) {
            let l = muser_engine::automate::compile_track(p, i, total).ok()?;
            return Some(muser_engine::automate::scalar(&l.gain, beat, p.tracks[i].gain_db));
        }
        let i = p.buses.iter().position(|b| b.name == target)?;
        let l = muser_engine::automate::compile_bus(p, i, total).ok()?;
        Some(muser_engine::automate::scalar(&l.gain, beat, p.buses[i].gain_db))
    }

    /// 调音量：从第 `at` 拍起 `ramp` 拍内渐变到 `db`。返回原来的值。
    pub fn mix(&mut self, target: &str, db: f64, at: f64, ramp: f64) -> Result<f64> {
        if !(-60.0..=12.0).contains(&db) {
            bail!("音量在 -60 到 +12 dB 之间");
        }
        let p = self.project()?;
        let from = Self::fader_at(&p, target, at).ok_or_else(|| {
            anyhow!("没有叫 '{target}' 的轨或总线（主输出写 master）；有：{}", p.tracks.iter().map(|t| t.name.as_str()).chain(p.buses.iter().map(|b| b.name.as_str())).collect::<Vec<_>>().join("、"))
        })?;
        self.mixes.push(Mix { target: target.to_string(), beat: at, ramp, from, to: db });
        if let Err(e) = self.project() {
            self.mixes.pop();
            return Err(e);
        }
        Ok(from)
    }

    /// 每条轨、总线、主输出现在的推子值（dB）。
    pub fn faders(&self, now: f64) -> Vec<(String, f64)> {
        let Ok(p) = self.project() else { return Vec::new() };
        let names = p.tracks.iter().map(|t| t.name.clone()).chain(p.buses.iter().map(|b| b.name.clone())).chain(std::iter::once("master".to_string()));
        names.filter_map(|n| Self::fader_at(&p, &n, now).map(|v| (n, (v * 10.0).round() / 10.0))).collect()
    }

    /// 给 agent 看的现状：现在第几小节、排到哪、是否在循环、每条轨、上一段写了什么。
    pub fn status(&self, now: f64) -> Value {
        let p = match self.project() {
            Ok(p) => p,
            Err(e) => return json!({ "error": format!("{e:#}") }),
        };
        let map = p.bar_map();
        let tm = self.tm();
        let secs_until = |b: f64| (tm.secs_at(b) - tm.secs_at(now)).max(0.0);
        let real_end = self.real_end();
        let cur = self.segs.iter().position(|s| s.end() > now).unwrap_or(self.segs.len() - 1);
        let seg = &self.segs[cur];
        let last = self.last_real();
        let looping = now >= real_end;
        let reps = self.segs.iter().filter(|s| s.rep && s.start >= real_end - 1e-9 && s.start <= now).count();
        let (bar_now, into) = map.bar_at(now);
        json!({
            "bar": bar_now + 1,
            "beat_in_bar": (into * 100.0).floor() / 100.0 + 1.0,
            "absolute_beat": now,
            "tempo": (tm.bpm_at(now) * 10.0).round() / 10.0,
            "time_signature": [seg.ts.numerator, seg.ts.denominator],
            "title": self.title,
            "next": self.segs.iter().skip(cur + 1).find(|s| !s.rep).map(|s| json!({ "name": s.change.name, "note": s.change.note,
                "bar": map.bar_at(s.start).0 + 1, "starts_in_seconds": secs_until(s.start) })),
            "segment": { "index": cur + 1, "name": seg.change.name, "note": seg.change.note, "start_bar": map.bar_at(seg.start).0 + 1, "bars": seg.bars, "repeat": seg.rep,
                         "seconds_to_end": secs_until(seg.end()) },
            "queued_until_bar": map.bar_at(real_end).0,
            "runway_seconds": secs_until(real_end),
            "looping": looping,
            "loop": { "start_bar": map.bar_at(last.start).0 + 1, "bars": last.bars, "repeats_played": if looping { reps } else { 0 } },
            "tracks": p.tracks.iter().map(|t| json!({ "name": t.name, "instrument": t.instrument,
                "effects": t.effects.iter().map(|e| json!({ "type": e.type_name(), "from": e.from, "until": e.until })).collect::<Vec<_>>(),
                "output": t.output, "playing_now": seg.change.tracks.iter().any(|c| c.name == t.name && !c.clips.is_empty()) })).collect::<Vec<_>>(),
            "buses": p.buses.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
            "last_change": last.change.as_ref().clone().to_json(),
        })
    }
}

impl Session {
    /// 给页面画时间线：每一段（秒、小节、拍号、是不是循环出来的、哪些轨在演奏）；`now`（拍）附近的段带上卡拉 OK 字幕（秒，整场的时间）。
    pub fn timeline(&self, now: f64) -> Value {
        let tm = self.tm();
        let sec = |b: f64| tm.secs_at(b);
        let now_s = sec(now);
        let mut bar = 1u32;
        let segs: Vec<Value> = self
            .segs
            .iter()
            .map(|s| {
                let near = sec(s.end()) > now_s - 10.0 && sec(s.start) < now_s + 60.0;
                // 字幕的时间是相对这一段内容开头的拍数（karaoke_of 按 60 BPM 算），按整场的速度表换成秒。
                let base = s.start - s.offset;
                let at = |b: f64| sec(base + b);
                let karaoke: Vec<Value> = if near {
                    s.karaoke
                        .iter()
                        .filter(|l| base + l.end > s.start && base + l.start < s.end())
                        .map(|l| json!({ "start": at(l.start), "end": at(l.end),
                            "tokens": l.tokens.iter().map(|t| json!({ "text": t.text, "start": at(t.start), "end": at(t.end) })).collect::<Vec<_>>() }))
                        .collect()
                } else {
                    Vec::new()
                };
                let v = json!({ "start": sec(s.start), "end": sec(s.end()), "bar": bar, "bars": s.bars, "name": s.change.name, "karaoke": karaoke,
                    "meter": [s.ts.numerator, s.ts.denominator], "repeat": s.rep,
                    "tracks": s.change.tracks.iter().filter(|t| !t.clips.is_empty()).map(|t| t.name.clone()).collect::<Vec<_>>() });
                bar += s.bars;
                v
            })
            .collect();
        json!({ "segments": segs, "queued_until": sec(self.real_end()) })
    }
}

impl Change {
    /// 写回 JSON（给 status：agent 接着写下一段时参考）。
    fn to_json(&self) -> Value {
        let bus = |b: &BusChange| json!({ "name": b.name, "effects_add": b.effects_add, "effects_off": b.effects_off, "automation": b.automation });
        json!({
            "name": self.name,
            "note": self.note,
            "buses": self.buses.iter().map(bus).collect::<Vec<_>>(),
            "master": self.master.as_ref().map(bus),
            "bars": self.bars,
            "time_signature": self.time_signature,
            "tracks": self.tracks.iter().map(|t| {
                let mut o = json!({ "name": t.name, "clips": t.clips });
                if !t.automation.is_empty() { o["automation"] = json!(t.automation); }
                if !t.effects_add.is_empty() { o["effects_add"] = json!(t.effects_add); }
                o
            }).collect::<Vec<_>>(),
        })
    }
}

fn empty(name: &str) -> TrackChange {
    TrackChange {
        name: name.to_string(),
        instrument: None,
        instrument_params: None,
        output: None,
        sends: None,
        gain_db: None,
        pan: None,
        clips: Vec::new(),
        effects_add: Vec::new(),
        effects_off: Vec::new(),
        automation: Vec::new(),
    }
}

/// 一段的卡拉 OK 字幕：只用这一段写的内容算（时间是相对它开头的拍数：按 60 BPM 算，秒 = 拍）。乐器：新轨写的、模板里的，否则随便一个音高乐器（只为展开音符）。
fn karaoke_of(change: &Change, base: &Project, ts: TimeSignature) -> Vec<muser_engine::lyric::KaraokeLine> {
    let tracks: Vec<Value> = change
        .tracks
        .iter()
        .filter(|t| !t.clips.is_empty())
        .map(|t| {
            let inst = t.instrument.clone().or_else(|| base.tracks.iter().find(|b| b.name == t.name).map(|b| b.instrument.clone())).unwrap_or_else(|| "synth.poly".into());
            json!({ "name": t.name, "instrument": inst, "clips": t.clips })
        })
        .collect();
    let tmp = json!({ "tempo": 60.0, "time_signature": ts, "lyrics": change.lyrics, "tracks": tracks });
    serde_json::from_value::<Project>(tmp).map(|p| muser_engine::lyric::karaoke(&p)).unwrap_or_default()
}

/// 把一段写进工程：片段、自动化平移到这一段的开头；新轨在这里建；效果器从这里接入 / 撤下。
fn apply(p: &mut Project, seg: &Seg) -> Result<()> {
    // `off`：这一段写的内容从哪一拍算起（patch 从中间接上时比 seg.start 早）；只放 [lo, hi) 这一截。
    let (off, len) = (seg.start - seg.offset, seg.len());
    let (lo, hi) = (seg.offset, seg.offset + len);
    let tempo = p.tempo;
    for tc in &seg.change.tracks {
        let i = match p.tracks.iter().position(|t| t.name == tc.name) {
            // 循环出来的重复、patch 改写：新轨那一遍已经建好了。
            Some(i) if seg.rep || seg.patch => i,
            Some(i) => {
                if tc.instrument.is_some() && tc.instrument.as_deref() != Some(p.tracks[i].instrument.as_str()) {
                    bail!("音轨 '{}' 已经有乐器（{}）：中途不能换，换乐器就新开一条轨", tc.name, p.tracks[i].instrument);
                }
                if tc.instrument_params.is_some() || tc.output.is_some() || tc.sends.is_some() || tc.gain_db.is_some() || tc.pan.is_some() {
                    bail!("音轨 '{}' 已经有了：instrument_params / output / sends / gain_db / pan 只在新轨上写，中途改用 automation", tc.name);
                }
                i
            }
            None => {
                let instrument = tc.instrument.clone().ok_or_else(|| anyhow!("没有叫 '{}' 的轨：新轨要写 instrument", tc.name))?;
                let mut t: Track = serde_json::from_value(json!({ "name": tc.name, "instrument": instrument, "clips": [] }))?;
                if let Some(x) = &tc.instrument_params {
                    t.instrument_params = serde_json::from_value(Value::Object(x.clone()))?;
                }
                t.output = tc.output.clone();
                t.sends = tc.sends.clone().unwrap_or_default();
                t.gain_db = tc.gain_db.unwrap_or(0.0);
                t.pan = tc.pan.unwrap_or(0.0);
                p.tracks.push(t);
                p.tracks.len() - 1
            }
        };
        let t = &mut p.tracks[i];
        for c in &tc.clips {
            let (cs, ce) = (c.start.0, c.start.0 + c.length.0);
            if cs < 0.0 || cs >= hi - 1e-9 || ce <= lo + 1e-9 {
                continue;
            }
            let mut c = c.clone();
            if cs < lo - 1e-9 {
                // 跨过接上的地方：展开成音符，只留之后起音的（之前的已经在放的那一段里了）。
                c = clip_from(&c, lo - cs, &t.instrument, tempo)?;
            }
            c.length = Beats(c.length.0.min(hi - c.start.0));
            c.start = Beats(c.start.0 + off);
            t.clips.push(c);
        }
        if !seg.rep {
            let begin = seg.start;
            for &k in &tc.effects_off {
                let e = t.effects.get_mut(k).ok_or_else(|| anyhow!("音轨 '{}' 没有 effects[{k}]", tc.name))?;
                e.until = Some(Beats(e.until.map_or(begin, |u| u.0.min(begin))));
            }
            for e in &tc.effects_add {
                let mut e = e.clone();
                e.from = Some(Beats((off + e.from.map_or(0.0, |b| b.0)).max(begin)));
                e.until = e.until.map(|b| Beats(off + b.0));
                t.effects.push(e);
            }
        }
        for a in &tc.automation {
            t.automation.push(shift_from(a, off, seg.start));
        }
    }
    for bc in &seg.change.buses {
        let b = p.buses.iter_mut().find(|b| b.name == bc.name).ok_or_else(|| anyhow!("没有叫 '{}' 的总线（总线要先在模板里建好）", bc.name))?;
        // 总线、主输出的变化只在这一段第一次出现时：渐弱以后循环停在弱的那里，不会每遍重来。
        if !seg.rep {
            effects_change(&mut b.effects, bc, seg.start, &format!("总线 '{}'", bc.name))?;
            b.automation.extend(bc.automation.iter().map(|a| shift_from(a, off, seg.start)));
        }
    }
    if let Some(m) = &seg.change.master {
        if !seg.rep {
            effects_change(&mut p.master_effects, m, seg.start, "主输出")?;
            p.automation.extend(m.automation.iter().map(|a| shift_from(a, off, seg.start)));
        }
    }
    Ok(())
}

/// 同 [`shift`]，但最早从第 `begin` 拍起生效（patch 从中间接上时，之前的部分不算）。
fn shift_from(a: &Automation, off: f64, begin: f64) -> Automation {
    let mut a = shift(a, off);
    a.start = a.start.map(|s| Beats(s.0.max(begin)));
    a
}

/// 片段从它的第 `skip` 拍开始的那一截：展开成音符，只留之后起音的，时间往前挪 `skip`。
fn clip_from(c: &Clip, skip: f64, instrument: &str, tempo: f64) -> Result<Clip> {
    let tmp: Project = serde_json::from_value(json!({ "tempo": tempo, "tracks": [{ "name": "x", "instrument": instrument, "clips": [c] }] }))?;
    let notes = muser_engine::clip_notes(&tmp, 0, 0).map_err(|e| anyhow!(e))?;
    let mut out = c.clone();
    out.abc = None;
    out.grid = None;
    out.drummer = None;
    out.notes = notes
        .into_iter()
        .filter(|n| n.start.0 >= skip - 1e-9)
        .map(|mut n| {
            n.start = Beats(n.start.0 - skip);
            n
        })
        .collect();
    out.pedal = c.pedal.iter().filter(|(_, e)| *e > skip).map(|(a, e)| ((a - skip).max(0.0), e - skip)).collect();
    out.start = Beats(c.start.0 + skip);
    out.length = Beats(c.length.0 - skip);
    Ok(out)
}

/// 自动化平移到这一段的开头，从这一段开头起生效。
fn shift(a: &Automation, off: f64) -> Automation {
    let mut a = a.clone();
    if let muser_core::automation::Shape::Points { points, .. } = &mut a.shape {
        for pt in points.iter_mut() {
            pt.0 += off;
        }
    }
    a.start = Some(Beats(off + a.start.map_or(0.0, |b| b.0)));
    a.end = a.end.map(|b| Beats(off + b.0));
    a
}

/// 总线 / 主输出的效果链：从 `off` 拍起撤下、接入。
fn effects_change(effects: &mut Vec<EffectSlot>, bc: &BusChange, off: f64, what: &str) -> Result<()> {
    for &k in &bc.effects_off {
        let e = effects.get_mut(k).ok_or_else(|| anyhow!("{what} 没有 effects[{k}]"))?;
        e.until = Some(Beats(e.until.map_or(off, |u| u.0.min(off))));
    }
    for e in &bc.effects_add {
        let mut e = e.clone();
        e.from = Some(Beats(off + e.from.map_or(0.0, |b| b.0)));
        e.until = e.until.map(|b| Beats(off + b.0));
        effects.push(e);
    }
    Ok(())
}

// ---------- 常驻进程 ----------

struct Live {
    session: Mutex<Session>,
    player: muser_engine::playback::Player,
    record: Option<PathBuf>,
    /// 自主演奏到什么时候（之后 `wait` 告诉 agent 停下）。
    autoplay_until: Mutex<Option<Instant>>,
    /// 上次 `wait` 返回的时刻，与那之后到下一次提交用了几秒。
    last_wait: Mutex<Option<Instant>>,
    last_gap: Mutex<Option<f64>>,
    started: Instant,
    /// 最近一次掉帧（播放器的欠载数涨了）是什么时候，与当时的欠载数。
    last_underrun: Mutex<(Option<Instant>, usize)>,
    /// 已经 `stop`：声音停了，再留几秒让页面看到"已结束"。
    stopped: std::sync::atomic::AtomicBool,
    /// agent 上次看状态时的欠载数（之后又多了就提醒，不管隔了多久）。
    underruns_seen: Mutex<usize>,
}

/// 掉帧后这么多秒内，status / wait 都提醒。
const UNDERRUN_WARN: Duration = Duration::from_secs(30);

impl Live {
    fn now_beat(&self) -> f64 {
        self.session.lock().unwrap().tm().beat_at(self.player.status().position)
    }

    /// 把整场写进 `--record`（只到现在为止开始的段）。
    fn save(&self, now: f64) {
        let Some(path) = &self.record else { return };
        let r = self.session.lock().unwrap().project_until(now.max(1e-9)).map_err(|e| e.to_string()).and_then(|p| {
            let text = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
            std::fs::write(path, text + "\n").map_err(|e| e.to_string())
        });
        if let Err(e) = r {
            eprintln!("[live] 写录音工程失败: {e}");
        }
    }

    fn push(&self) -> Result<()> {
        let p = self.session.lock().unwrap().project()?;
        self.player.load(p);
        self.player.play();
        Ok(())
    }

    fn autoplay_left(&self) -> Option<f64> {
        self.autoplay_until.lock().unwrap().map(|t| t.saturating_duration_since(Instant::now()).as_secs_f64())
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::Acquire)
    }

    /// 播放器的欠载数涨了就记下时刻（后台线程每 250 毫秒看一次）。
    fn watch_underruns(&self) {
        let n = self.player.status().underruns;
        let mut u = self.last_underrun.lock().unwrap();
        if n > u.1 {
            *u = (Some(Instant::now()), n);
        }
    }

    /// 每次提交写一行：什么时候、生效在哪、原文（复盘用）。
    fn log_change(&self, body: &Value, receipt: &Receipt) {
        let Some(path) = &self.record else { return };
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let stem = name.strip_suffix(".json").unwrap_or(&name);
        let stem = stem.strip_suffix(".muser").unwrap_or(stem);
        let log = path.with_file_name(format!("{stem}.changes.jsonl"));
        let line = json!({ "elapsed_seconds": self.started.elapsed().as_secs_f64(), "receipt": receipt, "change": body }).to_string();
        let r = std::fs::OpenOptions::new().create(true).append(true).open(&log).and_then(|mut f| std::io::Write::write_all(&mut f, (line + "\n").as_bytes()));
        if let Err(e) = r {
            eprintln!("[live] 写 {} 失败: {e}", log.display());
        }
    }

    /// 调音量：约 0.4 秒后开始（渲染领先播放的那一段已经算好了），0.3 秒内渐变到 `db`。
    fn mix(&self, body: &Value) -> Result<Value> {
        if self.is_stopped() {
            bail!("演奏已经结束了");
        }
        let target = body["track"].as_str().ok_or_else(|| anyhow!("要 track（轨名、总线名，或 master）"))?;
        let db = body["gain_db"].as_f64().ok_or_else(|| anyhow!("要 gain_db（推子的目标值，dB）"))?;
        let now = self.now_beat();
        let (at, ramp) = {
            let s = self.session.lock().unwrap();
            let at = s.beat_after(now, 0.4);
            (at, s.beat_after(at, 0.3) - at)
        };
        let from = self.session.lock().unwrap().mix(target, db, at, ramp)?;
        self.push()?;
        self.save(now);
        Ok(json!({ "ok": true, "track": target, "from_db": (from * 10.0).round() / 10.0, "to_db": db, "starts_in_seconds": 0.4 }))
    }

    /// 给 agent 的状态：上次看过以后又掉过帧就提醒（页面的轮询不算"看过"）。
    fn agent_status(&self) -> Value {
        let mut v = self.status();
        // 混音：每个节点的推子值与现在的响度（RMS dBFS，约 1.5 秒），agent 自己看谁太小、太大。
        let now = self.now_beat();
        let faders: std::collections::HashMap<String, f64> = self.session.lock().unwrap().faders(now).into_iter().collect();
        let loud: std::collections::HashMap<String, Option<f64>> = self.player.loudness().into_iter().map(|(n, l)| (if n.is_empty() { "master".into() } else { n }, l)).collect();
        let mut mix: Vec<Value> = faders
            .iter()
            .map(|(n, g)| json!({ "name": n, "gain_db": g, "rms_db": loud.get(n).copied().flatten().map(|x| (x * 10.0).round() / 10.0) }))
            .filter(|m| !m["rms_db"].is_null() || m["name"] == "master")
            .collect();
        mix.sort_by(|a, b| b["rms_db"].as_f64().unwrap_or(-200.0).total_cmp(&a["rms_db"].as_f64().unwrap_or(-200.0)));
        v["mix"] = json!(mix);
        let n = v["underruns"].as_u64().unwrap_or(0) as usize;
        let mut seen = self.underruns_seen.lock().unwrap();
        if n > *seen && v["warning"].is_null() {
            v["warning"] = json!(format!("上次看过以后掉了 {} 次帧：看 slow（慢在哪）和 load.heaviest（最耗资源的轨）", n - *seen));
        }
        *seen = n;
        v
    }

    fn status(&self) -> Value {
        let now = self.now_beat();
        let mut v = self.session.lock().unwrap().status(now);
        let st = self.player.status();
        v["stopped"] = json!(self.is_stopped());
        v["record"] = json!(self.record.as_ref().map(|p| p.display().to_string()));
        v["underruns"] = json!(st.underruns);
        let slow = self.player.slow_events();
        v["slow"] = json!(slow.iter().rev().take(5).collect::<Vec<_>>());
        let loads = self.player.loads();
        v["load"] = json!({ "total": loads.iter().map(|l| l.1).sum::<f32>(),
            "heaviest": loads.iter().filter(|l| !l.0.is_empty()).take(3).map(|(n, l)| json!({ "name": n, "cores": (f64::from(*l) * 100.0).round() / 100.0 })).collect::<Vec<_>>() });
        let recent = self.last_underrun.lock().unwrap().0.is_some_and(|t| t.elapsed() < UNDERRUN_WARN);
        if recent {
            let names: Vec<&str> = loads.iter().filter(|l| !l.0.is_empty()).take(3).map(|l| l.0.as_str()).collect();
            v["warning"] = json!(format!(
                "最近 {} 秒内掉过帧（电脑跟不上）：最耗资源的是 {}。让不用的轨休息（下一段不给它片段）、换轻一点的乐器，或少开几条轨",
                UNDERRUN_WARN.as_secs(), names.join("、")
            ));
        }
        v["error"] = json!(st.error);
        v["autoplay_seconds_left"] = json!(self.autoplay_left());
        v["last_write_seconds"] = json!(*self.last_gap.lock().unwrap());
        v["elapsed_seconds"] = json!(self.started.elapsed().as_secs_f64());
        v
    }

    fn change(&self, body: Value) -> Result<Value> {
        if self.is_stopped() {
            bail!("演奏已经结束了");
        }
        let change: Change = serde_json::from_value(body.clone()).context("改动的写法不对")?;
        let now = self.now_beat();
        let receipt = self.session.lock().unwrap().submit(change, now)?;
        self.log_change(&body, &receipt);
        if let Some(t) = self.last_wait.lock().unwrap().take() {
            *self.last_gap.lock().unwrap() = Some(t.elapsed().as_secs_f64());
        }
        self.push()?;
        self.save(now);
        Ok(json!({ "ok": true, "receipt": receipt }))
    }

    /// 等到"排好的音乐短于 `runway` 秒"或自主演奏时间到了，最多 30 秒。
    fn wait(&self, runway: f64) -> Value {
        let t0 = Instant::now();
        let reason = loop {
            if self.is_stopped() {
                break "stopped";
            }
            if self.autoplay_left().is_some_and(|s| s <= 0.0) {
                break "autoplay_over";
            }
            let s = self.status();
            if s["runway_seconds"].as_f64().unwrap_or(0.0) < runway {
                break "runway";
            }
            if t0.elapsed() >= WAIT_MAX {
                break "idle";
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        *self.last_wait.lock().unwrap() = Some(Instant::now());
        let mut s = self.agent_status();
        s["reason"] = json!(reason);
        s["hint"] = json!(match reason {
            "stopped" => "演奏已经结束",
            "autoplay_over" => "自主演奏的时间到了：交最后一段（适合循环的乐句）就停下，等人类说话；音乐会一直循环",
            "runway" => "排好的音乐快放完了：写下一段并提交（没写也不会冷场，会循环最后一段）",
            _ => "还早：再调用一次 wait",
        });
        s
    }
}

/// 解析时长：`90`、`90s`、`3m`、`1m30s`；`0` 或 `off` 关掉。
pub fn parse_duration(s: &str) -> Result<Option<Duration>> {
    let s = s.trim().to_lowercase();
    if s == "off" || s == "0" {
        return Ok(None);
    }
    let mut total = 0f64;
    let mut num = String::new();
    for c in s.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'm' | 's' | 'h' => {
                let n: f64 = num.parse().map_err(|_| anyhow!("时长写法不对：{s}（如 90s、3m、1m30s）"))?;
                total += n * match c { 'h' => 3600.0, 'm' => 60.0, _ => 1.0 };
                num.clear();
            }
            _ => bail!("时长写法不对：{s}（如 90s、3m、1m30s）"),
        }
    }
    if !num.is_empty() {
        total += num.parse::<f64>().map_err(|_| anyhow!("时长写法不对：{s}"))?;
    }
    Ok((total > 0.0).then(|| Duration::from_secs_f64(total)))
}

/// 启动一场现场演奏（前台常驻，直到 `muser live stop`）。
#[allow(clippy::too_many_arguments)]
pub fn serve(template: &str, settings: muser_engine::RenderSettings, port: u16, autoplay: Option<Duration>, record: Option<String>, open: bool, title: Option<String>) -> Result<()> {
    let text = std::fs::read_to_string(template).with_context(|| format!("读不了模板 {template}"))?;
    let tpl = Project::from_json(&text).map_err(|e| anyhow!("{e}"))?.at_file(std::path::Path::new(template));
    let mut session = Session::new(&tpl)?;
    if let Some(t) = &title {
        session.set_title(t);
    }
    let player = muser_engine::playback::Player::start_live(settings).map_err(|e| anyhow!(e))?;
    let (server, port) = crate::web::bind(port)?;
    let live = Arc::new(Live {
        session: Mutex::new(session),
        player,
        record: record.map(PathBuf::from),
        autoplay_until: Mutex::new(autoplay.map(|d| Instant::now() + d)),
        last_wait: Mutex::new(None),
        last_gap: Mutex::new(None),
        started: Instant::now(),
        last_underrun: Mutex::new((None, 0)),
        stopped: Default::default(),
        underruns_seen: Mutex::new(0),
    });
    {
        let mut s = live.session.lock().unwrap();
        let keep = s.beat_after(0.0, KEEP_SECONDS);
        s.extend(keep);
    }
    live.push()?;
    std::fs::write(muser_engine::live_client::port_file(), json!({ "port": port, "pid": std::process::id(), "template": template }).to_string())?;
    let st = live.player.status();
    println!("muser live 已开始：http://127.0.0.1:{port}/  （只监听本机）  声卡：{}（{} Hz）", st.device, st.sample_rate);
    if open {
        crate::web::open_browser(&format!("http://127.0.0.1:{port}/"));
    }
    println!("用 muser live send / status / wait / autoplay / stop 控制；整场写进 {}", live.record.as_ref().map_or("（没有 --record，不保存）".into(), |p| p.display().to_string()));
    // 循环往后排：保持至少 KEEP_SECONDS 秒。
    {
        let l = Arc::clone(&live);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            if l.is_stopped() {
                continue;
            }
            l.watch_underruns();
            let now = l.now_beat();
            let changed = {
                let mut s = l.session.lock().unwrap();
                let until = s.beat_after(now, KEEP_SECONDS);
                s.extend(until)
            };
            if changed {
                if let Err(e) = l.push() {
                    eprintln!("[live] {e:#}");
                }
                l.save(now);
            }
        });
    }
    for mut req in server.incoming_requests() {
        if !crate::web::host_allowed(&req, port) {
            let _ = req.respond(tiny_http::Response::empty(403));
            continue;
        }
        let l = Arc::clone(&live);
        std::thread::spawn(move || {
            let url = req.url().to_string();
            let path = url.split('?').next().unwrap_or("").to_string();
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::Read::take(req.as_reader(), 4 << 20), &mut body);
            let q = crate::web::query(&url);
            let num = |k: &str, d: f64| q.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
            let (code, v) = match (req.method(), path.as_str()) {
                (tiny_http::Method::Get, "/api/status") => (200, l.agent_status()),
                (tiny_http::Method::Get, "/api/view") => {
                    let mut v = l.status();
                    v["position"] = json!(l.player.status().position);
                    let now = l.now_beat();
                    v["timeline"] = l.session.lock().unwrap().timeline(now);
                    v["levels"] = json!(l.player.levels().into_iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>());
                    (200, v)
                }
                (tiny_http::Method::Get, "/api/wait") => (200, l.wait(num("runway", 8.0))),
                (tiny_http::Method::Post, "/api/change") => match serde_json::from_str::<Value>(&body).map_err(anyhow::Error::from).and_then(|b| l.change(b)) {
                    Ok(v) => (200, v),
                    Err(e) => (400, json!({ "error": format!("{e:#}") })),
                },
                (tiny_http::Method::Post, "/api/mix") => match serde_json::from_str::<Value>(&body).map_err(anyhow::Error::from).and_then(|b| l.mix(&b)) {
                    Ok(v) => (200, v),
                    Err(e) => (400, json!({ "error": format!("{e:#}") })),
                },
                (tiny_http::Method::Post, "/api/autoplay") => {
                    let d = serde_json::from_str::<Value>(&body).ok().and_then(|b| b["duration"].as_str().map(str::to_string)).unwrap_or_default();
                    match parse_duration(&d) {
                        Ok(d) => {
                            *l.autoplay_until.lock().unwrap() = d.map(|d| Instant::now() + d);
                            (200, json!({ "ok": true, "autoplay_seconds_left": l.autoplay_left() }))
                        }
                        Err(e) => (400, json!({ "error": format!("{e:#}") })),
                    }
                }
                (tiny_http::Method::Post, "/api/stop") => {
                    if !l.stopped.swap(true, std::sync::atomic::Ordering::AcqRel) {
                        l.player.pause();
                        l.save(l.now_beat());
                        let _ = std::fs::remove_file(muser_engine::live_client::port_file());
                        // 再留几秒：页面看到"已结束"再退出。
                        std::thread::spawn(|| {
                            std::thread::sleep(Duration::from_secs(3));
                            // 加载过插件（Kontakt）时跳过 C++ 的退出析构，不然插件的后台线程还在跑就被清理，进程会崩溃。
                            muser_engine::plugin::exit(0);
                        });
                    }
                    (200, json!({ "ok": true, "saved": l.record.as_ref().map(|p| p.display().to_string()) }))
                }
                (tiny_http::Method::Get, "/") => {
                    let r = tiny_http::Response::from_string(include_str!("live.html"))
                        .with_header(crate::web::header("Content-Type", "text/html; charset=utf-8"))
                        .with_header(crate::web::header("Cache-Control", "no-store"));
                    let _ = req.respond(r);
                    return;
                }
                (tiny_http::Method::Get, "/favicon.ico") => {
                    let _ = req.respond(tiny_http::Response::empty(204));
                    return;
                }
                _ => (404, json!({ "error": "没有这个接口" })),
            };
            crate::web::respond_json(req, code, &v);
        });
    }
    Ok(())
}

// ---------- 客户端（muser live send / status / …） ----------

/// 向本机的 `muser live` 发一个请求。
pub fn request(port: Option<u16>, method: &str, path: &str, body: Option<&str>) -> Result<Value> {
    muser_engine::live_client::request(port, method, path, body).map_err(|e| anyhow!(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> Project {
        serde_json::from_value(json!({ "tempo": 120, "buses": [{ "name": "verb", "effects": [{ "type": "reverb" }] }],
            "tracks": [
                { "name": "drums", "instrument": "drums.909", "clips": [{ "start": 0, "length": 4, "grid": { "kick": "x...x...x...x..." } }] },
                { "name": "bass", "instrument": "bass.acid", "sends": { "verb": -12 }, "clips": [] }
            ] }))
        .unwrap()
    }

    fn change(v: Value) -> Change {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn segments_queue_loop_and_cut() {
        let mut s = Session::new(&template()).unwrap();
        // 模板一小节，循环往后排。
        assert!(s.extend(10.0));
        assert_eq!(s.segs.len(), 3, "0–4、4–8、8–12");
        // 现在第 1.5 拍：排在已经提交的（模板那一小节，第 4 拍）之后——但那里离现在不到 1.5 秒（3 拍），等这一遍循环放完。
        let r = s.submit(change(json!({ "bars": 2, "tracks": [{ "name": "bass", "clips": [{ "start": 0, "length": 8, "notes": [{ "pitch": 36, "start": 0, "length": 1 }] }] }] })), 1.5).unwrap();
        assert_eq!((r.absolute_beat, r.bar, r.beat_in_bar), (8.0, 3, 1.0), "{r:?}");
        assert_eq!(s.segs.len(), 3, "模板、一遍循环、新的一段");
        let p = s.project().unwrap();
        assert_eq!(p.form.len(), 3);
        let bass = p.tracks.iter().find(|t| t.name == "bass").unwrap();
        assert_eq!((bass.clips[0].start.0, bass.clips[0].length.0), (8.0, 8.0));
        // 再排一段 7/8：接在上一段后面（第 16 拍），新轨、效果器、自动化。
        let r = s.submit(change(json!({ "bars": 2, "time_signature": { "numerator": 7, "denominator": 8 }, "tracks": [
            { "name": "lead", "instrument": "synth.poly", "output": "verb", "clips": [{ "start": 0, "length": 7, "notes": [{ "pitch": 72, "start": 0, "length": 1 }] }] },
            { "name": "bass", "effects_add": [{ "type": "eq", "shape": "lowpass", "frequency": 800 }],
              "automation": [{ "target": "effects[0]", "param": "frequency", "shape": "points", "points": [[0, 800], [7, 3000]] }] } ] })), 2.0).unwrap();
        assert_eq!((r.absolute_beat, r.bar), (16.0, 5));
        let p = s.project().unwrap();
        let bass = p.tracks.iter().find(|t| t.name == "bass").unwrap();
        assert_eq!(bass.effects[0].from.map(|b| b.0), Some(16.0));
        assert_eq!(bass.automation[0].start.map(|b| b.0), Some(16.0));
        assert_eq!(p.bar_map().bar_at(19.5).0, 5, "7/8 一小节 3.5 拍：第 19.5 拍是第 6 小节的开头");
        // 循环最后一段：接缝在 23 拍；循环出来的不再接入效果器。
        s.extend(40.0);
        let p = s.project().unwrap();
        assert_eq!(p.tracks.iter().find(|t| t.name == "bass").unwrap().effects.len(), 1);
        assert_eq!(p.tracks.iter().find(|t| t.name == "lead").unwrap().clips.len(), s.segs.iter().filter(|g| g.start >= 16.0).count());
        // "now"：现在第 25 拍（循环里），下一条小节线在 1.5 秒（3 拍）之后的第 30 拍（23 + 2×3.5）。
        let r = s.submit(change(json!({ "at": "now", "bars": 1, "tracks": [] })), 25.0).unwrap();
        assert_eq!(r.absolute_beat, 30.0);
        assert_eq!(s.segs[s.segs.len() - 2].bars, 2);
        // 出错不改：已有的轨不能换乐器。
        let n = s.segs.len();
        assert!(s.submit(change(json!({ "bars": 1, "tracks": [{ "name": "bass", "instrument": "synth.poly" }] })), 26.0).is_err());
        assert!(s.submit(change(json!({ "bars": 1, "tracks": [{ "name": "nobody", "clips": [] }] })), 26.0).is_err(), "新轨要写乐器");
        assert_eq!(s.segs.len(), n);
        // 录下来：只到现在开始的段。
        assert!(s.project_until(26.0).unwrap().form.len() < s.project().unwrap().form.len());
    }

    /// 总线、主输出的自动化与效果器，段落名（循环、截短的注明）。
    /// 段里换速度：会话自己的速度表和生成的工程一致；渐变的段循环时停在新速度上；回执、status 的秒数按速度表算。
    #[test]
    fn tempo_changes_follow_the_map() {
        let mut s = Session::new(&template()).unwrap();
        let r = s.submit(change(json!({ "bars": 2, "tempo": 60, "tempo_ramp_bars": 2, "tracks": [] })), 0.0).unwrap();
        assert_eq!(r.absolute_beat, 4.0);
        assert!((r.starts_in_seconds - 2.0).abs() < 1e-9, "{r:?}");
        s.extend(60.0);
        let (mine, theirs) = (s.tm(), s.project().unwrap().tempo_map());
        for b in [0.0, 3.0, 5.5, 12.0, 30.0, 59.0] {
            assert!((mine.secs_at(b) - theirs.secs_at(b)).abs() < 1e-9, "第 {b} 拍");
        }
        assert!((mine.bpm_at(12.0) - 60.0).abs() < 1e-9 && (mine.bpm_at(30.0) - 60.0).abs() < 1e-9, "循环停在 60");
        // 120 → 60 在 8 拍里按时间线性：120·8/(120+60) 秒。
        assert!((mine.secs_between(4.0, 8.0) - 960.0 / 180.0).abs() < 1e-9);
        assert_eq!(s.status(12.0)["tempo"], 60.0);
    }

    #[test]
    fn buses_master_and_names() {
        let mut s = Session::new(&template()).unwrap();
        s.submit(change(json!({ "name": "outro", "bars": 2,
            "buses": [{ "name": "verb", "effects_add": [{ "type": "eq", "shape": "lowpass", "frequency": 2000 }],
                        "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, 0], [8, -12]] }] }],
            "master": { "automation": [{ "target": "master_gain_db", "shape": "points", "points": [[0, 0], [8, -40]] }] } })), 2.0).unwrap();
        s.extend(30.0);
        let p = s.project().unwrap();
        assert_eq!(p.buses[0].effects[1].from.map(|b| b.0), Some(8.0), "第 2 拍提交：模板那一小节再循环一遍，第 8 拍接上");
        assert_eq!(p.buses[0].effects.len(), 2, "循环出来的不再接入");
        assert_eq!(p.automation[0].start.map(|b| b.0), Some(8.0));
        assert_eq!(p.automation.len(), 1, "主输出的渐弱不在循环里重来");
        match &p.automation[0].shape { muser_core::automation::Shape::Points { points, .. } => assert_eq!(points[1].0, 16.0), _ => panic!() }
        assert!(p.form.iter().any(|f| matches!(f, muser_core::arrange::FormEntry::Name(n) if n == "003 outro")));
        assert!(p.form.iter().any(|f| matches!(f, muser_core::arrange::FormEntry::Name(n) if n == "004 outro (repeat)")));
        assert!(s.submit(change(json!({ "bars": 1, "buses": [{ "name": "nope" }] })), 3.0).is_err(), "总线要先在模板里建好");
        s.submit(change(json!({ "at": "now", "bars": 1 })), 9.0).unwrap();
        assert!(s.project().unwrap().form.iter().any(|f| matches!(f, muser_core::arrange::FormEntry::Name(n) if n.ends_with("(cut)"))));
    }

    /// 调音量写成那一刻起的自动化；比它晚的段写的音量自动化会覆盖它。
    #[test]
    fn mix_is_automation_in_time_order() {
        let mut s = Session::new(&template()).unwrap();
        s.extend(20.0);
        assert_eq!(s.mix("bass", -3.0, 5.0, 0.5).unwrap(), 0.0, "原来是 bass 的 gain_db 0");
        assert!(s.mix("nobody", -3.0, 5.0, 0.5).is_err());
        assert!(s.mix("bass", 30.0, 5.0, 0.5).is_err());
        let p = s.project().unwrap();
        assert_eq!(Session::fader_at(&p, "bass", 6.0), Some(-3.0));
        // 之后的一段写了音量自动化：从那里起听它的。
        s.submit(change(json!({ "bars": 1, "tracks": [{ "name": "bass", "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, -10]] }] }] })), 10.0).unwrap();
        let p = s.project().unwrap();
        let at = s.segs.last().unwrap().start;
        assert_eq!(Session::fader_at(&p, "bass", at + 0.5), Some(-10.0));
        assert_eq!(Session::fader_at(&p, "bass", 6.0), Some(-3.0), "之前的不变");
        s.mix("master", -6.0, at + 1.0, 0.5).unwrap();
        assert_eq!(Session::fader_at(&s.project().unwrap(), "master", at + 2.0), Some(-6.0));
    }

    /// patch：从下一小节起接着这一段的同一位置；跨过接口的片段只留之后的音；之后循环从头完整地重复。
    #[test]
    fn patch_continues_at_the_same_place() {
        let mut s = Session::new(&template()).unwrap();
        let song = |kick: &str| change(json!({ "at": "after", "bars": 4, "tracks": [
            { "name": "bass", "clips": [{ "start": 0, "length": 16, "notes": (0..16).map(|i| json!({ "pitch": 36 + i, "start": i, "length": 0.5 })).collect::<Vec<_>>() }] },
            { "name": "drums", "clips": [{ "start": 0, "length": 16, "grid": { "kick": kick } }] } ] }));
        s.submit(song("x..............."), 0.0).unwrap();
        // 这一段从第 4 拍开始（模板一小节之后）。现在第 9 拍：1.5 秒（3 拍）以后的下一条小节线是第 12 拍 = 这一段的第 3 小节。
        let mut c = song("x...x...x...x...");
        c.at = At::Patch;
        let r = s.submit(c, 9.0).unwrap();
        assert_eq!((r.absolute_beat, r.section_bar), (12.0, 3), "{r:?}");
        let p = s.project().unwrap();
        let bass = p.tracks.iter().find(|t| t.name == "bass").unwrap();
        // 原来那段被截在第 12 拍；改写的那段从第 12 拍起，只有这一段第 8 拍以后的音（音高 44 起）。
        let patched = bass.clips.iter().find(|c| (c.start.0 - 12.0).abs() < 1e-9).expect("patch 的片段");
        assert_eq!(patched.notes.first().map(|n| n.pitch), Some(44));
        assert_eq!(patched.length.0, 8.0);
        // 循环：从第 20 拍起，整段 4 小节从头重复。
        s.extend(40.0);
        let p = s.project().unwrap();
        let bass = p.tracks.iter().find(|t| t.name == "bass").unwrap();
        let rep = bass.clips.iter().find(|c| (c.start.0 - 20.0).abs() < 1e-9).expect("循环的片段");
        assert_eq!(rep.length.0, 16.0);
        assert!(p.form.iter().any(|f| matches!(f, muser_core::arrange::FormEntry::Name(n) if n.contains("patch from bar 3"))));
        // 已经放到这一段写的范围之外：报错。
        let mut c = song("x...");
        c.bars = 1;
        c.at = At::Patch;
        assert!(s.submit(c, 21.0).is_err());
    }

    /// 一段带歌词：时间线上有卡拉 OK 字幕，每个字的时间是整场的秒数。
    #[test]
    fn karaoke_in_the_timeline() {
        let mut s = Session::new(&template()).unwrap();
        s.submit(change(json!({ "bars": 2, "lyrics": "一闪一闪\n亮晶晶", "tracks": [{ "name": "guide", "instrument": "choir.solo", "gain_db": -80,
            "clips": [{ "start": 0, "length": 8, "abc": "L:1/4\nK:C\nCCGG | AAG2 |\nw: yi shan yi shan liang jing jing" }] }] })), 0.0).unwrap();
        let t = s.timeline(4.0);
        let seg = &t["segments"][1];
        let lines = seg["karaoke"].as_array().unwrap();
        assert_eq!(lines.len(), 2, "{seg}");
        // 120 BPM：这一段从第 4 拍（2 秒）开始，"一" 在 2 秒，"亮" 在第 2 小节开头（4 秒）。
        assert_eq!(lines[0]["tokens"][0]["text"], "一");
        assert_eq!(lines[0]["tokens"][0]["start"], 2.0);
        assert_eq!(lines[1]["tokens"][0]["text"], "亮");
        assert_eq!(lines[1]["start"], 4.0);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("3m").unwrap(), Some(Duration::from_secs(180)));
        assert_eq!(parse_duration("1m30s").unwrap(), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("45").unwrap(), Some(Duration::from_secs(45)));
        assert_eq!(parse_duration("off").unwrap(), None);
        assert!(parse_duration("3 minutes").is_err());
    }
}
