//! ABC 记谱 → 音符。给 agent 快速写旋律、和声用（一行文字代替一长串音符 JSON）。
//!
//! 支持（单声部；多声部请分到不同音轨的 clip）：
//! - 头部：`L:` 单位时值（默认 1/8）、`M:` 拍号（`Z` 整小节休止用）、`K:` 调号（含调式：`Am`、`Dmix`、`Edor`、`F#m`、`Bb`…），
//!   `X:` `T:` `Q:` 等其它字段忽略（速度由工程决定）。
//! - 音高：`C` = 中央 C（MIDI 60），`c` 高八度；`'` 升八度、`,` 降八度；`^` 升、`_` 降、`=` 还原（`^^` `__` 重升重降）。
//!   临时记号在本小节内持续，调号按 `K:` 自动生效。
//! - 时值：`A2`、`A/2`、`A/`、`A3/2`、`A//`；休止 `z` / `x`，整小节休止 `Z`（`Z4` 四小节）。
//! - 和弦 `[CEG]2`，连音线 `A2-A`（和弦里单个音 `[C-EG]`），附点节奏 `A>B` / `A<B`，连音 `(3abc`（`(2`–`(9`）。
//! - 反复 `|: … :|`，一二房 `[1 … :|[2 …`（或 `|1` / `:|2`）。
//! - 力度 `!ppp!` … `!fff!`（之后的音都用这个力度），重音 `!>!` / `!accent!`，断奏 `.`（时值减半）。
//! - 和弦标记 `"Am"`、装饰音 `{…}`、其它装饰 `!…!` `+…+`、`~` `H` `L` `M` `O` `P` `S` `T` `u` `v`、注释 `%` 忽略。

use crate::{Beats, Note};

/// 解析出的音符（拍，相对 clip 起点）。
pub fn parse(text: &str) -> Result<Vec<Note>, String> {
    let p = Parser::new(text)?;
    let mut notes = p.run()?.0;
    if !p.lyrics.is_empty() {
        assign_lyrics(&mut notes, &p.lyrics);
    }
    Ok(notes)
}

/// `w:` 歌词行里的音节，按 ABC 2.1：空格或 `-` 分开（`hel-lo` 是两个音节），`*` 跳过一个音，
/// `_` 把上一个音节延长到这个音（这个音不唱新音节），`|` 只是对齐用，`~` 把两个词放在同一个音上。
fn lyric_tokens(lines: &[String]) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for line in lines {
        let mut cur = String::new();
        let flush = |cur: &mut String, out: &mut Vec<Option<String>>| {
            if !cur.is_empty() {
                out.push(Some(std::mem::take(cur).replace('~', " ")));
            }
        };
        for c in line.chars() {
            match c {
                ' ' | '\t' | '|' => flush(&mut cur, &mut out),
                '-' => flush(&mut cur, &mut out),
                '*' | '_' => {
                    flush(&mut cur, &mut out);
                    out.push(None);
                }
                _ => cur.push(c),
            }
        }
        flush(&mut cur, &mut out);
    }
    out
}

/// 音节按顺序放到音上；同一起点的和弦音唱同一个音节。
fn assign_lyrics(notes: &mut [Note], lines: &[String]) {
    let tokens = lyric_tokens(lines);
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].start.0.total_cmp(&notes[b].start.0));
    let mut k = 0;
    let mut i = 0;
    while i < order.len() && k < tokens.len() {
        let t = notes[order[i]].start.0;
        let mut j = i;
        while j < order.len() && (notes[order[j]].start.0 - t).abs() < 1e-9 {
            notes[order[j]].lyric = tokens[k].clone();
            j += 1;
        }
        i = j;
        k += 1;
    }
}

/// 小节时值和拍号对不上的地方（`meter` 是工程一小节的拍数，ABC 写了 `M:` 时用它）。只看用 `|` 分开的小节：
/// 中间的小节要正好一小节；第一小节可以短（弱起），不能长；最后一段不能长。有反复、一二房时不查。
pub fn bar_problems(text: &str, meter: f64) -> Result<Vec<String>, String> {
    let mut p = Parser::new(text)?;
    if !p.meter_given {
        p.bar = meter;
    }
    let (notes, marks, repeats, meters) = p.run()?;
    if repeats || marks.is_empty() {
        return Ok(Vec::new());
    }
    let end = notes.iter().map(|n| n.start.0 + n.length.0).fold(0.0, f64::max).max(marks.last().map_or(0.0, |m| m.1));
    let mut starts = vec![0.0];
    starts.extend(marks.iter().map(|m| m.1));
    starts.push(end);
    let beats = |x: f64| format!("{}", (x * 1000.0).round() / 1000.0);
    let units = |x: f64| (x / p.unit * 100.0).round() / 100.0;
    let mut out = Vec::new();
    let last = starts.len() - 2;
    for (k, w) in starts.windows(2).enumerate() {
        let len = w[1] - w[0];
        // 内联 [M:] 换了拍号：这一小节按它开头时的拍号查。
        let bar = meters.iter().rev().find(|m| m.0 <= w[0] + 1e-9).map_or(p.bar, |m| m.1);
        let wrong = if k == 0 || k == last { len > bar + 1e-6 } else { (len - bar).abs() > 1e-6 };
        if wrong && len > 1e-9 {
            out.push(format!("bar {} is {} beats ({} units) long, but a bar should be {} beats ({} units)", k + 1, beats(len), units(len), beats(bar), units(bar)));
        }
    }
    Ok(out)
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
        _ => return Err(format!("K: unrecognized mode '{s}'")),
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
        _ => return Err(format!("K: unrecognized tonic '{s}'")),
    };
    let fifths = base + 7 * tonic_acc + mode_shift;
    if !(-7..=7).contains(&fifths) {
        return Err(format!("K: key signature '{s}' is out of range (at most 7 sharps or flats)"));
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
    /// `w:` 歌词行。
    lyrics: Vec<String>,
    /// 单位时值（拍）。
    unit: f64,
    /// 一小节多少拍。
    bar: f64,
    /// 写了 `M:`。
    meter_given: bool,
    key: [i32; 7],
}

/// 一个待放的音（和弦里的音共享起点）。
struct Pending {
    pitches: Vec<i32>,
    length: f64,
    /// 整个音（和弦）连到下一个。
    tie: bool,
    /// 和弦里单个音的连音线（`[C-EG]` 只连 C），与 `pitches` 一一对应。
    tied: Vec<bool>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self, String> {
        let (mut unit, mut bar, mut key, mut meter_given) = (0.5, 4.0, [0; 7], false);
        let mut body = Vec::new();
        let mut lyrics = Vec::new();
        for (ln, raw) in text.lines().enumerate() {
            let line = raw.split('%').next().unwrap_or("").trim_end();
            let header = line.len() >= 2 && line.as_bytes()[1] == b':' && line.as_bytes()[0].is_ascii_alphabetic() && !line.starts_with('|');
            if header {
                let (field, value) = (line.as_bytes()[0] as char, line[2..].trim());
                match field {
                    'L' => unit = fraction(value).ok_or_else(|| format!("line {} L: expected a fraction, e.g. 1/8", ln + 1))? * 4.0,
                    'M' => {
                        if let Some(f) = fraction(value) {
                            (bar, meter_given) = (f * 4.0, true);
                        }
                    }
                    'K' => {
                        key = key_signature(value).map_err(|e| format!("line {} {e}", ln + 1))?;
                    }
                    'V' => return Err(format!("line {}: multi-voice V: is not supported; put each voice in its own track's clip", ln + 1)),
                    'w' => lyrics.push(value.to_string()),
                    _ => {}
                }
                continue;
            }
            if !line.trim().is_empty() {
                body.push((ln + 1, line));
            }
        }
        Ok(Parser { body, lyrics, unit, bar, meter_given, key })
    }

    /// 音符，小节线的位置（行号, 拍），有没有反复 / 一二房，内联 `[M:]` 换拍号的位置（拍, 一小节几拍）。
    #[allow(clippy::type_complexity)]
    fn run(&self) -> Result<(Vec<Note>, Vec<(usize, f64)>, bool, Vec<(f64, f64)>), String> {
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
        let mut marks = Vec::new();
        let mut bar = self.bar;
        let mut meters = Vec::new();
        let repeats = tokens.iter().any(|(_, t)| matches!(t, Tok::RepeatStart | Tok::RepeatEnd | Tok::Ending(_)));
        for &i in &order {
            let (line, tok) = &tokens[i];
            let at = |m: &str| format!("line {line}: {m}");
            match tok {
                Tok::Bar => {
                    accidentals = [None; 128];
                    marks.push((*line, t));
                }
                Tok::RepeatStart | Tok::RepeatEnd | Tok::Ending(_) => accidentals = [None; 128],
                Tok::Meter(b) => {
                    bar = *b;
                    meters.push((t, bar));
                }
                Tok::Dynamic(v) => velocity = *v,
                Tok::Accent => accent = true,
                Tok::Staccato => staccato = true,
                Tok::Tuplet(p) => {
                    let q = match p {
                        2 | 4 | 8 => 3.0,
                        3 | 6 => 2.0,
                        _ => {
                            if (bar / 4.0 * 8.0).round() as i64 % 3 == 0 {
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
                    // 多小节休止：中间的小节线也记下。
                    marks.extend((1..*n).map(|k| (*line, t + bar * f64::from(k))));
                    t += bar * f64::from(*n);
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
                    for (k, &spec) in p.pitches.iter().enumerate() {
                        let tie = p.tie || p.tied[k];
                        let pitch = self.pitch(spec, &mut accidentals).ok_or_else(|| at("pitch is outside the MIDI range 0-127"))?;
                        // 连音线：前面同音高的音延长，不新起一个。
                        if let Some(pos) = tied.iter().position(|&j| notes[j].pitch == pitch) {
                            let j = tied.remove(pos);
                            notes[j].length = Beats(notes[j].length.0 + l);
                            if tie {
                                next_tied.push(j);
                            }
                            continue;
                        }
                        let length = if staccato { l * 0.5 } else { l };
                        notes.push(Note { pitch, start: Beats(t), length: Beats(length), velocity, accent, mute: false, bend: Vec::new(), lyric: None });
                        if tie {
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
            return Err("ABC has no notes (write the melody after K:, e.g. \"L:1/8\\nK:Am\\nA2 cB e2 dc |\")".into());
        }
        Ok((notes, marks, repeats, meters))
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
            let at = |i: usize, m: &str| format!("line {line} column {}: {m}", i + 1);
            while i < b.len() {
                let c = b[i] as char;
                match c {
                    ' ' | '\t' | '\\' | '`' => i += 1,
                    '"' => {
                        // 和弦标记 / 注释文字：跳过。
                        let end = text[i + 1..].find('"').ok_or_else(|| at(i, "unclosed quote"))?;
                        i += end + 2;
                    }
                    '{' => {
                        let end = text[i..].find('}').ok_or_else(|| at(i, "unclosed grace note {"))?;
                        i += end + 1;
                    }
                    '!' | '+' => {
                        let end = text[i + 1..].find(c).ok_or_else(|| at(i, "unclosed decoration"))?;
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
                            _ => return Err(at(i, "a dotted rhythm > / < needs a note before it")),
                        }
                        out.push((line, Tok::Broken(next)));
                    }
                    '|' | ':' | '[' if self.bar_token(text, i).is_some() => {
                        let (tok, len) = self.bar_token(text, i).unwrap();
                        out.extend(tok.into_iter().map(|t| (line, t)));
                        i += len;
                    }
                    '[' if b.get(i + 2) == Some(&b':') && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) => {
                        // 内联字段 [M:7/8]：认拍号，其它的跳过。
                        let end = text[i..].find(']').ok_or_else(|| at(i, "unclosed inline field ["))?;
                        if b[i + 1] == b'M' {
                            let f = fraction(text[i + 3..i + end].trim()).ok_or_else(|| at(i, "[M:] expects a fraction, e.g. [M:7/8]"))?;
                            out.push((line, Tok::Meter(f * 4.0)));
                        }
                        i += end + 1;
                    }
                    '[' => {
                        // 和弦 [CEG]2
                        let end = text[i..].find(']').ok_or_else(|| at(i, "unclosed chord ["))?;
                        let inner = &text[i + 1..i + end];
                        let mut pitches = Vec::new();
                        let mut tied = Vec::new();
                        let mut j = 0;
                        let ib = inner.as_bytes();
                        let mut inner_len = None;
                        while j < ib.len() {
                            if ib[j] == b' ' {
                                j += 1;
                                continue;
                            }
                            let (spec, l, n) = note_spec(inner, j).ok_or_else(|| at(i + 1 + j, "expected a note inside the chord"))?;
                            pitches.push(spec);
                            inner_len.get_or_insert(l);
                            j += n;
                            tied.push(ib.get(j) == Some(&b'-'));
                            if tied.last() == Some(&true) {
                                j += 1;
                            }
                        }
                        if pitches.is_empty() {
                            return Err(at(i, "empty chord []"));
                        }
                        i += end + 1;
                        let (mult, n) = length(text, i);
                        i += n;
                        let tie = b.get(i) == Some(&b'-');
                        if tie {
                            i += 1;
                        }
                        out.push((line, Tok::Notes(Pending { pitches, length: inner_len.unwrap_or(1.0) * mult, tie, tied })));
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
                        let (spec, l, n) = note_spec(text, i).ok_or_else(|| at(i, &format!("unrecognized '{c}'")))?;
                        i += n;
                        let tie = b.get(i) == Some(&b'-');
                        if tie {
                            i += 1;
                        }
                        out.push((line, Tok::Notes(Pending { pitches: vec![spec], length: l, tie, tied: vec![false] })));
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
    /// 内联 `[M:]`：之后一小节几拍。
    Meter(f64),
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

    #[test]
    fn inline_meter_changes_bar_length() {
        // [M:7/8] 之后一小节 3.5 拍：Z 按新拍号算，小节检查也按新拍号。
        let n = parse("M:4/4\nL:1/8\nK:C\nC8 |[M:7/8] Z | D7 |[M:4/4] E8 |").unwrap();
        let starts: Vec<f64> = n.iter().map(|x| x.start.0).collect();
        assert_eq!(starts, [0.0, 7.5, 11.0]);
        assert!(bar_problems("M:4/4\nL:1/8\nK:C\nC8 |[M:7/8] D7 | E7 |[M:4/4] F8 |", 4.0).unwrap().is_empty());
        assert_eq!(bar_problems("M:4/4\nL:1/8\nK:C\nC8 |[M:7/8] D8 | E7 |", 4.0).unwrap().len(), 1);
    }

    #[test]
    fn lyric_lines_follow_the_notes() {
        let n = parse("L:1/4\nK:C\nC D [EG] F | G2 A B |\nw: la shi-ni * hao_ | sing").unwrap();
        let l: Vec<Option<&str>> = n.iter().map(|x| x.lyric.as_deref()).collect();
        // C la, D shi, [E G] ni（和弦两个音同一个音节）, F 跳过, G hao, A 延长（不唱新的）, B sing。
        assert_eq!(l, vec![Some("la"), Some("shi"), Some("ni"), Some("ni"), None, Some("hao"), None, Some("sing")]);
    }

    #[test]
    fn bar_lengths_against_the_meter() {
        // 9/8：第 1 小节 10 个八分、第 2 小节 8 个。
        let p = bar_problems("L:1/8\nK:Dm\nD2 E2 F2 G2 A2 | c2 z2 D2 E2 |", 4.5).unwrap();
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].starts_with("bar 1 is 5 beats (10 units) long, but a bar should be 4.5 beats (9 units)"), "{p:?}");
        // 对的：弱起、整小节休止、M: 优先于工程拍号、有反复不查。
        assert!(bar_problems("L:1/8\nK:C\nG | c4 e4 | Z2 | g8 |", 4.0).unwrap().is_empty());
        assert!(bar_problems("M:3/4\nL:1/4\nK:C\nC D E | F G A |", 4.0).unwrap().is_empty());
        assert!(bar_problems("L:1/8\nK:C\n|: c4 :|", 4.0).unwrap().is_empty());
        assert!(bar_problems("L:1/8\nK:C\nc4 e4 g4", 4.0).unwrap().is_empty(), "没有小节线不查");
    }

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
        // 和弦里单个音的连音线：只有 C 连到下一个和弦。
        let n = parse("L:1/4\nK:C\n[C-EG] [CF]").unwrap();
        let c: Vec<_> = n.iter().filter(|n| n.pitch == 60).map(|n| n.length.0).collect();
        assert_eq!((n.len(), c), (4, vec![2.0]));
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
        assert!(parse("K:C\nC [DE").unwrap_err().contains("line 2"));
        assert!(parse("K:C\nC Q").unwrap_err().contains("unrecognized"));
        assert!(parse("X:1\nV:1\nK:C\nC").unwrap_err().contains("multi-voice"));
    }
}
