//! ABC 记谱 → 音符。给 agent 快速写旋律、和声用（一行文字代替一长串音符 JSON）。
//!
//! 支持（单声部；多声部请分到不同音轨的 clip）：
//! - 头部：`L:` 单位时值（默认 1/8）、`M:` 拍号（`Z` 整小节休止用）、`K:` 调号（含调式：`Am`、`Dmix`、`Edor`、`F#m`、`Bb`…），
//!   `X:` `T:` `Q:` 等其它字段忽略（速度由工程决定）。
//! - 音高：`C` = 中央 C（MIDI 60），`c` 高八度；`'` 升八度、`,` 降八度；`^` 升、`_` 降、`=` 还原（`^^` `__` 重升重降）。
//!   临时记号在本小节内持续，调号按 `K:` 自动生效。
//! - 时值：`A2`、`A/2`、`A/`、`A3/2`、`A//`；休止 `z` / `x`，整小节休止 `Z`（`Z4` 四小节）。
//! - 和弦 `[CEG]2`，连音线 `A2-A`，附点节奏 `A>B` / `A<B`，连音 `(3abc`（`(2`–`(9`）。
//! - 反复 `|: … :|`，一二房 `[1 … :|[2 …`（或 `|1` / `:|2`）。
//! - 力度 `!ppp!` … `!fff!`（之后的音都用这个力度），重音 `!>!` / `!accent!`，断奏 `.`（时值减半）。
//! - 和弦标记 `"Am"`、装饰音 `{…}`、其它装饰 `!…!` `+…+`、`~` `H` `L` `M` `O` `P` `S` `T` `u` `v`、注释 `%` 忽略。

use crate::{Beats, Note};

/// 解析出的音符（拍，相对 clip 起点）。
pub fn parse(text: &str) -> Result<Vec<Note>, String> {
    Parser::new(text)?.run()
}

/// 调号：每个音名（C..B）的默认升降（半音）。
fn key_signature(spec: &str) -> Result<[i32; 7], String> {
    let s = spec.split_whitespace().next().unwrap_or("C");
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok([0; 7]);
    }
    let mut chars = s.chars();
    let tonic = chars.next().unwrap().to_ascii_uppercase();
    let mut rest: String = chars.collect();
    let mut tonic_acc = 0;
    if rest.starts_with('#') {
        tonic_acc = 1;
        rest.remove(0);
    } else if rest.starts_with('b') {
        tonic_acc = -1;
        rest.remove(0);
    }
    let mode = rest.to_ascii_lowercase();
    // 调式相对大调的主音偏移（五度圈上的位置）。
    let mode_shift = match mode.as_str() {
        "" | "maj" | "major" | "ion" | "ionian" => 0,
        "m" | "min" | "minor" | "aeo" | "aeolian" => -3,
        "mix" | "mixolydian" => -1,
        "dor" | "dorian" => -2,
        "phr" | "phrygian" => -4,
        "lyd" | "lydian" => 1,
        "loc" | "locrian" => -5,
        _ => return Err(format!("K: 无法识别的调式 '{s}'")),
    };
    // 主音在五度圈上的位置（C = 0，G = 1，F = -1 …）。
    let base = match tonic {
        'C' => 0,
        'G' => 1,
        'D' => 2,
        'A' => 3,
        'E' => 4,
        'B' => 5,
        'F' => -1,
        _ => return Err(format!("K: 无法识别的主音 '{s}'")),
    };
    let fifths = base + 7 * tonic_acc + mode_shift;
    if !(-7..=7).contains(&fifths) {
        return Err(format!("K: 调号 '{s}' 超出范围（最多 7 个升降号）"));
    }
    // 升号顺序 F C G D A E B，降号顺序 B E A D G C F；音名下标 C=0 … B=6。
    let mut sig = [0; 7];
    if fifths > 0 {
        for &i in &[3, 0, 4, 1, 5, 2, 6][..fifths as usize] {
            sig[i] = 1;
        }
    } else {
        for &i in &[6, 2, 5, 1, 4, 0, 3][..(-fifths) as usize] {
            sig[i] = -1;
        }
    }
    Ok(sig)
}

/// `1/8`、`3/4`、`C`（4/4）、`C|`（2/2）→ 全音符的分数。
fn fraction(s: &str) -> Option<f64> {
    match s.trim() {
        "C" => return Some(1.0),
        "C|" => return Some(1.0),
        _ => {}
    }
    let (a, b) = s.trim().split_once('/')?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0.0 && b > 0.0).then_some(a / b)
}

struct Parser<'a> {
    body: Vec<(usize, &'a str)>,
    /// 单位时值（拍）。
    unit: f64,
    /// 一小节多少拍。
    bar: f64,
    key: [i32; 7],
}

/// 一个待放的音（和弦里的音共享起点）。
struct Pending {
    pitches: Vec<i32>,
    length: f64,
    tie: bool,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self, String> {
        let (mut unit, mut bar, mut key) = (0.5, 4.0, [0; 7]);
        let mut body = Vec::new();
        for (ln, raw) in text.lines().enumerate() {
            let line = raw.split('%').next().unwrap_or("").trim_end();
            let header = line.len() >= 2 && line.as_bytes()[1] == b':' && line.as_bytes()[0].is_ascii_alphabetic() && !line.starts_with('|');
            if header {
                let (field, value) = (line.as_bytes()[0] as char, line[2..].trim());
                match field {
                    'L' => unit = fraction(value).ok_or_else(|| format!("第 {} 行 L: 应为分数，如 1/8", ln + 1))? * 4.0,
                    'M' => bar = fraction(value).map_or(bar, |f| f * 4.0),
                    'K' => {
                        key = key_signature(value).map_err(|e| format!("第 {} 行 {e}", ln + 1))?;
                    }
                    'V' => return Err(format!("第 {} 行：不支持多声部 V:，每个声部写进各自音轨的 clip", ln + 1)),
                    _ => {}
                }
                continue;
            }
            if !line.trim().is_empty() {
                body.push((ln + 1, line));
            }
        }
        Ok(Parser { body, unit, bar, key })
    }

    fn run(&self) -> Result<Vec<Note>, String> {
        // 先把整段展开成"小节流"处理反复：记下每个 token 的位置，遇到 :| 回到 |: 处再走一遍。
        let tokens = self.tokens()?;
        let order = repeat_order(&tokens);
        let mut notes: Vec<Note> = Vec::new();
        let mut t = 0.0;
        let mut accidentals: [Option<i32>; 128] = [None; 128];
        let mut velocity = 0.8f32;
        let mut accent = false;
        let mut staccato = false;
        let mut tuplet: Option<(f64, usize)> = None;
        let mut broken: Option<f64> = None;
        // 被连音线连起来、等着下一个同音高延长的音（notes 里的下标）。
        let mut tied: Vec<usize> = Vec::new();
        for &i in &order {
            let (line, tok) = &tokens[i];
            let at = |m: &str| format!("第 {line} 行：{m}");
            match tok {
                Tok::Bar => accidentals = [None; 128],
                Tok::RepeatStart | Tok::RepeatEnd | Tok::Ending(_) => accidentals = [None; 128],
                Tok::Dynamic(v) => velocity = *v,
                Tok::Accent => accent = true,
                Tok::Staccato => staccato = true,
                Tok::Tuplet(p) => {
                    let q = match p {
                        2 | 4 | 8 => 3.0,
                        3 | 6 => 2.0,
                        _ => {
                            if (self.bar / 4.0 * 8.0).round() as i64 % 3 == 0 {
                                3.0
                            } else {
                                2.0
                            }
                        }
                    };
                    tuplet = Some((q / f64::from(*p), *p as usize));
                }
                Tok::Broken(f) => broken = Some(*f),
                Tok::BarRest(n) => {
                    t += self.bar * f64::from(*n);
                    tied.clear();
                }
                Tok::Rest(len) => {
                    let mut l = len * self.unit;
                    self.scale(&mut l, &mut tuplet, &mut broken);
                    t += l;
                    tied.clear();
                }
                Tok::Notes(p) => {
                    let mut l = p.length * self.unit;
                    self.scale(&mut l, &mut tuplet, &mut broken);
                    let mut next_tied = Vec::new();
                    for &spec in &p.pitches {
                        let pitch = self.pitch(spec, &mut accidentals).ok_or_else(|| at("音高超出 MIDI 范围 0–127"))?;
                        // 连音线：前面同音高的音延长，不新起一个。
                        if let Some(pos) = tied.iter().position(|&j| notes[j].pitch == pitch) {
                            let j = tied.remove(pos);
                            notes[j].length = Beats(notes[j].length.0 + l);
                            if p.tie {
                                next_tied.push(j);
                            }
                            continue;
                        }
                        let length = if staccato { l * 0.5 } else { l };
                        notes.push(Note { pitch, start: Beats(t), length: Beats(length), velocity, accent });
                        if p.tie {
                            next_tied.push(notes.len() - 1);
                        }
                    }
                    tied = next_tied;
                    t += l;
                    accent = false;
                    staccato = false;
                }
            }
        }
        if notes.is_empty() && order.is_empty() {
            return Err("ABC 里没有音符（K: 之后写旋律，如 \"L:1/8\\nK:Am\\nA2 cB e2 dc |\"）".into());
        }
        Ok(notes)
    }

    /// 连音（tuplet）与附点节奏（broken rhythm）对时值的影响。
    fn scale(&self, l: &mut f64, tuplet: &mut Option<(f64, usize)>, broken: &mut Option<f64>) {
        if let Some((ratio, left)) = tuplet {
            *l *= *ratio;
            *left -= 1;
            if *left == 0 {
                *tuplet = None;
            }
        }
        if let Some(f) = broken.take() {
            *l *= f;
        }
    }

    /// 音名 + 记号 → MIDI 音高。`spec` 编码见 [`note_spec`]。
    fn pitch(&self, spec: i32, accidentals: &mut [Option<i32>; 128]) -> Option<u8> {
        let (letter, octave, acc) = (spec & 0xF, spec >> 8, (spec >> 4 & 0xF) - 8);
        const SEMI: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
        let natural = 60 + 12 * octave + SEMI[letter as usize];
        let slot = (natural.clamp(0, 127)) as usize;
        let shift = if acc != NO_ACC {
            accidentals[slot] = Some(acc);
            acc
        } else {
            accidentals[slot].unwrap_or(self.key[letter as usize])
        };
        u8::try_from(natural + shift).ok().filter(|p| *p <= 127)
    }

    fn tokens(&self) -> Result<Vec<(usize, Tok)>, String> {
        let mut out = Vec::new();
        for &(line, text) in &self.body {
            let b = text.as_bytes();
            let mut i = 0;
            let at = |i: usize, m: &str| format!("第 {line} 行第 {} 列：{m}", i + 1);
            while i < b.len() {
                let c = b[i] as char;
                match c {
                    ' ' | '\t' | '\\' | '`' => i += 1,
                    '"' => {
                        // 和弦标记 / 注释文字：跳过。
                        let end = text[i + 1..].find('"').ok_or_else(|| at(i, "引号没有闭合"))?;
                        i += end + 2;
                    }
                    '{' => {
                        let end = text[i..].find('}').ok_or_else(|| at(i, "装饰音 { 没有闭合"))?;
                        i += end + 1;
                    }
                    '!' | '+' => {
                        let end = text[i + 1..].find(c).ok_or_else(|| at(i, "装饰记号没有闭合"))?;
                        let name = &text[i + 1..i + 1 + end];
                        if let Some(v) = dynamic(name) {
                            out.push((line, Tok::Dynamic(v)));
                        } else if matches!(name, ">" | "accent" | "emphasis") {
                            out.push((line, Tok::Accent));
                        }
                        i += end + 2;
                    }
                    '.' => {
                        out.push((line, Tok::Staccato));
                        i += 1;
                    }
                    '~' | 'H' | 'L' | 'M' | 'O' | 'P' | 'S' | 'T' | 'u' | 'v' => i += 1,
                    '(' => {
                        let d = b.get(i + 1).copied().unwrap_or(b' ');
                        if d.is_ascii_digit() && (b'2'..=b'9').contains(&d) {
                            out.push((line, Tok::Tuplet(d - b'0')));
                            i += 2;
                            // (p:q:r 的 q、r 忽略（按常用默认）。
                            while i < b.len() && (b[i] == b':' || b[i].is_ascii_digit()) {
                                i += 1;
                            }
                        } else {
                            i += 1; // 连线（slur）开始：忽略
                        }
                    }
                    ')' => i += 1,
                    '>' | '<' => {
                        let mut n = 0;
                        while i < b.len() && b[i] == c as u8 {
                            n += 1;
                            i += 1;
                        }
                        // a>b：前一个音乘 (2 - 1/2^n)，后一个乘 1/2^n；a<b 相反。
                        let small = 0.5f64.powi(n);
                        let (prev, next) = if c == '>' { (2.0 - small, small) } else { (small, 2.0 - small) };
                        match out.iter_mut().rev().find(|(_, t)| matches!(t, Tok::Notes(_) | Tok::Rest(_))) {
                            Some((_, Tok::Notes(p))) => p.length *= prev,
                            Some((_, Tok::Rest(l))) => *l *= prev,
                            _ => return Err(at(i, "附点节奏 > / < 前面要有音符")),
                        }
                        out.push((line, Tok::Broken(next)));
                    }
                    '|' | ':' | '[' if self.bar_token(text, i).is_some() => {
                        let (tok, len) = self.bar_token(text, i).unwrap();
                        out.extend(tok.into_iter().map(|t| (line, t)));
                        i += len;
                    }
                    '[' => {
                        // 和弦 [CEG]2
                        let end = text[i..].find(']').ok_or_else(|| at(i, "和弦 [ 没有闭合"))?;
                        let inner = &text[i + 1..i + end];
                        let mut pitches = Vec::new();
                        let mut j = 0;
                        let ib = inner.as_bytes();
                        let mut inner_len = None;
                        while j < ib.len() {
                            if ib[j] == b' ' {
                                j += 1;
                                continue;
                            }
                            let (spec, l, n) = note_spec(inner, j).ok_or_else(|| at(i + 1 + j, "和弦里应为音符"))?;
                            pitches.push(spec);
                            inner_len.get_or_insert(l);
                            j += n;
                        }
                        if pitches.is_empty() {
                            return Err(at(i, "空和弦 []"));
                        }
                        i += end + 1;
                        let (mult, n) = length(text, i);
                        i += n;
                        let tie = b.get(i) == Some(&b'-');
                        if tie {
                            i += 1;
                        }
                        out.push((line, Tok::Notes(Pending { pitches, length: inner_len.unwrap_or(1.0) * mult, tie })));
                    }
                    'z' | 'x' => {
                        let (l, n) = length(text, i + 1);
                        out.push((line, Tok::Rest(l)));
                        i += 1 + n;
                    }
                    'Z' => {
                        let mut j = i + 1;
                        while j < b.len() && b[j].is_ascii_digit() {
                            j += 1;
                        }
                        out.push((line, Tok::BarRest(text[i + 1..j].parse().unwrap_or(1))));
                        i = j;
                    }
                    _ => {
                        let (spec, l, n) = note_spec(text, i).ok_or_else(|| at(i, &format!("无法识别的 '{c}'")))?;
                        i += n;
                        let tie = b.get(i) == Some(&b'-');
                        if tie {
                            i += 1;
                        }
                        out.push((line, Tok::Notes(Pending { pitches: vec![spec], length: l, tie })));
                    }
                }
            }
        }
        Ok(out)
    }

    /// 小节线、反复、一二房：返回 (token, 占用字符数)。
    fn bar_token(&self, text: &str, i: usize) -> Option<(Vec<Tok>, usize)> {
        let rest = &text[i..];
        for (pat, toks) in [
            ("::", vec![Tok::RepeatEnd, Tok::RepeatStart]),
            (":|:", vec![Tok::RepeatEnd, Tok::RepeatStart]),
            (":||:", vec![Tok::RepeatEnd, Tok::RepeatStart]),
            (":|]", vec![Tok::RepeatEnd]),
            (":||", vec![Tok::RepeatEnd]),
            (":|", vec![Tok::RepeatEnd]),
            ("|:", vec![Tok::Bar, Tok::RepeatStart]),
            ("[|", vec![Tok::Bar]),
            ("|]", vec![Tok::Bar]),
            ("||", vec![Tok::Bar]),
            ("|", vec![Tok::Bar]),
        ] {
            if let Some(after) = rest.strip_prefix(pat) {
                // 紧跟的一二房：|1、:|2、[1
                let d = after.as_bytes().first().copied();
                if let Some(d @ b'1'..=b'9') = d {
                    let mut toks = toks;
                    toks.push(Tok::Ending(d - b'0'));
                    return Some((toks, pat.len() + 1));
                }
                return Some((toks, pat.len()));
            }
        }
        if let Some(after) = rest.strip_prefix('[') {
            if let Some(d @ b'1'..=b'9') = after.as_bytes().first().copied() {
                return Some((vec![Tok::Ending(d - b'0')], 2));
            }
        }
        None
    }
}

const NO_ACC: i32 = 7;

/// 一个音符：返回 (编码, 时值倍数, 占用字符数)。编码：低 4 位音名（C=0…B=6），4–7 位临时记号 + 8，其余八度。
fn note_spec(text: &str, i: usize) -> Option<(i32, f64, usize)> {
    let b = text.as_bytes();
    let mut j = i;
    let mut acc = NO_ACC;
    let mut acc_set = false;
    while j < b.len() && matches!(b[j], b'^' | b'_' | b'=') {
        let d = match b[j] {
            b'^' => 1,
            b'_' => -1,
            _ => 0,
        };
        acc = if acc_set { acc + d } else { d };
        acc_set = true;
        j += 1;
    }
    let c = *b.get(j)? as char;
    let letter = "CDEFGAB".find(c.to_ascii_uppercase())? as i32;
    let mut octave = if c.is_ascii_lowercase() { 1 } else { 0 };
    j += 1;
    while j < b.len() && matches!(b[j], b'\'' | b',') {
        octave += if b[j] == b'\'' { 1 } else { -1 };
        j += 1;
    }
    let (l, n) = length(text, j);
    Some((letter | ((acc + 8) << 4) | (octave << 8), l, j + n - i))
}

/// 时值倍数：`2`、`/2`、`/`、`3/2`、`//`。返回 (倍数, 占用字符数)。
fn length(text: &str, i: usize) -> (f64, usize) {
    let b = text.as_bytes();
    let mut j = i;
    let num_start = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let num: f64 = if j > num_start { text[num_start..j].parse().unwrap_or(1.0) } else { 1.0 };
    let mut den = 1.0;
    while j < b.len() && b[j] == b'/' {
        j += 1;
        let ds = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        den *= if j > ds { text[ds..j].parse().unwrap_or(2.0) } else { 2.0 };
    }
    (num / den, j - i)
}

fn dynamic(name: &str) -> Option<f32> {
    Some(match name {
        "pppp" => 0.15,
        "ppp" => 0.22,
        "pp" => 0.32,
        "p" => 0.45,
        "mp" => 0.58,
        "mf" => 0.7,
        "f" => 0.82,
        "ff" => 0.92,
        "fff" | "ffff" => 1.0,
        _ => return None,
    })
}

#[derive(Debug)]
enum Tok {
    Notes(Pending),
    Rest(f64),
    BarRest(u32),
    Bar,
    RepeatStart,
    RepeatEnd,
    Ending(u8),
    Tuplet(u8),
    Broken(f64),
    Dynamic(f32),
    Accent,
    Staccato,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}x{}", self.pitches, self.length)
    }
}

/// 按反复记号展开 token 的演奏顺序（`|: A :|` 奏两遍；`[1` 只在第一遍、`[2` 只在第二遍）。
fn repeat_order(tokens: &[(usize, Tok)]) -> Vec<usize> {
    let mut order = Vec::new();
    let (mut i, mut start, mut pass) = (0, 0, 1u8);
    let mut ending: Option<u8> = None;
    // 每个 :| 只回跳一次。
    let mut jumped = std::collections::HashSet::new();
    while i < tokens.len() {
        match &tokens[i].1 {
            Tok::RepeatStart => {
                (start, pass, ending) = (i, 1, None);
            }
            Tok::Ending(n) => ending = Some(*n),
            Tok::RepeatEnd if !jumped.contains(&i) && ending.is_none_or(|e| e == pass) => {
                jumped.insert(i);
                order.push(i);
                (pass, ending, i) = (pass + 1, None, start);
                // 回到 |: 之后接着奏（不重置第几遍）。
                if matches!(tokens[i].1, Tok::RepeatStart) {
                    order.push(i);
                    i += 1;
                }
                continue;
            }
            Tok::RepeatEnd => ending = None,
            _ => {}
        }
        // 一二房里不属于这一遍的音跳过。
        let skip = matches!(ending, Some(e) if e != pass) && matches!(tokens[i].1, Tok::Notes(_) | Tok::Rest(_) | Tok::BarRest(_));
        if !skip {
            order.push(i);
        }
        i += 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pitches(abc: &str) -> Vec<u8> {
        parse(abc).unwrap().iter().map(|n| n.pitch).collect()
    }

    fn starts(abc: &str) -> Vec<f64> {
        parse(abc).unwrap().iter().map(|n| n.start.0).collect()
    }

    #[test]
    fn pitches_octaves_accidentals() {
        assert_eq!(pitches("K:C\nC D E F G A B c"), [60, 62, 64, 65, 67, 69, 71, 72]);
        assert_eq!(pitches("K:C\nC, c' ^C _D =E"), [48, 84, 61, 61, 64]);
        // 临时记号持续到小节线。
        assert_eq!(pitches("K:C\n^F F | F"), [66, 66, 65]);
    }

    #[test]
    fn key_signatures_and_modes() {
        assert_eq!(pitches("K:G\nF"), [66]);
        assert_eq!(pitches("K:Bb\nB E"), [70, 63]);
        assert_eq!(pitches("K:Am\nG A B c"), [67, 69, 71, 72], "A 小调没有升降号");
        assert_eq!(pitches("K:Dmix\nC F"), [60, 66], "D 混合利底亚 = G 大调");
        assert_eq!(pitches("K:F#m\nF C G"), [66, 61, 68], "F# 小调 = A 大调：F# C# G#");
        assert_eq!(pitches("K:G\n=F"), [65]);
        assert!(parse("K:Hm\nA").is_err());
    }

    #[test]
    fn lengths_rests_units() {
        let n = parse("L:1/8\nK:C\nA2 B/2 z c3/2 d//").unwrap();
        let l: Vec<f64> = n.iter().map(|n| n.length.0).collect();
        assert_eq!(l, [1.0, 0.25, 0.75, 0.125]);
        assert_eq!(n.iter().map(|n| n.start.0).collect::<Vec<_>>(), [0.0, 1.0, 1.75, 2.5]);
        assert_eq!(starts("L:1/4\nK:C\nC D"), [0.0, 1.0]);
        assert_eq!(starts("M:3/4\nL:1/4\nK:C\nZ2 C"), [6.0], "Z2 = 两小节休止");
    }

    #[test]
    fn chords_ties_broken_tuplets() {
        let n = parse("L:1/4\nK:C\n[CEG]2 [CE]").unwrap();
        assert_eq!((n.len(), n[0].length.0, n[3].start.0), (5, 2.0, 2.0));
        let n = parse("L:1/4\nK:C\nC2-C D").unwrap();
        assert_eq!((n.len(), n[0].length.0, n[1].start.0), (2, 3.0, 3.0));
        let n = parse("L:1/8\nK:C\nA>B").unwrap();
        assert_eq!((n[0].length.0, n[1].start.0, n[1].length.0), (0.75, 0.75, 0.25));
        let n = parse("L:1/8\nK:C\n(3ABc d").unwrap();
        assert!((n[1].start.0 - 1.0 / 3.0).abs() < 1e-9 && (n[3].start.0 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn repeats_and_endings() {
        assert_eq!(pitches("L:1/4\nK:C\n|: C D :| E"), [60, 62, 60, 62, 64]);
        assert_eq!(pitches("L:1/4\nK:C\n|: C [1 D :|[2 E |"), [60, 62, 60, 64]);
        assert_eq!(pitches("L:1/4\nK:C\n|: C |1 D :|2 E |"), [60, 62, 60, 64]);
    }

    #[test]
    fn dynamics_accents_staccato_and_ignored_stuff() {
        let n = parse("L:1/4\nK:C\n!p! C !f! D !>! E .F \"Am\" {g}G ~A % 注释").unwrap();
        assert_eq!(n.iter().map(|n| n.velocity).collect::<Vec<_>>(), [0.45, 0.82, 0.82, 0.82, 0.82, 0.82]);
        assert!(n[2].accent && !n[3].accent);
        assert_eq!(n[3].length.0, 0.5);
        assert_eq!(n.len(), 6);
    }

    /// 一首完整的曲子（爱尔兰吉格 The Kesh 的 A 段）：头部、6/8、反复与一二房。
    #[test]
    fn a_real_tune() {
        let tune = "X:1\nT:The Kesh\nM:6/8\nL:1/8\nR:jig\nK:G\n|:GAG GAB|ABA ABd|edd gdd|edB dBA|\nGAG GAB|ABA ABd|edd gdB|1 AGF G2A:|2 AGF G3|]";
        let n = parse(tune).unwrap();
        // 每遍 8 小节 × 3 拍；第一遍 1 房 6 个音、第二遍 2 房 4 个音。
        let first_pass = 7 * 6 + 3 + 2;
        assert_eq!(n.len(), first_pass + 7 * 6 + 3 + 1);
        assert_eq!(n[0].pitch, 67);
        assert!(n.iter().any(|x| x.pitch == 66), "K:G 的 F 是 F#");
        let end = n.last().map(|x| x.start.0 + x.length.0).unwrap();
        assert!((end - 48.0).abs() < 1e-9, "两遍 16 小节 × 3 拍：{end}");
    }

    #[test]
    fn errors_point_at_the_place() {
        assert!(parse("K:C\nC [DE").unwrap_err().contains("第 2 行"));
        assert!(parse("K:C\nC Q").unwrap_err().contains("无法识别"));
        assert!(parse("X:1\nV:1\nK:C\nC").unwrap_err().contains("多声部"));
    }
}
