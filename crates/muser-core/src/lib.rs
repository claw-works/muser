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
    pub fn at_file(mut self, file: impl AsRef<std::path::Path>) -> Self {
        self.base_dir = file.as_ref().parent().map(std::path::Path::to_path_buf);
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
            return Err(CoreError::Invalid(format!("transpose = {}：整首变调在 −24 到 24 个半音之间", self.transpose)));
        }
        if !(self.tempo > 0.0) {
            return Err(CoreError::Invalid(format!(
                "tempo 必须为正数，实际为 {}",
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
                    "tracks[{ti}].pan = {} 超出范围 [-1, 1]",
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
                            return Err(CoreError::Invalid(format!("{at}: 音频片段不能再写音符（notes / abc / grid / drummer / pedal）")));
                        }
                    }
                    (Some(_), false) => {
                        return Err(CoreError::Invalid(format!("{at}.audio: 音频片段要放在 \"instrument\": \"audio\" 的音轨上")));
                    }
                    (None, true) => return Err(CoreError::Invalid(format!("{at}: audio 音轨的 clip 要写 audio（音频文件）"))),
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
                        "track[{ti}].clip[{ci}] start 为负: {}",
                        clip.start.0
                    )));
                }
                if clip.length.0 <= 0.0 {
                    return Err(CoreError::Invalid(format!(
                        "track[{ti}].clip[{ci}] length 必须为正: {}",
                        clip.length.0
                    )));
                }
                let mut prev_end = f64::NEG_INFINITY;
                for (pi, &(a, b)) in clip.pedal.iter().enumerate() {
                    let at = format!("tracks[{ti}].clips[{ci}].pedal[{pi}]");
                    if !(a.is_finite() && b.is_finite()) || a < 0.0 || b <= a {
                        return Err(CoreError::Invalid(format!("{at} = [{a}, {b}] 无效：应为 [起, 止]，0 ≤ 起 < 止（拍，相对 clip 起点）")));
                    }
                    if a < prev_end {
                        return Err(CoreError::Invalid(format!("{at} 与前一个区间重叠或顺序颠倒：区间应按时间递增、互不重叠")));
                    }
                    if a >= clip.length.0 {
                        return Err(CoreError::Invalid(format!("{at} 起点 {a} 超出 clip 长度 {}", clip.length.0)));
                    }
                    prev_end = b;
                }
                let notes = clip.all_notes().map_err(|e| CoreError::Invalid(format!("tracks[{ti}].clips[{ci}].{e}")))?;
                for (ni, note) in notes.iter().enumerate() {
                    if note.pitch > 127 || note.start.0 < 0.0 || note.length.0 <= 0.0 {
                        return Err(CoreError::Invalid(format!(
                            "track[{ti}].clip[{ci}].note[{ni}] 无效: pitch 0-127、start ≥ 0、length > 0"
                        )));
                    }
                    if !(0.0..=1.0).contains(&note.velocity) {
                        return Err(CoreError::Invalid(format!(
                            "track[{ti}].clip[{ci}].note[{ni}].velocity = {} 超出范围 [0, 1]",
                            note.velocity
                        )));
                    }
                    let mut last = 0.0;
                    for &(at, semis) in &note.bend {
                        if !(at >= last && semis.abs() <= 2.0) {
                            return Err(CoreError::Invalid(format!(
                                "track[{ti}].clip[{ci}].note[{ni}].bend: 点 [{at}, {semis}] 无效：拍 ≥ 0 且递增，半音在 ±2 之内"
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
                    return Err(format!("sections.{name}.tempo = {t} 应在 0–999 BPM 之间"));
                }
            }
            if sec.tempo_ramp_bars.is_some_and(|r| r > sec.bars) {
                return Err(format!("sections.{name}.tempo_ramp_bars 不能比这一段的小节数（{}）多", sec.bars));
            }
            if sec.bars == 0 || sec.bars > 1024 {
                return Err(format!("sections.{name}.bars = {} 应在 1–1024", sec.bars));
            }
            if name.contains('#') || name == "*" {
                return Err(format!("sections.{name}: 段落名不能含 # 或是 *"));
            }
            if let Some(c) = &sec.chords {
                chord::progression(c, sec.bars, self.section_meter(sec).beats_per_bar()).map_err(|e| format!("sections.{name}.chords: {e}"))?;
            }
        }
        for (i, e) in self.form.iter().enumerate() {
            if !self.sections.contains_key(e.section()) {
                let names: Vec<&str> = self.sections.keys().map(String::as_str).collect();
                return Err(format!("form[{i}]: 没有段落 '{}'（sections 里有: {}）", e.section(), names.join(", ")));
            }
            if let FormEntry::Detail(d) = e {
                for m in &d.mute {
                    if !self.tracks.iter().any(|t| &t.name == m) {
                        return Err(format!("form[{i}].mute: 没有叫 '{m}' 的音轨"));
                    }
                }
            }
        }
        for (ti, t) in self.tracks.iter().enumerate() {
            if t.parts.is_empty() {
                continue;
            }
            if self.form.is_empty() {
                return Err(format!("tracks[{ti}].parts: 要先写 form（段落顺序）与 sections"));
            }
            if t.instrument.trim() == audio::INSTRUMENT {
                return Err(format!("tracks[{ti}].parts: audio 音轨用 clips 放音频，不用 parts"));
            }
            for (key, part) in &t.parts {
                let (name, k) = arrange::part_key(key);
                if name != "*" && !self.sections.contains_key(name) {
                    return Err(format!("tracks[{ti}].parts.{key}: 没有段落 '{name}'"));
                }
                if k == Some(0) {
                    return Err(format!("tracks[{ti}].parts.{key}: 第几次从 1 起"));
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
        Layer::Track => "instrument / effects[i] / gain_db / pan / sends.<总线>",
        Layer::Bus => "effects[i] / gain_db / pan / sends.<总线>",
    };
    let target = Target::parse(&a.target).ok_or_else(|| err(format!("target '{}' 无效，可选: {allowed}", a.target)))?;
    let ok = matches!(
        (&target, layer),
        (Target::MasterGain | Target::MasterEffect(_), Layer::Master)
            | (Target::Instrument, Layer::Track)
            | (Target::Effect(_) | Target::Gain | Target::Pan | Target::Send(_), Layer::Track | Layer::Bus)
    );
    if !ok {
        let hint = match target {
            Target::MasterGain | Target::MasterEffect(_) => "；主总线目标写在工程级 automation",
            Target::Instrument if layer == Layer::Bus => "；总线没有乐器",
            _ if layer == Layer::Master => "；音轨/总线的目标写在各自的 automation",
            _ => "",
        };
        return Err(err(format!("target '{}' 不能写在这里，可选: {allowed}{hint}", a.target)));
    }
    if let Target::Effect(i) | Target::MasterEffect(i) = target {
        if i >= effects {
            return Err(err(format!("target '{}' 越界：这里只有 {effects} 个效果器", a.target)));
        }
    }
    if let Target::Send(name) = &target {
        if !sends.contains(&name.as_str()) {
            return Err(err(format!("没有发送到 '{name}'：先在 sends 里写上它（可写 -80 表示从无声开始）")));
        }
    }
    if target.is_scalar() && !a.param.is_empty() {
        return Err(err(format!("target '{}' 不需要 param", a.target)));
    }
    if !target.is_scalar() && a.param.is_empty() {
        return Err(err("缺少 param（要自动化的参数名）".into()));
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
    /// 来源标注（如 `loop:house_basic`），只作记录，渲染不看。
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

/// 一个音符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// MIDI 音高，0-127（60 = 中央 C / C4）。
    pub pitch: u8,
    /// 起点（beats，相对所在 clip 的起点）。
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
    /// 弯音（吉他推弦、滑进音、尾音下滑）：`[拍, 半音]` 折线，拍相对音符起点、递增，半音在 ±2 之内
    /// （MIDI 弯音轮的标准范围）。第一个点之前取第一个点的值，点与点之间直线过渡。
    /// 推弦：`[[0, 0], [0.25, 2]]`；预推再放：`[[0, 2], [0.5, 2], [1, 0]]`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bend: Vec<(f64, f64)>,
    /// 唱的音节（`choir.*` 乐器）：拼音或英文拼法，如 `"la"`、`"shi"`、`"hai"`、`"sing"`。起头的辅音模拟出来，
    /// 元音映射到 a e i o u。ABC 里用 `w:` 歌词行按顺序对上音符。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyric: Option<String>,
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
            bend: Vec::new(),
            lyric: None,
        }
    }
}

/// 核心层错误。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("JSON 序列化/反序列化错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("工程校验失败: {0}")]
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
        assert!(p("[[0, 4], [3, 6]]").unwrap_err().to_string().contains("重叠"));
        assert!(p("[[9, 10]]").unwrap_err().to_string().contains("超出"));
    }
}
