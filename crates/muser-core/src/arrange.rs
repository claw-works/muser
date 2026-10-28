//! 编曲层：段落（`sections`）、曲式（`form`）、音轨在各段落里演奏什么（`parts`）、按和弦写的音型（`chordal`）、变换（`transform`）。
//!
//! ```json
//! "sections": { "verse": { "bars": 8, "chords": "Am7 F C G" }, "chorus": { "bars": 8, "chords": "F G | Em Am" } },
//! "form": ["verse", "chorus", "verse", { "section": "chorus", "transpose": 2 }],
//! "tracks": [{ "name": "bass", "instrument": "bass.finger", "parts": {
//!     "verse":  { "chordal": "1 . . 5 1 . 8 .", "step": 0.5, "octave": 2 },
//!     "chorus": { "chordal": "1 1 5 5", "octave": 2, "end": { "beats": 2, "abc": "L:1/8\nK:Am\nE,F,^F,G," } } } }]
//! ```
//!
//! 渲染前由引擎展开成普通 clip（`muser_engine::arrange`）：每个段落的每一遍、每条音轨一个 clip。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::grid::{Drummer, Grid};
use crate::humanize::Humanize;
use crate::Note;

/// 一个段落：多少小节、和弦进行（可省略）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub bars: u32,
    /// 和弦进行，如 `"Am7 F C G"`（每个一小节）、`"F G | Em Am"`（`|` 分小节，一小节里平分）。比段落短时循环。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chords: Option<String>,
}

/// 曲式里的一项：段落名，或带这一遍的参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormEntry {
    Name(String),
    Detail(FormDetail),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormDetail {
    pub section: String,
    /// 这一遍移调（半音；和弦一起移，鼓组不动）。
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub transpose: i32,
    /// 这一遍休息的音轨（名字）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute: Vec<String>,
    /// 这一遍的随机种子（鼓手、vary、humanize 换一种）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

impl FormEntry {
    pub fn section(&self) -> &str {
        match self {
            FormEntry::Name(n) => n,
            FormEntry::Detail(d) => &d.section,
        }
    }
    pub fn transpose(&self) -> i32 {
        match self {
            FormEntry::Name(_) => 0,
            FormEntry::Detail(d) => d.transpose,
        }
    }
    pub fn muted(&self, track: &str) -> bool {
        matches!(self, FormEntry::Detail(d) if d.mute.iter().any(|m| m == track))
    }
    pub fn seed(&self) -> Option<u64> {
        match self {
            FormEntry::Name(_) => None,
            FormEntry::Detail(d) => d.seed,
        }
    }
}

/// 音轨在一个段落里演奏的内容：clip 的写法（不写起点），加上按和弦写的音型、循环长度、结尾替换与变换。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// 循环长度（拍）：内容按这个长度重复铺满段落。缺省 = 段落长度（不循环）；chordal 缺省 = 音型长度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drummer: Option<Drummer>,
    /// 按当前和弦写的音型，见 [`Chordal`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chordal: Option<String>,
    /// chordal 每格多少拍，默认 0.5（八分音符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// chordal 的音区：根音放在第几组（C4 = 中央 C），默认 3；`c` 整个和弦以这一组为中心排转位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octave: Option<i32>,
    /// chordal 的力度，默认 0.8。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<Humanize>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legato: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pedal: Vec<(f64, f64)>,
    /// 变换（每一遍循环按顺序施加），见 [`Transform`]。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transform: Vec<Transform>,
    /// 结尾替换：段落每一遍的最后 `beats` 拍换成这里的内容（过门、转折）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Box<Part>>,
    /// 只用在 `end` 里：替换最后几拍。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats: Option<f64>,
}

/// 一个变换：每个对象只写一种。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// 移调（半音；鼓组不受影响）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i32>,
    /// 升降八度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub octave: Option<i32>,
    /// 逆行：时间倒过来。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse: bool,
    /// 倒影：以这个 MIDI 音高为轴上下翻转。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<u8>,
    /// 时值缩放（2 = 放慢一倍，0.5 = 加快一倍），循环长度一起缩放。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stretch: Option<f64>,
    /// 错位：在循环里往后挪（拍），挪出去的绕回开头。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<f64>,
    /// 模进：第 k 遍循环按 `sequence[k % 长度]` 个音阶级移位（音阶取这一段和弦里的音，不够 5 个音时用第一个和弦根音的大调）。
    /// 如 `[0, -1, -2, -3]`：动机逐级下行四遍。鼓组不受影响。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence: Vec<i32>,
    /// 裁截：只留前这么多拍（循环长度也变成这么长），常和 every / sequence 一起用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment: Option<f64>,
    /// 每第 N 遍才施加 `then`（第 N、2N… 遍）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<Transform>,
    /// 随机变奏：每个音以这个概率换成相邻的和弦音（没有和弦时换成音型里相邻的音高），少数省掉不弹；鼓组改为省音、变力度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vary: Option<f64>,
    /// vary 的种子（缺省由工程 seed、音轨、段落这一遍推导）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Transform {
    pub fn validate(&self, at: &str) -> Result<(), String> {
        let ops = [
            self.transpose.is_some(), self.octave.is_some(), self.reverse, self.invert.is_some(),
            self.stretch.is_some(), self.shift.is_some(), self.every.is_some(), self.vary.is_some(),
            !self.sequence.is_empty(), self.fragment.is_some(),
        ];
        if ops.iter().filter(|&&o| o).count() != 1 {
            return Err(format!("{at}: 每个变换对象只写一种：transpose / octave / reverse / invert / stretch / shift / sequence / fragment / every+then / vary"));
        }
        if self.every.is_some() == self.then.is_empty() {
            return Err(format!("{at}: every 与 then 要一起写，如 {{\"every\": 4, \"then\": [{{\"reverse\": true}}]}}"));
        }
        if self.every == Some(0) {
            return Err(format!("{at}.every 应 ≥ 1"));
        }
        if self.seed.is_some() && self.vary.is_none() {
            return Err(format!("{at}.seed 只用于 vary"));
        }
        if let Some(s) = self.stretch {
            if !(s > 0.0 && s <= 8.0) {
                return Err(format!("{at}.stretch = {s} 应在 (0, 8]"));
            }
        }
        if let Some(v) = self.vary {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{at}.vary = {v} 超出范围 [0, 1]"));
            }
        }
        if let Some(f) = self.fragment {
            if !(f > 0.0) {
                return Err(format!("{at}.fragment 应 > 0 拍"));
            }
        }
        if self.sequence.iter().any(|s| s.abs() > 14) {
            return Err(format!("{at}.sequence: 每一步在 ±14 个音阶级以内"));
        }
        if let Some(s) = self.shift {
            if !s.is_finite() {
                return Err(format!("{at}.shift 应为有限数"));
            }
        }
        for (i, t) in self.then.iter().enumerate() {
            t.validate(&format!("{at}.then[{i}]"))?;
        }
        Ok(())
    }
}

/// chordal 音型里的一格。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Rest,
    Hold,
    /// 和弦的第几个音：`deg` 1 2 3 4 5 6 7 8 9 11 13，`acc` 显式升降（`b7`、`#11`），`oct` 八度偏移（`'` / `,`）。
    Degree { deg: u8, acc: i8, oct: i8, accent: bool },
    /// 整个和弦（自动转位）。
    Chord { oct: i8, accent: bool },
    /// 低音：斜线和弦的低音，否则根音。
    Bass { oct: i8, accent: bool },
}

/// 解析好的 chordal 音型。
pub struct Chordal;

impl Chordal {
    /// 解析音型：空格分隔的格子，`|` 只为好看。见 [`Step`]。
    pub fn parse(text: &str) -> Result<Vec<Step>, String> {
        let mut out = Vec::new();
        for tok in text.split_whitespace().filter(|t| *t != "|") {
            let err = || format!("chordal 的 '{tok}' 看不懂：1 3 5 7 8 9（和弦的第几个音，可加 b/# 与 ' ,）、c（整个和弦）、B（低音）、.（休止）、-（延长），末尾 ! 为重音");
            let (body, accent) = match tok.strip_suffix('!') {
                Some(b) => (b, true),
                None => (tok, false),
            };
            let oct = body.chars().filter(|&c| c == '\'').count() as i8 - body.chars().filter(|&c| c == ',').count() as i8;
            let core = body.trim_end_matches(['\'', ',']);
            let step = match core {
                "." => Step::Rest,
                "-" => Step::Hold,
                "c" => Step::Chord { oct, accent },
                "B" => Step::Bass { oct, accent },
                _ => {
                    let (acc, digits) = match core.chars().next() {
                        Some('b') => (-1, &core[1..]),
                        Some('#') => (1, &core[1..]),
                        _ => (0, core),
                    };
                    let deg: u8 = digits.parse().map_err(|_| err())?;
                    if ![1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13].contains(&deg) {
                        return Err(err());
                    }
                    Step::Degree { deg, acc, oct, accent }
                }
            };
            if matches!(step, Step::Rest | Step::Hold) && (oct != 0 || accent) {
                return Err(err());
            }
            out.push(step);
        }
        if out.is_empty() {
            return Err("chordal 为空".into());
        }
        Ok(out)
    }
}

impl Part {
    /// 校验写法（和弦是否存在等要看段落，由展开时检查）。
    pub fn validate(&self, at: &str, in_end: bool) -> Result<(), String> {
        if let Some(l) = self.length {
            if !(l > 0.0) {
                return Err(format!("{at}.length 应 > 0"));
            }
        }
        if let Some(s) = self.step {
            if !(s > 0.0 && s <= 16.0) {
                return Err(format!("{at}.step 应在 (0, 16] 拍"));
            }
        }
        if let Some(c) = &self.chordal {
            Chordal::parse(c).map_err(|e| format!("{at}.{e}"))?;
        } else if self.step.is_some() || self.octave.is_some() {
            return Err(format!("{at}: step / octave 只用于 chordal"));
        }
        if let Some(v) = self.velocity {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{at}.velocity 超出范围 [0, 1]"));
            }
        }
        if let Some(a) = &self.abc {
            crate::abc::parse(a).map_err(|e| format!("{at}.abc: {e}"))?;
        }
        if let Some(h) = &self.humanize {
            h.validate(&format!("{at}.humanize"))?;
        }
        if let Some(d) = &self.drummer {
            d.validate(&format!("{at}.drummer"))?;
        }
        for (i, t) in self.transform.iter().enumerate() {
            t.validate(&format!("{at}.transform[{i}]"))?;
        }
        match (in_end, self.beats) {
            (true, None) => return Err(format!("{at}: end 要写 beats（替换最后几拍）")),
            (true, Some(b)) if !(b > 0.0) => return Err(format!("{at}.beats 应 > 0")),
            (false, Some(_)) => return Err(format!("{at}.beats 只用在 end 里")),
            _ => {}
        }
        if let Some(e) = &self.end {
            if in_end {
                return Err(format!("{at}: end 里不能再写 end"));
            }
            e.validate(&format!("{at}.end"), true)?;
        }
        if self.notes.is_empty() && self.abc.is_none() && self.grid.is_none() && self.drummer.is_none() && self.chordal.is_none() {
            return Err(format!("{at}: 没有内容（notes / abc / grid / drummer / chordal 至少写一种）"));
        }
        Ok(())
    }
}

/// `parts` 的键：`"verse"`、`"verse#2"`（第 2 次 verse）、`"*"`（没单独写的段落）。返回 (段落名, 第几次)。
pub fn part_key(key: &str) -> (&str, Option<usize>) {
    match key.rsplit_once('#') {
        Some((name, n)) => match n.parse::<usize>() {
            Ok(k) => (name, Some(k)),
            Err(_) => (key, None),
        },
        None => (key, None),
    }
}

/// 一条音轨在第 `occurrence` 次（从 1 起）`section` 里用哪个 part：`name#k` > `name` > `*`。
pub fn pick<'a>(parts: &'a BTreeMap<String, Part>, section: &str, occurrence: usize) -> Option<&'a Part> {
    parts.get(&format!("{section}#{occurrence}")).or_else(|| parts.get(section)).or_else(|| parts.get("*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chordal_tokens() {
        let s = Chordal::parse("1 . b7 3' 5, c! - B |").unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s[2], Step::Degree { deg: 7, acc: -1, oct: 0, accent: false });
        assert_eq!(s[3], Step::Degree { deg: 3, acc: 0, oct: 1, accent: false });
        assert_eq!(s[5], Step::Chord { oct: 0, accent: true });
        assert!(Chordal::parse("10").is_err() && Chordal::parse(".!").is_err() && Chordal::parse("x").is_err());
    }

    #[test]
    fn keys_and_transforms() {
        assert_eq!(part_key("verse#2"), ("verse", Some(2)));
        assert_eq!(part_key("*"), ("*", None));
        let t: Transform = serde_json::from_str(r#"{ "every": 4, "then": [{ "reverse": true }] }"#).unwrap();
        assert!(t.validate("t").is_ok());
        let bad: Transform = serde_json::from_str(r#"{ "transpose": 2, "reverse": true }"#).unwrap();
        assert!(bad.validate("t").unwrap_err().contains("只写一种"));
        assert!(serde_json::from_str::<Transform>(r#"{ "speed": 2 }"#).is_err());
    }
}
