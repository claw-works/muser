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
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// 能同时发几个音（风琴、合奏）；不给就是独奏乐器，连着的音是连奏。
    pub poly: bool,
    /// 连奏时音高滑过去的时间常数（秒）；不给用默认的（管乐，几毫秒）。
    pub glide_s: Option<f64>,
}

struct Analysis {
    hz: f64,
    amps: Vec<f64>,
    phases: Vec<f64>,
    noise: Vec<f64>,
    /// 头几次泛音的音量浮动多少（比例）。
    fluct: f64,
    cents: f64,
    seconds: f64,
    /// 起音：时间常数（秒）、比稳定以后响多少 dB。
    attack: Option<(f64, f64)>,
    /// 收音的时间常数（秒）。
    release: Option<f64>,
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

/// 一个长音 → 泛音、噪声、起伏、起音和收音。
fn analyze(x: &[f64], hint: f64) -> Result<Analysis, String> {
    // 包络（5 ms 一个点）：找稳定的那一段。
    let hop = (0.005 * SR) as usize;
    let env: Vec<f64> = x.chunks(hop).map(|c| (c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64).sqrt()).collect();
    let loud = median(env.iter().copied().filter(|e| *e > 0.05 * env.iter().copied().fold(0.0, f64::max)).collect());
    let (first, last) = (env.iter().position(|e| *e >= 0.7 * loud).unwrap_or(0), env.iter().rposition(|e| *e >= 0.7 * loud).unwrap_or(0));
    let (from, to) = (first * hop + (0.25 * SR) as usize, (last * hop).saturating_sub((0.15 * SR) as usize));
    if to < from + (0.8 * SR) as usize {
        return Err("less than 0.8 s of steady sound: hold every note 2 s or more".into());
    }
    let s = &x[from..to];
    let steady = median(env[from / hop..to / hop].to_vec());
    let hz = fundamental(s, hint);
    let count = (((0.5 * SR - 500.0) / hz) as usize).clamp(1, MAX_PARTIALS);
    // 逐帧解调：窗长 12 个周期，至多 120 帧。
    let window = ((12.0 * SR / hz).round() as usize).min(s.len() / 2);
    let w = hann(window);
    let norm: f64 = w.iter().sum();
    let frames = ((s.len() - window) / (window / 2).max(1)).clamp(2, 120);
    let starts: Vec<usize> = (0..frames).map(|k| k * (s.len() - window) / (frames - 1)).collect();
    let mut level = vec![vec![0.0; frames]; count];
    let mut relative = vec![Complex::<f64>::ZERO; count];
    let mut first_phase = Vec::with_capacity(frames);
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
                first_phase.push(z.im.atan2(z.re));
            }
            // 相对基频的相位：z_n · conj(u)^(n)，u 是基频的方向。
            let mut turn = Complex::new(1.0, 0.0);
            for _ in 0..=n {
                turn = Complex::new(turn.re * unit.re - turn.im * unit.im, turn.re * unit.im + turn.im * unit.re);
            }
            relative[n] = Complex::new(relative[n].re + z.re * turn.re - z.im * turn.im, relative[n].im + z.re * turn.im + z.im * turn.re);
        }
    }
    let amps: Vec<f64> = level.iter().map(|l| (l.iter().map(|v| v * v).sum::<f64>() / frames as f64).sqrt()).collect();
    // 引擎用 sin(nθ + φ)、基频是 0：解调是按 cos 量的，换过去是 φ + π/2 − n·π/2。
    let phases: Vec<f64> =
        relative.iter().enumerate().map(|(n, z)| (z.im.atan2(z.re) + PI / 2.0 - (n + 1) as f64 * PI / 2.0 + PI).rem_euclid(TAU) - PI).collect();
    let spread = |l: &[f64]| {
        let m = l.iter().sum::<f64>() / l.len() as f64;
        (l.iter().map(|v| (v - m).powi(2)).sum::<f64>() / l.len() as f64).sqrt() / m.max(1e-30)
    };
    // 音量的浮动：最响的几个泛音的（弱的那些量到的是噪声）。
    let mut loudest: Vec<usize> = (0..count).collect();
    loudest.sort_by(|a, b| amps[*b].total_cmp(&amps[*a]));
    let fluct = median(loudest.iter().take(6).filter(|n| amps[**n] > 0.1 * amps[loudest[0]]).map(|n| spread(&level[*n])).collect());
    // 基频的浮动：相邻两帧基频相位的差。
    let cents: Vec<f64> = first_phase
        .windows(2)
        .zip(starts.windows(2))
        .map(|(p, s)| {
            let d = (p[1] - p[0] + PI).rem_euclid(TAU) - PI;
            1200.0 * ((hz + d * SR / (TAU * (s[1] - s[0]).max(1) as f64)) / hz).max(1e-6).log2()
        })
        .collect();
    let mean = cents.iter().sum::<f64>() / cents.len().max(1) as f64;
    let cents = (cents.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / cents.len().max(1) as f64).sqrt();
    // 噪声：平均功率谱上相邻泛音正中间那些格子的中位数 × 带宽；最高的泛音再往上的整个算进去；基频以下不算。
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
    let bin = SR / size as f64;
    let top = (count as f64 + 0.5) * hz;
    let noise: Vec<f64> = EDGES
        .windows(2)
        .map(|e| {
            let (lo, hi) = (e[0].max(1.3 * hz), e[1]);
            if hi <= lo {
                return 0.0;
            }
            let between: Vec<f64> =
                ((lo / bin) as usize..(hi.min(top) / bin) as usize).filter(|k| (0.3..0.7).contains(&((*k as f64 * bin / hz) % 1.0))).map(|k| psd[k]).collect();
            let mut energy = if between.len() > 8 { median(between) * (hi.min(top) - lo).max(0.0) } else { 0.0 };
            energy += ((lo.max(top) / bin) as usize..((hi / bin) as usize).min(psd.len())).map(|k| psd[k]).sum::<f64>() * bin;
            energy.sqrt()
        })
        .collect();
    // 起音：从稳定音量的一成到九成用多久；头 150 ms 里最响的比稳定以后响多少。
    let t10 = env.iter().position(|e| *e >= 0.1 * steady);
    let t90 = env.iter().position(|e| *e >= 0.9 * steady);
    let attack = match (t10, t90) {
        (Some(a), Some(b)) if b > a || a > 0 => {
            let peak = env[b..(b + 30).min(env.len())].iter().copied().fold(0.0, f64::max);
            Some((((b - a) as f64 * 0.005 / 2.2).clamp(0.004, 0.3), (20.0 * (peak / steady).log10()).clamp(0.0, 9.0)))
        }
        _ => None,
    };
    // 收音：最后一次还在七成以上之后，多久掉到一成（20 dB）。
    let release = env[last..].iter().position(|e| *e < 0.1 * steady).filter(|n| *n < 200).map(|n| (n as f64 * 0.005 / 10f64.ln()).clamp(0.01, 1.0));
    Ok(Analysis { hz, amps, phases, noise, fluct, cents, seconds: s.len() as f64 / SR, attack, release })
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
    let (mut flucts, mut cents, mut attacks, mut accents, mut releases) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for input in &inputs {
        let source = input.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let lr = read_stereo(&input.file, SR as u32)?;
        let mid: Vec<f64> = lr[0].iter().zip(&lr[1]).map(|(a, b)| 0.5 * (a + b)).collect();
        if mid.iter().fold(0.0f64, |m, v| m.max(v.abs())) < 1e-5 {
            return Err(format!("'{}' is silent", input.file.display()));
        }
        let on = onset_of(&mid).saturating_sub((0.002 * SR) as usize);
        let a = analyze(&mid[on..], 440.0 * ((input.pitch - 69.0) / 12.0).exp2()).map_err(|e| format!("'{}': {e}", input.file.display()))?;
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
        cents.push(a.cents);
        if let Some((seconds, db)) = a.attack {
            attacks.push(seconds);
            accents.push(db);
        }
        releases.extend(a.release);
        let layer = HeldLayer {
            at: round(input.at, 3),
            partials: a.amps[..keep].iter().map(|v| round(*v, 4)).collect(),
            phases: a.phases[..keep].iter().map(|v| (v * 1000.0).round() / 1000.0).collect(),
            noise: a.noise.iter().map(|v| round(*v, 3)).collect(),
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
    Ok(HeldTable {
        format: FORMAT.into(),
        version: VERSION,
        name: name.into(),
        level_db: 0.0,
        mono: !options.poly,
        attack_s: round(or(attacks, 0.02), 2),
        release_s: round(or(releases, 0.05), 2),
        glide_s: options.glide_s.unwrap_or(0.005),
        attack_db: if accent < 1.0 { 0.0 } else { (accent * 10.0).round() / 10.0 },
        wobble: Wobble { depth: round(median(flucts), 2), cents: round(median(cents).max(0.1), 2), rate_hz: 4.0 },
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
        let ((ga, _, _, la), (gb, _, _, lb)) = (features(&skip(&reference), SR), features(&skip(&ours), SR));
        let cells: Vec<f64> = ga.iter().zip(&gb).flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs())).collect();
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
        let a = analyze(&synth(220.0, [0.2, 0.1, 0.05, 0.02], 2.5), 219.0).unwrap();
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
        assert!((0.01..0.04).contains(&attack) && accent < 1.0, "attack {attack} s, accent {accent} dB");
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
