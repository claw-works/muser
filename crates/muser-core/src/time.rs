//! 音乐时间单位。
//!
//! muser 的核心模型用 **beats（拍）** 作为时间单位，与 tempo、采样率解耦。
//! 1 beat = 一个四分音符（在 x/4 拍号下）。engine 层负责 beats → 样本 的换算：
//!
//! ```text
//! samples = beats * (60 / tempo) * sample_rate
//! ```

use serde::{Deserialize, Serialize};

/// 以「拍」为单位的时间量（可表示位置或时长）。
///
/// 透明包装 `f64`，在 JSON 中直接序列化为数字（例如 `2.5` 表示两拍半），
/// 对 AI 友好——无需理解结构体，直接写数字即可。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Beats(pub f64);

impl Beats {
    pub const ZERO: Beats = Beats(0.0);

    /// 换算为秒。
    pub fn to_seconds(self, tempo: f64) -> f64 {
        self.0 * (60.0 / tempo)
    }

    /// 换算为样本数（向下取整）。
    pub fn to_samples(self, tempo: f64, sample_rate: f64) -> u64 {
        (self.to_seconds(tempo) * sample_rate) as u64
    }
}

impl From<f64> for Beats {
    fn from(v: f64) -> Self {
        Beats(v)
    }
}

impl From<i32> for Beats {
    fn from(v: i32) -> Self {
        Beats(v as f64)
    }
}

impl std::ops::Add for Beats {
    type Output = Beats;
    fn add(self, rhs: Beats) -> Beats {
        Beats(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Beats {
    type Output = Beats;
    fn sub(self, rhs: Beats) -> Beats {
        Beats(self.0 - rhs.0)
    }
}

/// 拍号，例如 4/4、3/4、6/8。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    /// 每小节的拍数（分子）。
    pub numerator: u8,
    /// 以几分音符为一拍（分母），常见 4 或 8。
    pub denominator: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        TimeSignature {
            numerator: 4,
            denominator: 4,
        }
    }
}

impl TimeSignature {
    /// 一个小节有多少「beat」（以四分音符为 1 beat）。
    /// 例如 4/4 = 4.0 beat；6/8 = 3.0 beat（6 个八分音符 = 3 个四分音符）。
    pub fn beats_per_bar(self) -> f64 {
        self.numerator as f64 * (4.0 / self.denominator as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_to_seconds_at_120bpm() {
        // 120 bpm → 每拍 0.5 秒
        assert_eq!(Beats(4.0).to_seconds(120.0), 2.0);
    }

    #[test]
    fn beats_to_samples() {
        // 1 拍 @ 120bpm @ 48000Hz = 0.5s * 48000 = 24000 样本
        assert_eq!(Beats(1.0).to_samples(120.0, 48_000.0), 24_000);
    }

    #[test]
    fn time_signature_beats_per_bar() {
        assert_eq!(TimeSignature::default().beats_per_bar(), 4.0);
        assert_eq!(
            TimeSignature {
                numerator: 6,
                denominator: 8
            }
            .beats_per_bar(),
            3.0
        );
    }

    #[test]
    fn beats_transparent_json() {
        // Beats 在 JSON 中就是裸数字
        let b: Beats = serde_json::from_str("2.5").unwrap();
        assert_eq!(b, Beats(2.5));
        assert_eq!(serde_json::to_string(&Beats(3.0)).unwrap(), "3.0");
    }
}
