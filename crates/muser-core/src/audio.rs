//! 音频片段：在音轨上直接放一段录音 / 采样文件（人声、吉他、现成的 loop、音效）。
//!
//! ```json
//! { "name": "vocal", "instrument": "audio", "clips": [
//!   { "start": 8, "length": 16, "audio": { "file": "vocal.wav", "offset": 1.5, "fade_out": 0.5 } } ] }
//! ```
//!
//! - 音轨的乐器写 `"audio"`，它的每个 clip 都写 `audio`、不写音符。之后照常走效果器、推子、总线、发送、侧链。
//! - clip 的 `start` / `length` 仍然是拍：文件从 `start` 开始播，到 `length` 截止（文件更短就提前结束）。
//! - 文件里的时间（`offset`、淡入淡出）是秒，按原速播放，不随 tempo 拉伸、不变调。
//! - 格式：WAV、AIFF、FLAC、Ogg Vorbis、Opus、MP3；macOS / Windows 上还有 AAC、ALAC（m4a）。采样率自动转换。
//! - 相对路径相对工程文件所在目录；也可写绝对路径或 `~/`。

use serde::{Deserialize, Serialize};

/// 音频音轨的乐器名。
pub const INSTRUMENT: &str = "audio";

/// clip 上的音频：哪个文件、从文件的哪里开始、音量与淡入淡出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    /// 音频文件路径。
    pub file: String,
    /// 从文件的第几秒开始播（裁掉开头），默认 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// 片段增益（dB），默认 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gain_db: f64,
    /// 淡入（秒），默认 0.005（防止开头咔哒声）。
    #[serde(default = "declick")]
    pub fade_in: f64,
    /// 淡出（秒，到片段结束处），默认 0.005。
    #[serde(default = "declick")]
    pub fade_out: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn declick() -> f64 {
    0.005
}

impl AudioClip {
    pub fn validate(&self, at: &str) -> Result<(), String> {
        if self.file.trim().is_empty() {
            return Err(format!("{at}.file 不能为空"));
        }
        for (name, v) in [("offset", self.offset), ("fade_in", self.fade_in), ("fade_out", self.fade_out)] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("{at}.{name} = {v} 应 ≥ 0（秒）"));
            }
        }
        if !(-60.0..=24.0).contains(&self.gain_db) {
            return Err(format!("{at}.gain_db = {} 超出范围 [-60, 24]", self.gain_db));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_checks() {
        let a: AudioClip = serde_json::from_str(r#"{ "file": "a.wav" }"#).unwrap();
        assert_eq!((a.offset, a.gain_db, a.fade_in, a.fade_out), (0.0, 0.0, 0.005, 0.005));
        assert!(a.validate("x").is_ok());
        assert!(AudioClip { offset: -1.0, ..a.clone() }.validate("x").unwrap_err().contains("x.offset"));
        assert!(serde_json::from_str::<AudioClip>(r#"{ "file": "a.wav", "speed": 2 }"#).is_err());
    }
}
