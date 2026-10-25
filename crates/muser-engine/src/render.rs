//! 离线渲染器：把 [`muser_core::Project`] 编译成音频并写出 WAV。
//!
//! 流程（纯离线，不碰音频设备）：
//! 1. 按路由计划（[`muser_core::routing::plan`]）的顺序处理每个节点：先音轨，后汇集它们的总线。
//! 2. 音轨：乐器按样本精确的音符事件渲染 → 轨效果链 → 增益/声像，得到推子后信号。
//! 3. 总线：送进来的信号求和 → 总线效果链 → 增益/平衡。
//! 4. 每个节点的推子后信号送到它的 `output`（主总线或某条总线），并按 `sends` 的量各送一份。
//!    被侧链引用的节点保留一份推子后信号，作为压缩器的触发信号。
//! 5. 主总线：求和 → `master_effects` → 主增益，写出 16-bit 立体声 WAV。
//!
//! 有延迟的效果器（limiter 前瞻、插件）处理后把信号前移相应帧数，各路信号保持对齐。

use std::collections::{HashMap, HashSet};

use muser_core::routing::{self, Node, Plan};
use muser_core::{Effect, Project};

use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, WavFormat, decibels, wav};

use crate::automate::{self, MasterLanes, TrackLanes};
use crate::effects::{self, Fx};
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

struct Ctx {
    tempo: f64,
    sr: f64,
    settings: RenderSettings,
    total_frames: usize,
    spec: ProcessSpec,
}

impl Ctx {
    fn beat_at(&self, frame: usize) -> f64 {
        frame as f64 / self.sr * self.tempo / 60.0
    }
}

/// 渲染一条音轨的乐器部分（干声）。
fn render_instrument(track: &muser_core::Track, cx: &Ctx, lanes: &mut TrackLanes) -> Result<AudioBuffer<f32>, EngineError> {
    let (settings, total_frames, sr, tempo) = (cx.settings, cx.total_frames, cx.sr, cx.tempo);
    let mut inst = instrument::resolve_for(&track.instrument, &track.instrument_params, &cx.spec)?;
    inst.prepare(&cx.spec);
    let channel = inst.channel();

    // 收集全轨事件（样本帧, 动作），并按时间排序；同帧先 Off 后 On。
    let mut acts: Vec<(usize, Act)> = Vec::new();
    for clip in &track.clips {
        let clip_start = clip.start.to_seconds(tempo) * sr;
        let clip_len_frames = (clip.length.to_seconds(tempo) * sr) as usize;
        let clip_end_frame = clip_start as usize + clip_len_frames;
        for note in &clip.notes {
            let on = (clip_start + note.start.to_seconds(tempo) * sr) as usize;
            // 音符结束不超过 clip 边界（被裁剪）。
            let raw_off = (clip_start + (note.start + note.length).to_seconds(tempo) * sr) as usize;
            let off = raw_off.min(clip_end_frame);
            if off <= on {
                continue;
            }
            acts.push((on, Act::On { key: note.pitch, velocity: note.velocity, accent: note.accent }));
            acts.push((off, Act::Off { key: note.pitch }));
        }
    }
    acts.sort_by_key(|&(frame, act)| (frame, matches!(act, Act::On { .. })));

    let mut out = AudioBuffer::<f32>::new(2, total_frames);
    let mut block = AudioBuffer::<f32>::new(2, settings.block_size);
    let (mut frame, mut next) = (0usize, 0usize);
    while frame < total_frames {
        while let Some(&(at, act)) = acts.get(next) {
            if at > frame {
                break;
            }
            match act {
                Act::On { key, velocity, accent } => inst.note_on(NoteRef::new(channel, key), velocity, accent),
                Act::Off { key } => inst.note_off(NoteRef::new(channel, key), 0.0),
            }
            next += 1;
        }
        for lane in lanes.instrument.iter_mut() {
            if let Some(v) = lane.poll(cx.beat_at(frame)) {
                inst.set_param(&lane.param, v);
            }
        }
        let step = if lanes.instrument.is_empty() { settings.block_size } else { settings.block_size.min(automate::BLOCK) };
        let until = acts.get(next).map_or(total_frames, |&(at, _)| at.min(total_frames));
        let end = (frame + step).min(until).max(frame + 1).min(total_frames);
        block.set_num_frames(end - frame);
        inst.process(&mut block);
        for ch in 0..2 {
            out.channel_mut(ch)[frame..end].copy_from_slice(block.channel(ch));
        }
        frame = end;
    }

    // 上游模型在极端参数下可能数值发散；宁可报错也不写出爆音。
    if (0..2).any(|ch| out.channel(ch).iter().any(|s| !s.is_finite() || s.abs() > 64.0)) {
        return Err(EngineError::Unstable(format!(
            "音轨 '{}'（{}）输出数值发散，请调整 instrument_params（如增大 release / attack）",
            track.name, track.instrument
        )));
    }
    Ok(out)
}

/// 施加效果链；`keys[i]` 为第 i 个效果器的侧链触发信号。处理后补偿链路延迟。
fn run_chain(
    fx: &[Effect],
    buf: &mut AudioBuffer<f32>,
    cx: &Ctx,
    lanes: &mut [Vec<automate::Lane>],
    keys: &[Option<&AudioBuffer<f32>>],
) -> Result<(), EngineError> {
    if fx.is_empty() {
        return Ok(());
    }
    let mut chain: Vec<Fx> = effects::build_chain(fx, &cx.spec)?;
    let beat_at = |f: usize| cx.beat_at(f);
    effects::apply_chain(&mut chain, buf, cx.settings.block_size, lanes, &beat_at, keys);
    let latency = effects::chain_latency(&chain);
    if latency > 0 {
        let n = buf.num_frames();
        for ch in 0..2 {
            let c = buf.channel_mut(ch);
            let l = latency.min(n);
            c.copy_within(l.., 0);
            c[n - l..].fill(0.0);
        }
    }
    Ok(())
}

/// 等功率声像（音轨）：居中时两声道各 -3 dB。
fn pan_gains(gain_db: f64, pan: f64) -> (f32, f32) {
    let gain = decibels::db_to_gain(gain_db);
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5 * std::f64::consts::FRAC_PI_2;
    ((angle.cos() * gain) as f32, (angle.sin() * gain) as f32)
}

/// 平衡（总线，已是立体声）：居中不衰减，偏向一侧时减弱另一侧。
fn balance_gains(gain_db: f64, pan: f64) -> (f32, f32) {
    let g = decibels::db_to_gain(gain_db) as f32;
    let p = pan.clamp(-1.0, 1.0) as f32;
    (g * (1.0 - p.max(0.0)), g * (1.0 + p.min(0.0)))
}

/// 推子：增益与声像（有自动化时逐帧）。
fn fader(buf: &mut AudioBuffer<f32>, cx: &Ctx, gain_db: f64, pan: f64, lanes: &TrackLanes, law: fn(f64, f64) -> (f32, f32)) {
    let fixed = lanes.gain.is_empty() && lanes.pan.is_empty();
    let (mut gl, mut gr) = law(gain_db, pan);
    for frame in 0..buf.num_frames() {
        if !fixed {
            let beat = cx.beat_at(frame);
            (gl, gr) = law(automate::scalar(&lanes.gain, beat, gain_db), automate::scalar(&lanes.pan, beat, pan));
        }
        buf.channel_mut(0)[frame] *= gl;
        buf.channel_mut(1)[frame] *= gr;
    }
}

/// 把 `src` 按 `level_db`（可被 lanes 自动化）加进 `dst`。
fn mix_into(dst: &mut AudioBuffer<f32>, src: &AudioBuffer<f32>, cx: &Ctx, level_db: f64, lanes: Option<&Vec<automate::Lane>>) {
    match lanes.filter(|l| !l.is_empty()) {
        None => {
            let g = decibels::db_to_gain(level_db) as f32;
            for ch in 0..2 {
                for (d, s) in dst.channel_mut(ch).iter_mut().zip(src.channel(ch)) {
                    *d += s * g;
                }
            }
        }
        Some(l) => {
            for frame in 0..src.num_frames() {
                let g = decibels::db_to_gain(automate::scalar(l, cx.beat_at(frame), level_db)) as f32;
                for ch in 0..2 {
                    dst.channel_mut(ch)[frame] += src.channel(ch)[frame] * g;
                }
            }
        }
    }
}

/// 分轨（stem）：一个直接进主总线的音轨或总线，在主总线效果与主增益之前送进主总线的信号。
/// 所有 stem 相加 = 主总线效果链的输入。
pub struct Stem {
    pub node: Node,
    /// 显示名：音轨名（空名时 `tracks[i]`）或总线名。
    pub label: String,
    pub audio: AudioBuffer<f32>,
}

/// 成品混音 + 分轨。
pub struct Rendered {
    pub mix: AudioBuffer<f32>,
    pub stems: Vec<Stem>,
}

/// 把工程渲染成一块立体声音频缓冲。
pub fn render(project: &Project, settings: RenderSettings) -> Result<AudioBuffer<f32>, EngineError> {
    Ok(render_graph(project, settings, false)?.mix)
}

/// 渲染成品混音，同时给出各分轨（见 [`Stem`]）。
pub fn render_with_stems(project: &Project, settings: RenderSettings) -> Result<Rendered, EngineError> {
    render_graph(project, settings, true)
}

fn render_graph(project: &Project, settings: RenderSettings, capture: bool) -> Result<Rendered, EngineError> {
    crate::validate(project)?;
    let plan: Plan = routing::plan(project).map_err(invalid)?;

    let sr = f64::from(settings.sample_rate);
    let duration = project.duration_beats().to_seconds(project.tempo);
    let total_frames = ((duration + settings.tail_seconds) * sr).ceil() as usize + 1;
    let cx = Ctx {
        tempo: project.tempo,
        sr,
        settings,
        total_frames,
        spec: ProcessSpec { sample_rate: sr, max_block_size: settings.block_size, num_channels: 2 },
    };
    let beats = automate::total_beats(project, settings.tail_seconds);

    // 需要保留推子后信号的节点：被侧链引用的。
    let mut key_nodes: HashSet<Node> = plan.sidechains.iter().map(|s| s.key).collect();
    let mut master_keys: Vec<Option<Node>> = Vec::new();
    for e in &project.master_effects {
        let k = match e {
            Effect::Compressor { sidechain: Some(name), .. } => Some(routing::find_node(project, name).map_err(invalid)?),
            _ => None,
        };
        if let Some(k) = k {
            key_nodes.insert(k);
        }
        master_keys.push(k);
    }

    let mut master = AudioBuffer::<f32>::new(2, total_frames);
    let mut bus_in: HashMap<usize, AudioBuffer<f32>> = HashMap::new();
    let mut keys: HashMap<Node, AudioBuffer<f32>> = HashMap::new();
    let mut stems: Vec<Stem> = Vec::new();

    for &node in &plan.order {
        let (muted, effects_list, gain_db, pan, mut lanes, law): (bool, &[Effect], f64, f64, TrackLanes, fn(f64, f64) -> (f32, f32)) =
            match node {
                Node::Track(i) => {
                    let t = &project.tracks[i];
                    (t.muted, &t.effects, t.gain_db, t.pan, automate::compile_track(project, i, beats).map_err(invalid)?, pan_gains)
                }
                Node::Bus(i) => {
                    let b = &project.buses[i];
                    (b.muted, &b.effects, b.gain_db, b.pan, automate::compile_bus(project, i, beats).map_err(invalid)?, balance_gains)
                }
            };
        // 静音且不作为侧链键的节点：完全跳过（静音节点的输入照常丢弃）。
        if muted && !key_nodes.contains(&node) {
            if let Node::Bus(i) = node {
                bus_in.remove(&i);
            }
            continue;
        }
        let mut buf = match node {
            Node::Track(i) => render_instrument(&project.tracks[i], &cx, &mut lanes)?,
            Node::Bus(i) => bus_in.remove(&i).unwrap_or_else(|| AudioBuffer::<f32>::new(2, total_frames)),
        };
        let chain_keys: Vec<Option<&AudioBuffer<f32>>> = (0..effects_list.len())
            .map(|ei| {
                plan.sidechains
                    .iter()
                    .find(|s| s.node == node && s.effect == ei)
                    .and_then(|s| keys.get(&s.key))
            })
            .collect();
        run_chain(effects_list, &mut buf, &cx, &mut lanes.effects, &chain_keys)?;
        drop(chain_keys);
        fader(&mut buf, &cx, gain_db, pan, &lanes, law);

        if !muted {
            for e in plan.edges.iter().filter(|e| e.from == node) {
                let dst = match e.to {
                    None => &mut master,
                    Some(b) => bus_in.entry(b).or_insert_with(|| AudioBuffer::<f32>::new(2, total_frames)),
                };
                let send_lanes = e.send.as_ref().and_then(|s| lanes.sends.get(s));
                mix_into(dst, &buf, &cx, e.level_db, send_lanes);
                if capture && e.to.is_none() {
                    let mut audio = AudioBuffer::<f32>::new(2, total_frames);
                    mix_into(&mut audio, &buf, &cx, e.level_db, send_lanes);
                    let label = match node {
                        Node::Track(i) if project.tracks[i].name.is_empty() => format!("tracks[{i}]"),
                        Node::Track(i) => project.tracks[i].name.clone(),
                        Node::Bus(i) => project.buses[i].name.clone(),
                    };
                    stems.push(Stem { node, label, audio });
                }
            }
        }
        if key_nodes.contains(&node) {
            keys.insert(node, buf);
        }
    }

    // 侧链键只被比它晚处理的节点使用；主总线用到的保留到最后。
    keys.retain(|n, _| master_keys.contains(&Some(*n)));

    let MasterLanes { effects: mut master_fx_lanes, gain: master_gain_lanes } =
        automate::compile_master(project, beats).map_err(invalid)?;
    let mk: Vec<Option<&AudioBuffer<f32>>> = master_keys.iter().map(|k| k.and_then(|n| keys.get(&n))).collect();
    run_chain(&project.master_effects, &mut master, &cx, &mut master_fx_lanes, &mk)?;

    // 主增益（有自动化时逐帧）。
    if master_gain_lanes.is_empty() {
        let mg = decibels::db_to_gain(project.master_gain_db) as f32;
        if (mg - 1.0).abs() > f32::EPSILON {
            master.apply_gain(mg);
        }
    } else {
        for frame in 0..total_frames {
            let db = automate::scalar(&master_gain_lanes, cx.beat_at(frame), project.master_gain_db);
            let g = decibels::db_to_gain(db) as f32;
            for ch in 0..2 {
                master.channel_mut(ch)[frame] *= g;
            }
        }
    }
    Ok(Rendered { mix: master, stems })
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
