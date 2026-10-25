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

pub mod time;

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
    /// 所有音轨。
    #[serde(default)]
    pub tracks: Vec<Track>,
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
            tracks: Vec::new(),
        }
    }

    /// 工程的总时长（beats）：所有 clip 结束位置的最大值。
    pub fn duration_beats(&self) -> Beats {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.length)
            .fold(Beats(0.0), |acc, b| if b.0 > acc.0 { b } else { acc })
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
        for (ti, track) in self.tracks.iter().enumerate() {
            for (ci, clip) in track.clips.iter().enumerate() {
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
            }
        }
        Ok(())
    }
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
    /// 音轨增益（dB），默认 0。
    #[serde(default)]
    pub gain_db: f64,
    /// 声像，-1.0（全左）到 1.0（全右），默认 0（居中）。
    #[serde(default)]
    pub pan: f64,
    /// 是否静音。
    #[serde(default)]
    pub muted: bool,
    /// 该音轨上的所有 clip。
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(name: impl Into<String>, instrument: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            instrument: instrument.into(),
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            clips: Vec::new(),
        }
    }
}

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
}

impl Clip {
    pub fn new(start: impl Into<Beats>, length: impl Into<Beats>) -> Self {
        Self {
            start: start.into(),
            length: length.into(),
            notes: Vec::new(),
        }
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
}
