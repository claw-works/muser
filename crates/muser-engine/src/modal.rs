//! 用户自己的模态乐器：从一组单音录音（一个文件一个音）里提取每个音的分音表——频率、起始音量、先快后慢两段衰减、
//! 一组弦之间的拍频、左右声道的比例与相位差、起音噪声、其余弦跟着响多少——存成一个很小的 JSON（`muser yinse extract`），
//! 再用模态弦模型演奏（`"instrument": "yinse:/path/x.yinse.json"`，见 [`crate::zither`]）。表里没有任何音频数据。
//!
//! 适合敲一下就自己衰减的乐器（扬琴、古筝、钢琴、钟……）；吹管、拉弦这类持续发声的不适合。

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use manguebyte::dsp::fft::{Complex, RealFft};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "muser-yinse-modal";
/// 每个音最多提取多少个分音（与模态弦模型的模式数相同）。
pub const MAX_PARTIALS: usize = 48;
/// 起音噪声的衰减时间常数（秒），提取和演奏用同一个。
pub const NOISE_SECONDS: f64 = 0.006;
/// 分析每个音的前几秒。
const ANALYSIS_SECONDS: f64 = 6.0;
const HOP_SECONDS: f64 = 0.01;

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
    pub noise: Noise,
    /// 余音里不在这个音泛音上的能量占多少（0 … 1）：其余弦的共振。
    pub off_harmonic: f64,
}

/// 一个分音：频率比（相对基频）、振幅（录音满幅为 1）与到 -60 dB 的秒数；起音时多出来、很快衰减的那一段；
/// 一组弦之间的拍（另一根弦占的比例、差几音分）；左右声道的增益与右边落后的相位（弧度）。
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
}

/// 起音噪声：振幅、低通频率（Hz）。衰减时间常数固定为 [`NOISE_SECONDS`]。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Noise {
    pub amp: f64,
    pub hz: f64,
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
            articulation = Some(if t == "muted" { "mute".into() } else if t == "plucked" { "pluck".into() } else { t });
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
    if den > 0.0 { num / den } else { f }
}

/// 一个频率在每一帧上的复振幅（左、右）：滑动汉宁窗解调。
fn demodulate(x: &[f64], hz: f64, window: usize, hop: usize, sr: f64) -> Vec<Complex<f64>> {
    let w = hann(window);
    let norm: f64 = w.iter().sum();
    let step = Complex::<f64>::from_polar(1.0, -TAU * hz / sr);
    let mut out = Vec::new();
    let mut start = 0;
    while start + window <= x.len() {
        let mut ph = Complex::<f64>::from_polar(1.0, -TAU * hz * start as f64 / sr);
        let (mut re, mut im) = (0.0, 0.0);
        for (v, w) in x[start..start + window].iter().zip(&w) {
            let a = v * w;
            re += a * ph.re;
            im += a * ph.im;
            ph = Complex::new(ph.re * step.re - ph.im * step.im, ph.re * step.im + ph.im * step.re);
        }
        out.push(Complex::new(2.0 * re / norm, 2.0 * im / norm));
        start += hop;
    }
    out
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

/// 一个分音的包络 → 慢衰减（振幅、t60）、起音多出的快衰减、拍。
fn fit_envelope(t: &[f64], amp: &[f64], hz: f64) -> Partial {
    let db: Vec<f64> = amp.iter().map(|a| 20.0 * a.max(1e-12).log10()).collect();
    let early = t.iter().take_while(|&&x| x < 0.08).count().max(1);
    let kp = (0..early).max_by(|&a, &b| db[a].total_cmp(&db[b])).unwrap_or(0);
    let top = db[kp];
    let end = (kp..db.len()).find(|&k| db[k] < top - 60.0).unwrap_or(db.len());
    let t_end = t[end.saturating_sub(1)];
    let mut from = (t[kp] + 0.25).max(0.3);
    if t_end - from < 0.25 {
        from = t[kp] + 0.05;
    }
    let slow: Vec<(f64, f64)> = (kp..end).filter(|&k| t[k] >= from).map(|k| (t[k], db[k])).collect();
    let Some((b, m)) = line(&slow).or_else(|| line(&(kp..end).map(|k| (t[k], db[k])).collect::<Vec<_>>())) else {
        return Partial { ratio: 1.0, amp: amp[kp], t60: 1.0, left: 1.0, right: 1.0, ..Default::default() };
    };
    let t60 = if m < -0.5 { -60.0 / m } else { 30.0 }.clamp(0.05, 30.0);
    // 演奏时从起音那一刻（峰值）开始：振幅取拟合线在峰值处的值，不往峰值以前外推。
    let a0 = 10f64.powf((b + m * t[kp]) / 20.0);
    let b = b + m * t[kp];
    let model = |x: f64| a0 * 10f64.powf(m * (x - t[kp]) / 20.0);
    // 起音那一段比慢衰减多出来的：对数线性拟合。
    let excess: Vec<(f64, f64)> = (kp..end)
        .filter(|&k| t[k] < 0.35)
        .filter_map(|k| {
            let e = amp[k] - model(t[k]);
            (e > 0.05 * amp[k]).then(|| (t[k], e.ln()))
        })
        .collect();
    let (mut fast_amp, mut fast_t60) = (0.0, 0.1);
    if let Some((c, d)) = line(&excess) {
        if d < -1.0 && amp[kp] - model(t[kp]) > 0.15 * model(t[kp]) {
            fast_amp = (c + d * t[kp]).exp().min(amp[kp]);
            fast_t60 = (1000f64.ln() / -d).clamp(0.01, 1.0);
        }
    }
    // 拍：残差（dB）里最强的周期起伏。
    let res: Vec<(f64, f64)> = (kp..end).filter(|&k| t[k] >= 0.1).map(|k| (t[k], db[k] - (b + m * (t[k] - t[kp])))).collect();
    let (mut beat, mut beat_cents) = (0.0, 0.0);
    if res.len() > 40 && res[res.len() - 1].0 - res[0].0 > 0.5 {
        let n = res.len() as f64;
        let mut best = (0.0, 0.0);
        let mut f = 0.3;
        while f < 12.0 {
            let (re, im) = res.iter().fold((0.0, 0.0), |(a, c), &(x, y)| (a + y * (TAU * f * x).cos(), c + y * (TAU * f * x).sin()));
            let a = 2.0 * (re * re + im * im).sqrt() / n;
            if a > best.0 {
                best = (a, f);
            }
            f += 0.05;
        }
        if best.0 > 0.4 {
            // 主弦占 1-w、另一根占 w：合起来在 1 与 1-2w 之间起伏，峰峰值 2A dB。
            beat = ((1.0 - 10f64.powf(-2.0 * best.0 / 20.0)) / 2.0).clamp(0.0, 0.45);
            beat_cents = 1200.0 * (1.0 + best.1 / hz).log2();
        }
    }
    Partial { ratio: 1.0, amp: a0, t60, fast_amp, fast_t60, beat, beat_cents, left: 1.0, right: 1.0, phase: 0.0 }
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
    let onset = mid.iter().position(|v| v.abs() > 0.05 * peak).unwrap_or(0).saturating_sub((0.002 * sr) as usize);
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
    let mut freqs = Vec::new();
    let (mut stretch, mut misses) = (1.0, 0);
    for n in 1..=MAX_PARTIALS {
        let expect = f0 * n as f64 * stretch;
        if expect > 0.45 * sr {
            break;
        }
        let (hz, m) = peak_near(&mag, bin_hz, expect, (0.3 * f0).max(2.0 * bin_hz));
        let lo = ((expect - f0 * 0.5) / bin_hz).max(1.0) as usize;
        let hi = (((expect + f0 * 0.5) / bin_hz) as usize).min(mag.len() - 1);
        let mut around: Vec<f64> = mag[lo..hi].to_vec();
        around.sort_by(f64::total_cmp);
        let floor = around.get(around.len() / 2).copied().unwrap_or(0.0);
        if m > 3.0 * floor && m > 1e-7 * size as f64 {
            freqs.push(hz);
            stretch = hz / (f0 * n as f64);
            misses = 0;
        } else {
            misses += 1;
            if misses >= 6 {
                break;
            }
        }
    }
    if freqs.is_empty() {
        return Err(format!("no partials found in '{}'", path.display()));
    }
    // 每个分音的包络：窗要够长才分得开相邻的分音。
    let window = ((4.5 / f0).max(0.02) * sr) as usize;
    let hop = (HOP_SECONDS * sr) as usize;
    let mut partials = Vec::new();
    for &hz in &freqs {
        let zl = demodulate(l, hz, window, hop, sr);
        let zr = demodulate(r, hz, window, hop, sr);
        let t: Vec<f64> = (0..zl.len()).map(|k| (k * hop + window / 2) as f64 / sr).collect();
        let amp: Vec<f64> = zl.iter().zip(&zr).map(|(a, b)| 0.5 * (a.norm_sqr().sqrt() + b.norm_sqr().sqrt())).collect();
        if amp.len() < 5 {
            continue;
        }
        let mut p = fit_envelope(&t, &amp, hz);
        p.ratio = hz / f0;
        // 立体声：起音后 20 … 600 ms 的左右平均振幅与右边相对左边的相位。
        let (mut sl, mut sr_, mut cr, mut ci) = (0.0, 0.0, 0.0, 0.0);
        for k in (0..t.len()).filter(|&k| (0.02..0.6).contains(&t[k])) {
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
            // 解调用的是 e^{-iωt}：右边的相位比左边大 φ，就是右边超前 φ。
            p.phase = -ci.atan2(cr);
        }
        partials.push(p);
    }
    // 起音噪声：前 15 ms 里不在分音上的能量。
    let w15 = (0.015 * sr) as usize;
    let size15 = next_pow2(w15 * 8);
    let (m15, b15) = spectrum(&x[..w15.min(len)], size15, sr);
    let near = |f: f64| freqs.iter().any(|&h| (f - h).abs() < 25.0 + 0.015 * h);
    let (mut total, mut rest, mut moment) = (0.0, 0.0, 0.0);
    for (k, m) in m15.iter().enumerate().skip(1) {
        let f = k as f64 * b15;
        let e = m * m;
        total += e;
        if !near(f) && f > 40.0 {
            rest += e;
            moment += e * f;
        }
    }
    let rms = (x[..w15.min(len)].iter().map(|v| v * v).sum::<f64>() / w15 as f64).sqrt();
    let hz = if rest > 0.0 { (moment / rest).clamp(200.0, 0.4 * sr) } else { 4_000.0 };
    let k = 1.0 - (-TAU * hz / sr).exp();
    // 白噪声（均匀分布，方差 1/3）过一阶低通、再按 NOISE_SECONDS 指数衰减，在 15 ms 里的均方根。
    let decay = (NOISE_SECONDS / (2.0 * 0.015) * (1.0 - (-2.0 * 0.015 / NOISE_SECONDS).exp())).sqrt();
    let unit = (k / (2.0 - k) / 3.0).sqrt() * decay;
    let noise = Noise { amp: if total > 0.0 { rms * (rest / total).sqrt() / unit.max(1e-9) } else { 0.0 }, hz };
    // 共振：1 … 2 秒的余音里不在这个音泛音上的能量。
    let mut off_harmonic = 0.0;
    if len as f64 > 2.2 * sr {
        let (mt, bt) = spectrum(&x[(1.0 * sr) as usize..(2.0 * sr) as usize], next_pow2((sr * 2.0) as usize), sr);
        let stretch_at = |n: usize| freqs.get(n - 1).copied().unwrap_or(f0 * n as f64 * freqs.last().map_or(1.0, |&h| h / (f0 * freqs.len() as f64)));
        let (mut all, mut harm) = (0.0, 0.0);
        for (k, m) in mt.iter().enumerate().skip(1) {
            let f = k as f64 * bt;
            if !(60.0..8_000.0).contains(&f) {
                continue;
            }
            let e = m * m;
            all += e;
            let n = (f / f0).round().max(1.0) as usize;
            if (f - stretch_at(n)).abs() < 3.0 + 0.03 * f0 {
                harm += e;
            }
        }
        if all > 0.0 {
            off_harmonic = (1.0 - harm / all).clamp(0.0, 1.0);
        }
    }
    Ok(NoteTable {
        source: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        pitch: 69.0 + 12.0 * (f0 / 440.0).log2(),
        velocity,
        partials,
        noise,
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
                .filter(|f| f.extension().and_then(|e| e.to_str()).is_some_and(|e| ["wav", "flac", "aif", "aiff", "ogg", "mp3"].contains(&e.to_ascii_lowercase().as_str())))
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
    let measured: Vec<f64> = notes.iter().filter(|n| n.articulation.is_empty()).map(|n| n.off_harmonic).filter(|&f| f > 0.0).collect();
    let resonance = if measured.is_empty() {
        0.0
    } else {
        let f = measured.iter().sum::<f64>() / measured.len() as f64;
        (RESONANCE_SCALE * (f / (1.0 - f).max(0.05)).sqrt()).clamp(0.0, 0.3)
    };
    // 五位有效数字就够了（文件小一半多）。
    let r = |x: f64| if x == 0.0 || !x.is_finite() { x } else { let k = 10f64.powi(4 - x.abs().log10().floor() as i32); (x * k).round() / k };
    for n in &mut notes {
        n.pitch = r(n.pitch);
        n.off_harmonic = r(n.off_harmonic);
        n.noise = Noise { amp: r(n.noise.amp), hz: r(n.noise.hz) };
        for p in &mut n.partials {
            for v in [&mut p.ratio, &mut p.amp, &mut p.t60, &mut p.fast_amp, &mut p.fast_t60, &mut p.beat, &mut p.beat_cents, &mut p.left, &mut p.right, &mut p.phase] {
                *v = r(*v);
            }
        }
    }
    let gestures = Gestures {
        tremolo: gestures.tremolo.map(|t| Tremolo { rate: r(t.rate), alternate: r(t.alternate), accel: r(t.accel) }),
        vibrato: gestures.vibrato.map(|v| Vibrato { depth: r(v.depth), rate: r(v.rate), delay: r(v.delay) }),
        damp: gestures.damp.map(r),
    };
    Ok(ModalTable { format: FORMAT.into(), version: 1, name: name.into(), resonance: r(resonance), notes, gestures })
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
    let swings: Vec<f64> = dev.chunks(per.max(2)).map(|c| (c.iter().copied().fold(f64::MIN, f64::max) - c.iter().copied().fold(f64::MAX, f64::min)) / 2.0).collect();
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
        let count = weights.iter().map(|&(i, _, _)| self.notes[i].partials.len()).max().unwrap_or(0);
        let mut partials = Vec::with_capacity(count);
        for k in 0..count {
            let present: Vec<(&Partial, f64, f64)> = weights.iter().filter_map(|&(i, w, g)| self.notes[i].partials.get(k).map(|p| (p, w, g))).collect();
            let wsum: f64 = present.iter().map(|&(_, w, _)| w).sum();
            if wsum <= 0.0 {
                continue;
            }
            let avg = |f: &dyn Fn(&Partial) -> f64| present.iter().map(|&(p, w, _)| f(p) * w).sum::<f64>() / wsum;
            let geo = |f: &dyn Fn(&Partial) -> f64| (present.iter().map(|&(p, w, _)| f(p).max(1e-9).ln() * w).sum::<f64>() / wsum).exp();
            let (pr, pi) = present.iter().fold((0.0, 0.0), |(a, b), &(p, w, _)| (a + p.phase.cos() * w, b + p.phase.sin() * w));
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
            });
        }
        let noise = Noise {
            amp: weights.iter().map(|&(i, w, g)| self.notes[i].noise.amp * w * g).sum(),
            hz: (weights.iter().map(|&(i, w, _)| self.notes[i].noise.hz.max(1.0).ln() * w).sum::<f64>() / weights.iter().map(|w| w.1).sum::<f64>().max(1e-9)).exp(),
        };
        Voice { partials, noise }
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
    if table.format != FORMAT || table.notes.is_empty() || table.notes.iter().any(|n| n.partials.is_empty()) {
        return Err(format!("'{}' is not a yinse instrument (format {FORMAT}, at least one note with partials)", path.display()));
    }
    let table: &'static ModalTable = Box::leak(Box::new(table));
    let profile = crate::zither::table_profile(table);
    cache.insert(path, (modified, table, profile));
    Ok((table, profile))
}

/// `yinse:<文件>` 这条音轨的表（不是录音提取的乐器、读不到都是 None）。
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

pub fn resolve(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
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

fn features(lr: &[Vec<f64>; 2], sr: f64) -> (Vec<Vec<f64>>, [f64; 4], f64) {
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
    (grid, decay, sab / (saa * sbb).sqrt().max(1e-12))
}

/// 把每个原录音在提取的乐器上用同样的音高、力度演奏一遍，比较。
pub fn compare(table_path: &str, files: &[PathBuf]) -> Result<Vec<Comparison>, String> {
    let (table, _) = load(table_path)?;
    let spec = format!("yinse:{}", resolve(table_path).display());
    let sr = 48_000u32;
    let mut out = Vec::new();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let known = table.notes.iter().find(|n| n.source == name);
        let info = parse_name(&stem);
        let (p, v) = (info.pitch, info.velocity);
        let pitch = p.or(known.map(|n| n.pitch.round())).ok_or_else(|| format!("'{name}': no pitch in the file name and not one of the instrument's notes"))?;
        let velocity = v.or(known.map(|n| n.velocity)).unwrap_or(0.8);
        let reference = read_stereo(f, sr)?;
        let mut project = muser_core::Project::new("compare", 60.0);
        let mut track = muser_core::Track::new("t", &spec);
        let mut clip = muser_core::Clip::new(0.0, 3.0);
        let mut note = muser_core::Note::new(pitch.round().clamp(0.0, 127.0) as u8, 0.0, 2.5);
        note.velocity = velocity as f32;
        clip.notes.push(note);
        track.clips.push(clip);
        project.tracks.push(track);
        let audio = crate::render::render(&project, crate::render::RenderSettings { tail_seconds: 1.0, ..Default::default() }).map_err(|e| e.to_string())?;
        let ours = [audio.channel(0).iter().map(|&v| f64::from(v)).collect(), audio.channel(1).iter().map(|&v| f64::from(v)).collect()];
        let (ga, da, sa) = features(&reference, f64::from(sr));
        let (gb, db, sb) = features(&ours, f64::from(sr));
        let cells: Vec<f64> = ga.iter().zip(&gb).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())).collect();
        out.push(Comparison {
            source: name,
            pitch,
            velocity,
            spectral_db: cells.iter().sum::<f64>() / cells.len().max(1) as f64,
            decay_ref: da,
            decay_ours: db,
            stereo_ref: sa,
            stereo_ours: sb,
        });
    }
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
        assert!(p1.left > p1.right && (p1.phase.abs() - 0.5).abs() < 0.1, "stereo {} {} {}", p1.left, p1.right, p1.phase);
        let p2 = note.partials.iter().find(|p| (p.ratio - 2.01).abs() < 0.01).expect("second partial");
        assert!((p2.t60 - 1.0).abs() < 0.2, "p2 t60 {}", p2.t60);
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
    fn velocities_and_pitches_in_between_are_blended() {
        let note = |pitch: f64, velocity: f64, amp: f64| NoteTable {
            source: String::new(),
            pitch,
            velocity,
            partials: vec![Partial { ratio: 1.0, amp, t60: 2.0, left: 1.0, right: 1.0, ..Default::default() }],
            noise: Noise::default(),
            off_harmonic: 0.0,
            articulation: String::new(),
        };
        let mut muted = note(60.0, 0.8, 0.05);
        muted.articulation = "mute".into();
        let t = ModalTable {
            format: FORMAT.into(), version: 1, name: String::new(), resonance: 0.0,
            notes: vec![note(60.0, 0.4, 0.1), note(60.0, 0.8, 0.3), note(72.0, 0.8, 0.5), muted], gestures: Gestures::default(),
        };
        assert!((t.voice(60.0, 0.6, "").partials[0].amp - 0.2).abs() < 1e-9);
        assert!((t.voice(66.0, 0.8, "").partials[0].amp - 0.4).abs() < 1e-9);
        assert!((t.voice(80.0, 0.4, "").partials[0].amp - 0.25).abs() < 1e-9);
        assert!((t.voice(66.0, 0.8, "mute").partials[0].amp - 0.05).abs() < 1e-9, "only the muted recording");
        assert!((t.voice(60.0, 0.8, "vibrato").partials[0].amp - 0.3).abs() < 1e-9, "gestures use the plain notes");
        assert_eq!(t.articulations(), vec!["mute".to_string()]);
    }
}
