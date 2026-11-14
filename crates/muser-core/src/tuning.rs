//! 调律（工程的 `tuning`）：十二平均律以外的音高——历史律制（纯律、中庸全音律、Werckmeister…）、
//! 平均分成 N 份的微分音（19 / 24 / 31 / 53 平均律等）、自定义每个音偏多少音分、Scala 律制文件。
//!
//! 只作用于有音高的内置乐器与 SFZ 采样器；鼓组保持标准音高，插件乐器不受影响。
//!
//! ```json
//! "tuning": { "scale": "werckmeister_iii" }
//! "tuning": { "scale": "just", "key": "D" }
//! "tuning": { "scale": "equal", "divisions": 24 }
//! "tuning": { "scale": "custom", "key": "D", "deviations": [0, 0, 0, -50, 0, 0, 0, 0, 0, 0, -50, 0] }
//! "tuning": { "scale": "scala", "scala": "slendro.scl" }
//! ```

use serde::{Deserialize, Serialize};

/// 可选的律制名。
pub const SCALES: &[&str] = &["equal", "pythagorean", "meantone", "werckmeister_iii", "kirnberger_iii", "vallotti", "young", "just", "custom", "scala"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    /// 律制：`equal`（平均律，配 `divisions`）、`pythagorean`、`meantone`（四分之一音差中庸全音律）、
    /// `werckmeister_iii`、`kirnberger_iii`、`vallotti`、`young`、`just`（五度纯律）、`custom`（配 `deviations`）、
    /// `scala`（配 `scala` 文件）。
    #[serde(default = "default_scale")]
    pub scale: String,
    /// 历史律制、纯律、custom 的主音（音名，如 `C`、`D`、`Bb`、`F#`），默认 C。
    #[serde(default = "default_key", skip_serializing_if = "is_c")]
    pub key: String,
    /// `equal`：一个周期分成几份，每个 MIDI 键一份。12 是普通十二平均律；19、24（四分音）、31、53 常用。
    #[serde(default = "default_divisions", skip_serializing_if = "is_12")]
    pub divisions: u32,
    /// `equal`：周期（音分），默认 1200（八度）；1901.955 是 Bohlen-Pierce 的"三度八度"。
    #[serde(default = "default_period", skip_serializing_if = "is_1200")]
    pub period_cents: f64,
    /// `custom`：十二个音（从主音起）各比十二平均律高（+）或低（-）多少音分，如阿拉伯木卡姆的 E 半降 = -50。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deviations: Option<Vec<f64>>,
    /// `scala`：`.scl` 律制文件（相对路径相对工程文件所在目录）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scala: Option<String>,
    /// `scala`：可选的 `.kbm` 键位映射文件；没有时音阶第 0 级在中央 C（60），按顺序一个键一级。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kbm: Option<String>,
    /// 参考音的频率（Hz），默认 440。巴洛克常用 415。
    #[serde(default = "default_reference_hz", skip_serializing_if = "is_440")]
    pub reference_hz: f64,
    /// 参考音的 MIDI 键，默认 69（A4）。
    #[serde(default = "default_reference_key", skip_serializing_if = "is_69")]
    pub reference_key: u8,
    /// 整体再偏多少音分，默认 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub detune_cents: f64,
}

fn default_scale() -> String {
    "equal".into()
}
fn default_key() -> String {
    "C".into()
}
fn default_divisions() -> u32 {
    12
}
fn default_period() -> f64 {
    1200.0
}
fn default_reference_hz() -> f64 {
    440.0
}
fn default_reference_key() -> u8 {
    69
}
fn is_c(k: &str) -> bool {
    k == "C"
}
fn is_12(d: &u32) -> bool {
    *d == 12
}
fn is_1200(p: &f64) -> bool {
    *p == 1200.0
}
fn is_440(p: &f64) -> bool {
    *p == 440.0
}
fn is_69(k: &u8) -> bool {
    *k == 69
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// 音名 → 音级（0 = C … 11 = B）：`C` `C#` `Db` `F#` `Bb`，大小写都行。
pub fn pitch_class(name: &str) -> Option<u8> {
    let s = name.trim();
    let mut chars = s.chars();
    let base = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let shift: i32 = chars
        .map(|c| match c {
            '#' | '♯' => Some(1),
            'b' | '♭' => Some(-1),
            _ => None,
        })
        .sum::<Option<i32>>()?;
    Some((base + shift).rem_euclid(12) as u8)
}

impl Tuning {
    /// 检查取值；文件存在与否由 engine 在加载时检查。错误信息可直接给 AI 看。
    pub fn validate(&self) -> Result<(), String> {
        let scale = self.scale.trim().to_ascii_lowercase();
        if !SCALES.contains(&scale.as_str()) {
            return Err(format!("tuning.scale = '{}' is invalid; choose one of: {}", self.scale, SCALES.join(" / ")));
        }
        if pitch_class(&self.key).is_none() {
            return Err(format!("tuning.key = '{}' is not a note name (e.g. C, D, Bb, F#)", self.key));
        }
        if !(1..=144).contains(&self.divisions) {
            return Err(format!("tuning.divisions = {} is out of range [1, 144]", self.divisions));
        }
        if !(self.period_cents.is_finite() && self.period_cents > 0.0 && self.period_cents <= 4800.0) {
            return Err(format!("tuning.period_cents = {} is out of range (0, 4800]", self.period_cents));
        }
        if !(self.reference_hz.is_finite() && (20.0..=2000.0).contains(&self.reference_hz)) {
            return Err(format!("tuning.reference_hz = {} is out of range [20, 2000]", self.reference_hz));
        }
        if self.reference_key > 127 {
            return Err(format!("tuning.reference_key = {} is out of range [0, 127]", self.reference_key));
        }
        if !(self.detune_cents.is_finite() && self.detune_cents.abs() <= 1200.0) {
            return Err(format!("tuning.detune_cents = {} is out of range [-1200, 1200]", self.detune_cents));
        }
        match scale.as_str() {
            "custom" => match &self.deviations {
                Some(d) if d.len() == 12 && d.iter().all(|x| x.is_finite() && x.abs() <= 1200.0) => {}
                Some(d) if d.len() != 12 => return Err(format!("tuning.deviations needs 12 numbers (one per semitone from the tonic), got {}", d.len())),
                Some(_) => return Err("every tuning.deviations value must be between -1200 and 1200 cents".into()),
                None => return Err("tuning.scale = 'custom' needs deviations: how many cents each of the 12 notes is offset".into()),
            },
            "scala" if self.scala.as_deref().is_none_or(|s| s.trim().is_empty()) => {
                return Err("tuning.scale = 'scala' needs scala: the path to a .scl file".into());
            }
            _ => {}
        }
        Ok(())
    }

    /// 是不是普通的十二平均律、A4 = 440（等于没写）。
    pub fn is_standard(&self) -> bool {
        self.scale.trim().eq_ignore_ascii_case("equal")
            && self.divisions == 12
            && self.period_cents == 1200.0
            && self.reference_hz == 440.0
            && self.reference_key == 69
            && self.detune_cents == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let t: Tuning = serde_json::from_str(r#"{ "scale": "just", "key": "D" }"#).unwrap();
        t.validate().unwrap();
        assert_eq!(pitch_class(&t.key), Some(2));
        assert_eq!(pitch_class("Bb"), Some(10));
        assert_eq!(pitch_class("f#"), Some(6));
        assert_eq!(pitch_class("Cb"), Some(11));
        let t: Tuning = serde_json::from_str(r#"{}"#).unwrap();
        assert!(t.is_standard());
        let bad = |j: &str| serde_json::from_str::<Tuning>(j).unwrap().validate().unwrap_err();
        assert!(bad(r#"{ "scale": "nope" }"#).contains("tuning.scale"));
        assert!(bad(r#"{ "scale": "custom" }"#).contains("deviations"));
        assert!(bad(r#"{ "scale": "custom", "deviations": [0, 1] }"#).contains("12 numbers"));
        assert!(bad(r#"{ "scale": "scala" }"#).contains(".scl"));
        assert!(bad(r#"{ "key": "H" }"#).contains("note name"));
        assert!(bad(r#"{ "divisions": 0 }"#).contains("divisions"));
    }
}
