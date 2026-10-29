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
pub mod effect;
pub mod grid;
pub mod humanize;
pub mod routing;
pub mod time;

pub use automation::{Automation, CompiledCurve, Curve, LfoWave, RandomStyle, Shape, Target};
pub use effect::{effect_spec, ChoiceSpec, Effect, EffectSpec, ParamSpec, EFFECT_SPECS};
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
    pub master_effects: Vec<Effect>,
    /// 主总线自动化（目标 `master_gain_db` / `master_effects[i]`），默认空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Automation>,
    /// 随机种子：没有写 `seed` 的 random 自动化由它推导。换一个就换一种随机变化。默认 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seed: u64,
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
            tempo,
            time_signature: TimeSignature::default(),
            master_gain_db: 0.0,
            master_effects: Vec::new(),
            automation: Vec::new(),
            seed: 0,
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

    /// 一小节多少拍（四分音符为一拍）。
    pub fn bar_beats(&self) -> f64 {
        f64::from(self.time_signature.numerator) * 4.0 / f64::from(self.time_signature.denominator)
    }

    /// 曲式的总长（拍）。段落名不存在时不计（校验会报错）。
    pub fn form_beats(&self) -> f64 {
        self.form.iter().filter_map(|e| self.sections.get(e.section())).map(|s| f64::from(s.bars) * self.bar_beats()).sum()
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
        if !(self.tempo > 0.0) {
            return Err(CoreError::Invalid(format!(
                "tempo 必须为正数，实际为 {}",
                self.tempo
            )));
        }
        routing::plan(self).map_err(CoreError::Invalid)?;
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
            if sec.bars == 0 || sec.bars > 1024 {
                return Err(format!("sections.{name}.bars = {} 应在 1–1024", sec.bars));
            }
            if name.contains('#') || name == "*" {
                return Err(format!("sections.{name}: 段落名不能含 # 或是 *"));
            }
            if let Some(c) = &sec.chords {
                chord::progression(c, sec.bars, self.bar_beats()).map_err(|e| format!("sections.{name}.chords: {e}"))?;
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
    pub effects: Vec<Effect>,
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
}

fn is_zero(v: &u64) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
