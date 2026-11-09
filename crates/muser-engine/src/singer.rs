//! 唱歌合成（`yinse.voice.*`）：muser 自己的共振峰歌声，不用采样。
//!
//! 结构照 Klatt（1980）的源–滤波模型：
//! - **声源**：声门脉冲（声门开、合一周的气流的导数：开相里起落、闭相里为零），开商（张开占一周的多少）决定实或虚，
//!   频谱倾斜决定亮暗；每一周的长短、强弱有一点不规则（jitter、shimmer）；气声是跟着声门开合一起出来的噪声。
//! - **声道**：五个共振峰串起来（F1–F5，各自的频率与带宽），目标随音素变化、平滑地滑过去（协同发音）。
//!   高音时第一共振峰抬到基音之上（女高音的"共振峰调整"），不然高音唱不响。
//! - **歌词**：音符的 `lyric` 是拼音（或英文拼法）。这一版唱韵母：单元音、复韵母在音符里滑过去（ai、iao …），
//!   声母（辅音）、鼻韵尾在下一步。
//!
//! 单音：连着的音（重叠）滑过去，像连唱；断开的音重新起声。

use manguebyte::dsp::Tuning;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Voice};

use core::f64::consts::{PI, TAU};

// —— 元音 ——

/// 元音（共振峰目标）。`ApicalFront` 是 z c s 后面的 i（"资"），`ApicalBack` 是 zh ch sh r 后面的 i（"知"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vowel {
    A,
    O,
    /// 汉语的 e（"鹅"，ɤ）。
    E,
    /// ê（"耶"里的 e，ɛ）。
    Eh,
    I,
    U,
    /// ü（y）。
    V,
    Er,
    ApicalFront,
    ApicalBack,
    /// 英文的 uh（ə）。
    Schwa,
}

/// 男声（成人）的 F1–F5（Hz）：汉语普通话元音的常见测量值，F4、F5 取典型值。
fn male(v: Vowel) -> [f64; 5] {
    match v {
        Vowel::A => [850.0, 1220.0, 2810.0, 3500.0, 4500.0],
        Vowel::O => [590.0, 880.0, 2540.0, 3400.0, 4400.0],
        Vowel::E => [570.0, 1260.0, 2600.0, 3400.0, 4400.0],
        Vowel::Eh => [530.0, 1840.0, 2480.0, 3500.0, 4500.0],
        Vowel::I => [290.0, 2350.0, 3200.0, 3700.0, 4700.0],
        Vowel::U => [330.0, 700.0, 2500.0, 3300.0, 4300.0],
        Vowel::V => [290.0, 1950.0, 2400.0, 3400.0, 4500.0],
        Vowel::Er => [550.0, 1450.0, 1850.0, 3200.0, 4300.0],
        Vowel::ApicalFront => [380.0, 1450.0, 2600.0, 3500.0, 4500.0],
        Vowel::ApicalBack => [390.0, 1650.0, 2200.0, 3300.0, 4400.0],
        Vowel::Schwa => [500.0, 1400.0, 2500.0, 3400.0, 4400.0],
    }
}

/// 带宽（Hz）：F1 窄、越往上越宽。
const BANDWIDTHS: [f64; 5] = [80.0, 100.0, 150.0, 220.0, 300.0];

/// 声部：共振峰按比例（声道长短）、音色。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Part {
    /// 共振峰相对男声（成人）的比例。
    formants: f64,
    /// 开商（0.4 实、0.7 虚）。
    open: f64,
    /// 气声（与有声部分之比）。
    breath: f64,
}

const PARTS: [Part; 5] = [
    // 女高、女中、男高、男低、童声
    Part { formants: 1.17, open: 0.6, breath: 0.06 },
    Part { formants: 1.1, open: 0.55, breath: 0.05 },
    Part { formants: 1.0, open: 0.5, breath: 0.04 },
    Part { formants: 0.93, open: 0.45, breath: 0.04 },
    Part { formants: 1.28, open: 0.62, breath: 0.07 },
];
pub(crate) const PART_NAMES: &[&str] = &["soprano", "alto", "tenor", "bass", "child"];

// —— 拼音 ——

/// 一个音节里要唱的元音（依次滑过去，最多三个）与每个元音占这个音的比例（介音短、韵腹长、韵尾短）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Syllable {
    pub vowels: [Vowel; 3],
    pub shares: [f64; 3],
    pub n: usize,
}

impl Syllable {
    fn one(v: Vowel) -> Self {
        Self { vowels: [v; 3], shares: [1.0, 0.0, 0.0], n: 1 }
    }
    fn two(a: Vowel, b: Vowel, glide_first: bool) -> Self {
        let shares = if glide_first { [0.12, 0.88, 0.0] } else { [0.7, 0.3, 0.0] };
        Self { vowels: [a, b, b], shares, n: 2 }
    }
    fn three(a: Vowel, b: Vowel, c: Vowel) -> Self {
        Self { vowels: [a, b, c], shares: [0.12, 0.6, 0.28], n: 3 }
    }
}

/// 拼音（或英文拼法）→ 要唱的元音。声母只用来认出"资 / 知"的 i、j q x y 后面的 u（= ü）；鼻韵尾这一版不唱。
pub fn syllable(text: &str) -> Syllable {
    use Vowel::*;
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphabetic() || *c == 'ü').collect::<String>().replace('v', "ü");
    const INITIALS: &[&str] = &["zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s", "y", "w"];
    let ini = INITIALS.iter().find(|i| s.starts_with(**i)).copied().unwrap_or("");
    let mut fin = s[ini.len()..].to_string();
    // y / w 开头：yi = i、ya = ia、wu = u、wa = ua、yu = ü …
    match ini {
        "y" => {
            fin = if let Some(r) = fin.strip_prefix('u') { format!("ü{r}") } else if fin.starts_with('i') { fin } else { format!("i{fin}") };
        }
        "w" => {
            if !fin.starts_with('u') {
                fin = format!("u{fin}");
            }
        }
        "j" | "q" | "x" if fin.starts_with('u') => fin = format!("ü{}", fin.strip_prefix('u').unwrap_or_default()),
        _ => {}
    }
    // 鼻韵尾：这一版唱成元音（n、ng 去掉；ang 的 a 偏后、ong 是 u）。
    let (core, _nasal) = if let Some(c) = fin.strip_suffix("ng") { (c.to_string(), true) } else if let Some(c) = fin.strip_suffix('n') { (c.to_string(), true) } else { (fin.clone(), false) };
    let apical = matches!(ini, "z" | "c" | "s");
    let retro = matches!(ini, "zh" | "ch" | "sh" | "r");
    match core.as_str() {
        "a" => Syllable::one(A),
        "o" if fin == "ong" => Syllable::one(U),
        "o" => Syllable::one(O),
        "e" => Syllable::one(if fin == "en" || fin == "eng" { Schwa } else { E }),
        "ê" => Syllable::one(Eh),
        "i" if apical => Syllable::one(ApicalFront),
        "i" if retro => Syllable::one(ApicalBack),
        "i" => Syllable::one(I),
        "u" => Syllable::one(U),
        "ü" => Syllable::one(V),
        "er" => Syllable::one(Er),
        "ai" => Syllable::two(A, I, false),
        "ei" => Syllable::two(Eh, I, false),
        "ao" => Syllable::two(A, U, false),
        "ou" => Syllable::two(O, U, false),
        "ia" => Syllable::two(I, A, true),
        "ie" => Syllable::two(I, Eh, true),
        "ua" => Syllable::two(U, A, true),
        "uo" => Syllable::two(U, O, true),
        "üe" | "ue" => Syllable::two(V, Eh, true),
        "iao" => Syllable::three(I, A, U),
        "iu" | "iou" => Syllable::three(I, O, U),
        "uai" => Syllable::three(U, A, I),
        "ui" | "uei" => Syllable::three(U, Eh, I),
        // iong（"用"）：圆唇的 u；üan（"远"）的 a 接近 ê。
        "io" => Syllable::two(I, U, true),
        "üa" => Syllable::two(V, Eh, true),
        "" => Syllable::one(Schwa),
        other => english(other),
    }
}

/// 不是拼音的（英文拼法）：按第一个元音字母唱，简单近似（英文的完整规则在后面）。
fn english(s: &str) -> Syllable {
    use Vowel::*;
    let v = s.chars().find(|c| "aeiouy".contains(*c)).unwrap_or('a');
    Syllable::one(match v {
        'a' => A,
        'e' => Eh,
        'i' | 'y' => I,
        'o' => O,
        _ => U,
    })
}

// —— 参数 ——

/// 歌声的音色。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SingerParams {
    /// 声部（[`PART_NAMES`] 里的序号）。
    pub part: f64,
    /// 共振峰整体比例（声道大小）：1 正常，0.9 更大更厚，1.1 更小更细。
    pub formant_shift: f64,
    /// 气声多少（倍数，1 = 声部的默认）。
    pub breath: f64,
    /// 亮度：0 暗、1 亮（声门脉冲的频谱倾斜）。
    pub brightness: f64,
    pub vibrato_depth: f64,
    pub vibrato_rate: f64,
    pub vibrato_delay: f64,
    /// 连唱时滑到下一个音的时间（秒）。
    pub glide: f64,
    /// 每一周的不规则（音高、音量），0–1。
    pub jitter: f64,
    pub attack: f64,
    pub release: f64,
    pub level: f64,
}

impl SingerParams {
    pub fn preset(part: usize) -> Self {
        Self {
            part: part as f64,
            formant_shift: 1.0,
            breath: 1.0,
            brightness: 0.55,
            vibrato_depth: 35.0,
            vibrato_rate: 5.5,
            vibrato_delay: 0.35,
            glide: 0.09,
            jitter: 0.4,
            attack: 0.06,
            release: 0.12,
            level: 0.04,
        }
    }
}

// —— 发声 ——

/// 0–1 均匀的伪随机数（xorshift），每个音从同一个种子起，结果确定。
#[derive(Clone, Copy)]
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn signed(&mut self) -> f64 {
        2.0 * self.next() - 1.0
    }
}

/// 一个二阶共振（Klatt 的共振器）。
#[derive(Clone, Copy, Default)]
struct Resonator {
    a: f64,
    b: f64,
    c: f64,
    y1: f64,
    y2: f64,
}
impl Resonator {
    fn set(&mut self, hz: f64, bw: f64, sr: f64) {
        let hz = hz.min(0.45 * sr);
        let r = (-PI * bw / sr).exp();
        self.c = -r * r;
        self.b = 2.0 * r * (TAU * hz / sr).cos();
        self.a = 1.0 - self.b - self.c;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.b * self.y1 + self.c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// 一个歌声。
pub(crate) struct SingerVoice {
    params: SingerParams,
    sr: f64,
    tuning: Tuning,
    key: u8,
    bend: f64,
    /// 现在唱到的音高（半音，含滑音）与目标。
    pitch: f64,
    target_pitch: f64,
    velocity: f64,
    // 声源
    phase: f64,
    period_scale: f64,
    amp_scale: f64,
    tilt: f64,
    // 声道
    tract: [Resonator; 5],
    formants: [f64; 5],
    // 音节：元音序列与这个音的长度（帧），已唱了几帧。
    syl: Syllable,
    note_frames: u64,
    at: u64,
    // 包络
    env: f64,
    held: bool,
    active: bool,
    vib_phase: f64,
    rng: Rng,
    notes: u64,
    /// 下一个音的歌词（`Lyric` 先到，`note_on` 再用）。
    pending: Option<(Syllable, u64)>,
}

impl SingerVoice {
    pub(crate) fn new(params: SingerParams) -> Self {
        Self {
            params,
            sr: 48_000.0,
            tuning: Tuning::default(),
            key: 60,
            bend: 0.0,
            pitch: 60.0,
            target_pitch: 60.0,
            velocity: 0.8,
            phase: 0.0,
            period_scale: 1.0,
            amp_scale: 1.0,
            tilt: 0.0,
            tract: [Resonator::default(); 5],
            formants: male(Vowel::A),
            syl: Syllable::one(Vowel::A),
            note_frames: 48_000,
            at: 0,
            env: 0.0,
            held: false,
            active: false,
            vib_phase: 0.0,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
            notes: 0,
            pending: None,
        }
    }

    pub(crate) fn set_params(&mut self, p: SingerParams) {
        self.params = p;
    }

    /// 下一个音唱的字与这个音有多长（帧）。
    pub(crate) fn lyric(&mut self, text: &str, frames: u64) {
        self.pending = Some((syllable(text), frames.max(1)));
    }

    fn part(&self) -> Part {
        PARTS[(self.params.part.round().max(0.0) as usize).min(PARTS.len() - 1)]
    }

    /// 这一刻的共振峰目标：按音节里的元音与唱到的位置在它们之间滑。
    fn target(&self) -> [f64; 5] {
        let s = self.syl;
        let x = (self.at as f64 / self.note_frames as f64).min(1.0);
        // 每个元音占的区间，交界处 25% 的长度里滑过去。
        let mut edges = [0.0; 3];
        let mut acc = 0.0;
        for (i, e) in edges.iter_mut().enumerate().take(s.n) {
            acc += s.shares[i];
            *e = acc;
        }
        let mut idx = 0;
        while idx + 1 < s.n && x > edges[idx] {
            idx += 1;
        }
        let a = male(s.vowels[idx]);
        let start = if idx == 0 { 0.0 } else { edges[idx - 1] };
        let w = (s.shares[idx] * 0.35).max(1e-6);
        if idx > 0 && x < start + w {
            let prev = male(s.vowels[idx - 1]);
            let t = ((x - start) / w).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            return std::array::from_fn(|k| prev[k] + (a[k] - prev[k]) * t);
        }
        a
    }

    fn sample(&mut self) -> f64 {
        let p = self.params;
        let part = self.part();
        let sr = self.sr;
        // 音高：滑音、颤音（开头一段之后才揉）。
        let glide = (-1.0 / (p.glide.max(0.005) * sr)).exp();
        self.pitch = self.target_pitch + (self.pitch - self.target_pitch) * glide;
        let t = self.at as f64 / sr;
        let vib_amt = ((t - p.vibrato_delay) / 0.3).clamp(0.0, 1.0) * p.vibrato_depth / 100.0;
        self.vib_phase = (self.vib_phase + p.vibrato_rate / sr).fract();
        let semis = self.pitch + self.bend + vib_amt * (TAU * self.vib_phase).sin();
        let f0 = 440.0 * ((semis - 69.0) / 12.0).exp2();

        // 声道：目标共振峰（按声部、整体比例），平滑地滑过去；高音时 F1 抬到基音之上。
        if self.at.is_multiple_of(32) {
            let scale = part.formants * p.formant_shift;
            let mut tgt = self.target().map(|f| f * scale);
            tgt[0] = tgt[0].max(f0 * 1.15);
            tgt[1] = tgt[1].max(tgt[0] + 200.0);
            let k = 1.0 - (-32.0 / (0.012 * sr)).exp();
            for i in 0..5 {
                self.formants[i] += (tgt[i] - self.formants[i]) * k;
                let bw = BANDWIDTHS[i] * (1.0 + 0.5 * (part.formants - 1.0));
                self.tract[i].set(self.formants[i], bw, sr);
            }
        }

        // 声源：声门气流导数。开相里升（正）后降（负的尖），闭相为零。
        let open = (part.open + 0.1 * (1.0 - self.velocity)).clamp(0.3, 0.8);
        self.phase += f0 * self.period_scale / sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            // 新的一周：这一周的长短、强弱略有不同。
            let j = p.jitter.clamp(0.0, 1.0);
            self.period_scale = 1.0 + 0.01 * j * self.rng.signed();
            self.amp_scale = 1.0 + 0.08 * j * self.rng.signed();
        }
        let ph = self.phase;
        let g = if ph < open {
            // 开相：u(x) = x² (1 − x)（x = ph / open），导数 2x − 3x²，结尾负的尖就是声门关上的那一下。
            let x = ph / open;
            (2.0 * x - 3.0 * x * x) * self.amp_scale
        } else {
            0.0
        };
        // 频谱倾斜：一阶低通，越暗截止越低。
        let cut = 600.0 + 5_400.0 * p.brightness.clamp(0.0, 1.0) * (0.6 + 0.4 * self.velocity);
        let k = 1.0 - (-TAU * cut / sr).exp();
        self.tilt += (g - self.tilt) * k;
        // 气声：跟着声门开合（开相里多）。
        let noise = self.rng.signed() * part.breath * p.breath * if ph < open { 1.0 } else { 0.3 };
        let mut x = self.tilt + noise;
        for r in self.tract.iter_mut() {
            x = r.tick(x);
        }
        x
    }
}

impl Voice<f32> for SingerVoice {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
    }

    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        self.notes += 1;
        self.key = note.key;
        self.velocity = f64::from(velocity);
        self.target_pitch = self.tuning.key_pitch(note.key);
        if !self.active {
            self.pitch = self.target_pitch;
            self.phase = 0.0;
            self.tilt = 0.0;
            self.tract = [Resonator::default(); 5];
            self.env = 0.0;
        }
        let (syl, frames) = self.pending.take().unwrap_or((Syllable::one(Vowel::A), (self.sr * 1.0) as u64));
        self.syl = syl;
        self.note_frames = frames;
        if !self.active {
            self.formants = male(syl.vowels[0]).map(|f| f * self.part().formants * self.params.formant_shift);
        }
        self.at = 0;
        self.rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (u64::from(note.key) << 32) ^ self.notes);
        self.held = true;
        self.active = true;
    }

    fn legato(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        // 连唱：不重新起声，只换音高与字。
        let _ = accent;
        self.note_on(note, velocity);
    }

    fn note_off(&mut self, _velocity: f32) {
        self.held = false;
    }

    fn choke(&mut self) {
        self.active = false;
        self.held = false;
        self.env = 0.0;
        // 歌词（pending）不清：合成器在每个新音之前先 choke，歌词是给这个新音的。
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn render(&mut self, buffer: &mut AudioBuffer<f32>) {
        if !self.active {
            return;
        }
        let p = self.params;
        let n = buffer.num_frames();
        let atk = 1.0 / (p.attack.max(0.005) * self.sr);
        let rel = 1.0 / (p.release.max(0.01) * self.sr);
        let gain = p.level * (0.35 + 0.65 * self.velocity);
        for i in 0..n {
            if self.held {
                self.env = (self.env + atk).min(1.0);
            } else {
                self.env -= rel;
                if self.env <= 0.0 {
                    self.active = false;
                    return;
                }
            }
            let y = (self.sample() * gain * self.env) as f32;
            self.at += 1;
            for c in 0..buffer.num_channels() {
                buffer.channel_mut(c)[i] += y;
            }
        }
    }

    fn pitch_bend(&mut self, semitones: f64) {
        self.bend = semitones;
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        self.tuning.clone_from(tuning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinyin_finals() {
        assert_eq!(syllable("ma").vowels[0], Vowel::A);
        assert_eq!(syllable("zi").vowels[0], Vowel::ApicalFront);
        assert_eq!(syllable("shi").vowels[0], Vowel::ApicalBack);
        assert_eq!(syllable("ju").vowels[0], Vowel::V);
        assert_eq!(syllable("yu").vowels[0], Vowel::V);
        let s = syllable("hai");
        assert_eq!((s.n, s.vowels[0], s.vowels[1]), (2, Vowel::A, Vowel::I));
        let s = syllable("liang");
        assert_eq!((s.n, s.vowels[0], s.vowels[1]), (2, Vowel::I, Vowel::A));
        let s = syllable("xiao");
        assert_eq!((s.n, s.vowels[0], s.vowels[1], s.vowels[2]), (3, Vowel::I, Vowel::A, Vowel::U));
        assert_eq!(syllable("wo").vowels[..2], [Vowel::U, Vowel::O]);
    }

    fn sing(text: &str, key: u8, seconds: f64) -> Vec<f32> {
        let mut v = SingerVoice::new(SingerParams::preset(1));
        v.prepare(&ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 1 });
        v.lyric(text, (seconds * 48_000.0) as u64);
        v.note_on(NoteRef::new(0, key), 0.8);
        let mut out = Vec::new();
        let mut buf = AudioBuffer::new(1, 512);
        while out.len() < (seconds * 48_000.0) as usize {
            buf.clear();
            v.render(&mut buf);
            out.extend_from_slice(buf.channel(0));
        }
        out
    }

    /// 频谱里 [lo, hi) 的能量占比。
    fn share(x: &[f32], lo: f64, hi: f64) -> f64 {
        let n = 4096;
        let seg = &x[x.len() / 2..x.len() / 2 + n];
        let (mut band, mut all) = (0.0, 0.0);
        for k in 1..n / 2 {
            let f = k as f64 * 48_000.0 / n as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, s) in seg.iter().enumerate() {
                let w = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
                re += f64::from(*s) * w * (TAU * k as f64 * i as f64 / n as f64).cos();
                im -= f64::from(*s) * w * (TAU * k as f64 * i as f64 / n as f64).sin();
            }
            let p = re * re + im * im;
            all += p;
            if f >= lo && f < hi {
                band += p;
            }
            if f > 6_000.0 {
                break;
            }
        }
        band / all
    }

    #[test]
    fn vowels_have_their_formants() {
        // i 的 F2 高（约 2.6 kHz），u 的 F2 低：2–3.5 kHz 的能量 i 远多于 u；a 的 F1 高：600–1300 Hz 多。
        let i = sing("yi", 57, 0.6);
        let u = sing("wu", 57, 0.6);
        let a = sing("a", 57, 0.6);
        assert!(i.iter().any(|s| s.abs() > 0.01), "it sings");
        assert!(share(&i, 2_000.0, 3_500.0) > 4.0 * share(&u, 2_000.0, 3_500.0));
        assert!(share(&a, 600.0, 1_300.0) > 2.0 * share(&i, 600.0, 1_300.0));
        assert_eq!(sing("a", 57, 0.3), sing("a", 57, 0.3), "deterministic");
    }
}
