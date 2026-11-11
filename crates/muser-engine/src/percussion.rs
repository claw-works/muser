//! yinse 自己的打击乐：每件是一组衰减的振动模式（鼓皮、锣面、钹、木头各有各的频率比）加一阵敲击的噪声，
//! 锣的音高在敲下去之后滑动（大锣往下"哐～"，小锣往上"镗儿"）。不经 manguebyte 的鼓组模型。
//!
//! 第一套是锣鼓 `yinse.strike.luogu`：大鼓、堂鼓、鼓边、板鼓、拍板、大锣、小锣、大钹（含闷击）、小钹、
//! 木鱼（高、低）、梆子、碰铃。每件一个 MIDI 键（见 [`LUOGU`]），在网格里也可以写名字（`daluo` `xiaoluo` …）。

use core::f64::consts::TAU;

use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec};

/// 一件打击乐的设计。
#[derive(Debug, PartialEq)]
pub struct PieceDef {
    /// 名字（网格行名、`instrument_params` 的键）与中文名。
    pub name: &'static str,
    pub zh: &'static str,
    pub key: u8,
    /// 基频（Hz）与模式：(频率比, 振幅, 衰减到 -60 dB 的秒数)。
    pub f0: f64,
    pub modes: &'static [(f64, f64, f64)],
    /// 敲下去之后音高滑多少半音、用多久（锣）。
    pub glide: f64,
    pub glide_s: f64,
    /// 敲击的噪声：多响、频带（Hz）、衰减时间常数（秒）。
    pub noise: f64,
    pub noise_lo: f64,
    pub noise_hi: f64,
    pub noise_s: f64,
    /// 同组的互相掐断（钹的闷击掐住正在响的钹），0 不掐。
    pub choke: u8,
    /// 整体增益（各件响度大致拉平）。
    pub gain: f64,
    /// 默认声像。
    pub pan: f64,
}

const MEMBRANE_LOW: &[(f64, f64, f64)] =
    &[(1.0, 1.0, 1.2), (1.59, 0.6, 0.8), (2.14, 0.4, 0.6), (2.30, 0.3, 0.5), (2.65, 0.25, 0.4), (2.92, 0.2, 0.35), (3.5, 0.12, 0.25)];
const MEMBRANE_MID: &[(f64, f64, f64)] =
    &[(1.0, 1.0, 0.6), (1.59, 0.6, 0.4), (2.14, 0.45, 0.3), (2.30, 0.35, 0.28), (2.65, 0.3, 0.22), (2.92, 0.22, 0.2), (3.5, 0.15, 0.15)];
const BANGU: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.12), (1.59, 0.7, 0.09), (2.14, 0.5, 0.07), (2.65, 0.4, 0.05), (3.2, 0.3, 0.04)];
const RIM: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.06), (1.7, 0.7, 0.05), (2.6, 0.5, 0.04), (3.9, 0.3, 0.03)];
const BIG_GONG: &[(f64, f64, f64)] = &[
    (1.0, 1.0, 4.0), (1.12, 0.4, 3.5), (1.53, 0.7, 3.2), (2.09, 0.6, 2.8), (2.55, 0.5, 2.4), (3.07, 0.45, 2.0),
    (3.6, 0.35, 1.7), (4.33, 0.3, 1.4), (5.1, 0.22, 1.1), (6.0, 0.16, 0.9), (7.3, 0.12, 0.7),
];
const SMALL_GONG: &[(f64, f64, f64)] = &[(1.0, 1.0, 1.4), (1.47, 0.5, 1.0), (2.03, 0.4, 0.8), (2.71, 0.3, 0.6), (3.4, 0.2, 0.45)];
const CYMBAL: &[(f64, f64, f64)] = &[
    (1.0, 0.5, 1.2), (1.41, 0.6, 1.1), (1.97, 0.7, 1.0), (2.53, 0.8, 1.0), (3.21, 0.8, 0.9), (3.86, 0.7, 0.85), (4.71, 0.7, 0.8),
    (5.53, 0.6, 0.75), (6.42, 0.6, 0.7), (7.39, 0.5, 0.65), (8.52, 0.5, 0.6), (9.81, 0.4, 0.55), (11.2, 0.4, 0.5), (12.9, 0.3, 0.45),
    (14.6, 0.3, 0.4), (16.8, 0.25, 0.35), (19.3, 0.2, 0.3), (22.1, 0.2, 0.28), (25.6, 0.15, 0.25), (29.3, 0.12, 0.22),
];
const CYMBAL_MUTED: &[(f64, f64, f64)] = &[
    (1.0, 0.5, 0.12), (1.41, 0.6, 0.11), (1.97, 0.7, 0.1), (2.53, 0.8, 0.1), (3.21, 0.8, 0.09), (3.86, 0.7, 0.085), (4.71, 0.7, 0.08),
    (5.53, 0.6, 0.075), (6.42, 0.6, 0.07), (7.39, 0.5, 0.065), (8.52, 0.5, 0.06), (9.81, 0.4, 0.055), (11.2, 0.4, 0.05), (12.9, 0.3, 0.045),
];
const SMALL_CYMBAL: &[(f64, f64, f64)] = &[
    (1.0, 0.5, 0.7), (1.41, 0.6, 0.65), (1.97, 0.7, 0.6), (2.53, 0.8, 0.6), (3.21, 0.8, 0.55), (3.86, 0.7, 0.5), (4.71, 0.7, 0.48),
    (5.53, 0.6, 0.45), (6.42, 0.6, 0.42), (7.39, 0.5, 0.4), (8.52, 0.5, 0.36), (9.81, 0.4, 0.33), (11.2, 0.4, 0.3), (12.9, 0.3, 0.27),
];
const WOOD: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.09), (2.61, 0.35, 0.05), (4.1, 0.15, 0.03)];
const WOOD_LOW: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.11), (2.61, 0.35, 0.06), (4.1, 0.15, 0.035)];
const CLAPPER: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.06), (2.4, 0.4, 0.04), (3.9, 0.2, 0.025)];
const BOARD: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.04), (2.3, 0.6, 0.03), (3.6, 0.4, 0.025)];
// 两只碰铃差 4 Hz 左右：一碰就拍出"嗡嗡"的起伏。
const BELLS: &[(f64, f64, f64)] = &[(1.0, 1.0, 2.5), (1.0014, 0.9, 2.5), (2.76, 0.3, 1.2), (2.762, 0.25, 1.2), (5.4, 0.12, 0.6)];

#[allow(clippy::too_many_arguments)] // 一行一件，表比结构体字面量好读
const fn piece(
    name: &'static str, zh: &'static str, key: u8, f0: f64, modes: &'static [(f64, f64, f64)], glide: (f64, f64),
    noise: (f64, f64, f64, f64), choke: u8, gain: f64, pan: f64,
) -> PieceDef {
    PieceDef { name, zh, key, f0, modes, glide: glide.0, glide_s: glide.1, noise: noise.0, noise_lo: noise.1, noise_hi: noise.2, noise_s: noise.3, choke, gain, pan }
}

/// 锣鼓（京剧、民间吹打）。键尽量落在 GM 里位置相近的鼓件上：大鼓在底鼓、堂鼓在军鼓、大锣在吊镲……
pub static LUOGU: &[PieceDef] = &[
    piece("dagu", "大鼓", 36, 65.0, MEMBRANE_LOW, (-1.5, 0.12), (0.5, 60.0, 900.0, 0.025), 0, 2.5, 0.0),
    piece("gubian", "鼓边", 37, 900.0, RIM, (0.0, 0.1), (0.8, 1_500.0, 7_000.0, 0.012), 0, 1.6, 0.0),
    piece("tanggu", "堂鼓", 38, 150.0, MEMBRANE_MID, (-1.0, 0.08), (0.5, 100.0, 2_500.0, 0.02), 0, 1.9, -0.1),
    piece("ban", "拍板", 39, 1_150.0, BOARD, (0.0, 0.1), (1.2, 800.0, 8_000.0, 0.01), 0, 1.95, 0.2),
    piece("bangu", "板鼓", 40, 750.0, BANGU, (0.0, 0.1), (1.0, 2_000.0, 9_000.0, 0.015), 0, 1.45, 0.15),
    piece("daluo", "大锣", 49, 180.0, BIG_GONG, (-2.5, 0.6), (0.4, 300.0, 6_000.0, 0.05), 0, 1.65, -0.25),
    piece("xiaoluo", "小锣", 52, 640.0, SMALL_GONG, (2.0, 0.25), (0.3, 800.0, 8_000.0, 0.02), 0, 2.0, 0.3),
    piece("xiaobo", "小钹", 55, 520.0, SMALL_CYMBAL, (0.0, 0.1), (0.8, 3_500.0, 12_000.0, 0.5), 2, 0.82, 0.35),
    piece("naobo", "大钹", 57, 300.0, CYMBAL, (0.0, 0.1), (1.0, 2_500.0, 12_000.0, 0.9), 1, 1.0, 0.2),
    piece("naobo_mute", "大钹闷击", 58, 300.0, CYMBAL_MUTED, (0.0, 0.1), (1.0, 2_500.0, 12_000.0, 0.1), 1, 0.86, 0.2),
    piece("bangzi", "梆子", 75, 1_750.0, CLAPPER, (0.0, 0.1), (0.5, 2_000.0, 9_000.0, 0.004), 0, 2.55, -0.2),
    piece("muyu", "木鱼（高）", 76, 900.0, WOOD, (0.0, 0.1), (0.3, 1_000.0, 6_000.0, 0.006), 0, 2.4, 0.1),
    piece("muyu_low", "木鱼（低）", 77, 560.0, WOOD_LOW, (0.0, 0.1), (0.3, 1_000.0, 6_000.0, 0.006), 0, 2.4, -0.1),
    piece("pengling", "碰铃", 81, 2_900.0, BELLS, (0.0, 0.1), (0.1, 3_000.0, 12_000.0, 0.003), 0, 0.94, 0.25),
];

/// 重踩底鼓时带出来的、不是底鼓本身的余音：鼓身（150 … 330 Hz 的几个共鸣）和军鼓响弦跟着嗡（1.5 … 10 kHz 的噪声）。
/// 量自一套放克鼓的采样：重踩时这两段比轻踩响 10 … 15 dB。
const KICK_SHELL: &[(f64, f64, f64)] = &[(1.0, 1.0, 0.35), (1.17, 0.6, 0.3), (1.31, 0.5, 0.3), (1.57, 0.4, 0.25), (1.95, 0.3, 0.2)];
pub static KICK_AFTER: &[PieceDef] = &[piece("kick_after", "底鼓重踩的余音", 36, 168.0, KICK_SHELL, (0.0, 0.1), (0.12, 1_500.0, 10_000.0, 0.12), 0, 0.1, 0.0)];

/// 套鼓的踩镲：两片镲的振动模式（量自一套放克鼓的采样）加一片高频噪声；闭镲、踩镲掐断开镲。
const HAT_CLOSED: &[(f64, f64, f64)] = &[(0.8642, 0.288, 0.09999), (1.0273, 0.2438, 0.08871), (1.0914, 0.3067, 0.1056), (1.1539, 0.2989, 0.1159), (1.1901, 0.6644, 0.08717), (1.2396, 0.4101, 0.1144), (1.3201, 0.4515, 0.1144), (1.3841, 0.3964, 0.09384), (1.5116, 0.5991, 0.08461), (1.5654, 0.6858, 0.1056), (1.6694, 0.9935, 0.0923), (1.8436, 0.6035, 0.1174), (1.893, 0.683, 0.07948), (2.0827, 0.9211, 0.09589), (2.1538, 0.955, 0.1108), (2.485, 1.0, 0.1118), (0.08305, 0.06087, 0.03)];
const HAT_PEDAL: &[(f64, f64, f64)] = &[(0.86342, 1.0, 0.3085), (0.94105, 0.8553, 0.3215), (1.0103, 0.7593, 0.3272), (1.0533, 0.7288, 0.3346), (1.1277, 0.6641, 0.3272), (1.3555, 0.4452, 0.2683), (1.4352, 0.4126, 0.2299), (1.5558, 0.3918, 0.2234), (2.3982, 0.1666, 0.2542), (10.413, 0.2808, 0.1879), (12.411, 0.2469, 0.2206), (14.269, 0.2872, 0.1898), (15.374, 0.2322, 0.201), (19.146, 0.1326, 0.2066), (20.126, 0.1981, 0.2094), (22.156, 0.4079, 0.2271), (0.99298, 0.02036, 0.03)];
const HAT_OPEN: &[(f64, f64, f64)] = &[(0.96593, 0.2474, 7.602), (1.1181, 0.5371, 8.293), (1.2514, 1.0, 7.597), (1.3128, 0.6833, 6.527), (1.4009, 0.4812, 6.33), (1.6174, 0.4004, 7.643), (1.7055, 0.289, 7.936), (1.8147, 0.336, 7.196), (1.8907, 0.4161, 6.771), (2.009, 0.2508, 5.792), (2.1348, 0.3193, 7.885), (2.281, 0.179, 7.305), (2.3886, 0.5103, 7.114), (2.5219, 0.2548, 5.77), (2.5843, 0.4106, 6.505), (2.8107, 0.357, 6.534), (0.097338, 0.004609, 0.03)];
pub static HATS: &[PieceDef] = &[
    piece("closed_hat", "闭镲", 42, 3612.3, HAT_CLOSED, (0.0, 0.1), (2.4853, 1096.6, 8955.3, 0.026678), 1, 0.055372, 0.25),
    piece("pedal_hat", "踩镲", 44, 302.12, HAT_PEDAL, (0.0, 0.1), (7.815, 561.2, 8392.7, 0.035455), 1, 0.0088213, 0.25),
    piece("open_hat", "开镲", 46, 3082.03, HAT_OPEN, (0.0, 0.1), (9.8204, 1998.2, 8010.1, 0.58151), 1, 0.074594, 0.25),
];

/// shortcut: 拟合时用环境变量 MUSER_FIT_HATS（JSON：PieceDef 的数组）代替 [`HATS`]，定下来就写回并删掉这里。
fn hats_override() -> Option<&'static [PieceDef]> {
    let text = std::fs::read_to_string(std::env::var_os("MUSER_FIT_HATS")?).ok()?;
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let leak = |s: &serde_json::Value| -> &'static str { Box::leak(s.as_str().unwrap_or_default().to_string().into_boxed_str()) };
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
    let defs: Vec<PieceDef> = list
        .iter()
        .map(|d| PieceDef {
            name: leak(&d["name"]),
            zh: leak(&d["zh"]),
            key: d["key"].as_u64().unwrap_or(0) as u8,
            f0: num(&d["f0"]),
            modes: Box::leak(d["modes"].as_array().into_iter().flatten().map(|m| (num(&m[0]), num(&m[1]), num(&m[2]))).collect::<Vec<_>>().into_boxed_slice()),
            glide: 0.0,
            glide_s: 0.1,
            noise: num(&d["noise"]),
            noise_lo: num(&d["noise_lo"]),
            noise_hi: num(&d["noise_hi"]),
            noise_s: num(&d["noise_s"]),
            choke: 1,
            gain: num(&d["gain"]),
            pan: num(&d["pan"]),
        })
        .collect();
    Some(Box::leak(defs.into_boxed_slice()))
}

/// 一件的设定：响度（倍）、调音（半音）、余音长短（倍）、亮度（0 … 1，0.5 不变）、声像。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PieceSet {
    pub level: f64,
    pub tune: f64,
    pub decay: f64,
    pub tone: f64,
    pub pan: f64,
}

/// 一套打击乐：设计表与每件的设定。
#[derive(Debug, Clone, PartialEq)]
pub struct KitParams {
    pub defs: &'static [PieceDef],
    pub set: Vec<PieceSet>,
    /// 噪声用两级的带通（踩镲：12 kHz 以上收得干净）；锣鼓用一级。
    pub steep: bool,
}

impl KitParams {
    /// 底鼓重踩的余音（[`KICK_AFTER`]），`yinse.strike.drums` 跟着底鼓敲，`gain` 是多响。
    pub fn kick_after(gain: f64) -> Self {
        let mut k = Self { defs: KICK_AFTER, set: KICK_AFTER.iter().map(|d| PieceSet { level: 1.0, tune: 0.0, decay: 1.0, tone: 0.5, pan: d.pan }).collect(), steep: false };
        k.set[0].level = gain;
        k
    }

    /// 套鼓的踩镲（[`HATS`]）。
    pub fn hats() -> Self {
        let defs = hats_override().unwrap_or(HATS);
        Self { defs, set: defs.iter().map(|d| PieceSet { level: 1.0, tune: 0.0, decay: 1.0, tone: 0.5, pan: d.pan }).collect(), steep: true }
    }

    pub fn luogu() -> Self {
        Self { defs: LUOGU, set: LUOGU.iter().map(|d| PieceSet { level: 1.0, tune: 0.0, decay: 1.0, tone: 0.5, pan: d.pan }).collect(), steep: false }
    }

    /// 名字或 MIDI 键 → 第几件。
    pub fn index(&self, name: &str) -> Option<usize> {
        let n = name.trim().to_ascii_lowercase();
        let key = n.parse::<u8>().ok();
        self.defs.iter().position(|d| d.name == n || Some(d.key) == key)
    }

    pub fn keys(&self) -> String {
        self.defs.iter().map(|d| format!("{} {} ({})", d.key, d.name, d.zh)).collect::<Vec<_>>().join(", ")
    }
}

const MAX_MODES: usize = 24;
const MAX_HITS: usize = 48;
const CONTROL: u32 = 32;

#[derive(Clone, Copy, Default)]
struct Osc {
    re: f64,
    im: f64,
    rot_re: f64,
    rot_im: f64,
    hz: f64,
    loss: f64,
}

struct Hit {
    piece: usize,
    oscs: [Osc; MAX_MODES],
    count: usize,
    age: u32,
    bend: f64,
    glide: f64,
    glide_k: f64,
    noise: f64,
    noise_k: f64,
    hp: f64,
    lp: f64,
    hp2: f64,
    lp2: f64,
    steep: bool,
    hp_a: f64,
    lp_a: f64,
    gains: [f64; 2],
    life: u32,
    choked: f64,
    rng: u64,
}

impl Hit {
    fn retune(&mut self, sr: f64) {
        let factor = (self.bend / 12.0).exp2();
        for o in &mut self.oscs[..self.count] {
            let w = TAU * (o.hz * factor).min(0.45 * sr) / sr;
            o.rot_re = o.loss * w.cos();
            o.rot_im = o.loss * w.sin();
        }
    }

    fn sample(&mut self, sr: f64) -> f64 {
        if self.age.is_multiple_of(CONTROL) && self.glide != 0.0 {
            // 锣：音高按指数靠近终点（滑 `glide` 个半音）。
            let t = f64::from(self.age) / sr;
            self.bend = self.glide * (1.0 - (-t * self.glide_k).exp());
            self.retune(sr);
        }
        let mut x = 0.0;
        for o in &mut self.oscs[..self.count] {
            let re = o.re * o.rot_re - o.im * o.rot_im;
            o.im = o.re * o.rot_im + o.im * o.rot_re;
            o.re = re;
            x += o.im;
        }
        if self.noise > 1e-6 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            let w = (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            self.lp += self.lp_a * (w - self.lp);
            if self.steep {
                self.lp2 += self.lp_a * (self.lp - self.lp2);
                self.hp += self.hp_a * (self.lp2 - self.hp);
                let band = self.lp2 - self.hp;
                self.hp2 += self.hp_a * (band - self.hp2);
                x += (band - self.hp2) * self.noise;
            } else {
                self.hp += self.hp_a * (self.lp - self.hp);
                x += (self.lp - self.hp) * self.noise;
            }
            self.noise *= self.noise_k;
        }
        self.age += 1;
        x
    }
}

/// 一套打击乐的发声：每敲一下是一个 [`Hit`]，响完就丢。
pub(crate) struct Kit {
    params: KitParams,
    hits: Vec<Hit>,
    sr: f64,
    seed: u64,
}

impl Kit {
    pub(crate) fn new(params: KitParams) -> Self {
        Self { params, hits: Vec::with_capacity(MAX_HITS), sr: 48_000.0, seed: 0x9E37_79B9_7F4A_7C15 }
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.hits.clear();
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let Some(i) = self.params.defs.iter().position(|d| d.key == note.key) else { return };
        let (def, set) = (&self.params.defs[i], self.params.set[i]);
        let v = (f64::from(velocity) * if accent { 1.25 } else { 1.0 }).clamp(0.0, 1.3);
        if def.choke > 0 {
            for h in &mut self.hits {
                if self.params.defs[h.piece].choke == def.choke {
                    h.choked = h.choked.min(1.0);
                    h.life = h.life.min(h.age + (0.015 * self.sr) as u32);
                }
            }
        }
        if self.hits.len() >= MAX_HITS {
            self.hits.remove(0);
        }
        let sr = self.sr;
        let f0 = def.f0 * (set.tune / 12.0).exp2();
        let mut oscs = [Osc::default(); MAX_MODES];
        let mut count = 0;
        let mut longest = def.noise_s * 6.0;
        for &(ratio, amp, t60) in def.modes.iter().take(MAX_MODES) {
            let hz = f0 * ratio;
            if hz > 0.45 * sr {
                continue;
            }
            // 敲得越重、`tone` 越大，高的模式越响。
            let octaves = ratio.log2().max(0.0);
            let bright = v.powf(0.5 * octaves) * (1.0 + (set.tone - 0.5) * 0.8 * octaves).max(0.05);
            let t = t60 * set.decay.clamp(0.1, 10.0);
            longest = longest.max(t);
            oscs[count] = Osc { re: 0.0, im: 0.0, rot_re: 0.0, rot_im: 0.0, hz, loss: (-3.0 * 10f64.ln() / (t * sr)).exp() };
            // 从零相位起振（不出咔嗒），振幅放在 re 上。
            oscs[count].re = amp * bright;
            count += 1;
        }
        self.seed = self.seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let level = def.gain * set.level * v.powf(1.4) * 0.25;
        let pan = set.pan.clamp(-1.0, 1.0);
        let a = (pan + 1.0) * core::f64::consts::FRAC_PI_4;
        let mut hit = Hit {
            piece: i,
            oscs,
            count,
            age: 0,
            bend: 0.0,
            glide: def.glide,
            glide_k: 1.0 / def.glide_s.max(0.005),
            noise: def.noise * (0.6 + 0.8 * set.tone) * v,
            noise_k: (-1.0 / (def.noise_s.max(0.001) * sr)).exp(),
            hp: 0.0,
            lp: 0.0,
            hp2: 0.0,
            lp2: 0.0,
            steep: self.params.steep,
            hp_a: 1.0 - (-TAU * def.noise_lo / sr).exp(),
            lp_a: 1.0 - (-TAU * def.noise_hi.min(0.45 * sr) / sr).exp(),
            gains: [level * a.cos(), level * a.sin()],
            life: (longest * 1.1 * sr) as u32 + 64,
            choked: 1.0,
            rng: self.seed | 1,
        };
        hit.retune(sr);
        self.hits.push(hit);
    }

    pub(crate) fn note_off(&mut self, _note: NoteRef, _velocity: f32) {}

    /// 掐断正在响的（15 ms 收掉）：套鼓的闭镲夹住开镲。
    pub(crate) fn choke(&mut self) {
        for h in &mut self.hits {
            h.life = h.life.min(h.age + (0.015 * self.sr) as u32);
        }
    }

    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        let n = buffer.num_frames();
        let channels = buffer.num_channels();
        let sr = self.sr;
        let fade = 1.0 / (0.015 * sr);
        buffer.clear();
        for h in &mut self.hits {
            for i in 0..n {
                if h.age >= h.life {
                    break;
                }
                let mut x = h.sample(sr);
                if h.life.saturating_sub(h.age) < (0.015 * sr) as u32 {
                    h.choked = (h.choked - fade).max(0.0);
                }
                x *= h.choked;
                if channels >= 2 {
                    buffer.channel_mut(0)[i] += (x * h.gains[0]) as f32;
                    buffer.channel_mut(1)[i] += (x * h.gains[1]) as f32;
                } else if channels == 1 {
                    buffer.channel_mut(0)[i] += (x * (h.gains[0] + h.gains[1]) * 0.5) as f32;
                }
            }
        }
        self.hits.retain(|h| h.age < h.life);
    }

    /// 自动化：`<件>.<参数>`，如 `daluo.tune`；下一下起作用。
    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        let Some((piece, param)) = name.split_once('.') else { return };
        let Some(i) = self.params.index(piece) else { return };
        let s = &mut self.params.set[i];
        match param {
            "level" => s.level = value.clamp(0.0, 4.0),
            "tune" => s.tune = value.clamp(-24.0, 24.0),
            "decay" => s.decay = value.clamp(0.1, 10.0),
            "tone" => s.tone = value.clamp(0.0, 1.0),
            "pan" => s.pan = value.clamp(-1.0, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(key: u8, seconds: f64) -> Vec<f32> {
        let mut k = Kit::new(KitParams::luogu());
        k.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 480 });
        k.note_on(NoteRef::new(9, key), 0.8, false);
        let mut out = Vec::new();
        let mut b = AudioBuffer::<f32>::new(1, 480);
        for _ in 0..(seconds * 100.0) as usize {
            b.clear();
            k.process(&mut b);
            out.extend_from_slice(b.channel(0));
        }
        out
    }

    #[test]
    fn every_piece_sounds_and_stops() {
        for d in LUOGU {
            let x = hit(d.key, 6.0);
            let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.01 && peak < 1.0, "{} peak {peak}", d.name);
            assert!(x[x.len() - 480..].iter().all(|v| v.abs() < 1e-4), "{} still ringing after 6 s", d.name);
        }
    }

    #[test]
    fn the_big_gong_falls_and_the_small_gong_rises() {
        let pitch = |x: &[f32], at: f64, around: f64| {
            let start = (at * 48_000.0) as usize;
            let w = &x[start..start + 9_600];
            (0..400).map(|k| around * (0.85 + 0.3 * f64::from(k) / 400.0)).max_by(|a, b| {
                let e = |hz: f64| { let (mut re, mut im) = (0.0, 0.0); for (i, &v) in w.iter().enumerate() { let p = TAU * hz * i as f64 / 48_000.0; re += f64::from(v) * p.cos(); im += f64::from(v) * p.sin(); } re * re + im * im };
                e(*a).total_cmp(&e(*b))
            }).unwrap()
        };
        let big = hit(49, 2.0);
        assert!(pitch(&big, 1.2, 180.0) < pitch(&big, 0.0, 180.0) * 0.9, "大锣往下");
        let small = hit(52, 1.0);
        assert!(pitch(&small, 0.6, 640.0) > pitch(&small, 0.0, 640.0) * 1.05, "小锣往上");
    }
}
