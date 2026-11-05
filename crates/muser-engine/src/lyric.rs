//! 唱音节：`choir.*` 乐器的音符写了 `lyric`（拼音或英文拼法）时，元音换成音节里的元音（哼唱）。
//!
//! 0.4 只唱元音：下面的辅音模拟（噪声叠在人声上）实测听起来像两个声音，默认关，`instrument_params."@consonants"`
//! 给正数时打开，仅供实验。能听出字的辅音要在 0.5 的合成器里做（共振峰轨迹、协同发音）。
//!
//! 不是真人声：元音是 manguebyte 的共振峰人声（a e i o u 与其间的过渡），辅音在这里用滤波噪声、爆破噪声、
//! 先哼后开的鼻音和元音滑动做出来，叠在人声上一起走音轨的效果器。像真人一样，辅音提前几十毫秒开始，元音落在拍上。

/// 起头的辅音。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Onset {
    None,
    /// 摩擦音：滤波噪声（s 最高、sh 低一些、f 宽而轻、h 最宽）。
    Fricative { hz: f32, q: f32, level: f32 },
    /// 爆破音：一下短促的噪声；不送气（b d g）只有爆破，送气（p t k）后面再跟一小段气声。
    Plosive { hz: f32, aspirated: bool },
    /// 塞擦音（zh ch j q z c）：爆破接一小段摩擦。
    Affricate { hz: f32, q: f32, aspirated: bool },
    /// 鼻音（m n）：先一小段闭着嘴的哼声，再打开元音。
    Nasal,
    /// 近音（l r w y）：不加噪声，从相近的元音形状快速滑到目标元音。
    Glide { from: f32 },
}

/// 音节结尾。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coda {
    None,
    /// -n / -ng / -m：结尾合上嘴，变成哼声。
    Nasal,
    /// -s：结尾一下"嘶"。
    Hiss,
}

/// 解析好的音节：起头的辅音、依次唱的元音（0–4：a e i o u，之间是过渡）、结尾。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Syl {
    pub onset: Onset,
    pub vowels: [f32; 3],
    pub n: u8,
    pub coda: Coda,
}

const A: f32 = 0.0;
const E: f32 = 1.0;
const I: f32 = 2.0;
const O: f32 = 3.0;
const U: f32 = 4.0;
/// "鹅"、英文的 "uh"：a 与 e 之间。
const SCHWA: f32 = 0.6;
/// 辅音相对内置人声（校准到约 −20 LUFS）的总音量。
const CONSONANT_GAIN: f32 = 0.22;

/// 闭着嘴的哼声（m、n 与 -n / -ng 结尾）用的元音形状。
pub const NASAL_VOWEL: f32 = U;

/// 解析一个音节（拼音或英文拼法，大小写不敏感，声调数字忽略）。
pub fn parse(text: &str) -> Syl {
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphabetic() || *c == 'ü').collect();
    let s = s.replace('v', "ü");
    // 起头的辅音（长的先比）。
    const ONSETS: &[&str] = &["zh", "ch", "sh", "th", "ph", "wh", "qu", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s", "y", "w", "v"];
    let ini = ONSETS.iter().find(|o| s.starts_with(**o)).copied().unwrap_or("");
    let mut rest = &s[ini.len()..];
    let onset = match ini {
        "s" => Onset::Fricative { hz: 7000.0, q: 2.5, level: 0.5 },
        "sh" => Onset::Fricative { hz: 3200.0, q: 2.0, level: 0.55 },
        "x" => Onset::Fricative { hz: 4800.0, q: 2.2, level: 0.5 },
        "f" | "ph" | "th" => Onset::Fricative { hz: 5000.0, q: 0.6, level: 0.22 },
        "h" => Onset::Fricative { hz: 1500.0, q: 0.5, level: 0.25 },
        "b" => Onset::Plosive { hz: 900.0, aspirated: false },
        "p" => Onset::Plosive { hz: 900.0, aspirated: true },
        "d" => Onset::Plosive { hz: 4000.0, aspirated: false },
        "t" => Onset::Plosive { hz: 4000.0, aspirated: true },
        "g" => Onset::Plosive { hz: 2000.0, aspirated: false },
        "k" | "qu" => Onset::Plosive { hz: 2000.0, aspirated: true },
        "zh" | "j" => Onset::Affricate { hz: 3600.0, q: 2.0, aspirated: false },
        "ch" | "q" => Onset::Affricate { hz: 3600.0, q: 2.0, aspirated: true },
        "z" => Onset::Affricate { hz: 6500.0, q: 2.5, aspirated: false },
        "c" => Onset::Affricate { hz: 6500.0, q: 2.5, aspirated: true },
        "m" | "n" => Onset::Nasal,
        "l" => Onset::Glide { from: E },
        "r" => Onset::Glide { from: O },
        "w" | "wh" => Onset::Glide { from: U },
        "y" => Onset::Glide { from: I },
        "v" => Onset::Fricative { hz: 4000.0, q: 0.6, level: 0.15 },
        _ => Onset::None,
    };
    // 结尾。
    let mut coda = Coda::None;
    for (end, c) in [("ng", Coda::Nasal), ("n", Coda::Nasal), ("m", Coda::Nasal), ("s", Coda::Hiss)] {
        if rest.len() > end.len() && rest.ends_with(end) {
            rest = &rest[..rest.len() - end.len()];
            coda = c;
            break;
        }
    }
    // 剩下的元音字母（中间的其它辅音忽略）。
    let jqx = matches!(ini, "j" | "q" | "x" | "y");
    // zhi chi shi ri zi ci si 的 i 是舌尖元音（不是 i）；有韵尾的（sing、shin）不算。
    let apical = matches!(ini, "zh" | "ch" | "sh" | "r" | "z" | "c" | "s") && rest == "i" && coda == Coda::None;
    let letters: Vec<char> = rest.chars().filter(|c| "aeiouüy".contains(*c)).collect();
    let mut vowels: Vec<f32> = Vec::new();
    for (k, &c) in letters.iter().enumerate() {
        let v = match c {
            'a' => A,
            'e' => {
                // 拼音单独的 e（de、ge、he……）是"鹅"；ie、ue、ei 里的 e 是 e。
                let alone = letters.len() == 1 || (k == 0 && letters.get(1) == Some(&'r'));
                if alone && !jqx && !s.ends_with("ee") { SCHWA } else { E }
            }
            'i' | 'y' => {
                if apical { SCHWA + 0.4 } else { I }
            }
            'o' => O,
            'u' => {
                if jqx && k == 0 { I } else { U }
            }
            'ü' => I,
            _ => continue,
        };
        if vowels.last() != Some(&v) {
            vowels.push(v);
        }
    }
    if s.ends_with("er") && letters.len() == 1 {
        vowels = vec![SCHWA];
    }
    if vowels.is_empty() {
        vowels.push(SCHWA);
    }
    let n = vowels.len().min(3);
    let mut arr = [vowels[0]; 3];
    arr[..n].copy_from_slice(&vowels[..n]);
    Syl { onset, vowels: arr, n: n as u8, coda }
}

impl Syl {
    /// 辅音比元音早开始多少秒（元音落在拍上）。
    pub fn lead(&self) -> f64 {
        match self.onset {
            Onset::None => 0.0,
            Onset::Fricative { level, .. } => if level < 0.3 { 0.08 } else { 0.12 },
            Onset::Plosive { aspirated, .. } => if aspirated { 0.05 } else { 0.02 },
            Onset::Affricate { .. } => 0.09,
            Onset::Nasal => 0.07,
            Onset::Glide { .. } => 0.05,
        }
    }

    /// 人声要提前开口（鼻音、近音：辅音本身是有声的）。
    pub fn voiced_onset(&self) -> bool {
        matches!(self.onset, Onset::Nasal | Onset::Glide { .. })
    }

    /// 元音在音符里换的时刻（占音符时长的比例）与目标：第一个元音在开头。
    /// 两个元音：前一个是 i / u 这类介音时它很短（ia、uo），否则主元音占大半（ai、ao）。
    pub fn vowel_plan(&self) -> Vec<(f64, f32)> {
        let v = &self.vowels[..self.n as usize];
        match v.len() {
            1 => vec![(0.0, v[0])],
            2 if v[0] == I || v[0] == U => vec![(0.0, v[0]), (0.18, v[1])],
            2 => vec![(0.0, v[0]), (0.62, v[1])],
            _ => vec![(0.0, v[0]), (0.18, v[1]), (0.7, v[2])],
        }
    }
}

/// 音节原文（放进 MIDI 的 lyric 事件），定长以便跟着演奏动作复制；超过 24 字节的截断。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Text {
    bytes: [u8; 24],
    len: u8,
}

impl Text {
    pub fn new(s: &str) -> Self {
        let mut end = s.len().min(24);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0u8; 24];
        bytes[..end].copy_from_slice(&s.as_bytes()[..end]);
        Self { bytes, len: end as u8 }
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

/// 一段辅音噪声。
#[derive(Debug, Clone, Copy)]
struct Burst {
    /// 起点（绝对帧）与长度（帧）。
    start: usize,
    len: usize,
    level: f32,
    /// 起音、收尾（帧）。
    attack: usize,
    release: usize,
    biquad: Biquad,
}

/// 叠在人声上的辅音：按帧时间排好的噪声段，每块加到输出里。
pub struct Consonants {
    bursts: Vec<Burst>,
    rng: u64,
    sr: f32,
    gain: f32,
}

impl Consonants {
    /// `gain`：辅音音量的倍数（音轨的 `instrument_params."@consonants"`；默认 0，即不发辅音）。
    pub fn new(sample_rate: f64, gain: f64) -> Self {
        Self { bursts: Vec::new(), rng: 0x9E37_79B9_7F4A_7C15, sr: sample_rate as f32, gain: gain.clamp(0.0, 8.0) as f32 }
    }

    /// 音节 `syl` 在 `start` 帧开始（元音在 `start + lead` 帧）、音符长 `len` 帧、力度 `velocity`：排好它的辅音。
    pub fn sing(&mut self, syl: &Syl, start: usize, lead: usize, len: usize, velocity: f32) {
        let ms = |x: f32| (x * 0.001 * self.sr) as usize;
        // 辅音比元音轻得多（摩擦音约低 12 dB，爆破只是一下）：整体按校准后的人声音量缩小。
        let v = self.gain * CONSONANT_GAIN * (0.35 + 0.65 * velocity.clamp(0.0, 1.0));
        let mut push = |start: usize, len: usize, hz: f32, q: f32, level: f32, attack: usize, release: usize, sr: f32| {
            self.bursts.push(Burst { start, len: len.max(1), level: level * v, attack: attack.max(1), release: release.max(1), biquad: Biquad::bandpass(hz, q, sr) });
        };
        let sr = self.sr;
        match syl.onset {
            Onset::Fricative { hz, q, level } => push(start, lead + ms(30.0), hz, q, level, ms(20.0), ms(35.0), sr),
            Onset::Plosive { hz, aspirated } => {
                push(start, ms(14.0), hz, 1.0, if aspirated { 0.9 } else { 0.55 }, ms(1.0), ms(9.0), sr);
                if aspirated {
                    push(start + ms(6.0), lead.saturating_sub(ms(6.0)) + ms(15.0), 1600.0, 0.5, 0.28, ms(5.0), ms(20.0), sr);
                }
            }
            Onset::Affricate { hz, q, aspirated } => {
                push(start, ms(8.0), hz, 1.2, 0.6, ms(1.0), ms(5.0), sr);
                push(start + ms(6.0), lead.saturating_sub(ms(6.0)) + ms(20.0), hz, q, if aspirated { 0.55 } else { 0.4 }, ms(8.0), ms(25.0), sr);
            }
            Onset::None | Onset::Nasal | Onset::Glide { .. } => {}
        }
        if syl.coda == Coda::Hiss && len > ms(120.0) {
            push(start + lead + len - ms(90.0), ms(110.0), 7000.0, 2.5, 0.42, ms(20.0), ms(40.0), sr);
        }
    }

    /// 把 `[frame, frame + 块长)` 里的辅音加进 `block`（两个声道一样）。
    pub fn add_to(&mut self, block: &mut manguebyte::prelude::AudioBuffer<f32>, frame: usize) {
        let n = block.num_frames();
        let end = frame + n;
        if self.bursts.iter().all(|b| b.start >= end || b.start + b.len <= frame) {
            self.bursts.retain(|b| b.start + b.len > frame);
            return;
        }
        let mut buf = vec![0.0f32; n];
        for b in self.bursts.iter_mut().filter(|b| b.start < end && b.start + b.len > frame) {
            for (k, out) in buf.iter_mut().enumerate() {
                let f = frame + k;
                if f < b.start || f >= b.start + b.len {
                    continue;
                }
                let i = f - b.start;
                let env = (i as f32 / b.attack as f32).min(1.0) * (((b.len - i) as f32) / b.release as f32).min(1.0);
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                let noise = (self.rng >> 40) as f32 / (1u64 << 23) as f32 * 2.0 - 1.0;
                *out += b.biquad.run(noise) * env * b.level;
            }
        }
        for ch in 0..block.num_channels() {
            for (x, a) in block.channel_mut(ch).iter_mut().zip(&buf) {
                *x += a;
            }
        }
        self.bursts.retain(|b| b.start + b.len > end);
    }
}

/// RBJ 带通（峰值增益 1）。
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn bandpass(hz: f32, q: f32, sr: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * hz.min(sr * 0.45) / sr;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self { b0: alpha / a0, b2: -alpha / a0, a1: -2.0 * w.cos() / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinyin_and_english_syllables() {
        let p = parse;
        assert_eq!(p("la").onset, Onset::Glide { from: E });
        assert_eq!(&p("la").vowels[..1], &[A]);
        assert!(matches!(p("shi").onset, Onset::Fricative { hz, .. } if hz < 4000.0));
        assert!(p("shi").vowels[0] < 1.5, "shi 的 i 是舌尖元音，不是 i");
        assert_eq!(&p("xi").vowels[..1], &[I]);
        assert_eq!(&p("ju").vowels[..1], &[I], "j 后的 u 是 ü");
        assert_eq!(p("hai").vowel_plan(), vec![(0.0, A), (0.62, I)]);
        assert_eq!(p("jia").vowel_plan(), vec![(0.0, I), (0.18, A)]);
        assert_eq!(p("ge").vowels[0], SCHWA);
        assert_eq!(p("xie").vowel_plan(), vec![(0.0, I), (0.18, E)]);
        assert_eq!(p("ming").coda, Coda::Nasal);
        assert_eq!(p("ming").onset, Onset::Nasal);
        assert_eq!(p("sing").coda, Coda::Nasal);
        assert_eq!(&p("sing").vowels[..1], &[I], "sing 的 i 是 i");
        assert!(matches!(p("ta").onset, Onset::Plosive { aspirated: true, .. }));
        assert!(matches!(p("da").onset, Onset::Plosive { aspirated: false, .. }));
        assert!(matches!(p("zhong").onset, Onset::Affricate { aspirated: false, .. }));
        assert_eq!(p("a").onset, Onset::None);
        assert_eq!(p("Ni3").onset, Onset::Nasal, "大小写与声调数字忽略");
        assert_eq!(p("yes").coda, Coda::Hiss);
        assert_eq!(p("ye").vowels[0], E, "ye 是 ie");
    }

    #[test]
    fn consonants_add_band_limited_noise_then_stop() {
        let mut c = Consonants::new(48_000.0, 1.0);
        c.sing(&parse("sa"), 100, 4320, 24_000, 0.8);
        let mut block = manguebyte::prelude::AudioBuffer::<f32>::new(2, 8192);
        c.add_to(&mut block, 0);
        let energy: f32 = block.channel(0).iter().map(|x| x * x).sum();
        assert!(energy > 0.05 && block.channel(0)[..100].iter().all(|&x| x == 0.0));
        let mut later = manguebyte::prelude::AudioBuffer::<f32>::new(2, 8192);
        c.add_to(&mut later, 8192);
        assert!(later.channel(0).iter().all(|&x| x == 0.0), "辅音过去以后不再出声");
    }
}

/// 卡拉 OK 的一行：起止（秒）与逐字的起止。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct KaraokeLine {
    pub start: f64,
    pub end: f64,
    pub tokens: Vec<KaraokeToken>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct KaraokeToken {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// 引导旋律：第一条写了 `lyric` 的音轨上带歌词的音符，`(起点秒, 终点秒, 音高)`，按时间排好（和弦只取最高音）。
/// 卡拉 OK 和 `muser takes --timing` 都以它为准。
pub fn guide_notes(project: &muser_core::Project) -> Vec<(f64, f64, u8)> {
    let Ok(p) = crate::arrange::arranged(project) else { return Vec::new() };
    let tm = p.tempo_map();
    let mut out: Vec<(f64, f64, u8)> = Vec::new();
    for (ti, t) in p.tracks.iter().enumerate() {
        for (ci, c) in t.clips.iter().enumerate() {
            for n in crate::clip_notes(&p, ti, ci).unwrap_or_default() {
                if n.lyric.is_some() {
                    let s = c.start.0 + n.start.0;
                    out.push((tm.secs_at(s), tm.secs_at(s + n.length.0.min(c.length.0 - n.start.0)), n.pitch));
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.2.cmp(&a.2)));
    out.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-9);
    out
}

/// 卡拉 OK 歌词：唱的时间来自第一条写了 `lyric` 的音轨（展开后的音符），显示的字来自工程的 `lyrics`——
/// 每一行（去掉 `[A]` 这类段落标记）里的汉字一个字对一个音节，英文按词；字数和音节数对不上时显示音节本身。
/// 没有 `lyrics` 时按音节显示，休止超过一拍半就换行。
pub fn karaoke(project: &muser_core::Project) -> Vec<KaraokeLine> {
    let Ok(p) = crate::arrange::arranged(project) else { return Vec::new() };
    let tm = p.tempo_map();
    let mut syl: Vec<(f64, f64, String)> = Vec::new();
    for (ti, t) in p.tracks.iter().enumerate() {
        for (ci, c) in t.clips.iter().enumerate() {
            for n in crate::clip_notes(&p, ti, ci).unwrap_or_default() {
                if let Some(l) = &n.lyric {
                    let s = c.start.0 + n.start.0;
                    syl.push((tm.secs_at(s), tm.secs_at(s + n.length.0.min(c.length.0 - n.start.0)), l.clone()));
                }
            }
        }
        if !syl.is_empty() {
            break;
        }
    }
    syl.sort_by(|a, b| a.0.total_cmp(&b.0));
    syl.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-9); // 和弦里同一个音节只算一次
    if syl.is_empty() {
        return Vec::new();
    }
    // 显示的字，按行。
    let lines: Vec<Vec<String>> = project
        .lyrics
        .as_deref()
        .unwrap_or("")
        .lines()
        .map(|l| match l.trim().strip_prefix('[') {
            Some(rest) => rest.split_once(']').map_or("", |(_, r)| r),
            None => l,
        })
        // 一句一行：逗号、句号也换行。
        .flat_map(|l| l.split(['，', ',', '。', '；', ';', '！', '？']))
        .map(tokens)
        .filter(|t| !t.is_empty())
        .collect();
    let total: usize = lines.iter().map(Vec::len).sum();
    let mut out = Vec::new();
    if total == syl.len() {
        let mut k = 0;
        for line in lines {
            let toks: Vec<KaraokeToken> = line
                .into_iter()
                .map(|text| {
                    let (s, e, _) = &syl[k];
                    k += 1;
                    KaraokeToken { text, start: *s, end: *e }
                })
                .collect();
            out.push(KaraokeLine { start: toks[0].start, end: toks.last().map_or(0.0, |t| t.end), tokens: toks });
        }
    } else {
        let mut cur: Vec<KaraokeToken> = Vec::new();
        for (s, e, text) in syl {
            if cur.last().is_some_and(|l| s - l.end > 1.5 * 60.0 / tm.bpm_at(tm.beat_at(l.end))) {
                out.push(KaraokeLine { start: cur[0].start, end: cur.last().unwrap().end, tokens: std::mem::take(&mut cur) });
            }
            cur.push(KaraokeToken { text, start: s, end: e });
        }
        if !cur.is_empty() {
            out.push(KaraokeLine { start: cur[0].start, end: cur.last().unwrap().end, tokens: cur });
        }
    }
    out
}

/// 一行歌词拆成要唱的单位：汉字（和其它 CJK 字）一个一个，其余按词；标点、括号去掉。
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in line.chars() {
        let cjk = matches!(c as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF);
        if cjk || !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if cjk {
                out.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod karaoke_tests {
    use super::*;

    #[test]
    fn characters_follow_the_sung_syllables() {
        let p: muser_core::Project = serde_json::from_value(serde_json::json!({ "tempo": 60,
            "lyrics": "[A] 月亮升起，\n风吹（哼）",
            "tracks": [{ "name": "v", "instrument": "choir.solo", "clips": [{ "start": 0, "length": 16,
                "abc": "L:1/4\nK:C\nC D E F | z4 | G A B c |\nw: yue liang sheng qi | feng chui heng" }] }] }))
        .unwrap();
        let k = karaoke(&p);
        assert_eq!(k.len(), 2, "逗号后面是空的，不算一行");
        assert_eq!(k[0].tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["月", "亮", "升", "起"]);
        assert_eq!((k[0].tokens[1].start, k[0].tokens[1].end), (1.0, 2.0));
        assert_eq!(k[1].tokens.iter().map(|t| t.text.as_str()).collect::<String>(), "风吹哼");
        assert_eq!(k[1].start, 8.0);
        // 字数对不上：按音节显示、按休止换行。
        let mut q = p.clone();
        q.lyrics = Some("月亮".into());
        let k = karaoke(&q);
        assert_eq!(k.len(), 2);
        assert_eq!(k[1].tokens[0].text, "feng");
    }
}
