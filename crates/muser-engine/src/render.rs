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
use muser_core::{Beats, Project};

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
    #[error("render failed: {0}")]
    Unstable(String),
    #[error("failed to write WAV: {0}")]
    Wav(String),
}

/// 时间线上一个样本精确的演奏动作。
#[derive(Debug, Clone, Copy)]
pub(crate) enum Act {
    /// `ch`：带弯音的音符各占一个通道（弯音轮是整个通道的），其余为 None（乐器的通道）。
    On { key: u8, velocity: f32, accent: bool, mute: bool, ch: Option<u8> },
    /// 唱一个音节（`choir.*` 的音符写了 `lyric`）：在辅音开始的帧发，`lead` 帧后元音落在拍上，音符长 `len` 帧。
    Sing { syl: crate::lyric::Syl, text: crate::lyric::Text, lead: u32, len: u32, velocity: f32 },
    Off { key: u8, ch: Option<u8> },
    Pedal { down: bool },
    /// 弯音轮，-8192 – 8191（通常 ±2 半音，古琴走手 ±24）。
    Bend { value: i16, ch: Option<u8> },
    /// 下一个音唱的字（`yinse.voice.*`）：在起音的同一帧、起音之前发，`len` 是这个音有多长（帧）。
    Lyric { text: crate::lyric::Text, len: u32, lead: u32 },
    /// 滑音（`yinse.voice.*`）：从这一帧起，用 `frames` 帧滑到 `key` 的音高（滑进刚起的这个音）。
    Slide { key: u8, frames: u32 },
    /// 换气（`yinse.voice.*`）：从这一帧起，`frames` 帧的一声吸气。
    Inhale { frames: u32 },
    /// 逐音古琴奏法，在同帧起音之前按键位与通道投递，和弦中的各音互不覆盖。
    Technique { key: u8, ch: Option<u8>, technique: crate::zither::NoteTag },
    /// 换成第 `i` 个调律（[`crate::tuning::changes`] 的顺序）：之后起音的音按它，已经在响的不变。
    Tuning(u16),
}

/// 带弯音的音符轮流用的通道（避开鼓的 9 和旋律乐器的 0）。
pub(crate) const BEND_CHANNELS: [u8; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15];

/// `bend` 折线在音符起点后 `beats` 拍的弯音轮值。
fn bend_value(bend: &[(f64, f64)], beats: f64) -> i16 {
    bend_value_with_range(bend, beats, 2.0)
}

fn bend_value_with_range(bend: &[(f64, f64)], beats: f64, range: f64) -> i16 {
    let i = bend.partition_point(|p| p.0 <= beats);
    let semis = match (i.checked_sub(1).map(|k| bend[k]), bend.get(i)) {
        (Some(a), Some(b)) => a.1 + (b.1 - a.1) * (beats - a.0) / (b.0 - a.0),
        (Some(a), None) => a.1,
        (None, Some(b)) => b.1,
        (None, None) => 0.0,
    };
    (semis / range * 8192.0).round().clamp(-8192.0, 8191.0) as i16
}

#[derive(Clone)]
struct Ctx {
    /// 速度表（拍 ↔ 秒）。
    tm: Arc<muser_core::tempo::TempoMap>,
    sr: f64,
    settings: RenderSettings,
    total_frames: usize,
    spec: ProcessSpec,
}

impl Ctx {
    fn beat_at(&self, frame: usize) -> f64 {
        self.tm.beat_at(frame as f64 / self.sr)
    }
    fn frame_at(&self, beat: f64) -> usize {
        (self.tm.secs_at(beat) * self.sr).round().max(0.0) as usize
    }
    /// 效果链里每格起作用的帧 `[from, until)`。
    fn windows(&self, effects: &[muser_core::EffectSlot]) -> Vec<(usize, usize)> {
        effects.iter().map(|e| (e.from.map_or(0, |b| self.frame_at(b.0)), e.until.map_or(usize::MAX, |b| self.frame_at(b.0)))).collect()
    }
}

/// 扫弦：每个音比写的起点晚几秒。同一刻起音的 3 个以上的音是一个和弦，下扫从低音弦起、上扫从高音弦起
/// （[`crate::electric::upstroke`]），相邻两根弦隔 `gap` 秒。`gap` 为 0 时都不动。
fn strum_offsets(notes: &[muser_core::Note], clip_start: f64, gap: f64) -> Vec<f64> {
    let mut out = vec![0.0; notes.len()];
    if gap <= 0.0 {
        return out;
    }
    let mut seen = vec![false; notes.len()];
    for i in 0..notes.len() {
        if seen[i] {
            continue;
        }
        let mut chord: Vec<usize> = (i..notes.len()).filter(|&j| (notes[j].start.0 - notes[i].start.0).abs() < 1e-6).collect();
        chord.iter().for_each(|&j| seen[j] = true);
        if chord.len() < 3 {
            continue;
        }
        chord.sort_by_key(|&j| notes[j].pitch);
        if crate::electric::upstroke(clip_start + notes[i].start.0) {
            chord.reverse();
        }
        for (rank, j) in chord.into_iter().enumerate() {
            out[j] = rank as f64 * gap;
        }
    }
    out
}

/// 一条音轨的全部演奏动作（样本帧, 动作），按时间排序；同帧先 Off 后 On。
fn schedule(project: &Project, ti: usize, cx: &Ctx) -> Vec<(usize, Act)> {
    let track = &project.tracks[ti];
    let (sr, tm) = (cx.sr, &*cx.tm);
    let mut acts: Vec<(usize, Act)> = Vec::new();
    let mut bent = 0usize;
    let sings = track.instrument.trim().to_lowercase().starts_with("choir.");
    let singer = track.instrument.trim().to_lowercase().starts_with("yinse.voice.");
    // 0.4 只唱元音（哼唱）：辅音的模拟听起来像另一个声音，默认关；`"@consonants"` 给正数时打开（实验）。
    let consonants = track.instrument_params.get("@consonants").and_then(serde_json::Value::as_f64).unwrap_or(0.0) > 0.0;
    for (ci, clip) in track.clips.iter().enumerate() {
        // notes 加上 abc、grid、drummer 展开的音符（工程校验时已检查过写法）。
        let mut all = crate::clip_notes(project, ti, ci).unwrap_or_default();
        // 颤音、虚颤音（笛子的指颤、半孔颤；其它乐器一样）：拆成交替的音 / 写成弯音（按 clip 开头的速度）。
        if all.iter().any(|n| n.trill > 0 || n.half_trill || n.scoop > 0.0 || n.fall > 0.0) {
            all = crate::chinese::expand_trills(&all, tm.bpm_at(clip.start.0) / 60.0);
        }
        // 轮音（琵琶的轮指、扬琴的轮音、古筝的摇指）：拆成一串重复的音（按 clip 开头的速度）。
        if let Some(rate) = crate::chinese::tremolo_rate(&track.instrument, &track.instrument_params) {
            if all.iter().any(|n| n.tremolo || n.articulation.as_deref() == Some("tremolo")) {
                let shape = crate::modal::table_of(&track.instrument).and_then(|t| t.gestures.tremolo);
                all = crate::chinese::expand_tremolo(&all, rate, tm.bpm_at(clip.start.0) / 60.0, shape);
            }
        }
        // 换气（`yinse.voice.*` 的 `breath`）：前一个音提早收住，空出一口气的时间（约 0.22 秒，在这个字的辅音之前）。
        let mut inhales: Vec<(f64, f64)> = Vec::new();
        if singer && all.iter().any(|n| n.breath) {
            let bps = tm.bpm_at(clip.start.0) / 60.0;
            let starts: Vec<(f64, f64)> = all
                .iter()
                .filter(|n| n.breath)
                .map(|n| (n.start.0, n.lyric.as_deref().map_or(0.0, crate::singer::lead_seconds) * bps))
                .collect();
            for (at, lead) in starts {
                let gap = 0.22 * bps;
                let mut end = None;
                for m in all.iter_mut().filter(|m| m.start.0 < at - 1e-9 && m.start.0 + m.length.0 > at - gap - lead) {
                    let room = at - lead - m.start.0;
                    let keep = (room - gap).max(room * 0.6).max(0.05);
                    m.length = muser_core::Beats(keep);
                    end = Some(end.map_or(m.start.0 + keep, |e: f64| e.max(m.start.0 + keep)));
                }
                // 吸气在空出来的地方；这个字是片段的第一个音时（前一个音在上一个片段里），就在它前面一口气的长度里。
                inhales.push((end.unwrap_or(at - lead - gap), at - lead));
            }
        }
        let notes: &[muser_core::Note] = &all;
        let c0 = clip.start.0;
        // 片段里第 `b` 拍（相对片段开头）离片段开头几秒。
        let rel = |b: f64| tm.secs_between(c0, b);
        let clip_start = tm.secs_at(c0) * sr;
        let clip_len_frames = (rel(clip.length.0) * sr) as usize;
        let clip_end_frame = clip_start as usize + clip_len_frames;
        // 连奏：延长到下一个不同音高的音之后约 40 ms（空隙超过 1/4 拍视为休止，不连）。
        let legato_len = |i: usize| -> Option<f64> {
            let n = &notes[i];
            let end = n.start.0 + n.length.0;
            let next = notes.iter().filter(|m| m.start.0 > n.start.0 + 1e-9).map(|m| m.start.0).fold(f64::INFINITY, f64::min);
            if !next.is_finite() || next - end > 0.25 {
                return None;
            }
            // 换气之前不连。
            if notes.iter().any(|m| m.breath && (m.start.0 - next).abs() < 1e-9) {
                return None;
            }
            let at_next: Vec<&muser_core::Note> = notes.iter().filter(|m| (m.start.0 - next).abs() < 1e-9).collect();
            if at_next.iter().any(|m| m.pitch == n.pitch) {
                return None;
            }
            let overlap = 0.04 * tm.bpm_at(c0 + n.start.0) / 60.0;
            Some((next - n.start.0 + overlap).max(n.length.0))
        };
        let lengths: Vec<Beats> =
            (0..notes.len()).map(|ni| if clip.legato { legato_len(ni).map_or(notes[ni].length, Beats) } else { notes[ni].length }).collect();
        // 人性化：clip 的覆盖音轨的；种子默认由工程 seed 与音轨、clip 位置推导。
        // drummer 的 clip 没写 humanize 时用底子 loop 的律动。
        let loop_feel = match (&clip.humanize, &clip.drummer, crate::bare_loop(clip)) {
            (None, Some(d), _) => crate::loop_library::load_for(&d.base, project.base_dir.as_deref()).ok().and_then(|l| l.humanize),
            (None, None, Some(name)) => crate::loop_library::load_for(name, project.base_dir.as_deref()).ok().and_then(|l| l.humanize),
            _ => None,
        };
        let clip_feel = clip.humanize.as_ref().or(loop_feel.as_ref());
        let performed = muser_core::humanize::Humanize::merged(track.humanize.as_ref(), clip_feel).map(|h| {
            let seed = h.seed.unwrap_or_else(|| muser_core::automation::derive_seed(project.seed, Some(ti), 0x4855_0000 + ci));
            let lens: Vec<f64> = lengths.iter().map(|l| l.0).collect();
            muser_core::humanize::perform(&h, notes, &lens, clip.start.0, seed)
        });
        let strums = strum_offsets(notes, c0, crate::electric::strum_seconds(&track.instrument, &track.instrument_params));
        let vibrato = crate::chinese::vibrato_for(&track.instrument, &track.instrument_params);
        let table = crate::modal::table_of(&track.instrument);
        for (ni, note) in notes.iter().enumerate() {
            let (on, raw_off, velocity) = match &performed {
                None => {
                    let note = &muser_core::Note { length: lengths[ni], ..note.clone() };
                    let on = (clip_start + rel(note.start.0) * sr) as usize;
                    (on, (clip_start + rel((note.start + note.length).0) * sr) as usize, note.velocity)
                }
                Some(p) => {
                    let p = p[ni];
                    let at = |beats: f64| (clip_start + (rel(beats) + p.offset_s) * sr).max(0.0) as usize;
                    (at(p.start), at(p.start + p.length), p.velocity)
                }
            };
            let on = on + (strums[ni] * sr) as usize;
            // 音符结束不超过 clip 边界（被裁剪）。
            let off = raw_off.min(clip_end_frame);
            if off <= on {
                continue;
            }
            // 弯音曲线按拍：用这个音开头的速度换算（渐变中按当时的速度近似）。
            let bps = tm.bpm_at(c0 + note.start.0) / 60.0;
            // 古筝的揉弦：音符自己没写弯音时，渲染器写一条（见 crate::chinese）。
            // 录音提取的乐器：写了 `"articulation": "vibrato"` 的音按量到的揉音。
            let vibrato = match (note.articulation.as_deref(), table) {
                (Some("vibrato"), Some(t)) => t.gestures.vibrato.map(|v| (v.depth, v.rate, v.delay)).or(vibrato),
                _ => vibrato,
            };
            let auto = match (note.bend.is_empty() && crate::zither::bends(&track.instrument, &track.instrument_params, note.pitch, note.string_technique()), vibrato) {
                (true, Some(v)) => crate::chinese::vibrato_bend(lengths[ni].0, bps, v),
                _ => Vec::new(),
            };
            let bend: &[(f64, f64)] = if note.bend.is_empty() { &auto } else { &note.bend };
            let ch = (!bend.is_empty()).then(|| {
                bent += 1;
                BEND_CHANNELS[(bent - 1) % BEND_CHANNELS.len()]
            });
            if ch.is_some() {
                // 每 ~5 ms 一个点，值不变就不发。
                let step = ((sr * 0.005) as usize).max(1);
                let mut last = None;
                for f in (on..off).step_by(step) {
                    let beats = (f - on) as f64 / sr * bps;
                    let v = if muser_core::takes_technique(&track.instrument) {
                        bend_value_with_range(bend, beats, crate::zither::BEND_RANGE)
                    } else {
                        bend_value(bend, beats)
                    };
                    if last != Some(v) {
                        acts.push((f, Act::Bend { value: v, ch }));
                        last = Some(v);
                    }
                }
            }
            let mut on_at = on;
            if let Some(text) = note.lyric.as_deref().filter(|_| sings) {
                let mut syl = crate::lyric::parse(text);
                // 连奏里紧接着上一个音的音节不发辅音，元音直接滑过去。
                let tied = clip.legato
                    && notes.iter().any(|m| m.start.0 < note.start.0 - 1e-9 && note.start.0 - (m.start.0 + m.length.0) <= 0.25 + 1e-9);
                if tied || !consonants {
                    syl.onset = crate::lyric::Onset::None;
                }
                if !consonants && syl.coda == crate::lyric::Coda::Hiss {
                    syl.coda = crate::lyric::Coda::None;
                }
                let lead = ((syl.lead() * sr) as usize).min(on);
                if syl.voiced_onset() {
                    on_at = on - lead;
                }
                acts.push((on - lead, Act::Sing { syl, text: crate::lyric::Text::new(text), lead: lead as u32, len: (off - on) as u32, velocity }));
            }
            if singer {
                if let Some(text) = note.lyric.as_deref() {
                    // 辅音提前：元音落在拍上，辅音（成阻、爆破、摩擦、送气）在它之前。
                    // 开头来不及提前时，辅音照样唱完（元音晚一点）。
                    let lead = (crate::singer::lead_seconds(text) * sr) as usize;
                    on_at = on_at.saturating_sub(lead);
                    acts.push((on_at, Act::Lyric { text: crate::lyric::Text::new(text), len: (off - on) as u32, lead: lead as u32 }));
                }
            }
            acts.push((on_at, Act::On { key: note.pitch, velocity, accent: note.accent, mute: note.mute, ch }));
            if note.damp || note.flutter || note.slide || note.string_technique().is_some() || note.articulation.is_some() {
                let art = note.articulation.as_deref();
                let technique = crate::zither::NoteTag {
                    technique: note.string_technique(),
                    damp: note.damp || art == Some("damp"),
                    flutter: note.flutter,
                    slide: note.slide,
                    articulation: match (art, table) {
                        (Some(a), Some(t)) => crate::modal::articulation_index(t, a),
                        _ => 0,
                    },
                };
                acts.push((on_at, Act::Technique { key: note.pitch, ch, technique }));
            }
            if singer && note.slide_out {
                // 滑出：在下一个字（的元音）开始前、这个字的最后一段里滑到下一个音的音高（至多 0.35 秒、这个音的四成）。
                if let Some(next) = notes.iter().filter(|m| m.start.0 > note.start.0 + 1e-9).min_by(|a, b| a.start.0.total_cmp(&b.start.0)) {
                    let arrive = (clip_start + rel(next.start.0) * sr) as usize;
                    let frames = ((0.35 * sr) as usize).min(arrive.saturating_sub(on) * 2 / 5).max(1);
                    acts.push((arrive.saturating_sub(frames), Act::Slide { key: next.pitch, frames: frames as u32 }));
                }
            }
            if singer && note.slide {
                // 滑进这个音：从上一个字的音高起，在这个字的开头（至多 0.4 秒、这个音的一半）里滑到自己的音高。
                let frames = ((0.4 * sr) as usize).min(off.saturating_sub(on) / 2).max(1);
                acts.push((on_at, Act::Slide { key: note.pitch, frames: frames as u32 }));
            }
            acts.push((off, Act::Off { key: note.pitch, ch }));
        }
        for &(a, b) in &inhales {
            let (from, to) = ((clip_start + rel(a) * sr) as usize, (clip_start + rel(b) * sr) as usize);
            if to > from {
                acts.push((from, Act::Inhale { frames: (to - from) as u32 }));
            }
        }
        for &(a, b) in &clip.pedal {
            let down = (clip_start + rel(a) * sr) as usize;
            let up = ((clip_start + rel(b) * sr) as usize).min(clip_end_frame);
            if up > down {
                acts.push((down, Act::Pedal { down: true }));
                acts.push((up, Act::Pedal { down: false }));
            }
        }
    }
    // 段落里换调律：在段落开头换（乐器从下一个起音起按新的律制）。
    for (i, (beat, _)) in project.tuning_changes().iter().enumerate() {
        acts.push((cx.frame_at(*beat), Act::Tuning(i as u16)));
    }
    // 同一帧：先松键，再抬踏板，再踩踏板，最后按新音（换踏板时新音不被旧踏板吞掉，松开的旧音在抬踏板时停）。
    acts.sort_by_key(|&(frame, act)| {
        let order = match act {
            Act::Off { .. } => 0,
            Act::Pedal { down: false } => 1,
            Act::Pedal { down: true } => 2,
            Act::Bend { .. } | Act::Tuning(_) => 3,
            Act::Sing { .. } | Act::Lyric { .. } | Act::Inhale { .. } | Act::Technique { .. } => 4,
            // 滑音紧跟在起音之后（起音先定下目标音高，滑音再把滑的快慢放慢）。
            Act::Slide { .. } => 6,
            Act::On { .. } => 5,
        };
        (frame, order)
    });
    acts
}

/// 一条音轨的演奏动作，以 MIDI tick（每四分音符 `tpq` 个）计时。与渲染用的完全相同
/// （humanize、连奏、clip 裁剪、踏板），只是把"每秒多少帧"换成"每秒多少 tick"。
pub(crate) fn schedule_ticks(project: &Project, ti: usize, tpq: u16) -> Vec<(usize, Act)> {
    // tick 按拍算，与速度无关："每秒 tpq·BPM/60 个 tick"在速度不变时就是拍 × tpq。
    // 速度会变时用 60 BPM、每秒 tpq 个 tick（秒 = 拍）：tick 准确。
    // shortcut: 变速的工程里 humanize 的毫秒偏移按 60 BPM 换成 tick（偏差是 BPM/60 倍）；要准时按每个音当时的速度换算。
    let tm = project.tempo_map();
    let (tm, bpm) = if tm.is_constant() { (tm, project.tempo) } else { (muser_core::tempo::TempoMap::constant(60.0), 60.0) };
    // 渲染按帧截断取整；拍 → 秒 → tick 的浮点误差可能让整拍落成 479.9999 → 479，放大一点点避免少一 tick。
    let sr = f64::from(tpq) * bpm / 60.0 * (1.0 + 1e-9);
    let settings = RenderSettings::default();
    let spec = ProcessSpec { sample_rate: sr, max_block_size: settings.block_size, num_channels: 2 };
    schedule(project, ti, &Ctx { tm: Arc::new(tm), sr, settings, total_frames: 0, spec })
}

/// 从 `start` 帧开始演奏时的动作：之前的都去掉，那时还按着的音与踏板在 `start` 补发。
fn chase(acts: Vec<(usize, Act)>, start: usize) -> Vec<(usize, Act)> {
    if start == 0 {
        return acts;
    }
    let split = acts.partition_point(|&(at, _)| at < start);
    let (mut held, mut pedal, mut wheels, mut tuning) = (Vec::new(), false, Vec::<Act>::new(), None);
    let mut guqin = Vec::<(usize, Act)>::new();
    for &(at, act) in &acts[..split] {
        match act {
            Act::On { key, ch, .. } => {
                guqin.retain(|(tag_at, g)| *tag_at == at || !matches!(g, Act::Technique { key: k, ch: c, .. } if *k == key && *c == ch));
                held.push(act);
            }
            Act::Off { key, ch } => {
                guqin.retain(|(_, g)| !matches!(g, Act::Technique { key: k, ch: c, .. } if *k == key && *c == ch));
                if let Some(i) = held.iter().position(|a| matches!(a, Act::On { key: k, ch: c, .. } if *k == key && *c == ch)) {
                    held.remove(i);
                }
            }
            Act::Pedal { down } => pedal = down,
            Act::Bend { ch, .. } => {
                wheels.retain(|w| !matches!(w, Act::Bend { ch: c, .. } if *c == ch));
                wheels.push(act);
            }
            Act::Tuning(_) => tuning = Some(act),
            Act::Technique { key, ch, .. } => {
                guqin.retain(|(_, g)| !matches!(g, Act::Technique { key: k, ch: c, .. } if *k == key && *c == ch));
                guqin.push((at, act));
            }
            Act::Sing { .. } | Act::Lyric { .. } | Act::Slide { .. } | Act::Inhale { .. } => {}
        }
    }
    let mut rest: Vec<(usize, Act)> = acts[split..].to_vec();
    // 恰好在 `start` 松开的音不补发。
    rest.retain(|&(at, act)| match act {
        Act::Off { key, ch } if at == start => match held.iter().position(|a| matches!(a, Act::On { key: k, ch: c, .. } if *k == key && *c == ch)) {
            Some(i) => {
                held.remove(i);
                false
            }
            None => true,
        },
        _ => true,
    });
    let mut out: Vec<(usize, Act)> = Vec::with_capacity(rest.len() + held.len() + 2);
    out.extend(tuning.map(|a| (start, a)));
    if pedal {
        out.push((start, Act::Pedal { down: true }));
    }
    out.extend(wheels.into_iter().map(|a| (start, a)));
    out.extend(guqin.into_iter().map(|(_, g)| g).filter(|g| {
        matches!(g, Act::Technique { key, ch, .. } if held.iter().any(|a| matches!(a, Act::On { key: k, ch: c, .. } if k == key && c == ch)))
    }).map(|a| (start, a)));
    out.extend(held.into_iter().map(|a| (start, a)));
    out.extend(rest);
    out
}

/// 边演奏边出声的一条音轨乐器（干声）。每一步的块长只取决于当前位置、下一个事件与自动化，
/// 和调用方要多少无关，所以分几次取与一次取完结果相同。
struct LiveInstrument {
    inst: instrument::TrackInstrument,
    channel: u8,
    /// 有带弯音的音符：踏板也发到它们的通道。
    bends: bool,
    acts: Vec<(usize, Act)>,
    /// `Act::Tuning(i)` 换成的调律表（[`crate::tuning::changes`]）。
    tunings: Vec<manguebyte::dsp::Tuning>,
    next: usize,
    frame: usize,
    lanes: Vec<automate::Lane>,
    block: AudioBuffer<f32>,
    out: Tape,
    /// 直接输出（`@outputs` 写了 `bus` 的各路），与路由图里读它们的总线共用。
    taps: Vec<SharedTape>,
    name: String,
    instrument: String,
    /// 乐器参数（JSON 文本，对账时比较）。
    params: String,
    /// 唱音节：叠在人声上的辅音，与排好的元音变化（帧, 元音 0–4）。
    consonants: Option<crate::lyric::Consonants>,
    vowels: Vec<(usize, f32)>,
    /// 现场模式：安静了一段时间、又没有新的动作时不处理（省 CPU）。离线渲染不这样做。
    may_rest: bool,
    /// 连续安静了多少帧。
    quiet: usize,
}

/// 安静（峰值低于这个）超过 [`REST_AFTER`] 秒、之后也没有新动作的乐器 / 效果链，现场模式下先不处理。
const SILENT: f32 = 1e-7;
const REST_AFTER: f64 = 1.0;

fn peak(buf: &AudioBuffer<f32>) -> f32 {
    (0..buf.num_channels()).flat_map(|c| buf.channel(c).iter()).fold(0f32, |m, x| m.max(x.abs()))
}

fn build_instrument(project: &Project, ti: usize, cx: &Ctx) -> Result<instrument::TrackInstrument, EngineError> {
    let track = &project.tracks[ti];
    let mut inst = instrument::resolve_for(&track.instrument, &track.instrument_params, &cx.spec)?;
    if let Some(t) = crate::tuning::build(project).map_err(invalid)? {
        inst.set_tuning(&t);
    }
    inst.prepare(&cx.spec);
    Ok(inst)
}

/// 一段带子，演奏的音轨写、总线读（插件的直接输出）。
type SharedTape = Arc<std::sync::Mutex<Tape>>;

fn lock(t: &SharedTape) -> std::sync::MutexGuard<'_, Tape> {
    t.lock().unwrap_or_else(|e| e.into_inner())
}

impl LiveInstrument {
    /// 从第 `start` 帧开始演奏。`start` 之前按下、到这时还没松开的音（以及踩着的踏板）在 `start` 补发（chase）。
    fn new(project: &Project, ti: usize, cx: &Ctx, beats: f64, start: usize) -> Result<Self, EngineError> {
        let track = &project.tracks[ti];
        let inst = build_instrument(project, ti, cx)?;
        let lanes = automate::compile_track(project, ti, beats).map_err(invalid)?.instrument;
        let acts = chase(schedule(project, ti, cx), start);
        let sings = acts.iter().any(|a| matches!(a.1, Act::Sing { .. }));
        Ok(Self {
            channel: inst.channel(),
            bends: acts.iter().any(|a| matches!(a.1, Act::Bend { .. })),
            inst,
            acts,
            tunings: crate::tuning::changes(project).map_err(invalid)?,
            next: 0,
            frame: start,
            lanes,
            block: AudioBuffer::<f32>::new(2, cx.settings.block_size),
            out: Tape { start, ..Default::default() },
            taps: Vec::new(),
            name: track.name.clone(),
            instrument: track.instrument.clone(),
            params: serde_json::to_string(&track.instrument_params).unwrap_or_default(),
            consonants: sings.then(|| {
                let gain = track.instrument_params.get("@consonants").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
                crate::lyric::Consonants::new(cx.sr, gain)
            }),
            vowels: Vec::new(),
            may_rest: false,
            quiet: 0,
        })
    }

    /// 换成新工程里这条轨的排程与乐器自动化（现场模式对账）：还没执行的动作按新工程来，乐器状态不动。
    /// 新工程在当前位置之前和旧的一样，所以按着的音会在新排程里照常松开。
    fn reschedule(&mut self, project: &Project, ti: usize, cx: &Ctx, beats: f64) -> Result<(), EngineError> {
        let frame = self.frame;
        self.acts = schedule(project, ti, cx).into_iter().filter(|a| a.0 >= frame).collect();
        self.next = 0;
        self.tunings = crate::tuning::changes(project).map_err(invalid)?;
        self.bends |= self.acts.iter().any(|a| matches!(a.1, Act::Bend { .. }));
        self.lanes = automate::compile_track(project, ti, beats).map_err(invalid)?.instrument;
        Ok(())
    }

    /// 演奏到 `until` 帧（不超过工程结尾）。
    fn run_to(&mut self, until: usize, cx: &Ctx) -> Result<(), EngineError> {
        let (block_size, total_frames) = (cx.settings.block_size, cx.total_frames);
        let until = until.min(total_frames);
        while self.frame < until {
            let frame = self.frame;
            let acted = self.acts.get(self.next).is_some_and(|a| a.0 <= frame);
            while let Some(&(at, act)) = self.acts.get(self.next) {
                if at > frame {
                    break;
                }
                match act {
                    Act::On { key, velocity, accent, mute, ch } => self.inst.note_on_muted(NoteRef::new(ch.unwrap_or(self.channel), key), velocity, accent, mute),
                    Act::Off { key, ch } => self.inst.note_off(NoteRef::new(ch.unwrap_or(self.channel), key), 0.0),
                    Act::Pedal { down } => {
                        self.inst.pedal(None, down);
                        if self.bends {
                            BEND_CHANNELS.iter().for_each(|&c| self.inst.pedal(Some(c), down));
                        }
                    }
                    Act::Bend { value, ch } => self.inst.bend(ch, value),
                    Act::Tuning(i) => {
                        if let Some(t) = self.tunings.get(usize::from(i)) {
                            self.inst.set_tuning(t);
                        }
                    }
                    Act::Lyric { text, len, lead } => self.inst.lyric(text.as_str(), u64::from(len), u64::from(lead)),
                    Act::Slide { key, frames } => self.inst.slide(key, u64::from(frames)),
                    Act::Inhale { frames } => self.inst.inhale(u64::from(frames)),
                    Act::Technique { key, ch, technique } => self.inst.technique(NoteRef::new(ch.unwrap_or(self.channel), key), technique),
                    Act::Sing { syl, lead, len, velocity, .. } => {
                        let (lead, len) = (lead as usize, len as usize);
                        if let Some(c) = self.consonants.as_mut() {
                            c.sing(&syl, at, lead, len, velocity);
                        }
                        // 元音：鼻音先闭着嘴（u）哼，近音从相近的元音滑进来；其余开口就是第一个元音。
                        let plan = syl.vowel_plan();
                        let first = match syl.onset {
                            crate::lyric::Onset::Nasal => crate::lyric::NASAL_VOWEL,
                            crate::lyric::Onset::Glide { from } => from,
                            _ => plan[0].1,
                        };
                        self.inst.set_param("vowel", f64::from(first));
                        let vowel_at = at + lead;
                        self.vowels.extend(plan.iter().map(|&(t, v)| (vowel_at + (t * len as f64) as usize, v)));
                        if syl.coda == crate::lyric::Coda::Nasal && len > 0 {
                            self.vowels.push((vowel_at + len * 4 / 5, crate::lyric::NASAL_VOWEL));
                        }
                        self.vowels.sort_by_key(|v| v.0);
                    }
                }
                self.next += 1;
            }
            while let Some(&(at, v)) = self.vowels.first() {
                if at > frame {
                    break;
                }
                self.inst.set_param("vowel", f64::from(v));
                self.vowels.remove(0);
            }
            for lane in self.lanes.iter_mut() {
                if let Some(v) = lane.poll(cx.beat_at(frame)) {
                    self.inst.set_param(&lane.param, v);
                }
            }
            let step = if self.lanes.is_empty() { block_size } else { block_size.min(automate::BLOCK) };
            let next_at = self.acts.get(self.next).map_or(total_frames, |&(at, _)| at.min(total_frames));
            let next_at = self.vowels.first().map_or(next_at, |v| v.0.min(next_at));
            let end = (frame + step).min(next_at).max(frame + 1).min(total_frames);
            self.block.set_num_frames(end - frame);
            if acted || !self.vowels.is_empty() {
                self.quiet = 0;
            }
            let resting = self.may_rest && self.quiet as f64 >= REST_AFTER * cx.sr;
            if resting {
                // 安静了一秒以上，到下一个动作之前不会出声：不处理（块在下一个动作处切开）。
                for c in 0..self.block.num_channels() {
                    self.block.channel_mut(c).fill(0.0);
                }
            } else {
                self.inst.process(&mut self.block);
                if self.may_rest {
                    self.quiet = if peak(&self.block) < SILENT { self.quiet + (end - frame) } else { 0 };
                }
            }
            if let Some(c) = self.consonants.as_mut() {
                c.add_to(&mut self.block, frame);
            }
            // 上游模型在极端参数下可能数值发散；宁可报错也不放出爆音。
            if (0..2).any(|ch| self.block.channel(ch).iter().any(|s| !s.is_finite() || s.abs() > 64.0)) {
                return Err(EngineError::Unstable(format!(
                    "track '{}' ({}) output diverged: adjust instrument_params (e.g. increase release / attack)",
                    self.name, self.instrument
                )));
            }
            let n = self.block.num_frames();
            self.out.push(&self.block, 0..n);
            for (tape, buf) in self.taps.iter().zip(self.inst.taps()) {
                lock(tape).push(buf, 0..n);
            }
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
            let _t = crate::transport::set(&project, start);
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
                        Err(_) => return Err(EngineError::Unstable("the track's playing thread has exited".into())),
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
        "ti": ti, "seed": project.seed, "tempo": project.tempo, "tm": format!("{:?}", cx.tm.points()), "tu": project.tuning, "tt": format!("{:?}", project.tuning_changes()), "tr": project.transpose, "ts": [ts.numerator, ts.denominator],
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
    let mut r = Renderer::build(project, settings, plan, Some(cache), false, false, 0, false, None)?.recording().with_loudness();
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

/// 侧链键节点：音轨/总线上压缩器、声码器引用的，以及主总线上的（后者按 `master_effects` 下标给出）。
fn sidechain_keys(project: &Project, plan: &Plan) -> Result<(HashSet<Node>, Vec<Option<Node>>), EngineError> {
    let mut key_nodes: HashSet<Node> = plan.sidechains.iter().map(|s| s.key).collect();
    let mut master_keys: Vec<Option<Node>> = Vec::new();
    for e in &project.master_effects {
        let k = e.effect.sidechain().map(|name| routing::find_node(project, name)).transpose().map_err(invalid)?;
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
        let tm = project.tempo_map();
        let duration = tm.secs_at(project.duration_beats().0);
        Ctx {
            tm: Arc::new(tm),
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
    let mut r = Renderer::build(project, settings, plan, Some(cache), false, meters.is_some(), 0, false, None)?.recording();
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
    /// 对账（[`Renderer::reconcile`]）时认节点用：轨名、总线名，主总线是空串。
    ident: String,
    /// 效果链的写法（对账时比较）。
    slots: Vec<muser_core::EffectSlot>,
    /// 要处理：未静音、作为侧链键，或要求静音节点也照常处理。
    active: bool,
    muted: bool,
    /// 音轨的乐器。
    source: Option<Source>,
    chain: Vec<Fx>,
    /// 每个效果器起作用的帧 `[from, until)`。
    windows: Vec<(usize, usize)>,
    lanes: TrackLanes,
    latency: usize,
    gain_db: f64,
    pan: f64,
    law: fn(f64, f64) -> (f32, f32),
    /// 每个效果器的侧链键（`strips` 下标）。
    keys: Vec<Option<usize>>,
    /// 送进本节点的边：(`plan.edges` 下标, 来源的 `strips` 下标)，按求和顺序。
    inputs: Vec<(usize, usize)>,
    /// 送进本节点的直接输出：(来源音轨的 `strips` 下标, 带子)。
    tap_inputs: Vec<(usize, SharedTape)>,
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
    /// 这个节点的乐器与效果链一共用了多少时间（[`Renderer::costs`]）。
    busy: std::time::Duration,
    /// 现场模式：输入、输出都安静了多少帧（够久就不处理效果链）。
    quiet: usize,
    /// 推子后输出的均方（约 1.5 秒的指数滑动平均，[`Renderer::loudness_now`]）。
    ms: f64,
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
    /// 插件走带用的速度表（每次 process 都设到当前线程）。
    tempo_map: std::sync::Arc<manguebyte::dsp::timeline::TempoMap>,
    beats: f64,
    meters: bool,
    settings: RenderSettings,
    /// 现场模式（[`Renderer::live`]）：乐器都在当前线程边演奏边出声，可以 [`Renderer::reconcile`]。
    live: bool,
}

/// 对账时带过去的旧节点（按身份）。
struct Reuse {
    strips: HashMap<String, Strip>,
}

/// 节点的身份：轨名 / 总线名（没名字时用序号），主总线是空串。
fn ident(project: &Project, node: Option<Node>) -> String {
    match node {
        Some(Node::Track(i)) => format!("t:{}", if project.tracks[i].name.is_empty() { format!("#{i}") } else { project.tracks[i].name.clone() }),
        Some(Node::Bus(i)) => format!("b:{}", project.buses[i].name),
        None => String::new(),
    }
}

/// 两条效果链写法上是否同一个效果器（不看 from / until）。
fn same_slot(a: &muser_core::EffectSlot, b: &muser_core::EffectSlot) -> bool {
    a.effect == b.effect && a.bypass == b.bypass
}

impl Renderer {
    /// 新建。`cache` 里有的音轨乐器输出直接用，其余边演奏边出声。
    /// `all_nodes`：静音的节点也照常处理（只是不送出），播放中切换静音时声音连续。
    pub fn new(project: &Project, settings: RenderSettings, cache: Option<&RenderCache>, all_nodes: bool) -> Result<Self, EngineError> {
        crate::validate(project)?;
        let plan = routing::plan(project).map_err(invalid)?;
        Self::build(project, settings, plan, cache, all_nodes, false, 0, false, None)
    }

    /// 同 [`Renderer::new`]，但从第 `frame` 帧（向下对齐到处理段）开始，用于播放中跳转：不从头算，立即可播。
    /// 那时还按着的音与踏板会补发；缓存里有的音轨乐器输出与从头播完全相同，
    /// 但效果器从空状态开始（跳转点之前的混响、延迟尾巴听不到），和 DAW 的跳转一样。
    pub fn new_at(project: &Project, settings: RenderSettings, cache: Option<&RenderCache>, all_nodes: bool, frame: usize) -> Result<Self, EngineError> {
        crate::validate(project)?;
        let plan = routing::plan(project).map_err(invalid)?;
        Self::build(project, settings, plan, cache, all_nodes, false, frame, false, None)
    }

    /// 现场模式：从头开始，所有节点都处理（静音随时可开），乐器都在当前线程里演奏，之后可以 [`Renderer::reconcile`]。
    /// 不支持音频轨（整轨干声要按整首长度铺好）。
    pub fn live(project: &Project, settings: RenderSettings) -> Result<Self, EngineError> {
        crate::validate(project)?;
        if project.tracks.iter().any(|t| t.instrument.trim() == muser_core::audio::INSTRUMENT) {
            return Err(invalid("live mode does not support audio tracks".into()));
        }
        let plan = routing::plan(project).map_err(invalid)?;
        Self::build(project, settings, plan, None, true, false, 0, true, None)
    }

    /// 换成新的工程，不停、不重建：已有的节点（按轨名、总线名认）乐器与效果器的状态原样带过去，
    /// 之后的音符、自动化、推子按新工程走，追加的效果器、新加的音轨从空状态开始。只用于现场模式。
    ///
    /// 新工程必须是在旧工程上"往后加"：已经演奏过的部分不变（改的只是还没演奏的音符与自动化）；音轨、效果器只加不删
    /// （撤下用 `until`，停奏就是不再写片段）；不加总线、不换已有音轨的乐器；中途加的效果器不能带延迟（如限幅器）。
    pub fn reconcile(&mut self, project: &Project) -> Result<(), EngineError> {

        if !self.live {
            return Err(invalid("only a live-mode renderer can swap projects".into()));
        }
        crate::validate(project)?;
        if project.tracks.iter().any(|t| t.instrument.trim() == muser_core::audio::INSTRUMENT) {
            return Err(invalid("live mode does not support audio tracks".into()));
        }
        let plan = routing::plan(project).map_err(invalid)?;
        // 先查，查过了才动现有的节点（出错时渲染器保持原样）。
        let arranged = crate::arrange::arranged(project).map_err(invalid)?;
        let p = &*arranged;
        let old: HashMap<&str, &Strip> = self.strips.iter().map(|s| (s.ident.as_str(), s)).collect();
        let buses: Vec<String> = (0..p.buses.len()).map(|i| ident(p, Some(Node::Bus(i)))).collect();
        let old_buses = self.strips.iter().filter(|s| matches!(s.node, Some(Node::Bus(_)))).count();
        if buses.len() != old_buses || buses.iter().any(|b| !old.contains_key(b.as_str())) {
            return Err(invalid("buses cannot be added, removed or renamed during a live session (create them in the template first)".into()));
        }
        for (i, t) in p.tracks.iter().enumerate() {
            let id = ident(p, Some(Node::Track(i)));
            let Some(o) = old.get(id.as_str()) else { continue };
            if let Some(Source::Live(l)) = &o.source {
                if l.instrument != t.instrument || l.params != serde_json::to_string(&t.instrument_params).unwrap_or_default() {
                    return Err(invalid(format!("track '{}' cannot change instrument or instrument parameters mid-session (use automation for parameters; for a different instrument, start a new track)", t.name)));
                }
            }
        }
        let lists: Vec<(String, &[muser_core::EffectSlot])> = std::iter::once((String::new(), p.master_effects.as_slice()))
            .chain(p.tracks.iter().enumerate().map(|(i, t)| (ident(p, Some(Node::Track(i))), t.effects.as_slice())))
            .chain(p.buses.iter().enumerate().map(|(i, b)| (ident(p, Some(Node::Bus(i))), b.effects.as_slice())))
            .collect();
        for (id, list) in &lists {
            let Some(o) = old.get(id.as_str()) else { continue };
            if list.len() < o.slots.len() || !o.slots.iter().zip(list.iter()).all(|(a, b)| same_slot(a, b)) {
                return Err(invalid(format!("the effect chain of {} can only be appended to (remove an effect with until, change parameters with automation)", if id.is_empty() { "the master" } else { &id[2..] })));
            }
            // shortcut: 插件效果器的延迟要打开才知道，这里不查（带延迟的插件中途加进来，那一轨会错开几毫秒）；要查时把这里建好的链传给 build 复用。
            let builtin: Vec<muser_core::EffectSlot> = list[o.slots.len()..].iter().filter(|e| !matches!(e.effect, muser_core::Effect::Plugin { .. })).cloned().collect();
            let tail = effects::build_chain(&builtin, &self.cx.spec)?;
            if effects::chain_latency(&tail) > 0 {
                return Err(invalid("an effect added mid-session cannot have latency (e.g. a limiter): put it in the template".into()));
            }
        }
        // 已有节点之间新加的连线：读的一方还没读到的部分要还在。
        for e in &plan.edges {
            let from = old.get(ident(p, Some(e.from)).as_str());
            let to = old.get(ident(p, e.to.map(Node::Bus)).as_str());
            if let (Some(f), Some(t)) = (from, to) {
                if f.out.start > t.pos {
                    return Err(invalid("this new connection could not be wired in time: try again in a moment".into()));
                }
            }
        }
        let start = self.strips.iter().map(|s| s.pos).min().unwrap_or(self.pos);
        let pos = self.pos;
        let strips: HashMap<String, Strip> = self.strips.drain(..).map(|s| (s.ident.clone(), s)).collect();
        let mut reuse = Reuse { strips };
        let mut next = Self::build(project, self.settings, plan, None, true, self.meters, start, true, Some(&mut reuse))?;
        next.pos = pos;
        *self = next;
        Ok(())
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
        live: bool,
        mut reuse: Option<&mut Reuse>,
    ) -> Result<Self, EngineError> {
        // 编曲层（form / parts）展开成普通 clip，之后只看展开后的工程。
        let arranged = crate::arrange::arranged(project).map_err(invalid)?;
        let project = &*arranged;
        let cx = Ctx::new(project, settings);
        let bs = settings.block_size;
        let align = bs / gcd(bs, automate::BLOCK) * automate::BLOCK;
        let start = (start.min(cx.total_frames) / align) * align;
        let _transport = crate::transport::set(project, start);
        let beats = automate::total_beats(project, settings.tail_seconds);
        let live_mode = live;
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
            let (muted, effects_list, gain_db, pan, lanes, law): (bool, &[muser_core::EffectSlot], f64, f64, TrackLanes, fn(f64, f64) -> (f32, f32)) = match node {
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
            let keys: Vec<Option<usize>> = (0..effects_list.len())
                .map(|ei| plan.sidechains.iter().find(|s| s.node == node && s.effect == ei).map(|s| index[&s.key]))
                .collect();
            let id = ident(project, Some(node));
            if let Some(mut o) = reuse.as_mut().and_then(|r| r.strips.remove(&id)) {
                if let (Some(Source::Live(l)), Node::Track(i)) = (o.source.as_mut(), node) {
                    l.reschedule(project, i, &cx, beats)?;
                }
                let tail = effects::build_chain(&effects_list[o.chain.len()..], &cx.spec)?;
                o.chain.extend(tail);
                (o.node, o.active, o.muted, o.gain_db, o.pan, o.law, o.lanes, o.keys) = (Some(node), active, muted, gain_db, pan, law, lanes, keys);
                (o.windows, o.slots, o.rank) = (cx.windows(effects_list), effects_list.to_vec(), rank(node));
                (o.inputs, o.tap_inputs, o.readers) = (Vec::new(), Vec::new(), Vec::new());
                strips.push(o);
                continue;
            }
            // 有直接输出的音轨不进缓存（缓存只存立体声，存不下各路直接输出）。
            let cache_key = match node {
                Node::Track(i) if !plan.taps.iter().any(|t| t.track == i) => Some(instrument_key(project, i, &cx)),
                _ => None,
            };
            let source = match (node, active) {
                (Node::Track(i), true) => {
                    let cached = cache.zip(cache_key).and_then(|(c, k)| c.map.get(&k));
                    Some(match cached {
                        Some(c) => Source::Cached(c.clone()),
                        // 音频音轨：整轨干声一次铺好（同一工程再次渲染时取自缓存，不再解码）。
                        None if project.tracks[i].instrument.trim() == muser_core::audio::INSTRUMENT => {
                            let audio = crate::audio::track_buffer(project, i, settings.sample_rate, cx.total_frames).map_err(invalid)?;
                            Source::Cached(Arc::new(Cached::new(audio)))
                        }
                        None if live || project.tracks[i].instrument.trim().starts_with("plugin:") => {
                            let mut live = LiveInstrument::new(project, i, &cx, beats, start)?;
                            live.may_rest = live_mode;
                            live.taps = plan.taps.iter().filter(|t| t.track == i).map(|_| Arc::new(std::sync::Mutex::new(Tape { start, ..Default::default() }))).collect();
                            Source::Live(Box::new(live))
                        }
                        None => Source::Worker(Worker::spawn(Arc::clone(shared.get_or_insert_with(|| Arc::new(project.clone()))), i, cx.clone(), beats, start)),
                    })
                }
                _ => None,
            };
            let chain = if active { effects::build_chain(effects_list, &cx.spec)? } else { Vec::new() };
            let windows = cx.windows(effects_list);
            strips.push(Strip {
                node: Some(node),
                ident: id,
                slots: effects_list.to_vec(),
                active,
                muted,
                source,
                latency: effects::chain_latency(&chain),
                chain,
                windows,
                lanes,
                gain_db,
                pan,
                law,
                keys,
                inputs: Vec::new(),
                tap_inputs: Vec::new(),
                readers: Vec::new(),
                pos: start,
                wet: Tape { start, ..Default::default() },
                out: Tape { start, ..Default::default() },
                meter: Vec::new(),
                rank: rank(node),
                cache_key,
                record: None,
                loud: None,
                busy: Default::default(),
                quiet: 0,
                ms: 0.0,
            });
        }
        let MasterLanes { effects: fx_lanes, gain } = automate::compile_master(project, beats).map_err(invalid)?;
        let master_lanes = TrackLanes { instrument: Vec::new(), effects: fx_lanes, gain, pan: Vec::new(), sends: Default::default() };
        let master_keys: Vec<Option<usize>> = master_keys.iter().map(|k| k.map(|n| index[&n])).collect();
        if let Some(mut o) = reuse.as_mut().and_then(|r| r.strips.remove("")) {
            let tail = effects::build_chain(&project.master_effects[o.chain.len()..], &cx.spec)?;
            o.chain.extend(tail);
            (o.gain_db, o.lanes, o.keys, o.windows, o.slots) = (project.master_gain_db, master_lanes, master_keys, cx.windows(&project.master_effects), project.master_effects.clone());
            (o.inputs, o.tap_inputs, o.readers) = (Vec::new(), Vec::new(), Vec::new());
            strips.push(o);
        } else {
        let chain = effects::build_chain(&project.master_effects, &cx.spec)?;
        strips.push(Strip {
            node: None,
            ident: String::new(),
            slots: project.master_effects.clone(),
            active: true,
            muted: false,
            source: None,
            latency: effects::chain_latency(&chain),
            chain,
            windows: cx.windows(&project.master_effects),
            lanes: master_lanes,
            gain_db: project.master_gain_db,
            pan: 0.0,
            law: master_gains,
            keys: master_keys,
            inputs: Vec::new(),
            tap_inputs: Vec::new(),
            readers: Vec::new(),
            pos: start,
            wet: Tape { start, ..Default::default() },
            out: Tape { start, ..Default::default() },
            meter: Vec::new(),
            rank: (usize::MAX, usize::MAX),
            cache_key: None,
            record: None,
            loud: None,
            busy: Default::default(),
            quiet: 0,
            ms: 0.0,
        });
        }

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
        for tap in &plan.taps {
            let from = index[&Node::Track(tap.track)];
            let to = tap.to.map_or(master, |b| index[&Node::Bus(b)]);
            if let Some(Source::Live(l)) = &strips[from].source {
                let tape = l.taps[tap.index].clone();
                strips[to].tap_inputs.push((from, tape));
            }
        }

        Ok(Renderer {
            align,
            cx,
            plan,
            strips,
            pos: start,
            tempo_map: crate::transport::current(),
            beats,
            meters,
            settings,
            live,
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
        let _transport = crate::transport::set_shared(&self.tempo_map);
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
            for (from, _) in &st.tap_inputs {
                need[*from] = need[*from].max(target[s]);
            }
        }
        for (s, &to) in target.iter().enumerate() {
            if self.strips[s].active {
                self.advance(s, to)?;
            }
        }
        // 直接输出的带子只有读它的总线读：读过的丢掉。
        for st in &self.strips {
            for (_, tape) in &st.tap_inputs {
                lock(tape).trim(st.pos);
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
        let live_mode = self.live;
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
            for (from, tape) in &self.strips[s].tap_inputs {
                let from = &self.strips[*from];
                if from.muted || !from.active {
                    continue;
                }
                lock(tape).read(a, &mut tmp);
                for c in 0..2 {
                    for (d, x) in buf.channel_mut(c).iter_mut().zip(tmp.channel(c)) {
                        *d += *x;
                    }
                }
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
            let t0 = std::time::Instant::now();
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
            // 现场模式：输入是静音、效果链也安静了一秒以上（混响、延迟的尾巴已经没了）就不处理。
            // shortcut: 离线渲染照常处理，所以录下的工程离线渲染时，休息过的效果器内部状态有极小的差别（低于 -140 dB）。
            let silent_in = live_mode && peak(&buf) < SILENT;
            if !(silent_in && st.quiet as f64 >= REST_AFTER * cx.sr) {
                effects::apply_chain(&mut st.chain, &mut buf, bs, &mut st.lanes.effects, &|f| cx.beat_at(a + f), &key_refs, &st.windows, a);
                st.quiet = if silent_in && peak(&buf) < SILENT { st.quiet + n } else { 0 };
            }
            st.busy += t0.elapsed();
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
                if live_mode {
                    // 约 1.5 秒的滑动均方：每帧 ms += (x² - ms) / τ。按块近似：块内平均后一次更新。
                    let n_o = o.num_frames();
                    let sum: f64 = (0..2).flat_map(|c| o.channel(c).iter()).map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / 2.0;
                    let a = (n_o as f64 / (1.5 * cx.sr)).min(1.0);
                    st.ms += (sum / n_o.max(1) as f64 - st.ms) * a;
                }
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
                    let t = project.tracks.get(i).ok_or_else(|| invalid("the project structure changed".into()))?;
                    (s.gain_db, s.muted) = (t.gain_db, t.muted);
                    s.lanes.gain = automate::compile_track(project, i, self.beats).map_err(invalid)?.gain;
                }
                Some(Node::Bus(i)) => {
                    let b = project.buses.get(i).ok_or_else(|| invalid("the project structure changed".into()))?;
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

    /// 第 `frame` 帧所在那一小段（[`METER_WINDOW`] 帧）各节点的峰值（线性，左右取大）：轨名 / 总线名，主总线是空串。
    /// 要先 [`Renderer::with_meters`]。
    // shortcut: 电平从头记到尾，现场演奏一小时每个节点约 1.4 MB；更长时改成只留最近一段。
    pub fn levels(&self, frame: usize) -> Vec<(String, f32)> {
        let w = frame / METER_WINDOW;
        self.strips
            .iter()
            .map(|s| (s.ident.get(2..).unwrap_or("").to_string(), s.meter.get(w).map_or(0.0, |p| p[0].max(p[1]))))
            .collect()
    }

    /// 现场模式：各节点（轨名 / 总线名，主总线是空串）推子后约 1.5 秒的响度（RMS，dBFS；没声音时 None）。
    pub fn loudness_now(&self) -> Vec<(String, Option<f64>)> {
        self.strips
            .iter()
            .map(|s| (s.ident.get(2..).unwrap_or("").to_string(), (s.ms > 1e-10).then(|| 10.0 * s.ms.log10())))
            .collect()
    }

    /// 各节点（轨名 / 总线名，主总线是空串）的乐器与效果链到目前为止一共用了多少时间：找出最耗资源的轨。
    pub fn costs(&self) -> Vec<(String, std::time::Duration)> {
        self.strips.iter().map(|s| (s.ident.get(2..).unwrap_or("").to_string(), s.busy)).collect()
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

/// 一段已经渲染好的音频编码成 MP3（320 kbit/s，带工程的标题、速度标签）。`muser view` 导出用。
pub fn mp3_bytes(project: &Project, audio: &AudioBuffer<f32>, sample_rate: u32) -> Result<Vec<u8>, EngineError> {
    let meta = wav_metadata(project, sample_rate);
    let opts = manguebyte::formats::mp3::Mp3Options::constant(320).with_metadata(&meta);
    let mut out = std::io::Cursor::new(Vec::new());
    manguebyte::formats::mp3::write(&mut out, audio, sample_rate, &opts).map_err(|e| EngineError::Wav(format!("failed to write MP3: {e:?}")))?;
    Ok(out.into_inner())
}

/// 渲染工程并写出 WAV 文件（`path` 以 `.mp3` 结尾时写 MP3）。返回 (时长秒, 峰值 dBFS)。
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
    let meta = wav_metadata(project, settings.sample_rate);
    if path.to_lowercase().ends_with(".mp3") {
        // 输出文件名是 .mp3 时直接编码 MP3（320 kbit/s 恒定码率，带标题、速度标签）。
        let opts = manguebyte::formats::mp3::Mp3Options::constant(320).with_metadata(&meta);
        manguebyte::formats::mp3::write_file(path, &audio, settings.sample_rate, &opts).map_err(|e| EngineError::Wav(format!("failed to write MP3: {e:?}")))?;
    } else {
        let file = std::fs::File::create(path).map_err(|e| EngineError::Wav(format!("cannot write {path}: {e}")))?;
        wav::write_with_metadata(std::io::BufWriter::new(file), &audio, settings.sample_rate, WavFormat::Int16, &meta)
            .map_err(|e| EngineError::Wav(format!("{e:?}")))?;
    }
    let seconds = audio.num_frames() as f64 / f64::from(settings.sample_rate);
    let peak_db = decibels::gain_to_db(f64::from(audio.peak()));
    Ok((seconds, peak_db))
}

/// 写进 WAV 的信息：标题、速度、生成软件，以及每个段落起点的标记（cue point，名字如 `chorus`、第二遍 `chorus#2`），
/// DAW 和播放器打开就能按段落跳转。没有 `form` 时没有标记。
pub fn wav_metadata(project: &Project, sample_rate: u32) -> manguebyte::formats::Metadata {
    let mut m = manguebyte::formats::Metadata { bpm: Some(project.tempo as f32), ..Default::default() };
    if !project.name.is_empty() {
        m.title = Some(project.name.clone());
    }
    m.tags.push(("ISFT".into(), format!("muser {}", env!("CARGO_PKG_VERSION"))));
    let tm = project.tempo_map();
    let frames = |beats: f64| (tm.secs_at(beats) * f64::from(sample_rate)).round().max(0.0) as u64;
    for (i, s) in crate::arrange::timeline(project).unwrap_or_default().iter().enumerate() {
        m.cue_points.push(manguebyte::formats::metadata::CuePoint {
            id: i as u32 + 1,
            position: frames(s.start),
            length: frames(s.start + s.length) - frames(s.start),
            label: Some(if s.occurrence > 1 { format!("{}#{}", s.name, s.occurrence) } else { s.name.clone() }),
        });
    }
    m
}

/// 一条分轨：音轨名、写到的文件、峰值 dBFS。
pub struct Stem {
    pub track: String,
    pub path: std::path::PathBuf,
    pub peak_db: f64,
}

/// 分轨导出：每条没静音的音轨各渲染一个 24 位 WAV 到 `dir`（`01-名字.wav`…），带它自己的效果器、推子、声像，
/// 以及它发送到总线的部分（混响、延迟随这条轨走）；侧链照常生效。**不带主总线效果器**（母带处理留给 DAW），
/// 主输出音量与其自动化照旧。各轨长度相同（与整首一样长），在 DAW 里从 0 对齐即可。乐器只渲染一遍（缓存复用）。
pub fn render_stems(
    project: &Project,
    dir: &std::path::Path,
    settings: RenderSettings,
    progress: &mut dyn FnMut(&str, usize, usize),
) -> Result<Vec<Stem>, EngineError> {
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Wav(format!("cannot create directory {}: {e}", dir.display())))?;
    let mut base = project.clone();
    base.master_effects.clear();
    // 主总线效果器的自动化跟着效果器一起去掉（主输出音量的自动化保留）。
    base.automation.retain(|a| !matches!(muser_core::automation::Target::parse(&a.target), Some(muser_core::automation::Target::MasterEffect(_))));
    let mut cache = RenderCache::new();
    let mut out = Vec::new();
    let live: Vec<usize> = (0..project.tracks.len()).filter(|&i| !project.tracks[i].muted).collect();
    for (n, &i) in live.iter().enumerate() {
        let mut q = base.clone();
        for (j, t) in q.tracks.iter_mut().enumerate() {
            t.muted = j != i;
        }
        let name = project.tracks[i].name.clone();
        let audio = render_inner(&q, settings, &mut cache, None, Some(&mut |done, total| progress(&name, n * total + done, live.len() * total)))?;
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
        let path = dir.join(format!("{:02}-{safe}.wav", n + 1));
        let mut meta = wav_metadata(project, settings.sample_rate);
        meta.title = Some(name.clone());
        let file = std::fs::File::create(&path).map_err(|e| EngineError::Wav(format!("cannot write {}: {e}", path.display())))?;
        wav::write_with_metadata(std::io::BufWriter::new(file), &audio, settings.sample_rate, WavFormat::Int24, &meta).map_err(|e| EngineError::Wav(format!("{e:?}")))?;
        out.push(Stem { track: name, path, peak_db: decibels::gain_to_db(f64::from(audio.peak())) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guqin_schedule_retains_note_techniques_and_wide_slides_on_seek() {
        let p: Project = serde_json::from_value(serde_json::json!({
            "name":"q","tempo":60,"tracks":[{"name":"q","instrument":"yinse.pluck.guqin",
            "instrument_params":{"vibrato":0},"clips":[{"start":0,"length":4,"notes":[
                {"pitch":48,"start":0,"length":3,"technique":"open"},
                {"pitch":62,"start":0,"length":3,"technique":"harmonic","harmonic":2},
                {"pitch":57,"start":0,"length":3,"technique":"pressed","bend":[[0,0],[1,12],[2,12]]}
            ]}]}]
        })).unwrap();
        let acts = schedule(&p, 0, &Ctx::new(&p, RenderSettings::default()));
        assert_eq!(acts.iter().filter(|(_, a)| matches!(a, Act::Technique { .. })).count(), 3);
        assert!(acts.iter().any(|(at, a)| *at == 48_000 && matches!(a, Act::Bend { value: 4096, .. })));
        let on = acts.iter().position(|(_, a)| matches!(a, Act::On { .. })).unwrap();
        assert!(acts[..on].iter().filter(|(_, a)| matches!(a, Act::Technique { .. })).count() == 3);
        let from_middle = chase(acts, 72_000);
        assert_eq!(from_middle.iter().filter(|(_, a)| matches!(a, Act::Technique { .. })).count(), 3);
        assert!(from_middle.iter().any(|(_, a)| matches!(a, Act::Bend { value: 4096, .. })));
    }

    #[test]
    fn strums_chords_down_on_the_beat_and_up_off_it() {
        let n = |p: u8, at: f64| muser_core::Note::new(p, at, 0.4);
        let notes = [n(40, 0.0), n(47, 0.0), n(52, 0.0), n(40, 0.5), n(47, 0.5), n(52, 0.5), n(60, 1.0), n(64, 1.0)];
        let o = strum_offsets(&notes, 0.0, 0.01);
        assert_eq!(&o[..3], &[0.0, 0.01, 0.02]);
        assert_eq!(&o[3..6], &[0.02, 0.01, 0.0]);
        assert_eq!(&o[6..], &[0.0, 0.0], "two notes are not a strummed chord");
        assert!(strum_offsets(&notes, 0.0, 0.0).iter().all(|&x| x == 0.0));
    }

    #[test]
    fn wav_marks_each_section_start() {
        let p: Project = serde_json::from_value(serde_json::json!({ "name": "cues", "tempo": 120,
            "sections": { "verse": { "bars": 1 }, "chorus": { "bars": 1 } },
            "form": ["verse", "chorus", "chorus"],
            "tracks": [{ "name": "k", "instrument": "drums.909", "parts": { "*": { "grid": { "kick": "x..." } } } }] }))
        .unwrap();
        let path = std::env::temp_dir().join(format!("muser-cues-{}.wav", std::process::id()));
        render_to_wav(&p, &path.to_string_lossy(), RenderSettings::default()).unwrap();
        let f = wav::read_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let cues: Vec<(u64, String)> = f.metadata.cue_points.iter().map(|c| (c.position, c.label.clone().unwrap_or_default())).collect();
        // 120 BPM、4/4：一小节 2 秒 = 96000 帧。
        assert_eq!(cues, vec![(0, "verse".into()), (96_000, "chorus".into()), (192_000, "chorus#2".into())]);
        assert_eq!(f.metadata.title.as_deref(), Some("cues"));
    }

    /// 段落换速度：直接换（60 BPM）、渐变（一小节里 60 → 120，按时间线性：120·4/(60+120) 秒），段落标记落在对应的秒上；
    /// MIDI 导出写出速度表（渐变每拍一个），ABC 写内联 [Q:]。
    #[test]
    fn section_tempo_changes_and_ramps() {
        let p: Project = serde_json::from_value(serde_json::json!({ "name": "tempo", "tempo": 120,
            "sections": { "a": { "bars": 1 }, "b": { "bars": 1, "tempo": 60 }, "c": { "bars": 1, "tempo": 120, "tempo_ramp_bars": 1 }, "d": { "bars": 1 } },
            "form": ["a", "b", "c", "d"],
            "tracks": [{ "name": "k", "instrument": "drums.909", "parts": { "*": { "grid": { "kick": "x..." } } } }] }))
        .unwrap();
        let path = std::env::temp_dir().join(format!("muser-tempo-{}.wav", std::process::id()));
        render_to_wav(&p, &path.to_string_lossy(), RenderSettings::default()).unwrap();
        let f = wav::read_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let cues: Vec<u64> = f.metadata.cue_points.iter().map(|c| c.position).collect();
        assert_eq!(cues, vec![0, 96_000, 288_000, 288_000 + 128_000]);
        // 每拍一下底鼓：渐变那一小节第 2 拍在 t = (−60 + √(60² + 2k·60))/k 秒（k = 60/T BPM/秒，T = 8/3 秒）。
        let k = 60.0 / (8.0 / 3.0);
        let want = 288_000.0 + (-60.0 + (3600.0f64 + 2.0 * k * 60.0).sqrt()) / k * 48_000.0;
        let ons: Vec<usize> = schedule(&crate::arrange::arranged(&p).unwrap(), 0, &Ctx::new(&p, RenderSettings::default()))
            .into_iter().filter_map(|(f, a)| matches!(a, Act::On { .. }).then_some(f)).collect();
        assert!(ons.iter().any(|&f| (f as f64 - want).abs() < 2.0), "{ons:?} vs {want}");
        let midi = crate::midi::export(&p).unwrap();
        let tempos = midi.bytes.windows(3).filter(|w| w == &[0xFF, 0x51, 0x03]).count();
        assert_eq!(tempos, 1 + 1 + 4 + 1, "开头、60、渐变 4 拍、120");
        let mut p = p;
        p.tracks.push(serde_json::from_value(serde_json::json!({ "name": "pad", "instrument": "synth.poly",
            "clips": [{ "start": 0, "length": 16, "notes": [{ "pitch": 60, "start": 0, "length": 16 }] }] })).unwrap());
        let abc = crate::abc_export::export(&p, &crate::abc_export::AbcOptions::new(crate::abc_export::Dialect::Standard)).unwrap().text;
        assert!(abc.contains("[Q:1/4=60]") && abc.contains("[Q:1/4=120]"), "{abc}");
    }

    #[test]
    fn stems_drop_master_effect_automation_with_the_master_effects() {
        let p: Project = serde_json::from_value(serde_json::json!({ "name": "s", "tempo": 120,
            "master_effects": [{ "type": "eq", "shape": "lowpass", "frequency": 18000 }],
            "automation": [{ "target": "master_effects[0]", "param": "frequency", "shape": "points", "points": [[0, 1000], [4, 18000]] }],
            "tracks": [{ "name": "k", "instrument": "drums.909", "clips": [{ "start": 0, "length": 4, "grid": { "kick": "x..." } }] }] }))
        .unwrap();
        let dir = std::env::temp_dir().join(format!("muser-stems-auto-{}", std::process::id()));
        let stems = render_stems(&p, &dir, RenderSettings::default(), &mut |_, _, _| {}).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(stems.len(), 1);
    }

    #[test]
    fn stems_sum_back_to_the_mix_without_master_effects() {
        let mut p = project();
        p.master_effects.clear();
        let dir = std::env::temp_dir().join(format!("muser-stems-{}", std::process::id()));
        let stems = render_stems(&p, &dir, RenderSettings::default(), &mut |_, _, _| {}).unwrap();
        assert_eq!(stems.len(), p.tracks.iter().filter(|t| !t.muted).count());
        let mix = render(&p, RenderSettings::default()).unwrap();
        let mut sum = vec![0.0f32; mix.num_frames() * 2];
        for s in &stems {
            let a = wav::read_file(&s.path).unwrap().buffer;
            assert_eq!(a.num_frames(), mix.num_frames(), "{}", s.track);
            for ch in 0..2 {
                for (k, x) in a.channel(ch).iter().enumerate() {
                    sum[ch * mix.num_frames() + k] += *x;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        // 16 位量化与非线性的总线效果器（limiter、侧链压缩）会差一点，整体应吻合。
        let err: f64 = (0..2).flat_map(|ch| mix.channel(ch).iter().enumerate().map(move |(k, x)| (ch, k, *x)))
            .map(|(ch, k, x)| f64::from((sum[ch * mix.num_frames() + k] - x).abs())).fold(0.0, f64::max);
        assert!(err < 0.25, "分轨相加与整首差太多：{err}");
    }

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
    /// 现场模式：安静一阵的轨和效果链先不处理，之后照常出声，和离线渲染一样（只差休息前后内部状态的极小差别）。
    #[test]
    fn live_rests_silent_tracks() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120, "tracks": [
            { "name": "keys", "instrument": "poly.wurly", "effects": [{ "type": "reverb", "mix": 0.5, "decay_seconds": 1.0 }],
              "clips": [{ "start": 0, "length": 1, "notes": [{ "pitch": 60, "start": 0, "length": 0.5 }] },
                        { "start": 16, "length": 2, "notes": [{ "pitch": 64, "start": 0, "length": 1 }] }] },
            { "name": "drums", "instrument": "drums.909", "clips": [{ "start": 0, "length": 18, "grid": { "kick": "x..." } }] } ] })).unwrap();
        let settings = RenderSettings::default();
        let want = render(&p, settings).unwrap();
        let mut r = Renderer::live(&p, settings).unwrap();
        let mut block = AudioBuffer::<f32>::new(2, 512);
        let mut got = Vec::new();
        while let n @ 1.. = r.process(&mut block).unwrap() {
            got.extend_from_slice(&block.channel(0)[..n]);
        }
        let diff = got.iter().zip(want.channel(0)).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(diff < 1e-3, "最大差 {diff}");
        // 第 16 拍（8 秒）那个音照常出来。
        let at = 8 * 48_000;
        assert!(got[at..at + 4800].iter().any(|x| x.abs() > 1e-3));
    }

    /// 现场模式：放到一半换成"往后加了东西"的工程（已有轨的新片段、追加的效果器、自动化、一条新轨），
    /// 接着放完——和离线渲染新工程逐样本一致。
    #[test]
    fn live_reconcile_matches_offline() {
        let p1 = project();
        let mut p2 = p1.clone();
        let more = |pitch: u8| serde_json::json!({ "start": 2, "length": 2, "notes": (0..8).map(|i| serde_json::json!({ "pitch": pitch + (i % 4) as u8, "start": i as f64 * 0.25, "length": 0.2 })).collect::<Vec<_>>() });
        p2.tracks[0].clips.push(serde_json::from_value(more(36)).unwrap());
        p2.tracks[1].clips.push(serde_json::from_value(more(40)).unwrap());
        p2.tracks[1].effects.push(serde_json::from_value(serde_json::json!({ "type": "eq", "shape": "lowpass", "frequency": 900, "from": 2 })).unwrap());
        p2.tracks[1].automation.push(serde_json::from_value(serde_json::json!({ "target": "effects[1]", "param": "frequency", "shape": "points", "points": [[2, 900], [4, 3000]] })).unwrap());
        p2.tracks.push(serde_json::from_value(serde_json::json!({ "name": "lead", "instrument": "synth.poly", "output": "grp",
            "clips": [more(64)] })).unwrap());
        let settings = RenderSettings::default();
        let want = render(&p2, settings).unwrap();
        let mut r = Renderer::live(&p1, settings).unwrap();
        let mut block = AudioBuffer::<f32>::new(2, 1000);
        let mut got: [Vec<f32>; 2] = Default::default();
        let mut swapped = false;
        while let n @ 1.. = r.process(&mut block).unwrap() {
            for (c, g) in got.iter_mut().enumerate() {
                g.extend_from_slice(&block.channel(c)[..n]);
            }
            if !swapped && got[0].len() >= 24_000 {
                r.reconcile(&p2).unwrap();
                swapped = true;
            }
        }
        assert_eq!(got[0].len(), want.num_frames());
        for (c, g) in got.iter().enumerate() {
            let diff = g.iter().zip(want.channel(c)).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(diff < 1e-6, "声道 {c} 最大差 {diff}");
        }
        // 不合规的改动报错，渲染器不动。
        let mut bad = p2.clone();
        bad.tracks[1].effects.remove(0);
        assert!(r.reconcile(&bad).is_err());
        let mut bad = p2.clone();
        bad.tracks[0].instrument = "drums.808".into();
        assert!(r.reconcile(&bad).is_err());
        let mut bad = p2.clone();
        bad.buses.push(serde_json::from_value(serde_json::json!({ "name": "new" })).unwrap());
        assert!(r.reconcile(&bad).is_err());
    }

    /// 演奏中换上一个第 4 拍起渐慢到 90 BPM 的工程：和离线渲染逐样本一致（之前排好的不动，之后按新的速度表排）。
    #[test]
    fn live_reconcile_with_a_tempo_ramp_matches_offline() {
        let p1 = project();
        let mut p2: Project = serde_json::from_value({
            let mut v = serde_json::to_value(&p1).unwrap();
            v["sections"] = serde_json::json!({ "a": { "bars": 1 }, "b": { "bars": 1, "tempo": 90, "tempo_ramp_bars": 1 } });
            v["form"] = serde_json::json!(["a", "b"]);
            v
        })
        .unwrap();
        p2.tracks[1].clips.push(serde_json::from_value(serde_json::json!({ "start": 4, "length": 4,
            "notes": (0..16).map(|i| serde_json::json!({ "pitch": 40 + (i % 5) as u8, "start": i as f64 * 0.25, "length": 0.2 })).collect::<Vec<_>>() })).unwrap());
        let settings = RenderSettings { tail_seconds: 0.3, ..Default::default() };
        let want = render(&p2, settings).unwrap();
        let mut r = Renderer::live(&p1, settings).unwrap();
        let mut block = AudioBuffer::<f32>::new(2, 1000);
        let mut got: [Vec<f32>; 2] = Default::default();
        let mut swapped = false;
        while let n @ 1.. = r.process(&mut block).unwrap() {
            for (c, g) in got.iter_mut().enumerate() {
                g.extend_from_slice(&block.channel(c)[..n]);
            }
            if !swapped && got[0].len() >= 24_000 {
                r.reconcile(&p2).unwrap();
                swapped = true;
            }
        }
        assert_eq!(got[0].len(), want.num_frames());
        for (c, g) in got.iter().enumerate() {
            let diff = g.iter().zip(want.channel(c)).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(diff < 1e-6, "声道 {c} 最大差 {diff}");
        }
    }

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
        let on = |key| Act::On { key, velocity: 0.8, accent: false, mute: false, ch: None };
        let acts = vec![
            (0, Act::Pedal { down: true }),
            (0, on(60)),
            (10, on(64)),
            (20, Act::Off { key: 60, ch: None }),
            (50, Act::Off { key: 64, ch: None }),
            (50, on(67)),
            (90, Act::Off { key: 67, ch: None }),
        ];
        let keys = |v: &[(usize, Act)]| v.iter().map(|(at, a)| format!("{at}:{a:?}")).collect::<Vec<_>>().join(" ");
        // 30：60 已松开，64 还按着，踏板踩着。
        let c = chase(acts.clone(), 30);
        assert_eq!(keys(&c[..2]), keys(&[(30, Act::Pedal { down: true }), (30, on(64))]));
        assert_eq!(c.len(), 2 + 3);
        // 恰好在 50 松开的 64 不补发；50 新按的 67 照常。
        let c = chase(acts.clone(), 50);
        assert_eq!(keys(&c), keys(&[(50, Act::Pedal { down: true }), (50, on(67)), (90, Act::Off { key: 67, ch: None })]));
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
