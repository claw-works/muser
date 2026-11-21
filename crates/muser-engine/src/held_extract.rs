//! 从长音的录音里量出一件拖长音的乐器（`muser yinse extract --held`）：每个音高、几种力度各一个长音（两三秒以上，一个文件一个音）。
//!
//! 每个长音取稳定的那一段量：
//! 1. 基频（谱上各次泛音加起来最响的那个频率）；
//! 2. 各次泛音的振幅（逐帧解调，取均方根）和相对基频的相位（波形是尖是圆）；
//! 3. 泛音之间的噪声（气声、弓毛的摩擦）：谱上相邻泛音正中间的那些格子，分频带；
//! 4. 稳定段的起伏：泛音的音量、基频各浮动多少。
//!
//! 录音带着起音和收音的，再量起音多快、起音那一下比稳定以后响多少、松开以后多快收掉。

use std::f64::consts::{PI, TAU};
use std::path::{Path, PathBuf};

use manguebyte::dsp::fft::{Complex, RealFft};
use serde::Serialize;

use crate::held::{HeldLayer, HeldNote, HeldTable, Wobble, FORMAT, MAX_PARTIALS, VERSION};
use crate::modal::{features, median, next_pow2, onset_of, read_stereo};

const SR: f64 = 48_000.0;
/// 噪声的频带（Hz）。
const EDGES: [f64; 11] = [40.0, 100.0, 200.0, 400.0, 800.0, 1_600.0, 3_200.0, 5_000.0, 7_500.0, 11_000.0, 16_000.0];

/// 一个长音的录音：音高（MIDI）、力度（0 … 1）、文件。
#[derive(Debug, Clone)]
pub struct HeldInput {
    pub pitch: f64,
    pub at: f64,
    pub file: PathBuf,
}

/// 量完一个长音报告什么。
#[derive(Debug, Clone, Serialize)]
pub struct HeldReport {
    pub source: String,
    pub pitch: f64,
    pub at: f64,
    pub hz: f64,
    pub partials: usize,
    /// 噪声比泛音低多少 dB。
    pub noise_db: f64,
    /// 基频浮动多少音分。
    pub cents: f64,
    /// 量的那一段稳定的声音有几秒。
    pub seconds: f64,
}

/// 提取时的选择。
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// 能同时发几个音（风琴、合奏）；不给就是独奏乐器，连着的音是连奏。
    pub poly: bool,
    /// 连奏时音高滑过去的时间常数（秒）；不给用默认的（管乐，几毫秒）。
    pub glide_s: Option<f64>,
    /// 这件乐器平常的揉音：深度（音分）、速度（Hz）、延迟（秒）；不给就用录音里量到的（大半的录音都在揉才算），都没有就不揉。
    pub vibrato: Option<(f64, f64, f64)>,
    /// 合奏（一个声部几个人）：每个音由这么多人各拉各的。
    pub ensemble: Option<usize>,
    /// 一下一下的奏法（跳弓、断奏）的录音：音色还是从长音量，起音、收音和每一下响多久从这些短的量。
    pub strokes: Vec<PathBuf>,
    /// 录音是震音（弓飞快地来回拉）：量出一秒几弓、谷底多深、多不匀，这件乐器的每个音都这样拉。
    pub tremolo: bool,
}

struct Analysis {
    hz: f64,
    amps: Vec<f64>,
    phases: Vec<f64>,
    noise: Vec<f64>,
    /// 贴着泛音的那一圈噪声是泛音振幅的多少（见 [`HeldLayer::air`]）。
    air: f64,
    /// 噪声跟着音高一下一下出多少、在哪个相位、一阵一阵出多少（见 [`noise_shape`]）。
    shape: (f64, f64, f64),
    /// 录音里的揉音：深度（音分）、每秒几下。
    vibrato: Option<(f64, f64)>,
    /// 头 24 次泛音跟着揉音变多少：每个四个数（cos φ、sin φ、cos 2φ、sin 2φ 的系数，dB），是在揉 `sway_depth` 音分深的时候量的（各帧配出来的揉音深度的中位数）。
    sway: Vec<f64>,
    sway_depth: f64,
    /// 头几次泛音的音量浮动多少（比例）、多快（Hz）。
    fluct: f64,
    fluct_hz: f64,
    /// 揉着的时候另外的、比揉音快的闪动（比例）。录音里的这一份有一半上下是屋子带来的，演奏时自己加的混响里还会再有：乐器里只记一半。
    fluct_fast: f64,
    cents: f64,
    seconds: f64,
    /// 起音：时间常数（秒）、比稳定以后响多少 dB。
    attack: Option<(f64, f64)>,
    /// 起音的重音多久退掉（时间常数，秒），退之前先保持多久。
    accent_len: Option<f64>,
    accent_hold: f64,
    /// 起音时高次泛音晚到多少（每个八度几个起音时间）、重音在高处多突出多少（dB / 八度）。
    late: f64,
    tilt: f64,
    /// 头 0.6 秒各次泛音比稳定以后多出多少 dB（响度的变化不算）、多久退掉（秒）。
    early: Vec<f64>,
    early_s: f64,
    /// 头两秒的响度（dB，相对稳定段）。
    onset: Vec<f64>,
    /// 起音那一下泛音之间多出来的东西：各个频带最多的时候的振幅、各个时刻是最多时的几成。
    puff: Vec<f64>,
    puff_shape: Vec<f64>,
    /// 立体声的录音：头几十次泛音在左声道上是两个声道均方根的几倍、右声道的相位比左声道多转多少。
    left: Vec<f64>,
    turn: Vec<f64>,
    /// 收音的时间常数（秒）。
    release: Option<f64>,
}

/// 引擎的第 i 个频带的噪声（均方根 1）落在第 j 个量的频带（谱上直上直下地切）里多少能量：`[j][i]`。
/// 引擎的滤波器边上是斜的，一个频带的噪声会漏到旁边去。
fn leakage() -> &'static Vec<Vec<f64>> {
    static M: std::sync::OnceLock<Vec<Vec<f64>>> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let size = 1 << 16;
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let white: Vec<f64> = (0..size + 8_000)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                ((rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * 3f64.sqrt()
            })
            .collect();
        let mut fft = RealFft::<f64>::new(size);
        let mut out = vec![Complex::ZERO; fft.spectrum_len()];
        let bin = SR / size as f64;
        let mut m = vec![vec![0.0; EDGES.len() - 1]; EDGES.len() - 1];
        for (i, e) in EDGES.windows(2).enumerate() {
            let (coef, unit, mut state) = (crate::held::band_filter(e[0], e[1], SR), crate::held::band_unit(e[0], e[1], SR), [[0.0; 2]; 4]);
            let y: Vec<f64> = white.iter().map(|&w| unit * coef.iter().zip(&mut state).fold(w, |y, (c, z)| crate::percussion::biquad(c, z, y))).collect();
            fft.forward(&y[8_000..], &mut out);
            let total: f64 = out.iter().map(|c| c.norm_sqr()).sum::<f64>().max(1e-30);
            for (j, q) in EDGES.windows(2).enumerate() {
                m[j][i] = out[(q[0] / bin) as usize..(q[1] / bin) as usize].iter().map(|c| c.norm_sqr()).sum::<f64>() / total;
            }
        }
        m
    })
}

/// 各频带量到的噪声能量 → 引擎各频带该给多少能量（解掉互相漏的）。
/// 不直接解方程：真录音里相邻频带差得多的时候，直接解出来是一高一低地跳的（还会出负数）。
/// 用乘法迭代（Richardson–Lucy），每一步都是正的，从量到的数出发慢慢修；只走十步（走多了会把相邻的频带一个挤空、一个堆满）。
fn unleak(measured: &[f64]) -> Vec<f64> {
    let m = leakage();
    let n = measured.len();
    let mut x = measured.to_vec();
    for _ in 0..10 {
        let got: Vec<f64> = (0..n).map(|j| (0..n).map(|i| m[j][i] * x[i]).sum::<f64>()).collect();
        for i in 0..n {
            let (mut num, mut den) = (0.0, 0.0);
            for j in 0..n {
                if got[j] > 0.0 {
                    num += m[j][i] * measured[j] / got[j];
                }
                den += m[j][i];
            }
            if den > 0.0 {
                x[i] *= num / den;
            }
        }
    }
    x
}

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos()).collect()
}

/// 基频：`hint` 附近 ±3% 里各次泛音（头 15 次）在谱上加起来最响的频率。
fn fundamental(s: &[f64], hint: f64) -> f64 {
    let n = s.len().min(1 << 18);
    let size = 1 << 19;
    let w = hann(n);
    let mut input = vec![0.0; size];
    input.iter_mut().zip(s.iter().zip(&w)).for_each(|(y, (x, w))| *y = x * w);
    let mut fft = RealFft::<f64>::new(size);
    let mut out = vec![Complex::ZERO; fft.spectrum_len()];
    fft.forward(&input, &mut out);
    let mag: Vec<f64> = out.iter().map(|c| c.norm_sqr().sqrt()).collect();
    let bin = SR / size as f64;
    let score = |f: f64| (1..=15).map(|h| f * f64::from(h)).filter(|hz| *hz < 0.5 * SR - 200.0).map(|hz| mag[(hz / bin).round() as usize]).sum::<f64>();
    let mut f0 = hint;
    for (span, step) in [(0.03, 0.0005), (0.002, 0.00005)] {
        let steps = (2.0 * span / step) as usize;
        f0 = (0..=steps).map(|k| f0 * (1.0 - span + k as f64 * step)).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(f0);
    }
    f0
}

/// 噪声是怎么出来的：跟着音高一下一下出（`pulse`：每个周期里它的大小起伏多少，0 … 1；`at`：最响的那一刻在基频的哪个相位上），
/// 还是一阵一阵出（`grain`：增益的对数的标准差）。吹气是匀的，弓擦弦是每个周期擦一下、还带着噼啪。
/// 做法：每一小段（50 ms）找出最能把上一个周期减干净的周期长度，减掉（有音高的部分没了，剩下噪声），
/// 取 1.5 … 6 kHz（三次插值在这以上不准），按周期里的位置叠起来看、按时间看。
fn noise_shape(s: &[f64], hz: f64) -> (f64, f64, f64) {
    let cubic = |t: f64| {
        let (i, f) = (t.floor() as isize, t - t.floor());
        let at = |k: isize| s[(i + k).clamp(0, s.len() as isize - 1) as usize];
        let (a, b, c, d) = (at(-1), at(0), at(1), at(2));
        b + 0.5 * f * (c - a + f * (2.0 * a - 5.0 * b + 4.0 * c - d + f * (3.0 * (b - c) + d - a)))
    };
    let period = SR / hz;
    let block = (0.05 * SR) as usize;
    let start = (period * 1.05) as usize + 2;
    if s.len() < start + 4 * block {
        return (0.0, 0.0, 0.0);
    }
    let coef = crate::held::band_filter(1_500.0, 6_000.0, SR);
    let mut state = [[0.0; 2]; 4];
    let (mut residual, mut phase) = (Vec::with_capacity(s.len()), Vec::with_capacity(s.len()));
    // 基频那一份（看它的相位）：和 e^{-iφ} 相乘加起来。
    let (mut cycles, mut re, mut im) = (0.0, 0.0, 0.0);
    for b in (start..s.len() - block).step_by(block) {
        let left = |t: f64| (b..b + block).step_by(3).map(|i| (s[i] - cubic(i as f64 - t)).powi(2)).sum::<f64>();
        let mut best = period;
        for (span, steps) in [(0.01, 20), (0.001, 20)] {
            best = (0..=steps).map(|k| best * (1.0 - span + 2.0 * span * k as f64 / steps as f64)).min_by(|x, y| left(*x).total_cmp(&left(*y))).unwrap_or(best);
        }
        for (i, x) in s.iter().enumerate().skip(b).take(block) {
            residual.push(coef.iter().zip(&mut state).fold(x - cubic(i as f64 - best), |y, (c, z)| crate::percussion::biquad(c, z, y)));
            cycles += 1.0 / best;
            phase.push(cycles);
            re += x * (TAU * cycles).cos();
            im -= x * (TAU * cycles).sin();
        }
    }
    let skip = (0.02 * SR) as usize;
    let (residual, phase) = (&residual[skip..], &phase[skip..]);
    // 一个周期分 16 格，各格里的能量。
    let (mut power, mut count) = ([0.0; 16], [0.0; 16]);
    for (y, c) in residual.iter().zip(phase) {
        let k = ((c.fract() * 16.0) as usize).min(15);
        power[k] += y * y;
        count[k] += 1.0;
    }
    let v: Vec<f64> = power.iter().zip(&count).map(|(p, n)| p / f64::max(*n, 1.0)).collect();
    let mean = v.iter().sum::<f64>() / 16.0;
    if mean <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let (c1, s1) = v.iter().enumerate().fold((0.0, 0.0), |(c, s), (k, p)| {
        let a = TAU * (k as f64 + 0.5) / 16.0;
        (c + p * a.cos(), s + p * a.sin())
    });
    let pulse = ((c1 * c1 + s1 * s1).sqrt() / (16.0 * mean)).min(0.9);
    // 录音的基频是 cos(φ + arg)，引擎的是 sin(ψ)：ψ = φ + arg + π/2；噪声最响在 φ = atan2(s1, c1)。
    let at = (s1.atan2(c1) + im.atan2(re) + PI / 2.0).rem_euclid(TAU);
    // 一阵一阵：每两个周期（至少 5 ms）一格的能量，起伏比匀的噪声该有的多出多少。
    let frame = ((2.0 * period) as usize).max((0.005 * SR) as usize);
    let frames: Vec<f64> = residual.chunks_exact(frame).map(|c| c.iter().map(|y| y * y).sum::<f64>() / frame as f64).collect();
    let m = frames.iter().sum::<f64>() / frames.len().max(1) as f64;
    let cv2 = frames.iter().map(|p| (p / m - 1.0).powi(2)).sum::<f64>() / frames.len().max(1) as f64;
    // 匀的噪声自己也有起伏：减掉上一个周期以后谱是一道一道的，独立的带宽只有两千多赫兹。量出来很小的当它是匀的。
    let excess = (cv2 - SR / (2_500.0 * frame as f64)).max(0.0);
    let grain = ((1.0 + excess).ln().sqrt() / 2.0).min(1.2);
    let grain = if grain < 0.12 { 0.0 } else { grain };
    (pulse, at, grain)
}

/// 把一个长音的音高拉直：跟着头几次泛音里最响的那个的相位，量出每一刻比 `hz` 快了多少，按它重新取样，
/// 出来的音高一直是 `hz`。返回拉直的声音、揉音（深度音分、每秒几下；没有规律的揉就是没有）、揉音以外音高还浮动多少音分。
fn straighten(s: &[f64], hz: f64) -> (Vec<f64>, Option<(f64, f64)>, f64, Vec<f64>) {
    let period = SR / hz;
    let (window, hop) = ((6.0 * period).round() as usize, (period.round() as usize).max((0.002 * SR) as usize));
    if s.len() < 8 * window {
        return (s.to_vec(), None, 0.0, Vec::new());
    }
    let w = hann(window);
    let starts: Vec<usize> = (0..).map(|k| k * hop).take_while(|st| st + window <= s.len()).collect();
    // 每帧各次泛音的复振幅（相对各自的 n·hz）。
    let demod = |n: usize, start: usize| {
        let omega = TAU * hz * n as f64 / SR;
        let (step, mut ph) = (Complex::<f64>::from_polar(1.0, -omega), Complex::<f64>::from_polar(1.0, -omega * start as f64));
        let (mut re, mut im) = (0.0, 0.0);
        for (v, w) in s[start..start + window].iter().zip(&w) {
            re += v * w * ph.re;
            im += v * w * ph.im;
            ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
        }
        Complex::new(re, im)
    };
    let n = (1..=3)
        .max_by(|a, b| {
            starts
                .iter()
                .step_by(8)
                .map(|st| demod(*a, *st).norm_sqr())
                .sum::<f64>()
                .total_cmp(&starts.iter().step_by(8).map(|st| demod(*b, *st).norm_sqr()).sum::<f64>())
        })
        .unwrap_or(1);
    // 比 hz 领先了多少个基频周期（相位解开、除以 n）。
    let mut lead = Vec::with_capacity(starts.len());
    let (mut last, mut turns) = (0.0, 0.0);
    for (k, st) in starts.iter().enumerate() {
        let z = demod(n, *st);
        let ph = z.im.atan2(z.re);
        if k > 0 {
            turns += ((ph - last + PI).rem_euclid(TAU) - PI) / TAU;
        }
        last = ph;
        lead.push(turns / n as f64);
    }
    // 每一帧比 hz 高多少音分。
    let cents: Vec<f64> = lead.windows(2).map(|l| 1200.0 * (1.0 + (l[1] - l[0]) * SR / (hop as f64 * hz)).max(1e-6).log2()).collect();
    // 重新取样：第 j 个输出取在 t + lead(t)·周期 = j 的那个 t 上。
    let center = |k: usize| (starts[k] + window / 2) as f64;
    let lead_at = |t: f64| {
        let x = ((t - center(0)) / hop as f64).clamp(0.0, (lead.len() - 1) as f64);
        let (i, f) = (x.floor() as usize, x - x.floor());
        (lead[i] + (lead[(i + 1).min(lead.len() - 1)] - lead[i]) * f - lead[0]) * period
    };
    // 加窗的 sinc 插值（两边各 12 点）：三次插值在高处会掉几 dB，量出来的泛音就暗了。
    let cubic = |t: f64| {
        let (i, f) = (t.floor() as isize, t - t.floor());
        if f < 1e-9 {
            return s[i.clamp(0, s.len() as isize - 1) as usize];
        }
        let (mut sum, mut norm) = (0.0, 0.0);
        for k in -11..=12isize {
            let x = k as f64 - f;
            let w = (PI * x).sin() / (PI * x) * (0.5 + 0.5 * (PI * x / 12.5).cos());
            sum += w * s[(i + k).clamp(0, s.len() as isize - 1) as usize];
            norm += w;
        }
        sum / norm
    };
    let total = (s.len() as f64 + lead_at(s.len() as f64)).floor().max(0.0) as usize;
    let out: Vec<f64> = (0..total)
        .map(|j| {
            let mut t = j as f64;
            for _ in 0..4 {
                t = j as f64 - lead_at(t);
            }
            cubic(t.clamp(0.0, (s.len() - 1) as f64))
        })
        .collect();
    // 揉音：音分的轨迹（去掉平均和斜着的那一条）里 3.5 … 9 Hz 上最强的那个正弦；够深、占了大半才算。
    let (count, mid) = (cents.len() as f64, (cents.len() as f64 - 1.0) / 2.0);
    let mean = cents.iter().sum::<f64>() / count;
    let slope = cents.iter().enumerate().map(|(i, c)| (i as f64 - mid) * (c - mean)).sum::<f64>()
        / cents.iter().enumerate().map(|(i, _)| (i as f64 - mid).powi(2)).sum::<f64>().max(1e-30);
    let flat: Vec<f64> = cents.iter().enumerate().map(|(i, c)| c - mean - slope * (i as f64 - mid)).collect();
    // 三帧平均一下（单帧的相位抖动不算音高）。
    let smooth: Vec<f64> = (0..flat.len())
        .map(|i| flat[i.saturating_sub(1)..(i + 2).min(flat.len())].iter().sum::<f64>() / (i + 2).min(flat.len()).saturating_sub(i.saturating_sub(1)) as f64)
        .collect();
    // 个别帧跟丢了（基频很弱的时候）会跳出几百音分：超过中位绝对偏差 6 倍的压到那儿。
    let spread = median(smooth.iter().map(|c| c.abs()).collect()).max(0.01);
    let smooth: Vec<f64> = smooth.iter().map(|c| c.clamp(-6.0 * spread, 6.0 * spread)).collect();
    let variance = smooth.iter().map(|c| c * c).sum::<f64>() / count;
    let rate = SR / hop as f64;
    // 揉音的深浅、快慢在一个音里会变：每 1.2 秒一段（重叠一半）各配一个正弦，取各段的中位数；
    // 一半以上的段里这个正弦占了那一段起伏的三成以上、深度够 4 音分，才算在揉。
    let span = (1.2 * rate) as usize;
    let mut fits = Vec::new();
    for start in (0..smooth.len().saturating_sub(span)).step_by((span / 2).max(1)) {
        let piece = &smooth[start..start + span];
        let mean = piece.iter().sum::<f64>() / span as f64;
        let power = piece.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / span as f64;
        let (depth, speed) = (35..=90)
            .map(|k| {
                let f = k as f64 / 10.0;
                let (re, im) = piece.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, c)| {
                    let a = TAU * f * i as f64 / rate;
                    (re + (c - mean) * a.cos(), im + (c - mean) * a.sin())
                });
                (2.0 * (re * re + im * im).sqrt() / span as f64, f)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or((0.0, 5.0));
        fits.push((depth, speed, depth * depth / 2.0 >= 0.3 * power));
    }
    let clear = fits.iter().filter(|f| f.2 && f.0 >= 4.0).count();
    let vibrato =
        (!fits.is_empty() && 2 * clear >= fits.len()).then(|| (median(fits.iter().map(|f| f.0).collect()), median(fits.iter().map(|f| f.1).collect())));
    // 揉音以外音高还浮动多少：揉着的录音只算比一个揉音周期慢的那部分（揉音自己深浅、快慢的变化不算走音）。
    let rest = match vibrato {
        Some((_, speed)) => {
            let half = ((rate / speed) / 2.0).round().max(1.0) as usize;
            let slow: Vec<f64> = (0..smooth.len())
                .map(|i| {
                    let (a, b) = (i.saturating_sub(half), (i + half + 1).min(smooth.len()));
                    smooth[a..b].iter().sum::<f64>() / (b - a) as f64
                })
                .collect();
            (slow.iter().map(|c| c * c).sum::<f64>() / count).sqrt()
        }
        None => variance.sqrt(),
    };
    // 拉直以后的每 64 个采样上，原来的音高偏了多少音分（去掉了平均和斜线；量各次泛音跟着揉音变多少用）。
    let pitch: Vec<f64> = (0..total.div_ceil(64))
        .map(|i| {
            let j = (i * 64) as f64;
            let mut t = j;
            for _ in 0..4 {
                t = j - lead_at(t);
            }
            let x = ((t - center(0)) / hop as f64 - 0.5).clamp(0.0, (smooth.len() - 1) as f64);
            let (a, f) = (x.floor() as usize, x - x.floor());
            smooth[a] + (smooth[(a + 1).min(smooth.len() - 1)] - smooth[a]) * f
        })
        .collect();
    (out, vibrato, rest, pitch)
}

/// 一个长音 → 泛音、噪声、起伏、起音和收音。
/// 一个震音的录音：一秒几弓、换弓时掉多少（引擎的 `depth`）、相邻两弓间隔的起伏、各弓轻重的起伏。
fn strokes_of(x: &[f64]) -> Option<(f64, f64, f64, f64)> {
    let from = (0.4 * SR) as usize;
    let s = x.get(from..(from + (3.5 * SR) as usize).min(x.len()))?;
    if s.len() < (1.5 * SR) as usize {
        return None;
    }
    // 包络：绝对值过两级 45 Hz 的低通，每毫秒一个点。
    let k = 1.0 - (-TAU * 45.0 / SR).exp();
    let (mut a, mut b) = (0.0, 0.0);
    let env: Vec<f64> = s
        .iter()
        .map(|v| {
            a += (v.abs() - a) * k;
            b += (a - b) * k;
            b
        })
        .step_by(48)
        .skip(100)
        .collect();
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    // 一弓：包络从下往上穿过它自己 0.3 秒的平均（掉回九成以下才算下一弓）。
    let slow = |i: usize| {
        let (lo, hi) = (i.saturating_sub(150), (i + 150).min(env.len()));
        env[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
    };
    let (mut ups, mut armed) = (Vec::new(), true);
    for i in 1..env.len() {
        let m = slow(i);
        if armed && env[i - 1] < m && env[i] >= m {
            ups.push(i);
            armed = false;
        } else if env[i] < 0.9 * m {
            armed = true;
        }
    }
    if ups.len() < 8 {
        return None;
    }
    let gaps: Vec<f64> = ups.windows(2).map(|w| (w[1] - w[0]) as f64 / 1000.0).collect();
    let typical = median(gaps.clone());
    let kept: Vec<f64> = gaps.iter().copied().filter(|g| (0.4 * typical..2.5 * typical).contains(g)).collect();
    let spread = |v: &[f64]| {
        let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt() / m.max(1e-30)
    };
    let peaks: Vec<f64> = ups.windows(2).map(|w| env[w[0]..w[1]].iter().copied().fold(0.0, f64::max)).collect();
    let mut sorted = env.clone();
    sorted.sort_by(|x, y| x.total_cmp(y));
    let floor = (sorted[sorted.len() / 20] / mean).clamp(0.0, 1.0);
    Some((1.0 / typical, (1.0 - floor) / (1.0 - 0.3 * floor), spread(&kept), spread(&peaks)))
}

/// 这个长音揉不揉（深度音分、每秒几下）：只做 [`analyze`] 开头找稳定段、拉直的那几步。
fn vibrato_of(x: &[f64], hint: f64) -> Option<(f64, f64)> {
    let hop = (0.005 * SR) as usize;
    let env: Vec<f64> = x.chunks(hop).map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt()).collect();
    let loud = median(env.iter().copied().filter(|e| *e > 0.05 * env.iter().copied().fold(0.0, f64::max)).collect());
    let (first, last) = (env.iter().position(|e| *e >= 0.7 * loud).unwrap_or(0), env.iter().rposition(|e| *e >= 0.7 * loud).unwrap_or(0));
    let (from, to) = (first * hop + (0.25 * SR) as usize, (last * hop).saturating_sub((0.15 * SR) as usize));
    if to < from + (0.8 * SR) as usize {
        return None;
    }
    let s = &x[from..to.min(from + (4.0 * SR) as usize)];
    straighten(s, fundamental(s, hint)).1
}

/// 揉音是怎么起来的：`x` 从起音开始，`hz` 是它的音高，`speed` 是揉音每秒几下。
/// 返回（起音后头一个揉音周期里已经揉到稳定时的几成，起音后多少秒揉到九成）。
fn vibrato_begins(x: &[f64], hz: f64, speed: f64) -> Option<(f64, f64)> {
    let s = &x[..x.len().min((1.8 * SR) as usize)];
    let pitch = straighten(s, hz).3;
    let step = 64.0 / SR;
    let span = ((1.0 / speed) / step).round() as usize;
    if span < 4 || pitch.len() < 6 * span {
        return None;
    }
    // 每个揉音周期长的一段里音高摆多宽（每四分之一个周期挪一下）。
    let swing = |from: usize| {
        let p = &pitch[from..from + span];
        p.iter().copied().fold(f64::MIN, f64::max) - p.iter().copied().fold(f64::MAX, f64::min)
    };
    // 头 40 ms 音还没吹稳，音高不作数。
    let first = (0.04 / step) as usize;
    let widths: Vec<(f64, f64)> = (first..pitch.len() - span).step_by((span / 4).max(1)).map(|i| ((i + span / 2) as f64 * step, swing(i))).collect();
    let steady = median(widths.iter().filter(|w| w.0 >= 0.8).map(|w| w.1).collect());
    if steady <= 0.0 || widths.is_empty() {
        return None;
    }
    let full = widths.iter().find(|w| w.1 >= 0.9 * steady).map_or(0.8, |w| w.0);
    Some(((widths[0].1 / steady).min(1.0), full))
}

/// 立体声的录音里，各次泛音到右声道比到左声道相位多转了多少：两个声道在同样的时刻、按同样的频率各解调一遍，
/// 逐帧的"右 × 左的共轭"加起来取辐角。（两个声道各分析各的再拿相位相减不行：各自挑的稳定段、各自拉直的音高都不一样，减出来是乱的。）
fn turn_of(left: &[f64], right: &[f64], hz: f64, count: usize) -> Vec<f64> {
    let len = left.len().min(right.len());
    let (from, to) = ((0.3 * SR) as usize, len.saturating_sub((0.2 * SR) as usize).min((3.3 * SR) as usize));
    let window = (12.0 * SR / hz).round() as usize;
    if to < from + 2 * window {
        return vec![0.0; count];
    }
    let w = hann(window);
    let starts: Vec<usize> = (from..to - window).step_by((window / 2).max(1)).collect();
    (1..=count)
        .map(|n| {
            let omega = TAU * hz * n as f64 / SR;
            let demod = |x: &[f64], start: usize| {
                let (step, mut ph) = (Complex::<f64>::from_polar(1.0, -omega), Complex::<f64>::from_polar(1.0, -omega * start as f64));
                let (mut re, mut im) = (0.0, 0.0);
                for (v, w) in x[start..start + window].iter().zip(&w) {
                    re += v * w * ph.re;
                    im += v * w * ph.im;
                    ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
                }
                Complex::new(re, im)
            };
            let (mut re, mut im) = (0.0, 0.0);
            for &start in &starts {
                let (l, r) = (demod(left, start), demod(right, start));
                re += r.re * l.re + r.im * l.im;
                im += r.im * l.re - r.re * l.im;
            }
            im.atan2(re)
        })
        .collect()
}

/// `assume`：这件乐器的录音大半在揉（深度、速度）——个别没认出来的也按揉着的录音对待（噪声怎么量不一样）。
/// `bows`：录音是震音。
fn analyze(x: &[f64], hint: f64, assume: Option<(f64, f64)>, bows: bool) -> Result<Analysis, String> {
    // 包络（5 ms 一个点）：找稳定的那一段。
    let hop = (0.005 * SR) as usize;
    let env: Vec<f64> = x.chunks(hop).map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt()).collect();
    let loud = median(env.iter().copied().filter(|e| *e > 0.05 * env.iter().copied().fold(0.0, f64::max)).collect());
    let (first, last) = (env.iter().position(|e| *e >= 0.7 * loud).unwrap_or(0), env.iter().rposition(|e| *e >= 0.7 * loud).unwrap_or(0));
    let (from, to) = (first * hop + (0.25 * SR) as usize, (last * hop).saturating_sub((0.15 * SR) as usize));
    if to < from + (0.8 * SR) as usize {
        return Err("less than 0.8 s of steady sound: hold every note 2 s or more".into());
    }
    // 很长的录音（十几秒）后面常常慢慢变轻、变暗：只量头四秒，平常演奏的音没那么长。
    let s = &x[from..to.min(from + (4.0 * SR) as usize)];
    // 揉着的、音高在漂的录音先拉直（不然高次泛音跟不上，量出来偏暗），揉了多少另外记下。
    let rough = fundamental(s, hint);
    let (straight, vibrato, drift, pitch) = straighten(s, rough);
    let vibrato = vibrato.or(assume);
    let raw = s;
    let s = &straight[..];
    let steady = median(env[from / hop..(to.min(from + (4.0 * SR) as usize)) / hop].to_vec());
    let hz = fundamental(s, hint);
    let count = (((0.5 * SR - 500.0) / hz) as usize).clamp(1, MAX_PARTIALS);
    // 逐帧解调：窗长 12 个周期，每半个窗一帧。
    let window = ((12.0 * SR / hz).round() as usize).min(s.len() / 2);
    let w = hann(window);
    let norm: f64 = w.iter().sum();
    let frames = ((s.len() - window) / (window / 2).max(1) + 1).max(2);
    let starts: Vec<usize> = (0..frames).map(|k| k * (s.len() - window) / (frames - 1)).collect();
    let mut level = vec![vec![0.0; frames]; count];
    let mut relative = vec![Complex::<f64>::ZERO; count];
    for (k, &start) in starts.iter().enumerate() {
        let mut unit = Complex::new(1.0, 0.0);
        for n in 0..count {
            // 以整段的开头为时间零点（相邻两帧的相位差才是频率的偏差）。
            let omega = TAU * hz * (n + 1) as f64 / SR;
            let (step, mut ph) = (Complex::<f64>::from_polar(1.0, -omega), Complex::<f64>::from_polar(1.0, -omega * start as f64));
            let (mut re, mut im) = (0.0, 0.0);
            for (v, w) in s[start..start + window].iter().zip(&w) {
                let a = v * w;
                re += a * ph.re;
                im += a * ph.im;
                ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
            }
            let z = Complex::new(2.0 * re / norm, 2.0 * im / norm);
            level[n][k] = z.norm_sqr().sqrt();

            if n == 0 {
                let m = level[0][k].max(1e-30);
                unit = Complex::new(z.re / m, -z.im / m);
            }
            // 相对基频的相位：z_n · conj(u)^(n)，u 是基频的方向。
            let mut turn = Complex::new(1.0, 0.0);
            for _ in 0..=n {
                turn = Complex::new(turn.re * unit.re - turn.im * unit.im, turn.re * unit.im + turn.im * unit.re);
            }
            relative[n] = Complex::new(relative[n].re + z.re * turn.re - z.im * turn.im, relative[n].im + z.re * turn.im + z.im * turn.re);
        }
    }
    let mut amps: Vec<f64> = level.iter().map(|l| (l.iter().map(|v| v * v).sum::<f64>() / frames as f64).sqrt()).collect();
    // 引擎用 sin(nθ + φ)、基频是 0：解调是按 cos 量的，换过去是 φ + π/2 − n·π/2。
    let phases: Vec<f64> =
        relative.iter().enumerate().map(|(n, z)| (z.im.atan2(z.re) + PI / 2.0 - (n + 1) as f64 * PI / 2.0 + PI).rem_euclid(TAU) - PI).collect();
    // 先减掉一条直线：一个长音慢慢变弱、变强不算起伏。
    let spread = |l: &[f64]| {
        let (n, m) = (l.len() as f64, l.iter().sum::<f64>() / l.len() as f64);
        let mid = (n - 1.0) / 2.0;
        let slope = l.iter().enumerate().map(|(i, v)| (i as f64 - mid) * (v - m)).sum::<f64>()
            / l.iter().enumerate().map(|(i, _)| (i as f64 - mid).powi(2)).sum::<f64>().max(1e-30);
        (l.iter().enumerate().map(|(i, v)| (v - m - slope * (i as f64 - mid)).powi(2)).sum::<f64>() / n).sqrt() / m.max(1e-30)
    };
    // 揉音带着音色变：琴身的共鸣是固定在频率上的，泛音跟着揉音上下扫过去，有的变响有的变轻（每个泛音各不一样，揉一下能差好几 dB，
    // 扫过一个共鸣峰的还会一个来回响两次）。先定出每一帧揉到哪儿了（音高 = 深度 · sin φ：前后各大半个周期里配一个正弦），
    // 再量每个泛音的音量（dB）跟着 φ 怎么变：cos φ、sin φ、cos 2φ、sin 2φ 四个系数。这一份从起伏里拿掉。
    let cents_at: Vec<f64> = starts.iter().map(|st| pitch.get((st + window / 2) / 64).copied().unwrap_or(0.0)).collect();
    let mut sway = Vec::new();
    let mut sway_depth = 0.0;
    if let Some((_, rate)) = vibrato {
        let omega = TAU * rate;
        let time: Vec<f64> = starts.iter().map(|st| (st + window / 2) as f64 / SR).collect();
        let reach = 0.6 / rate;
        let turn: Vec<(f64, f64)> = (0..frames)
            .map(|k| {
                let near: Vec<usize> = (0..frames).filter(|j| (time[*j] - time[k]).abs() <= reach).collect();
                let mean = near.iter().map(|j| cents_at[*j]).sum::<f64>() / near.len() as f64;
                let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for &j in &near {
                    let (sn, cs, y) = ((omega * time[j]).sin(), (omega * time[j]).cos(), cents_at[j] - mean);
                    ss += sn * sn;
                    cc += cs * cs;
                    sc += sn * cs;
                    ys += y * sn;
                    yc += y * cs;
                }
                let det = ss * cc - sc * sc;
                if near.len() < 5 || det.abs() < 1e-9 {
                    return (0.0, 0.0);
                }
                let (a, b) = ((ys * cc - yc * sc) / det, (yc * ss - ys * sc) / det);
                ((a * a + b * b).sqrt(), b.atan2(a) + omega * time[k])
            })
            .collect();
        sway_depth = median(turn.iter().map(|t| t.0).collect());
        let used: Vec<usize> = (0..frames).filter(|k| turn[*k].0 >= 0.5 * sway_depth).collect();
        if sway_depth >= 5.0 && used.len() >= 12 {
            #[allow(clippy::needless_range_loop)] // 同一个泛音的序号用在好几个数组上
            for n in 0..count.min(24) {
                let db: Vec<f64> = level[n].iter().map(|v| 20.0 * v.max(1e-12).log10()).collect();
                let mean = used.iter().map(|k| db[*k]).sum::<f64>() / used.len() as f64;
                let basis = |k: usize| [turn[k].1.cos(), turn[k].1.sin(), (2.0 * turn[k].1).cos(), (2.0 * turn[k].1).sin()];
                let mut m = [[0.0; 5]; 4];
                for &k in &used {
                    let b = basis(k);
                    for r in 0..4 {
                        for c in 0..4 {
                            m[r][c] += b[r] * b[c];
                        }
                        m[r][4] += b[r] * (db[k] - mean);
                    }
                }
                for c in 0..4 {
                    let p = (c..4).max_by(|x, y| m[*x][c].abs().total_cmp(&m[*y][c].abs())).unwrap_or(c);
                    m.swap(c, p);
                    if m[c][c].abs() < 1e-9 {
                        continue;
                    }
                    for r in 0..4 {
                        if r != c {
                            let f = m[r][c] / m[c][c];
                            let pivot = m[c];
                            m[r].iter_mut().zip(&pivot).for_each(|(x, p)| *x -= f * p);
                        }
                    }
                }
                let coef: Vec<f64> = (0..4).map(|r| if m[r][r].abs() < 1e-9 { 0.0 } else { (m[r][4] / m[r][r]).clamp(-8.0, 8.0) }).collect();
                for k in 0..frames {
                    let b = basis(k);
                    let scale = (turn[k].0 / sway_depth).min(2.0);
                    let part = scale * (coef[0] * b[0] + coef[1] * b[1]) + scale * scale * (coef[2] * b[2] + coef[3] * b[3]);
                    level[n][k] /= 10f64.powf(part / 20.0);
                }
                sway.extend(coef);
            }
        }
    }
    // 音量的浮动：最响的几个泛音的（弱的那些量到的是噪声）。
    let mut loudest: Vec<usize> = (0..count).collect();
    loudest.sort_by(|a, b| amps[*b].total_cmp(&amps[*a]));
    // 揉着的录音：比一个揉音周期慢的算平常的起伏，快的那些（屋子里的反射声和直达声互相拍出来的）另记，演奏时只在揉的时候有。
    let per_period = vibrato.map_or(1, |v| ((SR / v.1) / ((s.len() - window) as f64 / (frames - 1) as f64)).round().max(1.0) as usize);
    let slow = |l: &[f64]| -> Vec<f64> {
        (0..l.len())
            .map(|i| {
                let (a, b) = (i.saturating_sub(per_period / 2), (i + per_period / 2 + 1).min(l.len()));
                l[a..b].iter().sum::<f64>() / (b - a) as f64
            })
            .collect()
    };
    let mean_of = |l: &[f64]| l.iter().sum::<f64>() / l.len().max(1) as f64;
    let strong: Vec<usize> = loudest.iter().take(6).filter(|n| amps[**n] > 0.1 * amps[loudest[0]]).copied().collect();
    // 慢的起伏里再去掉一秒以上的走势（一个长音慢慢推上去、收下来是表情，不是抖）。
    let second = ((SR / ((s.len() - window) as f64 / (frames - 1) as f64)).round() as usize).max(3);
    let trendless = |l: &[f64]| -> Vec<f64> {
        let fine = slow(l);
        (0..l.len())
            .map(|i| {
                let (a, b) = (i.saturating_sub(second / 2), (i + second / 2 + 1).min(l.len()));
                let mean = fine[a..b].iter().sum::<f64>() / (b - a) as f64;
                mean_of(&fine) * fine[i] / mean.max(1e-30)
            })
            .collect()
    };
    let fluct = median(strong.iter().map(|n| spread(&trendless(&level[*n]))).collect());
    let fluct_fast = if vibrato.is_some() {
        median(strong.iter().map(|n| (spread(&level[*n]).powi(2) - spread(&slow(&level[*n])).powi(2)).max(0.0).sqrt()).collect())
    } else {
        0.0
    };
    // 噪声分两样。一样是泛音之间平平的那一层：有泛音的那一段（半个基频以上、最高的泛音以下）取相邻泛音正中间
    // 那些格子的中位数 × 带宽；基频以下、最高的泛音以上也是中位数 × 带宽。
    // 另一样贴着每个泛音（真的管乐器气流不稳，泛音两边十几到一百多赫兹有一圈按距离的平方掉下去的噪声）：见下面的 `air`。
    let size = next_pow2(s.len() / 2 + 1).clamp(1 << 14, 1 << 17).min(next_pow2(s.len()) / 2);
    let w = hann(size);
    let scale = 2.0 / (SR * w.iter().map(|v| v * v).sum::<f64>());
    let mut fft = RealFft::<f64>::new(size);
    let (mut input, mut out) = (vec![0.0; size], vec![Complex::ZERO; fft.spectrum_len()]);
    let mut psd = vec![0.0; fft.spectrum_len()];
    let segments: Vec<usize> = (0..).map(|k| k * size / 2).take_while(|st| st + size <= s.len()).collect();
    for &st in &segments {
        input.iter_mut().zip(s[st..st + size].iter().zip(&w)).for_each(|(y, (x, w))| *y = x * w);
        fft.forward(&input, &mut out);
        psd.iter_mut().zip(&out).for_each(|(p, c)| *p += c.norm_sqr() * scale / segments.len() as f64);
    }
    // 揉着的录音：拉直以后，屋子里的余响（它的音高是早先的，和直达声不一样）被拉成了泛音之间的一片，不是噪声。
    // 没拉直的谱里泛音之间反倒干净（低次泛音揉得不宽）：两个谱里取小的那个。
    if vibrato.is_some() {
        let mut other = vec![0.0; fft.spectrum_len()];
        let pieces: Vec<usize> = (0..).map(|k| k * size / 2).take_while(|st| st + size <= raw.len()).collect();
        for &st in &pieces {
            input.iter_mut().zip(raw[st..st + size].iter().zip(&w)).for_each(|(y, (x, w))| *y = x * w);
            fft.forward(&input, &mut out);
            other.iter_mut().zip(&out).for_each(|(p, c)| *p += c.norm_sqr() * scale / pieces.len() as f64);
        }
        if !pieces.is_empty() {
            psd.iter_mut().zip(&other).for_each(|(p, o)| *p = p.min(*o));
        }
    }
    let bin = SR / size as f64;
    let (bottom, top) = (0.5 * hz, (count as f64 + 0.5) * hz);
    // 都取中位数：录音里泛音之间、基频以下常有别的有音高的东西（低八度的"咕噜"、交流电的嗡声），那不是气声，
    // 算进去的话吹出来是一大股气。几段平均出来的谱，中位数比平均小一点，补回来。
    let unbias = 1.0 / (1.0 - 2.0 / (9.0 * segments.len().max(1) as f64)).powi(3);
    let sum = |lo: f64, hi: f64| {
        let (a, b) = ((lo / bin) as usize, ((hi / bin) as usize).min(psd.len()));
        if b > a + 8 {
            median(psd[a..b].to_vec()) * unbias * (hi - lo)
        } else if b > a {
            psd[a..b].iter().sum::<f64>() * bin
        } else {
            0.0
        }
    };
    // 贴着泛音的那一圈：头几次泛音（9 kHz 以下）两边 10 Hz 以外、0.3 个基频（至多 120 Hz）以内的谱叠起来，减掉正中间的那层底，
    // 配一条 A / (1 + (Δ / AIR_HZ)²)（对数上取中位数）；它的总能量 A·π·AIR_HZ 比上这几次泛音振幅的平方和，开方。
    let harmonics = count.min((9_000.0 / hz) as usize).max(1);
    let at = |f: f64| psd.get((f / bin).round() as usize).copied().unwrap_or(0.0);
    let floor: f64 = (1..=harmonics)
        .map(|n| {
            let (lo, hi) = ((((n as f64 + 0.3) * hz) / bin) as usize, ((((n as f64 + 0.7) * hz) / bin) as usize).min(psd.len()));
            if hi > lo {
                median(psd[lo..hi].to_vec()) * unbias
            } else {
                0.0
            }
        })
        .sum();
    let offsets: Vec<f64> = (0..).map(|i| 10.0 + i as f64 * bin).take_while(|d| *d < (0.3 * hz).min(120.0)).collect();
    let around = |d: f64| (1..=harmonics).map(|n| (at(n as f64 * hz + d) + at(n as f64 * hz - d)) / 2.0).sum::<f64>();
    let near = offsets.iter().filter(|d| **d < 25.0).map(|d| around(*d)).sum::<f64>() / offsets.iter().filter(|d| **d < 25.0).count().max(1) as f64;
    // 慢的起伏有多快：泛音两边 1.5 … 25 Hz 的谱的重心。
    let slow: Vec<(f64, f64)> = (0..).map(|i| 1.5 + i as f64 * bin).take_while(|d| *d < 25.0).map(|d| (d, (around(d) - floor).max(0.0))).collect();
    let fluct_hz = slow.iter().map(|(d, p)| d * p).sum::<f64>() / slow.iter().map(|(_, p)| p).sum::<f64>().max(1e-30);
    let air = if offsets.len() > 8 && floor > 0.0 && near > 2.0 * floor {
        // 中位数：录音里偶尔有别的东西（嗡声、一下抖动）在某个距离上鼓一个包，不让它把整条线抬起来。
        // 每个倍频程一样重（格子是等距的，不加权的话全是远处的说了算，而听得见的是贴着泛音的那一段）。
        let mut points: Vec<(f64, f64)> =
            offsets.iter().map(|d| (((around(*d) - floor).max(0.05 * floor) * (1.0 + (d / crate::held::AIR_HZ).powi(2))).ln(), 1.0 / d)).collect();
        points.sort_by(|x, y| x.0.total_cmp(&y.0));
        let half = points.iter().map(|p| p.1).sum::<f64>() / 2.0;
        let mut seen = 0.0;
        let log_a = points
            .iter()
            .find(|p| {
                seen += p.1;
                seen >= half
            })
            .map_or(0.0, |p| p.0);
        let power: f64 = amps[..harmonics].iter().map(|a| a * a).sum();
        (log_a.exp() * PI * crate::held::AIR_HZ / power.max(1e-30)).sqrt().min(0.3)
    } else {
        0.0
    };
    // 起音：从稳定音量的一成到九成用多久；头 150 ms 里最响的比稳定以后响多少。
    let t10 = env.iter().position(|e| *e >= 0.1 * steady);
    let t90 = env.iter().position(|e| *e >= 0.9 * steady);
    let attack = match (t10, t90) {
        (Some(a), Some(b)) if b > a || a > 0 => {
            let peak = env[b..(b + 30).min(env.len())].iter().copied().fold(0.0, f64::max);
            Some((((b - a) as f64 * 0.005).clamp(0.005, 1.0), (20.0 * (peak / steady).log10()).clamp(0.0, 9.0)))
        }
        _ => None,
    };
    // 一个音刚起来的头半秒，音色常常和稳定以后不一样（弓刚咬住弦、气刚吹进去：某几次泛音特别突出，过一两秒才落回去）。
    // 短的音听到的全是这一段。量头 0.3 秒、之后 0.3 … 0.9 秒各次泛音的能量（它左右各半个基频里的），和稳定段的比：
    // 头 0.3 秒多出多少 dB（整体的响度变化不算，那归上面的响度曲线），多久退掉。
    // 头两秒的响度怎么走：包络（震音的按一弓多的长度抹平）在 [`crate::held::ONSET_AT`] 那些时刻上比稳定段的均方根高多少 dB。
    let onset: Vec<f64> = {
        let level = (s.iter().map(|v| v * v).sum::<f64>() / s.len().max(1) as f64).sqrt().max(1e-30);
        let half = if bows { 9 } else { 3 };
        crate::held::ONSET_AT
            .iter()
            .map(|t| {
                let i = (t / 0.005) as usize;
                // 抹平的窗不伸到起音以前（那儿是静的，会把头几个时刻拉低）。
                let reach = half.min(i);
                let (a, b) = (i - reach, (i + reach + 1).min(env.len()));
                if b > a {
                    20.0 * ((env[a..b].iter().map(|e| e * e).sum::<f64>() / (b - a) as f64).sqrt() / level).max(1e-6).log10()
                } else {
                    0.0
                }
            })
            .collect()
    };
    let (early, early_s) = {
        let energies = |seg: &[f64]| -> Vec<f64> {
            let size = next_pow2(seg.len()).max(1 << 12);
            let mut fft = RealFft::<f64>::new(size);
            let (mut input, mut out) = (vec![0.0; size], vec![Complex::ZERO; fft.spectrum_len()]);
            let w = hann(seg.len());
            input.iter_mut().zip(seg.iter().zip(&w)).for_each(|(y, (x, w))| *y = x * w);
            fft.forward(&input, &mut out);
            let bin = SR / size as f64;
            (0..count.min(32))
                .map(|n| {
                    let (lo, hi) = ((((n as f64 + 0.5) * hz) / bin) as usize, ((((n as f64 + 1.5) * hz) / bin) as usize).min(out.len()));
                    out[lo..hi.max(lo)].iter().map(|c| c.norm_sqr()).sum::<f64>() / w.iter().map(|v| v * v).sum::<f64>().max(1e-30)
                })
                .collect()
        };
        let begin = first * hop;
        let piece =
            |a: f64, b: f64| x.get(begin + (a * SR) as usize..(begin + (b * SR) as usize).min(x.len())).filter(|p| p.len() as f64 >= 0.6 * (b - a) * SR);
        match (piece(0.04, 0.3), piece(0.3, 0.9)) {
            (Some(head), Some(next)) if raw.len() as f64 >= 1.5 * SR => {
                let (e0, e1, e2) = (energies(head), energies(next), energies(&raw[(0.5 * SR) as usize..]));
                let top = e2.iter().copied().fold(0.0, f64::max);
                let strong: Vec<usize> = (0..e2.len()).filter(|n| e2[*n] > top * 1e-5).collect();
                let offsets = |e: &[f64]| -> Vec<f64> {
                    let shift = 10.0 * (strong.iter().map(|n| e[*n]).sum::<f64>() / strong.iter().map(|n| e2[*n]).sum::<f64>().max(1e-30)).max(1e-30).log10();
                    (0..e2.len())
                        .map(|n| if strong.contains(&n) { (10.0 * (e[n] / e2[n].max(1e-30)).max(1e-30).log10() - shift).clamp(-12.0, 15.0) } else { 0.0 })
                        .collect()
                };
                let (d0, d1) = (offsets(&e0), offsets(&e1));
                let (s0, s1) = (d0.iter().map(|v| v.abs()).sum::<f64>(), d1.iter().map(|v| v.abs()).sum::<f64>());
                (d0, if s0 > s1 && s1 > 0.0 { (0.43 / (s0 / s1).ln()).clamp(0.1, 3.0) } else { 3.0 })
            }
            _ => (Vec::new(), 0.6),
        }
    };
    // 起音那一下：舌头一放、簧片刚振起来（弓刚咬住弦）的头一两百毫秒，泛音之间有很多稳定以后没有的东西——
    // 气声、簧片还没稳住时低八度的"咯"、弓毛的擦声，比稳定以后的噪声多一二十 dB，之后退掉。
    // 短的音听到的大半是这一段；没有它，音是"直接亮起来"的，像电子琴。
    // 从出声算起（`x` 就是从那儿开始的），在 [`crate::held::PUFF_AT`] 那些时刻各取一小段（十二个周期、至少 40 ms），
    // 量各个频带里泛音之间（离泛音三成基频以外）的能量，减掉 0.8 秒以后同样量出来的：
    // `puff` 是各个频带多出来的这一份的振幅（和 `noise` 一个单位），`puff_shape` 是它在各个时刻是最大时的几成。
    let (puff, puff_shape): (Vec<f64>, Vec<f64>) = {
        let span = ((12.0 * SR / hz) as usize).max((0.04 * SR) as usize);
        let size = next_pow2(span) * 4;
        let mut fft = RealFft::<f64>::new(size);
        let w = hann(span);
        let (bin, scale) = (SR / size as f64, 2.0 / (SR * w.iter().map(|v| v * v).sum::<f64>()));
        let mut between = |center: f64| -> Option<Vec<f64>> {
            let from = ((center * SR) as usize).saturating_sub(span / 2);
            let seg = x.get(from..from + span)?;
            // 音正在起来的时候，泛音自己的谱是抹宽的（一小段里音量变了好几倍），抹到泛音之间的那些不是噪声。
            // 先把有音高的部分减掉：每个采样减去一个周期以后的那个采样的 g 倍（g：这一段里前后两个周期的比，最小二乘），
            // 重复的东西就抵掉了；剩下的谱再除以这个梳子在各个频率上放大的倍数 |1 − g·e^{jωT}|²。
            let period = SR / hz;
            let later: Vec<f64> = (0..span)
                .map(|n| {
                    let t = (from + n) as f64 + period;
                    let (i, f) = (t.floor() as usize, t - t.floor());
                    match (x.get(i), x.get(i + 1)) {
                        (Some(a), Some(b)) => a * (1.0 - f) + b * f,
                        _ => 0.0,
                    }
                })
                .collect();
            let g = seg.iter().zip(&later).map(|(a, b)| a * b).sum::<f64>() / later.iter().map(|b| b * b).sum::<f64>().max(1e-30);
            let (mut input, mut out) = (vec![0.0; size], vec![Complex::ZERO; fft.spectrum_len()]);
            input.iter_mut().zip(seg.iter().zip(later.iter().zip(&w))).for_each(|(y, (x, (l, w)))| *y = (x - g * l) * w);
            fft.forward(&input, &mut out);
            Some(
                EDGES
                    .windows(2)
                    .map(|e| {
                        let bins: Vec<f64> = ((e[0] / bin) as usize..((e[1] / bin) as usize).min(out.len()))
                            .filter(|k| {
                                let r = *k as f64 * bin / hz;
                                (r - r.round()).abs() > 0.3
                            })
                            .map(|k| out[k].norm_sqr() * scale / (1.0 + g * g - 2.0 * g * (TAU * k as f64 * bin / hz).cos()))
                            .collect();
                        if bins.len() < 4 {
                            0.0
                        } else {
                            bins.iter().sum::<f64>() / bins.len() as f64 * (e[1] - e[0])
                        }
                    })
                    .collect(),
            )
        };
        let steady: Vec<Vec<f64>> = [0.8, 1.0, 1.2, 1.4].iter().filter_map(|t| between(*t)).collect();
        let bands = EDGES.len() - 1;
        if bows || steady.len() < 3 {
            (Vec::new(), Vec::new())
        } else {
            let floor: Vec<f64> = (0..bands).map(|b| median(steady.iter().map(|s| s[b]).collect())).collect();
            let more: Vec<Vec<f64>> = crate::held::PUFF_AT
                .iter()
                .map(|t| between(*t).map_or(vec![0.0; bands], |now| (0..bands).map(|b| (now[b] - floor[b]).max(0.0)).collect()))
                .collect();
            let peak: Vec<f64> = (0..bands).map(|b| more.iter().map(|m| m[b]).fold(0.0, f64::max)).collect();
            let total: f64 = peak.iter().sum();
            // 比稳定以后泛音之间的那点东西多不到一倍（3 dB）：不算有。
            if total < floor.iter().sum::<f64>() {
                (Vec::new(), Vec::new())
            } else {
                // 各个频带最多的时刻不一样，演奏时却都按同一条曲线走：每个频带的大小按它这一段里的总能量定（不按它自己的峰值，那样会多）。
                let shape: Vec<f64> = more.iter().map(|m| (m.iter().sum::<f64>() / total).sqrt().min(1.0)).collect();
                let area = shape.iter().map(|v| v * v).sum::<f64>().max(1e-30);
                // 演奏时每个频带的噪声会漏到隔壁的频带里：和稳定段的噪声一样，先解回去。
                let power: Vec<f64> = (0..bands).map(|b| more.iter().map(|m| m[b]).sum::<f64>() / area).collect();
                (unleak(&power).into_iter().map(f64::sqrt).collect(), shape)
            }
        }
    };
    // 起音时高处和低处谁先到、谁更突出：基频那一带（0.5 … 1.5 倍基频）和高处（3.5 … 12 倍基频）各自的包络。
    // `late`：高处从一成到九成用的时间是低处的几倍（管乐的高次泛音来得晚；弓一擦弦高处先到）→ 每高一个八度晚多少个起音时间；
    // `tilt`：起音那一下高处比低处多突出多少 dB（每个八度）——弓头那一下亮而短的擦弦声。
    let (late, tilt) = {
        let band = |lo: f64, hi: f64| -> Vec<f64> {
            let coef = crate::held::band_filter(lo, hi.min(0.45 * SR), SR);
            let mut state = [[0.0; 2]; 4];
            let y: Vec<f64> = x.iter().map(|v| coef.iter().zip(&mut state).fold(*v, |y, (c, z)| crate::percussion::biquad(c, z, y))).collect();
            let e: Vec<f64> = y.chunks(hop).map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt()).collect();
            let half = if bows { 6 } else { 1 };
            (0..e.len())
                .map(|i| {
                    let (a, b) = (i.saturating_sub(half), (i + half + 1).min(e.len()));
                    e[a..b].iter().sum::<f64>() / (b - a) as f64
                })
                .collect()
        };
        let measure = |e: &[f64]| -> Option<(f64, f64)> {
            let level = median(e[from / hop..(to.min(from + (4.0 * SR) as usize)) / hop].to_vec());
            let (a, b) = (e.iter().position(|v| *v >= 0.1 * level)?, e.iter().position(|v| *v >= 0.9 * level)?);
            let peak = e[b..(b + 50).min(e.len())].iter().copied().fold(0.0, f64::max);
            Some((((b.max(a) - a) as f64 + 0.5) * 0.005, 20.0 * (peak / level.max(1e-30)).log10()))
        };
        match (measure(&band(0.5 * hz, 1.5 * hz)), measure(&band(3.5 * hz, 12.0 * hz))) {
            (Some(low), Some(high)) => (((high.0 / low.0 - 1.0) / 3.0).clamp(0.0, 1.0), ((high.1 - low.1) / 3.0).clamp(-2.0, 4.0)),
            _ => (0.6, 0.0),
        }
    };
    // 起音的那一下重音多久退掉：从最响那一刻到落回稳定音量上面 1 dB 用了多久（按 dB 上指数地退：除以 ln(重音的 dB 数)）。
    let accent_len = t90.and_then(|b| {
        let top = (b..(b + 30).min(env.len())).max_by(|x, y| env[*x].total_cmp(&env[*y]))?;
        let db = 20.0 * (env[top] / steady).log10();
        if db < 1.5 {
            return None;
        }
        let back = env[top..(top + 120).min(env.len())].iter().position(|e| *e < 1.12 * steady)?;
        Some((back as f64 * 0.005 / db.ln()).clamp(0.03, 0.4))
    });
    // 震音的录音：包络一弓一弓地起落，上面那样量会把头一弓的谷当成重音退完了。把包络按一弓多的长度（120 ms）抹平了再量：
    // 头半秒里最响的比稳定以后响多少，多久落回来。
    let mut accent_hold = 0.0;
    let (attack, accent_len) = if bows {
        let smooth: Vec<f64> = (0..env.len())
            .map(|i| {
                let (a, b) = (i.saturating_sub(6), (i + 6).min(env.len()));
                env[a..b].iter().sum::<f64>() / (b - a) as f64
            })
            .collect();
        let level = median(smooth[from / hop..(to.min(from + (4.0 * SR) as usize)) / hop].to_vec());
        let top = (0..100.min(smooth.len())).max_by(|x, y| smooth[*x].total_cmp(&smooth[*y])).unwrap_or(0);
        let db = (20.0 * (smooth[top] / level).log10()).clamp(0.0, 12.0);
        // 头一弓是长长的一弓（响着不抖），之后才抖起来：它在最响的七成以上待了多久。
        let up = smooth[..=top].iter().position(|e| *e >= 0.7 * smooth[top]).unwrap_or(top);
        let down = top + smooth[top..].iter().position(|e| *e < 0.7 * smooth[top]).unwrap_or(0);
        let back = top + smooth[top..(top + 400).min(smooth.len())].iter().position(|e| *e < 1.12 * level).unwrap_or(40);
        if db >= 1.5 {
            accent_hold = ((down - up) as f64 * 0.005 - 0.03).clamp(0.0, 0.6);
        }
        (attack.map(|a| (a.0, db)), (db >= 1.5).then(|| (back.saturating_sub(down).max(4) as f64 * 0.005 / db.ln()).clamp(0.03, 1.0)))
    } else {
        (attack, accent_len)
    };
    // 收音：最后一次还在七成以上之后，多久掉到一成（20 dB）。
    let release = env[last..].iter().position(|e| *e < 0.1 * steady).filter(|n| *n < 200).map(|n| (n as f64 * 0.005 / 10f64.ln()).clamp(0.01, 1.0));
    // 揉着的录音里贴着泛音的那一圈量不准（上面说的余响也在里头，量出来总是顶到上限，演奏出来是一片沙沙声）：不要了，
    // 揉音带出来的泛音闪动另有一份（`sway` 和 `wobble.vibrato`）。
    // 震音的录音也一样：每换一弓泛音的相位就重来一次，谱上每条泛音都被抹宽了，泛音之间的那些是琴声自己，不是噪声
    //（当噪声放出来是一片沙沙声盖着很弱的泛音，像捂着弦拉，听不出一弓一弓的律动）。
    let air = if vibrato.is_some() || bows { 0.0 } else { air };
    if bows {
        // 泛音的振幅按它左右各半个基频里的全部能量算（抹宽了的都算回它自己）。
        for (n, a) in amps.iter_mut().enumerate() {
            let (lo, hi) = ((((n as f64 + 0.5) * hz) / bin) as usize, ((((n as f64 + 1.5) * hz) / bin) as usize).min(psd.len()));
            if hi > lo {
                *a = a.max((2.0 * psd[lo..hi].iter().sum::<f64>() * bin).sqrt());
            }
        }
    }
    // 震音的录音里泛音之间真正的噪声：取最低的那一成格子（中位数落在抹宽的泛音上）。
    let quiet = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 10]
    };
    // 平的那一层：贴着泛音的那一圈的尾巴伸到两个泛音正中间还有（拉弦乐器不稳的音上，正中间量到的大半是它），先减掉，
    // 不然引擎里这一份出两遍，多出来的那一遍是平平的"嘶"，听着像吹气。
    let skirt = |n: usize| if n >= 1 && n <= count.min(crate::held::AIR) { air * air * amps[n - 1] * amps[n - 1] / (PI * crate::held::AIR_HZ) } else { 0.0 };
    let tail = |f: f64| {
        let n = (f / hz).floor() as usize;
        let lorentz = |d: f64| 1.0 / (1.0 + (d / crate::held::AIR_HZ).powi(2));
        skirt(n) * lorentz(f - n as f64 * hz) + skirt(n + 1) * lorentz((n + 1) as f64 * hz - f)
    };
    let noise: Vec<f64> = EDGES
        .windows(2)
        .map(|e| {
            let (lo, hi) = (e[0].max(bottom), e[1].min(top));
            let between: Vec<f64> = if hi > lo {
                ((lo / bin) as usize..(hi / bin) as usize)
                    .filter(|k| (0.3..0.7).contains(&((*k as f64 * bin / hz) % 1.0)))
                    .map(|k| (psd[k] - tail(k as f64 * bin)).max(0.0))
                    .collect()
            } else {
                Vec::new()
            };
            let floor = if between.len() <= 8 {
                0.0
            } else if bows {
                quiet(between)
            } else {
                median(between) * unbias
            };
            let harmonic = floor * (hi - lo);
            harmonic + sum(e[0], e[1].min(bottom)) + sum(e[0].max(top), e[1])
        })
        .collect();
    let noise: Vec<f64> = unleak(&noise).into_iter().map(f64::sqrt).collect();
    let shape = noise_shape(s, hz);
    // 一阵一阵的程度是从"减掉上一个周期"剩下的东西量的，揉着的录音里剩下的大半是余响：不算。
    let shape = if vibrato.is_some() { (shape.0, shape.1, 0.0) } else { shape };
    // 音高的浮动：拉直以前量的（揉音不算在里面）。
    let cents = drift;
    Ok(Analysis {
        hz,
        amps,
        phases,
        noise,
        air,
        shape,
        vibrato,
        sway,
        sway_depth,
        fluct_fast,
        fluct,
        fluct_hz,
        cents,
        seconds: s.len() as f64 / SR,
        attack,
        accent_len,
        accent_hold,
        late,
        tilt,
        early,
        early_s,
        onset,
        puff,
        puff_shape,
        left: Vec::new(),
        turn: Vec::new(),
        release,
    })
}

/// 量到的起音（音量从一成到九成用了 `rise` 秒，起音时比稳定以后响 `accent_db`）→ 引擎的起音时间常数。
/// 引擎里基音的起音是 (1 − e^(−t/τ))² 再乘上那一下重音（`accent_len` 秒退掉）；重音会让它更早到九成，所以按引擎自己的样子反解 τ。
fn attack_constant(rise: f64, accent_db: f64, accent_len: f64) -> f64 {
    let rise_of = |tau: f64| {
        let g = |t: f64| (1.0 - (-t / tau).exp()).powi(2) * 10f64.powf(accent_db / 20.0 * (-t / accent_len).exp());
        let at = |level: f64| (1..40_000).map(|i| i as f64 * 0.000_05).find(|t| g(*t) >= level).unwrap_or(2.0);
        at(0.9) - at(0.1)
    };
    let (mut lo, mut hi) = (0.001_f64, 0.5_f64);
    for _ in 0..40 {
        let mid = (lo * hi).sqrt();
        if rise_of(mid) < rise {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo * hi).sqrt()
}

fn round(v: f64, digits: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return 0.0;
    }
    let scale = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

/// 量一件。同一个音高的几个录音是它的力度层。
pub fn extract(name: &str, inputs: &[HeldInput], options: Options, mut on_note: impl FnMut(&HeldReport)) -> Result<HeldTable, String> {
    if inputs.is_empty() {
        return Err("no recordings: give long notes, one per file, named with pitch and dynamic (A2_p.wav, A2_f.wav ...)".into());
    }
    let mut inputs = inputs.to_vec();
    inputs.sort_by(|a, b| a.pitch.total_cmp(&b.pitch).then(a.at.total_cmp(&b.at)));
    let mut notes: Vec<HeldNote> = Vec::new();
    let mut rates = Vec::new();
    let mut lens = Vec::new();
    let mut holds = Vec::new();
    let (mut lates, mut tilts) = (Vec::new(), Vec::new());
    let mut earlies = Vec::new();
    let mut onsets: Vec<Vec<f64>> = Vec::new();
    let mut quicks = Vec::new();
    let mut vibratos = Vec::new();
    let mut begins: Vec<(f64, f64)> = Vec::new();
    let (mut flucts, mut cents, mut attacks, mut accents, mut releases) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // 先看一遍：大半的录音都在揉，就当整件乐器的录音是揉着的。
    let mut seen = Vec::new();
    let mut bows = Vec::new();
    for input in &inputs {
        let lr = read_stereo(&input.file, SR as u32)?;
        let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
        let on = onset_of(&mid).saturating_sub((0.002 * SR) as usize);
        seen.extend(vibrato_of(&mid[on..], 440.0 * ((input.pitch - 69.0) / 12.0).exp2()));
        if options.tremolo {
            bows.extend(strokes_of(&mid[on..]));
        }
    }
    // 震音的录音音量一直在抖，认揉音的那一步会认错：不看。
    if options.tremolo {
        seen.clear();
    }
    let tremolo = (!bows.is_empty()).then(|| crate::held::Tremolo {
        rate_hz: round(median(bows.iter().map(|b| b.0).collect()).clamp(8.0, 15.0), 3),
        depth: round(median(bows.iter().map(|b| b.1).collect()), 2),
        // 间隔的起伏按量到的，但至多两成：再不匀就听不出一弓一弓的律动了（量的时候数漏、数多一弓也会把它抬高）。
        uneven: round(median(bows.iter().map(|b| b.2).collect()).min(0.2), 2),
        weight: round(median(bows.iter().map(|b| b.3).collect()), 2),
    });
    if options.tremolo && tremolo.is_none() {
        return Err("--tremolo: no bow strokes found in the recordings (the level should rise and fall ten times a second or so)".into());
    }
    let assume = (2 * seen.len() > inputs.len()).then(|| (median(seen.iter().map(|v| v.0).collect()), median(seen.iter().map(|v| v.1).collect())));
    for input in &inputs {
        let source = input.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let lr = read_stereo(&input.file, SR as u32)?;
        let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
        if mid.iter().fold(0.0f64, |m, v| m.max(v.abs())) < 1e-5 {
            return Err(format!("'{}' is silent", input.file.display()));
        }
        let on = onset_of(&mid).saturating_sub((0.002 * SR) as usize);
        let mut a = analyze(&mid[on..], 440.0 * ((input.pitch - 69.0) / 12.0).exp2(), assume, options.tremolo)
            .map_err(|e| format!("'{}': {e}", input.file.display()))?;
        // 立体声的录音：两支话筒离得开，同一个泛音到两边的相位不一样，加在一起有的抵掉、有的叠起来（差得出八九 dB），
        // 那不是这件乐器的音色。各次泛音、各带噪声的大小取两个声道各自量出来的均方根；别的（相位、起音、揉音）还是看加在一起的。
        let apart = lr[0].iter().zip(&lr[1]).skip(on).take((2.0 * SR) as usize).map(|(l, r)| (l - r).abs()).fold(0.0, f64::max) > 1e-4;
        if apart {
            let hint = a.hz;
            if let (Ok(left), Ok(right)) = (analyze(&lr[0][on..], hint, assume, options.tremolo), analyze(&lr[1][on..], hint, assume, options.tremolo)) {
                let both = |l: f64, r: f64| ((l * l + r * r) / 2.0).sqrt();
                for (n, v) in a.amps.iter_mut().enumerate() {
                    *v = both(left.amps.get(n).copied().unwrap_or(*v), right.amps.get(n).copied().unwrap_or(*v));
                }
                // 左右怎么分：记头 48 次（够响的那些）。
                let top = a.amps.iter().copied().fold(0.0, f64::max);
                let turns = turn_of(&lr[0][on..], &lr[1][on..], a.hz, a.amps.len().min(48));
                for n in 0..a.amps.len().min(48) {
                    let (l, r) = (left.amps.get(n).copied().unwrap_or(0.0), right.amps.get(n).copied().unwrap_or(0.0));
                    let strong = a.amps[n] > top * 1e-3 && l + r > 0.0;
                    a.left.push(if strong { l / both(l, r).max(1e-30) } else { 1.0 });
                    a.turn.push(if strong { turns.get(n).copied().unwrap_or(0.0) } else { 0.0 });
                }
                for (n, v) in a.noise.iter_mut().enumerate() {
                    *v = both(left.noise.get(n).copied().unwrap_or(*v), right.noise.get(n).copied().unwrap_or(*v));
                }
                if left.puff.len() == a.puff.len() && right.puff.len() == a.puff.len() {
                    for (n, v) in a.puff.iter_mut().enumerate() {
                        *v = both(left.puff[n], right.puff[n]);
                    }
                }
            }
        }
        let top = a.amps.iter().copied().fold(0.0, f64::max);
        let keep = a.amps.iter().rposition(|v| *v > top * 10f64.powf(-70.0 / 20.0)).map_or(1, |i| i + 1);
        let harmonic = (a.amps.iter().map(|v| v * v).sum::<f64>() / 2.0).sqrt();
        on_note(&HeldReport {
            source,
            pitch: input.pitch,
            at: input.at,
            hz: a.hz,
            partials: keep,
            noise_db: 20.0 * (a.noise.iter().map(|v| v * v).sum::<f64>().sqrt() / harmonic.max(1e-30)).max(1e-9).log10(),
            cents: a.cents,
            seconds: a.seconds,
        });
        flucts.push(a.fluct);
        if a.vibrato.is_some() {
            quicks.push(a.fluct_fast);
        }
        rates.push(a.fluct_hz);
        vibratos.extend(a.vibrato);
        if let Some((_, speed)) = a.vibrato {
            begins.extend(vibrato_begins(&mid[on..], a.hz, speed));
        }
        cents.push(a.cents);
        if let Some((seconds, db)) = a.attack {
            attacks.push(seconds);
            accents.push(db);
            lens.extend(a.accent_len);
            holds.push(a.accent_hold);
            lates.push(a.late);
            onsets.push(a.onset.clone());
            if !a.early.is_empty() {
                earlies.push(a.early_s);
            }
            tilts.push(a.tilt);
        }
        releases.extend(a.release);
        let layer = HeldLayer {
            at: round(input.at, 3),
            partials: a.amps[..keep].iter().map(|v| round(*v, 4)).collect(),
            phases: a.phases[..keep].iter().map(|v| (v * 1000.0).round() / 1000.0).collect(),
            noise: a.noise.iter().map(|v| round(*v, 3)).collect(),
            // 合奏里贴着泛音的那一圈是几个人音高不齐拍出来的（引擎里自己会有），不再另加。
            air: round(if options.ensemble.is_some_and(|n| n > 1) { a.air.min(0.03) } else { a.air }, 2),
            // 先原样记着（量的时候揉得多深也记着），整件乐器的揉音深度定下来以后再换算过去。
            sway: a.sway.clone(),
            vibrato: round(if a.sway_depth >= 3.0 { a.sway_depth } else { a.vibrato.map_or(0.0, |v| v.0) }, 2),
            left: a.left.iter().take(keep).map(|v| (v * 100.0).round() / 100.0).collect(),
            turn: a.turn.iter().take(keep).map(|v| (v * 100.0).round() / 100.0).collect(),
            early: if a.early.iter().all(|v| v.abs() < 1.0) { Vec::new() } else { a.early.iter().map(|v| (v * 10.0).round() / 10.0).collect() },
            puff: a.puff.iter().map(|v| round(*v, 2)).collect(),
            puff_shape: a.puff_shape.iter().map(|v| (v * 100.0).round() / 100.0).collect(),
            pulse: round(a.shape.0, 2),
            pulse_at: if a.shape.0 >= 0.005 { round(a.shape.1, 3) } else { 0.0 },
            grain: if options.tremolo { 0.0 } else { round(a.shape.2, 2) },
        };
        match notes.last_mut() {
            Some(n) if (n.pitch - input.pitch).abs() < 0.01 => {
                if n.layers.iter().any(|l| (l.at - layer.at).abs() < 1e-3) {
                    return Err(format!("'{}': pitch {} already has a recording at dynamic {}", input.file.display(), input.pitch, input.at));
                }
                n.layers.push(layer);
            }
            _ => notes.push(HeldNote { pitch: input.pitch, hz: round(a.hz, 6), layers: vec![layer] }),
        }
    }
    let or = |v: Vec<f64>, default: f64| if v.is_empty() { default } else { median(v) };
    let accent = or(accents, 0.0);
    let accent_len = or(lens, 0.06);
    // 一下一下的奏法：每个短录音的包络（5 ms 一个点）——从一成到九成用多久、从一成到最响那一刻多久、从最响掉 20 dB 多久。
    let mut stroke = None;
    if !options.strokes.is_empty() {
        let (mut rises, mut holds, mut falls) = (Vec::new(), Vec::new(), Vec::new());
        for file in &options.strokes {
            let lr = read_stereo(file, SR as u32)?;
            let hop = (0.005 * SR) as usize;
            let env: Vec<f64> = lr[0]
                .chunks(hop)
                .zip(lr[1].chunks(hop))
                .map(|(l, r)| (l.iter().zip(r).map(|(a, b)| ((a + b) / 2.0).powi(2)).sum::<f64>() / l.len() as f64).sqrt())
                .collect();
            let top = (0..env.len()).max_by(|a, b| env[*a].total_cmp(&env[*b])).unwrap_or(0);
            let peak = env.get(top).copied().unwrap_or(0.0);
            let (Some(t10), Some(t90)) = (env.iter().position(|e| *e >= 0.1 * peak), env.iter().position(|e| *e >= 0.9 * peak)) else { continue };
            let Some(fall) = env[top..].iter().position(|e| *e < 0.1 * peak) else { continue };
            rises.push(((t90 - t10) as f64 * 0.005).max(0.005));
            holds.push((top - t10) as f64 * 0.005);
            falls.push(fall as f64 * 0.005);
        }
        if rises.is_empty() {
            return Err("the stroke recordings have no clear attack and decay".into());
        }
        stroke = Some((attack_constant(median(rises), 0.0, 0.06), median(holds), median(falls) / 10f64.ln()));
    }
    // 起音那一下，个别录音里特别重（那一个音吹劈了、弓压重了）：是那一次的事，不是这件乐器的规律。
    // 每个音高看它比音本身大多少，比左右两个音高都大的，压到三个里中间的那个。
    let heavy: Vec<f64> = notes
        .iter()
        .map(|n| {
            n.layers.iter().map(|l| l.puff.iter().map(|v| v * v).sum::<f64>() / l.partials.iter().map(|v| v * v).sum::<f64>().max(1e-30)).fold(0.0, f64::max)
        })
        .collect();
    for (i, note) in notes.iter_mut().enumerate() {
        let around = median(heavy[i.saturating_sub(1)..(i + 2).min(heavy.len())].to_vec());
        if heavy[i] > around && heavy.len() >= 3 {
            let less = (around / heavy[i]).sqrt();
            note.layers.iter_mut().for_each(|l| l.puff.iter_mut().for_each(|v| *v = round(*v * less, 2)));
        }
    }
    // 录音里的揉音：大半的录音都在揉，就当这件乐器平常是揉的。
    let measured = (2 * vibratos.len() > flucts.len())
        .then(|| (round(median(vibratos.iter().map(|v| v.0).collect()), 2), round(median(vibratos.iter().map(|v| v.1).collect()), 2), 0.2));
    // 量出来的揉音是怎么起来的：起音后头一个周期已经揉到几成，多久揉到头。（另外给的揉音按默认的来：等一会儿，再慢慢加深。）
    let (begin, ramp) = match (options.vibrato, measured, begins.is_empty()) {
        (None, Some(_), false) => {
            let (share, full) = (median(begins.iter().map(|b| b.0).collect()), median(begins.iter().map(|b| b.1).collect()));
            if share >= 0.9 {
                (1.0, 0.05)
            } else {
                (round(share, 2), round(full.clamp(0.05, 1.0), 2))
            }
        }
        _ => (0.0, 0.3),
    };
    let measured = measured.map(|m| (m.0, m.1, if begin > 0.0 { 0.0 } else { m.2 }));
    let vibrato = options.vibrato.or(measured);
    // 各次泛音跟着揉音变的那一份是按各个录音自己的揉音深度量的（演奏时也按它自己的深度揉）；乐器不揉就都不要了。每个系数至多 4 dB。
    for layer in notes.iter_mut().flat_map(|n| n.layers.iter_mut()) {
        if vibrato.is_none() || stroke.is_some() || options.vibrato.is_some() {
            layer.sway.clear();
            layer.vibrato = 0.0;
            continue;
        }
        layer.sway = layer.sway.iter().map(|v| (v.clamp(-4.0, 4.0) * 100.0).round() / 100.0).collect();
        // 在 dB 上摆来摆去，平均下来的振幅比不摆的时候大：从泛音的振幅里除掉，摆着的时候平均还是量到的那么响。
        for (n, c) in layer.sway.chunks(4).enumerate() {
            let mean = (0..16)
                .map(|i| {
                    let phi = TAU * i as f64 / 16.0;
                    (0.115_129 * (c[0] * phi.cos() + c[1] * phi.sin() + c[2] * (2.0 * phi).cos() + c[3] * (2.0 * phi).sin())).exp()
                })
                .sum::<f64>()
                / 16.0;
            if let Some(p) = layer.partials.get_mut(n) {
                *p = round(*p / mean, 4);
            }
        }
    }
    // 合奏：几个人的音高互相差多少，就是量到的音高浮动（一个人的没这么大）；每个人自己的浮动按独奏的算。
    // 震音的录音里每换一弓相位就跳一下，音高跟不准（量出来几十音分）：按不抖的算。
    let drift = if options.tremolo { median(cents).clamp(0.1, 2.0) } else { median(cents).max(0.1) };
    let apart = if options.tremolo { 10.0 } else { drift };
    let ensemble = options.ensemble.filter(|n| *n > 1).map(|voices| crate::held::Ensemble { voices, cents: round(apart.clamp(3.0, 25.0), 2) });
    let drift = if ensemble.is_some() { drift.min(3.0) } else { drift };
    Ok(HeldTable {
        format: FORMAT.into(),
        version: VERSION,
        name: name.into(),
        level_db: 0.0,
        mono: !options.poly && ensemble.is_none(),
        attack_s: round(stroke.map_or_else(|| attack_constant(or(attacks.clone(), 0.05), accent, accent_len), |s| s.0), 2),
        attack_len_s: round(accent_len, 2),
        // 各个录音的中位数；引擎自己起音的那一下爬升（(1 − e^(−t/τ))²）已经有了，从曲线里除掉。整条都在 1 dB 以内就不记。
        onset: {
            let tau = stroke.map_or_else(|| attack_constant(or(attacks.clone(), 0.05), accent, accent_len), |s| s.0).max(0.002);
            let curve: Vec<f64> = crate::held::ONSET_AT
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let rise = 20.0 * (1.0 - (-t / tau).exp()).powi(2).max(1e-3).log10();
                    ((median(onsets.iter().map(|o| o[i]).collect()) - rise).clamp(-12.0, 18.0) * 10.0).round() / 10.0
                })
                .collect();
            if stroke.is_some() || onsets.is_empty() || curve.iter().all(|v| v.abs() < 1.0) {
                Vec::new()
            } else {
                curve
            }
        },
        early_s: if earlies.is_empty() { 0.6 } else { round(median(earlies), 2) },
        attack_late: if lates.is_empty() { 0.6 } else { round(median(lates), 2) },
        attack_tilt_db: if tilts.is_empty() { 0.0 } else { round(median(tilts), 2) },
        // 震音头一弓多长：响度曲线从头上那个高处落到一半以下的时刻（没有曲线就用包络上量的）。
        attack_hold_s: {
            let measured = if holds.is_empty() { 0.0 } else { median(holds.clone()) };
            let curve: Vec<f64> =
                if onsets.is_empty() { Vec::new() } else { (0..crate::held::ONSET_AT.len()).map(|i| median(onsets.iter().map(|o| o[i]).collect())).collect() };
            let peak = (0..curve.len().min(6)).max_by(|a, b| curve[*a].total_cmp(&curve[*b])).unwrap_or(0);
            let from_curve = (options.tremolo && curve.get(peak).is_some_and(|v| *v >= 3.0))
                .then(|| (peak..curve.len()).find(|i| curve[*i] < 0.5 * curve[peak]).map(|i| crate::held::ONSET_AT[i]))
                .flatten();
            round(from_curve.unwrap_or(measured), 2)
        },
        release_s: round(stroke.map_or_else(|| or(releases, 0.05), |s| s.2), 2),
        stroke_s: stroke.map(|s| round(s.1, 2)),
        glide_s: options.glide_s.unwrap_or(0.005),
        vibrato_cents: if stroke.is_some() { 0.0 } else { vibrato.map_or(0.0, |v| v.0) },
        vibrato_hz: vibrato.map_or(5.0, |v| v.1),
        vibrato_delay_s: vibrato.map_or(0.25, |v| v.2),
        vibrato_ramp_s: ramp,
        vibrato_start: begin,
        ensemble,
        tremolo,
        attack_db: if accent < 1.0 || stroke.is_some() { 0.0 } else { (accent * 10.0).round() / 10.0 },
        // 合奏里泛音的起伏大半是几个人互相拍出来的（引擎里几个人各拉各的，自己就会拍）：每个人自己的起伏按独奏的上限算。
        wobble: Wobble {
            // 揉着的录音里慢的起伏、音高的漂移和揉音分不干净，量出来偏大（演奏出来发飘）：按不揉的独奏乐器的水平封顶。
            depth: round(
                if ensemble.is_some() {
                    median(flucts).min(0.12)
                } else if measured.is_some() || tremolo.is_some() {
                    // 震音的录音也一样：音量一弓一弓的起伏另有 `tremolo`，不是随机的起伏。
                    median(flucts).min(0.08)
                } else {
                    median(flucts)
                },
                2,
            ),
            cents: round(if measured.is_some() { drift.min(2.0) } else { drift }, 2),
            // 揉着的乐器：比揉音快的那些另算了（`vibrato`），这里只剩慢的。
            rate_hz: round(median(rates).clamp(2.0, if vibrato.is_some() { 2.5 } else { 12.0 }), 2),
            vibrato: if vibrato.is_some() && ensemble.is_none() && stroke.is_none() && !quicks.is_empty() { round(0.5 * median(quicks), 2) } else { 0.0 },
        },
        bands: EDGES.to_vec(),
        notes,
    })
}

/// 用 muser 的一个乐器（`sfz:` 音色库、插件、内置乐器）把每个音高、每个力度层各吹一个长音，存成 `<音高>_v<力度>.wav`。
/// `layers`：每层的力度和这一层用的 `instrument_params`（力度靠控制器的音色库：`{"cc": {"1": 30}}`）；不出声的跳过。
pub fn render_notes(
    spec: &str,
    params: &muser_core::InstrumentParams,
    pitches: &[u8],
    layers: &[(f64, muser_core::InstrumentParams)],
    hold: f64,
    dir: &Path,
) -> Result<Vec<HeldInput>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    let gap = hold + 1.5;
    let mut out = Vec::new();
    for (at, extra) in layers {
        let mut project = muser_core::Project::new("held", 60.0);
        let mut track = muser_core::Track::new("t", spec);
        track.instrument_params = params.clone();
        track.instrument_params.extend(extra.clone());
        // 居中的音轨两声道各 -3 dB（等功率声像）：补回来。
        track.gain_db = 20.0 * core::f64::consts::SQRT_2.log10();
        let mut clip = muser_core::Clip::new(0.0, pitches.len() as f64 * gap);
        for (i, &p) in pitches.iter().enumerate() {
            let mut n = muser_core::Note::new(p, 0.25 + i as f64 * gap, hold);
            n.velocity = *at as f32;
            clip.notes.push(n);
        }
        track.clips.push(clip);
        project.tracks.push(track);
        let audio = crate::render::render(&project, crate::render::RenderSettings { tail_seconds: 0.5, ..Default::default() }).map_err(|e| e.to_string())?;
        for (i, &p) in pitches.iter().enumerate() {
            let start = ((0.2 + i as f64 * gap) * SR) as usize;
            let end = (start + ((hold + 1.0) * SR) as usize).min(audio.num_frames());
            if start >= end || (0..2).all(|c| audio.channel(c)[start..end].iter().all(|s| s.abs() < 1e-4)) {
                continue;
            }
            let mut b = manguebyte::prelude::AudioBuffer::<f32>::new(2, end - start);
            for c in 0..2 {
                b.channel_mut(c).copy_from_slice(&audio.channel(c)[start..end]);
            }
            let file = dir.join(format!("{p}_v{at:.2}.wav"));
            manguebyte::prelude::wav::write_file(&file, &b, 48_000, manguebyte::prelude::WavFormat::Float32)
                .map_err(|e| format!("cannot write '{}': {e:?}", file.display()))?;
            out.push(HeldInput { pitch: f64::from(p), at: *at, file });
        }
    }
    Ok(out)
}

/// 一个长音的对比结果（意思同 [`crate::modal::Comparison`]）。
#[derive(Debug, Clone, Serialize)]
pub struct NoteComparison {
    pub source: String,
    pub pitch: f64,
    pub at: f64,
    pub spectral_db: f64,
    pub level_db: f64,
}

/// 把每个录音在提取的乐器上用同样的音高、力度吹一遍，比较（各取稳定的那一段开头的 2 秒）。
pub fn compare(table_path: &str, inputs: &[HeldInput]) -> Result<Vec<NoteComparison>, String> {
    let table = crate::held::load(table_path)?;
    let spec = format!("yinse:{}", crate::modal::resolve(table_path).display());
    let dir = std::env::temp_dir().join(format!("muser-yinse-held-compare-{}", std::process::id()));
    let mut out = Vec::new();
    for input in inputs {
        let reference = read_stereo(&input.file, SR as u32)?;
        let hold = (reference[0].len() as f64 / SR - 0.5).clamp(1.5, 6.0);
        let ours = render_notes(&spec, &Default::default(), &[input.pitch.round() as u8], &[(input.at, Default::default())], hold, &dir)?;
        let Some(ours) = ours.first() else { continue };
        let ours = read_stereo(&ours.file, SR as u32)?;
        // 跳过起音（各有各的起法），从 0.3 秒比起。
        let skip = |lr: &[Vec<f64>; 2]| -> [Vec<f64>; 2] {
            let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
            let from = (onset_of(&mid) + (0.3 * SR) as usize).min(mid.len().saturating_sub(1));
            [lr[0][from..].to_vec(), lr[1][from..].to_vec()]
        };
        // 两个声道各比各的（立体声的录音两边不一样，乐器照着两边各自的大小出声；加在一起比的话比的是两支话筒之间的相位）。
        let (reference, ours) = (skip(&reference), skip(&ours));
        let one = |lr: &[Vec<f64>; 2], c: usize| features(&[lr[c].clone(), lr[c].clone()], SR);
        let mut cells = Vec::new();
        let (mut la, mut lb) = (0.0, 0.0);
        for c in 0..2 {
            let ((ga, _, _, a), (gb, _, _, b)) = (one(&reference, c), one(&ours, c));
            cells.extend(ga.iter().zip(&gb).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())));
            la += a * a / 2.0;
            lb += b * b / 2.0;
        }
        let (la, lb) = (la.sqrt(), lb.sqrt());
        out.push(NoteComparison {
            source: input.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            pitch: input.pitch,
            at: input.at,
            spectral_db: cells.iter().sum::<f64>() / cells.len().max(1) as f64,
            level_db: 20.0 * (lb / la).log10() - table.level_db,
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}

/// 这件乐器在参考乐句（音域中间的分解和弦，力度 0.8）上多响（LUFS）。
pub fn reference_lufs(path: &Path) -> Option<f64> {
    let table = crate::held::load(&path.to_string_lossy()).ok()?;
    let (lo, hi) = table.range();
    let project = crate::levels::phrase(&format!("yinse:{}", path.display()), &Default::default(), (lo.round() as u8, hi.round() as u8), false);
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

    /// 合成一个已知的长音：四个泛音（带相位）、一点 3 … 5 kHz 的噪声、起音 20 ms、收音 50 ms。
    fn synth(hz: f64, amps: [f64; 4], seconds: f64) -> Vec<f64> {
        let mut rng = 7u64;
        let n = ((seconds + 0.5) * SR) as usize;
        let phases = [0.0, 0.8, -1.2, 2.0];
        let noise: Vec<f64> = (0..n)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
            })
            .collect();
        let (hp, lp) = (crate::percussion::butterworth(true, 3_300.0, 0.707, SR), crate::percussion::butterworth(false, 4_800.0, 0.707, SR));
        let (mut a, mut b) = ([0.0; 2], [0.0; 2]);
        let mut gate = 0.0;
        (0..n)
            .map(|i| {
                let t = i as f64 / SR;
                let open = if t < seconds { 1.0 } else { 0.0 };
                gate += (open - gate) * (1.0 - (-1.0 / (if open > 0.5 { 0.02 } else { 0.05 } * SR)).exp());
                let tone: f64 = amps.iter().zip(phases).enumerate().map(|(k, (a, p))| a * (TAU * hz * (k + 1) as f64 * t + p).sin()).sum();
                gate * (tone + 0.004 * crate::percussion::biquad(&lp, &mut b, crate::percussion::biquad(&hp, &mut a, noise[i])))
            })
            .collect()
    }

    #[test]
    fn a_synthetic_long_note_is_measured_back() {
        let a = analyze(&synth(220.0, [0.2, 0.1, 0.05, 0.02], 2.5), 219.0, None, false).unwrap();
        assert!((a.hz - 220.0).abs() < 0.05, "hz {}", a.hz);
        for (k, want) in [0.2, 0.1, 0.05, 0.02].iter().enumerate() {
            assert!((a.amps[k] / want - 1.0).abs() < 0.03, "partial {}: {}", k + 1, a.amps[k]);
        }
        assert!(a.amps[5] < 0.001);
        for (k, want) in [0.0, 0.8, -1.2, 2.0].iter().enumerate() {
            assert!(((a.phases[k] - want + PI).rem_euclid(TAU) - PI).abs() < 0.05, "phase {}: {}", k + 1, a.phases[k]);
        }
        // 噪声落在 3.2 … 5 kHz 那一带。
        let loudest = (0..a.noise.len()).max_by(|&x, &y| a.noise[x].total_cmp(&a.noise[y])).unwrap();
        assert_eq!(EDGES[loudest], 3_200.0);
        assert!(a.fluct < 0.01 && a.cents < 0.5);
        let (attack, accent) = a.attack.unwrap();
        assert!((0.02..0.09).contains(&attack) && accent < 1.0, "attack {attack} s, accent {accent} dB");
        assert!((0.03..0.08).contains(&a.release.unwrap()), "release {:?}", a.release);
    }

    #[test]
    fn the_extracted_instrument_plays_back_like_the_recordings() {
        let dir = std::env::temp_dir().join(format!("muser-held-extract-test-{}", std::process::id())).join("blow");
        std::fs::create_dir_all(&dir).unwrap();
        let mut inputs = Vec::new();
        for (pitch, hz) in [(57.0, 220.0), (64.0, 329.63)] {
            for (at, amps) in [(0.3, [0.05, 0.01, 0.0, 0.0]), (0.9, [0.2, 0.1, 0.05, 0.02])] {
                let x = synth(hz, amps, 2.5);
                let mut b = manguebyte::prelude::AudioBuffer::<f32>::new(2, x.len());
                for c in 0..2 {
                    b.channel_mut(c).iter_mut().zip(&x).for_each(|(y, x)| *y = *x as f32);
                }
                let file = dir.join(format!("{pitch}_v{at}.wav"));
                manguebyte::prelude::wav::write_file(&file, &b, 48_000, manguebyte::prelude::WavFormat::Float32).unwrap();
                inputs.push(HeldInput { pitch, at, file });
            }
        }
        let table = extract("test", &inputs, Options::default(), |_| {}).unwrap();
        assert_eq!((table.notes.len(), table.notes[0].layers.len()), (2, 2));
        assert!(table.mono && (table.attack_s - 0.02).abs() < 0.015);
        let file = dir.join("test.yinse.json");
        std::fs::write(&file, serde_json::to_string(&table).unwrap()).unwrap();
        for c in compare(&file.to_string_lossy(), &inputs).unwrap() {
            assert!(c.spectral_db < 2.5 && c.level_db.abs() < 1.5, "{}: spectral {:.1} dB, level {:+.1} dB", c.source, c.spectral_db, c.level_db);
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
