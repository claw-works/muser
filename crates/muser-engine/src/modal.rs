//! 用户自己的模态乐器：从一组单音录音（一个文件一个音）里提取每个音的分音表——频率、起始音量、先快后慢两段衰减、
//! 一组弦之间的拍频、左右声道的比例与相位差、起音那一刻的相位、分频段的起音噪声、其余弦跟着响多少——存成一个很小的 JSON（`muser yinse extract`），
//! 再用模态弦模型演奏（`"instrument": "yinse:/path/x.yinse.json"`，见 [`crate::zither`]）。表里没有任何音频数据。
//!
//! 适合敲一下就自己衰减的乐器（扬琴、古筝、钢琴，以及分音不成整数倍的钟、木琴、鼓……）；吹管、拉弦这类持续发声的不适合。

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use manguebyte::dsp::fft::{Complex, RealFft};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "muser-yinse-modal";
/// 表的版本。2：分音带泛音序号与起始相位，起音噪声分频段（[`NoiseBand`]），不成整数倍的分音也收。
/// 1 的表（只有一个低通的起音噪声、分音不带序号）照样能演奏。
pub const VERSION: u32 = 2;
/// 每个音最多提取多少个分音（与模态弦模型的模式数相同；钢琴的低音要上百个）。
pub const MAX_PARTIALS: usize = 128;
/// 版本 1 的起音噪声的衰减时间常数（秒）。
pub const NOISE_SECONDS: f64 = 0.006;
/// 起音噪声分成几个倍频程的频段（中心 31.25 Hz 起，每个高一倍）。
pub const NOISE_BANDS: usize = 10;
/// 分析每个音的前几秒（录音没这么长就到末尾）。
const ANALYSIS_SECONDS: f64 = 20.0;
const HOP_SECONDS: f64 = 0.01;
/// 噪声只看起音后的这几秒。
const NOISE_ANALYSIS_SECONDS: f64 = 3.0;
/// 起音时每个分音从零升到位的时间（秒），和演奏时的一样（[`crate::zither`]）。
pub(crate) const ATTACK_SECONDS: f64 = 0.0015;

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// 一件乐器。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModalTable {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// 一敲琴码其余的弦跟着响多少（相对这个音的基音的振幅）。
    pub resonance: f64,
    pub notes: Vec<NoteTable>,
    /// 从带 `tremolo` / `vibrato` / `damp` 标记的录音量出来的动作。
    #[serde(default)]
    pub gestures: Gestures,
    /// 松开后余音到 -60 dB 的秒数（有制音器的乐器，如钢琴）；没有就是不止音，弦自己响完。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<f64>,
}

/// 动作类的演奏法：不是音色，是怎么动。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Gestures {
    /// 轮音：每秒几下、第二下（另一只手）相对第一下的力度、结尾比开头快几倍。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<Tremolo>,
    /// 揉音：上下各几音分、每秒几次、敲下去多久之后开始（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<Vibrato>,
    /// 止音：按住之后余音多少秒到 -60 dB。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damp: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Tremolo {
    pub rate: f64,
    pub alternate: f64,
    pub accel: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Vibrato {
    pub depth: f64,
    pub rate: f64,
    pub delay: f64,
}

/// 演奏法标记里属于动作的几个（其余的标记是音色，各自一组表）。
pub const GESTURES: [&str; 3] = ["tremolo", "vibrato", "damp"];
/// 文件名里认作演奏法的词（另外 `art-<名字>` 可以是任何名字）。
const ARTICULATION_WORDS: [&str; 12] = ["mute", "muted", "harmonic", "back", "pluck", "plucked", "staccato", "rim", "soft", "hard", "tremolo", "vibrato"];

/// 一个录音音：音高（MIDI，可带小数）、力度（0 … 1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteTable {
    pub source: String,
    /// 演奏法（录音文件名里的标记，空 = 普通的敲法）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub articulation: String,
    pub pitch: f64,
    pub velocity: f64,
    pub partials: Vec<Partial>,
    /// 版本 1 的起音噪声（版本 2 用 `bands`）。
    #[serde(default, skip_serializing_if = "Noise::is_none")]
    pub noise: Noise,
    /// 起音噪声，一个频段一项。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<NoiseBand>,
    /// 余音里不在这个音泛音上的能量占多少（0 … 1）：其余弦的共振。
    pub off_harmonic: f64,
}

/// 一个分音：频率比（相对基频）、振幅（录音满幅为 1）与到 -60 dB 的秒数；起音时多出来、很快衰减的那一段；
/// 一组弦之间的拍（另一根弦占的比例、差几音分）；左右声道的增益与右边落后的相位（弧度）；
/// 是第几次泛音（0 = 不在泛音列上的分音，或版本 1 的表）；起音那一刻左声道的相位（弧度，sin 起算）。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Partial {
    pub ratio: f64,
    pub amp: f64,
    pub t60: f64,
    pub fast_amp: f64,
    pub fast_t60: f64,
    pub beat: f64,
    pub beat_cents: f64,
    pub left: f64,
    pub right: f64,
    pub phase: f64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub n: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
}

/// 版本 1 的起音噪声：振幅、低通频率（Hz）。衰减时间常数固定为 [`NOISE_SECONDS`]。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Noise {
    pub amp: f64,
    pub hz: f64,
}

impl Noise {
    fn is_none(&self) -> bool {
        self.amp == 0.0
    }
}

/// 起音噪声的一个频段（一个倍频程宽）：中心频率、起音时的均方根（录音满幅为 1）与到 -60 dB 的秒数；
/// 后面拖着的慢的一段（没有就是 0）；左右声道各多响、左右有多像（相关度，1 = 一样，0 = 各是各的）。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct NoiseBand {
    pub hz: f64,
    pub amp: f64,
    pub t60: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tail: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tail_t60: f64,
    pub left: f64,
    pub right: f64,
    pub corr: f64,
}

/// 一个噪声频段的带通（两级同样的二阶带通串起来；演奏和这里算响度用同一个）：`[b0, b1, b2, a1, a2]`。
pub(crate) fn band_filter(hz: f64, sr: f64) -> [f64; 5] {
    let w = TAU * hz.min(0.42 * sr) / sr;
    let alpha = w.sin() / (2.0 * core::f64::consts::SQRT_2);
    let a0 = 1.0 + alpha;
    [alpha / a0, 0.0, -alpha / a0, -2.0 * w.cos() / a0, (1.0 - alpha) / a0]
}

/// -1 … 1 的均匀白噪声过 [`band_filter`] 两级之后的均方根（对频率响应积分；算过的记下来）。
pub(crate) fn band_unit_rms(hz: f64, sr: f64) -> f64 {
    static CACHE: Mutex<Vec<(u64, u64, f64)>> = Mutex::new(Vec::new());
    let key = (hz.to_bits(), sr.to_bits());
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&(_, _, v)) = cache.iter().find(|c| (c.0, c.1) == key) {
        return v;
    }
    let [b0, _, b2, a1, a2] = band_filter(hz, sr);
    let points = 1 << 15;
    let mut sum = 0.0;
    for k in 0..points {
        let w = core::f64::consts::PI * (k as f64 + 0.5) / points as f64;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let num = (b0 + b2 * c2).powi(2) + (b2 * s2).powi(2);
        let den = (1.0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2);
        sum += (num / den).powi(2);
    }
    let v = (sum / points as f64 / 3.0).sqrt();
    if cache.len() < 256 {
        cache.push((key.0, key.1, v));
    }
    v
}

// ---------------------------------------------------------------------------
// 文件名里的音高与力度
// ---------------------------------------------------------------------------

fn note_name(token: &str) -> Option<f64> {
    let mut chars = token.chars();
    let pc = match chars.next()?.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (shift, octave) = if let Some(o) = rest.strip_prefix('#').or_else(|| rest.strip_prefix('s')) {
        (1, o)
    } else if let Some(o) = rest.strip_prefix('b') {
        (-1, o)
    } else {
        (0, rest.as_str())
    };
    let octave: i32 = octave.parse().ok().filter(|o| (-1..=9).contains(o))?;
    Some(f64::from(12 * (octave + 1) + pc + shift))
}

/// 文件名里读出来的：音高、力度、演奏法。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NameInfo {
    pub pitch: Option<f64>,
    pub velocity: Option<f64>,
    pub articulation: Option<String>,
}

/// 从文件名里读音高（`A4` `Eb3` `C#5`，或单独的 MIDI 号 12 … 120）、力度（`v80` `vel100`（0 … 127）、`v0.7`、`pp` … `ff`）
/// 和演奏法（`mute` `harmonic` `back` `pluck` … ，`tremolo` `vibrato` `damp` 是动作，或 `art-<任何名字>`）。
pub fn parse_name(stem: &str) -> NameInfo {
    let (mut pitch, mut velocity) = (None, None);
    let mut articulation = None;
    let lower = stem.to_ascii_lowercase();
    if let Some(i) = lower.find("art-") {
        let name: String = lower[i + 4..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !name.is_empty() {
            articulation = Some(name);
        }
    }
    for token in stem.split(|c: char| !c.is_ascii_alphanumeric() && c != '#' && c != '.').filter(|t| !t.is_empty()) {
        let t = token.to_ascii_lowercase();
        if articulation.is_none() && (ARTICULATION_WORDS.contains(&t.as_str()) || t == "damp") {
            articulation = Some(if t == "muted" {
                "mute".into()
            } else if t == "plucked" {
                "pluck".into()
            } else {
                t
            });
            continue;
        }
        if pitch.is_none() {
            if let Some(p) = note_name(&t) {
                pitch = Some(p);
                continue;
            }
        }
        if velocity.is_none() {
            let number = t.strip_prefix("vel").or_else(|| t.strip_prefix('v')).and_then(|n| n.parse::<f64>().ok());
            if let Some(v) = number {
                velocity = Some(if v <= 1.0 { v } else { v / 127.0 }.clamp(0.0, 1.0));
                continue;
            }
            let dynamic = match t.as_str() {
                "ppp" => Some(0.15),
                "pp" => Some(0.25),
                "p" => Some(0.4),
                "mp" => Some(0.55),
                "mf" => Some(0.7),
                "f" => Some(0.85),
                "ff" => Some(0.95),
                "fff" => Some(1.0),
                _ => None,
            };
            if dynamic.is_some() {
                velocity = dynamic;
                continue;
            }
        }
        if pitch.is_none() {
            if let Ok(n) = t.parse::<u8>() {
                if (12..=120).contains(&n) {
                    pitch = Some(f64::from(n));
                }
            }
        }
    }
    NameInfo { pitch, velocity, articulation }
}

// ---------------------------------------------------------------------------
// 分析
// ---------------------------------------------------------------------------

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos()).collect()
}

/// 一段的幅度谱（加汉宁窗，补零到 `size`）。返回 (幅度, 每格多少 Hz)。
fn spectrum(x: &[f64], size: usize, sr: f64) -> (Vec<f64>, f64) {
    let w = hann(x.len());
    let mut input = vec![0.0; size];
    for (i, (v, w)) in x.iter().zip(&w).enumerate().take(size) {
        input[i] = v * w;
    }
    let mut fft = RealFft::<f64>::new(size);
    let mut out = vec![Complex::ZERO; fft.spectrum_len()];
    fft.forward(&input, &mut out);
    (out.iter().map(|c| c.norm_sqr().sqrt()).collect(), sr / size as f64)
}

/// `hz` 附近 ±`width` Hz 里最高的峰：抛物线插值后的 (频率, 幅度)。
fn peak_near(mag: &[f64], bin_hz: f64, hz: f64, width: f64) -> (f64, f64) {
    let lo = ((hz - width) / bin_hz).floor().max(1.0) as usize;
    let hi = (((hz + width) / bin_hz).ceil() as usize).min(mag.len() - 2);
    if lo >= hi {
        return (hz, 0.0);
    }
    let k = (lo..=hi).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).unwrap_or(lo);
    let (a, b, c) = (mag[k - 1].max(1e-30).ln(), mag[k].max(1e-30).ln(), mag[k + 1].max(1e-30).ln());
    let d = (a - c) / (2.0 * (a - 2.0 * b + c)).min(-1e-12);
    (((k as f64) + d.clamp(-0.5, 0.5)) * bin_hz, mag[k])
}

fn next_pow2(n: usize) -> usize {
    n.max(2).next_power_of_two()
}

/// 基频：在谱上找各次泛音加起来最响的那个频率（有提示就只在提示的 ±70 音分里找）。
fn fundamental(mag: &[f64], bin_hz: f64, hint: Option<f64>) -> f64 {
    let score = |f: f64| (1..=8).map(|n| peak_near(mag, bin_hz, f * n as f64, (f * n as f64 * 0.012).max(bin_hz)).1.sqrt()).sum::<f64>();
    let (lo, hi, step) = match hint {
        Some(p) => (p - 0.7, p + 0.7, 0.02),
        None => (24.0, 100.0, 0.05),
    };
    let mut best = (f64::NEG_INFINITY, lo);
    let mut p = lo;
    while p <= hi {
        let s = score(440.0 * ((p - 69.0) / 12.0).exp2());
        if s > best.0 {
            best = (s, p);
        }
        p += step;
    }
    // 网格只到 2 音分，且宽窗口下附近的分数都一样：用前四个分音的实测峰（按响度加权）定准。
    let f = 440.0 * ((best.1 - 69.0) / 12.0).exp2();
    let (num, den) = (1..=4).fold((0.0, 0.0), |(a, b), n| {
        let (hz, m) = peak_near(mag, bin_hz, f * n as f64, (f * n as f64 * 0.015).max(bin_hz));
        (a + m * hz / n as f64, b + m)
    });
    if den > 0.0 {
        num / den
    } else {
        f
    }
}

/// 一个频率在给定的各帧（起点 `starts`，窗长 `window`）上的复振幅：加汉宁窗解调。放不下一整窗的帧不算。
fn demodulate_at(x: &[f64], hz: f64, window: usize, starts: &[usize], sr: f64) -> Vec<Complex<f64>> {
    let w = hann(window);
    let norm: f64 = w.iter().sum();
    let step = Complex::<f64>::from_polar(1.0, -TAU * hz / sr);
    let mut out = Vec::with_capacity(starts.len());
    for &start in starts.iter().filter(|&&s| s + window <= x.len()) {
        let mut ph = Complex::<f64>::from_polar(1.0, -TAU * hz * start as f64 / sr);
        let (mut re, mut im) = (0.0, 0.0);
        for (v, w) in x[start..start + window].iter().zip(&w) {
            let a = v * w;
            re += a * ph.re;
            im += a * ph.im;
            ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
        }
        out.push(Complex::new(2.0 * re / norm, 2.0 * im / norm));
    }
    out
}

/// 等间隔的帧上的复振幅。
fn demodulate(x: &[f64], hz: f64, window: usize, hop: usize, sr: f64) -> Vec<Complex<f64>> {
    let starts: Vec<usize> = (0..).map(|k| k * hop.max(1)).take_while(|s| s + window <= x.len()).collect();
    demodulate_at(x, hz, window, &starts, sr)
}

/// 分析用的帧：前 2 秒每 10 ms 一帧，之后每 40 ms 一帧（余音变化慢，长录音不必那么密）。
fn frame_starts(len: usize, window: usize, sr: f64) -> Vec<usize> {
    let hop = ((HOP_SECONDS * sr) as usize).max(1);
    let dense = (2.0 * sr) as usize;
    let mut out = Vec::new();
    let mut s = 0;
    while s + window <= len {
        out.push(s);
        s += if s < dense { hop } else { 4 * hop };
    }
    out
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// 最小二乘直线 y = a + b·x。
fn line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if points.len() < 3 {
        return None;
    }
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(a, b), &(x, y)| (a + x, b + y));
    let (mx, my) = (sx / n, sy / n);
    let (sxx, sxy) = points.iter().fold((0.0, 0.0), |(a, b), &(x, y)| (a + (x - mx).powi(2), b + (x - mx) * (y - my)));
    (sxx > 1e-12).then(|| (my - sxy / sxx * mx, sxy / sxx))
}

/// 一条包络拟合成的两段衰减：慢的一段在峰值处的振幅和斜率（dB/秒）、起音多出来的快的一段；
/// 用的是第 `kp` 帧（峰值）到第 `end` 帧之前，一共 `span` 秒。
struct Decay {
    amp: f64,
    slope: f64,
    fast_amp: f64,
    fast_t60: f64,
    kp: usize,
    end: usize,
    span: f64,
}

/// 包络 `amp[k]`（时刻 `t[k]`）→ 先快后慢两段。峰值在前 `early` 秒里找；只用峰值之后还在底噪 `floor` 以上
///（并且没掉过 60 dB）的那一段：掉进底噪的部分是平的，拟合进去余音就长得离谱。
/// 两段的分界在哪里因乐器而异：`splits` 里的几个都试（不早于峰值后 `lead` 秒），取整条包络（dB）拟合得最好的；
/// 分界之后至少要剩 `min_after` 秒。
fn fit_decay(t: &[f64], amp: &[f64], floor: f64, early: f64, lead: f64, splits: &[f64], min_after: f64) -> Option<Decay> {
    let db: Vec<f64> = amp.iter().map(|a| 20.0 * a.max(1e-12).log10()).collect();
    let first = t.iter().take_while(|&&x| x < early).count().clamp(1, t.len());
    let kp = (0..first).max_by(|&a, &b| db[a].total_cmp(&db[b])).unwrap_or(0);
    let top = db[kp];
    let gate = 2.5 * floor;
    let end = (kp..db.len()).find(|&k| db[k] < top - 60.0 || (amp[k] < gate && amp.get(k + 1).is_none_or(|&a| a < gate))).unwrap_or(db.len());
    if end < kp + 3 {
        return None;
    }
    // 分界以后拟合慢的一段，分界以前比慢的多出来的拟合快的一段。
    let fit = |from: f64| -> Option<(f64, f64, f64, f64, f64)> {
        let slow: Vec<(f64, f64)> = (kp..end).filter(|&k| t[k] >= from).map(|k| (t[k] - t[kp], db[k])).collect();
        let (b, m) = line(&slow)?;
        let a0 = 10f64.powf(b / 20.0);
        let slow_at = |x: f64| a0 * 10f64.powf(m * (x - t[kp]) / 20.0);
        let excess: Vec<(f64, f64)> = (kp..end)
            .filter(|&k| t[k] < from)
            .filter_map(|k| {
                let e = amp[k] - slow_at(t[k]);
                (e > 0.05 * amp[k]).then(|| (t[k] - t[kp], e.ln()))
            })
            .collect();
        let (mut fa, mut ft) = (0.0, 0.1);
        if let Some((c, d)) = line(&excess) {
            if d < -0.5 && amp[kp] - slow_at(t[kp]) > 0.15 * slow_at(t[kp]) {
                fa = c.exp().min(amp[kp]);
                ft = (1000f64.ln() / -d).clamp(0.003, 3.0);
            }
        }
        let total = |x: f64| slow_at(x) + fa * (-(x - t[kp]) * 1000f64.ln() / ft).exp();
        // 响的地方差一点比快听不见的地方差一点要紧：按振幅加权。
        let err = (kp..end).map(|k| amp[k] * (db[k] - 20.0 * total(t[k]).max(1e-12).log10()).powi(2)).sum::<f64>()
            / (kp..end).map(|k| amp[k]).sum::<f64>().max(1e-30);
        Some((err, b, m, fa, ft))
    };
    let t_end = t[end - 1];
    let base = t[kp] + lead;
    let best = splits
        .iter()
        .map(|&s| base.max(s))
        .filter(|&from| t_end - from >= min_after || from <= base.max(splits[0]))
        .filter_map(fit)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let (_, b, m, fast_amp, fast_t60) = best.or_else(|| fit(t[kp] + 0.2 * lead))?;
    Some(Decay { amp: 10f64.powf(b / 20.0), slope: m, fast_amp, fast_t60, kp, end, span: t_end - t[kp] })
}

/// 一个分音的包络 → 慢衰减（振幅、t60）、起音多出的快衰减、拍。第二个返回值：衰减量得可靠吗
///（底噪以上那一段够长；不可靠的由 [`analyze`] 按这个音其余分音的规律补）。
fn fit_envelope(t: &[f64], amp: &[f64], hz: f64, floor: f64) -> (Partial, bool) {
    let Some(d) = fit_decay(t, amp, floor, 0.08, 0.06, &[0.1, 0.2, 0.3, 0.6, 1.0, 1.5, 2.0], 0.8) else {
        let first = t.iter().take_while(|&&x| x < 0.08).count().clamp(1, amp.len());
        let peak = amp[..first].iter().copied().fold(0.0, f64::max);
        return (Partial { ratio: 1.0, amp: peak, t60: 1.0, left: 1.0, right: 1.0, ..Default::default() }, false);
    };
    let (kp, end) = (d.kp, d.end);
    // 斜率几乎是平的：量了 1.5 秒以上才信它真的这么长。
    let reliable = d.span >= 0.3 && end - kp >= 8 && (d.slope < -0.5 || d.span >= 1.5);
    let t60 = if d.slope < -0.5 { -60.0 / d.slope } else { 30.0 }.clamp(0.05, 30.0);
    let fast_t60 = d.fast_t60.max(0.01);
    // 演奏时从起音那一刻（峰值）开始：振幅取拟合线在峰值处的值。
    let model_db =
        |x: f64| 20.0 * (d.amp * 10f64.powf(d.slope * (x - t[kp]) / 20.0) + d.fast_amp * (-(x - t[kp]) * 1000f64.ln() / fast_t60).exp()).max(1e-12).log10();
    // 拍：残差（dB）里最强的周期起伏。要它真的是一个周期起伏（占残差的一半以上、看到两个周期以上、起伏 1 dB 以上），
    // 而且两根弦差得不离谱（25 音分以内）：否则只是包络上的毛刺。
    let res: Vec<(f64, f64)> = (kp..end).filter(|&k| t[k] >= 0.1).map(|k| (t[k], 20.0 * amp[k].max(1e-12).log10() - model_db(t[k]))).collect();
    let (mut beat, mut beat_cents) = (0.0, 0.0);
    if res.len() > 40 && res[res.len() - 1].0 - res[0].0 > 0.5 {
        let n = res.len() as f64;
        let span = res[res.len() - 1].0 - res[0].0;
        let mean = res.iter().map(|r| r.1).sum::<f64>() / n;
        let var = res.iter().map(|r| (r.1 - mean).powi(2)).sum::<f64>() / n;
        let mut best = (0.0, 0.0);
        let mut f = 0.3;
        while f < 12.0 {
            let (re, im) = res.iter().fold((0.0, 0.0), |(a, c), &(x, y)| (a + (y - mean) * (TAU * f * x).cos(), c + (y - mean) * (TAU * f * x).sin()));
            let a = 2.0 * (re * re + im * im).sqrt() / n;
            if a > best.0 {
                best = (a, f);
            }
            f += 0.05;
        }
        let cents = 1200.0 * (1.0 + best.1 / hz).log2();
        if best.0 >= 1.0 && best.0 * best.0 / 2.0 >= 0.5 * var && span * best.1 >= 2.0 && cents <= 25.0 {
            // 主弦占 1-w、另一根占 w：合起来在 1 与 1-2w 之间起伏，峰峰值 2A dB。
            beat = ((1.0 - 10f64.powf(-2.0 * best.0 / 20.0)) / 2.0).clamp(0.0, 0.45);
            beat_cents = cents;
        }
    }
    (Partial { ratio: 1.0, amp: d.amp, t60, fast_amp: d.fast_amp, fast_t60, beat, beat_cents, left: 1.0, right: 1.0, ..Default::default() }, reliable)
}

/// 量到的一个分音在各帧上的复振幅（左、右）：用来从录音里减掉它，剩下的就是噪声。
struct Track {
    hz: f64,
    zl: Vec<Complex<f64>>,
    zr: Vec<Complex<f64>>,
    centers: Vec<usize>,
    /// 解调窗的等效噪声带宽（Hz）：减掉这个分音时一起减掉的噪声。
    width: f64,
}

/// 从 `x` 里减掉一个跟踪到的分音（帧之间线性插值；`on` 之前没有，之后和演奏时一样在 [`ATTACK_SECONDS`] 里升起来）。
fn subtract_track(x: &mut [f64], z: &[Complex<f64>], centers: &[usize], hz: f64, sr: f64, on: usize) {
    if z.is_empty() {
        return;
    }
    let ramp = (ATTACK_SECONDS * sr).max(1.0);
    let step = Complex::<f64>::from_polar(1.0, TAU * hz / sr);
    let mut ph = Complex::<f64>::from_polar(1.0, 0.0);
    let mut j = 0;
    let last = z.len() - 1;
    for (i, sample) in x.iter_mut().enumerate().skip(on) {
        if (i - on).is_multiple_of(4096) {
            ph = Complex::<f64>::from_polar(1.0, TAU * hz * i as f64 / sr);
        }
        while j < last && centers[j + 1] <= i {
            j += 1;
        }
        let (re, im) = if i <= centers[0] {
            (z[0].re, z[0].im)
        } else if j >= last {
            (z[last].re, z[last].im)
        } else {
            let f = (i - centers[j]) as f64 / (centers[j + 1] - centers[j]) as f64;
            (z[j].re + (z[j + 1].re - z[j].re) * f, z[j].im + (z[j + 1].im - z[j].im) * f)
        };
        let a = ((i - on) as f64 / ramp).min(1.0);
        *sample -= a * a * (3.0 - 2.0 * a) * (re * ph.re - im * ph.im);
        ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
    }
}

/// 减掉分音以后剩下的声音（左、右）→ 每个倍频程一个频段的噪声：起音时多响、多快收掉、后面拖不拖一段。
/// `on` 是起音的样本；`tracks` 是减掉的分音（它们的窗一起减掉了一点噪声，按比例补回来）。
fn noise_bands(l: &[f64], r: &[f64], sr: f64, on: usize, tracks: &[Track]) -> Vec<NoiseBand> {
    let n = l.len();
    let size = next_pow2(n);
    let mut fft = RealFft::<f64>::new(size);
    let fade = ((0.02 * sr) as usize).min(n / 4).max(1);
    let mut transform = |x: &[f64]| {
        let mut input = vec![0.0; size];
        input[..n].copy_from_slice(x);
        // 截断的地方渐出：不然那个台阶会散到所有频段里。
        for k in 0..fade {
            input[n - 1 - k] *= k as f64 / fade as f64;
        }
        let mut out = vec![Complex::ZERO; size / 2 + 1];
        fft.forward(&input, &mut out);
        out
    };
    let (sl, sr_) = (transform(l), transform(r));
    let bin = sr / size as f64;
    let mut out = Vec::new();
    for i in 0..NOISE_BANDS {
        let c = 31.25 * (i as f64).exp2();
        let (lo, hi) = (c / core::f64::consts::SQRT_2, (c * core::f64::consts::SQRT_2).min(0.48 * sr));
        if lo >= 0.45 * sr {
            break;
        }
        let mut band = |s: &[Complex<f64>]| {
            let mut b = vec![Complex::ZERO; s.len()];
            let (a, z) = ((lo / bin).ceil() as usize, ((hi / bin).floor() as usize).min(s.len() - 1));
            b[a..=z.max(a)].copy_from_slice(&s[a..=z.max(a)]);
            let mut y = vec![0.0; size];
            fft.inverse(&b, &mut y);
            y.truncate(n);
            y
        };
        let (bl, br) = (band(&sl), band(&sr_));
        // 每帧至少三个周期。
        let frame = (((3.0 / c).clamp(0.003, 0.04)) * sr) as usize;
        let count = (n - on) / frame.max(1);
        if count < 4 {
            continue;
        }
        let power = |x: &[f64], k: usize| x[on + k * frame..on + (k + 1) * frame].iter().map(|v| v * v).sum::<f64>() / frame as f64;
        // 减分音时一起减掉的噪声补回来。
        let removed: f64 = tracks.iter().filter(|t| (lo..hi).contains(&t.hz)).map(|t| t.width).sum::<f64>() / (hi - lo);
        let gain = 1.0 / (1.0 - removed.min(0.6)).sqrt();
        let t: Vec<f64> = (0..count).map(|k| (k as f64 + 0.5) * frame as f64 / sr).collect();
        let e: Vec<f64> = (0..count).map(|k| gain * (0.5 * (power(&bl, k) + power(&br, k))).sqrt()).collect();
        // 底：这几秒的最后五分之一（没减干净的分音、房间的声音、录音的底噪都在里面）。
        let floor = if n as f64 >= sr { median(e[count - count / 5 - 1..].to_vec()) } else { 0.0 };
        let first = t.iter().take_while(|&&x| x < 0.08).count().clamp(1, count);
        let kp = (0..first).max_by(|&a, &b| e[a].total_cmp(&e[b])).unwrap_or(0);
        let peak = e[kp];
        if peak < 4.0 * floor || peak < 3e-6 {
            continue;
        }
        let (amp, t60, tail, tail_t60) = match fit_decay(&t, &e, floor, 0.08, 0.0, &[0.03, 0.06, 0.12, 0.25, 0.5], 0.1) {
            // 一两帧就没了：按第一帧掉了多少算。
            None => {
                let drop = 20.0 * (peak / e.get(kp + 1).copied().unwrap_or(floor).max(peak * 1e-3)).log10();
                (peak, (60.0 * frame as f64 / sr / drop.max(1.0)).clamp(0.003, 0.5), 0.0, 0.0)
            }
            Some(d) => {
                // 拖得比 4 秒还长的不是敲击的噪声（是没减干净的分音、别的弦）：不要。
                let slow = if d.slope < -15.0 { Some((-60.0 / d.slope).max(0.003)) } else { None };
                match (d.fast_amp > 0.0, slow) {
                    (true, Some(s)) => (d.fast_amp, d.fast_t60, d.amp, s),
                    (true, None) => (d.fast_amp, d.fast_t60, 0.0, 0.0),
                    (false, Some(s)) => (d.amp, s, 0.0, 0.0),
                    (false, None) => continue,
                }
            }
        };
        // 左右：峰值起 50 ms（低的频段两帧）。
        let a = on + kp * frame;
        let b = (a + (2 * frame).max((0.05 * sr) as usize)).min(n);
        let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
        for k in a..b {
            ll += bl[k] * bl[k];
            rr += br[k] * br[k];
            lr += bl[k] * br[k];
        }
        let mid = (0.5 * (ll + rr)).max(1e-30);
        out.push(NoiseBand {
            hz: c,
            amp,
            t60,
            tail,
            tail_t60,
            left: (ll / mid).sqrt(),
            right: (rr / mid).sqrt(),
            corr: (lr / (ll * rr).sqrt().max(1e-30)).clamp(-1.0, 1.0),
        });
    }
    out
}

/// 分析一个单音录音。`pitch` 为 None 时从声音里测。
pub fn analyze(path: &Path, pitch: Option<f64>, velocity: f64) -> Result<NoteTable, String> {
    let file = manguebyte::formats::read_file(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let sr = f64::from(file.sample_rate);
    let buf = &file.buffer;
    if buf.num_channels() == 0 || buf.num_frames() == 0 {
        return Err(format!("'{}' is empty", path.display()));
    }
    let left: Vec<f64> = buf.channel(0).iter().map(|&v| f64::from(v)).collect();
    let right: Vec<f64> = buf.channel(buf.num_channels().min(2) - 1).iter().map(|&v| f64::from(v)).collect();
    let mid: Vec<f64> = left.iter().zip(&right).map(|(a, b)| 0.5 * (a + b)).collect();
    let peak = mid.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if peak < 1e-4 {
        return Err(format!("'{}' is silent", path.display()));
    }
    let crossing = mid.iter().position(|v| v.abs() > 0.05 * peak).unwrap_or(0);
    let onset = crossing.saturating_sub((0.002 * sr) as usize);
    // 起音在这一段里的第几个样本（演奏时分音从这里开始）。
    let on = crossing - onset;
    let len = (mid.len() - onset).min((ANALYSIS_SECONDS * sr) as usize);
    let (l, r, x) = (&left[onset..onset + len], &right[onset..onset + len], &mid[onset..onset + len]);
    if len < (0.3 * sr) as usize {
        return Err(format!("'{}' is shorter than 0.3 s after the attack", path.display()));
    }
    // 基频与各分音的频率：起音后 20 ms 起一秒（不够就到文件末尾）。
    let a = (0.02 * sr) as usize;
    let b = len.min((1.0 * sr) as usize);
    let size = next_pow2(4 * (b - a)).max(65_536);
    let (mag, bin_hz) = spectrum(&x[a..b], size, sr);
    let f0 = fundamental(&mag, bin_hz, pitch);
    // 泛音列上的分音：(频率, 第几次)。
    let mut found: Vec<(f64, u32)> = Vec::new();
    let (mut stretch, mut misses) = (1.0, 0);
    for n in 1..=MAX_PARTIALS {
        let expect = f0 * n as f64 * stretch;
        if expect > 0.45 * sr {
            break;
        }
        // 基音只在 ±4% 里找；高的分音按已经量到的伸张往外推，窗口不超过它自己频率的 5%（高音区的窗口不会抓到别的峰）。
        let width = if n == 1 { 0.04 * f0 } else { (0.3 * f0).min(0.05 * expect) };
        let (hz, m) = peak_near(&mag, bin_hz, expect, width.max(2.0 * bin_hz));
        let lo = ((expect - f0 * 0.5) / bin_hz).max(1.0) as usize;
        let hi = (((expect + f0 * 0.5) / bin_hz) as usize).min(mag.len() - 1);
        let floor = median(mag[lo..hi].to_vec());
        if m > 3.0 * floor && m > 1e-7 * size as f64 {
            found.push((hz, n as u32));
            stretch = hz / (f0 * n as f64);
            misses = 0;
        } else {
            misses += 1;
            if misses >= 6 {
                break;
            }
        }
    }
    let harmonics: Vec<f64> = found.iter().map(|f| f.0).collect();
    // 不在泛音列上的分音（钟、木琴、鼓的各个振动模式，弦乐器琴体的共鸣）：谱上其余的峰，从最响的收起。
    // 要明显高出周围（8 倍）、不比最响的峰低 50 dB 以上，并且离已经收的分音够远（离得近的是同一个分音的拍）。
    {
        let block = ((f0.max(100.0) / bin_hz) as usize).max(8);
        let floors: Vec<f64> = mag.chunks(block).map(|c| median(c.to_vec())).collect();
        let top = mag.iter().copied().fold(0.0, f64::max);
        let (lo, hi) = ((30.0 / bin_hz) as usize + 1, ((0.45 * sr / bin_hz) as usize).min(mag.len() - 2));
        let mut peaks: Vec<(f64, usize)> = (lo..hi)
            .filter(|&k| {
                mag[k] > mag[k - 1]
                    && mag[k] >= mag[k + 1]
                    && mag[k] > 8.0 * floors[k / block]
                    && mag[k] > top * 10f64.powf(-50.0 / 20.0)
                    && mag[k] > 1e-7 * size as f64
            })
            .map(|k| (mag[k], k))
            .collect();
        peaks.sort_by(|a, b| b.0.total_cmp(&a.0));
        let apart = (0.3 * f0).max(8.0);
        let mut free = 0;
        for (_, k) in peaks {
            if found.len() >= MAX_PARTIALS || free >= MAX_PARTIALS / 2 {
                break;
            }
            let (hz, _) = peak_near(&mag, bin_hz, k as f64 * bin_hz, bin_hz);
            if found.iter().all(|f| (f.0 - hz).abs() >= apart) {
                found.push((hz, 0));
                free += 1;
            }
        }
    }
    if found.is_empty() {
        return Err(format!("no partials found in '{}'", path.display()));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    // 文件名给了音高：分音的频率比就相对那个音高（录音本身偏了多少音分也照样还原）；没给就相对量到的基频。
    let base = pitch.map_or(f0, |p| 440.0 * ((p - 69.0) / 12.0).exp2());
    let hop = ((HOP_SECONDS * sr) as usize).max(1);
    let wrap = |a: f64| (a + core::f64::consts::PI).rem_euclid(TAU) - core::f64::consts::PI;
    let mut partials = Vec::new();
    let mut reliable = Vec::new();
    let mut tracks = Vec::new();
    for &(hz0, n) in &found {
        // 每个分音的包络：窗要够长才分得开相邻的分音（比基频还低的分音至少四个周期）。
        let window = ((4.5 / f0).max(0.02).max(4.0 / hz0) * sr) as usize;
        let starts = frame_starts(len, window, sr);
        let zl = demodulate_at(l, hz0, window, &starts, sr);
        let zr = demodulate_at(r, hz0, window, &starts, sr);
        if zl.len() < 5 {
            continue;
        }
        let t: Vec<f64> = starts.iter().take(zl.len()).map(|s| (s + window / 2) as f64 / sr).collect();
        let amp: Vec<f64> = zl.iter().zip(&zr).map(|(a, b)| 0.5 * (a.norm_sqr().sqrt() + b.norm_sqr().sqrt())).collect();
        // 这个频率上的底噪：录音最后一秒里，它上下各偏开一点（解调窗的两格多：泛音列上正好是两个分音之间）量到的。
        let aside = 2.25 * sr / window as f64;
        let from = len - ((sr as usize).min(len / 4).max(window)).min(len);
        let tail: Vec<usize> = (from..).step_by((window / 2).max(1)).take_while(|s| s + window <= len).collect();
        let level = |f: f64| {
            if !(20.0..0.45 * sr).contains(&f) || tail.is_empty() {
                return f64::INFINITY;
            }
            let (a, b) = (demodulate_at(l, f, window, &tail, sr), demodulate_at(r, f, window, &tail, sr));
            median(a.iter().zip(&b).map(|(a, b)| 0.5 * (a.norm_sqr().sqrt() + b.norm_sqr().sqrt())).collect())
        };
        let floor = level(hz0 - aside).min(level(hz0 + aside));
        let floor = if floor.is_finite() { floor } else { 0.0 };
        let (mut p, ok) = fit_envelope(&t, &amp, hz0, floor);
        // 不在泛音列上、又一直不衰减的不是这个音的一部分（电源的哼声、房间里一直响着的东西）：不收。
        // 起音时在它上面多出来、很快收掉的那一段是（琴键、琴槌"咚"的一下）：只留这一段。
        if n == 0 && p.t60 >= 30.0 {
            if p.fast_amp <= 0.0 {
                continue;
            }
            (p.amp, p.t60, p.fast_amp, p.beat, p.beat_cents) = (p.fast_amp, p.fast_t60, 0.0, 0.0, 0.0);
        }
        // 频率定准：前 0.6 秒里相邻两帧的相位差（谱峰只准到零点几 Hz，起始相位要更准）。
        let (mut cr, mut ci) = (0.0, 0.0);
        for k in 0..zl.len() - 1 {
            if t[k] >= 0.03 && t[k + 1] <= 0.6 && starts[k + 1] - starts[k] == hop && amp[k] > 2.5 * floor {
                for z in [&zl, &zr] {
                    cr += z[k + 1].re * z[k].re + z[k + 1].im * z[k].im;
                    ci += z[k + 1].im * z[k].re - z[k + 1].re * z[k].im;
                }
            }
        }
        let shift = if cr != 0.0 || ci != 0.0 { ci.atan2(cr) / (TAU * hop as f64 / sr) } else { 0.0 };
        let hz = hz0 + if shift.abs() <= 2.0 { shift } else { 0.0 };
        p.ratio = hz / base;
        p.n = n;
        // 立体声：起音后 20 … 600 ms 的左右平均振幅与右边相对左边的相位。
        let (mut sl, mut sr_, mut cr, mut ci) = (0.0, 0.0, 0.0, 0.0);
        let early = t.iter().any(|&x| (0.02..0.6).contains(&x));
        for k in (0..t.len()).filter(|&k| !early || (0.02..0.6).contains(&t[k])) {
            sl += zl[k].norm_sqr().sqrt();
            sr_ += zr[k].norm_sqr().sqrt();
            // zr · conj(zl)
            cr += zr[k].re * zl[k].re + zr[k].im * zl[k].im;
            ci += zr[k].im * zl[k].re - zr[k].re * zl[k].im;
        }
        let g = 0.5 * (sl + sr_);
        if g > 1e-12 {
            p.left = sl / g;
            p.right = sr_ / g;
            // 右边的相位比左边大多少（右边超前）。
            p.phase = ci.atan2(cr);
        }
        // 起音那一刻的相位：第一帧量到的相位（cos 起算，相对解调的频率）按定准的频率推回到起音。
        let (z0, lag) = if sl >= 0.1 * sr_ { (zl[0], 0.0) } else { (zr[0], p.phase) };
        let at = z0.im.atan2(z0.re) + TAU * hz0 * t[0] - TAU * hz * (t[0] - on as f64 / sr) - lag;
        p.start = wrap(at + core::f64::consts::FRAC_PI_2);
        partials.push(p);
        reliable.push(ok);
        tracks.push(Track { hz: hz0, centers: starts.iter().take(zl.len()).map(|s| s + window / 2).collect(), zl, zr, width: 1.5 * sr / window as f64 });
    }
    if partials.is_empty() {
        return Err(format!("'{}' is too short to measure", path.display()));
    }
    // 衰减量不出来的分音（太弱，一起音就掉进底噪）：按这个音量得出来的那些分音"频率越高衰减越快"的规律补。
    let known: Vec<(f64, f64)> = partials.iter().zip(&reliable).filter(|(_, ok)| **ok).map(|(p, _)| (p.ratio.ln(), p.t60.ln())).collect();
    if !known.is_empty() {
        let longest = known.iter().map(|k| k.1).fold(f64::NEG_INFINITY, f64::max);
        let law = line(&known).filter(|l| known.len() >= 4 && l.1 <= 0.0);
        let middle = median(known.iter().map(|k| k.1).collect());
        for (p, _) in partials.iter_mut().zip(&reliable).filter(|(_, ok)| !**ok) {
            let guess = law.map_or(middle, |(a, b)| a + b * p.ratio.ln());
            p.t60 = guess.min(longest).exp().clamp(0.05, 30.0);
            p.fast_amp = 0.0;
            p.beat = 0.0;
            p.beat_cents = 0.0;
        }
    }
    // 起音的噪声：前几秒里把量到的分音都减掉，剩下的按频段量。
    let span = len.min((NOISE_ANALYSIS_SECONDS * sr) as usize);
    let (mut rl, mut rr) = (l[..span].to_vec(), r[..span].to_vec());
    for t in &tracks {
        subtract_track(&mut rl, &t.zl, &t.centers, t.hz, sr, on);
        subtract_track(&mut rr, &t.zr, &t.centers, t.hz, sr, on);
    }
    let bands = noise_bands(&rl, &rr, sr, on, &tracks);
    // 共振：1 … 2 秒的余音里不在这个音泛音上的能量（只看量到泛音的那一段频率）。
    let mut off_harmonic = 0.0;
    if len as f64 > 2.2 * sr && !harmonics.is_empty() {
        let (mt, bt) = spectrum(&x[(1.0 * sr) as usize..(2.0 * sr) as usize], next_pow2((sr * 2.0) as usize), sr);
        let top = (harmonics[harmonics.len() - 1] + 0.5 * f0).min(8_000.0);
        let (mut all, mut harm) = (0.0, 0.0);
        for (k, m) in mt.iter().enumerate().skip(1) {
            let f = k as f64 * bt;
            if !(60.0..top).contains(&f) {
                continue;
            }
            let e = m * m;
            all += e;
            let i = harmonics.partition_point(|&h| h < f);
            let near = [i.checked_sub(1), Some(i)].into_iter().flatten().filter_map(|i| harmonics.get(i)).map(|h| (h - f).abs()).fold(f64::INFINITY, f64::min);
            if near < 3.0 + 0.03 * f0 {
                harm += e;
            }
        }
        if all > 0.0 {
            off_harmonic = (1.0 - harm / all).clamp(0.0, 1.0);
        }
    }
    Ok(NoteTable {
        source: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        pitch: pitch.unwrap_or(69.0 + 12.0 * (f0 / 440.0).log2()),
        velocity,
        partials,
        noise: Noise::default(),
        bands,
        off_harmonic,
        articulation: String::new(),
    })
}

/// 从一组文件（或文件夹里的 .wav / .flac / .aif …）提取一件乐器。`on_note` 每分析完一个音调用一次（报告进度）。
pub fn extract(inputs: &[PathBuf], name: &str, mut on_note: impl FnMut(&NoteTable)) -> Result<ModalTable, String> {
    let mut files = Vec::new();
    for p in inputs {
        if p.is_dir() {
            let mut list: Vec<PathBuf> = std::fs::read_dir(p)
                .map_err(|e| format!("cannot read folder '{}': {e}", p.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|f| {
                    f.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| ["wav", "flac", "aif", "aiff", "ogg", "mp3"].contains(&e.to_ascii_lowercase().as_str()))
                })
                .collect();
            list.sort();
            files.extend(list);
        } else {
            files.push(p.clone());
        }
    }
    if files.is_empty() {
        return Err("no audio files given".into());
    }
    let mut notes = Vec::new();
    let (mut tremolos, mut vibratos, mut damps) = (Vec::new(), Vec::new(), Vec::new());
    for f in &files {
        let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let info = parse_name(&stem);
        let gesture = info.articulation.as_deref().filter(|a| GESTURES.contains(a));
        let mut n = analyze(f, info.pitch, info.velocity.unwrap_or(-1.0))?;
        n.articulation = info.articulation.clone().unwrap_or_default();
        on_note(&n);
        match gesture {
            // 动作的录音只量动作，不当音色。
            Some(g) => {
                let (x, sr) = read_mid(f)?;
                let f0 = 440.0 * ((n.pitch - 69.0) / 12.0).exp2();
                let found = match g {
                    "tremolo" => analyze_tremolo(&x, sr).map(|t| tremolos.push(t)),
                    "vibrato" => analyze_vibrato(&x, sr, f0, &n.partials).map(|v| vibratos.push(v)),
                    _ => analyze_damp(&x, sr).map(|d| damps.push(d)),
                };
                if found.is_none() {
                    return Err(format!("'{}': could not measure the {g} in it", f.display()));
                }
            }
            None => {
                n.velocity = info.velocity.unwrap_or(-1.0);
                notes.push(n);
            }
        }
    }
    if notes.is_empty() {
        return Err("only gesture recordings (tremolo / vibrato / damp) were given: the instrument also needs plain notes".into());
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let gestures = Gestures {
        tremolo: (!tremolos.is_empty()).then(|| Tremolo {
            rate: mean(&tremolos.iter().map(|t: &Tremolo| t.rate).collect::<Vec<_>>()),
            alternate: mean(&tremolos.iter().map(|t| t.alternate).collect::<Vec<_>>()),
            accel: mean(&tremolos.iter().map(|t| t.accel).collect::<Vec<_>>()),
        }),
        vibrato: (!vibratos.is_empty()).then(|| Vibrato {
            depth: mean(&vibratos.iter().map(|v: &Vibrato| v.depth).collect::<Vec<_>>()),
            rate: mean(&vibratos.iter().map(|v| v.rate).collect::<Vec<_>>()),
            delay: mean(&vibratos.iter().map(|v| v.delay).collect::<Vec<_>>()),
        }),
        damp: (!damps.is_empty()).then(|| mean(&damps)),
    };
    // 文件名里没写力度的：同一个音高按响度从轻到重排开。
    let loud = |n: &NoteTable| n.partials.iter().map(|p| (p.amp + p.fast_amp).powi(2)).sum::<f64>();
    let mut groups: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut names: Vec<String> = notes.iter().map(|n| n.articulation.clone()).collect();
    names.sort();
    names.dedup();
    for (i, n) in notes.iter().enumerate() {
        if n.velocity < 0.0 {
            let a = names.iter().position(|x| *x == n.articulation).unwrap_or(0) as i64;
            groups.entry(n.pitch.round() as i64 * 1000 + a).or_default().push(i);
        }
    }
    for idx in groups.values_mut() {
        idx.sort_by(|&a, &b| loud(&notes[a]).total_cmp(&loud(&notes[b])));
        let k = idx.len();
        for (j, &i) in idx.iter().enumerate() {
            notes[i].velocity = if k == 1 { 0.8 } else { 0.35 + 0.6 * j as f64 / (k - 1) as f64 };
        }
    }
    notes.sort_by(|a, b| a.articulation.cmp(&b.articulation).then(a.pitch.total_cmp(&b.pitch)).then(a.velocity.total_cmp(&b.velocity)));
    // 其余弦的共振：余音里不在泛音上的能量越多，琴码带响的弦越响（经验换算，见 [`crate::zither::Profile`] 的 bridge）。
    // 中位数：只剩一两个分音的高音、量偏了的个别音不左右整体。
    let mut measured: Vec<f64> = notes.iter().filter(|n| n.articulation.is_empty()).map(|n| n.off_harmonic).filter(|&f| f > 0.0).collect();
    measured.sort_by(f64::total_cmp);
    let resonance = if measured.is_empty() {
        0.0
    } else {
        let f = measured[measured.len() / 2];
        (RESONANCE_SCALE * (f / (1.0 - f).max(0.05)).sqrt()).clamp(0.0, 0.3)
    };
    // 五位有效数字就够了（文件小一半多）。
    let r = |x: f64| {
        if x == 0.0 || !x.is_finite() {
            x
        } else {
            let k = 10f64.powi(4 - x.abs().log10().floor() as i32);
            (x * k).round() / k
        }
    };
    for n in &mut notes {
        n.pitch = r(n.pitch);
        n.off_harmonic = r(n.off_harmonic);
        for b in &mut n.bands {
            for v in [&mut b.hz, &mut b.amp, &mut b.t60, &mut b.tail, &mut b.tail_t60, &mut b.left, &mut b.right, &mut b.corr] {
                *v = r(*v);
            }
        }
        for p in &mut n.partials {
            for v in [
                &mut p.ratio,
                &mut p.amp,
                &mut p.t60,
                &mut p.fast_amp,
                &mut p.fast_t60,
                &mut p.beat,
                &mut p.beat_cents,
                &mut p.left,
                &mut p.right,
                &mut p.phase,
                &mut p.start,
            ] {
                *v = r(*v);
            }
        }
    }
    let gestures = Gestures {
        tremolo: gestures.tremolo.map(|t| Tremolo { rate: r(t.rate), alternate: r(t.alternate), accel: r(t.accel) }),
        vibrato: gestures.vibrato.map(|v| Vibrato { depth: r(v.depth), rate: r(v.rate), delay: r(v.delay) }),
        damp: gestures.damp.map(r),
    };
    Ok(ModalTable { format: FORMAT.into(), version: VERSION, name: name.into(), resonance: r(resonance), notes, gestures, release: None })
}

/// 一整条录音里找到的一个音：从第几秒起、多长、量到的音高（MIDI）、起音时多响（dBFS）；留下了（存成的文件）还是为什么没用。
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub source: String,
    pub start: f64,
    pub seconds: f64,
    pub pitch: Option<f64>,
    pub level_db: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<&'static str>,
}

/// 一段声音的音高（MIDI）和它有多像一个单音（泛音上的能量占 50 Hz 到第 8 次泛音这一段的多少）。
fn measure_pitch(x: &[f64], sr: f64) -> Option<(f64, f64)> {
    let a = ((0.02 * sr) as usize).min(x.len() / 4);
    let b = x.len().min(sr as usize);
    if b - a < (0.2 * sr) as usize {
        return None;
    }
    let size = next_pow2(4 * (b - a)).max(65_536);
    let (mag, bin) = spectrum(&x[a..b], size, sr);
    let mut f0 = fundamental(&mag, bin, None);
    // 低了一个八度的也凑得出一列泛音，但它的单数次泛音上什么都没有：那就是高一个八度的音。
    for _ in 0..2 {
        let power = |n: usize| peak_near(&mag, bin, f0 * n as f64, (0.03 * f0 * n as f64).max(2.0 * bin)).1.powi(2);
        let (odd, even) = ([1, 3, 5, 7].map(power).iter().sum::<f64>(), [2, 4, 6, 8].map(power).iter().sum::<f64>());
        if odd < 0.01 * even {
            f0 *= 2.0;
        }
    }
    let peaks: Vec<f64> = (1..=8).map(|n| peak_near(&mag, bin, f0 * n as f64, (0.03 * f0 * n as f64).max(2.0 * bin)).0).collect();
    let (mut all, mut on) = (0.0, 0.0);
    for (k, m) in mag.iter().enumerate().skip(1) {
        let f = k as f64 * bin;
        if !(50.0..8.5 * f0).contains(&f) {
            continue;
        }
        all += m * m;
        if peaks.iter().any(|p| (p - f).abs() < 4.0 + 0.01 * f) {
            on += m * m;
        }
    }
    (all > 0.0).then(|| (69.0 + 12.0 * (f0 / 440.0).log2(), on / all))
}

/// 把一整条录音（一次只弹一个音，一个接一个）切成一个音一个文件，存进 `dir`（文件名带量到的音高），交给 [`extract`]。
/// 只留干净的单音：起音之前上一个音已经收下去（比这个音低 20 dB 以上）、到下一个音之前至少响了半秒、
/// 听得出一个音高（不是和弦、杂音）。同一个音高弹了好几遍的，留最长的那几遍里轻、中、重各一个（响度差不到 3 dB 的算同一种力度）。
/// `on_segment` 每找到一个音调用一次（留下的和没用的都报）。
pub fn split_notes(inputs: &[PathBuf], dir: &Path, mut on_segment: impl FnMut(&Segment)) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    let names = ["C", "Cs", "D", "Ds", "E", "F", "Fs", "G", "Gs", "A", "As", "B"];
    let mut files = Vec::new();
    for (index, path) in inputs.iter().enumerate() {
        let file = manguebyte::formats::read_file(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        let sr = f64::from(file.sample_rate);
        let buf = &file.buffer;
        if buf.num_channels() == 0 || buf.num_frames() == 0 {
            return Err(format!("'{}' is empty", path.display()));
        }
        let last = buf.num_channels().min(2) - 1;
        let mid: Vec<f64> = buf.channel(0).iter().zip(buf.channel(last)).map(|(&a, &b)| 0.5 * (f64::from(a) + f64::from(b))).collect();
        let source = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // 每 5 ms 一帧的电平：整个声音的（往回 30 ms 的窗，先滤掉 40 Hz 以下：录音里的低频隆隆声在短窗里忽大忽小），
        // 和强调高频的（一阶差分，往回 10 ms 的窗：新的一拨在余音上面也看得出来）。
        let hop = ((0.005 * sr) as usize).max(1);
        let frames = mid.len() / hop;
        if frames < 40 {
            return Err(format!("'{}' is too short to split into notes", path.display()));
        }
        let k40 = 1.0 - (-TAU * 40.0 / sr).exp();
        let (mut low, mut prev) = (0.0, 0.0);
        let (mut e_full, mut e_high) = (vec![0.0; frames], vec![0.0; frames]);
        for (i, &v) in mid.iter().enumerate().take(frames * hop) {
            low += (v - low) * k40;
            e_full[i / hop] += (v - low).powi(2);
            e_high[i / hop] += (v - prev).powi(2);
            prev = v;
        }
        let level = |e: &[f64], back: usize| -> Vec<f64> {
            (0..frames).map(|k| 10.0 * (e[k.saturating_sub(back - 1)..=k].iter().sum::<f64>() / (back * hop) as f64).max(1e-14).log10()).collect()
        };
        let (full, high) = (level(&e_full, 6), level(&e_high, 2));
        // 150 Hz 以上各个三分之一倍频程频段的电平（往回 20 ms 的窗）：新的音落在差不多响的余音上，整体电平几乎不动，
        // 但它的分音所在的频段里会突然多出东西来。
        let window = next_pow2((0.02 * sr) as usize);
        let centers: Vec<f64> = (0..40).map(|i| 150.0 * 2f64.powf(i as f64 / 3.0)).take_while(|&c| c * 1.13 < 0.45 * sr).collect();
        let edges: Vec<(usize, usize)> =
            centers.iter().map(|&c| (((c / 1.122) * window as f64 / sr).floor() as usize, ((c * 1.122) * window as f64 / sr).ceil() as usize)).collect();
        let (taper, mut fft, mut spec, mut input) = (hann(window), RealFft::<f64>::new(window), vec![Complex::ZERO; window / 2 + 1], vec![0.0; window]);
        let mut bands: Vec<Vec<f64>> = vec![vec![-140.0; frames]; centers.len()];
        for k in (window / hop)..frames {
            let end = (k + 1) * hop;
            for (i, v) in input.iter_mut().enumerate() {
                *v = mid[end - window + i] * taper[i];
            }
            fft.forward(&input, &mut spec);
            for (band, &(lo, hi)) in bands.iter_mut().zip(&edges) {
                let e: f64 = spec[lo.max(1)..=hi.min(window / 2)].iter().map(|c| c.norm_sqr()).sum();
                band[k] = 10.0 * (e / (window * window) as f64).max(1e-14).log10();
            }
        }
        let band_top = bands.iter().flat_map(|b| b.iter().copied()).fold(f64::NEG_INFINITY, f64::max);
        let top = full.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top < -80.0 {
            return Err(format!("'{}' is silent", path.display()));
        }
        // 起音：15 ms 里比前 30 ms 的最低处高出一截，而且比前 150 ms 里最响的时候还响（整个声音升 6 dB、超出 3 dB；
        // 或高频升 10 dB、超出 6 dB；或哪一个频段里多出 10 dB）。余音里两根弦的拍从谷底回升得也快，
        // 但回不到之前那么响，不算。挨着的（80 ms 内）算一个。
        let mut onsets: Vec<usize> = Vec::new();
        for k in 0..frames.saturating_sub(8) {
            let struck = |l: &[f64], rise: f64, over: f64| {
                let low = l[k.saturating_sub(6)..=k].iter().copied().fold(f64::INFINITY, f64::min);
                let before = l[k.saturating_sub(30)..k].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let after = l[k + 1..k + 8].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (k == 0 || l[k + 3] - low >= rise) && after - before >= over
            };
            // 多出来的要站得住（之后 40 ms 的平均比之前 150 ms 里最响的还高 8 dB）：安静处一两帧的起伏不算。
            let appears = || {
                bands.iter().any(|l| {
                    let before = l[k.saturating_sub(30)..k].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let after = l[k + 2..(k + 10).min(frames)].iter().sum::<f64>() / ((k + 10).min(frames) - k - 2) as f64;
                    k > 0 && after - before >= 8.0 && after > band_top - 50.0
                })
            };
            if (struck(&full, 6.0, 3.0) || struck(&high, 10.0, 6.0) || appears()) && full[k + 3] > top - 45.0 && onsets.last().is_none_or(|&o| k > o + 16) {
                // 往回找到开始升的那一帧。
                let mut at = k;
                while at > 0 && at + 6 > k && full[at] > full[at - 1] {
                    at -= 1;
                }
                if onsets.last().is_none_or(|&o| at > o + 16) {
                    onsets.push(at);
                }
            }
        }
        struct Found {
            seg: Segment,
            from: usize,
            to: usize,
        }
        let mut found: Vec<Found> = Vec::new();
        for (i, &at) in onsets.iter().enumerate() {
            let from = (at * hop).saturating_sub((0.01 * sr) as usize);
            let next = onsets.get(i + 1).map_or(mid.len(), |&n| (n * hop).saturating_sub((0.01 * sr) as usize));
            let to = next.min(from + (ANALYSIS_SECONDS * sr) as usize);
            let peak = full[at..(at + 20).min(frames)].iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let before = if at >= 8 { full[at - 8..at - 1].iter().sum::<f64>() / 7.0 } else { -140.0 };
            let mut seg = Segment {
                source: source.clone(),
                start: from as f64 / sr,
                seconds: (to - from) as f64 / sr,
                pitch: None,
                level_db: peak,
                file: None,
                skipped: None,
            };
            if seg.seconds < 0.5 {
                seg.skipped = Some("shorter than 0.5 s before the next note");
            } else if before > peak - 20.0 {
                seg.skipped = Some("the note before it is still ringing (less than 20 dB below this one)");
            } else {
                match measure_pitch(&mid[from..to], sr) {
                    Some((p, clear)) if clear >= 0.4 && (20.0..=110.0).contains(&p) => seg.pitch = Some(p),
                    Some((p, _)) => {
                        seg.pitch = Some(p);
                        seg.skipped = Some("no single clear pitch (a chord, noise, or partials that are not harmonics)");
                    }
                    None => seg.skipped = Some("no pitch found"),
                }
            }
            found.push(Found { seg, from, to });
        }
        // 同一个音高的几遍：比最长的短一半以上的不要；剩下的按响度排，差不到 3 dB 的留长的；多于三个留轻、中、重。
        let key = |f: &Found| f.seg.pitch.map_or(0, |p| p.round() as i64);
        let mut pitches: Vec<i64> = found.iter().filter(|f| f.seg.skipped.is_none()).map(key).collect();
        pitches.sort_unstable();
        pitches.dedup();
        for p in pitches {
            let mut same: Vec<usize> = (0..found.len()).filter(|&i| found[i].seg.skipped.is_none() && key(&found[i]) == p).collect();
            let longest = same.iter().map(|&i| found[i].seg.seconds).fold(0.0, f64::max);
            let short: Vec<usize> = same.iter().copied().filter(|&i| found[i].seg.seconds < 0.5 * longest).collect();
            for i in short {
                found[i].seg.skipped = Some("much shorter than another take of this pitch");
            }
            same.retain(|&i| found[i].seg.skipped.is_none());
            same.sort_by(|&a, &b| found[a].seg.level_db.total_cmp(&found[b].seg.level_db));
            let mut kept: Vec<usize> = Vec::new();
            for i in same {
                match kept.last().copied() {
                    Some(j) if found[i].seg.level_db - found[j].seg.level_db < 3.0 => {
                        let (drop, keep) = if found[i].seg.seconds > found[j].seg.seconds { (j, i) } else { (i, j) };
                        found[drop].seg.skipped = Some("as loud as another take of this pitch (within 3 dB): the longer one is used");
                        if let Some(last) = kept.last_mut() {
                            *last = keep;
                        }
                    }
                    _ => kept.push(i),
                }
            }
            if kept.len() > 3 {
                for &i in &kept[1..kept.len() - 1] {
                    if i != kept[kept.len() / 2] {
                        found[i].seg.skipped = Some("more than three loudness levels of this pitch: the softest, middle and loudest are used");
                    }
                }
            }
        }
        for f in &mut found {
            if let (None, Some(p)) = (f.seg.skipped, f.seg.pitch) {
                let n = p.round() as i32;
                let name = format!("{}{}_f{}t{:.2}.wav", names[n.rem_euclid(12) as usize], n / 12 - 1, index + 1, f.seg.start);
                let len = f.to - f.from;
                let mut out = manguebyte::prelude::AudioBuffer::<f32>::new(2, len);
                // 末尾 5 ms 渐出（下一个音之前剪断的地方）。
                let fade = ((0.005 * sr) as usize).min(len);
                for (c, ch) in [0, last].into_iter().enumerate() {
                    let dst = out.channel_mut(c);
                    dst.copy_from_slice(&buf.channel(ch)[f.from..f.to]);
                    for k in 0..fade {
                        dst[len - 1 - k] *= k as f32 / fade as f32;
                    }
                }
                let file = dir.join(name);
                manguebyte::prelude::wav::write_file(&file, &out, file_rate(sr), manguebyte::prelude::WavFormat::Float32)
                    .map_err(|e| format!("cannot write '{}': {e:?}", file.display()))?;
                files.push(file.clone());
                f.seg.file = Some(file);
            }
            on_segment(&f.seg);
        }
    }
    if files.is_empty() {
        return Err("no clean single notes found: play one note at a time and let each ring before the next (see `muser yinse extract --help`)".into());
    }
    Ok(files)
}

fn file_rate(sr: f64) -> u32 {
    sr.round() as u32
}

/// 用 muser 的一个乐器（`sfz:` 音色库、`plugin:`、内置乐器）演奏每个音高、力度，各存成一个文件（文件名带音高、力度），
/// 只留按住的那 `hold` 秒（松开以后的不要）。再每个音高演奏一个 1 秒的短音，量松开后余音收掉的快慢（有制音器的乐器）。
pub fn render_notes(
    spec: &str,
    params: &muser_core::InstrumentParams,
    pitches: &[u8],
    velocities: &[f64],
    hold: f64,
    dir: &Path,
) -> Result<(Vec<PathBuf>, Option<f64>), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    let gap = hold + 4.0;
    let mut notes = Vec::new();
    for &p in pitches {
        for &v in velocities {
            notes.push((p, v, hold));
        }
        notes.push((p, velocities[velocities.len() / 2], 1.0));
    }
    let mut project = muser_core::Project::new("notes", 60.0);
    let mut track = muser_core::Track::new("t", spec);
    track.instrument_params = params.clone();
    let mut clip = muser_core::Clip::new(0.0, notes.len() as f64 * gap);
    for (i, &(p, v, len)) in notes.iter().enumerate() {
        let mut n = muser_core::Note::new(p, 0.5 + i as f64 * gap, len);
        n.velocity = v as f32;
        clip.notes.push(n);
    }
    track.clips.push(clip);
    project.tracks.push(track);
    let audio = crate::render::render(&project, crate::render::RenderSettings { tail_seconds: 0.5, ..Default::default() }).map_err(|e| e.to_string())?;
    let sr = 48_000.0;
    let names = ["C", "Cs", "D", "Ds", "E", "F", "Fs", "G", "Gs", "A", "As", "B"];
    let (mut files, mut releases) = (Vec::new(), Vec::new());
    for (i, &(p, v, len)) in notes.iter().enumerate() {
        let start = ((0.5 + i as f64 * gap) * sr) as usize;
        let short = len < hold;
        let end = (start + (if short { 3.0 } else { len } * sr) as usize).min(audio.num_frames());
        if start >= end {
            continue;
        }
        if short {
            let mid: Vec<f64> = audio.channel(0)[start..end].iter().zip(&audio.channel(1)[start..end]).map(|(a, b)| 0.5 * f64::from(a + b)).collect();
            if let Some(d) = analyze_damp(&mid, sr) {
                releases.push(d);
            }
            continue;
        }
        let mut b = manguebyte::prelude::AudioBuffer::<f32>::new(2, end - start);
        for c in 0..2 {
            b.channel_mut(c).copy_from_slice(&audio.channel(c)[start..end]);
        }
        let path = dir.join(format!("{}{}_v{v:.2}.wav", names[usize::from(p % 12)], i32::from(p / 12) - 1));
        manguebyte::prelude::wav::write_file(&path, &b, 48_000, manguebyte::prelude::WavFormat::Float32)
            .map_err(|e| format!("cannot write '{}': {e:?}", path.display()))?;
        files.push(path);
    }
    releases.sort_by(f64::total_cmp);
    // 一半以上的音松开后很快收掉，才算有制音器。
    let release = (releases.len() * 2 > pitches.len()).then(|| releases[releases.len() / 2]);
    Ok((files, release))
}

fn read_mid(path: &Path) -> Result<(Vec<f64>, f64), String> {
    let file = manguebyte::formats::read_file(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let b = &file.buffer;
    let last = b.num_channels().clamp(1, 2) - 1;
    let x: Vec<f64> = b.channel(0).iter().zip(b.channel(last)).map(|(&l, &r)| 0.5 * (f64::from(l) + f64::from(r))).collect();
    Ok((x, f64::from(file.sample_rate)))
}

/// 轮音：起音检测（高频能量的突增），算每秒几下、单双数两下的力度比、结尾比开头快多少。
fn analyze_tremolo(x: &[f64], sr: f64) -> Option<Tremolo> {
    let hop = (0.001 * sr) as usize;
    let w = (0.004 * sr) as usize;
    // 一阶差分强调起音；每 1 ms 一帧的能量。前面几下还在响，所以看的是比 5 ms 前多出来多少（dB）。
    let d: Vec<f64> = x.windows(2).map(|p| p[1] - p[0]).collect();
    let e: Vec<f64> = (0..d.len().saturating_sub(w)).step_by(hop).map(|i| d[i..i + w].iter().map(|v| v * v).sum::<f64>()).collect();
    let top = e.iter().copied().fold(0.0, f64::max);
    let rise: Vec<f64> = (0..e.len()).map(|i| if i < 5 || e[i] < top * 1e-4 { 0.0 } else { 10.0 * (e[i] / e[i - 5].max(1e-30)).log10() }).collect();
    let gap = (0.03 * sr / hop as f64) as usize;
    let mut onsets: Vec<(usize, f64)> = Vec::new();
    for i in 1..rise.len().saturating_sub(1) {
        if rise[i] > 1.0 && rise[i] >= rise[i - 1] && rise[i] >= rise[i + 1] {
            // 这一下有多响：比 5 ms 前多出来的能量。
            let gain = (e[(i + 3).min(e.len() - 1)] - e[i - 5]).max(0.0);
            match onsets.last_mut() {
                Some(last) if i - last.0 < gap => {
                    if gain > last.1 {
                        *last = (i, gain);
                    }
                }
                _ => onsets.push((i, gain)),
            }
        }
    }
    if onsets.len() < 6 {
        return None;
    }
    let ms = |k: usize| (onsets[k + 1].0 - onsets[k].0) as f64 * hop as f64 / sr;
    let mut gaps: Vec<f64> = (0..onsets.len() - 1).map(ms).collect();
    let third = gaps.len() / 3;
    let rate_of = |g: &[f64]| {
        let mut g = g.to_vec();
        g.sort_by(f64::total_cmp);
        1.0 / g[g.len() / 2].max(1e-3)
    };
    let accel = if third >= 2 { rate_of(&gaps[gaps.len() - third..]) / rate_of(&gaps[..third]) } else { 1.0 };
    gaps.sort_by(f64::total_cmp);
    let rate = 1.0 / gaps[gaps.len() / 2];
    let mut ratios: Vec<f64> = onsets.as_chunks::<2>().0.iter().map(|p| (p[1].1 / p[0].1.max(1e-30)).sqrt()).collect();
    ratios.sort_by(f64::total_cmp);
    let alternate = ratios[ratios.len() / 2].clamp(0.3, 3.0);
    Some(Tremolo { rate: rate.clamp(2.0, 30.0), alternate, accel: accel.clamp(0.5, 2.0) })
}

/// 揉音：最响的一个低分音的瞬时频率（相邻两帧解调相位之差），算起伏的深浅、快慢和从哪里开始。
fn analyze_vibrato(x: &[f64], sr: f64, f0: f64, partials: &[Partial]) -> Option<Vibrato> {
    let p = partials.iter().take(4).max_by(|a, b| a.amp.total_cmp(&b.amp))?;
    let hz = f0 * p.ratio;
    let hop = (0.005 * sr) as usize;
    let window = ((3.0 / f0).max(0.03) * sr) as usize;
    let on = x.iter().position(|v| v.abs() > 0.05 * x.iter().fold(0.0f64, |m, v| m.max(v.abs()))).unwrap_or(0);
    let end = (on + (4.0 * sr) as usize).min(x.len());
    let z = demodulate(&x[on..end], hz, window, hop, sr);
    if z.len() < 60 {
        return None;
    }
    let dt = hop as f64 / sr;
    let cents: Vec<f64> = z
        .windows(2)
        .map(|w| {
            let d = (w[1].im * w[0].re - w[1].re * w[0].im).atan2(w[1].re * w[0].re + w[1].im * w[0].im);
            1200.0 * (1.0 + d / (TAU * dt) / hz).max(0.5).log2()
        })
        .collect();
    let mean = cents.iter().sum::<f64>() / cents.len() as f64;
    let dev: Vec<f64> = cents.iter().map(|c| c - mean).collect();
    // 起伏的快慢：2 … 12 Hz 里最强的。
    let t: Vec<f64> = (0..dev.len()).map(|k| k as f64 * dt).collect();
    let mut best = (0.0, 0.0);
    let mut f = 2.0;
    while f <= 12.0 {
        let (re, im) = dev.iter().zip(&t).fold((0.0, 0.0), |(a, b), (&y, &x)| (a + y * (TAU * f * x).cos(), b + y * (TAU * f * x).sin()));
        let a = (re * re + im * im).sqrt();
        if a > best.0 {
            best = (a, f);
        }
        f += 0.05;
    }
    let rate = best.1;
    // 深浅：一个周期一个窗的峰峰值的一半；从第一个超过一半深度的窗开始算 delay。
    let per = ((1.0 / rate) / dt) as usize;
    let swings: Vec<f64> =
        dev.chunks(per.max(2)).map(|c| (c.iter().copied().fold(f64::MIN, f64::max) - c.iter().copied().fold(f64::MAX, f64::min)) / 2.0).collect();
    let mut sorted = swings.clone();
    sorted.sort_by(f64::total_cmp);
    let depth = sorted[sorted.len() * 3 / 4];
    if depth < 3.0 {
        return None;
    }
    let delay = swings.iter().position(|&s| s > depth / 2.0).unwrap_or(0) as f64 * per as f64 * dt;
    Some(Vibrato { depth: depth.clamp(1.0, 100.0), rate, delay: delay.clamp(0.0, 4.0) })
}

/// 止音：余音里最陡的一段（50 ms 一段）的衰减速度换成到 -60 dB 的秒数。
fn analyze_damp(x: &[f64], sr: f64) -> Option<f64> {
    let frame = (0.01 * sr) as usize;
    let db: Vec<f64> = x.chunks(frame).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).max(1e-20).log10()).collect();
    let peak = (0..db.len().min(10)).max_by(|&a, &b| db[a].total_cmp(&db[b]))?;
    let top = db[peak];
    // 0.1 秒以后、还没掉进底噪的范围里，相隔 50 ms 掉得最多的地方。
    let (mut best, mut at) = (0.0, None);
    for k in (peak + 10)..db.len().saturating_sub(5) {
        let drop = db[k] - db[k + 5];
        if db[k] > top - 50.0 && drop > best {
            best = drop;
            at = Some(k);
        }
    }
    at?;
    let per_second = best / 0.05;
    (per_second > 40.0).then(|| (60.0 / per_second).clamp(0.01, 1.0))
}

/// 余音里不在泛音上的能量比例 → 琴码带响的弦的振幅（用 Kontakt 扬琴校准过）。
const RESONANCE_SCALE: f64 = 0.25;

// ---------------------------------------------------------------------------
// 演奏：在相邻的音高、力度之间插值
// ---------------------------------------------------------------------------

/// 一个要演奏的音的分音与噪声。
#[derive(Debug, Clone, Default)]
pub struct Voice {
    pub partials: Vec<Partial>,
    pub noise: Noise,
    pub bands: Vec<NoiseBand>,
}

impl ModalTable {
    /// 这件乐器有的演奏法：音色类（有自己的录音）和动作类（量到了的）。普通的敲法不列。
    pub fn articulations(&self) -> Vec<String> {
        let mut out: Vec<String> = self.notes.iter().map(|n| n.articulation.clone()).filter(|a| !a.is_empty()).collect();
        out.sort();
        out.dedup();
        let g = &self.gestures;
        for (name, has) in [("tremolo", g.tremolo.is_some()), ("vibrato", g.vibrato.is_some()), ("damp", g.damp.is_some())] {
            if has && !out.iter().any(|a| a == name) {
                out.push(name.into());
            }
        }
        out
    }

    /// 音高范围（MIDI）。
    pub fn range(&self) -> (f64, f64) {
        let lo = self.notes.iter().map(|n| n.pitch).fold(f64::INFINITY, f64::min);
        let hi = self.notes.iter().map(|n| n.pitch).fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    }

    /// 这个音高、力度上的分音：音高上取相邻的两个录音音高，各自在相邻的两个力度之间插值。
    pub fn voice(&self, pitch: f64, velocity: f64, articulation: &str) -> Voice {
        // 这个演奏法没有录音（或是动作类的）就用普通的敲法。
        let art = if self.notes.iter().any(|n| n.articulation == articulation) { articulation } else { "" };
        let art = if self.notes.iter().any(|n| n.articulation == art) { art } else { self.notes[0].articulation.as_str() };
        let mut pitches: Vec<f64> = self.notes.iter().filter(|n| n.articulation == art).map(|n| n.pitch).collect();
        pitches.sort_by(f64::total_cmp);
        pitches.dedup_by(|a, b| (*a - *b).abs() < 0.3);
        let mut weights: Vec<(usize, f64, f64)> = Vec::new(); // (note, weight, 振幅倍数)
        let (pa, pb, wb) = match pitches.iter().position(|&p| p >= pitch) {
            None => (pitches[pitches.len() - 1], pitches[pitches.len() - 1], 0.0),
            Some(0) => (pitches[0], pitches[0], 0.0),
            Some(i) => {
                let (a, b) = (pitches[i - 1], pitches[i]);
                (a, b, ((pitch - a) / (b - a).max(1e-9)).clamp(0.0, 1.0))
            }
        };
        for (p, w) in [(pa, 1.0 - wb), (pb, wb)] {
            if w <= 0.0 {
                continue;
            }
            let mut layer: Vec<usize> = (0..self.notes.len()).filter(|&i| self.notes[i].articulation == art && (self.notes[i].pitch - p).abs() < 0.3).collect();
            layer.sort_by(|&a, &b| self.notes[a].velocity.total_cmp(&self.notes[b].velocity));
            let v = |i: usize| self.notes[layer[i]].velocity;
            match layer.iter().position(|&i| self.notes[i].velocity >= velocity) {
                // 比最响的还响 / 比最轻的还轻：用那一层，振幅按力度比例放大缩小。
                None => weights.push((layer[layer.len() - 1], w, velocity / v(layer.len() - 1).max(0.05))),
                Some(0) => weights.push((layer[0], w, velocity / v(0).max(0.05))),
                Some(i) => {
                    let x = ((velocity - v(i - 1)) / (v(i) - v(i - 1)).max(1e-9)).clamp(0.0, 1.0);
                    weights.push((layer[i - 1], w * (1.0 - x), 1.0));
                    weights.push((layer[i], w * x, 1.0));
                }
            }
        }
        // 把几个录音的分音对齐：有泛音序号的按序号（哪个录音漏了一个也不会错位），没有序号的（不在泛音列上的分音、
        // 版本 1 的表）按频率比最近的对。最响的录音先排。
        let mut order: Vec<usize> = (0..weights.len()).collect();
        order.sort_by(|&a, &b| weights[b].1.total_cmp(&weights[a].1));
        let mut slots: Vec<(u32, f64, Vec<(usize, &Partial)>)> = Vec::new();
        for &wi in &order {
            for p in &self.notes[weights[wi].0].partials {
                let near = (0.02 * p.ratio).clamp(0.03, 0.45);
                let hit = slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.2.iter().any(|q| q.0 == wi))
                    .filter(|(_, s)| if p.n > 0 { s.0 == p.n } else { s.0 == 0 && (s.1 - p.ratio).abs() <= near })
                    .min_by(|a, b| (a.1 .1 - p.ratio).abs().total_cmp(&(b.1 .1 - p.ratio).abs()))
                    .map(|(k, _)| k);
                match hit {
                    Some(k) => slots[k].2.push((wi, p)),
                    None => slots.push((p.n, p.ratio, vec![(wi, p)])),
                }
            }
        }
        let mut partials = Vec::with_capacity(slots.len());
        for (n, _, parts) in &slots {
            let present: Vec<(&Partial, f64, f64)> = parts.iter().map(|&(wi, p)| (p, weights[wi].1, weights[wi].2)).collect();
            let wsum: f64 = present.iter().map(|&(_, w, _)| w).sum();
            if wsum <= 0.0 {
                continue;
            }
            let avg = |f: &dyn Fn(&Partial) -> f64| present.iter().map(|&(p, w, _)| f(p) * w).sum::<f64>() / wsum;
            let geo = |f: &dyn Fn(&Partial) -> f64| (present.iter().map(|&(p, w, _)| f(p).max(1e-9).ln() * w).sum::<f64>() / wsum).exp();
            let (pr, pi) = present.iter().fold((0.0, 0.0), |(a, b), &(p, w, _)| (a + p.phase.cos() * w, b + p.phase.sin() * w));
            let (sr, si) = present
                .iter()
                .fold((0.0, 0.0), |(a, b), &(p, w, _)| (a + p.start.cos() * w * (p.amp + p.fast_amp), b + p.start.sin() * w * (p.amp + p.fast_amp)));
            partials.push(Partial {
                ratio: avg(&|p| p.ratio),
                // 振幅按全部权重加（缺这个分音的录音算 0），其余按有这个分音的录音平均。
                amp: present.iter().map(|&(p, w, g)| p.amp * w * g).sum(),
                t60: geo(&|p| p.t60),
                fast_amp: present.iter().map(|&(p, w, g)| p.fast_amp * w * g).sum(),
                fast_t60: geo(&|p| p.fast_t60.max(0.01)),
                beat: avg(&|p| p.beat),
                beat_cents: avg(&|p| p.beat_cents),
                left: avg(&|p| p.left),
                right: avg(&|p| p.right),
                phase: pi.atan2(pr),
                n: *n,
                start: if sr == 0.0 && si == 0.0 { 0.0 } else { si.atan2(sr) },
            });
        }
        // 对出来比能演奏的多：留最响的。
        if partials.len() > MAX_PARTIALS {
            partials.sort_by(|a, b| (b.amp + b.fast_amp).total_cmp(&(a.amp + a.fast_amp)));
            partials.truncate(MAX_PARTIALS);
        }
        partials.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
        let noise = Noise {
            amp: weights.iter().map(|&(i, w, g)| self.notes[i].noise.amp * w * g).sum(),
            hz: (weights.iter().map(|&(i, w, _)| self.notes[i].noise.hz.max(1.0).ln() * w).sum::<f64>() / weights.iter().map(|w| w.1).sum::<f64>().max(1e-9))
                .exp(),
        };
        // 噪声的频段：各录音同一个频段的加权（没有这个频段的录音算 0）。
        let mut bands = Vec::new();
        for k in 0..NOISE_BANDS {
            let hz = 31.25 * (k as f64).exp2();
            let present: Vec<(&NoiseBand, f64, f64)> =
                weights.iter().filter_map(|&(i, w, g)| self.notes[i].bands.iter().find(|b| (b.hz / hz - 1.0).abs() < 0.02).map(|b| (b, w, g))).collect();
            let wsum: f64 = present.iter().map(|p| p.1).sum();
            if wsum <= 0.0 {
                continue;
            }
            let avg = |f: &dyn Fn(&NoiseBand) -> f64| present.iter().map(|&(b, w, _)| f(b) * w).sum::<f64>() / wsum;
            let geo = |f: &dyn Fn(&NoiseBand) -> f64, has: &dyn Fn(&NoiseBand) -> bool| {
                let w: f64 = present.iter().filter(|p| has(p.0)).map(|p| p.1).sum();
                if w <= 0.0 {
                    0.0
                } else {
                    (present.iter().filter(|p| has(p.0)).map(|&(b, w, _)| f(b).max(1e-9).ln() * w).sum::<f64>() / w).exp()
                }
            };
            bands.push(NoiseBand {
                hz,
                amp: present.iter().map(|&(b, w, g)| b.amp * w * g).sum(),
                t60: geo(&|b| b.t60, &|_| true),
                tail: present.iter().map(|&(b, w, g)| b.tail * w * g).sum(),
                tail_t60: geo(&|b| b.tail_t60, &|b| b.tail > 0.0),
                left: avg(&|b| b.left),
                right: avg(&|b| b.right),
                corr: avg(&|b| b.corr),
            });
        }
        Voice { partials, noise, bands }
    }
}

// ---------------------------------------------------------------------------
// 读文件（按修改时间缓存）
// ---------------------------------------------------------------------------

type Cached = (std::time::SystemTime, &'static ModalTable, &'static crate::zither::Profile);
static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// 读一件乐器（`yinse:` 后面的路径，`~/` 开头的展开到家目录），同一文件没改过就不再读。
/// shortcut: 每读一次新版本就留下一份（&'static），改文件很频繁的长会话会占一点内存，需要时再改成引用计数。
pub(crate) fn load(path: &str) -> Result<(&'static ModalTable, &'static crate::zither::Profile), String> {
    let path = resolve(path);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|e| format!("cannot read yinse instrument '{}': {e}", path.display()))?;
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(&(t, table, profile)) = cache.get(&path) {
        if t == modified {
            return Ok((table, profile));
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read yinse instrument '{}': {e}", path.display()))?;
    let table: ModalTable = serde_json::from_str(&text).map_err(|e| format!("'{}' is not a yinse instrument: {e}", path.display()))?;
    if table.format == FORMAT && table.version > VERSION {
        return Err(format!(
            "'{}' was extracted by a newer muser (table version {}, this muser reads up to {VERSION}): update muser",
            path.display(),
            table.version
        ));
    }
    if table.format != FORMAT || table.notes.is_empty() || table.notes.iter().any(|n| n.partials.is_empty()) {
        return Err(format!("'{}' is not a yinse instrument (format {FORMAT}, at least one note with partials)", path.display()));
    }
    let table: &'static ModalTable = Box::leak(Box::new(table));
    let profile = crate::zither::table_profile(table);
    cache.insert(path, (modified, table, profile));
    Ok((table, profile))
}

/// `$MUSER_YINSE_PATH` 音色库里的一件乐器（muser-yinse 的目录：`instruments/<类>/<名>/<名>.yinse.json` 和旁边的 `meta.json`）。
#[derive(Debug, Clone, Serialize)]
pub struct LibraryEntry {
    /// 写进音轨 `instrument` 的值：`yinse:<类>/<名>`。
    pub spec: String,
    pub name: String,
    /// strike / pluck。
    pub family: String,
    pub description: String,
    /// 录音的许可证（SPDX）与来源，`meta.json` 里没写就是空的。
    pub license: String,
    pub source: String,
    pub path: PathBuf,
}

/// 本机的 yinse 音色库（`muser yinse install` 装到这里）：[`crate::profile::config_dir`] 下的 `yinse`。
pub fn user_library() -> PathBuf {
    crate::profile::config_dir().join("yinse")
}

/// 找乐器的库：先 `$MUSER_YINSE_PATH` 里的（路径分隔符隔开），最后是本机的 [`user_library`]。
pub fn roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> =
        std::env::var_os("MUSER_YINSE_PATH").map(|v| std::env::split_paths(&v).filter(|r| !r.as_os_str().is_empty()).collect()).unwrap_or_default();
    roots.push(user_library());
    roots
}

/// 列出所有库的乐器（按写法排序；同名的只取前面的库）。
pub fn library() -> Vec<LibraryEntry> {
    let roots = roots();
    let mut out: Vec<LibraryEntry> = Vec::new();
    let dirs = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        v
    };
    for root in roots.iter().filter(|r| !r.as_os_str().is_empty()) {
        for family in dirs(&root.join("instruments")) {
            for dir in dirs(&family) {
                let (f, n) =
                    (family.file_name().unwrap_or_default().to_string_lossy().into_owned(), dir.file_name().unwrap_or_default().to_string_lossy().into_owned());
                let path = dir.join(format!("{n}.yinse.json"));
                let spec = format!("yinse:{f}/{n}");
                if !path.is_file() || out.iter().any(|e| e.spec == spec) {
                    continue;
                }
                let meta: serde_json::Value =
                    std::fs::read_to_string(dir.join("meta.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
                let zh = text(&meta["zh"]);
                out.push(LibraryEntry {
                    spec,
                    name: if zh.is_empty() { n.clone() } else { format!("{n} ({zh})") },
                    family: f,
                    description: text(&meta["description"]),
                    license: text(&meta["source"]["license"]),
                    source: [text(&meta["source"]["title"]), text(&meta["source"]["author"])]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", "),
                    path,
                });
            }
        }
    }
    out.sort_by(|a, b| a.spec.cmp(&b.spec));
    out
}

/// `yinse:<文件>` 这条音轨的表（不是录音提取的乐器、读不到都是 None）。
/// 内置古琴（`yinse.pluck.guqin`）的分音表：从古琴独奏录音里挑出来的单音量的（散音、按音一组，泛音一组）。
/// shortcut: 选定录音之前可用环境变量 MUSER_FIT_GUQIN（一个 .yinse.json）换一张表试听，定下来就删掉。
pub(crate) fn builtin_guqin() -> &'static ModalTable {
    static TABLE: std::sync::OnceLock<ModalTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let text = std::env::var_os("MUSER_FIT_GUQIN").and_then(|p| std::fs::read_to_string(p).ok());
        let text = text.as_deref().unwrap_or(include_str!("data/guqin.yinse.json"));
        serde_json::from_str(text).expect("valid built-in guqin table")
    })
}

/// 内置古筝（`yinse.pluck.guzheng`）的分音表：从古筝独奏录音里挑出来的单音量的。
/// shortcut: 选定录音之前可用环境变量 MUSER_FIT_GUZHENG（一个 .yinse.json）换一张表试听，定下来就删掉。
pub(crate) fn builtin_guzheng() -> &'static ModalTable {
    static TABLE: std::sync::OnceLock<ModalTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let text = std::env::var_os("MUSER_FIT_GUZHENG").and_then(|p| std::fs::read_to_string(p).ok());
        let text = text.as_deref().unwrap_or(include_str!("data/guzheng.yinse.json"));
        serde_json::from_str(text).expect("valid built-in guzheng table")
    })
}

pub(crate) fn table_of(instrument: &str) -> Option<&'static ModalTable> {
    load(instrument.trim().strip_prefix("yinse:")?).ok().map(|(t, _)| t)
}

/// 音符的演奏法 → 交给乐器的编号（0 普通、动作类也是 0，音色类是在 [`ModalTable::articulations`] 里的位置 + 1）。
pub(crate) fn articulation_index(table: &ModalTable, name: &str) -> u8 {
    if GESTURES.contains(&name) && !table.notes.iter().any(|n| n.articulation == name) {
        return 0;
    }
    table.articulations().iter().position(|a| a == name).map_or(0, |i| (i + 1) as u8)
}

/// `yinse:` 后面的路径 → 文件。存在的文件（`~/` 展开）原样用；否则在每个库（[`roots`]）里找
/// `库/路径`、`库/路径.yinse.json`、`库/instruments/<类>/<名>/<名>.yinse.json`（`yinse:strike/yangqin`，muser-yinse 的目录）。
/// 都找不到时原样返回（由调用方报"找不到"）。
pub fn resolve(path: &str) -> PathBuf {
    let path = path.trim();
    let direct = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    };
    if direct.is_absolute() || direct.is_file() {
        return direct;
    }
    let roots = roots();
    let last = direct.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for root in roots.iter().filter(|r| !r.as_os_str().is_empty()) {
        let candidates = [
            root.join(&direct),
            root.join(format!("{path}.yinse.json")),
            root.join("instruments").join(&direct).join(format!("{last}.yinse.json")),
            root.join(&direct).join(format!("{last}.yinse.json")),
        ];
        if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
            return found;
        }
    }
    direct
}

// ---------------------------------------------------------------------------
// 对比：同样的音高、力度，提取出的乐器和原录音差多少
// ---------------------------------------------------------------------------

/// 一个音的对比结果。
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub source: String,
    pub pitch: f64,
    pub velocity: f64,
    /// 三分之一倍频程、20 ms 一帧，前 2 秒里各格的平均差（dB，两边先拉到一样响）。
    pub spectral_db: f64,
    /// 前 2 秒的响度：提取的比原录音响多少（dB，不拉平）。
    pub level_db: f64,
    /// 把这个音高的录音从乐器里拿掉、用两边相邻的音高插值出来，再和这个录音比的频谱差（dB）：
    /// 没录的音高演奏起来差多少。没让算、或者它是最低最高的音高，就没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between_db: Option<f64>,
    /// 0.1 / 0.5 / 1 / 2 秒时比峰值低多少（dB）：原录音、提取的。
    pub decay_ref: [f64; 4],
    pub decay_ours: [f64; 4],
    /// 左右声道的相关度：原录音、提取的。
    pub stereo_ref: f64,
    pub stereo_ours: f64,
}

fn read_stereo(path: &Path, sr: u32) -> Result<[Vec<f64>; 2], String> {
    let f = manguebyte::formats::read_file(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let buf = if f.sample_rate == sr { f.buffer } else { manguebyte::dsp::resample(&f.buffer, f64::from(f.sample_rate), f64::from(sr)) };
    let last = buf.num_channels().clamp(1, 2) - 1;
    Ok([buf.channel(0).iter().map(|&v| f64::from(v)).collect(), buf.channel(last).iter().map(|&v| f64::from(v)).collect()])
}

fn onset_of(x: &[f64]) -> usize {
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    x.iter().position(|v| v.abs() > 0.05 * peak).unwrap_or(0)
}

/// 起音起 2 秒：各帧各频段的电平（dB，先按均方根拉平）、衰减、左右相关度、均方根。
fn features(lr: &[Vec<f64>; 2], sr: f64) -> (Vec<Vec<f64>>, [f64; 4], f64, f64) {
    let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
    let on = onset_of(&mid).saturating_sub((0.002 * sr) as usize);
    let n = ((2.1 * sr) as usize).min(mid.len().saturating_sub(on));
    let x = &mid[on..on + n];
    let rms = (x.iter().map(|v| v * v).sum::<f64>() / n.max(1) as f64).sqrt().max(1e-12);
    let frame = (0.02 * sr) as usize;
    let size = next_pow2(frame * 2);
    let bands: Vec<f64> = (0..28).map(|i| 50.0 * 2f64.powf(i as f64 / 3.0)).filter(|&f| f < 0.45 * sr).collect();
    let mut grid: Vec<Vec<f64>> = Vec::new();
    let mut s = 0;
    while s + frame <= n {
        let seg: Vec<f64> = x[s..s + frame].iter().map(|v| v / rms).collect();
        let (mag, bin) = spectrum(&seg, size, sr);
        grid.push(
            bands
                .iter()
                .map(|&c| {
                    let (lo, hi) = ((c / 2f64.powf(1.0 / 6.0) / bin) as usize, ((c * 2f64.powf(1.0 / 6.0) / bin) as usize).min(mag.len() - 1));
                    let e: f64 = mag[lo..=hi.max(lo)].iter().map(|m| m * m).sum();
                    10.0 * e.max(1e-12).log10()
                })
                .collect(),
        );
        s += frame;
    }
    let env = |t: f64| {
        let k = (t * sr) as usize;
        let w = (0.02 * sr) as usize;
        (k + w <= x.len()).then(|| (x[k..k + w].iter().map(|v| v * v).sum::<f64>() / w as f64).sqrt())
    };
    // 各格比全段最响的一格低 60 dB 以下的都算 -60（录音底噪、数字静音不比）。
    let top = grid.iter().flat_map(|row: &Vec<f64>| row.iter().copied()).fold(f64::NEG_INFINITY, f64::max);
    for row in &mut grid {
        row.iter_mut().for_each(|v| *v = (*v - top).max(-60.0));
    }
    let peak = (0..10).filter_map(|i| env(i as f64 * 0.005)).fold(1e-12, f64::max);
    let decay = [0.1, 0.5, 1.0, 2.0].map(|t| env(t).map_or(f64::NAN, |v| 20.0 * (v / peak).max(1e-9).log10()));
    let (a, b) = (&lr[0][on..on + n], &lr[1][on..on + n]);
    let (ma, mb) = (a.iter().sum::<f64>() / n as f64, b.iter().sum::<f64>() / n as f64);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    (grid, decay, sab / (saa * sbb).sqrt().max(1e-12), rms)
}

/// 把每个原录音在提取的乐器上用同样的音高、力度（和演奏法）演奏一遍，比较。
/// `between`：再把每个音高的录音拿掉、让相邻的音高插值出它来比一遍（看没录的音高差多少）。
pub fn compare(table_path: &str, files: &[PathBuf], between: bool) -> Result<Vec<Comparison>, String> {
    let (table, _) = load(table_path)?;
    let spec = format!("yinse:{}", resolve(table_path).display());
    let sr = 48_000u32;
    let play = |spec: &str, pitch: f64, velocity: f64, articulation: &str| -> Result<[Vec<f64>; 2], String> {
        let mut project = muser_core::Project::new("compare", 60.0);
        let mut track = muser_core::Track::new("t", spec);
        // 居中的音轨两声道各 -3 dB（等功率声像）：补回来，响度才能直接和录音比。
        track.gain_db = 20.0 * core::f64::consts::SQRT_2.log10();
        let mut clip = muser_core::Clip::new(0.0, 3.0);
        let mut note = muser_core::Note::new(pitch.round().clamp(0.0, 127.0) as u8, 0.0, 2.5);
        note.velocity = velocity as f32;
        note.articulation = (!articulation.is_empty()).then(|| articulation.to_string());
        clip.notes.push(note);
        track.clips.push(clip);
        project.tracks.push(track);
        let audio = crate::render::render(&project, crate::render::RenderSettings { tail_seconds: 1.0, ..Default::default() }).map_err(|e| e.to_string())?;
        Ok([audio.channel(0).iter().map(|&v| f64::from(v)).collect(), audio.channel(1).iter().map(|&v| f64::from(v)).collect()])
    };
    let difference = |a: &[Vec<f64>], b: &[Vec<f64>]| {
        let cells: Vec<f64> = a.iter().zip(b).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())).collect();
        cells.iter().sum::<f64>() / cells.len().max(1) as f64
    };
    let dir = std::env::temp_dir().join(format!("muser-yinse-between-{}", std::process::id()));
    let mut out = Vec::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let known = table.notes.iter().find(|n| n.source == name);
        let info = parse_name(&stem);
        // 动作的录音（轮音、揉音、止音）不是单音：不比。
        if info.articulation.as_deref().is_some_and(|a| GESTURES.contains(&a)) {
            continue;
        }
        let (p, v) = (info.pitch, info.velocity);
        let pitch = p.or(known.map(|n| n.pitch.round())).ok_or_else(|| format!("'{name}': no pitch in the file name and not one of the instrument's notes"))?;
        let velocity = v.or(known.map(|n| n.velocity)).unwrap_or(0.8);
        let articulation = info.articulation.clone().or(known.map(|n| n.articulation.clone())).unwrap_or_default();
        let reference = read_stereo(f, sr)?;
        let ours = play(&spec, pitch, velocity, &articulation)?;
        let (ga, da, sa, la) = features(&reference, f64::from(sr));
        let (gb, db, sb, lb) = features(&ours, f64::from(sr));
        let mut between_db = None;
        let same = |n: &&NoteTable| n.articulation == articulation;
        if between && table.notes.iter().filter(same).any(|n| n.pitch < pitch - 0.5) && table.notes.iter().filter(same).any(|n| n.pitch > pitch + 0.5) {
            let file = dir.join(format!("{}-{articulation}.yinse.json", pitch.round()));
            if !file.is_file() {
                let mut without = table.clone();
                without.notes.retain(|n| n.articulation != articulation || (n.pitch - pitch).abs() > 0.5);
                std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
                std::fs::write(&file, serde_json::to_string(&without).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("cannot write '{}': {e}", file.display()))?;
            }
            let (gc, ..) = features(&play(&format!("yinse:{}", file.display()), pitch, velocity, &articulation)?, f64::from(sr));
            between_db = Some(difference(&ga, &gc));
        }
        out.push(Comparison {
            source: name,
            pitch,
            velocity,
            spectral_db: difference(&ga, &gb),
            level_db: 20.0 * (lb / la).log10(),
            between_db,
            decay_ref: da,
            decay_ours: db,
            stereo_ref: sa,
            stereo_ours: sb,
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_give_pitch_and_velocity() {
        let pv = |s: &str| {
            let i = parse_name(s);
            (i.pitch, i.velocity, i.articulation)
        };
        assert_eq!(pv("Yangqin_A4_v100"), (Some(69.0), Some(100.0 / 127.0), None));
        assert_eq!(pv("eb3-mf"), (Some(51.0), Some(0.7), None));
        assert_eq!(pv("piano 60 f"), (Some(60.0), Some(0.85), None));
        assert_eq!(pv("C#5_v0.5_muted"), (Some(73.0), Some(0.5), Some("mute".into())));
        assert_eq!(pv("D4 tremolo"), (Some(62.0), None, Some("tremolo".into())));
        assert_eq!(pv("A3_art-fanzhu_v90"), (Some(57.0), Some(90.0 / 127.0), Some("fanzhu".into())));
        assert_eq!(pv("take2"), (None, None, None));
    }

    /// 合成一个已知的音（两个分音、两段衰减、左右不同），提取后应能读回这些数。
    #[test]
    fn a_synthetic_note_is_measured_back() {
        let sr = 48_000.0;
        let f0 = 220.0;
        let n = (3.0 * sr) as usize;
        let mut l = vec![0.0f32; n + 480];
        let mut r = vec![0.0f32; n + 480];
        for i in 0..n {
            let t = i as f64 / sr;
            let p1 = 0.3 * 10f64.powf(-3.0 * t / 2.0) + 0.3 * 10f64.powf(-3.0 * t / 0.1);
            let p2 = 0.1 * 10f64.powf(-3.0 * t / 1.0);
            let s1 = (TAU * f0 * t).sin();
            l[i + 480] = (p1 * s1 + p2 * (TAU * 2.01 * f0 * t).sin()) as f32 * 1.2;
            r[i + 480] = (p1 * (TAU * f0 * t - 0.5).sin() + p2 * (TAU * 2.01 * f0 * t).sin()) as f32 * 0.8;
        }
        let dir = std::env::temp_dir().join(format!("muser-modal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("A3_v90.wav");
        let mut buf = manguebyte::prelude::AudioBuffer::<f32>::new(2, l.len());
        buf.channel_mut(0).copy_from_slice(&l);
        buf.channel_mut(1).copy_from_slice(&r);
        manguebyte::prelude::wav::write_file(&path, &buf, 48_000, manguebyte::prelude::WavFormat::Float32).unwrap();
        let table = extract(&[path], "test", |_| {}).unwrap();
        let note = &table.notes[0];
        assert!((note.pitch - 57.0).abs() < 0.05, "pitch {}", note.pitch);
        let p1 = note.partials[0];
        assert!((p1.t60 - 2.0).abs() < 0.3, "slow t60 {}", p1.t60);
        assert!((p1.amp - 0.3).abs() < 0.06, "slow amp {}", p1.amp);
        assert!(p1.fast_amp > 0.1 && p1.fast_t60 < 0.3, "fast {} {}", p1.fast_amp, p1.fast_t60);
        // 右边落后 0.5 弧度。
        assert!(p1.left > p1.right && (p1.phase + 0.5).abs() < 0.1, "stereo {} {} {}", p1.left, p1.right, p1.phase);
        assert_eq!(p1.n, 1);
        let p2 = note.partials.iter().find(|p| (p.ratio - 2.01).abs() < 0.01).expect("second partial");
        assert!((p2.t60 - 1.0).abs() < 0.2, "p2 t60 {}", p2.t60);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写一个测试用的录音：`f(t)` 给左右两路，前面留 10 ms 空白。
    fn write_note(dir: &Path, name: &str, seconds: f64, f: impl Fn(f64, usize) -> (f64, f64)) -> PathBuf {
        let sr = 48_000.0;
        let n = (seconds * sr) as usize;
        let mut buf = manguebyte::prelude::AudioBuffer::<f32>::new(2, n + 480);
        for i in 0..n {
            let (l, r) = f(i as f64 / sr, i);
            buf.channel_mut(0)[i + 480] = l as f32;
            buf.channel_mut(1)[i + 480] = r as f32;
        }
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        manguebyte::prelude::wav::write_file(&path, &buf, 48_000, manguebyte::prelude::WavFormat::Float32).unwrap();
        path
    }

    fn white(i: usize, salt: u64) -> f64 {
        let mut x = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }

    /// 录音有底噪：很快掉进底噪的分音不能量成"余音很长"。
    #[test]
    fn a_partial_that_sinks_into_the_noise_floor_keeps_its_decay() {
        let dir = std::env::temp_dir().join(format!("muser-modal-floor-{}", std::process::id()));
        let path = write_note(&dir, "A3_v90.wav", 6.0, |t, i| {
            let s = 0.3 * (TAU * 220.0 * t).sin() * 10f64.powf(-3.0 * t / 4.0) + 0.03 * (TAU * 1320.0 * t).sin() * 10f64.powf(-3.0 * t / 0.6);
            (s + 0.001 * white(i, 1), s + 0.001 * white(i, 2))
        });
        let note = analyze(&path, Some(57.0), 0.7).unwrap();
        let p6 = note.partials.iter().find(|p| p.n == 6).expect("sixth partial");
        assert!((p6.t60 - 0.6).abs() < 0.15, "t60 {} (the envelope is flat in the noise after 0.5 s)", p6.t60);
        let p1 = note.partials.iter().find(|p| p.n == 1).unwrap();
        assert!((p1.t60 - 4.0).abs() < 0.5 && p1.beat == 0.0, "t60 {} beat {}", p1.t60, p1.beat);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 钟、木琴的分音不成整数倍：也收，而且标成不在泛音列上（n = 0）。
    #[test]
    fn partials_off_the_harmonic_series_are_found() {
        let dir = std::env::temp_dir().join(format!("muser-modal-bell-{}", std::process::id()));
        let path = write_note(&dir, "A4_v90.wav", 3.0, |t, _| {
            let s = 0.3 * (TAU * 440.0 * t).sin() * 10f64.powf(-3.0 * t / 2.0)
                + 0.2 * (TAU * 440.0 * 2.76 * t).sin() * 10f64.powf(-3.0 * t / 1.0)
                + 0.1 * (TAU * 440.0 * 5.4 * t).sin() * 10f64.powf(-3.0 * t / 0.5);
            (s, s)
        });
        let note = analyze(&path, Some(69.0), 0.7).unwrap();
        for (ratio, amp, t60) in [(2.76, 0.2, 1.0), (5.4, 0.1, 0.5)] {
            let p = note
                .partials
                .iter()
                .find(|p| (p.ratio - ratio).abs() < 0.01)
                .unwrap_or_else(|| panic!("partial at {ratio}: {:?}", note.partials.iter().map(|p| p.ratio).collect::<Vec<_>>()));
            assert!(p.n == 0 && (p.amp - amp).abs() < 0.03 && (p.t60 - t60).abs() < 0.15, "{p:?}");
        }
        assert_eq!(note.partials.len(), 3, "{:?}", note.partials.iter().map(|p| p.ratio).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 起音的一下噪声：量出来在对的频段、对的快慢；提取的乐器演奏出来，波形和录音对得上（分音的起始相位）。
    #[test]
    fn attack_noise_and_start_phases_are_played_back() {
        let dir = std::env::temp_dir().join(format!("muser-modal-play-{}", std::process::id()));
        // 分音在 3 ms 里升起来（真的乐器不会从半截波形突然开始）。
        let rise = |t: f64| 0.5 - 0.5 * (core::f64::consts::PI * (t / 0.003).min(1.0)).cos();
        let tone = |t: f64| {
            rise(t) * (0.3 * (TAU * 220.0 * t + 0.7).sin() * 10f64.powf(-3.0 * t / 3.0) + 0.1 * (TAU * 440.0 * t - 1.0).sin() * 10f64.powf(-3.0 * t / 1.5))
        };
        // 2 … 8 kHz 的一下噪声（白噪声的二阶差分近似高通），60 ms 到 -60 dB。
        let burst = |t: f64, i: usize| 0.05 * (white(i, 3) - white(i + 1, 3)) * 10f64.powf(-3.0 * t / 0.06);
        let path = write_note(&dir, "A3_v100.wav", 4.0, |t, i| (tone(t) + burst(t, i), tone(t) + burst(t, i)));
        let table = extract(std::slice::from_ref(&path), "test", |_| {}).unwrap();
        assert_eq!(table.version, VERSION);
        let note = &table.notes[0];
        let high: f64 = note.bands.iter().filter(|b| b.hz >= 4_000.0).map(|b| b.amp * b.amp).sum::<f64>().sqrt();
        assert!(high > 0.02 && high < 0.12, "bands {:?}", note.bands);
        let top = note.bands.iter().find(|b| b.hz == 16_000.0).expect("16 kHz band");
        assert!(top.t60 > 0.03 && top.t60 < 0.1 && top.corr > 0.9, "{top:?}");
        // 低的频段里只有分音起音那几毫秒没对上的一点（演奏时分音升起来的快慢是固定的），比分音轻得多。
        assert!(note.bands.iter().filter(|b| b.hz <= 500.0).all(|b| b.amp < 0.1 * 0.3), "bands {:?}", note.bands);
        // 演奏出来和录音比：一样响，1 kHz 以下（分音）的波形对得上。
        let file = dir.join("t.yinse.json");
        std::fs::write(&file, serde_json::to_string(&table).unwrap()).unwrap();
        let list = compare(&file.to_string_lossy(), &[path], false).unwrap();
        assert!(list[0].level_db.abs() < 0.7 && list[0].spectral_db < 1.5, "{:?}", list[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 一个录音漏了一次泛音：和别的录音插值时按泛音序号对齐，不会把第 3 次和第 4 次平均成 3.5 次。
    #[test]
    fn partials_are_aligned_by_harmonic_number() {
        let partial = |n: u32| Partial { ratio: f64::from(n), amp: 0.1, t60: 2.0, left: 1.0, right: 1.0, n, ..Default::default() };
        let note = |velocity: f64, harmonics: &[u32]| NoteTable {
            source: String::new(),
            pitch: 60.0,
            velocity,
            partials: harmonics.iter().map(|&n| partial(n)).collect(),
            noise: Noise::default(),
            bands: Vec::new(),
            off_harmonic: 0.0,
            articulation: String::new(),
        };
        let mut t = ModalTable {
            format: FORMAT.into(),
            version: VERSION,
            name: String::new(),
            resonance: 0.0,
            notes: vec![note(0.4, &[1, 2, 4, 5]), note(0.8, &[1, 2, 3, 4, 5])],
            gestures: Gestures::default(),
            release: None,
        };
        let v = t.voice(60.0, 0.6, "");
        let ratios = |v: &Voice| v.partials.iter().map(|p| (p.ratio * 1e6).round() / 1e6).collect::<Vec<_>>();
        assert_eq!(ratios(&v), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!((v.partials[2].amp - 0.05).abs() < 1e-9 && (v.partials[3].amp - 0.1).abs() < 1e-9, "{:?}", v.partials);
        // 版本 1 的表没有序号：按频率比最近的对。
        for n in &mut t.notes {
            n.partials.iter_mut().for_each(|p| p.n = 0);
        }
        assert_eq!(ratios(&t.voice(60.0, 0.6, "")), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    /// 一整条录音：一个接一个的音切开来，各自量出音高；压在上一个音余音上的、太短的不用；同一个音高轻重两遍是两层力度。
    #[test]
    fn a_take_is_split_into_its_clean_notes() {
        let dir = std::env::temp_dir().join(format!("muser-modal-split-{}", std::process::id()));
        // (第几秒起, 频率, 振幅, 余音 t60)：A3 轻、A3 重、E4（还响着的时候 1 秒后来了个差不多响的 C5：E4 用到 C5 之前，C5 不干净不用）、最后 D4 只有 0.3 秒。
        let notes = [(0.2, 220.0, 0.1, 1.0), (3.2, 220.0, 0.4, 1.0), (6.2, 329.63, 0.3, 3.0), (7.2, 523.25, 0.3, 1.0), (9.5, 293.66, 0.3, 1.0)];
        let path = write_note(&dir, "take.wav", 9.8, |t, i| {
            let mut s = 0.0005 * white(i, 5);
            for &(at, hz, amp, t60) in &notes {
                if t >= at {
                    let x = t - at;
                    let rise = 0.5 - 0.5 * (core::f64::consts::PI * (x / 0.003).min(1.0)).cos();
                    s += rise * amp * 10f64.powf(-3.0 * x / t60) * ((TAU * hz * x).sin() + 0.4 * (TAU * 2.0 * hz * x).sin() + 0.2 * (TAU * 3.0 * hz * x).sin());
                }
            }
            (s, s)
        });
        let mut all = Vec::new();
        let files = split_notes(&[path], &dir.join("notes"), |s| all.push(s.clone())).unwrap();
        let used: Vec<i64> = all.iter().filter(|s| s.skipped.is_none()).map(|s| s.pitch.unwrap().round() as i64).collect();
        assert_eq!(used, vec![57, 57, 64], "{all:#?}");
        assert_eq!(all.len(), 5, "{all:#?}");
        assert!(all[3].skipped.is_some_and(|why| why.contains("still ringing")) && all[4].skipped.is_some_and(|why| why.contains("shorter")), "{all:#?}");
        let table = extract(&files, "take", |_| {}).unwrap();
        let a3: Vec<&NoteTable> = table.notes.iter().filter(|n| n.pitch == 57.0).collect();
        assert_eq!(a3.len(), 2);
        assert!(
            a3[0].velocity < a3[1].velocity && (a3[1].partials[0].amp / a3[0].partials[0].amp - 4.0).abs() < 0.4,
            "{:?} {:?}",
            a3[0].partials[0],
            a3[1].partials[0]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gestures_are_measured_from_synthetic_recordings() {
        let sr = 48_000.0;
        let n = (3.0 * sr) as usize;
        // 轮音：每秒 12 下，单数下 0.7 倍。
        let mut x = vec![0.0; n];
        for k in 0..36 {
            let at = (k as f64 / 12.0 * sr) as usize + 480;
            let g = if k % 2 == 0 { 1.0 } else { 0.7 };
            for i in 0..(0.08 * sr) as usize {
                if at + i < n {
                    let t = i as f64 / sr;
                    x[at + i] += g * (TAU * 440.0 * t).sin() * (-t / 0.03).exp();
                }
            }
        }
        let t = analyze_tremolo(&x, sr).expect("tremolo");
        assert!((t.rate - 12.0).abs() < 0.5 && (t.alternate - 0.7).abs() < 0.1 && (t.accel - 1.0).abs() < 0.1, "{t:?}");
        // 揉音：0.3 秒以后 ±20 音分、每秒 5 次。
        let mut phase = 0.0;
        let y: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / sr;
                let cents = if t > 0.3 { 20.0 * (TAU * 5.0 * (t - 0.3)).sin() } else { 0.0 };
                phase += TAU * 440.0 * (cents / 1200.0).exp2() / sr;
                0.5 * phase.sin() * (-t / 1.5).exp()
            })
            .collect();
        let p = [Partial { ratio: 1.0, amp: 0.5, t60: 3.0, left: 1.0, right: 1.0, ..Default::default() }];
        let v = analyze_vibrato(&y, sr, 440.0, &p).expect("vibrato");
        assert!((v.depth - 20.0).abs() < 4.0 && (v.rate - 5.0).abs() < 0.3 && (0.1..0.6).contains(&v.delay), "{v:?}");
        // 止音：0.8 秒按住，之后 0.08 秒掉 60 dB。
        let z: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / sr;
                let db = if t < 0.8 { -20.0 * t / 3.0 } else { -20.0 * 0.8 / 3.0 - 60.0 * (t - 0.8) / 0.08 };
                10f64.powf(db / 20.0) * (TAU * 440.0 * t).sin()
            })
            .collect();
        let d = analyze_damp(&z, sr).expect("damp");
        assert!((d - 0.08).abs() < 0.03, "damp {d}");
    }

    #[test]
    fn library_names_are_found_on_the_search_path() {
        let root = std::env::temp_dir().join(format!("muser-yinse-lib-{}", std::process::id()));
        let dir = root.join("instruments/strike/bell");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bell.yinse.json"), "{}").unwrap();
        std::fs::write(dir.join("meta.json"), r#"{"zh":"钟","description":"a bell","source":{"title":"Bells","author":"Someone","license":"CC0-1.0"}}"#)
            .unwrap();
        // 只有这个测试读写 MUSER_YINSE_PATH。
        unsafe { std::env::set_var("MUSER_YINSE_PATH", &root) };
        let list = library();
        let bell = list.iter().find(|e| e.spec == "yinse:strike/bell").expect("listed");
        assert_eq!((bell.name.as_str(), bell.license.as_str(), bell.source.as_str()), ("bell (钟)", "CC0-1.0", "Bells, Someone"));
        assert_eq!(resolve("strike/bell"), dir.join("bell.yinse.json"));
        assert_eq!(resolve("instruments/strike/bell/bell.yinse.json"), dir.join("bell.yinse.json"));
        assert_eq!(resolve("strike/none"), PathBuf::from("strike/none"));
        unsafe { std::env::remove_var("MUSER_YINSE_PATH") };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn velocities_and_pitches_in_between_are_blended() {
        let note = |pitch: f64, velocity: f64, amp: f64| NoteTable {
            source: String::new(),
            pitch,
            velocity,
            partials: vec![Partial { ratio: 1.0, amp, t60: 2.0, left: 1.0, right: 1.0, ..Default::default() }],
            noise: Noise::default(),
            bands: Vec::new(),
            off_harmonic: 0.0,
            articulation: String::new(),
        };
        let mut muted = note(60.0, 0.8, 0.05);
        muted.articulation = "mute".into();
        let t = ModalTable {
            format: FORMAT.into(),
            version: 1,
            name: String::new(),
            resonance: 0.0,
            notes: vec![note(60.0, 0.4, 0.1), note(60.0, 0.8, 0.3), note(72.0, 0.8, 0.5), muted],
            gestures: Gestures::default(),
            release: None,
        };
        assert!((t.voice(60.0, 0.6, "").partials[0].amp - 0.2).abs() < 1e-9);
        assert!((t.voice(66.0, 0.8, "").partials[0].amp - 0.4).abs() < 1e-9);
        assert!((t.voice(80.0, 0.4, "").partials[0].amp - 0.25).abs() < 1e-9);
        assert!((t.voice(66.0, 0.8, "mute").partials[0].amp - 0.05).abs() < 1e-9, "only the muted recording");
        assert!((t.voice(60.0, 0.8, "vibrato").partials[0].amp - 0.3).abs() < 1e-9, "gestures use the plain notes");
        assert_eq!(t.articulations(), vec!["mute".to_string()]);
    }
}
