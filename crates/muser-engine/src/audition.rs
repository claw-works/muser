//! 试听（audition）：给 AI 一副"耳朵"。
//!
//! 对一件乐器或一个效果器（内置或外置插件），把某个参数依次设成几个值，各渲染一段测试音，
//! 返回可量化的听感特征：响度、亮度（频谱重心）、频段分布、起音时间、余音长度。
//! AI 可以据此判断"这个参数调大，声音变亮 / 变长 / 变响"，不需要预先知道插件。

use manguebyte::analysis::LoudnessMeter;
use manguebyte::dsp::fft::{Complex, RealFft};
use manguebyte::prelude::{decibels, AudioBuffer, ProcessSpec, Processor, WavFormat, wav};
use muser_core::{Clip, Effect, Note, Project, Track};
use serde::Serialize;
use serde_json::Value;

use crate::{render, EngineError, RenderSettings};

/// 一段音频的听感特征。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Analysis {
    /// 峰值（dBFS）。
    pub peak_db: f64,
    /// 有声部分的均方根电平（dBFS）。
    pub rms_db: f64,
    /// 整体响度（LUFS，EBU R128 积分响度）。
    pub lufs: f64,
    /// 频谱重心（Hz）：越高越亮。
    pub centroid_hz: f64,
    /// 能量分布（%）：低频 < 250 Hz、中频 250-4000 Hz、高频 > 4000 Hz。
    pub low_pct: f64,
    pub mid_pct: f64,
    pub high_pct: f64,
    /// 从音符开始到包络达到峰值 90% 的时间（毫秒）。
    pub attack_ms: f64,
    /// 音符结束后衰减到峰值以下 40 dB 的时间（秒）；渲染结束仍未衰减到则为渲染剩余长度。
    pub release_s: f64,
    /// `release_s` 是否被渲染长度截断（真实余音更长）。
    pub release_truncated: bool,
}

/// 分析一段立体声音频。`note` 为测试音符所在的帧区间 [开始, 结束)。
pub fn analyze(audio: &AudioBuffer<f32>, sample_rate: f64, note: (usize, usize)) -> Analysis {
    let frames = audio.num_frames();
    let mono: Vec<f32> = (0..frames).map(|i| 0.5 * (audio.channel(0)[i] + audio.channel(1)[i])).collect();
    let peak = audio.peak().max(1e-12);

    // 包络：每毫秒窗口的绝对值最大值。
    let win = ((sample_rate / 1000.0) as usize).max(1);
    let env: Vec<f32> = mono.chunks(win).map(|c| c.iter().fold(0f32, |m, s| m.max(s.abs()))).collect();
    let env_peak = env.iter().cloned().fold(0f32, f32::max).max(1e-12);
    let on = note.0 / win;
    let attack_ms = env[on.min(env.len())..]
        .iter()
        .position(|e| *e >= 0.9 * env_peak)
        .map_or(0.0, |i| i as f64 * win as f64 / sample_rate * 1000.0);
    let off = (note.1 / win).min(env.len());
    let floor = env_peak * 0.01; // -40 dB
    let after = &env[off..];
    let (release_s, release_truncated) = match after.iter().position(|e| *e < floor) {
        Some(i) => (i as f64 * win as f64 / sample_rate, false),
        None => (after.len() as f64 * win as f64 / sample_rate, true),
    };

    // 有声部分的 RMS：包络高于峰值 -60 dB 的窗口。
    let gate = env_peak * 0.001;
    let (mut sum, mut n) = (0.0f64, 0usize);
    for (i, e) in env.iter().enumerate() {
        if *e >= gate {
            for s in &mono[i * win..((i + 1) * win).min(frames)] {
                sum += f64::from(*s).powi(2);
                n += 1;
            }
        }
    }
    let rms_db = decibels::gain_to_db((sum / n.max(1) as f64).sqrt());

    // 响度。
    let mut meter = LoudnessMeter::stereo();
    Processor::<f32>::prepare(&mut meter, &ProcessSpec { sample_rate, max_block_size: 1024, num_channels: 2 });
    let mut copy = AudioBuffer::<f32>::new(2, frames);
    for ch in 0..2 {
        copy.channel_mut(ch).copy_from_slice(audio.channel(ch));
    }
    let mut start = 0;
    while start < frames {
        let end = (start + 1024).min(frames);
        copy.windowed(start..end, |b| Processor::<f32>::process(&mut meter, b));
        start = end;
    }
    let lufs = meter.readings().integrated();

    // 平均幅度谱（Hann 窗，4096 点，半重叠），只取有声部分。
    const N: usize = 4096;
    let mut fft = RealFft::<f32>::new(N);
    let hann: Vec<f32> = (0..N).map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos()) as f32).collect();
    let mut spectrum = vec![Complex::<f32>::default(); fft.spectrum_len()];
    let mut mag = vec![0f64; fft.spectrum_len()];
    let mut frame = vec![0f32; N];
    let mut pos = 0;
    while pos + N <= frames {
        let level = mono[pos..pos + N].iter().fold(0f32, |m, s| m.max(s.abs()));
        if level >= peak * 0.01 {
            for i in 0..N {
                frame[i] = mono[pos + i] * hann[i];
            }
            fft.forward(&frame, &mut spectrum);
            for (m, c) in mag.iter_mut().zip(&spectrum) {
                *m += f64::from((c.re * c.re + c.im * c.im).sqrt());
            }
        }
        pos += N / 2;
    }
    let bin_hz = sample_rate / N as f64;
    let total: f64 = mag.iter().sum::<f64>().max(1e-12);
    let centroid_hz = mag.iter().enumerate().map(|(i, m)| i as f64 * bin_hz * m).sum::<f64>() / total;
    let energy: Vec<f64> = mag.iter().map(|m| m * m).collect();
    let etot: f64 = energy.iter().sum::<f64>().max(1e-12);
    let band = |lo: f64, hi: f64| {
        100.0 * energy.iter().enumerate().filter(|(i, _)| { let f = *i as f64 * bin_hz; f >= lo && f < hi }).map(|(_, e)| e).sum::<f64>() / etot
    };

    let round = |x: f64, d: i32| { let p = 10f64.powi(d); (x * p).round() / p };
    Analysis {
        peak_db: round(decibels::gain_to_db(f64::from(peak)), 1),
        rms_db: round(rms_db, 1),
        lufs: round(lufs, 1),
        centroid_hz: round(centroid_hz, 0),
        low_pct: round(band(0.0, 250.0), 1),
        mid_pct: round(band(250.0, 4000.0), 1),
        high_pct: round(band(4000.0, f64::INFINITY), 1),
        attack_ms: round(attack_ms, 0),
        release_s: round(release_s, 2),
        release_truncated,
    }
}

/// 试听的对象。
#[derive(Debug, Clone)]
pub enum Subject {
    /// 乐器（内置名、`sfz:` 或 `plugin:`），附带基础 `instrument_params`。
    Instrument { name: String, params: serde_json::Map<String, Value> },
    /// 效果器（内置或 `plugin`），用测试音源（`source` 乐器弹一个和弦）送进去。
    Effect { effect: Effect, source: String },
}

/// 试听设置。
#[derive(Debug, Clone)]
pub struct AuditionSettings {
    /// 测试音高（乐器）；鼓组用 GM 鼓键。效果器的测试和弦以它为根音。
    pub pitch: u8,
    pub velocity: f32,
    /// 音符按住多久（秒）。
    pub hold_seconds: f64,
    /// 音符结束后再录多久（秒），用于听余音。
    pub tail_seconds: f64,
    /// 把每一版写成 wav 的目录（可选）。
    pub out_dir: Option<std::path::PathBuf>,
}

impl Default for AuditionSettings {
    fn default() -> Self {
        Self { pitch: 60, velocity: 0.8, hold_seconds: 1.0, tail_seconds: 1.5, out_dir: None }
    }
}

/// 一版试听结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Take {
    /// 参数取值（无参数时为 null，表示原样）。
    pub value: Value,
    pub analysis: Analysis,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wav: Option<String>,
}

const TEMPO: f64 = 120.0;

fn project_for(subject: &Subject, param: Option<(&str, &Value)>, s: &AuditionSettings) -> Result<Project, String> {
    let beats = s.hold_seconds * TEMPO / 60.0;
    let mut p = Project::new("audition", TEMPO);
    let mut clip = Clip::new(0.0, beats);
    let mut track;
    match subject {
        Subject::Instrument { name, params } => {
            track = Track::new("audition", name.clone());
            track.instrument_params = params.clone();
            if let Some((k, v)) = param {
                track.instrument_params.insert(k.to_string(), v.clone());
            }
            let mut n = Note::new(s.pitch, 0.0, beats);
            n.velocity = s.velocity;
            clip.notes.push(n);
        }
        Subject::Effect { effect, source } => {
            track = Track::new("audition", source.clone());
            let mut fx = effect.clone();
            if let Some((k, v)) = param {
                fx = set_effect_param(&fx, k, v)?;
            }
            track.effects.push(fx.into());
            for o in [0u8, 7, 12, 16] {
                let mut n = Note::new(s.pitch.saturating_sub(12).saturating_add(o), 0.0, beats);
                n.velocity = s.velocity;
                clip.notes.push(n);
            }
        }
    }
    track.clips.push(clip);
    p.tracks.push(track);
    Ok(p)
}

/// 在效果器上设置一个参数：插件写进 `params`，内置效果器改对应字段。
fn set_effect_param(effect: &Effect, key: &str, v: &Value) -> Result<Effect, String> {
    if let Effect::Plugin { path, id, params } = effect {
        let mut params = params.clone();
        params.insert(key.to_string(), v.clone());
        return Ok(Effect::Plugin { path: path.clone(), id: id.clone(), params });
    }
    let mut obj = serde_json::to_value(effect).map_err(|e| e.to_string())?;
    let map = obj.as_object_mut().expect("效果器序列化为对象");
    if key == "type" || !map.contains_key(key) {
        let spec = muser_core::effect_spec(effect.type_name()).expect("描述");
        return Err(format!(
            "{} 没有参数 '{key}'，可用: {}",
            spec.name,
            spec.params.iter().map(|p| p.name).chain(spec.choices.iter().map(|c| c.name)).collect::<Vec<_>>().join(", ")
        ));
    }
    map.insert(key.to_string(), v.clone());
    serde_json::from_value(obj).map_err(|e| format!("{key}: {e}"))
}

/// 逐个取值渲染并分析。`param` 为 None 时只渲染原样一版。
pub fn audition(
    subject: &Subject,
    param: Option<&str>,
    values: &[Value],
    settings: &AuditionSettings,
) -> Result<Vec<Take>, EngineError> {
    let invalid = |m: String| EngineError::Core(muser_core::CoreError::Invalid(m));
    let rs = RenderSettings { tail_seconds: settings.tail_seconds, ..RenderSettings::default() };
    let sr = f64::from(rs.sample_rate);
    let note = (0usize, (settings.hold_seconds * sr) as usize);
    let runs: Vec<Option<&Value>> = match param {
        Some(_) if values.is_empty() => return Err(invalid("给出了参数但没有取值".into())),
        Some(_) => values.iter().map(Some).collect(),
        None => vec![None],
    };
    let mut takes = Vec::new();
    for (i, v) in runs.into_iter().enumerate() {
        let project = project_for(subject, param.zip(v), settings).map_err(invalid)?;
        let audio = render(&project, rs)?;
        let wav_path = match &settings.out_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| EngineError::Wav(e.to_string()))?;
                let label = v.map_or("base".to_string(), |v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
                let path = dir.join(format!("{:02}_{}.wav", i + 1, safe));
                wav::write_file(&path, &audio, rs.sample_rate, WavFormat::Int16).map_err(|e| EngineError::Wav(format!("{e:?}")))?;
                Some(path.display().to_string())
            }
            None => None,
        };
        takes.push(Take { value: v.cloned().unwrap_or(Value::Null), analysis: analyze(&audio, sr, note), wav: wav_path });
    }
    Ok(takes)
}

/// 把命令行里的取值转成 JSON：能读成数字/布尔就用数字/布尔，否则当文字。
/// 用双引号括起来强制当文字（插件显示为纯数字时，如 `"200.0"`）。
pub fn parse_value(s: &str) -> Value {
    match serde_json::from_str::<Value>(s.trim()) {
        Ok(v @ (Value::Number(_) | Value::Bool(_) | Value::String(_))) => v,
        _ => Value::String(s.trim().to_string()),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_quotes() {
        assert_eq!(parse_value("200"), serde_json::json!(200));
        assert_eq!(parse_value("\"200.0\""), serde_json::json!("200.0"));
        assert_eq!(parse_value("200 Hz"), serde_json::json!("200 Hz"));
        assert_eq!(parse_value("true"), serde_json::json!(true));
    }
}
