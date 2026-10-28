//! 渲染器：把 [`muser_core::Project`] 编译成音频。
//!
//! 核心是流式的 [`Renderer`]：每次往前推进一段，逐块走完整张路由图，可以边渲染边播放。
//! 离线渲染（[`render`] / WAV）就是从头到尾调用它，两者结果逐样本一致。
//!
//! 每一段的处理：
//! 1. 按路由计划（[`muser_core::routing::plan`]）的顺序处理每个节点：先音轨，后汇集它们的总线。
//! 2. 音轨：乐器按样本精确的音符事件渲染 → 轨效果链 → 增益/声像，得到推子后信号。
//! 3. 总线：送进来的信号求和 → 总线效果链 → 增益/平衡。
//! 4. 每个节点的推子后信号送到它的 `output`（主总线或某条总线），并按 `sends` 的量各送一份。
//!    被侧链引用的节点的推子后信号同时作为压缩器的触发信号。
//! 5. 主总线：求和 → `master_effects` → 主增益。
//!
//! 有延迟的效果器（limiter 前瞻、插件）：节点的输出取效果链输出往后 `latency` 帧的样本（链路先多算这么多），
//! 各路信号保持对齐。
//!
//! 效果链按从工程开头算起、固定步长的块处理；流式推进的每一段都对齐到这些块的公倍数，
//! 所以分块方式与一次处理整首完全相同。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use muser_core::routing::{self, Node, Plan};
use muser_core::{Beats, Effect, Project};

use manguebyte::analysis::LoudnessMeter;
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
pub(crate) enum Act {
    On { key: u8, velocity: f32, accent: bool },
    Off { key: u8 },
    Pedal { down: bool },
}

#[derive(Clone)]
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

/// 一条音轨的全部演奏动作（样本帧, 动作），按时间排序；同帧先 Off 后 On。
fn schedule(project: &Project, ti: usize, cx: &Ctx) -> Vec<(usize, Act)> {
    let track = &project.tracks[ti];
    let (sr, tempo) = (cx.sr, cx.tempo);
    let mut acts: Vec<(usize, Act)> = Vec::new();
    for (ci, clip) in track.clips.iter().enumerate() {
        // notes 加上 abc、grid、drummer 展开的音符（工程校验时已检查过写法）。
        let all = crate::clip_notes(project, ti, ci).unwrap_or_default();
        let notes: &[muser_core::Note] = &all;
        let clip_start = clip.start.to_seconds(tempo) * sr;
        let clip_len_frames = (clip.length.to_seconds(tempo) * sr) as usize;
        let clip_end_frame = clip_start as usize + clip_len_frames;
        // 连奏：延长到下一个不同音高的音之后约 40 ms（空隙超过 1/4 拍视为休止，不连）。
        let legato_len = |i: usize| -> Option<f64> {
            let n = &notes[i];
            let end = n.start.0 + n.length.0;
            let next = notes.iter().filter(|m| m.start.0 > n.start.0 + 1e-9).map(|m| m.start.0).fold(f64::INFINITY, f64::min);
            if !next.is_finite() || next - end > 0.25 {
                return None;
            }
            let at_next: Vec<&muser_core::Note> = notes.iter().filter(|m| (m.start.0 - next).abs() < 1e-9).collect();
            if at_next.iter().any(|m| m.pitch == n.pitch) {
                return None;
            }
            let overlap = 0.04 * tempo / 60.0;
            Some((next - n.start.0 + overlap).max(n.length.0))
        };
        let lengths: Vec<Beats> =
            (0..notes.len()).map(|ni| if clip.legato { legato_len(ni).map_or(notes[ni].length, Beats) } else { notes[ni].length }).collect();
        // 人性化：clip 的覆盖音轨的；种子默认由工程 seed 与音轨、clip 位置推导。
        // drummer 的 clip 没写 humanize 时用底子 loop 的律动。
        let loop_feel = match (&clip.humanize, &clip.drummer) {
            (None, Some(d)) => crate::loop_library::load(&d.base).ok().and_then(|l| l.humanize),
            _ => None,
        };
        let clip_feel = clip.humanize.as_ref().or(loop_feel.as_ref());
        let performed = muser_core::humanize::Humanize::merged(track.humanize.as_ref(), clip_feel).map(|h| {
            let seed = h.seed.unwrap_or_else(|| muser_core::automation::derive_seed(project.seed, Some(ti), 0x4855_0000 + ci));
            let lens: Vec<f64> = lengths.iter().map(|l| l.0).collect();
            muser_core::humanize::perform(&h, notes, &lens, clip.start.0, seed)
        });
        for (ni, note) in notes.iter().enumerate() {
            let (on, raw_off, velocity) = match &performed {
                None => {
                    let note = &muser_core::Note { length: lengths[ni], ..note.clone() };
                    let on = (clip_start + note.start.to_seconds(tempo) * sr) as usize;
                    (on, (clip_start + (note.start + note.length).to_seconds(tempo) * sr) as usize, note.velocity)
                }
                Some(p) => {
                    let p = p[ni];
                    let at = |beats: f64| (clip_start + (Beats(beats).to_seconds(tempo) + p.offset_s) * sr).max(0.0) as usize;
                    (at(p.start), at(p.start + p.length), p.velocity)
                }
            };
            // 音符结束不超过 clip 边界（被裁剪）。
            let off = raw_off.min(clip_end_frame);
            if off <= on {
                continue;
            }
            acts.push((on, Act::On { key: note.pitch, velocity, accent: note.accent }));
            acts.push((off, Act::Off { key: note.pitch }));
        }
        for &(a, b) in &clip.pedal {
            let down = (clip_start + Beats(a).to_seconds(tempo) * sr) as usize;
            let up = ((clip_start + Beats(b).to_seconds(tempo) * sr) as usize).min(clip_end_frame);
            if up > down {
                acts.push((down, Act::Pedal { down: true }));
                acts.push((up, Act::Pedal { down: false }));
            }
        }
    }
    // 同一帧：先松键，再抬踏板，再踩踏板，最后按新音（换踏板时新音不被旧踏板吞掉，松开的旧音在抬踏板时停）。
    acts.sort_by_key(|&(frame, act)| {
        let order = match act {
            Act::Off { .. } => 0,
            Act::Pedal { down: false } => 1,
            Act::Pedal { down: true } => 2,
            Act::On { .. } => 3,
        };
        (frame, order)
    });
    acts
}

/// 一条音轨的演奏动作，以 MIDI tick（每四分音符 `tpq` 个）计时。与渲染用的完全相同
/// （humanize、连奏、clip 裁剪、踏板），只是把"每秒多少帧"换成"每秒多少 tick"。
pub(crate) fn schedule_ticks(project: &Project, ti: usize, tpq: u16) -> Vec<(usize, Act)> {
    // 渲染按帧截断取整；拍 → 秒 → tick 的浮点误差可能让整拍落成 479.9999 → 479，放大一点点避免少一 tick。
    let sr = f64::from(tpq) * project.tempo / 60.0 * (1.0 + 1e-9);
    let settings = RenderSettings::default();
    let spec = ProcessSpec { sample_rate: sr, max_block_size: settings.block_size, num_channels: 2 };
    schedule(project, ti, &Ctx { tempo: project.tempo, sr, settings, total_frames: 0, spec })
}

/// 从 `start` 帧开始演奏时的动作：之前的都去掉，那时还按着的音与踏板在 `start` 补发。
fn chase(acts: Vec<(usize, Act)>, start: usize) -> Vec<(usize, Act)> {
    if start == 0 {
        return acts;
    }
    let split = acts.partition_point(|&(at, _)| at < start);
    let (mut held, mut pedal) = (Vec::new(), false);
    for &(_, act) in &acts[..split] {
        match act {
            Act::On { .. } => held.push(act),
            Act::Off { key } => {
                if let Some(i) = held.iter().position(|a| matches!(a, Act::On { key: k, .. } if *k == key)) {
                    held.remove(i);
                }
            }
            Act::Pedal { down } => pedal = down,
        }
    }
    let mut rest: Vec<(usize, Act)> = acts[split..].to_vec();
    // 恰好在 `start` 松开的音不补发。
    rest.retain(|&(at, act)| match act {
        Act::Off { key } if at == start => match held.iter().position(|a| matches!(a, Act::On { key: k, .. } if *k == key)) {
            Some(i) => {
                held.remove(i);
                false
            }
            None => true,
        },
        _ => true,
    });
    let mut out: Vec<(usize, Act)> = Vec::with_capacity(rest.len() + held.len() + 1);
    if pedal {
        out.push((start, Act::Pedal { down: true }));
    }
    out.extend(held.into_iter().map(|a| (start, a)));
    out.extend(rest);
    out
}

/// 边演奏边出声的一条音轨乐器（干声）。每一步的块长只取决于当前位置、下一个事件与自动化，
/// 和调用方要多少无关，所以分几次取与一次取完结果相同。
struct LiveInstrument {
    inst: instrument::TrackInstrument,
    channel: u8,
    acts: Vec<(usize, Act)>,
    next: usize,
    frame: usize,
    lanes: Vec<automate::Lane>,
    block: AudioBuffer<f32>,
    out: Tape,
    name: String,
    instrument: String,
}

impl LiveInstrument {
    /// 从第 `start` 帧开始演奏。`start` 之前按下、到这时还没松开的音（以及踩着的踏板）在 `start` 补发（chase）。
    fn new(project: &Project, ti: usize, cx: &Ctx, beats: f64, start: usize) -> Result<Self, EngineError> {
        let track = &project.tracks[ti];
        let mut inst = instrument::resolve_for(&track.instrument, &track.instrument_params, &cx.spec)?;
        inst.prepare(&cx.spec);
        let lanes = automate::compile_track(project, ti, beats).map_err(invalid)?.instrument;
        Ok(Self {
            channel: inst.channel(),
            inst,
            acts: chase(schedule(project, ti, cx), start),
            next: 0,
            frame: start,
            lanes,
            block: AudioBuffer::<f32>::new(2, cx.settings.block_size),
            out: Tape { start, ..Default::default() },
            name: track.name.clone(),
            instrument: track.instrument.clone(),
        })
    }

    /// 演奏到 `until` 帧（不超过工程结尾）。
    fn run_to(&mut self, until: usize, cx: &Ctx) -> Result<(), EngineError> {
        let (block_size, total_frames) = (cx.settings.block_size, cx.total_frames);
        let until = until.min(total_frames);
        while self.frame < until {
            let frame = self.frame;
            while let Some(&(at, act)) = self.acts.get(self.next) {
                if at > frame {
                    break;
                }
                match act {
                    Act::On { key, velocity, accent } => self.inst.note_on(NoteRef::new(self.channel, key), velocity, accent),
                    Act::Off { key } => self.inst.note_off(NoteRef::new(self.channel, key), 0.0),
                    Act::Pedal { down } => self.inst.pedal(down),
                }
                self.next += 1;
            }
            for lane in self.lanes.iter_mut() {
                if let Some(v) = lane.poll(cx.beat_at(frame)) {
                    self.inst.set_param(&lane.param, v);
                }
            }
            let step = if self.lanes.is_empty() { block_size } else { block_size.min(automate::BLOCK) };
            let next_at = self.acts.get(self.next).map_or(total_frames, |&(at, _)| at.min(total_frames));
            let end = (frame + step).min(next_at).max(frame + 1).min(total_frames);
            self.block.set_num_frames(end - frame);
            self.inst.process(&mut self.block);
            // 上游模型在极端参数下可能数值发散；宁可报错也不放出爆音。
            if (0..2).any(|ch| self.block.channel(ch).iter().any(|s| !s.is_finite() || s.abs() > 64.0)) {
                return Err(EngineError::Unstable(format!(
                    "音轨 '{}'（{}）输出数值发散，请调整 instrument_params（如增大 release / attack）",
                    self.name, self.instrument
                )));
            }
            let n = self.block.num_frames();
            self.out.push(&self.block, 0..n);
            self.frame = end;
        }
        Ok(())
    }
}

/// 一段连续的立体声：帧 `[start, end())`。读过的部分用 [`Tape::trim`] 丢掉，内存只占一小段。
#[derive(Default)]
struct Tape {
    start: usize,
    ch: [Vec<f32>; 2],
}

impl Tape {
    fn end(&self) -> usize {
        self.start + self.ch[0].len()
    }
    fn push(&mut self, buf: &AudioBuffer<f32>, range: std::ops::Range<usize>) {
        for (c, dst) in self.ch.iter_mut().enumerate() {
            dst.extend_from_slice(&buf.channel(c)[range.clone()]);
        }
    }
    /// 把帧 `[at, at + dst 长度)` 复制进 `dst`（这段必须都在带上）。
    fn read(&self, at: usize, dst: &mut AudioBuffer<f32>) {
        let (a, n) = (at - self.start, dst.num_frames());
        for c in 0..2 {
            dst.channel_mut(c).copy_from_slice(&self.ch[c][a..a + n]);
        }
    }
    /// 丢掉 `before` 之前的帧。
    fn trim(&mut self, before: usize) {
        let k = before.saturating_sub(self.start).min(self.ch[0].len());
        for c in &mut self.ch {
            c.drain(..k);
        }
        self.start += k;
    }
}

/// 在自己的线程里演奏的音轨乐器：领先一段先算好，路由图来取。乐器在线程里创建、一直留在那里，
/// 线程之间只传音频块。各音轨并行演奏，快进（跳转）与渐进渲染都快得多。
struct Worker {
    rx: std::sync::mpsc::Receiver<Result<AudioBuffer<f32>, EngineError>>,
    tape: Tape,
}

impl Worker {
    /// 每块的帧数，与最多领先的块数（约 1.4 秒）。
    const BLOCK: usize = 8192;
    const AHEAD: usize = 8;

    fn spawn(project: Arc<Project>, ti: usize, cx: Ctx, beats: f64, start: usize) -> Worker {
        let (tx, rx) = std::sync::mpsc::sync_channel(Self::AHEAD);
        // 线程起不来时发送端随闭包丢弃，取音频时报"演奏线程已退出"。
        let _ = std::thread::Builder::new().name(format!("muser-track-{ti}")).spawn(move || {
            let ts = project.time_signature;
            let _t = crate::transport::set(project.tempo, u16::from(ts.numerator), u16::from(ts.denominator));
            let _quiet = manguebyte::NoDenormals::new();
            let mut live = match LiveInstrument::new(&project, ti, &cx, beats, start) {
                Ok(l) => l,
                Err(e) => return drop(tx.send(Err(e))),
            };
            let mut at = start;
            while at < cx.total_frames {
                let end = (at + Self::BLOCK).min(cx.total_frames);
                if let Err(e) = live.run_to(end, &cx) {
                    return drop(tx.send(Err(e)));
                }
                let mut b = AudioBuffer::<f32>::new(2, end - at);
                live.out.read(at, &mut b);
                live.out.trim(end);
                // 渲染器已丢弃（跳转、换工程）：结束。
                if tx.send(Ok(b)).is_err() {
                    return;
                }
                at = end;
            }
        });
        Worker { rx, tape: Tape { start, ..Default::default() } }
    }
}

/// 音轨的乐器来源：缓存里整首的输出、在演奏线程里出声（内置乐器），或在当前线程边演奏边出声（插件乐器）。
enum Source {
    Cached(Arc<Cached>),
    Worker(Worker),
    Live(Box<LiveInstrument>),
}

impl Source {
    /// 把帧 `[at, at + dst 长度)` 写进 `dst`。`at` 必须依次递增、不回头。
    fn fill(&mut self, at: usize, dst: &mut AudioBuffer<f32>, cx: &Ctx) -> Result<(), EngineError> {
        match self {
            Source::Cached(c) => {
                for ch in 0..2 {
                    let d = dst.channel_mut(ch);
                    d.fill(0.0);
                    let src = c.audio.channel(ch);
                    for (i, s) in d.iter_mut().enumerate() {
                        if let Some(&v) = (at + i).checked_sub(c.offset).and_then(|j| src.get(j)) {
                            *s = v;
                        }
                    }
                }
            }
            Source::Worker(w) => {
                while w.tape.end() < at + dst.num_frames() {
                    match w.rx.recv() {
                        Ok(Ok(b)) => w.tape.push(&b, 0..b.num_frames()),
                        Ok(Err(e)) => return Err(e),
                        Err(_) => return Err(EngineError::Unstable("音轨的演奏线程已退出".into())),
                    }
                }
                w.tape.read(at, dst);
                w.tape.trim(at + dst.num_frames());
            }
            Source::Live(l) => {
                l.run_to(at + dst.num_frames(), cx)?;
                l.out.read(at, dst);
                l.out.trim(at + dst.num_frames());
            }
        }
        Ok(())
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
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

/// 推子：增益与声像（有自动化时逐帧）。`buf` 的第 0 帧是工程的第 `at` 帧。
fn fader(buf: &mut AudioBuffer<f32>, at: usize, cx: &Ctx, gain_db: f64, pan: f64, lanes: &TrackLanes, law: fn(f64, f64) -> (f32, f32)) {
    let fixed = lanes.gain.is_empty() && lanes.pan.is_empty();
    let (mut gl, mut gr) = law(gain_db, pan);
    for frame in 0..buf.num_frames() {
        if !fixed {
            let beat = cx.beat_at(at + frame);
            (gl, gr) = law(automate::scalar(&lanes.gain, beat, gain_db), automate::scalar(&lanes.pan, beat, pan));
        }
        buf.channel_mut(0)[frame] *= gl;
        buf.channel_mut(1)[frame] *= gr;
    }
}

/// 把 `src` 按 `level_db`（可被 lanes 自动化）加进 `dst`。两者的第 0 帧是工程的第 `at` 帧。
fn mix_into(dst: &mut AudioBuffer<f32>, src: &AudioBuffer<f32>, at: usize, cx: &Ctx, level_db: f64, lanes: Option<&Vec<automate::Lane>>) {
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
                let g = decibels::db_to_gain(automate::scalar(l, cx.beat_at(at + frame), level_db)) as f32;
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
    // 音频文件可能被替换：计入修改时间。
    let audio_mtimes: Vec<String> = t
        .clips
        .iter()
        .filter_map(|c| c.audio.as_ref())
        .map(|a| format!("{:?}", std::fs::metadata(crate::audio::resolve(project, &a.file)).and_then(|m| m.modified()).ok()))
        .collect();
    let ts = project.time_signature;
    let key = serde_json::json!({ "am": audio_mtimes,
        "i": t.instrument, "p": t.instrument_params, "c": t.clips, "h": t.humanize, "a": autos.iter().map(|(i, a)| serde_json::json!([i, a])).collect::<Vec<_>>(),
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

/// 同 [`render`]，复用 `cache` 里的乐器输出，并把本次的乐器输出存回。
pub fn render_cached(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<AudioBuffer<f32>, EngineError> {
    render_inner(project, settings, cache, None, None)
}

/// 一个测量点的响度（静音或没有信号时为 `None`）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Level {
    /// 积分响度（LUFS，EBU R128，门限去掉静音段）。
    pub lufs: Option<f64>,
    /// 短时（3 秒窗）响度的最大值（LUFS）：最响的段落有多响。
    pub max_short_lufs: Option<f64>,
    /// 峰值（dBFS）。
    pub peak_db: Option<f64>,
}

/// 一个节点（音轨/总线；`node` 为 `None` 是主总线）整首的响度。
#[derive(Debug, Clone)]
pub struct NodeLoudness {
    pub node: Option<Node>,
    pub muted: bool,
    /// 效果链之前：音轨是乐器干声，总线/主总线是送进来的信号之和。
    pub input: Level,
    /// 推子后（效果链、增益、声像之后），也就是送进混音的信号。静音节点为空。
    pub output: Level,
}

/// 节点的响度表：效果链前一个、推子后一个。
struct LoudMeters {
    input: LoudnessMeter,
    output: LoudnessMeter,
    in_peak: f32,
    out_peak: f32,
}

impl LoudMeters {
    fn new(spec: &ProcessSpec) -> Box<Self> {
        let mut m = Box::new(LoudMeters { input: LoudnessMeter::stereo(), output: LoudnessMeter::stereo(), in_peak: 0.0, out_peak: 0.0 });
        m.input.prepare(spec);
        m.output.prepare(spec);
        m
    }
}

fn level(meter: &LoudnessMeter, peak: f32) -> Level {
    let finite = |x: f64| (x.is_finite()).then(|| (x * 10.0).round() / 10.0);
    let r = meter.readings();
    Level {
        lufs: finite(r.integrated()),
        max_short_lufs: finite(r.max_short_term()),
        peak_db: (peak > 0.0).then(|| finite(decibels::gain_to_db(f64::from(peak)))).flatten(),
    }
}

/// 渲染整首，给出每个节点的响度（按计划顺序，主总线在最后）。复用并更新 `cache` 里的乐器输出。
pub fn render_loudness(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<Vec<NodeLoudness>, EngineError> {
    crate::validate(project)?;
    let plan: Plan = routing::plan(project).map_err(invalid)?;
    let mut r = Renderer::build(project, settings, plan, Some(cache), false, false, 0)?.recording().with_loudness();
    let mut block = AudioBuffer::<f32>::new(2, 1 << 16);
    while r.process(&mut block)? > 0 {}
    r.store_instruments(cache);
    Ok(r.loudness())
}

/// 同 [`render_cached`]，同时给出每个节点的电平（供播放界面的电平表）。
pub fn render_metered(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<(AudioBuffer<f32>, Vec<Meter>), EngineError> {
    let mut meters = Vec::new();
    let audio = render_inner(project, settings, cache, Some(&mut meters), None)?;
    Ok((audio, meters))
}

/// 侧链键节点：音轨/总线压缩器引用的，以及主总线压缩器引用的（后者按 `master_effects` 下标给出）。
fn sidechain_keys(project: &Project, plan: &Plan) -> Result<(HashSet<Node>, Vec<Option<Node>>), EngineError> {
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
    Ok((key_nodes, master_keys))
}

impl Ctx {
    fn new(project: &Project, settings: RenderSettings) -> Ctx {
        let sr = f64::from(settings.sample_rate);
        let duration = project.duration_beats().to_seconds(project.tempo);
        Ctx {
            tempo: project.tempo,
            sr,
            settings,
            total_frames: ((duration + settings.tail_seconds) * sr).ceil() as usize + 1,
            spec: ProcessSpec { sample_rate: sr, max_block_size: settings.block_size, num_channels: 2 },
        }
    }
}

fn render_inner(
    project: &Project,
    settings: RenderSettings,
    cache: &mut RenderCache,
    meters: Option<&mut Vec<Meter>>,
    mut progress: Option<&mut dyn FnMut(usize, usize)>,
) -> Result<AudioBuffer<f32>, EngineError> {
    crate::validate(project)?;
    let plan: Plan = routing::plan(project).map_err(invalid)?;
    // 从头流式推进到结尾，直接写进整首的缓冲；缓存里没有的乐器边演奏边录下，渲染完存回缓存。
    let mut r = Renderer::build(project, settings, plan, Some(cache), false, meters.is_some(), 0)?.recording();
    let total_frames = r.total_frames();
    let mut master = AudioBuffer::<f32>::new(2, total_frames);
    let mut at = 0;
    while at < total_frames {
        let end = (at + (1 << 16)).min(total_frames);
        let n = master.windowed(at..end, |b| r.process(b))?;
        if n == 0 {
            break;
        }
        at += n;
        if let Some(p) = progress.as_deref_mut() {
            p(at, total_frames);
        }
    }
    r.store_instruments(cache);
    if let Some(m) = meters {
        *m = r.meters();
    }
    Ok(master)
}

/// 路由图上一个节点（音轨、总线或主总线）在流式渲染中的状态。
struct Strip {
    /// `None` 是主总线。
    node: Option<Node>,
    /// 要处理：未静音、作为侧链键，或要求静音节点也照常处理。
    active: bool,
    muted: bool,
    /// 音轨的乐器。
    source: Option<Source>,
    chain: Vec<Fx>,
    lanes: TrackLanes,
    latency: usize,
    gain_db: f64,
    pan: f64,
    law: fn(f64, f64) -> (f32, f32),
    /// 每个效果器的侧链键（`strips` 下标）。
    keys: Vec<Option<usize>>,
    /// 送进本节点的边：(`plan.edges` 下标, 来源的 `strips` 下标)，按求和顺序。
    inputs: Vec<(usize, usize)>,
    /// 读本节点输出的节点（`strips` 下标）。
    readers: Vec<usize>,
    /// 效果链已处理到的帧（输入侧）。
    pos: usize,
    /// 效果链输出（延迟补偿前）。
    wet: Tape,
    /// 推子后输出。
    out: Tape,
    meter: Vec<[f32; 2]>,
    /// 计划里的层与顺序（电平表按它排序）。
    rank: (usize, usize),
    /// 音轨乐器输出的缓存键。
    cache_key: Option<u64>,
    /// 录下的整首乐器输出（[`Renderer::recording`]）。
    record: Option<[Vec<f32>; 2]>,
    /// 响度表（[`Renderer::with_loudness`]）。
    loud: Option<Box<LoudMeters>>,
}

/// 主总线的"推子"：只有增益。
fn master_gains(gain_db: f64, _pan: f64) -> (f32, f32) {
    let g = decibels::db_to_gain(gain_db) as f32;
    (g, g)
}

/// 流式渲染器：每次调用 [`Renderer::process`] 往前推进一段，逐块走完整张路由图。
///
/// 与离线渲染结果逐样本一致（离线渲染就是从头到尾调用它）。可以边渲染边播放：
/// 乐器边演奏边出声（或取自 [`RenderCache`]），内存只占正在处理的一小段。
pub struct Renderer {
    cx: Ctx,
    plan: Plan,
    strips: Vec<Strip>,
    /// 流式推进的段对齐（帧）。
    align: usize,
    pos: usize,
    tempo: f64,
    ts: (u16, u16),
    beats: f64,
    meters: bool,
}

impl Renderer {
    /// 新建。`cache` 里有的音轨乐器输出直接用，其余边演奏边出声。
    /// `all_nodes`：静音的节点也照常处理（只是不送出），播放中切换静音时声音连续。
    pub fn new(project: &Project, settings: RenderSettings, cache: Option<&RenderCache>, all_nodes: bool) -> Result<Self, EngineError> {
        crate::validate(project)?;
        let plan = routing::plan(project).map_err(invalid)?;
        Self::build(project, settings, plan, cache, all_nodes, false, 0)
    }

    /// 同 [`Renderer::new`]，但从第 `frame` 帧（向下对齐到处理段）开始，用于播放中跳转：不从头算，立即可播。
    /// 那时还按着的音与踏板会补发；缓存里有的音轨乐器输出与从头播完全相同，
    /// 但效果器从空状态开始（跳转点之前的混响、延迟尾巴听不到），和 DAW 的跳转一样。
    pub fn new_at(project: &Project, settings: RenderSettings, cache: Option<&RenderCache>, all_nodes: bool, frame: usize) -> Result<Self, EngineError> {
        crate::validate(project)?;
        let plan = routing::plan(project).map_err(invalid)?;
        Self::build(project, settings, plan, cache, all_nodes, false, frame)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        project: &Project,
        settings: RenderSettings,
        plan: Plan,
        cache: Option<&RenderCache>,
        all_nodes: bool,
        meters: bool,
        start: usize,
    ) -> Result<Self, EngineError> {
        // 编曲层（form / parts）展开成普通 clip，之后只看展开后的工程。
        let arranged = crate::arrange::arranged(project).map_err(invalid)?;
        let project = &*arranged;
        let ts = project.time_signature;
        let _transport = crate::transport::set(project.tempo, u16::from(ts.numerator), u16::from(ts.denominator));
        let cx = Ctx::new(project, settings);
        let bs = settings.block_size;
        let align = bs / gcd(bs, automate::BLOCK) * automate::BLOCK;
        let start = (start.min(cx.total_frames) / align) * align;
        let beats = automate::total_beats(project, settings.tail_seconds);
        let (key_nodes, master_keys) = sidechain_keys(project, &plan)?;

        // 层：同一层的节点互不依赖。求和按（层, 计划顺序, 边顺序），与逐层处理的结果一致。
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
        let index: HashMap<Node, usize> = plan.order.iter().enumerate().map(|(i, n)| (*n, i)).collect();
        let rank = |n: Node| (level[&n], index[&n]);

        let mut strips: Vec<Strip> = Vec::with_capacity(plan.order.len() + 1);
        // 演奏线程共用的一份工程。
        let mut shared: Option<Arc<Project>> = None;
        for &node in &plan.order {
            let (muted, effects_list, gain_db, pan, lanes, law): (bool, &[Effect], f64, f64, TrackLanes, fn(f64, f64) -> (f32, f32)) = match node {
                Node::Track(i) => {
                    let t = &project.tracks[i];
                    (t.muted, &t.effects, t.gain_db, t.pan, automate::compile_track(project, i, beats).map_err(invalid)?, pan_gains)
                }
                Node::Bus(i) => {
                    let b = &project.buses[i];
                    (b.muted, &b.effects, b.gain_db, b.pan, automate::compile_bus(project, i, beats).map_err(invalid)?, balance_gains)
                }
            };
            let active = all_nodes || !muted || key_nodes.contains(&node);
            let cache_key = match node {
                Node::Track(i) => Some(instrument_key(project, i, &cx)),
                Node::Bus(_) => None,
            };
            let source = match (node, active) {
                (Node::Track(i), true) => {
                    let cached = cache.and_then(|c| c.map.get(&cache_key.unwrap()));
                    Some(match cached {
                        Some(c) => Source::Cached(c.clone()),
                        // 音频音轨：整轨干声一次铺好（同一工程再次渲染时取自缓存，不再解码）。
                        None if project.tracks[i].instrument.trim() == muser_core::audio::INSTRUMENT => {
                            let audio = crate::audio::track_buffer(project, i, settings.sample_rate, cx.total_frames).map_err(invalid)?;
                            Source::Cached(Arc::new(Cached::new(audio)))
                        }
                        None if project.tracks[i].instrument.trim().starts_with("plugin:") => {
                            Source::Live(Box::new(LiveInstrument::new(project, i, &cx, beats, start)?))
                        }
                        None => Source::Worker(Worker::spawn(Arc::clone(shared.get_or_insert_with(|| Arc::new(project.clone()))), i, cx.clone(), beats, start)),
                    })
                }
                _ => None,
            };
            let chain = if active { effects::build_chain(effects_list, &cx.spec)? } else { Vec::new() };
            let keys = (0..effects_list.len())
                .map(|ei| plan.sidechains.iter().find(|s| s.node == node && s.effect == ei).map(|s| index[&s.key]))
                .collect();
            strips.push(Strip {
                node: Some(node),
                active,
                muted,
                source,
                latency: effects::chain_latency(&chain),
                chain,
                lanes,
                gain_db,
                pan,
                law,
                keys,
                inputs: Vec::new(),
                readers: Vec::new(),
                pos: start,
                wet: Tape { start, ..Default::default() },
                out: Tape { start, ..Default::default() },
                meter: Vec::new(),
                rank: rank(node),
                cache_key,
                record: None,
                loud: None,
            });
        }
        let MasterLanes { effects: fx_lanes, gain } = automate::compile_master(project, beats).map_err(invalid)?;
        let chain = effects::build_chain(&project.master_effects, &cx.spec)?;
        strips.push(Strip {
            node: None,
            active: true,
            muted: false,
            source: None,
            latency: effects::chain_latency(&chain),
            chain,
            lanes: TrackLanes { instrument: Vec::new(), effects: fx_lanes, gain, pan: Vec::new(), sends: Default::default() },
            gain_db: project.master_gain_db,
            pan: 0.0,
            law: master_gains,
            keys: master_keys.iter().map(|k| k.map(|n| index[&n])).collect(),
            inputs: Vec::new(),
            readers: Vec::new(),
            pos: start,
            wet: Tape { start, ..Default::default() },
            out: Tape { start, ..Default::default() },
            meter: Vec::new(),
            rank: (usize::MAX, usize::MAX),
            cache_key: None,
            record: None,
            loud: None,
        });

        let master = strips.len() - 1;
        let mut edges: Vec<usize> = (0..plan.edges.len()).collect();
        edges.sort_by_key(|&e| (rank(plan.edges[e].from), e));
        for e in edges {
            let edge = &plan.edges[e];
            let to = edge.to.map_or(master, |b| index[&Node::Bus(b)]);
            strips[to].inputs.push((e, index[&edge.from]));
            strips[index[&edge.from]].readers.push(to);
        }
        for s in 0..strips.len() {
            for k in strips[s].keys.clone().into_iter().flatten() {
                strips[k].readers.push(s);
            }
        }

        Ok(Renderer {
            align,
            cx,
            plan,
            strips,
            pos: start,
            tempo: project.tempo,
            ts: (u16::from(ts.numerator), u16::from(ts.denominator)),
            beats,
            meters,
        })
    }

    /// 总帧数（含结尾余响）。
    pub fn total_frames(&self) -> usize {
        self.cx.total_frames
    }

    /// 已经输出到的帧。
    pub fn position(&self) -> usize {
        self.pos
    }

    /// 渲染接下来的 `out.num_frames()` 帧（不超过结尾），返回实际写入的帧数；到结尾后返回 0。
    pub fn process(&mut self, out: &mut AudioBuffer<f32>) -> Result<usize, EngineError> {
        let _transport = crate::transport::set(self.tempo, self.ts.0, self.ts.1);
        let _quiet = manguebyte::NoDenormals::new();
        let t = (self.pos + out.num_frames()).min(self.cx.total_frames);
        let n = t - self.pos;
        if n == 0 {
            return Ok(0);
        }
        self.run_to(t)?;
        let master = self.strips.len() - 1;
        out.windowed(0..n, |b| self.strips[master].out.read(self.pos, b));
        self.strips[master].out.trim(t);
        self.pos = t;
        Ok(n)
    }

    /// 让主总线输出覆盖到 `t`：从主总线往上游算出每个节点的效果链要处理到哪，再按计划顺序处理。
    fn run_to(&mut self, t: usize) -> Result<(), EngineError> {
        let (total, g) = (self.cx.total_frames, self.align);
        // 输出要覆盖到 `need` 时，效果链（输入侧）要处理到哪：多算 latency 帧，并对齐到段边界。
        let chain_target = |need: usize, latency: usize| -> usize {
            let n = need + latency;
            if n >= total { total } else { (n.div_ceil(g) * g).min(total) }
        };
        let count = self.strips.len();
        let mut need = vec![t; count];
        let mut target = vec![0; count];
        for s in (0..count).rev() {
            let st = &self.strips[s];
            target[s] = chain_target(need[s], st.latency);
            for &(_, from) in &st.inputs {
                need[from] = need[from].max(target[s]);
            }
            for k in st.keys.iter().flatten() {
                need[*k] = need[*k].max(target[s]);
            }
        }
        for (s, &to) in target.iter().enumerate() {
            if self.strips[s].active {
                self.advance(s, to)?;
            }
        }
        // 每个节点的输出只留还没被读的部分。
        for s in 0..count {
            let st = &self.strips[s];
            if st.node.is_none() {
                continue;
            }
            let keep = st.readers.iter().filter(|&&r| self.strips[r].active).map(|&r| self.strips[r].pos).min().unwrap_or(st.out.end());
            self.strips[s].out.trim(keep);
        }
        Ok(())
    }

    /// 把第 `s` 个节点的效果链处理到 `target` 帧（输入侧），并产出相应的推子后输出。
    fn advance(&mut self, s: usize, target: usize) -> Result<(), EngineError> {
        let (total, g, bs) = (self.cx.total_frames, self.align, self.cx.settings.block_size);
        while self.strips[s].pos < target {
            let a = self.strips[s].pos;
            let b = ((a / g + 1) * g).min(total);
            let n = b - a;
            let mut buf = AudioBuffer::<f32>::new(2, n);
            let mut tmp = AudioBuffer::<f32>::new(2, n);
            // 输入：音轨的乐器，或送进来的信号之和。
            for &(e, from) in &self.strips[s].inputs {
                let (edge, from) = (&self.plan.edges[e], &self.strips[from]);
                if from.muted || !from.active {
                    continue;
                }
                from.out.read(a, &mut tmp);
                let send_lanes = edge.send.as_ref().and_then(|name| from.lanes.sends.get(name));
                mix_into(&mut buf, &tmp, a, &self.cx, edge.level_db, send_lanes);
            }
            let keys: Vec<Option<AudioBuffer<f32>>> = self.strips[s]
                .keys
                .iter()
                .map(|k| {
                    k.map(|k| {
                        let mut kb = AudioBuffer::<f32>::new(2, n);
                        self.strips[k].out.read(a, &mut kb);
                        kb
                    })
                })
                .collect();
            let key_refs: Vec<Option<&AudioBuffer<f32>>> = keys.iter().map(|k| k.as_ref()).collect();
            let cx = &self.cx;
            let st = &mut self.strips[s];
            if let Some(src) = st.source.as_mut() {
                src.fill(a, &mut buf, cx)?;
                if let Some(rec) = st.record.as_mut() {
                    for (c, r) in rec.iter_mut().enumerate() {
                        r.extend_from_slice(buf.channel(c));
                    }
                }
            }
            if let Some(l) = st.loud.as_mut() {
                l.input.measure(&buf);
                l.in_peak = l.in_peak.max(buf.peak());
            }
            effects::apply_chain(&mut st.chain, &mut buf, bs, &mut st.lanes.effects, &|f| cx.beat_at(a + f), &key_refs);
            st.wet.push(&buf, 0..n);
            st.pos = b;

            // 输出：效果链输出往后 latency 帧的样本；结尾之后补 0。
            let ready = if b == total { total } else { b.saturating_sub(st.latency) };
            let from = st.out.end();
            if ready > from {
                let mut o = AudioBuffer::<f32>::new(2, ready - from);
                let lat = st.latency;
                for c in 0..2 {
                    let w = &st.wet.ch[c];
                    for (i, x) in o.channel_mut(c).iter_mut().enumerate() {
                        let f = from + i + lat;
                        *x = if f < total { w[f - st.wet.start] } else { 0.0 };
                    }
                }
                fader(&mut o, from, cx, st.gain_db, st.pan, &st.lanes, st.law);
                if self.meters && !st.muted {
                    for (i, f) in (from..ready).enumerate() {
                        let w = f / METER_WINDOW;
                        if st.meter.len() <= w {
                            st.meter.resize(w + 1, [0.0; 2]);
                        }
                        for c in 0..2 {
                            st.meter[w][c] = st.meter[w][c].max(o.channel(c)[i].abs());
                        }
                    }
                }
                if let (Some(l), false) = (st.loud.as_mut(), st.muted) {
                    l.output.measure(&o);
                    l.out_peak = l.out_peak.max(o.peak());
                }
                st.out.push(&o, 0..ready - from);
                st.wet.trim(st.out.end() + lat);
            }
        }
        Ok(())
    }

    /// 播放中改推子与静音：音轨/总线的 `gain_db`、`muted` 与音量自动化，以及主增益，从下一段起生效。
    /// `project` 的结构（音轨、总线、路由）须与创建时相同。创建时没有处理的节点
    /// （静音、且没有要求 `all_nodes`）取消静音也不会出声。
    pub fn set_mix(&mut self, project: &Project) -> Result<(), EngineError> {
        for s in &mut self.strips {
            match s.node {
                Some(Node::Track(i)) => {
                    let t = project.tracks.get(i).ok_or_else(|| invalid("工程结构变了".into()))?;
                    (s.gain_db, s.muted) = (t.gain_db, t.muted);
                    s.lanes.gain = automate::compile_track(project, i, self.beats).map_err(invalid)?.gain;
                }
                Some(Node::Bus(i)) => {
                    let b = project.buses.get(i).ok_or_else(|| invalid("工程结构变了".into()))?;
                    (s.gain_db, s.muted) = (b.gain_db, b.muted);
                    s.lanes.gain = automate::compile_bus(project, i, self.beats).map_err(invalid)?.gain;
                }
                None => {
                    s.gain_db = project.master_gain_db;
                    s.lanes.gain = automate::compile_master(project, self.beats).map_err(invalid)?.gain;
                }
            }
        }
        Ok(())
    }

    /// 同时记录各节点的电平（见 [`Renderer::meters`]）。
    pub fn with_meters(mut self) -> Self {
        self.meters = true;
        self
    }

    /// 同时测量各节点整首的响度（见 [`Renderer::loudness`]）。只对从头开始的渲染有意义。
    pub fn with_loudness(mut self) -> Self {
        let spec = self.cx.spec;
        for s in self.strips.iter_mut().filter(|s| s.active) {
            s.loud = Some(LoudMeters::new(&spec));
        }
        self
    }

    /// 到目前为止各节点的响度（需 [`Renderer::with_loudness`]），按计划顺序，主总线在最后。
    pub fn loudness(&self) -> Vec<NodeLoudness> {
        let mut v: Vec<&Strip> = self.strips.iter().collect();
        v.sort_by_key(|s| s.rank);
        let none = Level { lufs: None, max_short_lufs: None, peak_db: None };
        v.into_iter()
            .map(|s| {
                let (input, output) = s.loud.as_ref().map_or((none, none), |l| (level(&l.input, l.in_peak), level(&l.output, l.out_peak)));
                NodeLoudness { node: s.node, muted: s.muted, input, output: if s.muted { none } else { output } }
            })
            .collect()
    }

    /// 同时录下边演奏边出声的乐器输出，渲染到结尾后用 [`Renderer::store_instruments`] 存进缓存。
    pub fn recording(mut self) -> Self {
        // 不是从头开始的录不全，不存。
        if self.pos > 0 {
            return self;
        }
        for s in &mut self.strips {
            if matches!(s.source, Some(Source::Worker(_) | Source::Live(_))) {
                s.record = Some(Default::default());
            }
        }
        self
    }

    /// 到目前为止各节点（音轨/总线，不含静音的）的电平，按计划顺序。
    pub fn meters(&self) -> Vec<Meter> {
        let mut v: Vec<&Strip> = self.strips.iter().filter(|s| s.node.is_some() && s.active && !s.muted).collect();
        v.sort_by_key(|s| s.rank);
        v.into_iter().map(|s| Meter { node: s.node.unwrap(), peaks: s.meter.clone() }).collect()
    }

    /// 渲染到结尾后：把录下的乐器输出存进 `cache`，并只保留这个工程用到的条目（同 [`render_cached`]）。
    pub fn store_instruments(&mut self, cache: &mut RenderCache) {
        if self.pos < self.cx.total_frames {
            return;
        }
        for s in &mut self.strips {
            if let (Some(key), Some(Source::Cached(c))) = (s.cache_key, &s.source) {
                cache.map.insert(key, c.clone());
            }
            if let (Some(key), Some(rec)) = (s.cache_key, s.record.take()) {
                let mut full = AudioBuffer::<f32>::new(2, rec[0].len());
                for (c, r) in rec.iter().enumerate() {
                    full.channel_mut(c).copy_from_slice(r);
                }
                cache.map.insert(key, Arc::new(Cached::new(full)));
            }
        }
        let keys: Vec<u64> = self.strips.iter().filter_map(|s| s.cache_key).collect();
        cache.map.retain(|k, _| keys.contains(k));
    }
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
    render_to_wav_with_progress(project, path, settings, &mut |_, _| {})
}

/// 同 [`render_to_wav`]，渲染中每推进一段调用 `progress(已渲染帧数, 总帧数)`。
pub fn render_to_wav_with_progress(
    project: &Project,
    path: &str,
    settings: RenderSettings,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(f64, f64), EngineError> {
    let audio = render_inner(project, settings, &mut RenderCache::new(), None, Some(progress))?;
    wav::write_file(path, &audio, settings.sample_rate, WavFormat::Int16)
        .map_err(|e| EngineError::Wav(format!("{e:?}")))?;
    let seconds = audio.num_frames() as f64 / f64::from(settings.sample_rate);
    let peak_db = decibels::gain_to_db(f64::from(audio.peak()));
    Ok((seconds, peak_db))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 约 1 秒的小工程，覆盖：总线、发送与发送自动化、侧链（音轨与主总线）、有延迟的 limiter（总线与主总线）、
    /// 乐器/增益/声像/效果器自动化、静音音轨。
    fn project() -> Project {
        let hits: Vec<_> = (0..4).map(|i| serde_json::json!({ "pitch": 36, "start": i as f64 * 0.5, "length": 0.2 })).collect();
        let bass: Vec<_> = (0..8).map(|i| serde_json::json!({ "pitch": 36 + (i % 3) * 5, "start": i as f64 * 0.25, "length": 0.2 })).collect();
        serde_json::from_value(serde_json::json!({
            "tempo": 120,
            "buses": [
                { "name": "verb", "effects": [{ "type": "reverb", "mix": 1.0 }, { "type": "compressor", "threshold_db": -30, "ratio": 6, "sidechain": "kick" }] },
                { "name": "grp", "effects": [{ "type": "limiter", "ceiling_db": -6 }], "sends": { "verb": -10 } }
            ],
            "tracks": [
                { "name": "kick", "instrument": "drums.909", "clips": [{ "start": 0, "length": 2, "notes": hits }],
                  "automation": [{ "target": "pan", "shape": "lfo", "period_beats": 1, "min": -0.5, "max": 0.5 }] },
                { "name": "bass", "instrument": "bass.acid", "output": "grp", "sends": { "verb": -6 },
                  "clips": [{ "start": 0, "length": 2, "notes": bass }],
                  "automation": [
                      { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [2, 3000]] },
                      { "target": "sends.verb", "shape": "points", "points": [[0, -30], [2, 0]] },
                      { "target": "effects[0]", "param": "mix", "shape": "points", "points": [[0, 0.2], [2, 0.8]] }
                  ],
                  "effects": [{ "type": "delay", "time_ms": 120, "feedback": 0.4, "mix": 0.3 }] },
                { "name": "pad", "instrument": "synth.poly", "output": "grp", "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 60, "start": 0, "length": 2 }] }],
                  "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, -20], [2, 0]] }] }
            ],
            "master_effects": [{ "type": "compressor", "threshold_db": -12, "sidechain": "kick" }, { "type": "limiter", "ceiling_db": -1 }]
        }))
        .unwrap()
    }

    fn stream(p: &Project, settings: RenderSettings, all_nodes: bool, chunk: usize) -> [Vec<f32>; 2] {
        let mut r = Renderer::new(p, settings, None, all_nodes).unwrap();
        let mut block = AudioBuffer::<f32>::new(2, chunk);
        let mut got: [Vec<f32>; 2] = Default::default();
        while let n @ 1.. = r.process(&mut block).unwrap() {
            for (c, g) in got.iter_mut().enumerate() {
                g.extend_from_slice(&block.channel(c)[..n]);
            }
        }
        got
    }

    /// 播放中静音、改推子立即生效。
    #[test]
    fn live_mix() {
        let settings = RenderSettings { tail_seconds: 0.3, ..Default::default() };
        let p = project();
        let mut r = Renderer::new(&p, settings, None, true).unwrap();
        let mut b = AudioBuffer::<f32>::new(2, 20_000);
        r.process(&mut b).unwrap();
        // 音轨、总线全部静音：从下一段起无声（limiter 前瞻等延迟里的余量也在 0.1 秒内放完）。
        let mut q = p.clone();
        q.tracks.iter_mut().for_each(|t| t.muted = true);
        q.buses.iter_mut().for_each(|b| b.muted = true);
        r.set_mix(&q).unwrap();
        let mut b = AudioBuffer::<f32>::new(2, 4800);
        r.process(&mut b).unwrap();
        r.process(&mut b).unwrap();
        assert!(b.peak() < 1e-3, "静音后仍有声音: {}", b.peak());
        // 取消静音、推子拉低 12 dB：恢复出声。
        q.tracks.iter_mut().for_each(|t| { t.muted = false; t.gain_db -= 12.0; });
        q.buses.iter_mut().for_each(|b| b.muted = false);
        r.set_mix(&q).unwrap();
        r.process(&mut b).unwrap();
        assert!(b.peak() > 1e-3);
    }

    #[test]
    fn chase_resends_held_notes_and_pedal() {
        let on = |key| Act::On { key, velocity: 0.8, accent: false };
        let acts = vec![
            (0, Act::Pedal { down: true }),
            (0, on(60)),
            (10, on(64)),
            (20, Act::Off { key: 60 }),
            (50, Act::Off { key: 64 }),
            (50, on(67)),
            (90, Act::Off { key: 67 }),
        ];
        let keys = |v: &[(usize, Act)]| v.iter().map(|(at, a)| format!("{at}:{a:?}")).collect::<Vec<_>>().join(" ");
        // 30：60 已松开，64 还按着，踏板踩着。
        let c = chase(acts.clone(), 30);
        assert_eq!(keys(&c[..2]), keys(&[(30, Act::Pedal { down: true }), (30, on(64))]));
        assert_eq!(c.len(), 2 + 3);
        // 恰好在 50 松开的 64 不补发；50 新按的 67 照常。
        let c = chase(acts.clone(), 50);
        assert_eq!(keys(&c), keys(&[(50, Act::Pedal { down: true }), (50, on(67)), (90, Act::Off { key: 67 })]));
        assert_eq!(chase(acts.clone(), 0).len(), acts.len());
    }

    /// 从中间开始：缓存里的乐器输出与从头播相同（无效果器时整段逐样本一致）；没有缓存时按着的音立即出声。
    #[test]
    fn start_mid_song() {
        let settings = RenderSettings { tail_seconds: 0.3, ..Default::default() };
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120, "tracks": [
            { "name": "pad", "instrument": "synth.poly", "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 60, "start": 0, "length": 4 }] }] }] })).unwrap();
        let mut cache = RenderCache::new();
        let offline = render_cached(&p, settings, &mut cache).unwrap();
        let mut r = Renderer::new_at(&p, settings, Some(&cache), false, 30_000).unwrap();
        let s = r.position();
        assert!(s <= 30_000 && 30_000 - s < 256, "向下对齐到处理段: {s}");
        let mut b = AudioBuffer::<f32>::new(2, r.total_frames() - s);
        assert_eq!(r.process(&mut b).unwrap(), r.total_frames() - s);
        assert!((0..2).all(|c| b.channel(c) == &offline.channel(c)[s..]), "缓存的乐器：与从头播相同");
        // 没有缓存：补发按着的音，开头 50 ms 就有声音。
        let mut r = Renderer::new_at(&p, settings, None, false, 30_000).unwrap();
        let mut b = AudioBuffer::<f32>::new(2, 2400);
        r.process(&mut b).unwrap();
        assert!(b.peak() > 0.01, "按着的音应补发: {}", b.peak());
    }

    /// humanize：改变演奏；同一种子结果相同，换工程 seed（没写 humanize.seed 时）就换一遍。
    #[test]
    fn humanize_takes() {
        let settings = RenderSettings { tail_seconds: 0.2, ..Default::default() };
        let base = || {
            let hits: Vec<_> = (0..8).map(|i| serde_json::json!({ "pitch": 42, "start": i as f64 * 0.5, "length": 0.1 })).collect();
            serde_json::from_value::<Project>(serde_json::json!({ "tempo": 120,
                "tracks": [{ "instrument": "drums.909", "clips": [{ "start": 0, "length": 4, "notes": hits }] }] })).unwrap()
        };
        let render_with = |h: Option<muser_core::humanize::Humanize>, seed: u64| {
            let mut p = base();
            p.seed = seed;
            p.tracks[0].humanize = h;
            render(&p, settings).unwrap().channel(0).to_vec()
        };
        let h = muser_core::humanize::Humanize { timing_ms: Some(12.0), velocity: Some(0.1), swing: Some(60.0), ..Default::default() };
        let plain = render_with(None, 0);
        let take1 = render_with(Some(h.clone()), 0);
        assert_ne!(plain, take1);
        assert_eq!(take1, render_with(Some(h.clone()), 0), "同一种子同一遍");
        assert_ne!(take1, render_with(Some(h.clone()), 1), "换工程 seed 换一遍");
        let fixed = muser_core::humanize::Humanize { seed: Some(42), ..h };
        assert_eq!(render_with(Some(fixed.clone()), 0), render_with(Some(fixed), 1), "写了 humanize.seed 就不随工程 seed 变");
    }

    /// 乐器边演奏边出声、按任意块长流式推进，结果必须与离线渲染逐样本相同。
    #[test]
    fn streaming_matches_offline() {
        let settings = RenderSettings { tail_seconds: 0.3, ..Default::default() };
        let mut p = project();
        crate::validate(&p).unwrap();
        for muted in [false, true] {
            p.tracks[2].muted = muted;
            let offline = render(&p, settings).unwrap();
            assert!(offline.peak() > 0.01);
            for (all_nodes, chunk) in [(false, 37), (true, 1000), (false, 1 << 20)] {
                let got = stream(&p, settings, all_nodes, chunk);
                for (c, g) in got.iter().enumerate() {
                    assert!(g == offline.channel(c), "块长 {chunk}、all_nodes={all_nodes}、静音={muted}：与离线不同");
                }
            }
            // 边演奏边录下乐器输出：存进缓存后，用缓存渲染结果不变。
            let mut cache = RenderCache::new();
            let mut r = Renderer::new(&p, settings, None, false).unwrap().recording();
            let mut all = AudioBuffer::<f32>::new(2, r.total_frames());
            r.process(&mut all).unwrap();
            r.store_instruments(&mut cache);
            assert_eq!(cache.len(), if muted { 2 } else { 3 });
            let cached = render_cached(&p, settings, &mut cache).unwrap();
            assert!((0..2).all(|c| cached.channel(c) == offline.channel(c)), "录下的乐器缓存");
        }
    }
}
