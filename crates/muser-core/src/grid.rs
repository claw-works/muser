//! 步进网格（grid）→ 音符：一行字符一个鼓件（或一个音高），一个字符一步。给鼓、贝斯线这类循环节奏用。
//!
//! ```json
//! "grid": { "step": 0.25, "kick": "x...x...x...x...", "snare": "....x.......x...", "hat": "x.x.x.x.x.x.x.x." }
//! ```
//!
//! - `step`：每个字符多少拍，默认 0.25（十六分音符）。
//! - 其余键是一行：鼓件名（`kick` `snare` `hat` `open_hat` `clap` `rim` `crash` `ride` `tom_low` …，见 [`DRUM_PIECES`]）、
//!   MIDI 音高数字（`"36"`），或音名（`"c2"` `"eb3"` `"f#1"`，C4 = 60）。
//! - 字符：`x` 敲击（力度 0.8）、`X` 重音（1.0）、`o` 轻（0.45，鬼音）、`1`–`9` 力度 0.1–0.9；
//!   `=` 或 `~` 延长上一个音一步（贝斯线的长音）；`.` `-` 休止；空格、`|` 只为好看，忽略。
//! - 每一行按自己的长度在整个 clip 里循环（3 步的行在 16 步里会错开，可以做复合节奏）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Beats, Note};

/// GM 鼓键的别名（音轨乐器为 `drums.*` 时的鼓件）。
pub const DRUM_PIECES: &[(&str, u8)] = &[
    ("kick", 36), ("kick2", 35), ("side_stick", 37), ("rim", 37), ("snare", 38), ("clap", 39),
    ("snare2", 40), ("floor_tom_low", 41), ("closed_hat", 42), ("hat", 42), ("floor_tom", 43),
    ("pedal_hat", 44), ("tom_low", 45), ("open_hat", 46), ("tom_mid_low", 47), ("tom_mid", 48),
    ("crash", 49), ("tom_high", 50), ("ride", 51), ("china", 52), ("ride_bell", 53),
    ("tambourine", 54), ("splash", 55), ("cowbell", 56), ("crash2", 57), ("ride2", 59),
    ("bongo_high", 60), ("bongo_low", 61), ("conga_mute", 62), ("conga_high", 63), ("conga_low", 64),
    ("cabasa", 69), ("shaker", 70), ("maracas", 70), ("guiro_short", 73), ("guiro_long", 74), ("claves", 75),
];

/// 鼓件名或 MIDI 数字 → GM 鼓键。
pub fn drum_key(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    n.parse::<u8>().ok().filter(|k| *k <= 127).or_else(|| DRUM_PIECES.iter().find(|(a, _)| *a == n).map(|(_, k)| *k))
}

/// 音名（`c4` = 60、`eb3`、`f#1`）→ MIDI 音高。
pub fn note_name(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    let mut c = n.chars();
    let base = match c.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = c.collect();
    let (acc, oct) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') if rest.len() > 1 => (-1, &rest[1..]),
        _ => (0, rest.as_str()),
    };
    let octave: i32 = oct.parse().ok()?;
    u8::try_from(12 * (octave + 1) + base + acc).ok().filter(|p| *p <= 127)
}

/// 一个网格：步长与各行。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    /// 每个字符多少拍（默认 0.25）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// 行名 → 字符。
    #[serde(flatten)]
    pub rows: BTreeMap<String, String>,
}

impl Grid {
    /// 在 `length` 拍里展开成音符（相对 clip 起点）。
    pub fn expand(&self, length: f64) -> Result<Vec<Note>, String> {
        let step = self.step.unwrap_or(0.25);
        if !(step > 0.0 && step <= 4.0) {
            return Err(format!("grid.step = {step} 应在 (0, 4] 拍"));
        }
        if self.rows.is_empty() {
            return Err("grid 里没有行（如 \"kick\": \"x...x...\"）".into());
        }
        let mut notes = Vec::new();
        for (key, row) in &self.rows {
            let pitch = drum_key(key).or_else(|| note_name(key)).ok_or_else(|| {
                format!(
                    "grid.{key}: 不是鼓件名、MIDI 数字或音名（如 c2）。鼓件: {}",
                    DRUM_PIECES.iter().map(|(a, _)| *a).collect::<Vec<_>>().join(" ")
                )
            })?;
            let cells: Vec<char> = row.chars().filter(|c| !matches!(c, ' ' | '|')).collect();
            if cells.is_empty() {
                continue;
            }
            if let Some(bad) = cells.iter().find(|c| !matches!(c, 'x' | 'X' | 'o' | '1'..='9' | '=' | '~' | '.' | '-')) {
                return Err(format!("grid.{key}: 无法识别的字符 '{bad}'（x X o 1-9 = ~ . -）"));
            }
            let steps = (length / step - 1e-9).ceil().max(0.0) as usize;
            let mut last: Option<usize> = None;
            for k in 0..steps {
                let c = cells[k % cells.len()];
                let velocity = match c {
                    'x' => 0.8,
                    'X' => 1.0,
                    'o' => 0.45,
                    '1'..='9' => f32::from(c as u8 - b'0') / 10.0,
                    '=' | '~' => {
                        if let Some(i) = last {
                            let n: &mut Note = &mut notes[i];
                            n.length = Beats(n.length.0 + step);
                        }
                        continue;
                    }
                    _ => {
                        last = None;
                        continue;
                    }
                };
                notes.push(Note { pitch, start: Beats(k as f64 * step), length: Beats(step), velocity, accent: c == 'X' });
                last = Some(notes.len() - 1);
            }
        }
        notes.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
        Ok(notes)
    }
}

/// 生成式鼓手：以 loop 库里的一个节奏型为底，按复杂度、力度加变化，每隔几小节打一个过门。
/// 每小节都不完全一样；同一个种子结果相同，换种子换一种打法。由引擎展开（要读 loop 库）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drummer {
    /// 底子：loop 名（`muser loop list`）。
    #[serde(rename = "loop")]
    pub base: String,
    /// 复杂度 0–1：0 照着 loop 打；越高鬼音、加花、切分越多。
    #[serde(default = "half")]
    pub complexity: f64,
    /// 力度 0–1：整体打多重（0.5 适中）。
    #[serde(default = "half")]
    pub intensity: f64,
    /// 每几小节一个过门（在这一组的最后一小节末尾），0 不打。
    #[serde(default = "four")]
    pub fill_every: u32,
    /// 过门之后的下一小节第一拍加镲（crash）。
    #[serde(default = "yes")]
    pub crash: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

fn half() -> f64 {
    0.5
}
fn four() -> u32 {
    4
}
fn yes() -> bool {
    true
}

impl Drummer {
    pub fn validate(&self, at: &str) -> Result<(), String> {
        for (name, v) in [("complexity", self.complexity), ("intensity", self.intensity)] {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{at}.{name} = {v} 超出范围 [0, 1]"));
            }
        }
        if self.fill_every > 64 {
            return Err(format!("{at}.fill_every = {} 超出范围 [0, 64]", self.fill_every));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[(&str, &str)]) -> Grid {
        Grid { step: None, rows: rows.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn four_on_the_floor_tiles_the_clip() {
        let n = grid(&[("kick", "x..."), ("hat", "..x.")]).expand(4.0).unwrap();
        let kicks: Vec<f64> = n.iter().filter(|n| n.pitch == 36).map(|n| n.start.0).collect();
        assert_eq!(kicks, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(n.iter().filter(|n| n.pitch == 42).count(), 4);
    }

    #[test]
    fn velocities_accents_and_sustain() {
        let n = grid(&[("c2", "X=o.5~~")]).expand(1.75).unwrap();
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].pitch, n[0].velocity, n[0].accent, n[0].length.0), (36, 1.0, true, 0.5));
        assert_eq!(n[1].velocity, 0.45);
        assert_eq!((n[2].velocity, n[2].length.0), (0.5, 0.75));
    }

    #[test]
    fn names_and_errors() {
        assert_eq!(note_name("c4"), Some(60));
        assert_eq!(note_name("eb3"), Some(51));
        assert_eq!(note_name("f#1"), Some(30));
        assert_eq!(note_name("b"), None);
        assert_eq!(drum_key("Open_Hat"), Some(46));
        assert_eq!(drum_key("40"), Some(40));
        assert!(grid(&[("kik", "x")]).expand(1.0).unwrap_err().contains("grid.kik"));
        assert!(grid(&[("kick", "x?")]).expand(1.0).unwrap_err().contains("'?'"));
        // 空格与小节线只为好看。
        assert_eq!(grid(&[("kick", "x... | x...")]).expand(2.0).unwrap().len(), 2);
    }

    #[test]
    fn json_shape() {
        let g: Grid = serde_json::from_str(r#"{ "step": 0.5, "kick": "x.x." }"#).unwrap();
        assert_eq!((g.step, g.rows["kick"].as_str()), (Some(0.5), "x.x."));
    }
}
