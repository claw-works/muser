//! 导出 ABC 记谱：给能读 ABC 的模型（做音乐推理、续写、改编）或记谱软件用。
//!
//! 两种写法：
//! - [`Dialect::Standard`]：ABC 2.1 多声部，每条乐器音轨一个 `V:`，复音写成 `[CEG]`。
//! - [`Dialect::Yue2`]：YuE2（m-a-p/YuE2）的原生写法：固定的 `Vocal` / `Ins` 两个单音声部，和弦记号写在 `Vocal` 上，
//!   时值只用 1 2 3 4 6 8 12 16 24 32 48 个单位（其它用连音线拼），没有复音、连音、反复。
//!   每组 1–4 小节，一个 `V: Vocal` 块接一个 `V: Ins` 块；`% verse` 之类的注释标段落。
//!
//! 写的是谱面上的音（humanize 之前、clip 裁剪之后）：`notes` / `abc` / `grid` / `drummer` 展开的都算。
//! 鼓组与 audio 音轨不导出。调号由音高分布估计（Krumhansl 调性轮廓），升降号按调号拼写。
//! 和弦记号（可选）按每小节（必要时半小节）伴奏音的音级匹配和弦模板。
//! 起点与时值量化到 1/16（需要时 1/32、1/64）音符的网格，三连音等落不到网格上的会被就近取整（会提示）。

use std::collections::BTreeMap;

use muser_core::Project;

use crate::render::EngineError;

/// 写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Standard,
    Yue2,
}

/// 导出选项。
#[derive(Debug, Clone)]
pub struct AbcOptions {
    pub dialect: Dialect,
    /// 写和弦记号（YuE2 的 `full` 模式要；`melody` 模式不要）。
    pub chords: bool,
    /// 标准写法：只导出这些音轨（名字）；空 = 全部乐器音轨。
    pub tracks: Vec<String>,
    /// YuE2：唱旋律的音轨，可以几条（旋律在乐器间传递时），合并后每个时刻取最高的音。
    /// 缺省按名字（vocal / vox / lead / melody…），否则取偏高音区里发声最久的音轨。
    pub vocal: Vec<String>,
    /// YuE2：器乐旋律声部的音轨（可以几条；缺省全休止）。
    pub ins: Vec<String>,
    /// 段落：(名字, 小节数)，按顺序写成 `% 名字` 注释。
    pub sections: Vec<(String, u32)>,
    /// 调号（如 `G`、`F#m`）；缺省用 clip 里 ABC 写的 `K:`（都一样时），否则由音高估计。
    pub key: Option<String>,
    /// YuE2：Vocal 声部整体移几个八度（第三方实测说人声写在小写音名区——C5 以上——咬字更清楚，未经官方确认）。
    pub vocal_octave: i32,
}

impl AbcOptions {
    pub fn new(dialect: Dialect) -> Self {
        Self { dialect, chords: dialect == Dialect::Yue2, tracks: Vec::new(), vocal: Vec::new(), ins: Vec::new(), sections: Vec::new(), key: None, vocal_octave: 0 }
    }
}

/// 导出结果：ABC 文本与提示（跳过的音轨、量化、复音变单音等）。
pub struct AbcExport {
    pub text: String,
    pub notes: Vec<String>,
}

/// 绝对时间（拍）上的一个音。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tone {
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) pitch: u8,
}

const YUE2_DURATIONS: [usize; 11] = [48, 32, 24, 16, 12, 8, 6, 4, 3, 2, 1];
const NATURAL: [(char, i32); 7] = [('C', 0), ('D', 2), ('E', 4), ('F', 5), ('G', 7), ('A', 9), ('B', 11)];
/// 调号（五度圈位置，-7..=7）→ 名字。YuE2 支持的全部标准大小调。
const MAJOR: [&str; 15] = ["Cb", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#"];
const MINOR: [&str; 15] = ["Abm", "Ebm", "Bbm", "Fm", "Cm", "Gm", "Dm", "Am", "Em", "Bm", "F#m", "C#m", "G#m", "D#m", "A#m"];
/// 和弦模板（YuE2 的原生和弦记号）：后缀与相对根音的音程。
const CHORDS: [(&str, &[u8]); 15] = [
    ("", &[0, 4, 7]), ("m", &[0, 3, 7]), ("7", &[0, 4, 7, 10]), ("maj7", &[0, 4, 7, 11]), ("m7", &[0, 3, 7, 10]),
    ("dim", &[0, 3, 6]), ("aug", &[0, 4, 8]), ("sus4", &[0, 5, 7]), ("sus2", &[0, 2, 7]), ("6", &[0, 4, 7, 9]),
    ("m6", &[0, 3, 7, 9]), ("dim7", &[0, 3, 6, 9]), ("m7b5", &[0, 3, 6, 10]), ("7sus4", &[0, 5, 7, 10]), ("m(maj7)", &[0, 3, 7, 11]),
];

/// 调号：五度圈位置（正为升号个数，负为降号个数）与是否小调。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Key {
    fifths: i32,
    minor: bool,
}

impl Key {
    /// 标准大小调的名字（`G`、`F#m`、`Bb`…）→ 调号。
    fn parse(name: &str) -> Option<Key> {
        let n = name.trim();
        let find = |list: &[&str; 15]| list.iter().position(|k| *k == n).map(|i| i as i32 - 7);
        find(&MAJOR).map(|fifths| Key { fifths, minor: false }).or_else(|| find(&MINOR).map(|fifths| Key { fifths, minor: true }))
    }

    pub(crate) fn name(self) -> &'static str {
        let i = (self.fifths + 7) as usize;
        if self.minor { MINOR[i] } else { MAJOR[i] }
    }
    /// 调号给每个音名的升降。
    fn signature(self) -> [i32; 7] {
        let mut acc = [0; 7];
        let order = if self.fifths > 0 { "FCGDAEB" } else { "BEADGCF" };
        for l in order.chars().take(self.fifths.unsigned_abs() as usize) {
            acc[letter_index(l)] = self.fifths.signum();
        }
        acc
    }
    /// 调内音的音级：大调音阶；小调用自然小调加上导音（和声小调的 V 级大三和弦也算调内）。
    pub(crate) fn scale_pcs(self) -> Vec<u8> {
        let major = (self.fifths * 7).rem_euclid(12);
        let mut pcs: Vec<u8> = [0, 2, 4, 5, 7, 9, 11].iter().map(|i| ((major + i) % 12) as u8).collect();
        if self.minor {
            pcs.push(((self.tonic() + 11) % 12) as u8);
        }
        pcs
    }

    /// 主音的音级。
    fn tonic(self) -> i32 {
        (self.fifths * 7 - if self.minor { 3 } else { 0 }).rem_euclid(12)
    }
    /// 音级 → (音名, 升降)：调内音按调号拼；小调的导音写成升高的第七级（F#m 里是 E#）；
    /// 其它调外音先用还原（D 大调里的 F 写 =F），再按升号调升、降号调降。
    fn spell(self, pc: i32) -> (char, i32) {
        let sig = self.signature();
        let pc = pc.rem_euclid(12);
        for (i, &(l, n)) in NATURAL.iter().enumerate() {
            if (n + sig[i]).rem_euclid(12) == pc {
                return (l, sig[i]);
            }
        }
        if self.minor && pc == (self.tonic() + 11) % 12 {
            let (tl, _) = self.spell(self.tonic());
            let i = (letter_index(tl) + 6) % 7;
            return (NATURAL[i].0, sig[i] + 1);
        }
        if let Some(&(l, _)) = NATURAL.iter().find(|&&(_, n)| n == pc) {
            return (l, 0);
        }
        let shift = if self.fifths >= 0 { 1 } else { -1 };
        for (i, &(l, n)) in NATURAL.iter().enumerate() {
            if (n + sig[i] + shift).rem_euclid(12) == pc {
                return (l, sig[i] + shift);
            }
        }
        unreachable!("每个音级都能由某个音名升降一个半音得到")
    }
}

fn letter_index(l: char) -> usize {
    NATURAL.iter().position(|&(c, _)| c == l).unwrap()
}

/// Krumhansl–Kessler 调性轮廓，按时长加权的音级分布与 24 个调求相关，取最像的。
pub(crate) fn estimate_key(tones: &[Tone]) -> Key {
    const MAJ: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
    const MIN: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
    let mut hist = [0.0; 12];
    for t in tones {
        hist[usize::from(t.pitch % 12)] += t.end - t.start;
    }
    if hist.iter().all(|&h| h == 0.0) {
        return Key { fifths: 0, minor: false };
    }
    let corr = |profile: &[f64; 12], tonic: usize| {
        let (mx, my) = (hist.iter().sum::<f64>() / 12.0, profile.iter().sum::<f64>() / 12.0);
        let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            let (x, y) = (hist[(tonic + i) % 12] - mx, profile[i] - my);
            num += x * y;
            dx += x * x;
            dy += y * y;
        }
        num / (dx * dy).sqrt().max(1e-12)
    };
    let mut best = (f64::MIN, Key { fifths: 0, minor: false });
    for tonic in 0..12 {
        for minor in [false, true] {
            let r = corr(if minor { &MIN } else { &MAJ }, tonic);
            // 主音 → 五度圈位置；小调用关系大调的调号。取 -5..=6（Db 而不是 C#，F# 而不是 Gb）。
            let major_tonic = if minor { (tonic + 3) % 12 } else { tonic };
            let fifths = ((major_tonic as i32 * 7).rem_euclid(12) + 5).rem_euclid(12) - 5;
            if r > best.0 + 1e-9 {
                best = (r, Key { fifths, minor });
            }
        }
    }
    best.1
}

/// 一段时间里的音（伴奏，加上权重减半的旋律）的音级 → 最像的和弦（YuE2 的和弦记号）。没有音时为 None。
pub(crate) fn guess_chord(tones: &[(Tone, f64)], from: f64, to: f64, key: Key) -> Option<(String, f64)> {
    let mut w = [0.0; 12];
    let mut bass: Option<(u8, f64)> = None;
    for &(t, weight) in tones {
        let d = t.end.min(to) - t.start.max(from);
        if d <= 1e-9 {
            continue;
        }
        w[usize::from(t.pitch % 12)] += d * weight;
        if bass.is_none_or(|(p, _)| t.pitch < p) {
            bass = Some((t.pitch, d));
        }
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, String)> = None;
    for root in 0..12u8 {
        for (suffix, intervals) in CHORDS {
            let tones_in = |pc: usize| intervals.iter().any(|&i| usize::from((root + i) % 12) == pc);
            let (mut inside, mut outside) = (0.0, 0.0);
            for (pc, &v) in w.iter().enumerate() {
                if tones_in(pc) { inside += v } else { outside += v }
            }
            // 根音、三音要有；多出的音（七音、六音）要有证据，不然取三和弦。
            let has = |i: u8| w[usize::from((root + i) % 12)] > 0.0;
            if !has(0) || !has(intervals[1]) {
                continue;
            }
            let missing = intervals.iter().filter(|&&i| !has(i)).count() as f64;
            // 先验：大小三和弦最常见，七和弦次之，挂留、减、增要更多证据。
            let prior = match suffix {
                "" | "m" => 0.0,
                "7" | "m7" | "maj7" => 0.06,
                "6" | "m6" => 0.1,
                _ => 0.18,
            };
            let mut score = inside - 1.5 * outside - 0.25 * total * missing - prior * total;
            if bass.is_some_and(|(p, _)| p % 12 == root) {
                score += 0.3 * total;
            }
            if best.as_ref().is_none_or(|(s, _)| score > *s + 1e-9) {
                let (l, acc) = key.spell(i32::from(root));
                let name = format!("{l}{}{suffix}", match acc { 1 => "#", -1 => "b", 2 => "##", -2 => "bb", _ => "" });
                best = Some((score, name));
            }
        }
    }
    best.map(|(s, n)| (n, s / total))
}

/// 写好的和弦 → YuE2 的原生和弦记号（只有 15 种性质：九、十一、十三和弦退回七和弦，add9 退回三和弦）。
fn native_chord(c: &muser_core::chord::Chord, key: Key) -> String {
    let has = |i: u8| c.intervals.contains(&i);
    let third = [4, 3, 5, 2].into_iter().find(|&i| has(i)).unwrap_or(0);
    let fifth = if has(7) { 7 } else if has(6) { 6 } else if has(8) { 8 } else { 7 };
    let seventh = if has(11) { Some(11) } else if has(10) { Some(10) } else if has(9) && third == 3 && fifth == 6 { Some(9) } else { None };
    let six = has(9) && seventh.is_none();
    let suffix = match (third, fifth, seventh) {
        (3, 6, Some(9)) => "dim7",
        (3, 6, Some(10)) => "m7b5",
        (3, 6, _) => "dim",
        (4, 8, _) => "aug",
        (5, _, Some(10)) => "7sus4",
        (5, _, _) => "sus4",
        (2, _, _) => "sus2",
        (3, _, Some(11)) => "m(maj7)",
        (3, _, Some(10)) => "m7",
        (3, _, _) if six => "m6",
        (3, _, _) => "m",
        (_, _, Some(11)) => "maj7",
        (_, _, Some(10)) => "7",
        _ if six => "6",
        _ => "",
    };
    let name = |pc: u8| {
        let (l, acc) = key.spell(i32::from(pc));
        format!("{l}{}", match acc { 1 => "#", -1 => "b", 2 => "##", -2 => "bb", _ => "" })
    };
    match c.bass.filter(|&b| b != c.root) {
        Some(b) => format!("{}{suffix}/{}", name(c.root), name(b)),
        None => format!("{}{suffix}", name(c.root)),
    }
}

/// 一个声部在网格上的一个事件：从 `at` 起 `len` 个单位，音高（空 = 休止），每个音是否连到下一个事件。
#[derive(Debug, Clone)]
struct Event {
    at: usize,
    len: usize,
    pitches: Vec<(u8, bool)>,
}

/// 把音栅格化到单位格上，再合并成事件。`mono` 时每格只留最高的音。`breaks` 是必须断开的格（小节线、和弦变化）。
fn events(tones: &[Tone], unit: f64, slots: usize, mono: bool, breaks: &[bool]) -> Vec<Event> {
    // 每格：(音高, 这一格是否起音)。
    let mut grid: Vec<Vec<(u8, bool)>> = vec![Vec::new(); slots];
    for t in tones {
        let (a, b) = ((t.start / unit).round() as usize, ((t.end / unit).round() as usize).min(slots));
        for (s, cell) in grid.iter_mut().enumerate().take(b).skip(a) {
            if let Some(c) = cell.iter_mut().find(|(p, _)| *p == t.pitch) {
                c.1 |= s == a; // 同音重叠：重新起音
            } else {
                cell.push((t.pitch, s == a));
            }
        }
    }
    for cell in &mut grid {
        cell.sort_by(|x, y| y.0.cmp(&x.0));
        if mono {
            cell.truncate(1);
        }
    }
    let mut out: Vec<Event> = Vec::new();
    for s in 0..slots {
        let cell = &grid[s];
        let same = |e: &Event| e.at + e.len == s && !breaks[s] && e.pitches.len() == cell.len()
            && e.pitches.iter().zip(cell).all(|(a, b)| a.0 == b.0 && !b.1);
        match out.last_mut() {
            Some(e) if same(e) => e.len += 1,
            _ => out.push(Event { at: s, len: 1, pitches: cell.iter().map(|&(p, _)| (p, false)).collect() }),
        }
    }
    // 连音线：下一个事件里同一个音没有重新起音。
    for i in 0..out.len().saturating_sub(1) {
        let s = out[i + 1].at;
        let held: Vec<u8> = grid[s].iter().filter(|c| !c.1).map(|c| c.0).collect();
        for p in &mut out[i].pitches {
            p.1 = held.contains(&p.0);
        }
    }
    out
}

/// 小节内的升降记号状态：按音名（YuE2 的写法跨八度沿用）与按音名+八度（标准写法）同时记，任一不符就写出记号。
struct Accidentals {
    sig: [i32; 7],
    letter: [i32; 7],
    exact: BTreeMap<(usize, i32), i32>,
}

impl Accidentals {
    fn new(key: Key) -> Self {
        let sig = key.signature();
        Self { sig, letter: sig, exact: BTreeMap::new() }
    }
    fn bar(&mut self) {
        self.letter = self.sig;
        self.exact.clear();
    }
    /// MIDI 音高 → ABC 音符（不含时值）。
    fn note(&mut self, key: Key, pitch: u8) -> String {
        let (l, acc) = key.spell(i32::from(pitch % 12));
        let li = letter_index(l);
        let written = i32::from(pitch) - acc;
        let octave = (written - NATURAL[li].1).div_euclid(12) - 1; // MIDI 60 = 第 4 组
        let exact = *self.exact.get(&(li, octave)).unwrap_or(&self.sig[li]);
        let mut s = String::new();
        if acc != self.letter[li] || acc != exact {
            s.push_str(match acc { 2 => "^^", 1 => "^", 0 => "=", -1 => "_", _ => "__" });
            self.letter[li] = acc;
            self.exact.insert((li, octave), acc);
        }
        match octave {
            o if o >= 5 => {
                s.push(l.to_ascii_lowercase());
                s.extend(std::iter::repeat_n('\'', (o - 5) as usize));
            }
            o => {
                s.push(l);
                s.extend(std::iter::repeat_n(',', (4 - o).max(0) as usize));
            }
        }
        s
    }
}

/// 时值拆成可写的长度：标准写法一个整数；YuE2 拆成它支持的长度（之后用连音线连起来）。
fn pieces(len: usize, dialect: Dialect) -> Vec<usize> {
    if dialect == Dialect::Standard {
        return vec![len];
    }
    let mut out = Vec::new();
    let mut left = len;
    while left > 0 {
        let d = *YUE2_DURATIONS.iter().find(|&&d| d <= left).unwrap();
        out.push(d);
        left -= d;
    }
    out
}

fn dur(n: usize) -> String {
    if n == 1 { String::new() } else { n.to_string() }
}

/// 一个声部的一小节。`chords` 是这一小节里 (单位格, 和弦名)。
fn write_bar(evs: &[&Event], chords: &[(usize, String)], key: Key, acc: &mut Accidentals, dialect: Dialect) -> String {
    acc.bar();
    let mut s = String::new();
    for e in evs {
        let mut at = e.at;
        let parts = pieces(e.len, dialect);
        for (k, &n) in parts.iter().enumerate() {
            if let Some((_, c)) = chords.iter().find(|(a, _)| *a == at) {
                s.push_str(&format!("\"{c}\""));
            }
            let last = k + 1 == parts.len();
            match e.pitches.as_slice() {
                [] => s.push_str(&format!("z{}", dur(n))),
                [(p, tie)] => {
                    s.push_str(&acc.note(key, *p));
                    s.push_str(&dur(n));
                    if !last || *tie {
                        s.push('-');
                    }
                }
                many => {
                    s.push('[');
                    for (p, tie) in many {
                        s.push_str(&acc.note(key, *p));
                        if !last || *tie {
                            s.push('-');
                        }
                    }
                    s.push(']');
                    s.push_str(&dur(n));
                }
            }
            at += n;
        }
    }
    s
}

/// 把工程导出为 ABC。
pub fn export(project: &Project, opts: &AbcOptions) -> Result<AbcExport, EngineError> {
    crate::validate_score(project)?;
    let invalid = |m: String| EngineError::Core(muser_core::CoreError::Invalid(m));
    // 编曲层：段落名与写好的和弦直接用；音符看展开后的。
    let spans = crate::arrange::timeline(project).map_err(invalid)?;
    let arranged = crate::arrange::arranged(project).map_err(invalid)?;
    let project = &*arranged;
    let mut notes = Vec::new();
    // 每条乐器音轨的音（绝对拍）。
    let mut parts: Vec<(String, Vec<Tone>)> = Vec::new();
    for (ti, t) in project.tracks.iter().enumerate() {
        let name = if t.name.is_empty() { format!("tracks[{ti}]") } else { t.name.clone() };
        let inst = t.instrument.trim();
        if inst == muser_core::audio::INSTRUMENT || inst.starts_with("drums.") {
            notes.push(format!("没有导出 '{name}'（{}）", if inst.starts_with("drums.") { "鼓组" } else { "audio 音轨" }));
            continue;
        }
        let mut tones = Vec::new();
        for (ci, clip) in t.clips.iter().enumerate() {
            for n in crate::clip_notes(project, ti, ci).map_err(invalid)?.iter() {
                let (a, b) = (n.start.0, (n.start.0 + n.length.0).min(clip.length.0));
                if b > a {
                    tones.push(Tone { start: clip.start.0 + a, end: clip.start.0 + b, pitch: n.pitch });
                }
            }
        }
        if !tones.is_empty() {
            parts.push((name, tones));
        }
    }
    let find = |name: &str| parts.iter().position(|(n, _)| n == name).ok_or_else(|| {
        invalid(format!("没有叫 '{name}' 的乐器音轨（可选: {}）", parts.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")))
    });
    let find_all = |names: &[String]| names.iter().map(|n| find(n)).collect::<Result<Vec<usize>, _>>();
    // 声部：(声部名, 合并进来的音轨下标)。
    let voices: Vec<(String, Vec<usize>)> = match opts.dialect {
        Dialect::Standard if opts.tracks.is_empty() => (0..parts.len()).map(|i| (parts[i].0.clone(), vec![i])).collect(),
        Dialect::Standard => opts.tracks.iter().map(|n| find(n).map(|i| (n.clone(), vec![i]))).collect::<Result<_, _>>()?,
        Dialect::Yue2 => {
            let vocal = match opts.vocal.is_empty() {
                false => find_all(&opts.vocal)?,
                true => {
                    let named = parts.iter().position(|(n, _)| {
                        let n = n.to_ascii_lowercase();
                        ["vocal", "vox", "voice", "sing", "lead", "melody", "hook"].iter().any(|k| n.contains(k))
                    });
                    // 偏高音区（平均音高不低于各轨的中位数）里，发声时间最长的。
                    let mean = |ts: &[Tone]| ts.iter().map(|t| f64::from(t.pitch)).sum::<f64>() / ts.len() as f64;
                    let mut means: Vec<f64> = parts.iter().map(|(_, ts)| mean(ts)).collect();
                    means.sort_by(f64::total_cmp);
                    let median = means.get(means.len() / 2).copied().unwrap_or(0.0);
                    let sounding = |ts: &[Tone]| {
                        let mut iv: Vec<(f64, f64)> = ts.iter().map(|t| (t.start, t.end)).collect();
                        iv.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let (mut total, mut end) = (0.0, f64::MIN);
                        for (a, b) in iv {
                            total += (b - a.max(end)).max(0.0);
                            end = end.max(b);
                        }
                        total
                    };
                    let high = (0..parts.len()).filter(|&i| mean(&parts[i].1) >= median);
                    named.or_else(|| high.max_by(|&a, &b| sounding(&parts[a].1).total_cmp(&sounding(&parts[b].1)))).into_iter().collect()
                }
            };
            let ins = find_all(&opts.ins)?;
            if !vocal.is_empty() {
                let names: Vec<&str> = vocal.iter().map(|&i| parts[i].0.as_str()).collect();
                notes.push(format!("Vocal = {}{}", names.join(" + "), if opts.vocal.is_empty() { "（自动选的，用 --vocal 指定，可写几条：a,b,c）" } else { "" }));
            }
            vec![("Vocal".into(), vocal), ("Ins".into(), ins)]
        }
    };
    if voices.iter().all(|(_, v)| v.is_empty()) {
        return Err(invalid("没有可导出的乐器音轨（鼓组与 audio 音轨不导出）".into()));
    }

    // 网格：能精确表示所有起点与时值的最粗单位（1/16、1/32、1/64 音符），都不行就量化到 1/32。
    // YuE2 至少用 1/32：它的原生导出常用 1/32，第三方实测 L:1/16 的谱生成到一半会停。
    let used: Vec<&Tone> = voices.iter().flat_map(|(_, v)| v.iter()).flat_map(|&i| parts[i].1.iter()).collect();
    let on_grid = |u: f64| used.iter().all(|t| [t.start, t.end].iter().all(|x| ((x / u).round() * u - x).abs() < 1e-6));
    let grids: &[(u32, f64)] = if opts.dialect == Dialect::Yue2 { &[(32, 0.125), (64, 0.0625)] } else { &[(16, 0.25), (32, 0.125), (64, 0.0625)] };
    let (denominator, unit) = grids.iter().copied().find(|&(_, u)| on_grid(u)).unwrap_or_else(|| {
        notes.push("有落不到 1/64 音符网格上的音（三连音等），已就近量化到 1/32".into());
        (32, 0.125)
    });
    let ts = project.time_signature;
    let bar_beats = f64::from(ts.numerator) * 4.0 / f64::from(ts.denominator);
    let bar_units = (bar_beats / unit).round() as usize;
    let end_beats = used.iter().map(|t| t.end).fold(0.0, f64::max).max(project.duration_beats().0);
    let bars = ((end_beats / bar_beats) - 1e-9).ceil().max(1.0) as usize;
    let slots = bars * bar_units;
    // 调号：指定的 > clip 里 ABC 写的（都一样时）> 由整首（全部乐器音轨）的音高估计。
    let abc_keys: std::collections::BTreeSet<String> = parts
        .iter()
        .flat_map(|(name, _)| project.tracks.iter().find(|t| &t.name == name).into_iter().flat_map(|t| t.clips.iter()))
        .filter_map(|c| c.abc.as_deref())
        .filter_map(|a| a.lines().find_map(|l| l.trim().strip_prefix("K:").map(|k| k.trim().to_string())))
        .collect();
    let key = match &opts.key {
        Some(k) => Key::parse(k).ok_or_else(|| invalid(format!("调号 '{k}' 不认识：写标准大小调，如 G、Eb、F#m、Bbm")))?,
        None => match abc_keys.iter().next().filter(|_| abc_keys.len() == 1).and_then(|k| Key::parse(k)) {
            Some(k) => k,
            None => estimate_key(&parts.iter().flat_map(|(_, ts)| ts.iter().copied()).collect::<Vec<_>>()),
        },
    };

    // 和弦记号：伴奏 = 导出的乐器音轨里除 Vocal / Ins 之外的（标准写法：全部）；没有伴奏时用全部。
    let mut chord_at: Vec<(usize, String)> = Vec::new();
    let declared = spans.iter().any(|s| !s.chords.is_empty());
    if opts.chords && declared {
        let mut last: Option<String> = None;
        for p in spans.iter().flat_map(|s| s.chords.iter()) {
            let name = match opts.dialect {
                Dialect::Yue2 => native_chord(&p.chord, key),
                Dialect::Standard => p.chord.text.clone(),
            };
            let at = (p.start / unit).round() as usize;
            if at < slots && last.as_deref() != Some(&name) {
                chord_at.push((at, name.clone()));
                last = Some(name);
            }
        }
    } else if opts.chords {
        let melodic: Vec<usize> = match opts.dialect {
            Dialect::Yue2 => voices.iter().flat_map(|(_, v)| v.iter().copied()).collect(),
            Dialect::Standard => Vec::new(),
        };
        let harmony: Vec<(Tone, f64)> = (0..parts.len())
            .flat_map(|i| parts[i].1.iter().map(move |&t| (t, i)))
            .map(|(t, i)| (t, if melodic.contains(&i) { 0.5 } else { 1.0 }))
            .collect();
        let mut last: Option<String> = None;
        for b in 0..bars {
            let (a, z) = (b as f64 * bar_beats, (b + 1) as f64 * bar_beats);
            let whole = guess_chord(&harmony, a, z, key);
            // 半小节换和弦：两半各自更像不同的和弦时拆开。
            let mid = a + bar_beats / 2.0;
            let halves = (bar_units % 2 == 0).then(|| (guess_chord(&harmony, a, mid, key), guess_chord(&harmony, mid, z, key)));
            let plan: Vec<(usize, Option<(String, f64)>)> = match (halves, &whole) {
                (Some((Some(x), Some(y))), Some(w)) if x.0 != y.0 && x.1.min(y.1) > w.1 + 0.15 => {
                    vec![(b * bar_units, Some(x)), (b * bar_units + bar_units / 2, Some(y))]
                }
                _ => vec![(b * bar_units, whole)],
            };
            for (at, c) in plan {
                if let Some((name, _)) = c {
                    if last.as_deref() != Some(&name) {
                        chord_at.push((at, name.clone()));
                        last = Some(name);
                    }
                }
            }
        }
    }

    let mut breaks = vec![false; slots];
    for b in 0..bars {
        breaks[b * bar_units] = true;
    }
    for (at, _) in &chord_at {
        breaks[*at] = true;
    }
    let mono = opts.dialect == Dialect::Yue2;
    let voice_events: Vec<Vec<Event>> = voices
        .iter()
        .map(|(vname, v)| match v.as_slice() {
            [] => vec![Event { at: 0, len: slots, pitches: Vec::new() }],
            idx => {
                let shift = if opts.dialect == Dialect::Yue2 && vname == "Vocal" { 12 * opts.vocal_octave } else { 0 };
                let ts: Vec<Tone> = idx
                    .iter()
                    .flat_map(|&i| parts[i].1.iter().copied())
                    .filter_map(|t| u8::try_from(i32::from(t.pitch) + shift).ok().filter(|p| *p <= 127).map(|pitch| Tone { pitch, ..t }))
                    .collect();
                let poly = ts.iter().any(|a| ts.iter().any(|b| b.start > a.start + 1e-9 && b.start < a.end - 1e-9) || ts.iter().filter(|b| (b.start - a.start).abs() < 1e-9).count() > 1);
                if mono && poly {
                    notes.push(format!("{vname} 有复音（或几条音轨重叠），YuE2 的声部是单音：每个时刻只留最高的音"));
                }
                events(&ts, unit, slots, mono, &breaks)
            }
        })
        .map(|evs| {
            // 休止也按小节线切开。
            evs.into_iter().flat_map(|e| {
                let mut out = Vec::new();
                let mut at = e.at;
                while at < e.at + e.len {
                    let stop = ((at / bar_units + 1) * bar_units).min(e.at + e.len);
                    out.push(Event { at, len: stop - at, pitches: e.pitches.clone() });
                    at = stop;
                }
                out
            }).collect()
        })
        .collect();

    // 头部。
    let mut abc = String::new();
    let bpm = project.tempo.round().max(1.0) as u32;
    if opts.dialect == Dialect::Yue2 {
        abc.push_str(&format!("X:1\nT:\nM:{}/{}\nL:1/{denominator}\nQ:1/4={bpm}\n", ts.numerator, ts.denominator));
        abc.push_str("V: Vocal clef=treble name=\"Vocal Melody\" snm=\"Vocal\"\nV: Ins clef=treble name=\"Ins Melody\" snm=\"Inst.\"\n");
        abc.push_str(&format!("K:{}\n", key.name()));
    } else {
        let title = if project.name.is_empty() { "muser" } else { &project.name };
        abc.push_str(&format!("X:1\nT:{title}\nM:{}/{}\nL:1/{denominator}\nQ:1/4={bpm}\n", ts.numerator, ts.denominator));
        for (vi, (name, v)) in voices.iter().enumerate() {
            let low = v.iter().all(|&i| {
                let ts = &parts[i].1;
                ts.iter().map(|t| f64::from(t.pitch)).sum::<f64>() / (ts.len() as f64) < 55.0
            });
            abc.push_str(&format!("V:{} name=\"{}\" clef={}\n", vi + 1, name.replace('"', "'"), if low { "bass" } else { "treble" }));
        }
        abc.push_str(&format!("K:{}\n", key.name()));
    }

    // 分组：段落边界 + 每组最多 4 小节。没指定段落时用曲式里的。
    let form_sections: Vec<(String, u32)> = spans.iter().map(|s| (s.name.clone(), (s.length / bar_beats).round() as u32)).collect();
    let section_list = if opts.sections.is_empty() { &form_sections } else { &opts.sections };
    let mut starts: Vec<(usize, Option<&str>)> = Vec::new();
    let mut b = 0usize;
    for (name, n) in section_list {
        if b < bars {
            starts.push((b, Some(name.as_str())));
        }
        b += *n as usize;
    }
    if starts.first().is_none_or(|s| s.0 != 0) {
        starts.insert(0, (0, None));
    }
    let mut groups: Vec<(usize, usize, Option<&str>)> = Vec::new();
    for (k, &(s, label)) in starts.iter().enumerate() {
        let e = starts.get(k + 1).map_or(bars, |n| n.0).min(bars);
        let mut g = s;
        while g < e {
            groups.push((g, (g + 4).min(e), if g == s { label } else { None }));
            g += 4;
        }
    }
    let mut accs: Vec<Accidentals> = voices.iter().map(|_| Accidentals::new(key)).collect();
    for (g0, g1, label) in groups {
        if let Some(l) = label {
            abc.push_str(&format!("% {l}\n"));
        }
        for (vi, (vname, _)) in voices.iter().enumerate() {
            abc.push_str(&match opts.dialect {
                Dialect::Yue2 => format!("V: {vname}\n"),
                Dialect::Standard => format!("V:{}\n", vi + 1),
            });
            let mut line: Vec<String> = Vec::new();
            for bar in g0..g1 {
                let (a, z) = (bar * bar_units, (bar + 1) * bar_units);
                let evs: Vec<&Event> = voice_events[vi].iter().filter(|e| e.at >= a && e.at < z).collect();
                let bar_chords: Vec<(usize, String)> =
                    if vi == 0 { chord_at.iter().filter(|(c, _)| *c >= a && *c < z).cloned().collect() } else { Vec::new() };
                let empty = evs.iter().all(|e| e.pitches.is_empty());
                line.push(if empty && bar_chords.is_empty() { "Z".into() } else { write_bar(&evs, &bar_chords, key, &mut accs[vi], opts.dialect) });
            }
            // 整组都是整小节休止：写成 Zn。
            if line.len() > 1 && line.iter().all(|b| b == "Z") {
                line = vec![format!("Z{}", line.len())];
            }
            abc.push_str(&line.join("|"));
            abc.push_str("|\n");
        }
    }
    if opts.dialect == Dialect::Yue2 && section_list.is_empty() {
        notes.push("没有标段落：加 sections（如 intro:4,verse:8,chorus:8）写出 % verse 等注释，与歌词的 [Verse] 标签对应".into());
    }
    Ok(AbcExport { text: abc, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(v: serde_json::Value) -> Project {
        serde_json::from_value(v).unwrap()
    }

    /// 标准写法导出单声部，再用 muser 自己的 ABC 解析器读回：音高、起点、时值不变（含跨小节的连音线、升降号）。
    #[test]
    fn standard_round_trips_through_parser() {
        let abc = "L:1/8\nK:D\nd2 fe dc B2 | ^G4- G2 =F2 | [DFA]4 z2 c'2 | B,,8 |";
        let p = project(serde_json::json!({ "tempo": 100, "tracks": [
            { "name": "lead", "instrument": "wind.flute", "clips": [{ "start": 0, "length": 16, "abc": abc }] }] }));
        let out = export(&p, &AbcOptions::new(Dialect::Standard)).unwrap();
        let body: String = out.text.lines().filter(|l| !l.starts_with("V:")).collect::<Vec<_>>().join("\n");
        let key_line = out.text.lines().find(|l| l.starts_with("K:")).unwrap().to_string();
        let mut back = muser_core::abc::parse(&body).unwrap_or_else(|e| panic!("{e}\n{}", out.text));
        let mut orig = muser_core::abc::parse(abc).unwrap();
        let k = |n: &muser_core::Note| (n.pitch, (n.start.0 * 1000.0).round() as i64, (n.length.0 * 1000.0).round() as i64);
        back.sort_by_key(k);
        orig.sort_by_key(k);
        assert_eq!(back.iter().map(k).collect::<Vec<_>>(), orig.iter().map(k).collect::<Vec<_>>(), "{}", out.text);
        assert!(key_line == "K:D" || key_line == "K:Bm", "{}", out.text);
        assert!(out.text.contains("=F") || out.text.contains("=f"), "D 调里的 F 还原写成 =F：{}", out.text);
    }

    /// 多声部：每个声部单独读回，与各自的音轨一致（跨小节的和弦连音线、和弦里只有部分音连着、低音谱号）。
    #[test]
    fn standard_multi_voice_round_trip() {
        let tracks = [
            ("bass", "bass.finger", "L:1/8\nK:Eb\nE,,4 B,,,4 | C,,6 D,,2 |"),
            ("keys", "poly.juno_keys", "L:1/4\nK:Eb\n[EGB]3 [FA-c] | [FAc] [GB]2 z |"),
            ("lead", "wind.flute", "L:1/16\nK:Eb\ng4 f4 e8- | e2 =e2 ^f4 g8 |"),
        ];
        let p = project(serde_json::json!({ "tempo": 120, "tracks": tracks.iter().map(|(n, i, abc)|
            serde_json::json!({ "name": n, "instrument": i, "clips": [{ "start": 0, "length": 8, "abc": abc }] })).collect::<Vec<_>>() }));
        let out = export(&p, &AbcOptions::new(Dialect::Standard)).unwrap();
        let k = |n: &muser_core::Note| (n.pitch, (n.start.0 * 1000.0).round() as i64, (n.length.0 * 1000.0).round() as i64);
        let head: Vec<&str> = out.text.lines().filter(|l| l.starts_with("L:") || l.starts_with("K:")).collect();
        for (vi, (_, _, abc)) in tracks.iter().enumerate() {
            let mut body = head.join("\n");
            let mut on = false;
            for l in out.text.lines() {
                if let Some(v) = l.strip_prefix("V:") {
                    on = v.trim() == (vi + 1).to_string();
                } else if on {
                    body.push('\n');
                    body.push_str(l);
                }
            }
            let mut back: Vec<_> = muser_core::abc::parse(&body).unwrap_or_else(|e| panic!("{e}\n{body}")).iter().map(k).collect();
            let mut orig: Vec<_> = muser_core::abc::parse(abc).unwrap().iter().map(k).collect();
            back.sort();
            orig.sort();
            assert_eq!(back, orig, "声部 {}:\n{}", vi + 1, out.text);
        }
        assert!(out.text.contains("K:Eb") && out.text.contains("name=\"bass\" clef=bass"), "{}", out.text);
    }

    /// YuE2：两个声部、时值只用支持的长度、每小节满拍、和弦写在 Vocal、段落注释。
    #[test]
    fn yue2_native_shape() {
        let p = project(serde_json::json!({ "tempo": 88.4, "tracks": [
            { "name": "lead", "instrument": "wind.flute", "clips": [{ "start": 0, "length": 16, "abc": "L:1/8\nK:C\nE2G2A2G2 | c10 z2 B4 | A8 |" }] },
            { "name": "keys", "instrument": "poly.juno_keys", "clips": [{ "start": 0, "length": 16, "abc": "L:1/4\nK:C\n[CEG]4 | [FAc]4 | [Ace]4 | [GBdf]4 |" }] },
            { "name": "drums", "instrument": "drums.808", "clips": [{ "start": 0, "length": 4, "grid": { "kick": "x..." } }] }
        ] }));
        let mut opts = AbcOptions::new(Dialect::Yue2);
        opts.sections = vec![("verse".into(), 2), ("chorus".into(), 2)];
        let out = export(&p, &opts).unwrap();
        let t = &out.text;
        assert!(t.starts_with("X:1\nT:\nM:4/4\nL:1/32\nQ:1/4=88\nV: Vocal clef=treble name=\"Vocal Melody\" snm=\"Vocal\"\nV: Ins clef=treble name=\"Ins Melody\" snm=\"Inst.\"\nK:"), "{t}");
        assert!(t.contains("K:C\n% verse\nV: Vocal\n") || t.contains("K:Am\n% verse\nV: Vocal\n"), "{t}");
        assert!(t.contains("\"C\"") && t.contains("\"F\"") && t.contains("\"Am\"") && t.contains("\"G7\""), "{t}");
        assert!(t.contains("% chorus\nV: Vocal\n") && t.contains("V: Ins\nZ2|"), "{t}");
        // c 占 10 个八分 = 40 个 1/32：本小节 32，连到下一小节的 8。
        assert!(t.contains("\"F\"c32-|\n") && t.contains("V: Vocal\n\"Am\"c8z8B16|"), "{t}");
        // Vocal 升八度。
        opts.vocal_octave = 1;
        assert!(export(&p, &opts).unwrap().text.contains("\"C\"e8g8a8g8|"));
        assert!(out.notes.iter().any(|n| n.contains("drums")));
    }

    #[test]
    fn key_estimate_and_spelling() {
        let tone = |p: u8, d: f64| Tone { start: 0.0, end: d, pitch: p };
        // Eb 大调音阶，主音最长。
        let eb: Vec<Tone> = [63, 65, 67, 68, 70, 72, 74].iter().map(|&p| tone(p, if p == 63 { 4.0 } else { 1.0 })).collect();
        let k = estimate_key(&eb);
        assert_eq!(k.name(), "Eb");
        assert_eq!(k.spell(6), ('G', -1)); // 调外音按降号调拼
        let a = Key { fifths: 3, minor: true }; // F#m
        assert_eq!((a.name(), a.spell(5)), ("F#m", ('E', 1))); // 调外的 F 写成 E#
    }
}
