//! 从单击的录音里量出一套打击乐（`muser yinse extract --kit`）：每件乐器单独敲、一个文件一下、几种力度。
//!
//! 每件取最重的那一下量音色：
//! 1. 振动模式：一个一个找"衰减的正弦"——每一轮在剩下的声音里找和它最像的那个（频率、衰减多快），量出左右各自的振幅和相位，
//!    减掉，再找下一个（鼓的"咚"是十几二十个这样的正弦叠起来的，波形对得上才有那一下的劲）；像噪声的地方找不出突出的，不硬找。
//!    低处的主模式如果敲下去之后音高在滑（底鼓、通鼓、锣），先量出滑多少、多快，按滑音把时间拉直了再找。
//!    找完把余音查一遍：模型在后面比原来的声音响的，把衰减改短；
//! 2. 剩下的按频带量噪声：每个频带一段快的、一段慢的衰减（镲、响弦、鼓槌的"嗒"）；
//! 3. 左右声道的音量差是声像，相关程度是宽度（低、中、高三段各量各的）。
//!
//! 然后按频率分成低、中、高三个部分（鼓腔、鼓皮与响弦、镲与鼓槌）。录了几种力度的，最轻、最重和中间的一下各这么量一遍，
//! 做成力度层：轻敲不是重击变小了，是它自己那一下的音色。

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::path::{Path, PathBuf};

use manguebyte::dsp::fft::{Complex, RealFft};
use serde::Serialize;

use crate::kit::{Compress, Fallback, Glide, KitFile, Layer, Part, Piece, Stroke, Velocity, FORMAT, VERSION};
use crate::modal::{features, fit_decay, line, median, next_pow2, onset_of, read_stereo};
use crate::percussion::{biquad, butterworth};

const SR: f64 = 48_000.0;
/// 噪声层的频带（Hz）。
const EDGES: [f64; 11] = [40.0, 100.0, 200.0, 400.0, 800.0, 1_600.0, 3_200.0, 5_000.0, 7_500.0, 11_000.0, 16_000.0];
/// 低、中、高三个部分的分界（Hz）。
const SPLITS: [f64; 2] = [250.0, 2_500.0];
/// 一个部分最多几个模式（引擎一下最多 64 个）。
const PART_MODES: usize = 40;
/// 一件最多找几个模式。
const PIECE_MODES: usize = 72;

/// 一件的录音：名字、中文名、占哪些键、掐断哪些键、每个文件和它的力度（0 … 1）。
#[derive(Debug, Clone)]
pub struct PieceInput {
    pub name: String,
    pub zh: String,
    pub keys: Vec<u8>,
    pub chokes: Vec<u8>,
    pub hits: Vec<(PathBuf, f64)>,
}

/// 量完一件报告什么。
#[derive(Debug, Clone, Serialize)]
pub struct PieceReport {
    pub name: String,
    pub key: u8,
    pub hits: usize,
    /// 量了几个力度层。
    pub velocities: usize,
    /// 最重的那一层的模式数、噪声层数、部分数。
    pub modes: usize,
    pub layers: usize,
    pub parts: usize,
    /// 低处的主模式敲下去之后滑了多少半音（没滑就没有）。
    pub glide: Option<f64>,
    pub seconds: f64,
}

struct Mode {
    hz: f64,
    /// 两个声道平均的起始振幅。
    amp: f64,
    t60: f64,
    /// 中间声道（左右平均）的起振相位。
    phase: f64,
    /// 左、右各自的振幅和相位。
    sides: [(f64, f64); 2],
    /// 跟着低处的滑音走。
    glides: bool,
}

struct Band {
    lo: f64,
    hi: f64,
    /// 快、慢两段在起点的振幅（每声道的均方根）与时间常数（秒）。
    fast: f64,
    fast_s: f64,
    slow: f64,
    slow_s: f64,
    /// 这个频带多久到最响（秒）。
    peak_at: f64,
}

/// 一下的音色。
struct Timbre {
    modes: Vec<Mode>,
    bands: Vec<Band>,
    glide: Option<Glide>,
    pan: f64,
    /// 低、中、高三段各自的宽度（鼓的低处在正中，上面是头顶话筒收的一片）。
    width: [f64; 3],
    seconds: f64,
}

fn band_pass(x: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    // 和引擎的噪声层（`steep`、`sharp` 都开）一样的滤波器：四阶高通 + 四阶低通。
    let c = [
        butterworth(true, lo, 0.541_196, SR),
        butterworth(true, lo, 1.306_563, SR),
        butterworth(false, hi, 0.541_196, SR),
        butterworth(false, hi, 1.306_563, SR),
    ];
    let mut z = [[0.0; 2]; 4];
    x.iter().map(|&v| c.iter().zip(&mut z).fold(v, |y, (c, z)| biquad(c, z, y))).collect()
}

/// 量的时候用的频带：八阶的高通、低通（边上很陡）。引擎的噪声层边上很缓，直接用它来量的话，旁边频带里很响的东西
///（底鼓 60 Hz 的那一下）会漏进来，被当成这个频带里的噪声。
fn sharp_band(x: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    const Q: [f64; 4] = [0.509_795_6, 0.601_344_9, 0.899_976_2, 2.562_915_4];
    let mut y = x.to_vec();
    for (highpass, hz) in [(true, lo), (false, hi)] {
        if (highpass && hz <= 20.0) || (!highpass && hz >= 20_000.0) {
            continue;
        }
        for q in Q {
            let (c, mut z) = (butterworth(highpass, hz, q, SR), [0.0; 2]);
            y.iter_mut().for_each(|v| *v = biquad(&c, &mut z, *v));
        }
    }
    y
}

fn white(n: usize) -> Vec<f64> {
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    (0..n)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            ((rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * 3f64.sqrt()
        })
        .collect()
}

fn mean_square(x: &[f64]) -> f64 {
    x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64
}

/// 引擎的第 i 层噪声（振幅 1）在第 j 个量的频带里有多少能量：`[j][i]`。对角线是它自己频带里的，其余是漏到别的频带里的。
fn leakage() -> &'static Vec<Vec<f64>> {
    static M: std::sync::OnceLock<Vec<Vec<f64>>> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let noise = white(96_000);
        let layers: Vec<Vec<f64>> = EDGES.windows(2).map(|e| band_pass(&noise, e[0], e[1])).collect();
        EDGES.windows(2).map(|e| layers.iter().map(|l| mean_square(&sharp_band(l, e[0], e[1])[8_000..])).collect()).collect()
    })
}

/// 各频带量到的能量 → 各层该给多少能量（解掉互相漏的那部分；解出来太小的至少留量到的一成）。
fn unleak(measured: &[f64]) -> Vec<f64> {
    let m = leakage();
    let n = measured.len();
    let mut a: Vec<Vec<f64>> = (0..n).map(|j| m[j].iter().copied().chain([measured[j]]).collect()).collect();
    for c in 0..n {
        let p = (c..n).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs())).unwrap_or(c);
        a.swap(c, p);
        if a[c][c].abs() < 1e-30 {
            continue;
        }
        for r in 0..n {
            if r != c {
                let (f, pivot) = (a[r][c] / a[c][c], a[c].clone());
                a[r].iter_mut().zip(&pivot).skip(c).for_each(|(x, p)| *x -= f * p);
            }
        }
    }
    (0..n).map(|i| (a[i][n] / a[i][i].max(1e-30)).max(0.1 * measured[i] / m[i][i].max(1e-30))).collect()
}

/// 从起音开始的幅度谱：窗从 1 落到 0（起音那一下最响，不能像汉宁窗那样把它压掉）。
fn onset_spectrum(x: &[f64]) -> (Vec<f64>, f64) {
    let n = x.len().min(SR as usize);
    let size = next_pow2(n).max(32_768);
    let mut input = vec![0.0; size];
    for (i, v) in x[..n].iter().enumerate() {
        input[i] = v * (0.5 + 0.5 * (PI * i as f64 / n as f64).cos());
    }
    let mut fft = RealFft::<f64>::new(size);
    let mut out = vec![Complex::ZERO; fft.spectrum_len()];
    fft.forward(&input, &mut out);
    (out.iter().map(|c| c.norm_sqr().sqrt()).collect(), SR / size as f64)
}

/// 每个样本上"按没滑的频率算过了多少秒"：滑音是所有模式一起乘一个比例，所以相位是 2π·频率·这个数。
fn warped_time(len: usize, glide: Option<Glide>) -> Vec<f64> {
    let mut out = Vec::with_capacity(len);
    let mut t = 0.0;
    for i in 0..len {
        out.push(t);
        let bend = glide.map_or(0.0, |g| g.semitones * (1.0 - (-(i as f64 / SR) / g.seconds).exp() - g.from));
        t += (bend / 12.0).exp2() / SR;
    }
    out
}

/// 一个频率在各帧上的复振幅（汉宁窗；`clock` 是 [`warped_time`]）。
fn demodulate(x: &[f64], hz: f64, window: usize, starts: &[usize], clock: &[f64]) -> Vec<Complex<f64>> {
    let w: Vec<f64> = (0..window).map(|i| 0.5 - 0.5 * (TAU * i as f64 / window as f64).cos()).collect();
    let norm: f64 = w.iter().sum();
    starts
        .iter()
        .filter(|&&s| s + window <= x.len())
        .map(|&s| {
            let (mut re, mut im) = (0.0, 0.0);
            for i in 0..window {
                let (a, p) = (x[s + i] * w[i], -TAU * hz * clock[s + i]);
                re += a * p.cos();
                im += a * p.sin();
            }
            Complex::new(2.0 * re / norm, 2.0 * im / norm)
        })
        .collect()
}

/// 低处的主模式敲下去之后音高滑不滑（底鼓、通鼓、锣）：先滤到它附近（旁边的模式会把相位搅乱），量各帧的瞬时频率；
/// 头十几毫秒和后面差半个半音以上、而且是顺着一条指数曲线收到终点的，才算。
fn measure_glide(x: &[f64], hz: f64) -> Option<Glide> {
    let x = band_pass(x, 0.6 * hz, 1.6 * hz);
    let window = ((2.0 * SR / hz) as usize).clamp((0.008 * SR) as usize, (0.03 * SR) as usize);
    let hop = (0.001 * SR) as usize;
    let starts: Vec<usize> = (0..).map(|k| k * hop).take_while(|s| s + window <= x.len() && *s < (0.3 * SR) as usize).collect();
    let clock = warped_time(x.len(), None);
    let z = demodulate(&x, hz, window, &starts, &clock);
    if z.len() < 40 {
        return None;
    }
    let top = z.iter().map(|c| c.norm_sqr().sqrt()).fold(0.0, f64::max);
    // 相邻两帧的相位差 → 瞬时频率（只用还够响的帧；这一帧说的是窗中间那一刻）。
    let track: Vec<(f64, f64)> = z
        .windows(2)
        .enumerate()
        .take_while(|(_, w)| w[1].norm_sqr().sqrt() > 0.1 * top)
        .map(|(k, w)| {
            let d = (w[1].im * w[0].re - w[1].re * w[0].im).atan2(w[1].re * w[0].re + w[1].im * w[0].im);
            (k as f64 * 0.001 + 0.5 * window as f64 / SR, hz + d * SR / (TAU * hop as f64))
        })
        .collect();
    let mean = |lo: f64, hi: f64| {
        let v: Vec<f64> = track.iter().filter(|p| p.0 >= lo && p.0 < hi).map(|p| p.1).collect();
        (v.len() >= 4).then(|| median(v))
    };
    let (first, last) = (track.first()?.0, track.last()?.0);
    if last - first < 0.04 {
        return None;
    }
    let (start, end) = (mean(first, first + 0.012)?, mean(0.12, 0.25).or_else(|| mean((last - 0.03).max(first + 0.02), last + 0.001))?);
    if start <= 0.0 || end <= 0.0 || (12.0 * (start / end).log2()).abs() < 0.35 {
        return None;
    }
    // 离终点的距离按指数收掉：在对数上拟合一条直线，延回到起音那一刻就是一开始高（低）了多少，斜率是多快。
    let sign = (start - end).signum();
    let points: Vec<(f64, f64)> =
        track.iter().take_while(|p| sign * (p.1 - end) > 0.1 * (start - end).abs()).filter(|p| p.0 < 0.12).map(|p| (p.0, (sign * (p.1 - end)).ln())).collect();
    let (b, m) = line(&points)?;
    // 拟合得不好（不是一条顺着收的线）就不是滑音。
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64;
    let (res, var) = points.iter().fold((0.0, 0.0), |(r, v), p| (r + (p.1 - b - m * p.0).powi(2), v + (p.1 - mean_y).powi(2)));
    if points.len() < 8 || m > -5.0 || res > 0.3 * var {
        return None;
    }
    // 十来毫秒就"滑"完的是敲击那一下的瞬态，不是滑音。
    if -1.0 / m < 0.02 {
        return None;
    }
    let apart = b.exp().min(3.0 * (start - end).abs());
    let semitones = 12.0 * ((end + sign * apart) / end).log2();
    if !(0.5..=5.0).contains(&semitones.abs()) {
        return None;
    }
    Some(Glide {
        semitones: -((semitones * 100.0).round() / 100.0),
        seconds: ((-1.0 / m).clamp(0.02, 0.15) * 1000.0).round() / 1000.0,
        from: 1.0,
        velocity: 0.0,
    })
}

/// 一个模式在 `t` 秒（[`warped_time`] 的钟）上的样子。
#[cfg(test)]
fn mode_wave(amp: f64, t60: f64, hz: f64, phase: f64, i: usize, clock: &[f64]) -> f64 {
    amp * (-(i as f64 / SR) * 1000f64.ln() / t60).exp() * (TAU * hz * clock[i] + phase).sin()
}

/// 按滑音把时间拉直：输出的第 k 个样本是"按没滑的频率过了 k 个样本"的那一刻（[`warped_time`] 的反函数，线性插值）。
fn straighten(x: &[f64], clock: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len());
    let mut i = 0;
    for k in 0.. {
        let u = k as f64 / SR;
        while i + 1 < x.len() && clock[i + 1] <= u {
            i += 1;
        }
        if i + 1 >= x.len() {
            break;
        }
        let f = ((u - clock[i]) / (clock[i + 1] - clock[i]).max(1e-12)).clamp(0.0, 1.0);
        out.push(x[i] + (x[i + 1] - x[i]) * f);
    }
    out
}

/// 找"衰减的正弦"：对每一种衰减速度，把声音乘上同样的衰减再做一次 FFT，谱上每一格就是这个频率、这个衰减的正弦和声音有多像。
struct Pursuit {
    fft: RealFft<f64>,
    size: usize,
    /// 每格多少 Hz。
    bin: f64,
    input: Vec<f64>,
    output: Vec<Complex<f64>>,
    /// 每种衰减（时间常数，秒）的谱：这一格的正弦解释了多少能量。
    power: Vec<Vec<f64>>,
    /// 时间常数最长试到多少。
    longest: f64,
}

/// 试哪些衰减：时间常数 4 ms … 4 s（衰减到 -60 dB 是它的 6.9 倍）。
const TAUS: [f64; 11] = [0.004, 0.008, 0.015, 0.03, 0.06, 0.12, 0.25, 0.5, 1.0, 2.0, 4.0];

impl Pursuit {
    fn new(len: usize, longest: f64) -> Self {
        let size = next_pow2(len).max(65_536);
        let fft = RealFft::<f64>::new(size);
        let bins = fft.spectrum_len();
        Self {
            fft,
            size,
            bin: SR / size as f64,
            input: vec![0.0; size],
            output: vec![Complex::ZERO; bins],
            power: vec![vec![0.0; bins]; TAUS.len()],
            longest: longest.max(0.008),
        }
    }

    /// 一个频率、一个衰减的正弦解释了 `x` 多少能量（直接算，不经 FFT）。
    fn at(x: &[f64], hz: f64, tau: f64) -> f64 {
        let m = x.len().min((8.0 * tau * SR) as usize + 64);
        let (mut re, mut im, mut norm) = (0.0, 0.0, 0.0);
        for (i, v) in x.iter().enumerate().take(m) {
            let (e, p) = ((-(i as f64) / (tau * SR)).exp(), TAU * hz * i as f64 / SR);
            re += v * e * p.cos();
            im += v * e * p.sin();
            norm += e * e;
        }
        2.0 * (re * re + im * im) / norm.max(1e-30)
    }

    /// `from` … `to` Hz 里和 `x` 最像的衰减正弦：(频率, 时间常数, 解释的能量)。要比同一张谱上周围的格子突出十倍以上
    ///（噪声的谱上最高的格子也就比中位数高七八倍）；不突出的那一片记进 `blocked`，以后不再看。都不突出就是没有了。
    fn best(&mut self, x: &[f64], from: f64, to: f64, blocked: &mut Vec<(f64, f64)>) -> Option<(f64, f64, f64)> {
        let n = x.len().min(self.size);
        let taus = TAUS.iter().take_while(|t| **t <= self.longest).count().max(2);
        for (j, &tau) in TAUS.iter().enumerate().take(taus) {
            let (mut norm, m) = (0.0, n.min((8.0 * tau * SR) as usize + 64));
            self.input.iter_mut().for_each(|v| *v = 0.0);
            for (i, (y, v)) in self.input.iter_mut().zip(x).enumerate().take(m) {
                let e = (-(i as f64) / (tau * SR)).exp();
                *y = v * e;
                norm += e * e;
            }
            self.fft.forward(&self.input, &mut self.output);
            for (p, c) in self.power[j].iter_mut().zip(&self.output) {
                *p = 2.0 * c.norm_sqr() / norm;
            }
        }
        let (lo, hi) = (((from / self.bin) as usize).max(2), ((to / self.bin) as usize).min(self.power[0].len() - 3));
        let mut free = vec![true; hi + 1];
        let block = |free: &mut Vec<bool>, b: (f64, f64)| {
            let (a, z) = (((b.0 / self.bin).floor().max(0.0) as usize).min(hi), ((b.1 / self.bin).ceil().max(0.0) as usize).min(hi));
            free[a..=z].iter_mut().for_each(|f| *f = false);
        };
        blocked.iter().for_each(|&b| block(&mut free, b));
        let total: f64 = x[..n].iter().map(|v| v * v).sum();
        for _ in 0..400 {
            let (mut top, mut at) = (0.0, None);
            for (j, row) in self.power.iter().enumerate().take(taus) {
                for k in lo..hi {
                    if row[k] > top && free[k] && row[k] >= row[k - 1] && row[k] >= row[k + 1] {
                        (top, at) = (row[k], Some((j, k)));
                    }
                }
            }
            let (j, k) = at?;
            let hz = k as f64 * self.bin;
            // 和周围比：窗要比这个衰减的正弦自己的峰宽得多（衰减越快峰越宽）。
            let w = ((f64::max(60.0 + 0.04 * hz, 8.0 / (TAU * TAUS[j])) / self.bin) as usize).max(8);
            let around = median(self.power[j][k.saturating_sub(w)..(k + w).min(self.power[j].len())].to_vec());
            // 算数的：比周围突出十倍，或者一个就解释了剩下的声音的 2% 以上（鼓的低处几乎全是模式，挨得近、互相垫高了"周围"）。
            if top < 10.0 * around && top < 0.02 * total {
                let b = (hz - 0.5 * w as f64 * self.bin, hz + 0.5 * w as f64 * self.bin);
                block(&mut free, b);
                blocked.push(b);
                continue;
            }
            // 频率：三格的抛物线插值；衰减：在左右两档之间细找。
            let row = &self.power[j];
            let (a, b, c) = (row[k - 1].max(1e-300).ln(), row[k].max(1e-300).ln(), row[k + 1].max(1e-300).ln());
            let hz = (k as f64 + ((a - c) / (2.0 * (a - 2.0 * b + c)).min(-1e-12)).clamp(-0.5, 0.5)) * self.bin;
            let (mut lo_t, mut hi_t) = (TAUS[j.saturating_sub(1)].ln(), TAUS[(j + 1).min(taus - 1)].min(self.longest).ln());
            for _ in 0..14 {
                let (p, q) = (lo_t + 0.382 * (hi_t - lo_t), lo_t + 0.618 * (hi_t - lo_t));
                if Self::at(&x[..n], hz, p.exp()) > Self::at(&x[..n], hz, q.exp()) {
                    hi_t = q;
                } else {
                    lo_t = p;
                }
            }
            let tau = (0.5 * (lo_t + hi_t)).exp();
            return Some((hz, tau, Self::at(&x[..n], hz, tau)));
        }
        None
    }
}

/// 最重的那一下 → 音色。`lr` 从起音开始。
fn analyze(lr: &[Vec<f64>; 2]) -> Timbre {
    let n = lr[0].len();
    let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
    // 低处最响的那个峰滑不滑（它得是这一下里数得着的：比最响的峰低不到 12 dB）。
    let glide = {
        let (mag, bin) = onset_spectrum(&mid);
        let top = mag.iter().copied().fold(0.0, f64::max);
        let (lo, hi) = ((30.0 / bin) as usize + 1, ((400.0 / bin) as usize).min(mag.len() - 2));
        (lo..hi).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).filter(|&k| mag[k] > 0.25 * top).and_then(|k| measure_glide(&mid, k as f64 * bin))
    };
    let clocks = [warped_time(n, None), warped_time(n, glide)];
    // 模式：一个一个找"衰减的正弦"——每一轮在剩下的声音里找和它最像的那个（频率、衰减多快），量出左右各自的振幅和相位，减掉，再找下一个。
    // 滑着的、挨得近的、十几毫秒就没了的都是这么几个衰减正弦叠起来的。像噪声的地方（镲的那一片）找不出比周围突出的，留给噪声层。
    // 低处在滑音的：250 Hz 以下先按滑音把时间拉直了再找（找到的是滑完以后的频率，引擎演奏时再滑回去）。
    let mut rest = [lr[0].clone(), lr[1].clone()];
    let mut modes: Vec<Mode> = Vec::new();
    let stages: &[(f64, f64, bool)] = if glide.is_some() { &[(30.0, SPLITS[0], true), (SPLITS[0], 16_000.0, false)] } else { &[(30.0, 16_000.0, false)] };
    let window = n.min((2.0 * SR) as usize);
    // 整个声音在 `n` 个样本里退了 70 dB：单个模式的时间常数不会比它的四分之一还长。
    let mut pursuit = Pursuit::new(window, n as f64 / SR / 4.0);
    for &(from, to, warp) in stages {
        let clock = &clocks[usize::from(warp)];
        let mut blocked: Vec<(f64, f64)> = Vec::new();
        let mut loudest = 0.0;
        for _ in 0..4 * PIECE_MODES {
            if modes.len() >= PIECE_MODES {
                break;
            }
            let rest_mid: Vec<f64> = rest[0][..window].iter().zip(&rest[1][..window]).map(|(a, b)| 0.5 * (a + b)).collect();
            let search = if warp { straighten(&sharp_band(&rest_mid, 20.0, 1.5 * to), clock) } else { rest_mid };
            let Some((hz, tau, power)) = pursuit.best(&search, from, to, &mut blocked) else { break };
            loudest = f64::max(loudest, power);
            if power < loudest * 10f64.powf(-45.0 / 10.0) {
                break;
            }
            // 左、右、中间各自最合适的振幅和相位（最小二乘）；减掉。
            let fit = |x: &[f64]| -> (f64, f64) {
                let m = x.len().min((8.0 * tau * SR) as usize + 64);
                let (mut cc, mut cs, mut ss, mut xc, mut xs) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for (i, (v, t)) in x.iter().zip(clock.iter()).enumerate().take(m) {
                    let (e, p) = ((-(i as f64) / (tau * SR)).exp(), TAU * hz * t);
                    let (c, s) = (e * p.cos(), e * p.sin());
                    cc += c * c;
                    cs += c * s;
                    ss += s * s;
                    xc += v * c;
                    xs += v * s;
                }
                let det = (cc * ss - cs * cs).max(1e-30);
                // x ≈ a·cos + b·sin = A·sin(θ + φ)，A = √(a² + b²)，φ = atan2(a, b)。
                let (a, b) = ((xc * ss - xs * cs) / det, (xs * cc - xc * cs) / det);
                ((a * a + b * b).sqrt(), a.atan2(b))
            };
            let rest_mid: Vec<f64> = rest[0].iter().zip(&rest[1]).map(|(a, b)| 0.5 * (a + b)).collect();
            let ((al, pl), (ar, pr), (_, pm)) = (fit(&rest[0]), fit(&rest[1]), fit(&rest_mid));
            let t60 = (tau * 1000f64.ln()).clamp(0.01, 30.0);
            for (x, (a, p)) in rest.iter_mut().zip([(al, pl), (ar, pr)]) {
                let m = x.len().min((8.0 * tau * SR) as usize + 64);
                for (i, v) in x.iter_mut().enumerate().take(m) {
                    *v -= a * (-(i as f64) / (tau * SR)).exp() * (TAU * hz * clock[i] + p).sin();
                }
            }
            modes.push(Mode { hz, amp: ((al * al + ar * ar) / 2.0).sqrt(), t60, phase: pm, sides: [(al, pl), (ar, pr)], glides: warp });
            // 挨在一起的几个（差不到 4 Hz 或 1%）是在凑同一个模式的包络（先快后慢）：最多两个，之后这一小段不再找。
            // 不限的话会凑出一堆互相抵消的，量的那一段里对得上，后面的余音就乱了。
            let near = (0.01 * hz).max(4.0);
            if modes.iter().filter(|m| (m.hz - hz).abs() < near).count() >= 2 {
                blocked.push((hz - near, hz + near));
            }
        }
    }
    // 余音查一遍：找的时候只在乎头两秒里的能量，尾巴上差几十 dB 它不在乎，耳朵在乎——几个挨着的模式在头上互相抵着，
    // 后面抵不住了就会多出一截余音。每个频带在 0.25 … 2.5 秒上比一比模型和原来的声音：模型响出 6 dB 以上的，
    // 把这个频带里还在响的模式的衰减缩短一成，再比，直到不超。
    let model = |modes: &[Mode], c: usize| -> Vec<f64> {
        let mut out = vec![0.0; n];
        for m in modes {
            let (clock, (a, p), k) = (&clocks[usize::from(m.glides)], m.sides[c], 1000f64.ln() / (m.t60 * SR));
            let len = n.min((1.2 * m.t60 * SR) as usize + 64);
            for (i, v) in out.iter_mut().enumerate().take(len) {
                *v += a * (-(i as f64) * k).exp() * (TAU * m.hz * clock[i] + p).sin();
            }
        }
        out
    };
    let checks: Vec<usize> = [0.25, 0.5, 1.0, 1.5, 2.5].iter().map(|t| (t * SR) as usize).filter(|s| s + (0.1 * SR) as usize <= n).collect();
    let energy_at = |x: &[f64], s: usize| mean_square(&x[s..s + (0.1 * SR) as usize]);
    if !checks.is_empty() && !modes.is_empty() {
        let original: Vec<[Vec<f64>; 2]> = EDGES.windows(2).map(|e| [sharp_band(&lr[0], e[0], e[1]), sharp_band(&lr[1], e[0], e[1])]).collect();
        for _ in 0..24 {
            let synth = [model(&modes, 0), model(&modes, 1)];
            let mut changed = false;
            for (e, orig) in EDGES.windows(2).zip(&original) {
                if !modes.iter().any(|m| m.hz >= e[0] && m.hz < e[1]) {
                    continue;
                }
                let ours = [sharp_band(&synth[0], e[0], e[1]), sharp_band(&synth[1], e[0], e[1])];
                let top = energy_at(&orig[0], 0) + energy_at(&orig[1], 0);
                // 哪个时刻起模型比原来的响出一倍（能量四倍）以上（比开头低 80 dB 以下的不管）。
                let Some(&from) = checks.iter().find(|&&s| {
                    let (o, m) = (energy_at(&orig[0], s) + energy_at(&orig[1], s), energy_at(&ours[0], s) + energy_at(&ours[1], s));
                    m > 4.0 * o && m > 1e-8 * top
                }) else {
                    continue;
                };
                for m in modes.iter_mut().filter(|m| m.hz >= e[0] && m.hz < e[1] && m.t60 > 0.5 * from as f64 / SR) {
                    m.t60 *= 0.9;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // 改过衰减以后，"剩下的"重新算：原来的声音减掉所有的模式。
        for (c, x) in rest.iter_mut().enumerate() {
            let synth = model(&modes, c);
            x.iter_mut().zip(&lr[c]).zip(&synth).for_each(|((r, o), m)| *r = o - m);
        }
    }
    // 剩下的：每个频带的噪声，快、慢两段。包络每 4 ms 一个点，窗长至少两个周期（低频带的均方根才稳）。
    let hop = (0.004 * SR) as usize;
    let mut fits = Vec::new();
    for e in EDGES.windows(2) {
        let (l, r) = (sharp_band(&rest[0], e[0], e[1]), sharp_band(&rest[1], e[0], e[1]));
        let window = ((2.0 * SR / e[0]) as usize).max(hop);
        let env: Vec<f64> = (0..n / hop)
            .take(1_000)
            .map(|k| {
                let (a, b) = ((k * hop).saturating_sub(window / 2), (k * hop + window / 2).clamp(1, n));
                ((mean_square(&l[a..b]) + mean_square(&r[a..b])) / 2.0).sqrt()
            })
            .collect();
        if env.len() < 4 {
            fits.push((0.0, 0.008, 0.0, 1.0, 0.0));
            continue;
        }
        let t: Vec<f64> = (0..env.len()).map(|k| k as f64 * hop as f64 / SR).collect();
        let first = t.iter().take_while(|&&x| x < 0.3).count().clamp(1, env.len());
        let kp = (0..first).max_by(|&a, &b| env[a].total_cmp(&env[b])).unwrap_or(0);
        // 比峰值低 50 dB 以下的不拟合（录音的底噪、房间的余响）。
        let fit = match fit_decay(&t, &env, env[kp] * 10f64.powf(-50.0 / 20.0) / 2.5, 0.3, 0.012, &[0.02, 0.05, 0.1, 0.2, 0.4, 0.8], 0.15) {
            Some(d) => {
                let slow_s = if d.slope < -0.5 { (20.0 / 10f64.ln() / -d.slope).clamp(0.003, 8.0) } else { 3.0 };
                // 引擎里两段的能量相加：快的那段补到起点的振幅是两段之和。
                (((d.amp + d.fast_amp).powi(2) - d.amp.powi(2)).max(0.0).sqrt(), (d.fast_t60 / 1000f64.ln()).clamp(0.0015, 3.0), d.amp, slow_s, t[kp])
            }
            None => (env[kp], 0.008, 0.0, 1.0, t[kp]),
        };
        fits.push(fit);
    }
    // 引擎的噪声层边上缓，会漏到旁边的频带里：按起点的能量解一遍，各层给多少才正好。
    let measured: Vec<f64> = fits.iter().map(|f| f.0 * f.0 + f.2 * f.2).collect();
    let wanted = unleak(&measured);
    let mut bands = Vec::new();
    for (i, (e, f)) in EDGES.windows(2).zip(&fits).enumerate() {
        if measured[i] <= 0.0 {
            continue;
        }
        // `fast`、`slow`：引擎这一层的振幅该是多少（单位振幅的那一层在自己频带里的能量是 leakage()[i][i]）。
        let k = (wanted[i] / measured[i]).sqrt();
        bands.push(Band { lo: e[0], hi: e[1], fast: f.0 * k, fast_s: f.1, slow: f.2 * k, slow_s: f.3, peak_at: f.4 });
    }
    // 声像：左右的能量比；宽度：800 Hz 以上头半秒左右的相关系数（两边一样是 0）。
    let energy = |x: &[f64]| x.iter().map(|v| v * v).sum::<f64>().max(1e-20);
    let pan = (4.0 / PI * (energy(&lr[1]) / energy(&lr[0])).sqrt().atan() - 1.0).clamp(-1.0, 1.0);
    let m = n.min((0.5 * SR) as usize);
    let width = [(20.0, SPLITS[0]), (SPLITS[0], SPLITS[1]), (SPLITS[1], 20_000.0)].map(|(lo, hi)| {
        let (hl, hr) = (sharp_band(&lr[0][..m], lo, hi), sharp_band(&lr[1][..m], lo, hi));
        let correlation = hl.iter().zip(&hr).map(|(a, b)| a * b).sum::<f64>() / (energy(&hl) * energy(&hr)).sqrt();
        if correlation > 0.9 {
            return 0.0;
        }
        // sin(πw) / (πw) = 相关系数。
        let (mut a, mut b) = (0.0, 1.0);
        for _ in 0..30 {
            let w: f64 = 0.5 * (a + b);
            if (PI * w).sin() / (PI * w) > correlation.max(0.0) {
                a = w;
            } else {
                b = w;
            }
        }
        ((a + b) * 50.0).round() / 100.0
    });
    Timbre { modes, bands, glide, pan: (pan * 100.0).round() / 100.0, width, seconds: n as f64 / SR }
}

/// 读一下：从起音前 1 ms 开始，到比峰值低 70 dB 为止（至少 0.2 秒）。
fn read_hit(path: &Path) -> Result<[Vec<f64>; 2], String> {
    let lr = read_stereo(path, SR as u32)?;
    let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
    let peak = mid.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if peak < 1e-5 {
        return Err(format!("'{}' is silent", path.display()));
    }
    let on = onset_of(&mid).saturating_sub((0.001 * SR) as usize);
    let hop = (0.01 * SR) as usize;
    let quiet = peak * 10f64.powf(-70.0 / 20.0);
    let last = (on..mid.len()).step_by(hop).rfind(|&s| mid[s..(s + hop).min(mid.len())].iter().any(|v| v.abs() > quiet)).unwrap_or(on) + hop;
    let end = last.max(on + (0.2 * SR) as usize).min(mid.len());
    if end < on + (0.05 * SR) as usize {
        return Err(format!("'{}': less than 50 ms of sound after the stroke", path.display()));
    }
    Ok([lr[0][on..end].to_vec(), lr[1][on..end].to_vec()])
}

fn round(v: f64, digits: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return 0.0;
    }
    let scale = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

/// GM 鼓键常用的名字（`36` → `kick`）；没有名字的键是 `key<键>`。
pub fn key_name(key: u8) -> String {
    const GM: [&str; 47] = [
        "kick2",
        "kick",
        "side_stick",
        "snare",
        "clap",
        "snare2",
        "floor_tom_low",
        "closed_hat",
        "floor_tom",
        "pedal_hat",
        "tom_low",
        "open_hat",
        "tom_mid_low",
        "tom_mid",
        "crash",
        "tom_high",
        "ride",
        "china",
        "ride_bell",
        "tambourine",
        "splash",
        "cowbell",
        "crash2",
        "vibraslap",
        "ride2",
        "bongo_high",
        "bongo_low",
        "conga_mute",
        "conga_high",
        "conga_low",
        "timbale_high",
        "timbale_low",
        "agogo_high",
        "agogo_low",
        "cabasa",
        "maracas",
        "whistle_short",
        "whistle_long",
        "guiro_short",
        "guiro_long",
        "claves",
        "woodblock_high",
        "woodblock_low",
        "cuica_mute",
        "cuica_open",
        "triangle_mute",
        "triangle_open",
    ];
    key.checked_sub(35).and_then(|i| GM.get(usize::from(i))).map_or_else(|| format!("key{key}"), |n| (*n).to_string())
}

/// 文件名 → (件名, 键, 力度)：`kick_v100`、`36_v0.95`、`36_kick_ff`、`open_hat-mf-rr2`。
/// 力度写 `v80` `vel100`（0 … 127）、`v0.7` 或 `pp` … `ff`；开头单独的数字是键；`rr<数字>`（同一力度的第几个样本）不算名字。
pub fn hit_name(stem: &str) -> (String, Option<u8>, Option<f64>) {
    let (mut name, mut key, mut velocity): (Vec<String>, _, _) = (Vec::new(), None, None);
    for token in stem.split(|c: char| !c.is_ascii_alphanumeric() && c != '.').filter(|t| !t.is_empty()) {
        let t = token.to_ascii_lowercase();
        let number = t.strip_prefix("vel").or_else(|| t.strip_prefix('v')).and_then(|n| n.parse::<f64>().ok());
        let dynamic = [("ppp", 0.15), ("pp", 0.25), ("p", 0.4), ("mp", 0.55), ("mf", 0.7), ("f", 0.85), ("ff", 0.95), ("fff", 1.0)]
            .iter()
            .find(|d| d.0 == t)
            .map(|d| d.1);
        if velocity.is_none() && (number.is_some() || dynamic.is_some()) {
            velocity = number.map(|v| if v <= 1.0 { v } else { v / 127.0 }.clamp(0.01, 1.0)).or(dynamic);
        } else if key.is_none() && name.is_empty() && t.parse::<u8>().is_ok_and(|k| k <= 127) {
            key = t.parse::<u8>().ok();
        } else if !(t.starts_with("rr") && t[2..].chars().all(|c| c.is_ascii_digit())) {
            name.push(t);
        }
    }
    (name.join("_"), key, velocity)
}

/// 一堆单击的录音 → 每件的输入。键从文件名里的数字、`meta`（文件夹里的 `kit.json`：`{"pieces": {"<名>": {"key": 36, "keys": [36, 35],
/// "zh": "底鼓", "chokes": [46]}}}`）或 GM 鼓件名（kick、snare、closed_hat …）来；没写力度的按响度从轻到重排。
/// 没写掐断的：有开镲（46）的话，闭镲（42）、踩镲（44）掐断它。
pub fn group(files: &[PathBuf], meta: &serde_json::Value) -> Result<Vec<PieceInput>, String> {
    let mut pieces: Vec<PieceInput> = Vec::new();
    let mut unknown: Vec<(usize, PathBuf)> = Vec::new();
    for file in files {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (name, key, velocity) = hit_name(&stem);
        let about = &meta["pieces"][name.as_str()];
        let key = key
            .or_else(|| about["key"].as_u64().map(|k| k as u8))
            .or_else(|| about["keys"][0].as_u64().map(|k| k as u8))
            .or_else(|| (35..=81).find(|k| key_name(*k) == name))
            .or_else(|| muser_core::grid::drum_key(&name))
            .ok_or_else(|| format!("'{}': which key is '{name}'? Start the file name with the key (36_{stem}), use a GM piece name (kick, snare, closed_hat, crash ...) or list it in kit.json", file.display()))?;
        let name = if name.is_empty() { key_name(key) } else { name };
        let i = pieces.iter().position(|p| p.keys[0] == key).unwrap_or_else(|| {
            let about = &meta["pieces"][name.as_str()];
            let list = |v: &serde_json::Value| v.as_array().map(|a| a.iter().filter_map(|k| k.as_u64().map(|k| k as u8)).collect::<Vec<u8>>());
            let mut keys = list(&about["keys"]).unwrap_or_default();
            keys.retain(|k| *k != key);
            keys.insert(0, key);
            pieces.push(PieceInput {
                name: name.clone(),
                zh: about["zh"].as_str().unwrap_or_default().to_string(),
                keys,
                chokes: list(&about["chokes"]).unwrap_or_default(),
                hits: Vec::new(),
            });
            pieces.len() - 1
        });
        match velocity {
            Some(v) => pieces[i].hits.push((file.clone(), v)),
            None => unknown.push((i, file.clone())),
        }
    }
    // 没写力度的：同一件里按峰值从轻到重，匀着排在 0.3 … 0.95。
    for (i, piece) in pieces.iter_mut().enumerate() {
        let mut list: Vec<(f64, PathBuf)> = unknown
            .iter()
            .filter(|u| u.0 == i)
            .map(|u| Ok((read_stereo(&u.1, SR as u32)?.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs())), u.1.clone())))
            .collect::<Result<_, String>>()?;
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        let n = list.len();
        for (k, (_, file)) in list.into_iter().enumerate() {
            piece.hits.push((file, if n == 1 { 0.9 } else { 0.3 + 0.65 * k as f64 / (n - 1) as f64 }));
        }
    }
    pieces.sort_by_key(|p| p.keys[0]);
    if pieces.iter().any(|p| p.keys[0] == 46) {
        for p in pieces.iter_mut().filter(|p| [42, 44].contains(&p.keys[0]) && p.chokes.is_empty()) {
            p.chokes = vec![46];
        }
    }
    Ok(pieces)
}

/// 一下的音色 → 低、中、高三个部分。`velocity`：各部分随力度怎么变（分了力度层的不用）；`seed`：噪声的随机数从第几路起。
/// 返回 (各部分, 模式数, 噪声层数)。
fn parts_of(timbre: &Timbre, velocity: &Velocity, seed: usize) -> (Vec<Part>, usize, usize) {
    let regions = [(20.0, SPLITS[0]), (SPLITS[0], SPLITS[1]), (SPLITS[1], 20_000.0)];
    let mut parts = Vec::new();
    let (mut modes_n, mut layers_n) = (0, 0);
    // 这一下里最响的东西：比它低 48 dB 以下的不要。
    let heard = |b: &Band| {
        let i = EDGES.iter().position(|e| *e == b.lo).unwrap_or(0);
        ((b.fast.powi(2) + b.slow.powi(2)) * leakage()[i][i]).sqrt()
    };
    let loudest = timbre.modes.iter().map(|m| m.amp * FRAC_1_SQRT_2).chain(timbre.bands.iter().map(heard)).fold(0.0, f64::max);
    let gate = loudest * 10f64.powf(-48.0 / 20.0);
    for (r, &(lo, hi)) in regions.iter().enumerate() {
        let mut modes: Vec<&Mode> = timbre.modes.iter().filter(|m| m.hz >= lo && m.hz < hi && m.amp * FRAC_1_SQRT_2 > gate).collect();
        modes.sort_by(|a, b| b.amp.total_cmp(&a.amp));
        modes.truncate(PART_MODES);
        modes.sort_by(|a, b| a.hz.total_cmp(&b.hz));
        let bands: Vec<&Band> = timbre.bands.iter().filter(|b| (b.lo * b.hi).sqrt() >= lo && (b.lo * b.hi).sqrt() < hi && heard(b) > gate).collect();
        if modes.is_empty() && bands.is_empty() {
            continue;
        }
        // 这个部分里最响的模式是 1（没有模式就用最响的噪声层）；引擎里声像居中时每声道是它的 1/√2，再乘 0.25。
        let unit = modes.iter().map(|m| m.amp).fold(0.0, f64::max).max(if modes.is_empty() {
            bands.iter().map(|b| (b.fast.powi(2) + b.slow.powi(2)).sqrt()).fold(0.0, f64::max)
        } else {
            0.0
        });
        // 模式的频率比都不超过 1：引擎按"比基频高几个八度"给高处的模式加亮，这里量到多少就是多少。
        let f0 = modes.iter().map(|m| m.hz).fold(0.0, f64::max);
        let f0 = if f0 > 0.0 { f0 } else { (lo.max(100.0) * hi.min(16_000.0)).sqrt() };
        let glide = timbre.glide.filter(|_| r == 0 && modes.iter().any(|m| m.glides));
        let swell = {
            let late: Vec<f64> = bands.iter().filter(|b| b.lo >= 1_600.0).map(|b| b.peak_at).collect();
            if late.is_empty() {
                0.0
            } else {
                median(late)
            }
        };
        modes_n += modes.len();
        layers_n += bands.len();
        parts.push(Part {
            about: String::new(),
            velocity: velocity.clone(),
            f0: round(f0, 5),
            gain: round(4.0 * 2f64.sqrt() * unit, 4),
            pan: timbre.pan,
            seed: seed + r,
            steep: false,
            modes: modes.iter().map(|m| [round(m.hz / f0, 5), round(m.amp / unit, 3), round(m.t60, 3)]).collect(),
            phases: if timbre.width[r] > 0.0 { Vec::new() } else { modes.iter().map(|m| round((m.phase + PI).rem_euclid(TAU) - PI, 4)).collect() },
            glide,
            noise: None,
            layers: bands
                .iter()
                .map(|b| Layer {
                    lo: b.lo,
                    hi: b.hi,
                    fast: round(b.fast / unit, 3),
                    fast_s: round(b.fast_s, 3),
                    slow: round(b.slow / unit, 3),
                    slow_s: round(b.slow_s, 3),
                    pan: 0.0,
                    steep: true,
                    sharp: true,
                })
                .collect(),
            width: timbre.width[r],
            mode_vel: 1.4,
            // 高处的噪声是敲下去之后才涌到最响的（吊镲）：共振和噪声一起慢慢进来。
            mode_attack: if swell >= 0.03 { round((swell / 2.5).min(0.25), 2) } else { 0.0 },
            swell_above: 0.0,
            drive: 0.0,
        });
    }
    (parts, modes_n, layers_n)
}

/// 最多量几个力度层（最轻、最重、中间的一个）。
const STROKES: usize = 3;

/// 量一件。`index` 是它在这一套里排第几（噪声的随机数各用各的）。
/// 录了几种力度的：最轻、最重和中间的一下各量各的音色，做成力度层（轻敲的鼓皮没绷那么紧，音高、泛音、鼓槌那一下都不一样，
/// 不是重击变小了）；只有一种力度的：就这一种音色，音量照力度的 1.4 次方。
fn piece(input: &PieceInput, index: usize) -> Result<(Piece, PieceReport), String> {
    if input.hits.is_empty() {
        return Err(format!("piece '{}' has no recordings", input.name));
    }
    let mut hits: Vec<(f64, [Vec<f64>; 2])> = input.hits.iter().map(|(p, v)| Ok((*v, read_hit(p)?))).collect::<Result<_, String>>()?;
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    // 同一个力度录了几下的（round robin）只用第一下。
    hits.dedup_by(|b, a| (a.0 - b.0).abs() < 1e-3);
    let mut chosen: Vec<usize> = vec![0, hits.len() - 1];
    if hits.len() >= STROKES {
        let middle = 0.5 * (hits[0].0 + hits[hits.len() - 1].0);
        chosen.insert(1, (1..hits.len() - 1).min_by(|&a, &b| (hits[a].0 - middle).abs().total_cmp(&(hits[b].0 - middle).abs())).unwrap_or(1));
    }
    chosen.dedup();
    let (mut parts, mut velocities) = (Vec::new(), Vec::new());
    let (mut modes_n, mut layers_n, mut parts_n, mut loudest) = (0, 0, 0, None);
    for (k, &h) in chosen.iter().enumerate() {
        let (at, lr) = &hits[h];
        let timbre = analyze(lr);
        let velocity = if chosen.len() > 1 { Velocity::default() } else { Velocity { power: Some(1.4), at: Some(round(*at, 3)), ..Default::default() } };
        let (p, m, l) = parts_of(&timbre, &velocity, index * 3 * STROKES + k * 3);
        if p.is_empty() {
            return Err(format!("piece '{}': nothing measurable in its recording at velocity {at:.2}", input.name));
        }
        (modes_n, layers_n, parts_n) = (m, l, p.len());
        loudest = Some(timbre);
        if chosen.len() > 1 {
            velocities.push(Stroke { at: round(*at, 3), parts: p });
        } else {
            parts = p;
        }
    }
    let loudest = loudest.expect("至少一下");
    // 每个力度的录音有多响（头 0.6 秒，两个声道）：层与层之间的音量照它走。
    let levels: Vec<[f64; 2]> = if chosen.len() > 1 {
        hits.iter()
            .map(|(v, lr)| {
                let m = lr[0].len().min((0.6 * SR) as usize);
                [round(*v, 3), (10.0 * ((mean_square(&lr[0][..m]) + mean_square(&lr[1][..m])) / 2.0).max(1e-20).log10() * 10.0).round() / 10.0]
            })
            .collect()
    } else {
        Vec::new()
    };
    let report = PieceReport {
        name: input.name.clone(),
        key: input.keys[0],
        hits: input.hits.len(),
        velocities: chosen.len(),
        modes: modes_n,
        layers: layers_n,
        parts: parts_n,
        glide: loudest.glide.map(|g| -g.semitones),
        seconds: loudest.seconds,
    };
    Ok((
        Piece {
            name: input.name.clone(),
            zh: input.zh.clone(),
            keys: input.keys.clone(),
            about: String::new(),
            chokes: input.chokes.clone(),
            eq: Vec::new(),
            parts,
            velocities,
            levels,
        },
        report,
    ))
}

/// 量一套。`fallback`：这一套没有的键交给哪套内置鼓组（不给就不响）。
pub fn extract(name: &str, inputs: &[PieceInput], fallback: Option<&str>, mut on_piece: impl FnMut(&PieceReport)) -> Result<KitFile, String> {
    if inputs.is_empty() {
        return Err("no pieces: give a folder of single hits named <piece>_v<velocity>.wav (kick_v100.wav, snare_v60.wav ...)".into());
    }
    let mut pieces = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let (p, report) = piece(input, i)?;
        on_piece(&report);
        pieces.push(p);
    }
    Ok(KitFile {
        format: FORMAT.into(),
        version: VERSION,
        name: name.into(),
        level_db: 0.0,
        compress: None::<Compress>,
        fallback: fallback.map(|f| Fallback { instrument: f.into(), velocity: 0.8, params: Default::default() }),
        pieces,
    })
}

/// 用 muser 的一个乐器（`sfz:` 鼓组、插件、内置鼓组）把每个键、每种力度各敲一下，存成 `<键>_v<力度>.wav`；不出声的键跳过。
pub fn render_hits(
    spec: &str,
    params: &muser_core::InstrumentParams,
    keys: &[u8],
    velocities: &[f64],
    seconds: f64,
    dir: &Path,
) -> Result<Vec<(u8, f64, PathBuf)>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    let gap = seconds + 1.0;
    let hits: Vec<(u8, f64)> = keys.iter().flat_map(|&k| velocities.iter().map(move |&v| (k, v))).collect();
    let mut project = muser_core::Project::new("hits", 60.0);
    let mut track = muser_core::Track::new("t", spec);
    track.instrument_params = params.clone();
    // 居中的音轨两声道各 -3 dB（等功率声像）：补回来。
    track.gain_db = 20.0 * core::f64::consts::SQRT_2.log10();
    let mut clip = muser_core::Clip::new(0.0, hits.len() as f64 * gap);
    for (i, &(k, v)) in hits.iter().enumerate() {
        let mut n = muser_core::Note::new(k, 0.25 + i as f64 * gap, 0.25);
        n.velocity = v as f32;
        clip.notes.push(n);
    }
    track.clips.push(clip);
    project.tracks.push(track);
    let audio = crate::render::render(&project, crate::render::RenderSettings { tail_seconds: 0.5, ..Default::default() }).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (i, &(k, v)) in hits.iter().enumerate() {
        let start = ((0.2 + i as f64 * gap) * SR) as usize;
        let end = (start + (seconds * SR) as usize).min(audio.num_frames());
        if start >= end || (0..2).all(|c| audio.channel(c)[start..end].iter().all(|s| s.abs() < 1e-4)) {
            continue;
        }
        let mut b = manguebyte::prelude::AudioBuffer::<f32>::new(2, end - start);
        for c in 0..2 {
            b.channel_mut(c).copy_from_slice(&audio.channel(c)[start..end]);
        }
        let path = dir.join(format!("{k}_v{v:.2}.wav"));
        manguebyte::prelude::wav::write_file(&path, &b, 48_000, manguebyte::prelude::WavFormat::Float32)
            .map_err(|e| format!("cannot write '{}': {e:?}", path.display()))?;
        out.push((k, v, path));
    }
    Ok(out)
}

/// 一下的对比结果（意思同 [`crate::modal::Comparison`]）。
#[derive(Debug, Clone, Serialize)]
pub struct HitComparison {
    pub source: String,
    pub piece: String,
    pub key: u8,
    pub velocity: f64,
    pub spectral_db: f64,
    pub level_db: f64,
    pub decay_ref: [f64; 4],
    pub decay_ours: [f64; 4],
    pub stereo_ref: f64,
    pub stereo_ours: f64,
}

/// 把每个录音在这一套上用同样的键、力度敲一遍，比较。
pub fn compare(kit_path: &str, hits: &[(u8, f64, PathBuf)]) -> Result<Vec<HitComparison>, String> {
    let kit = crate::kit::load(kit_path)?;
    let spec = format!("yinse:{}", kit.path.display());
    let mut out = Vec::new();
    let dir = std::env::temp_dir().join(format!("muser-yinse-kit-compare-{}", std::process::id()));
    for (key, velocity, file) in hits {
        let Some(i) = kit.file.pieces.iter().position(|p| p.keys.contains(key)) else { continue };
        let reference = read_stereo(file, SR as u32)?;
        let seconds = (reference[0].len() as f64 / SR).clamp(1.0, 8.0);
        let ours = render_hits(&spec, &Default::default(), &[*key], &[*velocity], seconds, &dir)?;
        let Some((_, _, path)) = ours.first() else { continue };
        let ours = read_stereo(path, SR as u32)?;
        let ((ga, da, sa, la), (gb, db, sb, lb)) = (features(&reference, SR), features(&ours, SR));
        let cells: Vec<f64> = ga.iter().zip(&gb).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())).collect();
        out.push(HitComparison {
            source: file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            piece: kit.file.pieces[i].name.clone(),
            key: *key,
            velocity: *velocity,
            spectral_db: cells.iter().sum::<f64>() / cells.len().max(1) as f64,
            // 这一套输出端的校准增益不算（它是为了和别的乐器一样响，不是这一下量得准不准）。
            level_db: 20.0 * (lb / la).log10() - kit.file.level_db,
            decay_ref: da,
            decay_ours: db,
            stereo_ref: sa,
            stereo_ours: sb,
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}

/// 这一套在参考乐句（底鼓 / 军鼓 / 闭镲）上多响（LUFS）；没有这三件的量不出来。
pub fn reference_lufs(kit_path: &Path) -> Option<f64> {
    let project = crate::levels::phrase(&format!("yinse:{}", kit_path.display()), &Default::default(), (35, 81), true);
    let l = crate::render::render_loudness(
        &project,
        crate::render::RenderSettings { tail_seconds: 1.0, ..Default::default() },
        &mut crate::render::RenderCache::new(),
    )
    .ok()?;
    l.first()?.output.lufs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成一下已知的声音：两个模式（一个低的在滑）、一段高频噪声，量回来应该差不多。
    fn synth(velocity: f64) -> [Vec<f64>; 2] {
        let n = (1.5 * SR) as usize;
        let glide = Glide { semitones: -2.0, seconds: 0.03, from: 1.0, velocity: 0.0 };
        let (clock, straight) = (warped_time(n, Some(glide)), warped_time(n, None));
        let mut rng = 12345u64;
        let mut noise = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        let hiss: Vec<f64> = (0..n).map(|i| noise() * (-(i as f64 / SR) / 0.05).exp()).collect();
        let hiss = band_pass(&hiss, 5_000.0, 7_500.0);
        // 低处随力度变得少、高处变得多。
        let (low, high) = (velocity.powf(0.5), velocity.powf(3.0));
        let x: Vec<f64> = (0..n)
            .map(|i| {
                low * (mode_wave(0.5, 0.6, 80.0, 0.4, i, &clock) + mode_wave(0.2, 0.3, 190.0, -1.0, i, &clock))
                    + high * (0.3 * hiss[i] + mode_wave(0.05, 0.2, 3_000.0, 0.0, i, &straight))
            })
            .collect();
        [x.clone(), x]
    }

    #[test]
    fn file_names_give_piece_key_and_velocity() {
        assert_eq!(hit_name("kick_v100"), ("kick".into(), None, Some(100.0 / 127.0)));
        assert_eq!(hit_name("36_v0.95"), (String::new(), Some(36), Some(0.95)));
        assert_eq!(hit_name("49_da-luo_ff_rr2"), ("da_luo".into(), Some(49), Some(0.95)));
        assert_eq!(hit_name("open_hat"), ("open_hat".into(), None, None));
        assert_eq!(key_name(38), "snare");
        assert_eq!(key_name(90), "key90");
        assert_eq!(key_name(58), "vibraslap");
    }

    #[test]
    fn a_synthetic_hit_is_measured_back() {
        let t = analyze(&synth(1.0));
        let near = |hz: f64| t.modes.iter().find(|m| (m.hz - hz).abs() < 0.03 * hz).unwrap_or_else(|| panic!("no mode near {hz} Hz"));
        let low = near(80.0);
        assert!((low.amp / 0.5 - 1.0).abs() < 0.2 && (low.t60 / 0.6 - 1.0).abs() < 0.25, "80 Hz: amp {} t60 {}", low.amp, low.t60);
        assert!((near(190.0).amp / 0.2 - 1.0).abs() < 0.25);
        let g = t.glide.expect("the low mode glides");
        assert!((g.semitones + 2.0).abs() < 0.8 && (g.seconds / 0.03 - 1.0).abs() < 0.6, "glide {} semitones in {} s", g.semitones, g.seconds);
        assert!((near(3_000.0).amp / 0.05 - 1.0).abs() < 0.3);
        // 高频的噪声落在 5 … 7.5 kHz 那一层，几十毫秒。
        let band = t.bands.iter().find(|b| b.lo == 5_000.0).unwrap();
        let level = (band.fast.powi(2) + band.slow.powi(2)).sqrt();
        assert!(
            t.bands.iter().filter(|b| b.lo >= 1_600.0).all(|b| (b.fast.powi(2) + b.slow.powi(2)).sqrt() <= level * 1.01),
            "the hiss is the loudest band up there"
        );
        assert!(t.width == [0.0; 3] && t.pan.abs() < 0.02);
    }

    #[test]
    fn the_extracted_kit_plays_like_the_recordings_at_every_velocity() {
        let dir = std::env::temp_dir().join(format!("muser-kit-extract-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut hits = Vec::new();
        for v in [0.35, 0.65, 0.95] {
            let lr = synth(v);
            let mut b = manguebyte::prelude::AudioBuffer::<f32>::new(2, lr[0].len());
            for (c, x) in lr.iter().enumerate() {
                b.channel_mut(c).iter_mut().zip(x).for_each(|(y, x)| *y = *x as f32);
            }
            let path = dir.join(format!("36_v{v:.2}.wav"));
            manguebyte::prelude::wav::write_file(&path, &b, 48_000, manguebyte::prelude::WavFormat::Float32).unwrap();
            hits.push((path, v));
        }
        let input = PieceInput { name: "kick".into(), zh: String::new(), keys: vec![36], chokes: vec![], hits: hits.clone() };
        let kit = extract("test", &[input], None, |_| {}).unwrap();
        // 三种力度各是一层；低的部分轻敲只轻几 dB，高的部分轻很多（各层自己量的音量）。
        let strokes = &kit.pieces[0].velocities;
        assert_eq!(strokes.iter().map(|s| s.at).collect::<Vec<_>>(), vec![0.35, 0.65, 0.95]);
        let db = |soft: &Part, loud: &Part| 20.0 * (soft.gain / loud.gain).log10();
        let (low, high) = (db(&strokes[0].parts[0], &strokes[2].parts[0]), db(strokes[0].parts.last().unwrap(), strokes[2].parts.last().unwrap()));
        assert!(low > -7.0 && high < -20.0, "low {low} dB, high {high} dB");
        let file = dir.join("kit").join("test.yinse.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::to_string(&kit).unwrap()).unwrap();
        let list: Vec<(u8, f64, PathBuf)> = hits.into_iter().map(|(p, v)| (36, v, p)).collect();
        for c in compare(&file.to_string_lossy(), &list).unwrap() {
            assert!(c.spectral_db < 4.0 && c.level_db.abs() < 2.0, "{}: spectral {:.1} dB, level {:+.1} dB", c.source, c.spectral_db, c.level_db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
