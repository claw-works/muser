//! 哼唱转谱：一段单旋律录音（哼、唱、吹口哨、单音乐器）→ 音符。不用 AI，纯信号处理：
//!
//! 1. 每 10 ms 测一次音高（McLeod 音高法，70–1100 Hz）与响度；可信度低或太轻的帧算"没在唱"。
//! 2. 音高轨迹做中值滤波（去掉偶发的八度跳），再整体校正音准（人哼的调常常不在 A=440 上：
//!    所有帧偏离最近半音的中位数当作整体偏差减掉）。
//! 3. 切音：停顿、音高跳过大半个半音并稳住、或同一音高上响度先落下再起来（"嗯 嗯"重复哼一个音）都算新音。
//!    太短的碎片（< 60 ms）丢掉。
//! 4. 按速度换成拍；先减掉起点相对网格的整体偏移（相位的中位数：起音慢的音色、录音延迟、习惯性地拖），
//!    再把起点与结尾对齐到网格（`step`，0 为不对齐、只取整到 1/480 拍），第一个音移到第 0 拍
//!    （`from_start` 时保留录音开头到第一个音的空白，配合节拍器录音用）。力度按每个音的响度。
//! 5. 由音高分布估计调。

use manguebyte::analysis::{PitchDetector, PitchMethod, PitchSettings};
use manguebyte::prelude::{AudioBuffer, ProcessSpec};
use muser_core::{Beats, Note};

/// 转谱选项。
#[derive(Debug, Clone, Copy)]
pub struct HumOptions {
    pub tempo: f64,
    /// 对齐的网格（拍），0 为不对齐。
    pub step: f64,
    /// 保留录音开头到第一个音的时间（用节拍器录时为真），否则第一个音移到第 0 拍。
    pub from_start: bool,
}

/// 转谱结果。
#[derive(Debug, Clone)]
pub struct Transcription {
    pub notes: Vec<Note>,
    /// 多少小节（4/4）。
    pub bars: u32,
    /// 估计的调（如 `Am`）。
    pub key: String,
    /// 整体音准偏差（音分，正为偏高）。
    pub tuning_cents: f64,
}

const FRAME_SECONDS: f64 = 0.01;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() { 0.0 } else { v[v.len() / 2] }
}

/// 转谱。`audio` 任意声道数（取平均），`sample_rate` 是它的采样率。
pub fn transcribe(audio: &AudioBuffer<f32>, sample_rate: u32, opts: HumOptions) -> Result<Transcription, String> {
    if !(opts.tempo > 0.0) {
        return Err("tempo 应 > 0".into());
    }
    let sr = f64::from(sample_rate);
    let n = audio.num_frames();
    let chans = audio.num_channels().max(1);
    let mono: Vec<f32> = (0..n).map(|i| (0..audio.num_channels()).map(|c| audio.channel(c)[i]).sum::<f32>() / chans as f32).collect();
    let hop = ((sr * FRAME_SECONDS) as usize).max(1);
    let mut det = PitchDetector::new(PitchSettings::new(PitchMethod::Mpm { cutoff: 0.93 }).with_range(70.0, 1100.0).with_hop(hop));
    det.prepare(&ProcessSpec { sample_rate: sr, max_block_size: hop, num_channels: 1 });
    // 估计对应的是最近一个分析窗，窗的中心比当前位置早半个窗。
    let lag = det.frame_len() / 2;

    // 每帧：(时间秒, 音高 MIDI 浮点或 None, 均方根)。
    let mut frames: Vec<(f64, Option<f64>, f64)> = Vec::new();
    for (k, chunk) in mono.chunks(hop).enumerate() {
        det.measure_samples(chunk);
        let p = det.latest();
        let rms = (chunk.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / chunk.len() as f64).sqrt();
        let t = ((k * hop + chunk.len()) as f64 - lag as f64).max(0.0) / sr;
        let midi = (!p.is_none() && p.confidence >= 0.8).then(|| 69.0 + 12.0 * (p.frequency / 440.0).log2());
        frames.push((t, midi, rms));
    }
    // 响度门限：最响处以下 30 dB，且不低于 -55 dBFS。
    let loud = frames.iter().map(|f| f.2).fold(0.0, f64::max);
    if loud < 1e-4 {
        return Err("录音几乎没有声音".into());
    }
    let gate = (loud * 10f64.powf(-30.0 / 20.0)).max(10f64.powf(-55.0 / 20.0));
    // 窗的响度比帧滞后，用前后相邻帧的最大值判断"在唱"，免得音头音尾被切掉。
    let voiced: Vec<Option<f64>> = (0..frames.len())
        .map(|i| {
            let r = (i.saturating_sub(1)..(i + 2).min(frames.len())).map(|j| frames[j].2).fold(0.0, f64::max);
            frames[i].1.filter(|_| r >= gate)
        })
        .collect();
    // 中值滤波（5 帧，只用有音高的帧）。
    let smooth: Vec<Option<f64>> = (0..voiced.len())
        .map(|i| {
            voiced[i].map(|_| {
                let mut w: Vec<f64> = (i.saturating_sub(2)..(i + 3).min(voiced.len())).filter_map(|j| voiced[j]).collect();
                median(&mut w)
            })
        })
        .collect();
    // 整体音准：偏离最近半音的中位数。
    let mut dev: Vec<f64> = smooth.iter().flatten().map(|m| m - m.round()).collect();
    if dev.len() < 5 {
        return Err("没有听出音高：离麦克风近一点、哼得稳一点，或换一段更清楚的录音".into());
    }
    let offset = median(&mut dev);

    // 切音。
    struct Seg {
        first: usize,
        last: usize,
        pitches: Vec<f64>,
        rms: Vec<f64>,
    }
    let mut segs: Vec<Seg> = Vec::new();
    let mut cur: Option<Seg> = None;
    let mut gap = 0usize;
    let close = |s: Seg, segs: &mut Vec<Seg>| {
        if (s.last - s.first + 1) as f64 * FRAME_SECONDS >= 0.06 {
            segs.push(s);
        }
    };
    for i in 0..smooth.len() {
        let Some(m) = smooth[i].map(|m| m - offset) else {
            gap += 1;
            if gap >= 3 {
                if let Some(s) = cur.take() {
                    close(s, &mut segs);
                }
            }
            continue;
        };
        gap = 0;
        let rms = frames[i].2;
        if let Some(s) = cur.as_mut() {
            let mut ps = s.pitches.clone();
            let center = median(&mut ps);
            // 音高稳定地换到别的半音（接下来 3 帧都离开当前音 0.7 个半音以上）。
            let moved = (i..(i + 3).min(smooth.len())).all(|j| smooth[j].is_some_and(|x| (x - offset - center).abs() > 0.7));
            // 同一个音重新起音：响度落到这个音平均的 40% 以下又回来。
            let mean = s.rms.iter().sum::<f64>() / s.rms.len() as f64;
            let dipped = s.rms.last().is_some_and(|&r| r < 0.4 * mean) && rms > 0.7 * mean && s.rms.len() >= 6;
            if moved || dipped {
                let done = cur.take().unwrap();
                close(done, &mut segs);
            }
        }
        match cur.as_mut() {
            Some(s) => {
                s.last = i;
                s.pitches.push(m);
                s.rms.push(rms);
            }
            None => cur = Some(Seg { first: i, last: i, pitches: vec![m], rms: vec![rms] }),
        }
    }
    if let Some(s) = cur.take() {
        close(s, &mut segs);
    }
    if segs.is_empty() {
        return Err("没有切出音：录音太短或太轻".into());
    }

    // 换成拍、对齐网格。
    let beat = |t: f64| t * opts.tempo / 60.0;
    let q = |b: f64| if opts.step > 0.0 { (b / opts.step).round() * opts.step } else { (b * 480.0).round() / 480.0 };
    let peak = segs.iter().map(|s| s.rms.iter().sum::<f64>() / s.rms.len() as f64).fold(0.0, f64::max);
    let mut shift = if opts.from_start { 0.0 } else { beat(frames[segs[0].first].0) };
    // 整体的早 / 晚（起音慢的音色、录音延迟、人习惯性地拖）：起点相对网格的相位取中位数，对齐前减掉。
    // 相位在半格附近分不清是早还是晚，按"更可能是晚"处理：落在 [-0.3, 0.7) 格。
    if opts.step > 0.0 && segs.len() >= 4 {
        let mut phases: Vec<f64> = segs
            .iter()
            .map(|s| {
                let x = (beat(frames[s.first].0) - shift) / opts.step;
                let f = x - x.floor();
                if f >= 0.7 { f - 1.0 } else { f }
            })
            .collect();
        let m = median(&mut phases);
        if m.abs() > 0.1 {
            shift += m * opts.step;
        }
    }
    let mut notes: Vec<Note> = Vec::new();
    for s in &segs {
        let mut ps = s.pitches.clone();
        let pitch = median(&mut ps).round().clamp(0.0, 127.0) as u8;
        let (a, z) = (beat(frames[s.first].0) - shift, beat(frames[s.last].0 + FRAME_SECONDS) - shift);
        let (start, mut end) = (q(a.max(0.0)), q(z));
        let min = if opts.step > 0.0 { opts.step } else { 1.0 / 32.0 };
        if end - start < min {
            end = start + min;
        }
        let level = s.rms.iter().sum::<f64>() / s.rms.len() as f64 / peak;
        // 对齐后与上一个音同起点：后一个更长就留后一个（偶发的小碎片被吞掉）。
        if let Some(prev) = notes.last_mut() {
            if (prev.start.0 - start).abs() < 1e-9 {
                if end - start > prev.length.0 {
                    *prev = Note { pitch, start: Beats(start), length: Beats(end - start), velocity: (0.45 + 0.55 * level).clamp(0.3, 1.0) as f32, accent: false, bend: Vec::new() };
                }
                continue;
            }
            // 和下一个音重叠的部分截掉。
            if prev.start.0 + prev.length.0 > start {
                prev.length = Beats(start - prev.start.0);
            }
        }
        notes.push(Note { pitch, start: Beats(start), length: Beats(end - start), velocity: (0.45 + 0.55 * level).clamp(0.3, 1.0) as f32, accent: false, bend: Vec::new() });
    }
    let end = notes.iter().map(|n| n.start.0 + n.length.0).fold(0.0, f64::max);
    let bars = ((end / 4.0) - 1e-9).ceil().max(1.0) as u32;
    let tones: Vec<crate::abc_export::Tone> = notes.iter().map(|n| crate::abc_export::Tone { start: n.start.0, end: n.start.0 + n.length.0, pitch: n.pitch }).collect();
    let key = crate::abc_export::estimate_key(&tones).name().to_string();
    Ok(Transcription { notes, bars, key, tuning_cents: (offset * 100.0).round() })
}

/// 读音频文件（WAV / AIFF / FLAC / Ogg / Opus / MP3 …）并转谱。
pub fn transcribe_file(path: &std::path::Path, opts: HumOptions) -> Result<Transcription, String> {
    let f = manguebyte::formats::read_file(path).map_err(|e| format!("读取音频文件 '{}' 失败: {e}", path.display()))?;
    transcribe(&f.buffer, f.sample_rate, opts)
}

/// 网格写法：`1/16`、`1/8`、`1/8t`（三连音）、`0.25`（拍）、`off` / `0`（不对齐）→ 拍。
pub fn parse_grid(s: &str) -> Result<f64, String> {
    let s = s.trim().to_ascii_lowercase();
    if s == "off" || s == "0" || s == "free" {
        return Ok(0.0);
    }
    let (body, triplet) = match s.strip_suffix('t') {
        Some(b) => (b, true),
        None => (s.as_str(), false),
    };
    let beats = match body.split_once('/') {
        Some(("1", d)) => d.parse::<f64>().ok().filter(|d| *d > 0.0).map(|d| 4.0 / d),
        Some(_) => None,
        None => body.parse::<f64>().ok().filter(|b| *b > 0.0),
    }
    .ok_or_else(|| format!("网格 '{s}' 看不懂：写 1/16、1/8、1/8t、0.25（拍）或 off"))?;
    Ok(if triplet { beats * 2.0 / 3.0 } else { beats })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成一段"哼唱"：正弦带泛音、颤音、一点噪声、整体偏高 30 音分，音与音之间有短暂的气口，
    /// 同一音高重复两次（中间响度落下）。
    fn hum(notes: &[(u8, f64)], tempo: f64, sr: u32) -> AudioBuffer<f32> {
        let spb = 60.0 / tempo;
        let total: f64 = notes.iter().map(|n| n.1).sum::<f64>() * spb + 0.3;
        let frames = (total * f64::from(sr)) as usize;
        let mut buf = AudioBuffer::<f32>::new(1, frames);
        let mut t0 = 0.15; // 开头留一点空白
        let mut phase = 0.0f64;
        let mut rng = 12345u64;
        for &(pitch, beats) in notes {
            let dur = beats * spb;
            let f0 = 440.0 * 2f64.powf((f64::from(pitch) - 69.0 + 0.3) / 12.0);
            let (a, z) = ((t0 * f64::from(sr)) as usize, ((t0 + dur - 0.04) * f64::from(sr)) as usize);
            for i in a..z.min(frames) {
                let t = (i - a) as f64 / f64::from(sr);
                let f = f0 * (1.0 + 0.006 * (2.0 * std::f64::consts::PI * 5.5 * t).sin());
                phase += 2.0 * std::f64::consts::PI * f / f64::from(sr);
                let env = (t / 0.02).min(1.0) * ((z - i) as f64 / (0.02 * f64::from(sr))).min(1.0);
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let noise = (rng % 2000) as f64 / 1000.0 - 1.0;
                buf.channel_mut(0)[i] = (env * (0.5 * phase.sin() + 0.2 * (2.0 * phase).sin() + 0.08 * (3.0 * phase).sin()) + 0.003 * noise) as f32;
            }
            t0 += dur;
        }
        buf
    }

    #[test]
    fn transcribes_a_hummed_phrase() {
        // A C E D C C（八分、八分、四分、四分、四分、四分），100 bpm。
        let tune = [(69, 0.5), (72, 0.5), (76, 1.0), (74, 1.0), (72, 1.0), (72, 1.0)];
        let audio = hum(&tune, 100.0, 48_000);
        let r = transcribe(&audio, 48_000, HumOptions { tempo: 100.0, step: 0.25, from_start: false }).unwrap();
        let got: Vec<(u8, f64)> = r.notes.iter().map(|n| (n.pitch, n.start.0)).collect();
        assert_eq!(got, [(69, 0.0), (72, 0.5), (76, 1.0), (74, 2.0), (72, 3.0), (72, 4.0)], "{:?}", r.notes);
        assert!((25.0..=35.0).contains(&r.tuning_cents), "音准偏差 {}", r.tuning_cents);
        assert_eq!(r.bars, 2);
        assert!(r.key == "Am" || r.key == "C", "{}", r.key);
        // 不对齐：起点精确到 1/480 拍，不一定落在网格上。
        let free = transcribe(&audio, 48_000, HumOptions { tempo: 100.0, step: 0.0, from_start: false }).unwrap();
        assert_eq!(free.notes.len(), 6);
        assert!((free.notes[2].start.0 - 1.0).abs() < 0.1);
    }

    #[test]
    fn grids() {
        assert_eq!(parse_grid("1/16"), Ok(0.25));
        assert_eq!(parse_grid("off"), Ok(0.0));
        assert!((parse_grid("1/8t").unwrap() - 1.0 / 3.0).abs() < 1e-12);
        assert!(parse_grid("2/16").is_err());
    }

    #[test]
    fn silence_is_an_error() {
        let quiet = AudioBuffer::<f32>::new(1, 48_000);
        assert!(transcribe(&quiet, 48_000, HumOptions { tempo: 100.0, step: 0.25, from_start: false }).unwrap_err().contains("没有声音"));
    }
}
