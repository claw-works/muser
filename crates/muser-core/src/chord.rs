//! 和弦记号与和弦进行：`Am7`、`F#m7b5`、`C/E`、`Bbmaj9`、`G7#9`；进行 `"Am7 F C G"` / `"F G | Em Am"`。

/// 一个和弦：根音、相对根音的音程（升序、含根音 0）、斜线低音。
#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
    /// 根音音级 0–11（C = 0）。
    pub root: u8,
    /// 相对根音的半音数，升序，第一个是 0；超过 12 的是延伸音（9 = 14、11 = 17、13 = 21）。
    pub intervals: Vec<u8>,
    /// 斜线和弦的低音音级（`C/E` 的 E）。
    pub bass: Option<u8>,
    /// 原文。
    pub text: String,
}

/// 和弦性质（后缀）→ 音程。取能匹配的最长后缀。
const QUALITIES: &[(&str, &[u8])] = &[
    ("m(maj7)", &[0, 3, 7, 11]), ("mMaj7", &[0, 3, 7, 11]), ("mmaj7", &[0, 3, 7, 11]),
    ("maj13", &[0, 4, 7, 11, 14, 21]), ("maj11", &[0, 4, 7, 11, 14, 17]), ("maj9", &[0, 4, 7, 11, 14]), ("maj7", &[0, 4, 7, 11]),
    ("madd9", &[0, 3, 7, 14]), ("7sus4", &[0, 5, 7, 10]), ("7sus2", &[0, 2, 7, 10]), ("m7b5", &[0, 3, 6, 10]),
    ("dim7", &[0, 3, 6, 9]), ("add9", &[0, 4, 7, 14]), ("sus2", &[0, 2, 7]), ("sus4", &[0, 5, 7]),
    ("m13", &[0, 3, 7, 10, 14, 21]), ("m11", &[0, 3, 7, 10, 14, 17]), ("m9", &[0, 3, 7, 10, 14]), ("m7", &[0, 3, 7, 10]), ("m6", &[0, 3, 7, 9]),
    ("min", &[0, 3, 7]), ("maj", &[0, 4, 7]), ("dim", &[0, 3, 6]), ("aug", &[0, 4, 8]), ("sus", &[0, 5, 7]),
    ("M7", &[0, 4, 7, 11]), ("13", &[0, 4, 7, 10, 14, 21]), ("11", &[0, 7, 10, 14, 17]), ("°7", &[0, 3, 6, 9]),
    ("ø", &[0, 3, 6, 10]), ("°", &[0, 3, 6]), ("+", &[0, 4, 8]), ("9", &[0, 4, 7, 10, 14]), ("7", &[0, 4, 7, 10]),
    ("6", &[0, 4, 7, 9]), ("5", &[0, 7]), ("m", &[0, 3, 7]), ("", &[0, 4, 7]),
];

/// 变化音：加入或替换的音程（`b5` / `#5` 替换五音）。
const ALTERATIONS: &[(&str, u8)] = &[("b13", 20), ("#11", 18), ("b9", 13), ("#9", 15), ("b5", 6), ("#5", 8)];

fn pitch_class(s: &str) -> Option<(u8, usize)> {
    let mut c = s.chars();
    let base = match c.next()? {
        'C' => 0, 'D' => 2, 'E' => 4, 'F' => 5, 'G' => 7, 'A' => 9, 'B' => 11,
        _ => return None,
    };
    let (acc, n) = match c.next() {
        Some('#') => (1, 2),
        Some('b') => (-1, 2),
        _ => (0, 1),
    };
    Some((((base + acc + 12) % 12) as u8, n))
}

impl Chord {
    /// 解析一个和弦记号。
    pub fn parse(text: &str) -> Result<Chord, String> {
        let t = text.trim();
        let err = || format!("和弦 '{t}' 看不懂：写成 根音 + 性质 [+ 变化音] [/低音]，如 Am7、F#m7b5、Cmaj9、G7#9、C/E");
        let (main, bass) = match t.split_once('/') {
            Some((m, b)) => {
                let (pc, n) = pitch_class(b).ok_or_else(err)?;
                if n != b.len() {
                    return Err(err());
                }
                (m, Some(pc))
            }
            None => (t, None),
        };
        let (root, n) = pitch_class(main).ok_or_else(err)?;
        let mut rest = &main[n..];
        // 最长匹配（m7b5 而不是 m7，maj7 而不是 m）。
        let &(q, base) = QUALITIES.iter().filter(|(q, _)| rest.starts_with(q)).max_by_key(|(q, _)| q.len()).ok_or_else(err)?;
        rest = &rest[q.len()..];
        let mut intervals: Vec<u8> = base.to_vec();
        while !rest.is_empty() {
            let r = rest.trim_start_matches('(').trim_start_matches(',');
            let Some(&(a, iv)) = ALTERATIONS.iter().find(|(a, _)| r.starts_with(a)) else { return Err(err()) };
            if iv == 6 || iv == 8 {
                intervals.retain(|&i| i != 7);
            }
            intervals.push(iv);
            rest = r[a.len()..].trim_start_matches(')');
        }
        intervals.sort_unstable();
        intervals.dedup();
        Ok(Chord { root, intervals, bass, text: t.to_string() })
    }

    /// 三音（大三 4 / 小三 3；挂留和弦是挂留音 2 或 5；强力和弦没有，退回五音）。
    pub fn third(&self) -> u8 {
        *self.intervals.iter().find(|&&i| (2..=5).contains(&i)).unwrap_or(&7)
    }
    /// 五音（减 6 / 纯 7 / 增 8）。
    pub fn fifth(&self) -> u8 {
        *self.intervals.iter().find(|&&i| (6..=8).contains(&i)).unwrap_or(&7)
    }
    /// 七音（减七 9 / 小七 10 / 大七 11）；没有七音的和弦为 None。六和弦的六音不算七音。
    pub fn seventh(&self) -> Option<u8> {
        let six = self.intervals.contains(&9) && !self.intervals.contains(&6);
        self.intervals.iter().copied().find(|&i| (9..=11).contains(&i) && !(i == 9 && six))
    }
    /// 低音音级：斜线和弦的低音，否则根音。
    pub fn bass_pc(&self) -> u8 {
        self.bass.unwrap_or(self.root)
    }
    /// 和弦里的音级。
    pub fn pitch_classes(&self) -> Vec<u8> {
        let mut pcs: Vec<u8> = self.intervals.iter().map(|i| (self.root + i) % 12).collect();
        if let Some(b) = self.bass {
            pcs.push(b);
        }
        pcs.sort_unstable();
        pcs.dedup();
        pcs
    }
    /// 移调（记号一起改写：原来用降号的仍用降号，否则用升号）。
    pub fn transposed(&self, semitones: i32) -> Chord {
        if semitones.rem_euclid(12) == 0 {
            return self.clone();
        }
        let m = |pc: u8| ((i32::from(pc) + semitones).rem_euclid(12)) as u8;
        let flats = self.text.chars().nth(1) == Some('b');
        let spell = |pc: u8| {
            const SHARP: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
            const FLAT: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
            if flats { FLAT[usize::from(pc)] } else { SHARP[usize::from(pc)] }
        };
        let (main, _) = self.text.split_once('/').unwrap_or((&self.text, ""));
        let root_len = pitch_class(main).map_or(1, |(_, n)| n);
        let mut text = format!("{}{}", spell(m(self.root)), &main[root_len..]);
        if let Some(b) = self.bass {
            text = format!("{text}/{}", spell(m(b)));
        }
        Chord { root: m(self.root), intervals: self.intervals.clone(), bass: self.bass.map(m), text }
    }
}

/// 和弦进行里的一个和弦：从段落开头算的起点与时长（拍）。
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub start: f64,
    pub length: f64,
    pub chord: Chord,
}

/// 解析和弦进行并铺满 `bars` 小节（每小节 `bar_beats` 拍）。
/// 没有 `|` 时每个和弦一小节；有 `|` 时按小节分开，一小节里的几个和弦平分这一小节。进行比段落短时循环。
pub fn progression(text: &str, bars: u32, bar_beats: f64) -> Result<Vec<Placed>, String> {
    let groups: Vec<Vec<&str>> = if text.contains('|') {
        text.split('|').map(|b| b.split_whitespace().collect::<Vec<_>>()).filter(|b| !b.is_empty()).collect()
    } else {
        text.split_whitespace().map(|c| vec![c]).collect()
    };
    if groups.is_empty() {
        return Err("chords 为空".into());
    }
    let parsed: Vec<Vec<Chord>> = groups.iter().map(|g| g.iter().map(|c| Chord::parse(c)).collect::<Result<_, _>>()).collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for bar in 0..bars as usize {
        let g = &parsed[bar % parsed.len()];
        let each = bar_beats / g.len() as f64;
        for (k, c) in g.iter().enumerate() {
            out.push(Placed { start: bar as f64 * bar_beats + k as f64 * each, length: each, chord: c.clone() });
        }
    }
    Ok(out)
}

/// 某一时刻（拍，段落内）的和弦。
pub fn at(chords: &[Placed], beat: f64) -> Option<&Chord> {
    chords.iter().rev().find(|p| p.start <= beat + 1e-9).map(|p| &p.chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        let c = Chord::parse("F#m7b5").unwrap();
        assert_eq!((c.root, c.intervals.clone(), c.third(), c.fifth(), c.seventh()), (6, vec![0, 3, 6, 10], 3, 6, Some(10)));
        let c = Chord::parse("C/E").unwrap();
        assert_eq!((c.root, c.bass_pc(), c.seventh()), (0, 4, None));
        assert_eq!(Chord::parse("G7#9").unwrap().intervals, vec![0, 4, 7, 10, 15]);
        assert_eq!(Chord::parse("Bbmaj9").unwrap().root, 10);
        assert_eq!(Chord::parse("C6").unwrap().seventh(), None);
        assert_eq!(Chord::parse("Bdim7").unwrap().seventh(), Some(9));
        assert_eq!(Chord::parse("Dsus4").unwrap().third(), 5);
        assert!(Chord::parse("Hm").is_err() && Chord::parse("Cxyz").is_err());
    }

    #[test]
    fn progressions() {
        let p = progression("Am7 F", 4, 4.0).unwrap();
        assert_eq!(p.iter().map(|x| (x.start, x.chord.text.as_str())).collect::<Vec<_>>(), [(0.0, "Am7"), (4.0, "F"), (8.0, "Am7"), (12.0, "F")]);
        let p = progression("F G | Em", 2, 4.0).unwrap();
        assert_eq!(p.iter().map(|x| (x.start, x.length)).collect::<Vec<_>>(), [(0.0, 2.0), (2.0, 2.0), (4.0, 4.0)]);
        assert_eq!(at(&p, 3.0).unwrap().text, "G");
        assert_eq!(Chord::parse("F#m7/C#").unwrap().transposed(2).text, "G#m7/D#");
        assert_eq!(Chord::parse("Bbmaj7").unwrap().transposed(2).text, "Cmaj7");
        assert_eq!(Chord::parse("Em").unwrap().transposed(-1).text, "D#m");
    }
}
