//! muser-core —— 领域模型层
//!
//! 这是「AI ↔ 引擎」之间的结构化契约。AI 产出 / 编辑的是这里的 [`Project`]，
//! 引擎适配层（muser-engine）再把它编译成 manguebyte 的 Synth/Graph 并渲染。
//!
//! 设计原则：
//! - 用**音乐时间单位**（bar/beat）而非样本/秒，保持与 tempo 无关，便于 AI 推理。
//! - 全部 `serde` 可序列化为 JSON —— JSON 就是 AI 读写工程的格式。
//! - 概念层级贴近音乐人直觉：Project → Track → Clip → Note。
//! - 乐器以**语义名**（如 `"drums.808"`、`"synth.poly"`）引用，由 engine 层解析成
//!   具体的 manguebyte 乐器，核心层不耦合引擎实现。

use serde::{Deserialize, Serialize};

pub mod abc;
pub mod arrange;
pub mod chord;
pub mod audio;
pub mod automation;
pub mod tempo;
pub mod effect;
pub mod grid;
pub mod humanize;
pub mod routing;
pub mod time;
pub mod tuning;

pub use automation::{Automation, CompiledCurve, Curve, LfoWave, RandomStyle, Shape, Target};
pub use effect::{effect_spec, ChoiceSpec, Effect, EffectSlot, EffectSpec, ParamSpec, EFFECT_SPECS};
pub use routing::{Bus, MASTER};
pub use time::{Beats, TimeSignature};

/// 一个完整的编曲工程。AI 的顶层产出物，也是工程文件（`.muser.json`）的根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// 工程文件格式版本，用于未来迁移。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 工程名。
    #[serde(default)]
    pub name: String,
    /// 歌词原文（给人看：`muser view` 里显示，录人声时对着唱），如 `"[a]\n月亮升起来了\n风吹过山冈"`。
    /// 不影响声音；唱哪个音节写在音符的 `lyric` / ABC 的 `w:` 行里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<String>,
    /// 速度（每分钟四分音符数）。
    pub tempo: f64,
    /// 拍号，默认 4/4。
    #[serde(default)]
    pub time_signature: TimeSignature,
    /// 主输出增益（dB），默认 0。
    #[serde(default)]
    pub master_gain_db: f64,
    /// 主总线效果器链（按顺序串联），默认空。在主增益之前施加。
    #[serde(default)]
    pub master_effects: Vec<EffectSlot>,
    /// 主总线自动化（目标 `master_gain_db` / `master_effects[i]`），默认空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Automation>,
    /// 调律（可省略，默认十二平均律、A4 = 440）：历史律制、微分音、Scala 文件。见 [`tuning`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tuning: Option<tuning::Tuning>,
    /// 随机种子：没有写 `seed` 的 random 自动化由它推导。换一个就换一种随机变化。默认 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seed: u64,
    /// 整首变调（半音，−24 到 24）：所有有音高的乐器音轨演奏时都移这么多，音符本身不改；鼓组（`drums.*`）与音频轨不动。
    /// 换一个人唱（比如换成女声）时用它找合适的调。
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub transpose: i32,
    /// 总线（可省略）：汇集音轨/总线的信号统一处理，如共用混响、鼓组压缩。见 [`routing`]。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<Bus>,
    /// 所有音轨。
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// 段落定义：名字 → 小节数与和弦进行。见 [`arrange`]。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sections: std::collections::BTreeMap<String, arrange::Section>,
    /// 曲式：段落按顺序演奏（从工程开头起），各音轨在每段里演奏自己的 `parts`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<arrange::FormEntry>,
    /// 工程文件所在目录（不写进 JSON）：音频片段的相对路径相对它。读工程文件时由调用方设置。
    #[serde(skip)]
    pub base_dir: Option<std::path::PathBuf>,
}

fn default_schema_version() -> u32 {
    1
}

impl Project {
    /// 创建一个空工程。
    pub fn new(name: impl Into<String>, tempo: f64) -> Self {
        Self {
            schema_version: default_schema_version(),
            name: name.into(),
            lyrics: None,
            tempo,
            time_signature: TimeSignature::default(),
            master_gain_db: 0.0,
            master_effects: Vec::new(),
            automation: Vec::new(),
            tuning: None,
            seed: 0,
            transpose: 0,
            buses: Vec::new(),
            tracks: Vec::new(),
            sections: Default::default(),
            form: Vec::new(),
            base_dir: None,
        }
    }

    /// 工程的总时长（beats）：所有 clip 结束位置与曲式长度的最大值。
    pub fn duration_beats(&self) -> Beats {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.length)
            .fold(Beats(self.form_beats()), |acc, b| if b.0 > acc.0 { b } else { acc })
    }

    /// 一小节多少拍（四分音符为一拍），工程的拍号。段落可以有自己的拍号，按位置换算小节用 [`Self::bar_map`]。
    pub fn bar_beats(&self) -> f64 {
        f64::from(self.time_signature.numerator) * 4.0 / f64::from(self.time_signature.denominator)
    }

    /// 段落的拍号：段落写了就用它，否则用工程的。
    pub fn section_meter(&self, s: &arrange::Section) -> TimeSignature {
        s.time_signature.unwrap_or(self.time_signature)
    }

    /// 曲式的总长（拍）。段落名不存在时不计（校验会报错）。
    pub fn form_beats(&self) -> f64 {
        self.form.iter().filter_map(|e| self.sections.get(e.section())).map(|s| f64::from(s.bars) * self.section_meter(s).beats_per_bar()).sum()
    }

    /// 速度表：拍 ↔ 秒（段落各有速度、渐变时）。没有换速度时就是一个 `tempo` 到底。
    pub fn tempo_map(&self) -> tempo::TempoMap {
        let mut m = tempo::TempoMap::constant(self.tempo);
        let mut t = 0.0;
        for s in self.form.iter().filter_map(|e| self.sections.get(e.section())) {
            let bpb = self.section_meter(s).beats_per_bar();
            if let Some(bpm) = s.tempo {
                let ramp = f64::from(s.tempo_ramp_bars.unwrap_or(0).min(s.bars)) * bpb;
                m.change(t, bpm, ramp);
            }
            t += f64::from(s.bars) * bpb;
        }
        m
    }

    /// 段落里换调律的地方：(起点拍, 从那里起的调律)，按时间排好。没有段落写 `tuning` 时为空（整首用工程的 `tuning`）。
    pub fn tuning_changes(&self) -> Vec<(f64, &tuning::Tuning)> {
        let mut out = Vec::new();
        let mut t = 0.0;
        for s in self.form.iter().filter_map(|e| self.sections.get(e.section())) {
            if let Some(tu) = &s.tuning {
                out.push((t, tu));
            }
            t += f64::from(s.bars) * self.section_meter(s).beats_per_bar();
        }
        out
    }

    /// 拍号表：小节和拍之间的换算（段落各有拍号时，小节不再一样长）。
    pub fn bar_map(&self) -> BarMap {
        let mut segs: Vec<(f64, TimeSignature)> = vec![(0.0, self.time_signature)];
        let mut t = 0.0;
        for s in self.form.iter().filter_map(|e| self.sections.get(e.section())) {
            let ts = self.section_meter(s);
            if segs.last().is_some_and(|l| l.1 != ts) {
                if segs.last().is_some_and(|l| l.0 == t) {
                    segs.pop();
                }
                segs.push((t, ts));
            }
            t += f64::from(s.bars) * ts.beats_per_bar();
        }
        BarMap { segs }
    }

    /// 记下工程文件的位置（音频片段的相对路径相对它所在目录）。
    /// 插件的 `@state` 状态文件写相对路径时也相对工程所在目录（乐器的 `instrument_params`、插件效果器的 `params`）。
    pub fn at_file(mut self, file: impl AsRef<std::path::Path>) -> Self {
        self.base_dir = file.as_ref().parent().map(std::path::Path::to_path_buf);
        if let Some(dir) = self.base_dir.clone().filter(|d| !d.as_os_str().is_empty()) {
            let fix = |params: &mut serde_json::Map<String, serde_json::Value>| {
                if let Some(serde_json::Value::String(st)) = params.get_mut("@state") {
                    let p = std::path::Path::new(st.as_str());
                    if p.is_relative() && !st.starts_with('~') {
                        *st = dir.join(p).display().to_string();
                    }
                }
            };
            let fx = |list: &mut Vec<EffectSlot>| {
                for slot in list {
                    if let Effect::Plugin { params, .. } = &mut slot.effect {
                        fix(params);
                    }
                }
            };
            for t in &mut self.tracks {
                fix(&mut t.instrument_params);
                fx(&mut t.effects);
            }
            for b in &mut self.buses {
                fx(&mut b.effects);
            }
            fx(&mut self.master_effects);
        }
        self
    }

    /// 从 JSON 字符串解析工程。
    pub fn from_json(s: &str) -> Result<Self, CoreError> {
        serde_json::from_str(s).map_err(CoreError::Json)
    }

    /// 序列化为（带缩进的）JSON 字符串。
    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self).map_err(CoreError::Json)
    }

    /// 基本校验：tempo 为正、clip 时间非负等。
    pub fn validate(&self) -> Result<(), CoreError> {
        if !(-24..=24).contains(&self.transpose) {
            return Err(CoreError::Invalid(format!("transpose = {}: whole-piece transposition must be between -24 and 24 semitones", self.transpose)));
        }
        if !(self.tempo > 0.0) {
            return Err(CoreError::Invalid(format!(
                "tempo must be positive, got {}",
                self.tempo
            )));
        }
        routing::plan(self).map_err(CoreError::Invalid)?;
        if let Some(t) = &self.tuning {
            t.validate().map_err(CoreError::Invalid)?;
        }
        self.validate_arrangement().map_err(CoreError::Invalid)?;
        for (i, a) in self.automation.iter().enumerate() {
            check_automation(a, Layer::Master, self.master_effects.len(), &[], &format!("automation[{i}]"))?;
        }
        for (bi, bus) in self.buses.iter().enumerate() {
            let sends: Vec<&str> = bus.sends.keys().map(String::as_str).collect();
            for (i, a) in bus.automation.iter().enumerate() {
                check_automation(a, Layer::Bus, bus.effects.len(), &sends, &format!("buses[{bi}].automation[{i}]"))?;
            }
            for (i, e) in bus.effects.iter().enumerate() {
                e.validate()
                    .map_err(|m| CoreError::Invalid(format!("buses[{bi}].effects[{i}]: {m}")))?;
            }
        }
        for (i, e) in self.master_effects.iter().enumerate() {
            e.validate()
                .map_err(|m| CoreError::Invalid(format!("master_effects[{i}]: {m}")))?;
        }
        for (ti, track) in self.tracks.iter().enumerate() {
            if !(-1.0..=1.0).contains(&track.pan) {
                return Err(CoreError::Invalid(format!(
                    "tracks[{ti}].pan = {} is out of range [-1, 1]",
                    track.pan
                )));
            }
            let sends: Vec<&str> = track.sends.keys().map(String::as_str).collect();
            for (i, a) in track.automation.iter().enumerate() {
                check_automation(a, Layer::Track, track.effects.len(), &sends, &format!("tracks[{ti}].automation[{i}]"))?;
            }
            for (i, e) in track.effects.iter().enumerate() {
                e.validate()
                    .map_err(|m| CoreError::Invalid(format!("tracks[{ti}].effects[{i}]: {m}")))?;
            }
            if let Some(h) = &track.humanize {
                h.validate(&format!("tracks[{ti}].humanize")).map_err(CoreError::Invalid)?;
            }
            let audio_track = track.instrument.trim() == audio::INSTRUMENT;
            for (ci, clip) in track.clips.iter().enumerate() {
                let at = format!("tracks[{ti}].clips[{ci}]");
                match (&clip.audio, audio_track) {
                    (Some(a), true) => {
                        a.validate(&format!("{at}.audio")).map_err(CoreError::Invalid)?;
                        if !clip.notes.is_empty() || clip.abc.is_some() || clip.grid.is_some() || clip.drummer.is_some() || !clip.pedal.is_empty() {
                            return Err(CoreError::Invalid(format!("{at}: an audio clip cannot also contain notes (notes / abc / grid / drummer / pedal)")));
                        }
                    }
                    (Some(_), false) => {
                        return Err(CoreError::Invalid(format!("{at}.audio: audio clips must be on a track with \"instrument\": \"audio\"")));
                    }
                    (None, true) => return Err(CoreError::Invalid(format!("{at}: a clip on an audio track needs audio (an audio file)"))),
                    (None, false) => {}
                }
                if let Some(h) = &clip.humanize {
                    h.validate(&format!("tracks[{ti}].clips[{ci}].humanize")).map_err(CoreError::Invalid)?;
                }
                for (i, t) in clip.transform.iter().enumerate() {
                    t.validate(&format!("tracks[{ti}].clips[{ci}].transform[{i}]")).map_err(CoreError::Invalid)?;
                }
                if let Some(d) = &clip.drummer {
                    d.validate(&format!("tracks[{ti}].clips[{ci}].drummer")).map_err(CoreError::Invalid)?;
                }
                if clip.start.0 < 0.0 {
                    return Err(CoreError::Invalid(format!(
                        "track[{ti}].clip[{ci}] start is negative: {}",
                        clip.start.0
                    )));
                }
                if clip.length.0 <= 0.0 {
                    return Err(CoreError::Invalid(format!(
                        "track[{ti}].clip[{ci}] length must be positive: {}",
                        clip.length.0
                    )));
                }
                let mut prev_end = f64::NEG_INFINITY;
                for (pi, &(a, b)) in clip.pedal.iter().enumerate() {
                    let at = format!("tracks[{ti}].clips[{ci}].pedal[{pi}]");
                    if !(a.is_finite() && b.is_finite()) || a < 0.0 || b <= a {
                        return Err(CoreError::Invalid(format!("{at} = [{a}, {b}] is invalid: expected [start, end] with 0 <= start < end (beats, relative to the clip start)")));
                    }
                    if a < prev_end {
                        return Err(CoreError::Invalid(format!("{at} overlaps the previous range or is out of order: ranges must be in increasing time order and not overlap")));
                    }
                    if a >= clip.length.0 {
                        return Err(CoreError::Invalid(format!("{at} start {a} is beyond the clip length {}", clip.length.0)));
                    }
                    prev_end = b;
                }
                let notes = clip.all_notes().map_err(|e| CoreError::Invalid(format!("tracks[{ti}].clips[{ci}].{e}")))?;
                for (ni, note) in notes.iter().enumerate() {
                    // 倚音、弱起、提前的装饰音可以比 clip 起点早一点（至多 1 拍），但不能早于工程开头。
                    if note.pitch > 127 || note.start.0 < -1.0 || clip.start.0 + note.start.0 < 0.0 || note.length.0 <= 0.0 {
                        return Err(CoreError::Invalid(format!(
                            "track[{ti}].clip[{ci}].note[{ni}] is invalid: need pitch 0-127, start >= -1 (at most a beat before the clip, not before beat 0 of the project), length > 0"
                        )));
                    }
                    if !(0.0..=1.0).contains(&note.velocity) {
                        return Err(CoreError::Invalid(format!(
                            "track[{ti}].clip[{ci}].note[{ni}].velocity = {} is out of range [0, 1]",
                            note.velocity
                        )));
                    }
                    if (note.technique.is_some() || note.harmonic.is_some()) && !takes_technique(&track.instrument) {
                        return Err(CoreError::Invalid(format!("track[{ti}].clip[{ci}].note[{ni}].technique: only yinse.pluck.guqin and yinse.pluck.guzheng accept per-note string techniques")));
                    }
                    if note.trill > 4 {
                        return Err(CoreError::Invalid(format!("track[{ti}].clip[{ci}].note[{ni}].trill: {} semitones is too wide (1-4)", note.trill)));
                    }
                    if note.harmonic.is_some_and(|h| !(2..=8).contains(&h)) {
                        return Err(CoreError::Invalid(format!("track[{ti}].clip[{ci}].note[{ni}].harmonic: node denominator must be 2-8")));
                    }
                    let mut last = 0.0;
                    let bend_range = if takes_technique(&track.instrument) { 24.0 } else { 2.0 };
                    for &(at, semis) in &note.bend {
                        if !(at.is_finite() && at >= last && semis.abs() <= bend_range) {
                            return Err(CoreError::Invalid(format!(
                                "track[{ti}].clip[{ci}].note[{ni}].bend: point [{at}, {semis}] is invalid: beats must be >= 0 and increasing, semitones within +/-{bend_range}"
                            )));
                        }
                        last = at;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Project {
    /// 段落、曲式、parts 的写法。
    fn validate_arrangement(&self) -> Result<(), String> {
        for (name, sec) in &self.sections {
            if let Some(t) = sec.tempo {
                if !(t.is_finite() && t > 0.0 && t <= 999.0) {
                    return Err(format!("sections.{name}.tempo = {t} must be between 0 and 999 BPM"));
                }
            }
            if let Some(t) = &sec.tuning {
                t.validate().map_err(|e| format!("sections.{name}.{e}"))?;
            }
            if sec.tempo_ramp_bars.is_some_and(|r| r > sec.bars) {
                return Err(format!("sections.{name}.tempo_ramp_bars cannot exceed this section's bar count ({})", sec.bars));
            }
            if sec.bars == 0 || sec.bars > 1024 {
                return Err(format!("sections.{name}.bars = {} must be between 1 and 1024", sec.bars));
            }
            if name.contains('#') || name == "*" {
                return Err(format!("sections.{name}: a section name cannot contain # or be *"));
            }
            if let Some(c) = &sec.chords {
                chord::progression(c, sec.bars, self.section_meter(sec).beats_per_bar()).map_err(|e| format!("sections.{name}.chords: {e}"))?;
            }
        }
        for (i, e) in self.form.iter().enumerate() {
            if !self.sections.contains_key(e.section()) {
                let names: Vec<&str> = self.sections.keys().map(String::as_str).collect();
                return Err(format!("form[{i}]: no section '{}' (sections has: {})", e.section(), names.join(", ")));
            }
            if let FormEntry::Detail(d) = e {
                for m in &d.mute {
                    if !self.tracks.iter().any(|t| &t.name == m) {
                        return Err(format!("form[{i}].mute: no track named '{m}'"));
                    }
                }
            }
        }
        for (ti, t) in self.tracks.iter().enumerate() {
            if t.parts.is_empty() {
                continue;
            }
            if self.form.is_empty() {
                return Err(format!("tracks[{ti}].parts: write form (section order) and sections first"));
            }
            if t.instrument.trim() == audio::INSTRUMENT {
                return Err(format!("tracks[{ti}].parts: audio tracks put audio in clips, not parts"));
            }
            for (key, part) in &t.parts {
                let (name, k) = arrange::part_key(key);
                if name != "*" && !self.sections.contains_key(name) {
                    return Err(format!("tracks[{ti}].parts.{key}: no section '{name}'"));
                }
                if k == Some(0) {
                    return Err(format!("tracks[{ti}].parts.{key}: the occurrence number starts at 1"));
                }
                part.validate(&format!("tracks[{ti}].parts.{key}"), false)?;
            }
        }
        Ok(())
    }
}

use arrange::FormEntry;

/// 自动化写在哪一层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Master,
    Track,
    Bus,
}

/// 自动化的结构检查：目标是否属于这一层、效果器下标与发送是否存在、是否缺 param、形状是否合法。
/// 参数名与取值范围由 muser-engine 按参数表检查。
fn check_automation(a: &Automation, layer: Layer, effects: usize, sends: &[&str], at: &str) -> Result<(), CoreError> {
    let err = |m: String| CoreError::Invalid(format!("{at}: {m}"));
    let allowed = match layer {
        Layer::Master => "master_gain_db / master_effects[i]",
        Layer::Track => "instrument / effects[i] / gain_db / pan / sends.<bus>",
        Layer::Bus => "effects[i] / gain_db / pan / sends.<bus>",
    };
    let target = Target::parse(&a.target).ok_or_else(|| err(format!("target '{}' is invalid; choose one of: {allowed}", a.target)))?;
    let ok = matches!(
        (&target, layer),
        (Target::MasterGain | Target::MasterEffect(_), Layer::Master)
            | (Target::Instrument, Layer::Track)
            | (Target::Effect(_) | Target::Gain | Target::Pan | Target::Send(_), Layer::Track | Layer::Bus)
    );
    if !ok {
        let hint = match target {
            Target::MasterGain | Target::MasterEffect(_) => "; master targets go in the project-level automation",
            Target::Instrument if layer == Layer::Bus => "; a bus has no instrument",
            _ if layer == Layer::Master => "; track/bus targets go in their own automation",
            _ => "",
        };
        return Err(err(format!("target '{}' cannot be used here; choose one of: {allowed}{hint}", a.target)));
    }
    if let Target::Effect(i) | Target::MasterEffect(i) = target {
        if i >= effects {
            return Err(err(format!("target '{}' is out of range: there are only {effects} effects here", a.target)));
        }
    }
    if let Target::Send(name) = &target {
        if !sends.contains(&name.as_str()) {
            return Err(err(format!("no send to '{name}': add it to sends first (use -80 to start silent)")));
        }
    }
    if target.is_scalar() && !a.param.is_empty() {
        return Err(err(format!("target '{}' does not take param", a.target)));
    }
    if !target.is_scalar() && a.param.is_empty() {
        return Err(err("missing param (the name of the parameter to automate)".into()));
    }
    a.check_shape().map_err(err)
}

/// 一条音轨：一件乐器 + 一串 clip。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// 音轨名。
    #[serde(default)]
    pub name: String,
    /// 乐器的语义引用，例如 `"drums.808"`、`"synth.poly"`、`"fm"`。
    /// 由 muser-engine 的乐器注册表解析为具体 manguebyte 乐器。
    pub instrument: String,
    /// 覆盖乐器音色的参数（可省略）。先用所选乐器/预设的音色打底，只改写到的项。
    /// 键与取值范围见 `muser instruments <名称>`，例如
    /// `{ "cutoff": 800, "release": 0.2 }`；鼓组按鼓件：`{ "kick": { "tune": -2 } }`。
    #[serde(default, skip_serializing_if = "InstrumentParams::is_empty")]
    pub instrument_params: InstrumentParams,
    /// 音轨增益（dB），默认 0。
    #[serde(default)]
    pub gain_db: f64,
    /// 声像，-1.0（全左）到 1.0（全右），默认 0（居中）。
    #[serde(default)]
    pub pan: f64,
    /// 是否静音。
    #[serde(default)]
    pub muted: bool,
    /// 轨级效果器链（按顺序串联），默认空。在增益/声像之前施加。
    #[serde(default)]
    pub effects: Vec<EffectSlot>,
    /// 输出到哪里：`"master"`（默认）或某条总线的名字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// 发送：总线名 → 发送量（dB，推子后），如 `{ "reverb": -8 }`。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sends: std::collections::BTreeMap<String, f64>,
    /// 音轨自动化（目标 `instrument` / `effects[i]` / `gain_db` / `pan` / `sends.<总线>`），默认空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Automation>,
    /// 人性化演奏（整轨默认；clip 上的同名字段覆盖它）。见 [`humanize`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<humanize::Humanize>,
    /// 该音轨上的所有 clip。
    #[serde(default)]
    pub clips: Vec<Clip>,
    /// 在各段落里演奏什么：段落名（`"verse"`、`"verse#2"`、`"*"`）→ 内容。见 [`arrange`]。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parts: std::collections::BTreeMap<String, arrange::Part>,
}

impl Track {
    pub fn new(name: impl Into<String>, instrument: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instrument: instrument.into(),
            instrument_params: InstrumentParams::new(),
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            effects: Vec::new(),
            output: None,
            sends: std::collections::BTreeMap::new(),
            automation: Vec::new(),
            humanize: None,
            clips: Vec::new(),
            parts: Default::default(),
        }
    }
}

/// 乐器参数：参数名 → 值（数字、字符串、布尔，鼓组为嵌套对象）。
/// 具体含义由 muser-engine 的乐器参数表解释与校验。
pub type InstrumentParams = serde_json::Map<String, serde_json::Value>;

/// 一个片段：在时间线上的一段音符集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// 片段在时间线上的起点（beats，从工程开头算起）。
    pub start: Beats,
    /// 片段长度（beats）。超出此长度的音符会被引擎裁剪。
    pub length: Beats,
    /// 片段内的音符。音符时间相对 clip 起点。
    #[serde(default)]
    pub notes: Vec<Note>,
    /// 延音踏板（CC64）踩下的区间 `[起, 止]`（拍，相对 clip 起点），按时间递增、互不重叠。
    /// 踩下期间松开的键继续发声，抬起时一起进入释音。换踏板：前一区间结束与下一区间开始之间留一点空隙。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pedal: Vec<(f64, f64)>,
    /// 连奏：每个音延长到与下一个（不同音高的）音重叠约 40 毫秒，让乐器按连奏演奏
    /// （SFZ 的 `trigger=legato` 连奏采样、单音合成器的滑音）。音之间空隙超过 1/4 拍（休止）、
    /// 或下一个音同音高（重新发音）时不连。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legato: bool,
    /// 人性化演奏，覆盖音轨 `humanize` 的同名字段。见 [`humanize`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<humanize::Humanize>,
    /// 用 ABC 记谱写的音符（相对 clip 起点），渲染时展开，与 `notes` 合在一起。见 [`abc`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abc: Option<String>,
    /// 步进网格（鼓、贝斯线），在 clip 长度里循环展开，与 `notes` 合在一起。见 [`grid`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<grid::Grid>,
    /// 生成式鼓手（以 loop 为底加变化与过门），由引擎展开（`muser_engine::clip_notes`）。见 [`grid::Drummer`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drummer: Option<grid::Drummer>,
    /// 来源（如 `loop:house_basic`）。clip 有内容时只作记录；只写了 `from`、没有 notes / grid / abc / drummer 时，
    /// 引擎从 loop 库取这个 loop 的内容和律动（humanize），在 clip 长度里循环。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// 音频文件（只在 `"instrument": "audio"` 的音轨上）。见 [`audio`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<audio::AudioClip>,
    /// 变换（移调、逆行、错位、随机变奏…），施加在这个 clip 展开后的音符上。见 [`arrange::Transform`]。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transform: Vec<arrange::Transform>,
}

impl Clip {
    pub fn new(start: impl Into<Beats>, length: impl Into<Beats>) -> Self {
        Self {
            start: start.into(),
            length: length.into(),
            pedal: Vec::new(),
            legato: false,
            humanize: None,
            abc: None,
            grid: None,
            drummer: None,
            from: None,
            audio: None,
            transform: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// 全部音符：`notes` 加上 `abc`、`grid` 文字写法展开的音符（相对 clip 起点）。写法有错时报错（校验时已检查）。
    /// 不含 `drummer`（要读 loop 库，由引擎展开：`muser_engine::clip_notes`）。
    pub fn all_notes(&self) -> Result<std::borrow::Cow<'_, [Note]>, String> {
        if self.abc.is_none() && self.grid.is_none() {
            return Ok(std::borrow::Cow::Borrowed(&self.notes));
        }
        let mut all = self.notes.clone();
        if let Some(text) = self.abc.as_deref() {
            all.extend(abc::parse(text).map_err(|e| format!("abc: {e}"))?);
        }
        if let Some(g) = &self.grid {
            all.extend(g.expand(self.length.0)?);
        }
        Ok(std::borrow::Cow::Owned(all))
    }
}

/// 弦乐器（古琴、古筝）的逐音奏法：散音、按音、泛音。音高仍写听到的音高；泛音节点分母决定保留哪些弦模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Technique {
    Auto,
    Open,
    Pressed,
    Harmonic,
}

/// 接受逐音奏法（`technique` / `harmonic`）、可弯 ±24 半音的乐器（模态弦模型）。
pub fn takes_technique(instrument: &str) -> bool {
    ["yinse.pluck.guqin", "yinse.pluck.guzheng"].iter().any(|n| instrument.trim().eq_ignore_ascii_case(n))
}

/// 一个音符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// MIDI 音高，0-127（60 = 中央 C / C4）。
    pub pitch: u8,
    /// 起点（beats，相对所在 clip 的起点）。可以是负的，至多 -1 拍（倚音、弱起），但不能早于工程开头。
    pub start: Beats,
    /// 时值（beats）。
    pub length: Beats,
    /// 力度，0.0-1.0，默认 0.8。
    #[serde(default = "default_velocity")]
    pub velocity: f32,
    /// 重音（可省略，默认 false）。鼓组与 acid 贝斯等乐器会让重音更响、更亮；
    /// 强度由乐器参数（如 acid 的 `accent`）决定。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub accent: bool,
    /// 闷音（可省略，默认 false）：电吉他的掌闷（`yinse.pluck.electric_guitar*`），音短而暗，深浅由乐器参数 `mute` 决定。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub mute: bool,
    /// 轮音（可省略，默认 false）：这个音整个时值里快速重复地弹（琵琶的轮指、扬琴的轮音、古筝的摇指），
    /// 快慢由乐器参数 `tremolo_rate` 决定。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub tremolo: bool,
    /// 滑音（可省略，默认 false）：这个字从上一个字的音高滑进自己的音高（`yinse.voice.*` 唱歌的滑音，
    /// 在这个字的开头滑）。ABC 里写 `!slide!`（标准 ABC 的"滑进这个音"）。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub slide: bool,
    /// 滑出（可省略，默认 false）：这个字唱完前，在它的最后一段滑到下一个字的音高上（`yinse.voice.*`），
    /// 下一个字从那里接着唱。ABC 里写 `!slideout!`。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub slide_out: bool,
    /// 换气（可省略，默认 false）：这个音之前留一口气（`yinse.voice.*`）：前一个音提早收住，空出来的地方唱一声吸气；
    /// 连奏的音不会连进这个音。ABC 里写 `!breath!`。其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub breath: bool,
    /// 颤音（可省略，默认 0）：这个音和上方 `trill` 个半音的音快速交替（每秒约 8 个来回），整个时值都颤；
    /// `true` 等于 2（全音），`1` 是半音。笛子南派的指颤，其它乐器也一样展开。
    #[serde(default, skip_serializing_if = "is_zero_u8", deserialize_with = "trill_semitones")]
    pub trill: u8,
    /// 虚颤音（可省略，默认 false）：手指在音孔上半开半合地颤，音高小幅快速起伏（约 ±30 音分，每秒 7 下），比颤音柔。
    /// 音符自己写了 `bend` 时不加。能弯音的乐器才有。
    #[serde(default, skip_serializing_if = "is_false")]
    pub half_trill: bool,
    /// 花舌（可省略，默认 false）：吹这个音时舌尖打嘟噜，气流被打成每秒约 40 下的颤动，声音"嘟噜噜"带沙（`yinse.blow.dizi`）。
    /// 其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub flutter: bool,
    /// 止音（可省略，默认 false）：这个音结束时手按住弦，余音很快收掉（古筝、古琴谱上的 `×`）；
    /// 不写就任弦自己响。`yinse.pluck.guzheng`、`yinse.pluck.guqin`；其它乐器忽略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub damp: bool,
    /// 弯音（吉他推弦、滑进音、尾音下滑）：`[拍, 半音]` 折线，拍相对音符起点、递增，半音在 ±2 之内
    /// （MIDI 弯音轮的标准范围）；古琴按音可到 ±24。第一个点之前取第一个点的值，点与点之间直线过渡。
    /// 推弦：`[[0, 0], [0.25, 2]]`；预推再放：`[[0, 2], [0.5, 2], [1, 0]]`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bend: Vec<(f64, f64)>,
    /// 唱的音节（`choir.*` 乐器）：拼音或英文拼法，如 `"la"`、`"shi"`、`"hai"`、`"sing"`。起头的辅音模拟出来，
    /// 元音映射到 a e i o u。ABC 里用 `w:` 歌词行按顺序对上音符。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyric: Option<String>,
    /// 弦乐器（古琴）的逐音奏法，覆盖音轨默认值：`"open"` 散音、`"pressed"` 按音、`"harmonic"` 泛音、`"auto"`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technique: Option<Technique>,
    /// 泛音的节点分母 2-8（2 = 弦的一半处）；只写 `harmonic` 也就是泛音。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harmonic: Option<u8>,
}

impl Note {
    /// 这个音写了的逐音奏法：只写 `harmonic` 就是泛音。
    pub fn string_technique(&self) -> Option<(Technique, Option<u8>)> {
        match (self.technique, self.harmonic) {
            (None, None) => None,
            (t, h) => Some((t.unwrap_or(Technique::Harmonic), h)),
        }
    }
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

/// `"trill": true`（全音）、`false`、或半音数。
fn trill_semitones<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum T {
        B(bool),
        N(u8),
    }
    Ok(match T::deserialize(d)? {
        T::B(b) => if b { 2 } else { 0 },
        T::N(n) => n,
    })
}

fn default_velocity() -> f32 {
    0.8
}

impl Note {
    pub fn new(pitch: u8, start: impl Into<Beats>, length: impl Into<Beats>) -> Self {
        Self {
            pitch,
            start: start.into(),
            length: length.into(),
            velocity: default_velocity(),
            accent: false,
            mute: false,
            tremolo: false,
            slide: false,
            slide_out: false,
            breath: false, damp: false, flutter: false, trill: 0, half_trill: false,
            bend: Vec::new(),
            lyric: None,
            technique: None, harmonic: None,
        }
    }
}

/// 核心层错误。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project validation failed: {0}")]
    Invalid(String),
}

/// 拍号表（[`Project::bar_map`]）：每次换拍号的位置（拍）与拍号，第一项在 0；曲式之后保持最后的拍号。
#[derive(Debug, Clone, PartialEq)]
pub struct BarMap {
    pub segs: Vec<(f64, TimeSignature)>,
}

impl BarMap {
    /// 第 `beat` 拍在第几小节（从 0 起）、小节里第几拍（从 0 起）。
    pub fn bar_at(&self, beat: f64) -> (usize, f64) {
        let mut first = 0usize;
        for (i, &(start, ts)) in self.segs.iter().enumerate() {
            let bb = ts.beats_per_bar();
            let end = self.segs.get(i + 1).map_or(f64::INFINITY, |n| n.0);
            if beat < end - 1e-9 || i + 1 == self.segs.len() {
                let into = (beat - start).max(0.0);
                let k = (into / bb + 1e-9).floor();
                return (first + k as usize, (into - k * bb).max(0.0));
            }
            first += ((end - start) / bb).round() as usize;
        }
        (0, beat)
    }

    /// 第 `bar` 小节（从 0 起）从第几拍开始。
    pub fn bar_start(&self, bar: usize) -> f64 {
        let mut first = 0usize;
        for (i, &(start, ts)) in self.segs.iter().enumerate() {
            let bb = ts.beats_per_bar();
            let n = self.segs.get(i + 1).map_or(usize::MAX, |next| ((next.0 - start) / bb).round() as usize);
            if bar < first.saturating_add(n) {
                return start + (bar - first) as f64 * bb;
            }
            first += n;
        }
        0.0
    }

    /// 第 `bar` 小节（从 0 起）的拍号。
    pub fn meter_of(&self, bar: usize) -> TimeSignature {
        let b = self.bar_start(bar);
        self.segs.iter().rev().find(|s| s.0 <= b + 1e-9).map_or(self.segs[0].1, |s| s.1)
    }

    /// 到第 `beat` 拍为止一共几个小节（不足一小节算一小节）。
    pub fn bars_until(&self, beat: f64) -> usize {
        let (bar, into) = self.bar_at(beat);
        if into > 1e-9 { bar + 1 } else { bar }.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guqin_techniques_roundtrip_and_wide_bends_stay_in_scope() {
        let mut p: Project = serde_json::from_value(serde_json::json!({
            "name":"guqin","tempo":60,"tracks":[{"name":"q","instrument":"yinse.pluck.guqin",
            "clips":[{"start":0,"length":4,"notes":[{"pitch":62,"start":0,"length":3,
            "technique":"pressed","bend":[[0,0],[1,12],[2,-12]]}]}]}]
        })).unwrap();
        assert!(p.validate().is_ok());
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        p.tracks[0].clips[0].notes[0].bend[1].1 = 25.0;
        assert!(p.validate().is_err());
        p.tracks[0].clips[0].notes[0].bend[1].1 = 12.0;
        p.tracks[0].clips[0].notes[0].technique = None;
        p.tracks[0].instrument = "guitar.nylon".into();
        assert!(p.validate().is_err(), "other instruments retain their +/-2 bend range");
        p.tracks[0].instrument = "yinse.pluck.guqin".into();
        p.tracks[0].clips[0].notes[0].technique = Some(Technique::Harmonic);
        p.tracks[0].clips[0].notes[0].harmonic = Some(1);
        assert!(p.validate().is_err(), "touching at the end is not a harmonic node");
    }

    #[test]
    fn notes_may_start_up_to_a_beat_before_their_clip() {
        let p = |clip: f64, note: f64| Project::from_json(&format!(r#"{{"name":"n","tempo":120,"tracks":[{{"name":"d","instrument":"drums.808",
            "clips":[{{"start":{clip},"length":4,"notes":[{{"pitch":38,"start":{note},"length":0.1}}]}}]}}]}}"#)).unwrap().validate();
        assert!(p(4.0, -0.1).is_ok());
        assert!(p(4.0, -1.5).is_err(), "more than a beat early");
        assert!(p(0.0, -0.1).is_err(), "before the project starts");
    }

    #[test]
    fn bar_map_follows_section_meters() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120,
            "sections": { "a": { "bars": 2 }, "b": { "bars": 3, "time_signature": { "numerator": 7, "denominator": 8 } } },
            "form": ["a", "b", "a"], "tracks": [] }))
        .unwrap();
        let m = p.bar_map();
        // a: 2 × 4 拍 = 0–8；b: 3 × 3.5 拍 = 8–18.5；a: 18.5–26.5，之后保持 4/4。
        assert_eq!(p.form_beats(), 26.5);
        assert_eq!(m.segs.len(), 3);
        assert_eq!(m.bar_at(0.0), (0, 0.0));
        assert_eq!(m.bar_at(9.0), (2, 1.0));
        assert_eq!(m.bar_at(11.5), (3, 0.0));
        assert_eq!(m.bar_at(19.5), (5, 1.0));
        assert_eq!(m.bar_at(30.5), (8, 0.0));
        assert_eq!((m.bar_start(2), m.bar_start(4), m.bar_start(5), m.bar_start(7)), (8.0, 15.0, 18.5, 26.5));
        assert_eq!(m.meter_of(3).numerator, 7);
        assert_eq!(m.meter_of(5).numerator, 4);
        assert_eq!(m.bars_until(26.5), 7);
    }

    #[test]
    fn roundtrip_json() {
        let mut p = Project::new("demo", 120.0);
        let mut track = Track::new("drums", "drums.808");
        let mut clip = Clip::new(0.0, 4.0);
        clip.notes.push(Note::new(36, 0.0, 0.5)); // kick on beat 1
        clip.notes.push(Note::new(38, 1.0, 0.5)); // snare on beat 2
        track.clips.push(clip);
        p.tracks.push(track);

        let json = p.to_json().unwrap();
        let back = Project::from_json(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn duration_is_max_clip_end() {
        let mut p = Project::new("demo", 120.0);
        let mut t = Track::new("t", "synth.poly");
        t.clips.push(Clip::new(0.0, 4.0));
        t.clips.push(Clip::new(8.0, 2.0)); // ends at beat 10
        p.tracks.push(t);
        assert_eq!(p.duration_beats(), Beats(10.0));
    }

    #[test]
    fn validate_rejects_bad_tempo() {
        let p = Project::new("x", 0.0);
        assert!(p.validate().is_err());
    }

    #[test]
    fn defaults_apply_on_minimal_json() {
        // 最小 JSON：AI 只需给 tempo 和一条 track 即可，其余走默认。
        let json = r#"{
            "tempo": 90.0,
            "tracks": [
                { "instrument": "drums.lofi", "clips": [
                    { "start": 0.0, "length": 4.0, "notes": [
                        { "pitch": 36, "start": 0.0, "length": 0.5 }
                    ]}
                ]}
            ]
        }"#;
        let p = Project::from_json(json).unwrap();
        assert_eq!(p.schema_version, 1);
        assert_eq!(p.time_signature, TimeSignature::default());
        assert_eq!(p.tracks[0].clips[0].notes[0].velocity, 0.8);
        p.validate().unwrap();
    }

    #[test]
    fn pedal_validation() {
        let p = |pedal: &str| {
            Project::from_json(&format!(r#"{{ "tempo": 120, "tracks": [{{ "instrument": "synth.poly", "clips": [{{ "start": 0, "length": 8, "pedal": {pedal}, "notes": [] }}] }}] }}"#))
                .unwrap()
                .validate()
        };
        assert!(p("[[0, 3.9], [4, 7.9]]").is_ok());
        assert!(p("[[0, 9]]").is_ok(), "超出 clip 末尾时截到 clip 结束");
        assert!(p("[[2, 1]]").unwrap_err().to_string().contains("pedal[0]"));
        assert!(p("[[0, 4], [3, 6]]").unwrap_err().to_string().contains("overlaps"));
        assert!(p("[[9, 10]]").unwrap_err().to_string().contains("beyond"));
    }
}
