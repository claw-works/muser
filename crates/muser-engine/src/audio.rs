//! 音频音轨（`"instrument": "audio"`）：把 clip 上的音频文件解码、转成渲染采样率的立体声，
//! 按 clip 的位置铺成整轨的干声。之后和乐器音轨一样走效果链、推子与路由。见 [`muser_core::audio`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use manguebyte::prelude::AudioBuffer;
use muser_core::Project;

/// 音频文件的实际路径：`~/` 展开；相对路径相对工程文件所在目录（没有时相对当前目录）。
pub fn resolve(project: &Project, file: &str) -> PathBuf {
    let path = PathBuf::from(&*crate::plugin::expand_home(file.trim()));
    match &project.base_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// 音频文件的概况：(格式, 秒, 采样率, 声道数)。
pub fn info(path: &Path) -> Result<(String, f64, u32, usize), String> {
    let f = manguebyte::formats::read_file(path).map_err(|e| format!("读取音频文件 '{}' 失败: {e}", path.display()))?;
    Ok((format!("{:?}", f.format), f.buffer.num_frames() as f64 / f64::from(f.sample_rate), f.sample_rate, f.buffer.num_channels()))
}

/// 解码成 `sr` 采样率的立体声。单声道复制到两边。
fn load(path: &Path, sr: u32) -> Result<AudioBuffer<f32>, String> {
    let f = manguebyte::formats::read_file(path).map_err(|e| format!("读取音频文件 '{}' 失败: {e}", path.display()))?;
    let src = &f.buffer;
    let mut stereo = AudioBuffer::<f32>::new(2, src.num_frames());
    for c in 0..2 {
        // shortcut: 多于两个声道时只取前两个，需要时再做下混。
        let from = c.min(src.num_channels().saturating_sub(1));
        if src.num_channels() > 0 {
            stereo.channel_mut(c).copy_from_slice(src.channel(from));
        }
    }
    Ok(if f.sample_rate == sr { stereo } else { manguebyte::dsp::resample(&stereo, f64::from(f.sample_rate), f64::from(sr)) })
}

/// 包络的分辨率：每秒多少个峰值。
const ENVELOPE_RATE: f64 = 100.0;

/// 文件的电平包络（每 10 ms 一个峰值，0–1），给界面画波形。按路径与修改时间缓存在进程里。
fn envelope(path: &Path) -> Result<std::sync::Arc<Vec<f32>>, String> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Cache = Mutex<HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<Vec<f32>>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((m, env)) = cache.lock().unwrap().get(path) {
        if *m == mtime {
            return Ok(env.clone());
        }
    }
    let f = manguebyte::formats::read_file(path).map_err(|e| format!("读取音频文件 '{}' 失败: {e}", path.display()))?;
    let hop = ((f64::from(f.sample_rate) / ENVELOPE_RATE) as usize).max(1);
    let n = f.buffer.num_frames();
    let env: Vec<f32> = (0..n.div_ceil(hop))
        .map(|k| (0..f.buffer.num_channels()).flat_map(|c| f.buffer.channel(c)[k * hop..((k + 1) * hop).min(n)].iter()).fold(0.0f32, |m, s| m.max(s.abs())))
        .collect();
    let env = Arc::new(env);
    cache.lock().unwrap().insert(path.to_path_buf(), (mtime, env.clone()));
    Ok(env)
}

/// 音频 clip 的波形（最多 `buckets` 个峰值，含片段增益，0–1）与它覆盖的拍数（文件比 clip 短时小于 clip 长度）。
/// 不是音频 clip、或文件读不了时为 `None`。
pub fn clip_wave(project: &Project, clip: &muser_core::Clip, buckets: usize) -> Option<(Vec<f32>, f64)> {
    let a = clip.audio.as_ref()?;
    let env = envelope(&resolve(project, &a.file)).ok()?;
    let seconds = clip.length.to_seconds(project.tempo);
    let from = ((a.offset * ENVELOPE_RATE) as usize).min(env.len());
    let to = (((a.offset + seconds) * ENVELOPE_RATE).ceil() as usize).min(env.len());
    let part = &env[from..to];
    let gain = manguebyte::prelude::decibels::db_to_gain(a.gain_db) as f32;
    let k = part.len().div_ceil(buckets.max(1)).max(1);
    let wave = part.chunks(k).map(|c| (c.iter().fold(0.0f32, |m, &v| m.max(v)) * gain).min(1.0)).collect();
    Some((wave, part.len() as f64 / ENVELOPE_RATE * project.tempo / 60.0))
}

/// 第 `ti` 条音轨整首的干声（`total_frames` 帧，`sr` 采样率）。
pub fn track_buffer(project: &Project, ti: usize, sr: u32, total_frames: usize) -> Result<AudioBuffer<f32>, String> {
    let track = &project.tracks[ti];
    let rate = f64::from(sr);
    let mut out = AudioBuffer::<f32>::new(2, total_frames);
    let mut files: HashMap<PathBuf, AudioBuffer<f32>> = HashMap::new();
    for (ci, clip) in track.clips.iter().enumerate() {
        let Some(a) = &clip.audio else { continue };
        let path = resolve(project, &a.file);
        if !files.contains_key(&path) {
            let buf = load(&path, sr).map_err(|e| format!("tracks[{ti}].clips[{ci}].audio: {e}"))?;
            files.insert(path.clone(), buf);
        }
        let src = &files[&path];
        let at = (clip.start.to_seconds(project.tempo) * rate).round() as usize;
        let from = (a.offset * rate).round() as usize;
        let len = (clip.length.to_seconds(project.tempo) * rate).round() as usize;
        let n = len.min(src.num_frames().saturating_sub(from)).min(total_frames.saturating_sub(at));
        let gain = manguebyte::prelude::decibels::db_to_gain(a.gain_db) as f32;
        let (fade_in, fade_out) = ((a.fade_in * rate) as usize, (a.fade_out * rate) as usize);
        for c in 0..2 {
            let (s, d) = (&src.channel(c)[from..from + n], &mut out.channel_mut(c)[at..at + n]);
            for (i, (d, s)) in d.iter_mut().zip(s).enumerate() {
                let mut g = gain;
                if i < fade_in {
                    g *= i as f32 / fade_in as f32;
                }
                if n - i <= fade_out {
                    g *= (n - i) as f32 / fade_out as f32;
                }
                *d += s * g;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use manguebyte::prelude::{WavFormat, wav};

    /// 1 秒 24 kHz 单声道 0.5 直流的 WAV，放在 120 bpm 的第 2 拍（1 秒处），从文件 0.25 秒处开始播 1 拍（0.5 秒）。
    #[test]
    fn places_resamples_and_trims() {
        let dir = std::env::temp_dir().join(format!("muser-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = AudioBuffer::<f32>::new(1, 24_000);
        mono.channel_mut(0).fill(0.5);
        wav::write_file(dir.join("dc.wav"), &mono, 24_000, WavFormat::Float32).unwrap();
        let mut p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": "audio", "clips": [
            { "start": 2, "length": 1, "audio": { "file": "dc.wav", "offset": 0.25, "gain_db": -6.0206, "fade_in": 0, "fade_out": 0 } }
        ] }] }))
        .unwrap();
        p.base_dir = Some(dir.clone());
        crate::validate(&p).unwrap();
        let b = track_buffer(&p, 0, 48_000, 96_000).unwrap();
        let l = b.channel(0);
        assert_eq!(l[47_999], 0.0);
        assert!((l[48_000 + 100] - 0.25).abs() < 1e-3 && (b.channel(1)[60_000] - 0.25).abs() < 1e-3, "单声道复制到两边、-6 dB");
        assert!((l[48_000 + 23_900] - 0.25).abs() < 1e-3);
        assert_eq!(l[48_000 + 24_100], 0.0, "播满 clip 长度（0.5 秒）就停");

        // 整个工程渲染：音频音轨走推子（居中 -3 dB）。
        let mix = crate::render(&p, Default::default()).unwrap();
        assert!((mix.channel(0)[60_000] - 0.25 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);

        // 波形：0.5 秒（1 拍）× 0.5 直流 × -6 dB。
        let (wave, beats) = clip_wave(&p, &p.tracks[0].clips[0], 10).unwrap();
        assert_eq!(wave.len(), 10);
        assert!((wave[3] - 0.25).abs() < 1e-3 && (beats - 1.0).abs() < 1e-9, "{wave:?} {beats}");

        p.tracks[0].clips[0].audio.as_mut().unwrap().file = "nope.wav".into();
        assert!(crate::validate(&p).unwrap_err().to_string().contains("nope.wav"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
