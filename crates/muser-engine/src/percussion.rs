//! yinse 自己的打击乐的发声：每件是一组衰减的振动模式（鼓皮、锣面、钹、木头各有各的频率比）加分频带的噪声，
//! 锣的音高在敲下去之后滑动（大锣往下"哐～"，小锣往上"镗儿"）。不经 manguebyte 的鼓组模型。
//!
//! 这里只有发声的引擎；每一套鼓（哪些件、每件的模式和噪声层）是数据，在 muser-yinse 音色库里（`yinse:kit/<作者>/<名>`），
//! 由 [`crate::kit`] 读进来、组成一件乐器。

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
    /// 滑音落在哪儿：0 是从 `f0` 起滑、1 是滑到 `f0` 停（小锣），中间的是 `f0` 夹在起点和终点之间（大锣）。
    pub glide_from: f64,
    /// 滑多少跟力度的几次方走（0 是不管轻重都一样）：锣敲得轻几乎不滑。
    pub glide_vel: f64,
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
    /// 噪声分几下打出来（拍手：几只手前后差 10 ms 左右），最后一下接着按 `noise_s` 衰减；0 / 1 只有一下。
    pub bursts: u8,
    /// 分频带的噪声层（ride：模态之间那一片密的"沙"，鼓槌的"嗒"也是一层）；空的没有。同一件连着敲共用一组（见 [`Wash`]）。
    pub layers: &'static [NoiseLayer],
    /// 每个模态起振的相位（弧度，与 `modes` 一一对应）；空的都从零相位起。量出来的波形（底鼓头几十毫秒的那一下）靠它对上。
    pub phases: &'static [f64],
    /// 左右去相关多少（像头顶话筒收的一片镲，不是一个点）：0 两边一样，1 完全不相关（ride），0.45 左右的相关系数 0.7（踩镲）。
    /// 每个模态左右差一个固定的相位（最多 ±π·width），每一下的起始相位随机；噪声层左右一部分共用、一部分各出各的。
    pub width: f64,
    /// 共振的音量随力度的几次方（默认 1.4）；大了轻敲时只剩敲击声（ride 的钟面）。
    pub mode_vel: f64,
    /// 共振涌起来用多久（秒，0 一敲就满）：从四分之一涌到满（sizzle 吊镲的金属声是敲下去以后才起来的）。
    pub mode_attack: f64,
    /// 频率比不超过它的模态不涌、一敲就满（大抄锣：低处的"嗡"敲下去就在，上面那一片是后来才涌出来的）；0 全都涌。
    pub swell_above: f64,
    /// 共振推到饱和上多少（0 不推）：敲下去最响的那一段被软软地压平，余下的相对就大了，还多出一点奇次谐波——
    /// 压得很满的大鼓录音那种又厚又顶的"劲"。只管不去相关的件（鼓）。
    pub drive: f64,
}

/// 一层带通噪声：频带（Hz），快、慢两段衰减各自多响（相对模态里最响的那个）与时间常数（秒），声像相对这一件偏多少。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseLayer {
    pub lo: f64,
    pub hi: f64,
    pub fast: f64,
    pub fast_s: f64,
    pub slow: f64,
    pub slow_s: f64,
    pub pan: f64,
    /// 上沿用四阶低通（二阶的会漏到上面的频带里；底鼓 9 kHz 以上几乎没有东西）。
    pub steep: bool,
    /// 下沿也用四阶（二阶的高通往下漏得多：镲 400 Hz 以上很响的那一层会在一二百赫兹上漏出一片不该有的声音）。
    pub sharp: bool,
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
    /// 整套的压缩（锣鼓）：门限 dB、比例、起、落（秒）、压完补回多少 dB。没有就不压。
    pub compress: Option<(f64, f64, f64, f64, f64)>,
    /// 噪声层的随机数从第几路起（一套鼓拆成很多个 [`Kit`] 时，各件的噪声不能是同一路：同时敲会叠成一个声音）。
    pub noise_seed: usize,
    /// 同时最多几下在响。
    pub max_hits: usize,
}

impl KitParams {
    /// 名字或 MIDI 键 → 第几件。
    pub fn index(&self, name: &str) -> Option<usize> {
        let n = name.trim().to_ascii_lowercase();
        let key = n.parse::<u8>().ok();
        self.defs.iter().position(|d| d.name == n || Some(d.key) == key)
    }
}

const MAX_MODES: usize = 64;
const CONTROL: u32 = 32;

#[derive(Clone, Copy, Default)]
struct Osc {
    re: f64,
    im: f64,
    rot_re: f64,
    rot_im: f64,
    hz: f64,
    loss: f64,
    /// 去相关的件（`width` > 0）：左、右各取 im、re 的多少（= 各自的相位）。
    mix: [f64; 4],
    /// 不跟着涌（[`PieceDef::swell_above`]）。
    still: bool,
}

struct Hit {
    piece: usize,
    oscs: [Osc; MAX_MODES],
    count: usize,
    /// 共振还差多少没涌起来（1 − 现在的比例）与每帧乘多少（[`PieceDef::mode_attack`]）。
    swell: f64,
    swell_k: f64,
    drive: f64,
    age: u32,
    bend: f64,
    glide: f64,
    glide_k: f64,
    glide_from: f64,
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
    /// 几连拍：每下隔多少帧、到第几帧结束（之后噪声正常衰减）。
    burst_gap: u32,
    burst_end: u32,
    wide: bool,
}

/// 一件的噪声层的发声。噪声没有相位，连着敲的几下不必各算各的：每层只记快、慢两段还剩多少能量，
/// 新的一下把能量加进去，一路噪声按总能量出声（敲得再密也只算一组滤波器）。
struct Wash {
    piece: usize,
    bands: Vec<Band>,
    rng: u64,
    /// 左右的噪声有多少是共用的一路、多少是各出各的（振幅的比例）；底鼓在正中，全是共用的。
    shared: f64,
    apart: f64,
    /// 被掐断：能量 5 ms 收掉，下一下敲下去恢复。
    choked: bool,
    /// 涌起来（[`PieceDef::mode_attack`]）：还没涌进来的能量每帧进来多少（0 一敲就满）。
    swell_a: f64,
    /// 分两步涌（[`PieceDef::swell_above`] > 0 的件），越高的频带涌得越慢（大抄锣 4 kHz 以上半秒才到最响）。
    two_stage: bool,
}

struct Band {
    /// 快、慢两段的能量（振幅的平方）与每帧乘多少。
    energy: [f64; 2],
    k: [f64; 2],
    /// 还没涌进来的能量（快、慢两段）。
    pending: [f64; 2],
    /// 涌得慢的件（大抄锣）：能量分两步进来（先到这里，再到 `energy`），开头几十毫秒几乎没有、之后才涌起来。
    rising: [f64; 2],
    /// 这个频带涌得多快（相对整件的）。
    swell: f64,
    /// 二阶高通、二阶低通（Butterworth）：b0 b1 b2 a1 a2；`lp2` 是四阶低通的第二节（没有就是 `None`）。
    hp: [f64; 5],
    lp: [f64; 5],
    lp2: Option<[f64; 5]>,
    /// 四阶高通的第二节（[`NoiseLayer::sharp`]）。
    hp2: Option<[f64; 5]>,
    /// 三路噪声（共用、左、右）：高通、低通、第二节低通、第二节高通的状态。
    state: [[f64; 8]; 3],
    gains: [f64; 2],
}

pub(crate) fn butterworth(highpass: bool, hz: f64, q: f64, sr: f64) -> [f64; 5] {
    let w = TAU * hz.clamp(10.0, 0.45 * sr) / sr;
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
    let a0 = 1.0 + alpha;
    let (b0, b1) = if highpass { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
    [b0 / a0, b1 / a0, b0 / a0, -2.0 * cos / a0, (1.0 - alpha) / a0]
}

pub(crate) fn biquad(c: &[f64; 5], z: &mut [f64], x: f64) -> f64 {
    let y = c[0] * x + z[0];
    z[0] = c[1] * x - c[3] * y + z[1];
    z[1] = c[2] * x - c[4] * y;
    y
}

impl Wash {
    fn new(piece: usize, seed: usize, def: &PieceDef, sr: f64) -> Self {
        let bands = def
            .layers
            .iter()
            .map(|l| {
                // 四阶 Butterworth 拆成两节：Q 0.541 与 1.307。
                let (q, second) = if l.steep { (0.541_196, Some(butterworth(false, l.hi, 1.306_563, sr))) } else { (core::f64::consts::FRAC_1_SQRT_2, None) };
                Band {
                    energy: [0.0; 2],
                    k: [1.0; 2],
                    pending: [0.0; 2],
                    rising: [0.0; 2],
                    swell: if def.swell_above > 0.0 { (2_500.0 / l.lo).powf(0.7).min(1.0) } else { 1.0 },
                    hp: butterworth(true, l.lo, if l.sharp { 0.541_196 } else { core::f64::consts::FRAC_1_SQRT_2 }, sr),
                    hp2: l.sharp.then(|| butterworth(true, l.lo, 1.306_563, sr)),
                    lp: butterworth(false, l.hi, q, sr),
                    lp2: second,
                    state: [[0.0; 8]; 3],
                    gains: [0.0; 2],
                }
            })
            .collect();
        // 两边的相关系数：与模态一样，相位差均匀分布在 ±π·width 里。
        let x = core::f64::consts::PI * def.width.clamp(0.0, 1.0);
        let correlation = if x < 1e-6 { 1.0 } else { (x.sin() / x).max(0.0) };
        Self {
            piece,
            bands,
            shared: correlation.sqrt(),
            apart: (1.0 - correlation).sqrt(),
            rng: 0xD1B5_4A32_D192_ED03 ^ ((piece + seed) as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            choked: false,
            swell_a: if def.mode_attack > 0.0 { 1.0 - (-1.0 / (def.mode_attack * sr)).exp() } else { 0.0 },
            two_stage: def.swell_above > 0.0,
        }
    }

    fn strike(&mut self, def: &PieceDef, set: PieceSet, level: f64, sr: f64) {
        self.choked = false;
        let level = level * (0.6 + 0.8 * set.tone);
        let decay = set.decay.clamp(0.1, 10.0);
        for (b, l) in self.bands.iter_mut().zip(def.layers) {
            let add = [(level * l.fast).powi(2), (level * l.slow).powi(2)];
            // 涌起来的件：除了鼓槌那一层，能量慢慢进来。
            if self.swell_a > 0.0 && l.fast_s >= 0.02 {
                b.pending = [b.pending[0] + add[0], b.pending[1] + add[1]];
            } else {
                b.energy = [b.energy[0] + add[0], b.energy[1] + add[1]];
            }
            // 鼓槌的"嗒"（几毫秒的那一层）不跟着 `decay` 变长。
            let stretch = |s: f64| if s < 0.02 { s } else { s * decay };
            b.k = [(-2.0 / (stretch(l.fast_s).max(0.0002) * sr)).exp(), (-2.0 / (stretch(l.slow_s).max(0.0002) * sr)).exp()];
            let a = ((set.pan + l.pan).clamp(-1.0, 1.0) + 1.0) * core::f64::consts::FRAC_PI_4;
            b.gains = [a.cos(), a.sin()];
        }
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>, sr: f64) {
        let channels = buffer.num_channels();
        let fade = if self.choked { (-2.0 / (0.005 * sr)).exp() } else { 1.0 };
        // 均匀分布的噪声 → 方差 1。
        let unit = 3f64.sqrt();
        for b in &mut self.bands {
            if b.energy[0] + b.energy[1] + b.pending[0] + b.pending[1] + b.rising[0] + b.rising[1] < 1e-14 {
                b.energy = [0.0; 2];
                b.pending = [0.0; 2];
                b.rising = [0.0; 2];
                continue;
            }
            for i in 0..buffer.num_frames() {
                if b.pending[0] + b.pending[1] + b.rising[0] + b.rising[1] > 0.0 {
                    let a = self.swell_a * b.swell;
                    let moved = [b.pending[0] * a, b.pending[1] * a];
                    b.pending = [b.pending[0] - moved[0], b.pending[1] - moved[1]];
                    if self.two_stage {
                        let on = [b.rising[0] * a, b.rising[1] * a];
                        b.rising = [b.rising[0] + moved[0] - on[0], b.rising[1] + moved[1] - on[1]];
                        b.energy = [b.energy[0] + on[0], b.energy[1] + on[1]];
                    } else {
                        b.energy = [b.energy[0] + moved[0], b.energy[1] + moved[1]];
                    }
                }
                let amp = (b.energy[0] + b.energy[1]).sqrt() * unit;
                b.energy[0] *= b.k[0] * fade;
                b.energy[1] *= b.k[1] * fade;
                // 三路噪声：共用的一路、左右各一路（不去相关的件只算共用的那一路）。
                let mut noise = [0.0; 3];
                for (o, state) in noise.iter_mut().zip(&mut b.state).take(if self.apart > 0.0 { 3 } else { 1 }) {
                    self.rng ^= self.rng << 13;
                    self.rng ^= self.rng >> 7;
                    self.rng ^= self.rng << 17;
                    let w = (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
                    let (hp, rest) = state.split_at_mut(2);
                    let (lp, rest) = rest.split_at_mut(2);
                    let (lp2, hp2) = rest.split_at_mut(2);
                    let mut y = biquad(&b.lp, lp, biquad(&b.hp, hp, w));
                    if let Some(second) = &b.lp2 {
                        y = biquad(second, lp2, y);
                    }
                    if let Some(second) = &b.hp2 {
                        y = biquad(second, hp2, y);
                    }
                    *o = y * amp;
                }
                let mut out = [noise[0] * self.shared + noise[1] * self.apart, noise[0] * self.shared + noise[2] * self.apart];
                out = [out[0] * b.gains[0], out[1] * b.gains[1]];
                if channels >= 2 {
                    buffer.channel_mut(0)[i] += out[0] as f32;
                    buffer.channel_mut(1)[i] += out[1] as f32;
                } else if channels == 1 {
                    buffer.channel_mut(0)[i] += ((out[0] + out[1]) * 0.5) as f32;
                }
            }
        }
    }
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

    /// 这一帧的左、右（还没乘声像的增益）；不去相关的件两边一样。
    fn sample(&mut self, sr: f64) -> [f64; 2] {
        if self.age.is_multiple_of(CONTROL) && self.glide != 0.0 {
            // 锣：音高按指数靠近终点（滑 `glide` 个半音）。
            let t = f64::from(self.age) / sr;
            self.bend = self.glide * (1.0 - (-t * self.glide_k).exp() - self.glide_from);
            self.retune(sr);
        }
        let mut x = 0.0;
        let mut side = [0.0; 2];
        // 涌起来的模态乘上现在涌到的比例；不涌的（`still`）原样。
        let g = if self.swell > 1e-4 { 1.0 - self.swell } else { 1.0 };
        for o in &mut self.oscs[..self.count] {
            let re = o.re * o.rot_re - o.im * o.rot_im;
            o.im = o.re * o.rot_im + o.im * o.rot_re;
            o.re = re;
            let g = if o.still { 1.0 } else { g };
            if self.wide {
                side[0] += g * (o.im * o.mix[0] + o.re * o.mix[1]);
                side[1] += g * (o.im * o.mix[2] + o.re * o.mix[3]);
            } else {
                x += g * o.im;
            }
        }
        if self.swell > 1e-4 {
            self.swell *= self.swell_k;
        }
        if self.drive > 0.0 {
            x = (self.drive * x).tanh() / self.drive;
        }
        if self.noise > 1e-6 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            let w = (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            self.lp += self.lp_a * (w - self.lp);
            let g = if self.age < self.burst_end {
                // 前几下：每下 3.5 ms 收掉，一下比一下响。
                let k = self.age / self.burst_gap;
                let a = f64::from(self.age % self.burst_gap) / (0.0035 * sr);
                self.noise * (0.5 + 0.25 * f64::from(k)) * (-a).exp()
            } else {
                self.noise
            };
            if self.steep {
                self.lp2 += self.lp_a * (self.lp - self.lp2);
                self.hp += self.hp_a * (self.lp2 - self.hp);
                let band = self.lp2 - self.hp;
                self.hp2 += self.hp_a * (band - self.hp2);
                x += (band - self.hp2) * g;
            } else {
                self.hp += self.hp_a * (self.lp - self.hp);
                x += (self.lp - self.hp) * g;
            }
            if self.age >= self.burst_end {
                self.noise *= self.noise_k;
            }
        }
        self.age += 1;
        [x + side[0], x + side[1]]
    }
}

/// 一套打击乐的发声：每敲一下是一个 [`Hit`]，响完就丢。
pub(crate) struct Kit {
    params: KitParams,
    hits: Vec<Hit>,
    /// 有噪声层的件各一组。
    washes: Vec<Wash>,
    sr: f64,
    seed: u64,
    /// 整套的压缩（[`KitParams::compress`]）的包络。
    comp_env: f64,
}

impl Kit {
    pub(crate) fn new(params: KitParams) -> Self {
        // 先留四下的地方（一下 5 KB 多；一套分了力度层的鼓有几百个 Kit），敲得密了再长。
        Self { hits: Vec::with_capacity(params.max_hits.clamp(1, 4)), params, washes: Vec::new(), sr: 48_000.0, seed: 0x9E37_79B9_7F4A_7C15, comp_env: 0.0 }
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.hits.clear();
        self.washes = self
            .params
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.layers.is_empty())
            .map(|(i, d)| Wash::new(i, self.params.noise_seed, d, spec.sample_rate))
            .collect();
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let Some(i) = self.params.defs.iter().position(|d| d.key == note.key) else { return };
        let (def, set) = (&self.params.defs[i], self.params.set[i]);
        // 力度分层的套件（[`crate::kit`]）把较轻的一层放大了交过来，会比 1 大不少。
        let v = (f64::from(velocity) * if accent { 1.25 } else { 1.0 }).clamp(0.0, 8.0);
        if def.choke > 0 {
            for h in &mut self.hits {
                if self.params.defs[h.piece].choke == def.choke {
                    h.choked = h.choked.min(1.0);
                    h.life = h.life.min(h.age + (0.015 * self.sr) as u32);
                }
            }
            for w in &mut self.washes {
                w.choked |= self.params.defs[w.piece].choke == def.choke;
            }
        }
        if self.hits.len() >= self.params.max_hits.max(1) {
            self.hits.remove(0);
        }
        let sr = self.sr;
        let f0 = def.f0 * (set.tune / 12.0).exp2();
        let mut oscs = [Osc::default(); MAX_MODES];
        let mut count = 0;
        let mut longest = def.noise_s * 6.0;
        self.seed = self.seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        // 0 … 1 的伪随机数（splitmix64）。
        let unit = |mut z: u64| {
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        for (m, &(ratio, amp, t60)) in def.modes.iter().take(MAX_MODES).enumerate() {
            let hz = f0 * ratio;
            if hz > 0.45 * sr {
                continue;
            }
            // 敲得越重、`tone` 越大，高的模式越响。
            let octaves = ratio.log2().max(0.0);
            let bright = v.powf(0.5 * octaves) * (1.0 + (set.tone - 0.5) * 0.8 * octaves).max(0.05);
            let t = t60 * set.decay.clamp(0.1, 10.0);
            longest = longest.max(t);
            oscs[count] = Osc {
                re: 0.0,
                im: 0.0,
                rot_re: 0.0,
                rot_im: 0.0,
                hz,
                loss: (-3.0 * 10f64.ln() / (t * sr)).exp(),
                mix: [0.0; 4],
                still: ratio <= def.swell_above,
            };
            if def.width > 0.0 {
                // 起始相位每一下随机（量出来的就用量出来的）；左右差多少每个模态固定（话筒不动）。
                let start =
                    def.phases.get(m).map_or_else(|| TAU * unit(self.seed ^ (m as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)), |phase| phase - TAU * hz / sr);
                let apart = TAU * def.width.min(1.0) * (unit((u64::from(def.key) << 32) | m as u64) - 0.5);
                oscs[count].mix = [start.cos(), start.sin(), (start + apart).cos(), (start + apart).sin()];
            }
            let amp = amp * bright * v.powf(def.mode_vel - 1.4);
            // 量出来的起振相位（振荡器先转一步再出声：往回拨一步）；没有的从零相位起（不出咔嗒）。
            // 去相关的件相位放在左右的混合系数里（见上），振荡器本身从零相位起。
            let start = if def.width > 0.0 { 0.0 } else { def.phases.get(m).map_or(0.0, |phase| phase - TAU * hz / sr) };
            oscs[count].re = amp * start.cos();
            oscs[count].im = amp * start.sin();
            count += 1;
        }
        let level = def.gain * set.level * v.powf(1.4) * 0.25;
        let pan = set.pan.clamp(-1.0, 1.0);
        let a = (pan + 1.0) * core::f64::consts::FRAC_PI_4;
        let mut hit = Hit {
            piece: i,
            oscs,
            count,
            // 涌起来的件从四分之一涌到满；分了涌与不涌的（大抄锣）上面的模态从很轻（-24 dB）涌起来。
            swell: if def.mode_attack <= 0.0 {
                0.0
            } else if def.swell_above > 0.0 {
                0.94
            } else {
                0.75
            },
            swell_k: (-1.0 / (def.mode_attack.max(1e-4) * sr)).exp(),
            drive: def.drive,
            age: 0,
            bend: 0.0,
            glide: def.glide * v.powf(def.glide_vel),
            glide_from: def.glide_from,
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
            burst_gap: ((0.010 * sr) as u32).max(1),
            burst_end: u32::from(def.bursts.saturating_sub(1)) * ((0.010 * sr) as u32).max(1),
            wide: def.width > 0.0,
        };
        hit.retune(sr);
        self.hits.push(hit);
        if let Some(w) = self.washes.iter_mut().find(|w| w.piece == i) {
            w.strike(def, set, level, sr);
        }
    }

    pub(crate) fn note_off(&mut self, _note: NoteRef, _velocity: f32) {}

    /// 掐断正在响的（15 ms 收掉）：套鼓的闭镲夹住开镲。
    pub(crate) fn choke(&mut self) {
        for h in &mut self.hits {
            h.life = h.life.min(h.age + (0.015 * self.sr) as u32);
        }
        self.washes.iter_mut().for_each(|w| w.choked = true);
    }

    /// 没有东西在响（可以不算这一块）。
    pub(crate) fn idle(&self) -> bool {
        self.hits.is_empty()
            && self.washes.iter().all(|w| w.bands.iter().all(|b| b.energy[0] + b.energy[1] + b.pending[0] + b.pending[1] + b.rising[0] + b.rising[1] < 1e-14))
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
                let x = h.sample(sr);
                if h.life.saturating_sub(h.age) < (0.015 * sr) as u32 {
                    h.choked = (h.choked - fade).max(0.0);
                }
                let (l, r) = (x[0] * h.choked * h.gains[0], x[1] * h.choked * h.gains[1]);
                if channels >= 2 {
                    buffer.channel_mut(0)[i] += l as f32;
                    buffer.channel_mut(1)[i] += r as f32;
                } else if channels == 1 {
                    buffer.channel_mut(0)[i] += ((l + r) * 0.5) as f32;
                }
            }
        }
        self.hits.retain(|h| h.age < h.life);
        for w in &mut self.washes {
            w.process(buffer, sr);
        }
        // 压缩：峰值包络（快起慢落），超过门限的按比例压下去，再整体补回来。
        if let Some((thr_db, ratio, attack, release, makeup_db)) = self.params.compress {
            let thr = 10f64.powf(thr_db / 20.0);
            let makeup = 10f64.powf(makeup_db / 20.0);
            let (ka, kr) = (1.0 - (-1.0 / (attack * sr)).exp(), 1.0 - (-1.0 / (release * sr)).exp());
            for i in 0..n {
                let peak = (0..channels).map(|c| f64::from(buffer.channel(c)[i].abs())).fold(0.0, f64::max);
                self.comp_env += (peak - self.comp_env) * if peak > self.comp_env { ka } else { kr };
                let g = (makeup * if self.comp_env > thr { (thr / self.comp_env).powf(1.0 - 1.0 / ratio) } else { 1.0 }) as f32;
                for c in 0..channels {
                    buffer.channel_mut(c)[i] *= g;
                }
            }
        }
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
