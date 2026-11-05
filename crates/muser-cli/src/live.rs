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
    /// `"after"`（默认）：排在已经提交的音乐之后（在循环里时：这一遍放完再接）；`"now"`：下一条小节线就换（截断正在放的这段）。
    #[serde(default)]
    pub at: At,
    /// 这一段几小节。
    pub bars: u32,
    /// 这一段的拍号；不写就用模板的。
    #[serde(default)]
    pub time_signature: Option<TimeSignature>,
    #[serde(default)]
    pub tracks: Vec<TrackChange>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum At {
    #[default]
    After,
    Now,
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
}

impl Seg {
    fn len(&self) -> f64 {
        f64::from(self.bars) * self.ts.beats_per_bar()
    }
    fn end(&self) -> f64 {
        self.start + self.len()
    }
}

/// 提交的回执。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Receipt {
    /// 生效在第几小节（从 1 起）、第几拍。
    pub bar: usize,
    pub beat: f64,
    /// 离生效还有几秒。
    pub seconds: f64,
    /// 排到了第几小节（这一段之后）。
    pub queued_until_bar: usize,
}

/// 一场演奏的音乐部分：模板 + 依次的段。
pub struct Session {
    base: Project,
    segs: Vec<Seg>,
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
        let s = Session { base, segs: vec![Seg { start: 0.0, bars, ts, change: Arc::new(first), rep: false }] };
        s.project()?;
        Ok(s)
    }

    pub fn tempo(&self) -> f64 {
        self.base.tempo
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
            self.segs.push(Seg { start, rep: true, ..src.clone() });
            changed = true;
        }
        changed
    }

    /// 提交一段。`now` 是现在演奏到的拍。出错时什么也不改。
    pub fn submit(&mut self, change: Change, now: f64) -> Result<Receipt> {
        if change.bars == 0 || change.bars > 256 {
            bail!("bars 应在 1 到 256 之间");
        }
        let earliest = now + MARGIN_SECONDS * self.tempo() / 60.0;
        let before = self.segs.clone();
        let at = match change.at {
            At::After => {
                // 排在已经提交的之后；那里已经过了（在循环里）就等这一遍循环放完。
                let mut b = self.real_end();
                let l = self.last_real().len();
                while b < earliest {
                    b += l;
                }
                b
            }
            At::Now => {
                self.extend(earliest + 1.0);
                let s = self.segs.iter().find(|s| s.end() > earliest).expect("刚排过").clone();
                let bpb = s.ts.beats_per_bar();
                let k = ((earliest - s.start) / bpb - 1e-9).ceil().max(0.0) as u32;
                s.start + f64::from(k.min(s.bars)) * bpb
            }
        };
        self.extend(at);
        // 排着还没放的段里接入了效果器：效果链只能往后加，去不掉。
        if self.segs.iter().any(|s| s.start >= at - 1e-9 && !s.rep && s.change.tracks.iter().any(|t| !t.effects_add.is_empty())) {
            self.segs = before;
            bail!("后面排着的段接入了效果器，现在截不掉：等它开始放以后再用 \"now\"，或者用 \"after\"");
        }
        // 截在 `at`：之后的（还没演奏的循环）去掉，跨过 `at` 的那段只留前面整小节。
        self.segs.retain(|s| s.start < at - 1e-9);
        if let Some(last) = self.segs.last_mut() {
            if last.end() > at + 1e-9 {
                last.bars = ((at - last.start) / last.ts.beats_per_bar()).round() as u32;
            }
        }
        let ts = change.time_signature.unwrap_or(self.base.time_signature);
        self.segs.push(Seg { start: at, bars: change.bars, ts, change: Arc::new(change), rep: false });
        match self.project() {
            Ok(p) => {
                let map = p.bar_map();
                Ok(Receipt {
                    bar: map.bar_at(at).0 + 1,
                    beat: at,
                    seconds: ((at - now) * 60.0 / self.tempo()).max(0.0),
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
            let name = format!("{:03}", k + 1);
            sections.insert(name.clone(), muser_core::arrange::Section { bars: seg.bars, time_signature: Some(seg.ts), chords: None });
            p.form.push(muser_core::arrange::FormEntry::Name(name));
            apply(&mut p, seg).with_context(|| format!("第 {} 段", k + 1))?;
        }
        p.sections = sections;
        muser_engine::validate(&p).map_err(|e| anyhow!("{e}"))?;
        Ok(p)
    }

    /// 给 agent 看的现状：现在第几小节、排到哪、是否在循环、每条轨、上一段写了什么。
    pub fn status(&self, now: f64) -> Value {
        let p = match self.project() {
            Ok(p) => p,
            Err(e) => return json!({ "error": format!("{e:#}") }),
        };
        let map = p.bar_map();
        let spb = 60.0 / self.tempo();
        let real_end = self.real_end();
        let cur = self.segs.iter().position(|s| s.end() > now).unwrap_or(self.segs.len() - 1);
        let seg = &self.segs[cur];
        let last = self.last_real();
        let looping = now >= real_end;
        let reps = self.segs.iter().filter(|s| s.rep && s.start >= real_end - 1e-9 && s.start <= now).count();
        json!({
            "bar": map.bar_at(now).0 + 1,
            "beat": now,
            "tempo": self.tempo(),
            "time_signature": [seg.ts.numerator, seg.ts.denominator],
            "segment": { "index": cur + 1, "start_bar": map.bar_at(seg.start).0 + 1, "bars": seg.bars, "repeat": seg.rep,
                         "seconds_to_end": ((seg.end() - now) * spb).max(0.0) },
            "queued_until_bar": map.bar_at(real_end).0,
            "runway_seconds": ((real_end - now) * spb).max(0.0),
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
    /// 给页面画时间线：每一段（秒、小节、拍号、是不是循环出来的、哪些轨在演奏）。
    pub fn timeline(&self) -> Value {
        let spb = 60.0 / self.tempo();
        let mut bar = 1u32;
        let segs: Vec<Value> = self
            .segs
            .iter()
            .map(|s| {
                let v = json!({ "start": s.start * spb, "end": s.end() * spb, "bar": bar, "bars": s.bars,
                    "meter": [s.ts.numerator, s.ts.denominator], "repeat": s.rep,
                    "tracks": s.change.tracks.iter().filter(|t| !t.clips.is_empty()).map(|t| t.name.clone()).collect::<Vec<_>>() });
                bar += s.bars;
                v
            })
            .collect();
        json!({ "segments": segs, "queued_until": self.real_end() * spb })
    }
}

impl Change {
    /// 写回 JSON（给 status：agent 接着写下一段时参考）。
    fn to_json(&self) -> Value {
        json!({
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

/// 把一段写进工程：片段、自动化平移到这一段的开头；新轨在这里建；效果器从这里接入 / 撤下。
fn apply(p: &mut Project, seg: &Seg) -> Result<()> {
    let (off, len) = (seg.start, seg.len());
    for tc in &seg.change.tracks {
        let i = match p.tracks.iter().position(|t| t.name == tc.name) {
            // 循环出来的重复：新轨那一遍已经建好了。
            Some(i) if seg.rep => i,
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
            if c.start.0 < 0.0 || c.start.0 >= len - 1e-9 {
                continue;
            }
            let mut c = c.clone();
            c.length = Beats(c.length.0.min(len - c.start.0));
            c.start = Beats(c.start.0 + off);
            t.clips.push(c);
        }
        if !seg.rep {
            for &k in &tc.effects_off {
                let e = t.effects.get_mut(k).ok_or_else(|| anyhow!("音轨 '{}' 没有 effects[{k}]", tc.name))?;
                e.until = Some(Beats(e.until.map_or(off, |u| u.0.min(off))));
            }
            for e in &tc.effects_add {
                let mut e = e.clone();
                e.from = Some(Beats(off + e.from.map_or(0.0, |b| b.0)));
                e.until = e.until.map(|b| Beats(off + b.0));
                t.effects.push(e);
            }
        }
        for a in &tc.automation {
            let mut a = a.clone();
            if let muser_core::automation::Shape::Points { points, .. } = &mut a.shape {
                for pt in points.iter_mut() {
                    pt.0 += off;
                }
            }
            a.start = Some(Beats(off + a.start.map_or(0.0, |b| b.0)));
            a.end = a.end.map(|b| Beats(off + b.0));
            t.automation.push(a);
        }
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
}

impl Live {
    fn now_beat(&self) -> f64 {
        self.player.status().position * self.session.lock().unwrap().tempo() / 60.0
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

    fn status(&self) -> Value {
        let now = self.now_beat();
        let mut v = self.session.lock().unwrap().status(now);
        let st = self.player.status();
        v["underruns"] = json!(st.underruns);
        v["error"] = json!(st.error);
        v["autoplay_seconds_left"] = json!(self.autoplay_left());
        v["last_write_seconds"] = json!(*self.last_gap.lock().unwrap());
        v["elapsed_seconds"] = json!(self.started.elapsed().as_secs_f64());
        v
    }

    fn change(&self, body: Value) -> Result<Value> {
        let change: Change = serde_json::from_value(body).context("改动的写法不对")?;
        let now = self.now_beat();
        let receipt = self.session.lock().unwrap().submit(change, now)?;
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
        let mut s = self.status();
        s["reason"] = json!(reason);
        s["hint"] = json!(match reason {
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

fn port_file() -> PathBuf {
    std::env::temp_dir().join("muser-live.json")
}

/// 启动一场现场演奏（前台常驻，直到 `muser live stop`）。
pub fn serve(template: &str, settings: muser_engine::RenderSettings, port: u16, autoplay: Option<Duration>, record: Option<String>, open: bool) -> Result<()> {
    let text = std::fs::read_to_string(template).with_context(|| format!("读不了模板 {template}"))?;
    let tpl = Project::from_json(&text).map_err(|e| anyhow!("{e}"))?.at_file(std::path::Path::new(template));
    let session = Session::new(&tpl)?;
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
    });
    {
        let tempo = live.session.lock().unwrap().tempo();
        live.session.lock().unwrap().extend(KEEP_SECONDS * tempo / 60.0);
    }
    live.push()?;
    std::fs::write(port_file(), json!({ "port": port, "pid": std::process::id(), "template": template }).to_string())?;
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
            let now = l.now_beat();
            let changed = {
                let mut s = l.session.lock().unwrap();
                let keep = KEEP_SECONDS * s.tempo() / 60.0;
                s.extend(now + keep)
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
                (tiny_http::Method::Get, "/api/status") => (200, l.status()),
                (tiny_http::Method::Get, "/api/view") => {
                    let mut v = l.status();
                    v["position"] = json!(l.player.status().position);
                    v["timeline"] = l.session.lock().unwrap().timeline();
                    v["levels"] = json!(l.player.levels().into_iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>());
                    (200, v)
                }
                (tiny_http::Method::Get, "/api/wait") => (200, l.wait(num("runway", 8.0))),
                (tiny_http::Method::Post, "/api/change") => match serde_json::from_str::<Value>(&body).map_err(anyhow::Error::from).and_then(|b| l.change(b)) {
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
                    l.save(l.now_beat());
                    crate::web::respond_json(req, 200, &json!({ "ok": true, "saved": l.record.as_ref().map(|p| p.display().to_string()) }));
                    let _ = std::fs::remove_file(port_file());
                    std::thread::sleep(Duration::from_millis(200));
                    std::process::exit(0);
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

/// 找到正在跑的 `muser live` 的端口。
fn find_port(port: Option<u16>) -> Result<u16> {
    if let Some(p) = port {
        return Ok(p);
    }
    let text = std::fs::read_to_string(port_file()).map_err(|_| anyhow!("没有正在进行的现场演奏：先 muser live start <模板>"))?;
    let v: Value = serde_json::from_str(&text)?;
    v["port"].as_u64().map(|p| p as u16).ok_or_else(|| anyhow!("{} 写坏了", port_file().display()))
}

/// 向本机的 `muser live` 发一个请求（不加 HTTP 依赖：直接写 HTTP/1.1）。
pub fn request(port: Option<u16>, method: &str, path: &str, body: Option<&str>) -> Result<Value> {
    use std::io::{Read, Write};
    let port = find_port(port)?;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|_| anyhow!("连不上 muser live（127.0.0.1:{port}）：它还在跑吗？"))?;
    s.set_read_timeout(Some(WAIT_MAX + Duration::from_secs(10)))?;
    let body = body.unwrap_or("");
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    let (head, rest) = out.split_once("\r\n\r\n").ok_or_else(|| anyhow!("muser live 的回应不完整"))?;
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    // tiny_http 对小回应不分块；分块时取第一块之后的 JSON。
    let json_text = rest.find('{').map_or(rest, |i| &rest[i..]);
    let json_text = json_text.rfind('}').map_or(json_text, |j| &json_text[..=j]);
    let v: Value = serde_json::from_str(json_text).with_context(|| format!("muser live 的回应不是 JSON：{rest}"))?;
    if code >= 400 {
        bail!("{}", v["error"].as_str().unwrap_or("muser live 报错"));
    }
    Ok(v)
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
        assert_eq!((r.beat, r.bar), (8.0, 3), "{r:?}");
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
        assert_eq!((r.beat, r.bar), (16.0, 5));
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
        assert_eq!(r.beat, 30.0);
        assert_eq!(s.segs[s.segs.len() - 2].bars, 2);
        // 出错不改：已有的轨不能换乐器。
        let n = s.segs.len();
        assert!(s.submit(change(json!({ "bars": 1, "tracks": [{ "name": "bass", "instrument": "synth.poly" }] })), 26.0).is_err());
        assert!(s.submit(change(json!({ "bars": 1, "tracks": [{ "name": "nobody", "clips": [] }] })), 26.0).is_err(), "新轨要写乐器");
        assert_eq!(s.segs.len(), n);
        // 录下来：只到现在开始的段。
        assert!(s.project_until(26.0).unwrap().form.len() < s.project().unwrap().form.len());
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
