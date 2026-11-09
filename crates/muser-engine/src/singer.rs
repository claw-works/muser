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
    // 女中：声道比女高长一些（共振峰更低），声音更暗、更厚。
    Part { formants: 1.05, open: 0.52, breath: 0.05 },
    Part { formants: 1.0, open: 0.5, breath: 0.04 },
    Part { formants: 0.93, open: 0.45, breath: 0.04 },
    Part { formants: 1.28, open: 0.62, breath: 0.07 },
];
pub(crate) const PART_NAMES: &[&str] = &["soprano", "alto", "tenor", "bass", "child"];

// —— 拼音 ——

/// 辅音的部位：决定爆破、摩擦的频谱，和元音开头共振峰从哪里滑进来（音轨"锁点"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// 双唇、唇齿（b p m f）。
    Labial,
    /// 舌尖前（z c s）。
    Dental,
    /// 舌尖中（d t n l）。
    Alveolar,
    /// 舌尖后（zh ch sh r）。
    Retroflex,
    /// 舌面（j q x）。
    Palatal,
    /// 舌根（g k h）。
    Velar,
}

/// 辅音的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manner {
    None,
    /// 塞音：成阻（无声）→ 爆破 →（送气）→ 元音。
    Stop { aspirated: bool },
    /// 塞擦音：成阻 → 爆破 → 摩擦 →（送气）→ 元音。
    Affricate { aspirated: bool },
    /// 擦音：一段摩擦噪声 → 元音。
    Fricative,
    /// 鼻音：嘴闭着、从鼻腔出的哼声 → 元音。
    Nasal,
    /// 边音 l。
    Lateral,
    /// 卷舌的 r。
    Rhotic,
}

/// 声母。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Onset {
    pub place: Place,
    pub manner: Manner,
}

impl Onset {
    pub const NONE: Self = Self { place: Place::Alveolar, manner: Manner::None };

    /// 各段的时长（秒）：成阻、爆破、摩擦、送气、哼声（有声的辅音）。
    fn segments(&self) -> [f64; 5] {
        match self.manner {
            Manner::None => [0.0; 5],
            Manner::Stop { aspirated: false } => [0.035, 0.008, 0.0, 0.01, 0.0],
            Manner::Stop { aspirated: true } => [0.03, 0.01, 0.0, 0.07, 0.0],
            Manner::Affricate { aspirated: false } => [0.025, 0.005, 0.032, 0.0, 0.0],
            Manner::Affricate { aspirated: true } => [0.02, 0.006, 0.07, 0.05, 0.0],
            Manner::Fricative => [0.0, 0.0, if self.place == Place::Velar { 0.08 } else { 0.12 }, 0.0, 0.0],
            Manner::Nasal => [0.0, 0.0, 0.0, 0.0, 0.09],
            Manner::Lateral => [0.0, 0.0, 0.0, 0.0, 0.05],
            Manner::Rhotic => [0.0, 0.0, 0.0, 0.0, 0.07],
        }
    }

    /// 辅音从元音开始前多久起（秒）：元音要落在拍上，辅音提前。
    pub fn lead(&self) -> f64 {
        self.segments().iter().sum()
    }

    fn voiced(&self) -> bool {
        matches!(self.manner, Manner::Nasal | Manner::Lateral | Manner::Rhotic)
    }
}

/// 韵尾。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coda {
    None,
    /// -n。
    N,
    /// -ng。
    Ng,
}

/// 一个音节：声母、依次滑过去的元音（最多三个）与每个元音占这个音的比例（介音短、韵腹长、韵尾短）、鼻韵尾。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Syllable {
    pub onset: Onset,
    pub vowels: [Vowel; 3],
    pub shares: [f64; 3],
    pub n: usize,
    pub coda: Coda,
}

impl Syllable {
    fn one(v: Vowel) -> Self {
        Self { onset: Onset::NONE, vowels: [v; 3], shares: [1.0, 0.0, 0.0], n: 1, coda: Coda::None }
    }
    fn two(a: Vowel, b: Vowel, glide_first: bool) -> Self {
        let shares = if glide_first { [0.12, 0.88, 0.0] } else { [0.7, 0.3, 0.0] };
        Self { vowels: [a, b, b], shares, n: 2, ..Self::one(a) }
    }
    fn three(a: Vowel, b: Vowel, c: Vowel) -> Self {
        Self { vowels: [a, b, c], shares: [0.12, 0.6, 0.28], n: 3, ..Self::one(a) }
    }
}

/// 这个字的辅音要提前多久开始（秒），渲染器据此把音符提前。
pub fn lead_seconds(text: &str) -> f64 {
    syllable(text).onset.lead()
}

/// 拼音（或英文拼法）→ 音节。
pub fn syllable(text: &str) -> Syllable {
    let mut syl = finals(text);
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphabetic() || *c == 'ü').collect();
    use Manner::*;
    use Place::*;
    let o = |place, manner| Onset { place, manner };
    const INITIALS: &[&str] = &["zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s"];
    syl.onset = match INITIALS.iter().find(|i| s.starts_with(**i)).copied().unwrap_or("") {
        "b" => o(Labial, Stop { aspirated: false }),
        "p" => o(Labial, Stop { aspirated: true }),
        "m" => o(Labial, Nasal),
        "f" => o(Labial, Fricative),
        "d" => o(Alveolar, Stop { aspirated: false }),
        "t" => o(Alveolar, Stop { aspirated: true }),
        "n" => o(Alveolar, Nasal),
        "l" => o(Alveolar, Lateral),
        "g" => o(Velar, Stop { aspirated: false }),
        "k" => o(Velar, Stop { aspirated: true }),
        "h" => o(Velar, Fricative),
        "j" => o(Palatal, Affricate { aspirated: false }),
        "q" => o(Palatal, Affricate { aspirated: true }),
        "x" => o(Palatal, Fricative),
        "zh" => o(Retroflex, Affricate { aspirated: false }),
        "ch" => o(Retroflex, Affricate { aspirated: true }),
        "sh" => o(Retroflex, Fricative),
        "r" => o(Retroflex, Rhotic),
        "z" => o(Dental, Affricate { aspirated: false }),
        "c" => o(Dental, Affricate { aspirated: true }),
        "s" => o(Dental, Fricative),
        _ => Onset::NONE,
    };
    // 鼻韵尾（er 不算；单独的 n / ng 也不算）。
    syl.coda = if s.len() > 2 && s.ends_with("ng") { Coda::Ng } else if s.len() > 1 && s.ends_with('n') && !s.ends_with("er") { Coda::N } else { Coda::None };
    syl
}

/// 韵母 → 要唱的元音。声母只用来认出"资 / 知"的 i、j q x y 后面的 u（= ü）。
fn finals(text: &str) -> Syllable {
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
        // ian（"烟"）是 [iɛn]：a 读得接近 ê。
        "ia" if fin == "ian" => Syllable::two(I, Eh, true),
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
            // 各声部的默认亮度与颤音：女高亮、揉得宽一点，女中、男低暗一些。
            brightness: [0.62, 0.45, 0.55, 0.45, 0.6][part.min(4)],
            vibrato_depth: [42.0, 35.0, 35.0, 30.0, 25.0][part.min(4)],
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

/// 一个二阶的"反共振"（零点，Klatt 的 antiresonator）：把它所在频率附近的声音吸掉。
#[derive(Clone, Copy, Default)]
struct Antiresonator {
    a: f64,
    b: f64,
    c: f64,
    x1: f64,
    x2: f64,
}
impl Antiresonator {
    fn set(&mut self, hz: f64, bw: f64, sr: f64) {
        let mut r = Resonator::default();
        r.set(hz, bw, sr);
        self.a = 1.0 / r.a;
        self.b = -r.b / r.a;
        self.c = -r.c / r.a;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.b * self.x1 + self.c * self.x2;
        self.x2 = self.x1;
        self.x1 = x;
        y
    }
}

/// 鼻腔：软腭放下时，声音分出一路从鼻子出来，在频谱上加一个低的共振（约 270 Hz）和一个"零点"
/// （吸掉一段频率，位置随口腔里堵住的地方：双唇约 1 kHz、舌尖约 1.7 kHz、舌根约 3 kHz）。
/// 软腭抬起（普通元音）时零点与共振重合、互相抵消，等于没有。
const NASAL_POLE: f64 = 270.0;
fn nasal_zero(place: Place) -> f64 {
    match place {
        Place::Labial => 1_000.0,
        Place::Velar => 3_000.0,
        _ => 1_700.0,
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
    /// 辅音段多少帧（元音从这里开始）。
    lead_frames: u64,
    at: u64,
    // 辅音的噪声：摩擦（两级带通）、爆破（两级带通）。
    fric: [Resonator; 2],
    burst: [Resonator; 2],
    /// 元音开头是否已经把共振峰放到"锁点"上。
    landed: bool,
    /// 这个字开始时上一个字还在响（连着唱）：辅音前要把它收掉。
    carried: bool,
    // 鼻腔：一个共振 + 一个零点（见 NASAL_POLE）。
    nasal_pole: Resonator,
    nasal_zero: Antiresonator,
    /// 零点现在的位置（Hz，平滑地动）。
    zero_hz: f64,
    // 包络
    env: f64,
    held: bool,
    active: bool,
    vib_phase: f64,
    rng: Rng,
    notes: u64,
    /// 下一个音的歌词（`Lyric` 先到，`note_on` 再用）。
    pending: Option<(Syllable, u64, u64)>,
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
            lead_frames: 0,
            at: 0,
            fric: [Resonator::default(); 2],
            burst: [Resonator::default(); 2],
            landed: true,
            carried: false,
            nasal_pole: Resonator::default(),
            nasal_zero: Antiresonator::default(),
            zero_hz: NASAL_POLE,
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

    /// 下一个音唱的字、这个音（元音部分）有多长、辅音提前了多少（帧）。
    pub(crate) fn lyric(&mut self, text: &str, frames: u64, lead: u64) {
        self.pending = Some((syllable(text), frames.max(1), lead));
    }

    fn part(&self) -> Part {
        PARTS[(self.params.part.round().max(0.0) as usize).min(PARTS.len() - 1)]
    }

    /// 这一刻的共振峰目标（男声尺度）：辅音段用辅音的形状，元音段按元音与唱到的位置在它们之间滑，韵尾合到鼻音。
    fn target(&self) -> [f64; 5] {
        let s = self.syl;
        if self.at < self.lead_frames {
            if let Some(f) = murmur(s.onset) {
                return f;
            }
            // 无声的辅音段：用元音开头的样子（送气、h 带着后面元音的颜色）。
            return locus_start(s.onset, male(s.vowels[0]));
        }
        let v = self.at - self.lead_frames;
        if let Some(coda) = nasal_coda(s.coda) {
            let n = coda_frames(self.note_frames, self.sr);
            if v + n >= self.note_frames {
                return coda;
            }
        }
        let x = (v as f64 / self.note_frames as f64).min(1.0);
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

        let scale = part.formants * p.formant_shift;
        // 元音开始的那一帧：塞音、塞擦音、擦音之后，共振峰从"锁点"起（b d g 的区别主要在这段滑动里）。
        if !self.landed && self.at >= self.lead_frames {
            self.landed = true;
            if !self.syl.onset.voiced() && self.syl.onset.manner != Manner::None {
                self.formants = locus_start(self.syl.onset, male(self.syl.vowels[0])).map(|f| f * scale);
            }
        }
        // 辅音段的几样声音：t 是从这个字开始（辅音开头）算起的秒数。
        let segs = self.syl.onset.segments();
        let (closure, burst_end, fric_end, asp_end) = (segs[0], segs[0] + segs[1], segs[0] + segs[1] + segs[2], segs[0] + segs[1] + segs[2] + segs[3]);
        let in_lead = self.at < self.lead_frames;
        let (mut av, mut ab, mut af, mut asp) = (1.0, 0.0, 0.0, 0.0);
        if in_lead {
            // 噪声的起落：摩擦 20 ms 进来、15 ms 出去，不是一下子开关（一下子开关听起来是另外一个"斯"）。
            let fade = |a: f64, b: f64| (((t - a) / 0.02).min((b - t) / 0.015)).clamp(0.0, 1.0);
            // 无声的辅音：上一个字的声音在 12 ms 里收掉，不是一下切断（一下切断是"搭"的一声）。
            av = if self.syl.onset.voiced() { 1.0 } else if self.carried { (1.0 - t / 0.012).max(0.0) } else { 0.0 };
            if t >= closure && t < burst_end {
                // 爆破：1 ms 冲上去、很快衰减（真的爆破很短也不响）。
                let x = t - closure;
                ab = (x / 0.001).min(1.0) * (-x / 0.003).exp();
            }
            if t >= burst_end - 0.002 && t < fric_end {
                af = fade(burst_end - 0.002, fric_end);
            }
            if t >= fric_end - 0.005 && t < asp_end {
                asp = fade(fric_end - 0.005, asp_end);
            }
            if self.syl.onset.manner == Manner::Rhotic {
                af = 0.06;
            }
        } else if self.syl.onset.manner != Manner::None && !self.syl.onset.voiced() {
            // 声带开始振动：15 ms 里起来；摩擦、送气在元音开头再拖 15 ms 才没（和元音接上）。
            let after = (self.at - self.lead_frames) as f64 / sr;
            av = (after / 0.015).min(1.0);
            let tail = (1.0 - after / 0.015).max(0.0);
            if segs[3] > 0.0 {
                asp = tail;
            } else if segs[2] > 0.0 {
                af = tail;
            }
        }
        // 鼻音（声母的哼声、鼻韵尾）：嘴闭着，声音小、闷。
        let mut murmur_gain = 1.0;
        if in_lead && murmur(self.syl.onset).is_some() && self.syl.onset.manner == Manner::Nasal {
            murmur_gain = 0.6;
        }
        if !in_lead && self.syl.coda != Coda::None {
            let v = self.at - self.lead_frames;
            let n = coda_frames(self.note_frames, sr);
            if v + n >= self.note_frames {
                murmur_gain = 1.0 - 0.45 * ((v + n - self.note_frames) as f64 / (0.04 * sr)).min(1.0);
            }
        }

        // 声道：目标共振峰（按声部、整体比例），平滑地滑过去；高音时 F1 抬到基音之上。
        if self.at.is_multiple_of(32) {
            let mut tgt = self.target().map(|f| f * scale);
            if !in_lead {
                tgt[0] = tgt[0].max(f0 * 1.15);
            }
            tgt[1] = tgt[1].max(tgt[0] + 200.0);
            // 元音开头 60 ms 里滑得慢一些（听得出辅音到元音的过渡），其余跟得快。
            let after = (self.at.saturating_sub(self.lead_frames)) as f64 / sr;
            let tau = if !in_lead && after < 0.06 && self.syl.onset.manner != Manner::None { 0.022 } else { 0.012 };
            let k = 1.0 - (-32.0 / (tau * sr)).exp();
            for i in 0..5 {
                self.formants[i] += (tgt[i] - self.formants[i]) * k;
                let bw = BANDWIDTHS[i] * (1.0 + 0.5 * (part.formants - 1.0));
                self.tract[i].set(self.formants[i], bw, sr);
            }
            // 鼻腔：鼻音（声母的 m n、韵尾的 n ng）时零点挪到堵住的地方；鼻音前后的元音也带一点鼻化
            // （零点挪到约 600 Hz）。放开鼻音的那一下零点很快回去（这一下"放开"是听出 m n 的关键）。
            let v = self.at.saturating_sub(self.lead_frames);
            let coda_start = self.note_frames.saturating_sub(coda_frames(self.note_frames, sr));
            let (zero, fast) = if in_lead && self.syl.onset.manner == Manner::Nasal {
                (nasal_zero(self.syl.onset.place), true)
            } else if !in_lead && self.syl.coda != Coda::None && v >= coda_start {
                (nasal_zero(if self.syl.coda == Coda::Ng { Place::Velar } else { Place::Alveolar }), false)
            } else if !in_lead && self.syl.coda != Coda::None && v + (0.12 * sr) as u64 >= coda_start {
                (600.0, false)
            } else if !in_lead && self.syl.onset.manner == Manner::Nasal && v < (0.06 * sr) as u64 {
                (NASAL_POLE + (600.0 - NASAL_POLE) * (1.0 - v as f64 / (0.06 * sr)), true)
            } else {
                (NASAL_POLE, true)
            };
            let kz = 1.0 - (-32.0 / ((if fast { 0.004 } else { 0.02 }) * sr)).exp();
            self.zero_hz += (zero - self.zero_hz) * kz;
            self.nasal_pole.set(NASAL_POLE * part.formants.sqrt(), 100.0, sr);
            self.nasal_zero.set(self.zero_hz * part.formants.sqrt(), 100.0, sr);
            let (fc, fbw, _) = frication(self.syl.onset, self.formants[1]);
            for r in self.fric.iter_mut() {
                r.set(fc * scale.sqrt(), fbw, sr);
            }
            let (bc, bbw) = burst_band(self.syl.onset, self.formants[1]);
            for r in self.burst.iter_mut() {
                r.set(bc, bbw, sr);
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
        // 气声：跟着声门开合（开相里多）；送气、h 是声道里的噪声（带着后面元音的颜色）。
        let breath = self.rng.signed() * part.breath * p.breath * if ph < open { 1.0 } else { 0.3 } * av;
        let aspiration = self.rng.signed() * 0.18 * asp;
        let mut x = self.tilt * av + breath + aspiration;
        x = self.nasal_zero.tick(self.nasal_pole.tick(x));
        for r in self.tract.iter_mut() {
            x = r.tick(x);
        }
        x *= murmur_gain;
        // 摩擦、爆破：噪声经过各自的带通（舌、唇前面的那一小段腔），不经过整条声道。
        let level = frication(self.syl.onset, self.formants[1]).2;
        let (n1, n2) = (self.rng.signed(), self.rng.signed());
        let mut f = n1 * af * level;
        for r in self.fric.iter_mut() {
            f = r.tick(f);
        }
        let mut b = n2 * ab * 0.8;
        for r in self.burst.iter_mut() {
            b = r.tick(b);
        }
        x + 0.6 * f + 1.5 * b
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
        self.carried = self.active;
        if !self.active {
            self.pitch = self.target_pitch;
            self.phase = 0.0;
            self.tilt = 0.0;
            self.tract = [Resonator::default(); 5];
            self.env = 0.0;
        }
        let (syl, frames, lead) = self.pending.take().unwrap_or((Syllable::one(Vowel::A), (self.sr * 1.0) as u64, 0));
        self.syl = syl;
        self.note_frames = frames;
        self.lead_frames = lead;
        self.landed = lead == 0;
        if !self.active {
            let start = murmur(syl.onset).unwrap_or_else(|| male(syl.vowels[0]));
            self.formants = start.map(|f| f * self.part().formants * self.params.formant_shift);
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

/// 有声辅音（m n l r）本身的共振峰（男声尺度）；没有时为 None。
fn murmur(o: Onset) -> Option<[f64; 5]> {
    match (o.manner, o.place) {
        (Manner::Nasal, Place::Labial) => Some([250.0, 1100.0, 2200.0, 3300.0, 4300.0]),
        (Manner::Nasal, _) => Some([250.0, 1600.0, 2600.0, 3400.0, 4400.0]),
        (Manner::Lateral, _) => Some([350.0, 1150.0, 2800.0, 3500.0, 4500.0]),
        (Manner::Rhotic, _) => Some([400.0, 1350.0, 1700.0, 3200.0, 4300.0]),
        _ => None,
    }
}

/// 鼻韵尾的共振峰：-n 舌尖抵住上齿龈，-ng 舌根抵住软腭（F2 更低）。
fn nasal_coda(c: Coda) -> Option<[f64; 5]> {
    match c {
        Coda::None => None,
        Coda::N => Some([250.0, 1750.0, 2600.0, 3400.0, 4400.0]),
        Coda::Ng => Some([280.0, 1050.0, 2300.0, 3300.0, 4300.0]),
    }
}

/// 韵尾占这个音的最后多少帧：至多 0.2 秒、至多三成半。
fn coda_frames(note: u64, sr: f64) -> u64 {
    ((0.2 * sr) as u64).min(note * 7 / 20)
}

/// 元音开头的共振峰：从辅音部位的"锁点"与元音之间的一半起（Klatt），往元音滑。
fn locus_start(o: Onset, vowel: [f64; 5]) -> [f64; 5] {
    let (f1, f2, f3) = match o.place {
        Place::Labial => (250.0, 900.0, 2200.0),
        Place::Dental => (300.0, 1700.0, 2600.0),
        Place::Alveolar => (300.0, 1800.0, 2700.0),
        Place::Retroflex => (320.0, 1700.0, 1800.0),
        Place::Palatal => (280.0, 2300.0, 3000.0),
        // 舌根："软腭夹"，F2 与 F3 靠在一起。
        Place::Velar => (300.0, (vowel[1] + 250.0).min(2300.0), (vowel[1] + 600.0).min(2700.0)),
    };
    let mix = |l: f64, v: f64| l + 0.5 * (v - l);
    [mix(f1, vowel[0]), mix(f2, vowel[1]), mix(f3, vowel[2]), vowel[3], vowel[4]]
}

/// 摩擦噪声的中心频率、带宽（Hz）与强度。
fn frication(o: Onset, f2: f64) -> (f64, f64, f64) {
    match o.place {
        Place::Dental => (7_000.0, 3_000.0, 0.28),
        Place::Retroflex => (3_200.0, 1_800.0, 0.5),
        Place::Palatal => (4_800.0, 2_600.0, 0.35),
        Place::Labial => (5_000.0, 6_000.0, 0.3),
        Place::Velar => ((f2 * 1.1).max(1_000.0), 2_000.0, 0.9),
        Place::Alveolar => (5_500.0, 3_000.0, 0.4),
    }
}

/// 爆破的中心频率与带宽：双唇低而散，舌尖高，舌根集中在后面元音的 F2 附近。
fn burst_band(o: Onset, f2: f64) -> (f64, f64) {
    match o.place {
        Place::Labial => (1_200.0, 3_000.0),
        Place::Alveolar => (4_000.0, 2_500.0),
        Place::Velar => ((f2 * 1.1).clamp(1_200.0, 3_000.0), 900.0),
        Place::Dental => (6_000.0, 3_000.0),
        Place::Retroflex => (3_000.0, 1_800.0),
        Place::Palatal => (4_200.0, 2_400.0),
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

    #[test]
    fn initials_and_codas() {
        let s = syllable("ba");
        assert_eq!(s.onset, Onset { place: Place::Labial, manner: Manner::Stop { aspirated: false } });
        assert_eq!(syllable("qing").coda, Coda::Ng);
        assert_eq!(syllable("chuang").onset.manner, Manner::Affricate { aspirated: true });
        assert_eq!(syllable("chuang").vowels[..2], [Vowel::U, Vowel::A]);
        assert_eq!(syllable("men").coda, Coda::N);
        assert_eq!(syllable("er").coda, Coda::None);
        assert_eq!(syllable("yue").onset, Onset::NONE);
        // 送气的比不送气的提前得多；擦音最长。
        assert!(lead_seconds("pa") > lead_seconds("ba") && lead_seconds("sa") > lead_seconds("ba") && lead_seconds("a") == 0.0);
    }

    #[test]
    fn consonants_sound_before_the_vowel() {
        // sa：元音之前是一段高频的摩擦；ba：元音之前有一段闭着的无声。
        let lead = |t: &str| (lead_seconds(t) * 48_000.0) as usize;
        let sa = sing("sa", 57, 0.6);
        let fric = &sa[lead("sa") / 3..lead("sa") * 2 / 3];
        assert!(share(&[fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric, fric].concat(), 4_000.0, 12_000.0) > 0.5);
        let ba = sing("ba", 57, 0.6);
        let closure = &ba[200..1_200];
        assert!(closure.iter().all(|s| s.abs() < 1e-6), "the lips are closed before the burst");
        assert!(ba[lead("ba") + 4_800..].iter().any(|s| s.abs() > 0.01), "then the vowel");
    }

    fn sing(text: &str, key: u8, seconds: f64) -> Vec<f32> {
        let mut v = SingerVoice::new(SingerParams::preset(1));
        v.prepare(&ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 1 });
        let lead = (lead_seconds(text) * 48_000.0) as u64;
        v.lyric(text, (seconds * 48_000.0) as u64, lead);
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
