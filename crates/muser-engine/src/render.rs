//! 离线渲染器：把 [`muser_core::Project`] 编译成音频并写出 WAV。
//!
//! 流程（纯离线，不碰音频设备）：
//! 1. 对每条未静音的音轨，按 `instrument` 解析出一件乐器（[`crate::instrument`]）。
//! 2. 把该轨所有 clip 里的音符，换算成**样本精确**的时间线事件（note_on/note_off）。
//! 3. 分块（block）推进处理：每块开始前触发到点的事件，再 `process` 出该块音频。
//! 4. 按音轨增益/声像混入主缓冲。
//! 5. 应用主增益，写出 16-bit 立体声 WAV。

use muser_core::Project;

use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, WavFormat, decibels, wav};

use crate::automate::{self, MasterLanes, TrackLanes};
use crate::instrument;

/// 渲染参数。
#[derive(Debug, Clone, Copy)]
pub struct RenderSettings {
    pub sample_rate: u32,
    pub block_size: usize,
    /// 工程结尾额外留出的余响时间（秒），让混响/长音自然收尾。
    pub tail_seconds: f64,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            block_size: 256,
            tail_seconds: 2.0,
        }
    }
}

/// 渲染层错误。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Core(#[from] muser_core::CoreError),
    #[error(transparent)]
    Instrument(#[from] instrument::UnknownInstrument),
    #[error(transparent)]
    Effect(#[from] crate::effects::EffectError),
    #[error("渲染失败: {0}")]
    Unstable(String),
    #[error("写出 WAV 失败: {0}")]
    Wav(String),
}

/// 时间线上一个样本精确的演奏动作。
#[derive(Debug, Clone, Copy)]
enum Act {
    On { key: u8, velocity: f32, accent: bool },
    Off { key: u8 },
}

/// 把一条音轨渲染进 `master`（已按声像分配到左右声道）。
fn render_track(
    track: &muser_core::Track,
    tempo: f64,
    settings: RenderSettings,
    total_frames: usize,
    mut lanes: TrackLanes,
    master: &mut AudioBuffer<f32>,
) -> Result<(), EngineError> {
    let spec = ProcessSpec {
        sample_rate: f64::from(settings.sample_rate),
        max_block_size: settings.block_size,
        num_channels: 2,
    };

    let mut inst = instrument::resolve_for(&track.instrument, &track.instrument_params, &spec)?;
    inst.prepare(&spec);
    let channel = inst.channel();

    // 收集全轨事件（样本帧, 动作），并按时间排序；同帧先 Off 后 On。
    let sr = f64::from(settings.sample_rate);
    let beat_at = |frame: usize| frame as f64 / sr * tempo / 60.0;
    let mut acts: Vec<(usize, Act)> = Vec::new();
    for clip in &track.clips {
        let clip_start = clip.start.to_seconds(tempo) * sr;
        let clip_len_frames = (clip.length.to_seconds(tempo) * sr) as usize;
        let clip_end_frame = clip_start as usize + clip_len_frames;
        for note in &clip.notes {
            let on = (clip_start + note.start.to_seconds(tempo) * sr) as usize;
            // 音符结束不超过 clip 边界（被裁剪）。
            let raw_off = (clip_start
                + (note.start + note.length).to_seconds(tempo) * sr) as usize;
            let off = raw_off.min(clip_end_frame);
            if off <= on {
                continue;
            }
            acts.push((on, Act::On { key: note.pitch, velocity: note.velocity, accent: note.accent }));
            acts.push((off, Act::Off { key: note.pitch }));
        }
    }
    // 排序：先按帧，再让 Off 排在 On 前面（避免同帧 On 被随即 Off）。
    acts.sort_by_key(|&(frame, act)| (frame, matches!(act, Act::On { .. })));

    // 该轨自己的渲染缓冲（立体声），渲染完再按增益/声像混入 master。
    let mut track_buf = AudioBuffer::<f32>::new(2, total_frames);
    let mut block = AudioBuffer::<f32>::new(2, settings.block_size);

    let (mut frame, mut next) = (0usize, 0usize);
    while frame < total_frames {
        // 触发所有到点（<= 当前帧）的事件。
        while let Some(&(at, act)) = acts.get(next) {
            if at > frame {
                break;
            }
            match act {
                Act::On { key, velocity, accent } => {
                    inst.note_on(NoteRef::new(channel, key), velocity, accent)
                }
                Act::Off { key } => inst.note_off(NoteRef::new(channel, key), 0.0),
            }
            next += 1;
        }
        // 乐器自动化：在块开始处写入当前值。
        for lane in lanes.instrument.iter_mut() {
            if let Some(v) = lane.poll(beat_at(frame)) {
                inst.set_param(&lane.param, v);
            }
        }
        // 这一块推进到：下一个事件帧 或 块大小 或 总长，三者取最小（但至少前进 1 帧）。
        // 有乐器自动化时块不超过 automate::BLOCK 帧。
        let step = if lanes.instrument.is_empty() { settings.block_size } else { settings.block_size.min(automate::BLOCK) };
        let until = acts.get(next).map_or(total_frames, |&(at, _)| at.min(total_frames));
        let end = (frame + step)
            .min(until)
            .max(frame + 1)
            .min(total_frames);
        block.set_num_frames(end - frame);
        inst.process(&mut block);
        for ch in 0..2 {
            track_buf.channel_mut(ch)[frame..end].copy_from_slice(block.channel(ch));
        }
        frame = end;
    }

    // 上游模型在极端参数下可能数值发散；宁可报错也不写出爆音。
    if (0..2).any(|ch| track_buf.channel(ch).iter().any(|s| !s.is_finite() || s.abs() > 64.0)) {
        return Err(EngineError::Unstable(format!(
            "音轨 '{}'（{}）输出数值发散，请调整 instrument_params（如增大 release / attack）",
            track.name, track.instrument
        )));
    }

    // 轨级效果链（在增益/声像之前施加）。
    if !track.effects.is_empty() {
        let mut chain = crate::effects::build_chain(&track.effects, &spec)?;
        crate::effects::apply_chain(&mut chain, &mut track_buf, settings.block_size, &mut lanes.effects, &beat_at);
    }

    // 增益 + 等功率声像，混入 master。有自动化时逐帧计算。
    let gains = |gain_db: f64, pan: f64| {
        let gain = decibels::db_to_gain(gain_db);
        // 等功率声像：angle 从 0(左) 到 π/2(右)。
        let angle = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5 * std::f64::consts::FRAC_PI_2;
        ((angle.cos() * gain) as f32, (angle.sin() * gain) as f32)
    };
    let fixed = lanes.gain.is_empty() && lanes.pan.is_empty();
    let (mut gl, mut gr) = gains(track.gain_db, track.pan);
    for frame in 0..total_frames {
        if !fixed {
            let beat = beat_at(frame);
            (gl, gr) = gains(
                automate::scalar(&lanes.gain, beat, track.gain_db),
                automate::scalar(&lanes.pan, beat, track.pan),
            );
        }
        master.channel_mut(0)[frame] += track_buf.channel(0)[frame] * gl;
        master.channel_mut(1)[frame] += track_buf.channel(1)[frame] * gr;
    }
    Ok(())
}

/// 把工程渲染成一块立体声音频缓冲。
pub fn render(project: &Project, settings: RenderSettings) -> Result<AudioBuffer<f32>, EngineError> {
    crate::validate(project)?;

    let sr = f64::from(settings.sample_rate);
    let duration = project.duration_beats().to_seconds(project.tempo);
    let total_frames =
        ((duration + settings.tail_seconds) * sr).ceil() as usize + 1;

    let mut master = AudioBuffer::<f32>::new(2, total_frames);
    let beats = automate::total_beats(project, settings.tail_seconds);
    let beat_at = |frame: usize| frame as f64 / sr * project.tempo / 60.0;

    for (ti, track) in project.tracks.iter().enumerate() {
        if track.muted {
            continue;
        }
        let lanes = automate::compile_track(project, ti, beats).map_err(invalid)?;
        render_track(track, project.tempo, settings, total_frames, lanes, &mut master)?;
    }
    let MasterLanes { effects: mut master_fx_lanes, gain: master_gain_lanes } =
        automate::compile_master(project, beats).map_err(invalid)?;

    // 主总线效果链（在主增益之前施加）。
    if !project.master_effects.is_empty() {
        let spec = ProcessSpec {
            sample_rate: sr,
            max_block_size: settings.block_size,
            num_channels: 2,
        };
        let mut chain = crate::effects::build_chain(&project.master_effects, &spec)?;
        crate::effects::apply_chain(&mut chain, &mut master, settings.block_size, &mut master_fx_lanes, &beat_at);
    }

    // 主增益（有自动化时逐帧）。
    if master_gain_lanes.is_empty() {
        let mg = decibels::db_to_gain(project.master_gain_db) as f32;
        if (mg - 1.0).abs() > f32::EPSILON {
            master.apply_gain(mg);
        }
    } else {
        for frame in 0..total_frames {
            let db = automate::scalar(&master_gain_lanes, beat_at(frame), project.master_gain_db);
            let g = decibels::db_to_gain(db) as f32;
            for ch in 0..2 {
                master.channel_mut(ch)[frame] *= g;
            }
        }
    }
    Ok(master)
}

fn invalid(m: String) -> EngineError {
    EngineError::Core(muser_core::CoreError::Invalid(m))
}

/// 渲染工程并写出 WAV 文件。返回 (时长秒, 峰值 dBFS)。
pub fn render_to_wav(
    project: &Project,
    path: &str,
    settings: RenderSettings,
) -> Result<(f64, f64), EngineError> {
    let audio = render(project, settings)?;
    wav::write_file(path, &audio, settings.sample_rate, WavFormat::Int16)
        .map_err(|e| EngineError::Wav(format!("{e:?}")))?;
    let seconds = audio.num_frames() as f64 / f64::from(settings.sample_rate);
    let peak_db = decibels::gain_to_db(f64::from(audio.peak()));
    Ok((seconds, peak_db))
}
