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
use std::sync::Arc;

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
    advance(buf, effects::chain_latency(&chain));
    Ok(())
}

/// 延迟补偿：把信号前移 `latency` 帧。
fn advance(buf: &mut AudioBuffer<f32>, latency: usize) {
    if latency == 0 {
        return;
    }
    let n = buf.num_frames();
    let l = latency.min(n);
    for ch in 0..2 {
        let c = buf.channel_mut(ch);
        c.copy_within(l.., 0);
        c[n - l..].fill(0.0);
    }
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

/// 乐器输出缓存：同一条音轨的乐器部分（音符、乐器参数与乐器自动化都没变）不必重新渲染。
///
/// 渲染里最耗时的是乐器；改音量、静音、效果、路由时只需重跑效果链与总线。`muser view` 的联动服务
/// 用它把改动后的重新渲染从几十秒降到一两秒。缓存只保留最近一次渲染用到的条目（含静音音轨，取消静音不必重算）。
#[derive(Default)]
pub struct RenderCache {
    map: HashMap<u64, Arc<Cached>>,
}

/// 裁掉首尾静音后的乐器输出。
struct Cached {
    offset: usize,
    audio: AudioBuffer<f32>,
}

impl RenderCache {
    pub fn new() -> Self {
        Self::default()
    }
    /// 条目数。
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// 缓存占用的内存（字节，近似）。
    pub fn bytes(&self) -> usize {
        self.map.values().map(|c| c.audio.num_frames() * 2 * 4).sum()
    }
}

impl Cached {
    fn new(full: AudioBuffer<f32>) -> Cached {
        let n = full.num_frames();
        let live = |f: usize| full.channel(0)[f] != 0.0 || full.channel(1)[f] != 0.0;
        let first = (0..n).find(|&f| live(f));
        let Some(first) = first else {
            return Cached { offset: 0, audio: AudioBuffer::<f32>::new(2, 0) };
        };
        let last = (first..n).rev().find(|&f| live(f)).unwrap_or(first);
        let mut audio = AudioBuffer::<f32>::new(2, last + 1 - first);
        for ch in 0..2 {
            audio.channel_mut(ch).copy_from_slice(&full.channel(ch)[first..=last]);
        }
        Cached { offset: first, audio }
    }

    fn expand(&self, total_frames: usize) -> AudioBuffer<f32> {
        let mut out = AudioBuffer::<f32>::new(2, total_frames);
        let n = self.audio.num_frames().min(total_frames.saturating_sub(self.offset));
        for ch in 0..2 {
            out.channel_mut(ch)[self.offset..self.offset + n].copy_from_slice(&self.audio.channel(ch)[..n]);
        }
        out
    }
}

/// 一条音轨乐器输出的缓存键：只含影响乐器声音的内容。
fn instrument_key(project: &Project, ti: usize, cx: &Ctx) -> u64 {
    use std::hash::{Hash, Hasher};
    let t = &project.tracks[ti];
    // 乐器自动化：随机形状的默认种子与音轨下标、自动化序号有关，一并计入。
    let autos: Vec<(usize, &muser_core::Automation)> =
        t.automation.iter().enumerate().filter(|(_, a)| a.target.trim() == "instrument").collect();
    // 状态文件内容可能变化：计入修改时间。
    let state_mtime = t
        .instrument_params
        .get("@state")
        .and_then(|v| v.as_str())
        .and_then(|p| std::fs::metadata(&*crate::plugin::expand_home(p)).and_then(|m| m.modified()).ok())
        .map(|m| format!("{m:?}"));
    let ts = project.time_signature;
    let key = serde_json::json!({
        "i": t.instrument, "p": t.instrument_params, "c": t.clips, "a": autos.iter().map(|(i, a)| serde_json::json!([i, a])).collect::<Vec<_>>(),
        "ti": ti, "seed": project.seed, "tempo": project.tempo, "ts": [ts.numerator, ts.denominator],
        "sr": cx.settings.sample_rate, "bs": cx.settings.block_size, "n": cx.total_frames, "st": state_mtime,
    });
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.to_string().hash(&mut h);
    h.finish()
}

/// 把工程渲染成一块立体声音频缓冲。
pub fn render(project: &Project, settings: RenderSettings) -> Result<AudioBuffer<f32>, EngineError> {
    render_cached(project, settings, &mut RenderCache::new())
}

/// 电平表的窗口长度（帧）。
pub const METER_WINDOW: usize = 2048;

/// 一个节点（音轨/总线）推子后输出的电平：每 [`METER_WINDOW`] 帧一组 `[左峰值, 右峰值]`（线性）。
/// 静音的节点没有电平。
pub struct Meter {
    pub node: Node,
    pub peaks: Vec<[f32; 2]>,
}

fn meter_of(buf: &AudioBuffer<f32>) -> Vec<[f32; 2]> {
    let n = buf.num_frames();
    (0..n.div_ceil(METER_WINDOW))
        .map(|w| {
            let r = w * METER_WINDOW..((w + 1) * METER_WINDOW).min(n);
            let pk = |ch: usize| buf.channel(ch)[r.clone()].iter().fold(0f32, |m, s| m.max(s.abs()));
            [pk(0), pk(1)]
        })
        .collect()
}

/// 同 [`render`]，复用 `cache` 里的乐器输出，并把本次的乐器输出存回。
pub fn render_cached(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<AudioBuffer<f32>, EngineError> {
    render_inner(project, settings, cache, None)
}

/// 同 [`render_cached`]，同时给出每个节点的电平（供播放界面的电平表）。
pub fn render_metered(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<(AudioBuffer<f32>, Vec<Meter>), EngineError> {
    let mut meters = Vec::new();
    let audio = render_inner(project, settings, cache, Some(&mut meters))?;
    Ok((audio, meters))
}

fn render_inner(
    project: &Project,
    settings: RenderSettings,
    cache: &mut RenderCache,
    mut meters: Option<&mut Vec<Meter>>,
) -> Result<AudioBuffer<f32>, EngineError> {
    crate::validate(project)?;
    // 近乎静音的尾巴会产生非规格化浮点数，让滤波、混响慢几十倍；渲染期间把它们当作 0。
    let _quiet = manguebyte::NoDenormals::new();
    let ts = project.time_signature;
    let _transport = crate::transport::set(project.tempo, u16::from(ts.numerator), u16::from(ts.denominator));
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

    // 乐器部分：缺的先渲染（内置乐器多线程并行；插件乐器在当前线程依次渲染）。
    let track_keys: Vec<u64> = (0..project.tracks.len()).map(|i| instrument_key(project, i, &cx)).collect();
    let needed: Vec<usize> = (0..project.tracks.len())
        .filter(|&i| !project.tracks[i].muted || key_nodes.contains(&Node::Track(i)))
        .filter(|&i| !cache.map.contains_key(&track_keys[i]))
        .collect();
    let render_one = |i: usize| -> Result<(usize, Cached), EngineError> {
        let _t = crate::transport::set(project.tempo, u16::from(ts.numerator), u16::from(ts.denominator));
        let _quiet = manguebyte::NoDenormals::new();
        let mut lanes = automate::compile_track(project, i, beats).map_err(invalid)?;
        Ok((i, Cached::new(render_instrument(&project.tracks[i], &cx, &mut lanes)?)))
    };
    let is_plugin = |i: &usize| project.tracks[*i].instrument.trim().starts_with("plugin:");
    let (plugins, builtin): (Vec<usize>, Vec<usize>) = needed.into_iter().partition(is_plugin);
    let mut done: Vec<Result<(usize, Cached), EngineError>> = Vec::new();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(builtin.len().max(1));
    let queue = std::sync::Mutex::new(builtin.iter().copied());
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        // 先取出下标再释放锁，渲染期间不持锁。
                        let next = queue.lock().unwrap().next();
                        let Some(i) = next else { break };
                        out.push(render_one(i));
                    }
                    out
                })
            })
            .collect();
        for i in &plugins {
            done.push(render_one(*i));
        }
        for h in handles {
            done.extend(h.join().expect("渲染线程崩溃"));
        }
    });
    for r in done {
        let (i, c) = r?;
        cache.map.insert(track_keys[i], Arc::new(c));
    }
    cache.map.retain(|k, _| track_keys.contains(k));

    let mut master = AudioBuffer::<f32>::new(2, total_frames);
    let mut bus_in: HashMap<usize, AudioBuffer<f32>> = HashMap::new();
    let mut keys: HashMap<Node, AudioBuffer<f32>> = HashMap::new();

    // 按依赖分层：同一层的节点互不依赖（输入都来自更早的层），可以并行处理；
    // 分发（送进总线/主总线、保留侧链键）仍按计划顺序依次做，结果与串行一致。
    let mut level: HashMap<Node, usize> = HashMap::new();
    for &node in &plan.order {
        let mut l = 0;
        for e in plan.edges.iter().filter(|e| e.to.map(Node::Bus) == Some(node)) {
            l = l.max(level.get(&e.from).map_or(0, |x| x + 1));
        }
        for sc in plan.sidechains.iter().filter(|s| s.node == node) {
            l = l.max(level.get(&sc.key).map_or(0, |x| x + 1));
        }
        level.insert(node, l);
    }
    let depth = level.values().copied().max().map_or(0, |d| d + 1);
    let node_info = |node: Node| -> (bool, &[Effect]) {
        match node {
            Node::Track(i) => (project.tracks[i].muted, &project.tracks[i].effects[..]),
            Node::Bus(i) => (project.buses[i].muted, &project.buses[i].effects[..]),
        }
    };
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());

    for l in 0..depth {
        // 本层要处理的节点（静音且不作为侧链键的跳过，其输入丢弃）。
        let mut jobs: Vec<(Node, Option<AudioBuffer<f32>>)> = Vec::new();
        for &node in plan.order.iter().filter(|n| level[n] == l) {
            let (muted, _) = node_info(node);
            let input = match node {
                Node::Bus(i) => Some(bus_in.remove(&i).unwrap_or_else(|| AudioBuffer::<f32>::new(2, total_frames))),
                Node::Track(_) => None,
            };
            if muted && !key_nodes.contains(&node) {
                continue;
            }
            jobs.push((node, input));
        }
        let process = |node: Node, input: Option<AudioBuffer<f32>>| -> Result<(AudioBuffer<f32>, TrackLanes), EngineError> {
            let _t = crate::transport::set(project.tempo, u16::from(ts.numerator), u16::from(ts.denominator));
            let _quiet = manguebyte::NoDenormals::new();
            let (_, effects_list) = node_info(node);
            let (gain_db, pan, mut lanes, law): (f64, f64, TrackLanes, fn(f64, f64) -> (f32, f32)) = match node {
                Node::Track(i) => {
                    let t = &project.tracks[i];
                    (t.gain_db, t.pan, automate::compile_track(project, i, beats).map_err(invalid)?, pan_gains)
                }
                Node::Bus(i) => {
                    let b = &project.buses[i];
                    (b.gain_db, b.pan, automate::compile_bus(project, i, beats).map_err(invalid)?, balance_gains)
                }
            };
            let mut buf = match (node, input) {
                (_, Some(b)) => b,
                (Node::Track(i), None) => cache.map[&track_keys[i]].expand(total_frames),
                (Node::Bus(_), None) => AudioBuffer::<f32>::new(2, total_frames),
            };
            let chain_keys: Vec<Option<&AudioBuffer<f32>>> = (0..effects_list.len())
                .map(|ei| plan.sidechains.iter().find(|s| s.node == node && s.effect == ei).and_then(|s| keys.get(&s.key)))
                .collect();
            run_chain(effects_list, &mut buf, &cx, &mut lanes.effects, &chain_keys)?;
            fader(&mut buf, &cx, gain_db, pan, &lanes, law);
            Ok((buf, lanes))
        };
        // 插件效果器在当前线程处理；其余分批并行（每批不超过线程数，限制同时占用的内存）。
        let has_plugin = |node: Node| node_info(node).1.iter().any(|e| matches!(e, Effect::Plugin { .. }));
        let mut results: Vec<(Node, Result<(AudioBuffer<f32>, TrackLanes), EngineError>)> = Vec::new();
        let (serial, parallel): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|(n, _)| has_plugin(*n));
        let mut parallel = parallel.into_iter().peekable();
        while parallel.peek().is_some() {
            let batch: Vec<_> = parallel.by_ref().take(workers).collect();
            let done: Vec<_> = std::thread::scope(|sc| {
                let hs: Vec<_> = batch.into_iter().map(|(n, input)| (n, sc.spawn(move || process(n, input)))).collect();
                hs.into_iter().map(|(n, h)| (n, h.join().expect("渲染线程崩溃"))).collect()
            });
            results.extend(done);
        }
        for (n, input) in serial {
            results.push((n, process(n, input)));
        }
        // 按计划顺序分发。
        let mut by_node: HashMap<Node, (AudioBuffer<f32>, TrackLanes)> = HashMap::new();
        for (n, r) in results {
            by_node.insert(n, r?);
        }
        for &node in plan.order.iter().filter(|n| level[n] == l) {
            let Some((buf, lanes)) = by_node.remove(&node) else { continue };
            let (muted, _) = node_info(node);
            if let (Some(m), false) = (meters.as_deref_mut(), muted) {
                m.push(Meter { node, peaks: meter_of(&buf) });
            }
            if !muted {
                for e in plan.edges.iter().filter(|e| e.from == node) {
                    let send_lanes = e.send.as_ref().and_then(|s| lanes.sends.get(s));
                    let dst = match e.to {
                        None => &mut master,
                        Some(b) => bus_in.entry(b).or_insert_with(|| AudioBuffer::<f32>::new(2, total_frames)),
                    };
                    mix_into(dst, &buf, &cx, e.level_db, send_lanes);
                }
            }
            if key_nodes.contains(&node) {
                keys.insert(node, buf);
            }
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
