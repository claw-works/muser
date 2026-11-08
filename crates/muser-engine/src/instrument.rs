//! 乐器注册表与描述表。
//!
//! 音轨的 `instrument` 字段有三种写法：
//! - 内置乐器名，如 `"drums.808"`、`"guitar.nylon"`、`"strings.cello"`；
//! - poly 合成器预设，如 `"poly.moog_bass"`、`"poly.warm_analog_pad"`（共 68 个）；
//! - `"sfz:/path/to.sfz"` 用内置 SFZ 采样器加载音色库；
//! - `"plugin:/path/to.vst3"`（可带 `#插件id`）加载第三方 VSTi/CLAP 乐器（需 `host` 特性）。
//!
//! 内置乐器可用 `instrument_params` 在预设音色上覆盖参数（见 [`crate::params`]）。
//!
//! 每个内置乐器的名称、类别、音高约定、推荐音域、复音数、说明都写在 [`catalog`] 里，
//! CLI (`muser instruments`)、MCP (`list_instruments` / `describe_instrument`) 与
//! 校验都从这里取数据。

use manguebyte::dsp::instruments::{
    poly, AcidParams, AcidVoice, BowedParams, BowedString, ChoirParams, ChoirVoice, DrumKit,
    DrumKitVoice, FmParams, FmVoice, MalletParams, MalletVoice, OrganParams, OrganVoice,
    PluckedString, PolySynthParams, PolySynthVoice, StringParams, WindParams, WindVoice,
};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth, Voice, decibels};
use serde::Serialize;

use crate::params::{self, ParamInfo};
use muser_core::InstrumentParams;

// ---------------------------------------------------------------------------
// 统一的可演奏接口
// ---------------------------------------------------------------------------

/// 任何能接收音符并输出音频的东西。
trait Playable {
    fn prepare(&mut self, spec: &ProcessSpec);
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool);
    /// 带掌闷标记的起音（音符的 `"mute"`）。默认忽略标记（只有电吉他用）。
    fn note_on_muted(&mut self, note: NoteRef, velocity: f32, accent: bool, _mute: bool) {
        self.note_on(note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32);
    fn process(&mut self, buffer: &mut AudioBuffer<f32>);
    /// 通道消息：延音踏板（CC64）、弯音轮。默认忽略（鼓组）。
    fn midi(&mut self, _message: MidiMessage) {}
    /// 自动化：运行中改一个参数（名称与 `instrument_params` 相同，鼓组为 `鼓件.参数`）。
    fn set_param(&mut self, _name: &str, _value: f64) {}
    /// 调律（工程的 `tuning`）。默认忽略（鼓组、插件）。
    fn set_tuning(&mut self, _tuning: &manguebyte::dsp::Tuning) {}
    /// 上一块送到总线的各路直接输出（插件的 `@outputs` 写了 `bus` 的项，按顺序）。默认没有。
    fn taps(&self) -> &[AudioBuffer<f32>] {
        &[]
    }
}

fn midi_event(message: MidiMessage) -> manguebyte::prelude::Event {
    manguebyte::prelude::Event::from_midi(0, message)
}

impl<V: Voice<f32>> Playable for Synth<V, f32> {
    fn midi(&mut self, message: MidiMessage) {
        self.handle_event(&midi_event(message));
    }
    fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        Synth::note_on_accented(self, note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        Synth::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(self, buffer);
    }
}

/// 一件已就绪、可渲染的音轨乐器。
pub struct TrackInstrument {
    inner: Box<dyn Playable>,
    channel: u8,
    /// 输出端的校准增益（内置乐器，见 crate::levels）。
    trim: f32,
}

impl TrackInstrument {
    fn new(inner: impl Playable + 'static, channel: u8) -> Self {
        Self { inner: Box::new(inner), channel, trim: 1.0 }
    }

    /// 该乐器约定使用的 MIDI 通道（鼓组按惯例用 9）。
    pub fn channel(&self) -> u8 {
        self.channel
    }
    pub fn prepare(&mut self, spec: &ProcessSpec) {
        self.inner.prepare(spec);
    }
    pub fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.inner.note_on(note, velocity, accent);
    }
    /// 起音，带音符的掌闷标记（电吉他用，其它乐器忽略）。
    pub fn note_on_muted(&mut self, note: NoteRef, velocity: f32, accent: bool, mute: bool) {
        self.inner.note_on_muted(note, velocity, accent, mute);
    }
    pub fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.inner.note_off(note, velocity);
    }
    /// 延音踏板踩下/抬起（`channel` 缺省为乐器的通道）。
    pub fn pedal(&mut self, channel: Option<u8>, down: bool) {
        self.inner.midi(MidiMessage::ControlChange { channel: channel.unwrap_or(self.channel), controller: 64, value: if down { 127 } else { 0 } });
    }
    /// 弯音轮（-8192 – 8191，满偏 ±2 半音）。
    pub fn bend(&mut self, channel: Option<u8>, value: i16) {
        self.inner.midi(MidiMessage::PitchBend { channel: channel.unwrap_or(self.channel), value });
    }
    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.inner.process(buffer);
        if self.trim != 1.0 {
            for ch in 0..buffer.num_channels() {
                buffer.channel_mut(ch).iter_mut().for_each(|s| *s *= self.trim);
            }
        }
    }
    pub fn set_param(&mut self, name: &str, value: f64) {
        self.inner.set_param(name, value);
    }
    /// 调律：有音高的内置乐器与 SFZ 采样器照着它发音；鼓组、插件不变。
    pub fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.inner.set_tuning(tuning);
    }
    /// 上一次 [`process`](Self::process) 送到总线的各路直接输出（立体声，帧数与那一块相同）。
    pub fn taps(&self) -> &[AudioBuffer<f32>] {
        self.inner.taps()
    }
}

/// 可运行中调参的乐器：持有当前音色参数，改参数后推给每个 voice。
struct Tunable<V: Voice<f32>, P: Copy + 'static> {
    synth: Synth<V, f32>,
    params: P,
    defs: &'static [params::ParamDef<P>],
    push: fn(&mut V, P),
}

impl<V: Voice<f32>, P: Copy + 'static> Playable for Tunable<V, P> {
    fn midi(&mut self, message: MidiMessage) {
        self.synth.handle_event(&midi_event(message));
    }
    fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.synth, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.synth.note_on_accented(note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.synth, buffer);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(self.defs, &mut self.params, name, value) {
            let (push, p) = (self.push, self.params);
            self.synth.voices_mut().for_each(|v| push(v, p));
        }
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.synth.set_tuning(tuning);
    }
}

fn tunable<V: Voice<f32> + 'static, P: Copy + 'static>(
    synth: Synth<V, f32>,
    params: P,
    defs: &'static [params::ParamDef<P>],
    push: fn(&mut V, P),
) -> TrackInstrument {
    TrackInstrument::new(Tunable { synth, params, defs, push }, 0)
}

impl Playable for crate::electric::Electric {
    fn prepare(&mut self, spec: &ProcessSpec) {
        crate::electric::Electric::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        crate::electric::Electric::note_on(self, note, velocity, accent, false);
    }
    fn note_on_muted(&mut self, note: NoteRef, velocity: f32, accent: bool, mute: bool) {
        crate::electric::Electric::note_on(self, note, velocity, accent, mute);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        crate::electric::Electric::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        crate::electric::Electric::process(self, buffer);
    }
    fn midi(&mut self, message: MidiMessage) {
        crate::electric::Electric::midi(self, message);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        crate::electric::Electric::set_param(self, name, value);
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        crate::electric::Electric::set_tuning(self, tuning);
    }
}

/// 鼓组：参数名为 `鼓件.参数`，如 `kick.tune`；改动从下一次击打生效。
struct DrumsTunable {
    synth: Synth<DrumKitVoice, f32>,
    kit: DrumKit,
}

impl Playable for DrumsTunable {
    fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.synth, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.synth.note_on_accented(note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.synth, buffer);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        let Some((piece, field)) = name.split_once('.') else { return };
        let Some(key) = params::piece_key(piece) else { return };
        let Some(mut p) = self.kit.piece(key).copied() else { return };
        if params::set_number(params::PIECE, &mut p, field, value) {
            self.kit.set(key, Some(p));
            let kit = self.kit;
            self.synth.voices_mut().for_each(|v| v.set_params(kit));
        }
    }
}

// ---------------------------------------------------------------------------
// 描述表
// ---------------------------------------------------------------------------

/// 音符的音高约定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteConvention {
    /// 鼓组：pitch 是 GM 鼓键（36=底鼓, 38=军鼓, 42=闭镲…）。
    GmDrums,
    /// 音阶乐器：pitch 是 MIDI 音高（60=中央C）。
    Pitched,
}

/// 一件内置乐器的描述。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstrumentInfo {
    /// `instrument` 字段的值。
    pub name: String,
    /// 类别：drums / synth / poly / guitar / bass / organ / mallet / strings / wind / choir。
    pub family: String,
    /// 音高约定。
    pub notes: NoteConvention,
    /// 推荐音域（MIDI 音高，含边界）。超出也能发声，但可能不自然。
    pub range: (u8, u8),
    /// 最大同时发声数。1 表示单音（新音符会接管旧音符）。
    pub polyphony: usize,
    /// 音色与用途说明。
    pub description: String,
    /// 可调参数（`instrument_params` 的键）及该乐器当前取值。列表接口里省略，详情接口里给出。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamInfo>,
    /// 补充说明（如鼓组按鼓件设置参数的写法、支持的鼓键）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub params_note: String,
}

/// 一件乐器的音色：manguebyte 的参数结构 + 复音数。先应用 `instrument_params`，再实例化。
#[derive(Clone)]
enum Patch {
    Drums(DrumKit),
    Poly(PolySynthParams),
    Fm(FmParams),
    String(StringParams, usize),
    Electric(crate::electric::ElectricParams, usize),
    Acid(AcidParams),
    Organ(OrganParams),
    Mallet(MalletParams, usize),
    Bowed(BowedParams, usize),
    Wind(WindParams, usize),
    Choir(ChoirParams, usize),
    Sfz(std::path::PathBuf, crate::sfz::Options),
}

impl Patch {
    /// 在预设音色上覆盖参数。错误信息以参数名（鼓组为鼓件名）开头。
    fn apply(&mut self, params: &InstrumentParams) -> Result<(), String> {
        if params.is_empty() {
            return Ok(());
        }
        match self {
            Patch::Poly(p) => {
                // `patch`：完整音色（见 poly_patch），先套上，再套常用参数；`sound`：音色来源，只作标注。
                let mut rest = params.clone();
                rest.remove("sound");
                if let Some(patch) = rest.remove("patch") {
                    crate::poly_patch::apply(p, &patch)?;
                }
                params::apply(params::POLY, p, &rest)
            }
            Patch::Fm(p) => params::apply(params::FM, p, params),
            Patch::String(p, _) => params::apply(params::STRING, p, params),
            Patch::Electric(p, _) => params::apply(params::ELECTRIC, p, params),
            Patch::Acid(p) => params::apply(params::ACID, p, params),
            Patch::Organ(p) => params::apply(params::ORGAN, p, params),
            Patch::Mallet(p, _) => params::apply(params::MALLET, p, params),
            Patch::Bowed(p, _) => params::apply(params::BOWED, p, params),
            Patch::Wind(p, _) => params::apply(params::WIND, p, params),
            Patch::Choir(p, _) => params::apply(params::CHOIR, p, params),
            Patch::Drums(kit) => {
                for (piece, value) in params {
                    let key = params::piece_key(piece).ok_or_else(|| {
                        format!(
                            "unknown drum piece '{piece}': use a GM drum key number or an alias ({})",
                            params::PIECE_ALIASES.iter().map(|(a, _)| *a).collect::<Vec<_>>().join(" ")
                        )
                    })?;
                    let mut p = *kit.piece(key).ok_or_else(|| {
                        format!("this kit has no drum key {key} ({piece}), supported: {}", kit_keys(kit))
                    })?;
                    let obj = value
                        .as_object()
                        .ok_or_else(|| format!("{piece}: expected an object, e.g. {{ \"tune\": -2 }}"))?;
                    params::apply(params::PIECE, &mut p, obj).map_err(|e| format!("{piece}.{e}"))?;
                    kit.set(key, Some(p));
                }
                Ok(())
            }
            Patch::Sfz(_, cc) => {
                *cc = crate::sfz::parse_params(params)?;
                Ok(())
            }
        }
    }

    /// 参数规格 + 当前值。
    fn param_info(&self) -> (Vec<ParamInfo>, String) {
        match self {
            Patch::Poly(p) => (
                params::describe(params::POLY, p, &[]),
                "A full sound (oscillators, filters, envelopes, LFOs, modulation matrix) is written with instrument_params \"patch\"; see `muser poly schema` for the structure; \
                 get the full JSON of a sound in the local sound library with `muser poly show <name>`.".into(),
            ),
            Patch::Fm(p) => (params::describe(params::FM, p, params::NEXT_FM), String::new()),
            Patch::String(p, _) => (params::describe(params::STRING, p, params::NEXT_STRING), String::new()),
            Patch::Electric(p, _) => (
                params::describe(params::ELECTRIC, p, params::NEXT_ELECTRIC),
                "Palm-mute single notes with `\"mute\": true` on the note (depth: `mute`); bends with the note's `bend`. Chords of 3+ notes are strummed (`strum`). \
                 Made for amps: put a `distortion` with `\"cabinet\": \"guitar\"` (or a guitar amp plugin) after it for rock tones.".into(),
            ),
            Patch::Acid(p) => (
                params::describe(params::ACID, p, &[]),
                "Monophonic instrument: when the next note starts before the previous one ends, it glides over in the slide time".into(),
            ),
            Patch::Organ(p) => (params::describe(params::ORGAN, p, &[]), String::new()),
            Patch::Mallet(p, _) => (params::describe(params::MALLET, p, params::NEXT_MALLET), String::new()),
            Patch::Bowed(p, _) => (params::describe(params::BOWED, p, params::NEXT_BOWED), String::new()),
            Patch::Wind(p, _) => (params::describe(params::WIND, p, &[]), String::new()),
            Patch::Choir(p, _) => (params::describe(params::CHOIR, p, &[]), String::new()),
            Patch::Drums(kit) => (
                params::specs(params::PIECE, params::NEXT_PIECE),
                format!(
                    "Set per drum piece: {{ \"kick\": {{ \"tune\": -2, \"decay\": 1.5 }}, \"38\": {{ \"snap\": 0.8 }} }}. \
                     Keys can be a GM drum key number or an alias (kick snare clap closed_hat open_hat crash ride tom_low tom_mid tom_high ...). \
                     Drum keys supported by this kit: {}",
                    kit_keys(kit)
                ),
            ),
            Patch::Sfz(path, _) => (Vec::new(), sfz_controls_note(path)),
        }
    }

    /// 生成可演奏的乐器。
    fn instantiate(self) -> Result<TrackInstrument, String> {
        Ok(match self {
            Patch::Drums(kit) => TrackInstrument::new(
                DrumsTunable { synth: Synth::new(DrumKitVoice::voices(24, kit)), kit },
                9,
            ),
            Patch::Poly(p) => tunable(Synth::with_voices(16, move || PolySynthVoice::new(p)), p, params::POLY, |v, p| v.set_params(p)),
            Patch::Fm(p) => tunable(Synth::with_voices(16, move || FmVoice::new(p)), p, params::FM, |v, p| v.set_params(p)),
            Patch::String(p, n) => tunable(Synth::with_voices(n, move || PluckedString::new(p)), p, params::STRING, |v, p| v.set_params(p)),
            Patch::Electric(p, n) => TrackInstrument::new(crate::electric::Electric::new(p, n), 0),
            Patch::Acid(p) => {
                let mut s = Synth::with_voices(1, move || AcidVoice::new(p));
                s.set_legato(true);
                tunable(s, p, params::ACID, |v, p| v.set_params(p))
            }
            Patch::Organ(p) => tunable(Synth::new(OrganVoice::voices(16, p)), p, params::ORGAN, |v, p| v.set_params(p)),
            Patch::Mallet(p, n) => tunable(Synth::with_voices(n, move || MalletVoice::new(p)), p, params::MALLET, |v, p| v.set_params(p)),
            Patch::Bowed(p, n) => tunable(Synth::with_voices(n, move || BowedString::new(p)), p, params::BOWED, |v, p| v.set_params(p)),
            Patch::Wind(p, n) => tunable(Synth::with_voices(n, move || WindVoice::new(p)), p, params::WIND, |v, p| v.set_params(p)),
            Patch::Choir(p, n) => tunable(Synth::with_voices(n, move || ChoirVoice::new(p)), p, params::CHOIR, |v, p| v.set_params(p)),
            Patch::Sfz(path, cc) => sfz_instrument(&path, &cc)?,
        })
    }
}

/// 鼓组乐器（`drums.*`）有哪些鼓键；不是鼓组时为 `None`。
pub fn drum_kit_keys(name: &str) -> Option<Vec<u8>> {
    match lookup(name)?.1 {
        Patch::Drums(kit) => Some(kit.keys().collect()),
        _ => None,
    }
}

fn kit_keys(kit: &DrumKit) -> String {
    kit.keys().map(|k| k.to_string()).collect::<Vec<_>>().join(",")
}

/// SFZ 采样器乐器（内置，manguebyte-sampler）。
/// `muser instruments sfz:…` 的说明：列出 .sfz 声明的控制器。
fn sfz_controls_note(path: &std::path::Path) -> String {
    let base = "The sound is defined by the .sfz file. Set controller values with instrument_params \"cc\" (applied at load), e.g. { \"cc\": { \"105\": 127 } }.";
    let mut out = base.to_string();
    if let Ok(c) = crate::sfz::read_controls(path) {
        if !c.is_empty() {
            let list: Vec<String> = c.iter().map(|(n, (label, v))| format!("cc{n}{} = {v}", if label.is_empty() { String::new() } else { format!(" ({label})") })).collect();
            out.push_str(&format!("This sound declares these controllers (default values): {}.", list.join(", ")));
        }
    }
    if let Ok((sw, def)) = crate::sfz::read_keyswitches(path) {
        if !sw.is_empty() {
            let list: Vec<String> = sw.iter().map(|(n, l)| if l.is_empty() { n.to_string() } else { format!("{n} ({l})") }).collect();
            out.push_str(&format!(
                "Keyswitch articulations (choose with instrument_params \"keyswitch\", default {}): {}.",
                def.map_or("the first".to_string(), |d| d.to_string()),
                list.join(", ")
            ));
        }
    }
    out
}

fn sfz_instrument(path: &std::path::Path, cc: &crate::sfz::Options) -> Result<TrackInstrument, String> {
    use manguebyte::sampler::Sampler;
    let loaded = crate::sfz::load_shared(path, cc)?;
    // 刚加载（不是共用已有的）时提示一次。
    if !loaded.warnings.is_empty() && std::sync::Arc::strong_count(&loaded) == 1 {
        eprintln!(
            "[muser] sfz '{}': opcodes not supported by the sampler (ignored): {}. The sound may differ slightly from the original design",
            path.display(),
            crate::sfz::summary(&loaded.warnings)
        );
    }
    let (mut sampler, loader) = Sampler::with_instrument(loaded.instrument.clone(), SFZ_VOICES);
    // 工程里给的控制器值：上游采样器按它实时调制（`<opcode>_onccN`）。
    for (&n, &v) in &loaded.cc {
        sampler.set_controller(u16::from(n), f32::from(v) / 127.0, 0);
    }
    Ok(TrackInstrument::new(SfzSampler { sampler, _loader: loader, _shared: loaded }, 0))
}

/// 完整的 SFZ 采样器：自己管理复音、按住的键与踏板，所以 `trigger=first` / `legato`（连奏：
/// 前一个键还按着时换用连奏采样）、`release` 等触发方式按 .sfz 的设计工作。
struct SfzSampler {
    sampler: manguebyte::sampler::Sampler<f32>,
    /// 采样器的加载端，需与采样器同生命周期。
    _loader: manguebyte::sampler::SamplerLoader,
    /// 共用的音色：用着就留在池里。
    _shared: std::sync::Arc<crate::sfz::Loaded>,
}

impl Playable for SfzSampler {
    fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.sampler, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, _accent: bool) {
        self.sampler.note_on(note, velocity);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.sampler.note_off(note, velocity);
    }
    fn midi(&mut self, message: MidiMessage) {
        self.sampler.handle_event(&midi_event(message));
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        // 流式读的采样：先等磁盘读够（渲染比实时快；不等就会断音），结果与整个读进内存一样。
        // 渲染与播放都在自己的线程里算，不在声卡回调里，可以等。
        self._shared.instrument.wait_for_streams(std::time::Duration::from_secs(10));
        Processor::process(&mut self.sampler, buffer);
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.sampler.set_tuning(tuning);
    }
}

/// 同时发声的区域数（一个音可能叠几层区域，连奏时前一个音还在淡出）。
const SFZ_VOICES: usize = 64;

/// 内置（非 poly 预设）乐器条目。
struct Builtin {
    name: &'static str,
    aliases: &'static [&'static str],
    family: &'static str,
    drums: bool,
    range: (u8, u8),
    polyphony: usize,
    description: &'static str,
    patch: fn() -> Patch,
}

const DRUM_RANGE: (u8, u8) = (35, 81);

macro_rules! b {
    ($name:expr, [$($alias:expr),*], $family:expr, $drums:expr, $range:expr, $poly:expr, $desc:expr, $patch:expr) => {
        Builtin {
            name: $name,
            aliases: &[$($alias),*],
            family: $family,
            drums: $drums,
            range: $range,
            polyphony: $poly,
            description: $desc,
            patch: $patch,
        }
    };
}

static BUILTINS: &[Builtin] = &[
    // —— 鼓组 ——
    b!("drums.808", ["808"], "drums", true, DRUM_RANGE, 24, "TR-808 style: long-tail low kick, crisp clap; hip-hop / trap / electronic", || Patch::Drums(DrumKit::style_808())),
    b!("drums.909", ["909"], "drums", true, DRUM_RANGE, 24, "TR-909 style: punchy kick and cymbals; house / techno", || Patch::Drums(DrumKit::style_909())),
    b!("drums.606", ["606"], "drums", true, DRUM_RANGE, 24, "TR-606 style: light and crisp; acid / electro", || Patch::Drums(DrumKit::style_606())),
    b!("drums.cr78", ["cr78"], "drums", true, DRUM_RANGE, 24, "CR-78 style: soft and vintage, with shaker, cowbell and bongos", || Patch::Drums(DrumKit::style_cr78())),
    b!("drums.acoustic", ["acoustic"], "drums", true, DRUM_RANGE, 24, "Acoustic drum kit (physical modeling); pop / rock / jazz", || Patch::Drums(DrumKit::acoustic())),
    b!("drums.trap", ["trap"], "drums", true, DRUM_RANGE, 24, "trap kit: heavy kick, crisp snare and clap; suited to rapid closed-hat rolls", || Patch::Drums(DrumKit::trap())),
    b!("drums.techno", ["techno"], "drums", true, DRUM_RANGE, 24, "techno kit: kick with a rumbling tail, hard clap", || Patch::Drums(DrumKit::techno())),
    b!("drums.house", ["house"], "drums", true, DRUM_RANGE, 24, "house kit: four-on-the-floor kick, clap, shaker, congas", || Patch::Drums(DrumKit::house())),
    b!("drums.dnb", ["dnb", "drums.drum_and_bass"], "drums", true, DRUM_RANGE, 24, "drum & bass kit: tight snare and deep kick", || Patch::Drums(DrumKit::drum_and_bass())),
    b!("drums.lofi", ["lofi", "drums.lo_fi"], "drums", true, DRUM_RANGE, 24, "lo-fi kit: soft, dark, with a 12-bit sampled texture", || Patch::Drums(DrumKit::lo_fi())),
    // —— 通用合成器 ——
    b!("synth.poly", ["poly", "synth"], "synth", false, (36, 96), 16, "Initial sound of the polyphonic synth (fairly plain). For a specific sound use the poly.* presets", || Patch::Poly(PolySynthParams::default())),
    b!("synth.fm", ["fm"], "synth", false, (24, 96), 16, "Default sound of the four-operator FM synth; electronic and metallic, can be used for bass", || Patch::Fm(FmParams::default())),
    // —— 吉他 / 贝斯（物理建模拨弦）——
    b!("guitar.nylon", [], "guitar", false, (40, 84), 8, "Nylon-string classical guitar: warm and soft; suited to arpeggios, Latin, folk", || Patch::String(StringParams::nylon_guitar(), 8)),
    b!("guitar.steel", [], "guitar", false, (40, 84), 8, "Steel-string folk guitar: bright and crisp; suited to strumming and arpeggios", || Patch::String(StringParams::steel_guitar(), 8)),
    b!("guitar.electric", ["electric"], "guitar", false, (40, 88), 8, "Electric guitar, humbucker on the bridge pickup: long sustain, thick and biting; made for distortion (rock / metal riffs, power chords, leads). Palm mute per note", || Patch::Electric(crate::electric::ElectricParams::humbucker_bridge(), 8)),
    b!("guitar.electric_clean", [], "guitar", false, (40, 88), 8, "Electric guitar, single coil on the neck pickup: clean, glassy and warm; funk, pop, blues and jazz comping, arpeggios", || Patch::Electric(crate::electric::ElectricParams::single_coil_neck(), 8)),
    b!("bass.finger", [], "bass", false, (28, 60), 4, "Fingerstyle electric bass: round and full; the standard bass for pop / funk / R&B", || Patch::String(StringParams::finger_bass(), 4)),
    b!("bass.slap", [], "bass", false, (28, 60), 4, "Slap bass: percussive and punchy; funk / disco", || Patch::String(StringParams::slap_bass(), 4)),
    b!("bass.acid", ["acid"], "bass", false, (24, 60), 1, "TB-303 style acid bass (monophonic, overlapping notes glide); acid house / techno", || Patch::Acid(AcidParams::classic())),
    b!("bass.acid_squelch", [], "bass", false, (24, 60), 1, "acid bass with higher resonance and envelope, a stronger squelch", || Patch::Acid(AcidParams::squelch())),
    b!("bass.acid_square", [], "bass", false, (24, 60), 1, "acid bass, square-wave version, more hollow", || Patch::Acid(AcidParams::square())),
    // —— 风琴 ——
    b!("organ.jazz", [], "organ", false, (36, 96), 16, "Drawbar organ (Hammond style) jazz sound: soft, with percussion overtones", || Patch::Organ(OrganParams::jazz())),
    b!("organ.gospel", [], "organ", false, (36, 96), 16, "Gospel organ: full and bright; suited to chord beds", || Patch::Organ(OrganParams::gospel())),
    b!("organ.full", [], "organ", false, (36, 96), 16, "Full-drawbar organ: the fullest and loudest, rock organ", || Patch::Organ(OrganParams::full())),
    b!("organ.ballad", [], "organ", false, (36, 96), 16, "Ballad organ: soft and round; suited to slow-song beds", || Patch::Organ(OrganParams::ballad())),
    // —— 打击旋律乐器（模态合成）——
    b!("mallet.marimba", ["marimba"], "mallet", false, (45, 96), 12, "Marimba: warm and woody; suited to melodies and arpeggios", || Patch::Mallet(MalletParams::marimba(), 12)),
    b!("mallet.xylophone", ["xylophone"], "mallet", false, (65, 108), 12, "Xylophone: bright and short", || Patch::Mallet(MalletParams::xylophone(), 12)),
    b!("mallet.vibraphone", ["vibraphone"], "mallet", false, (53, 89), 12, "Vibraphone: metallic with a long tail; common in jazz and lo-fi", || Patch::Mallet(MalletParams::vibraphone(), 12)),
    b!("mallet.glockenspiel", ["glockenspiel"], "mallet", false, (79, 108), 12, "Glockenspiel: high, clear bell tones", || Patch::Mallet(MalletParams::glockenspiel(), 12)),
    b!("mallet.celesta", ["celesta"], "mallet", false, (60, 108), 12, "Celesta: soft bell tones, dreamy", || Patch::Mallet(MalletParams::celesta(), 12)),
    b!("mallet.kalimba", ["kalimba"], "mallet", false, (60, 88), 12, "Kalimba (thumb piano): crisp and gentle", || Patch::Mallet(MalletParams::kalimba(), 12)),
    b!("mallet.steel_pan", ["steel_pan"], "mallet", false, (60, 84), 12, "Steel pan: metallic Caribbean sound", || Patch::Mallet(MalletParams::steel_pan(), 12)),
    b!("mallet.tubular_bells", ["tubular_bells"], "mallet", false, (60, 77), 8, "Tubular bells: solemn bell tones with a long tail", || Patch::Mallet(MalletParams::tubular_bells(), 8)),
    // —— 弦乐（弓弦波导模型）——
    b!("strings.violin", ["violin"], "strings", false, (55, 100), 8, "Violin (bowed): melody lead", || Patch::Bowed(BowedParams::violin(), 8)),
    b!("strings.viola", ["viola"], "strings", false, (48, 88), 8, "Viola (bowed): darker and thicker than the violin", || Patch::Bowed(BowedParams::viola(), 8)),
    b!("strings.cello", ["cello"], "strings", false, (36, 76), 8, "Cello (bowed): warm and deep", || Patch::Bowed(BowedParams::cello(), 8)),
    b!("strings.contrabass", ["contrabass"], "strings", false, (28, 60), 8, "Double bass (bowed): the string bass voice", || Patch::Bowed(BowedParams::contrabass(), 8)),
    b!("strings.ensemble", ["strings"], "strings", false, (36, 96), 8, "String ensemble: many players together; suited to chord beds and lyrical passages", || Patch::Bowed(BowedParams::string_ensemble(), 8)),
    b!("strings.violin_pizz", [], "strings", false, (55, 100), 8, "Violin pizzicato: short and bouncy", || Patch::Bowed(BowedParams::violin_pizzicato(), 8)),
    b!("strings.cello_pizz", [], "strings", false, (36, 76), 8, "Cello pizzicato", || Patch::Bowed(BowedParams::cello_pizzicato(), 8)),
    b!("strings.contrabass_pizz", [], "strings", false, (28, 60), 8, "Double bass pizzicato: jazz walking bass", || Patch::Bowed(BowedParams::contrabass_pizzicato(), 8)),
    // —— 管乐（波导模型）——
    b!("wind.flute", ["flute"], "wind", false, (60, 96), 4, "Flute: light and clear", || Patch::Wind(WindParams::flute(), 4)),
    b!("wind.clarinet", ["clarinet"], "wind", false, (50, 91), 4, "Clarinet: round and woody", || Patch::Wind(WindParams::clarinet(), 4)),
    b!("wind.sax", ["sax", "wind.saxophone"], "wind", false, (49, 81), 4, "Saxophone: lead for jazz / pop", || Patch::Wind(WindParams::saxophone(), 4)),
    b!("wind.trumpet", ["trumpet"], "wind", false, (54, 82), 4, "Trumpet: bright and piercing", || Patch::Wind(WindParams::trumpet(), 4)),
    b!("wind.trombone", ["trombone"], "wind", false, (40, 72), 4, "Trombone: rich low-mid brass", || Patch::Wind(WindParams::trombone(), 4)),
    // —— 人声（共振峰合成）——
    b!("choir.aah", ["choir"], "choir", false, (48, 79), 8, "Choir 'aah': open and bright; suited to chord beds", || Patch::Choir(ChoirParams::aah_choir(), 8)),
    b!("choir.ooh", [], "choir", false, (48, 79), 8, "Choir 'ooh': soft and dark", || Patch::Choir(ChoirParams::ooh_choir(), 8)),
    b!("choir.solo", ["voice"], "choir", false, (48, 79), 2, "Solo voice (vowel)", || Patch::Choir(ChoirParams::solo_voice(), 2)),
];

/// 把 poly 预设名转成 `poly.` 后缀：小写，非字母数字变 `_`。
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// poly 预设类别的推荐音域。
fn poly_range(category: poly::Category) -> (u8, u8) {
    use poly::Category as C;
    match category {
        C::Bass => (24, 60),
        C::Lead => (48, 96),
        C::Pad => (36, 84),
        C::Pluck => (48, 96),
        C::Keys => (36, 96),
        C::Brass => (40, 84),
        C::Fx => (36, 84),
    }
}

/// 按名称找到内置乐器或 poly 预设的音色（不实例化）。`sfz:` 也在这里处理。
fn lookup(name: &str) -> Option<(InstrumentInfo, Patch)> {
    let trimmed = name.trim();
    if let Some(path) = trimmed.strip_prefix("sfz:") {
        let path = crate::sfz::resolve(path.trim());
        return Some((
            InstrumentInfo {
                name: trimmed.to_string(),
                family: "sfz".into(),
                notes: NoteConvention::Pitched,
                range: (0, 127),
                polyphony: SFZ_VOICES,
                description: format!("SFZ sampler: {}", path.display()),
                params: Vec::new(),
                // 说明要解析整个 SFZ 文件（大音色库要几百毫秒）：只在 describe 里生成（param_info），校验、建乐器时不要。
                params_note: Default::default(),
            },
            Patch::Sfz(path, Default::default()),
        ));
    }
    let key = trimmed.to_ascii_lowercase();
    if let Some(b) = BUILTINS.iter().find(|b| b.name == key || b.aliases.contains(&key.as_str())) {
        return Some((builtin_info(b), (b.patch)()));
    }
    let suffix = key.strip_prefix("poly.")?;
    let preset = poly::PRESETS.iter().find(|p| slug(p.name) == suffix)?;
    Some((preset_info(preset), Patch::Poly(preset.params)))
}

fn builtin_info(b: &Builtin) -> InstrumentInfo {
    InstrumentInfo {
        name: b.name.to_string(),
        family: b.family.to_string(),
        notes: if b.drums { NoteConvention::GmDrums } else { NoteConvention::Pitched },
        range: b.range,
        polyphony: b.polyphony,
        description: b.description.to_string(),
        params: Vec::new(),
        params_note: String::new(),
    }
}

fn preset_info(preset: &poly::Preset) -> InstrumentInfo {
    InstrumentInfo {
        name: format!("poly.{}", slug(preset.name)),
        family: format!("poly/{}", preset.category.name()),
        notes: NoteConvention::Pitched,
        range: poly_range(preset.category),
        polyphony: 16,
        description: format!("{}：{}", preset.name, preset.description),
        params: Vec::new(),
        params_note: String::new(),
    }
}

/// 所有内置乐器（含 68 个 poly 预设）的描述，不含参数详情。
pub fn catalog() -> Vec<InstrumentInfo> {
    BUILTINS
        .iter()
        .map(builtin_info)
        .chain(poly::PRESETS.iter().map(preset_info))
        .collect()
}

/// 按名称查描述（大小写不敏感，支持别名），含可调参数及当前取值。插件乐器返回 `None`。
pub fn describe(name: &str) -> Option<InstrumentInfo> {
    let (mut info, patch) = lookup(name)?;
    let (params, note) = patch.param_info();
    info.params = params;
    info.params_note = note;
    Some(info)
}

/// 所有可用的内置乐器名（用于错误提示与文档）。另有 `plugin:路径`、`sfz:路径`。
pub fn available() -> Vec<String> {
    catalog().into_iter().map(|i| i.name).collect()
}

/// 不实例化乐器、不加载插件或采样，检查乐器名与 `instrument_params`。
/// `plugin:` 只检查语法；`sfz:` 检查文件存在。
/// `@label`（给人和 agent 看的音色名，见 muser view 的音色面板）对任何乐器都只是注释。
/// 去掉 muser 自己用、乐器不认识的键：`@label`（显示名）、`@consonants`（唱音节时辅音的音量，见 lyric.rs）。
fn without_label(params: &InstrumentParams) -> std::borrow::Cow<'_, InstrumentParams> {
    const OURS: [&str; 2] = ["@label", "@consonants"];
    if OURS.iter().any(|k| params.contains_key(*k)) {
        let mut p = params.clone();
        OURS.iter().for_each(|k| {
            p.remove(*k);
        });
        std::borrow::Cow::Owned(p)
    } else {
        std::borrow::Cow::Borrowed(params)
    }
}

pub fn check(name: &str, params: &InstrumentParams) -> Result<(), InstrumentError> {
    let params = &*without_label(params);
    let trimmed = name.trim();
    // 音频音轨：没有乐器，声音来自 clip 上的音频文件（crate::audio）。
    if trimmed == muser_core::audio::INSTRUMENT {
        return match params.is_empty() {
            true => Ok(()),
            false => Err(InstrumentError::Param { detail: "an audio track has no instrument parameters (set volume with gain_db or the clip's audio.gain_db)".into() }),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        if rest.trim().is_empty() {
            return Err(InstrumentError::PluginLoad { path: String::new(), detail: "missing plugin path after plugin:".into() });
        }
        let (path, id) = crate::plugin::split_spec(rest);
        return crate::plugin::check_params(path, id, params).map_err(|detail| InstrumentError::Param { detail });
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })?;
    if let Patch::Sfz(path, _) = &patch {
        if !path.is_file() {
            let hint = if path.is_relative() { "(relative paths are searched in the sound library directories of MUSER_SFZ_PATH and two levels of subdirectories)" } else { "" };
            return Err(InstrumentError::Load { detail: format!("cannot find SFZ file '{}'{hint}", path.display()) });
        }
    }
    patch.apply(params).map_err(|detail| InstrumentError::Param { detail })
}

/// 自动化目标参数的规格、行为与静态值（已应用 `instrument_params`）。
pub(crate) fn automation_param(
    name: &str,
    instrument_params: &InstrumentParams,
    param: &str,
) -> Result<(params::InstParamSpec, params::Automatable, f64), String> {
    let trimmed = name.trim();
    if trimmed == muser_core::audio::INSTRUMENT {
        return Err("an audio track has no instrument parameters to automate (use target gain_db for volume)".into());
    }
    if trimmed.starts_with("sfz:") {
        return Err("parameter automation for sfz instruments is not supported yet".into());
    }
    if trimmed.starts_with("plugin:") {
        return Err("internal error: plugin parameter automation must go through automate::plugin_rule".into());
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| format!("unknown instrument '{name}'"))?;
    patch.apply(instrument_params)?;

    fn find<P>(
        defs: &[params::ParamDef<P>],
        p: &P,
        param: &str,
        next: &[&str],
    ) -> Result<(params::InstParamSpec, params::Automatable, f64), String> {
        let list = || {
            defs.iter()
                .filter(|d| params::automatable(&d.spec, next) != params::Automatable::No)
                .map(|d| d.spec.name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let d = defs
            .iter()
            .find(|d| d.spec.name == param)
            .ok_or_else(|| format!("unknown parameter '{param}', automatable: {}", list()))?;
        let a = params::automatable(&d.spec, next);
        if a == params::Automatable::No {
            return Err(format!("parameter '{param}' cannot be automated (switches, choices, integers); automatable: {}", list()));
        }
        let base = (d.get)(p)
            .as_f64()
            .ok_or_else(|| format!("parameter '{param}' has no effect on this sound (no filter enabled)"))?;
        Ok((d.spec, a, base))
    }

    match &patch {
        Patch::Poly(p) => find(params::POLY, p, param, &[]),
        Patch::Fm(p) => find(params::FM, p, param, params::NEXT_FM),
        Patch::String(p, _) => find(params::STRING, p, param, params::NEXT_STRING),
        Patch::Electric(p, _) => find(params::ELECTRIC, p, param, params::NEXT_ELECTRIC),
        Patch::Acid(p) => find(params::ACID, p, param, &[]),
        Patch::Organ(p) => find(params::ORGAN, p, param, &[]),
        Patch::Mallet(p, _) => find(params::MALLET, p, param, params::NEXT_MALLET),
        Patch::Bowed(p, _) => find(params::BOWED, p, param, params::NEXT_BOWED),
        Patch::Wind(p, _) => find(params::WIND, p, param, &[]),
        Patch::Choir(p, _) => find(params::CHOIR, p, param, &[]),
        Patch::Drums(kit) => {
            let (piece, field) = param
                .split_once('.')
                .ok_or_else(|| format!("drum kit parameters are written as `piece.param`, e.g. \"kick.tune\", got '{param}'"))?;
            let key = params::piece_key(piece).ok_or_else(|| format!("unknown drum piece '{piece}'"))?;
            let p = kit.piece(key).ok_or_else(|| format!("this kit has no drum key {key} ({piece})"))?;
            find(params::PIECE, p, field, params::NEXT_PIECE)
        }
        Patch::Sfz(..) => Err("parameter automation for sfz instruments is not supported yet".into()),
    }
}

/// 兼容旧接口：只检查名称。
pub fn check_name(name: &str) -> Result<(), InstrumentError> {
    check(name, &InstrumentParams::new())
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------


/// 解析失败。
#[derive(Debug, thiserror::Error)]
pub enum InstrumentError {
    #[error("unknown instrument '{name}'. See all with `muser instruments`; common: drums.808 / drums.acoustic / synth.poly / poly.moog_bass / guitar.nylon / strings.ensemble / mallet.marimba; also sfz:/path/to.sfz")]
    Unknown { name: String },
    #[error("invalid instrument_params: {detail} (for built-in instruments see `muser instruments <name>`, for plugins `muser plugin params <plugin>` to list tunable parameters)")]
    Param { detail: String },
    #[error("{detail}")]
    Load { detail: String },
    #[error("plugin instrument unavailable: this muser build has no plugin host enabled (the `host` feature); release and default builds include it, so do not remove it with --no-default-features")]
    HostDisabled,
    #[error("failed to load plugin instrument '{path}': {detail}")]
    PluginLoad { path: String, detail: String },
}

/// 兼容旧名。
pub type UnknownInstrument = InstrumentError;

/// 把乐器名 + 参数解析为就绪的乐器（插件按 48 kHz / 512 帧激活）。大小写不敏感。
pub fn resolve_with(name: &str, params: &InstrumentParams) -> Result<TrackInstrument, InstrumentError> {
    resolve_for(name, params, &ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 2 })
}

/// 同 [`resolve_with`]，插件按给定的采样率与块大小激活。
pub fn resolve_for(name: &str, params: &InstrumentParams, spec: &ProcessSpec) -> Result<TrackInstrument, InstrumentError> {
    let params = &*without_label(params);
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        return resolve_plugin(rest, params, spec);
    }
    let (info, mut patch) = lookup(trimmed).ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })?;
    patch.apply(params).map_err(|detail| InstrumentError::Param { detail })?;
    let trim = trim_of(&info, &patch, params);
    let mut inst = patch.instantiate().map_err(|detail| InstrumentError::Load { detail })?;
    inst.trim = decibels::db_to_gain(trim) as f32;
    Ok(inst)
}

/// 先把工程里 `sfz:` 音色的采样加载进进程内的共享池（[`crate::sfz::load_shared`]），在调用的线程里做。
/// 现场演奏在播放线程之外调它、拿着返回值：播放线程换上新工程时直接取用，不在那里读几秒钟的采样（Salamander 约 9 秒）。
pub fn preload_sfz(project: &muser_core::Project) -> Result<Vec<std::sync::Arc<crate::sfz::Loaded>>, String> {
    let mut out = Vec::new();
    for t in &project.tracks {
        if !t.instrument.trim().starts_with("sfz:") {
            continue;
        }
        let Some((_, mut patch)) = lookup(&t.instrument) else { continue };
        patch.apply(&without_label(&t.instrument_params)).map_err(|e| format!("track '{}': {e}", t.name))?;
        if let Patch::Sfz(path, cc) = &patch {
            out.push(crate::sfz::load_shared(path, cc).map_err(|e| format!("track '{}': {e}", t.name))?);
        }
    }
    Ok(out)
}

/// 这件内置乐器在输出端补的校准增益（dB）；SFZ、插件、音频轨、不认识的乐器为 0。`muser migrate` 用它把旧工程的 `gain_db` 换算过来。
pub fn calibration_db(name: &str, params: &InstrumentParams) -> f64 {
    let params = &*without_label(params);
    let Some((info, mut patch)) = lookup(name.trim()) else { return 0.0 };
    if patch.apply(params).is_err() {
        return 0.0;
    }
    trim_of(&info, &patch, params)
}

/// 校准跟着乐器名（调了参数的预设仍按预设校准）；`synth.poly` 内嵌完整音色（`patch`）时，和某个预设一样就用那个预设的
/// 校准（内嵌预设 = 预设本身），自己设计的音色不校准（音色库里存的音色照旧）。
fn trim_of(info: &InstrumentInfo, patch: &Patch, params: &InstrumentParams) -> f64 {
    let level_name = match patch {
        Patch::Poly(p) if info.name == "synth.poly" && params.contains_key("patch") => {
            poly::PRESETS.iter().find(|x| x.params == *p).map(|x| format!("poly.{}", slug(x.name)))
        }
        _ => Some(info.name.clone()),
    };
    level_name.map_or(0.0, |n| crate::levels::trim_db(&n))
}

/// 用乐器自身的默认音色解析。
pub fn resolve(name: &str) -> Result<TrackInstrument, InstrumentError> {
    resolve_with(name, &InstrumentParams::new())
}

// ---------------------------------------------------------------------------
// 插件乐器（host 特性）
// ---------------------------------------------------------------------------

/// 第三方插件乐器：持有宿主激活的 processor，把音符累积成事件，
/// 在每个 `process` 块通过 `process_with` 喂给插件。
///
/// 渲染循环会在每个子块开始前触发到点的 note_on/note_off，再 `process`；
/// 由于子块恰好在事件处切分，这里把事件记为 offset 0 即可做到样本精确。
#[cfg(feature = "host")]
struct PluginInstrument {
    // 先 drop 音频半，再 drop 控制半。
    processor: manguebyte::host::any::AnyProcessor,
    opened: crate::plugin::Opened,
    pending: manguebyte::prelude::EventBuffer,
    clock: crate::transport::Clock,
    sample_rate: f64,
    blocks: u32,
    /// 多声道输出：插件按全部主输出声道处理，再按 `@outputs` 混成立体声。
    wide: Option<WideOutput>,
}

/// 插件主输出的宽缓冲、辅助输出的缓冲和混音方案；`taps` 是送到总线的各路（每块重写）。
#[cfg(feature = "host")]
struct WideOutput {
    buffer: AudioBuffer<f32>,
    aux: Vec<AudioBuffer<f32>>,
    mix: Vec<crate::plugin::OutputMix>,
    taps: Vec<AudioBuffer<f32>>,
}

#[cfg(feature = "host")]
impl PluginInstrument {
    /// 交给插件的缓冲：有 `@outputs` 时是宽缓冲（加上辅助输出），按方案混进 `buffer` 与各路直接输出；否则就是调用方的立体声缓冲。
    fn run(
        processor: &mut manguebyte::host::any::AnyProcessor,
        wide: &mut Option<WideOutput>,
        buffer: &mut AudioBuffer<f32>,
        ctx: &manguebyte::prelude::ProcessContext,
    ) {
        let Some(wide) = wide else {
            processor.process_with(buffer, ctx);
            return;
        };
        let frames = buffer.num_frames();
        for b in std::iter::once(&mut wide.buffer).chain(wide.aux.iter_mut()).chain(wide.taps.iter_mut()) {
            if frames > b.capacity() {
                b.resize(b.num_channels(), frames);
            }
            b.set_num_frames(frames);
            b.clear();
        }
        if wide.aux.is_empty() {
            processor.process_with(&mut wide.buffer, ctx);
        } else if let Some(p) = processor.as_vst3_mut() {
            p.process_with_aux(&mut wide.buffer, ctx, &mut wide.aux);
        } else if let Some(p) = processor.as_clap_mut() {
            p.process_with_aux(&mut wide.buffer, ctx, &mut wide.aux);
        } else if let Some(p) = processor.as_lv2_mut() {
            p.process_with_aux(&mut wide.buffer, ctx, &mut wide.aux);
        } else if let Some(p) = processor.as_au_mut() {
            p.process_with_aux(&mut wide.buffer, ctx, &mut wide.aux);
        } else {
            processor.process_with(&mut wide.buffer, ctx);
        }
        for ch in 0..buffer.num_channels().min(2) {
            buffer.channel_mut(ch).fill(0.0);
        }
        for m in &wide.mix {
            let dst = match m.tap {
                Some(t) => &mut wide.taps[t],
                None => &mut *buffer,
            };
            for (side, gain) in [(0, m.left), (1, m.right)] {
                if side >= dst.num_channels() {
                    continue;
                }
                let src = match m.aux {
                    // 辅助输出是立体声（单声道两边都取第一路）
                    Some(k) => wide.aux[k].channel(side.min(wide.aux[k].num_channels().saturating_sub(1))),
                    None => wide.buffer.channel(m.channel),
                };
                for (d, s) in dst.channel_mut(side).iter_mut().zip(src) {
                    *d += s * gain;
                }
            }
        }
    }

    /// 等插件在后台线程加载完采样：静音喂给它，按真实时间推进 `seconds` 秒。
    fn warm_up(&mut self, seconds: f64, spec: &ProcessSpec) {
        use manguebyte::prelude::ProcessContext;
        let frames = spec.max_block_size.clamp(32, 1024);
        let mut scratch = AudioBuffer::<f32>::new(spec.num_channels.max(2), frames);
        let none = manguebyte::prelude::EventBuffer::with_capacity(1);
        let start = std::time::Instant::now();
        while start.elapsed().as_secs_f64() < seconds {
            scratch.clear();
            let transport = self.clock.now(self.sample_rate);
            let ctx = ProcessContext::new(&none, &transport);
            Self::run(&mut self.processor, &mut self.wide, &mut scratch, &ctx);
            let _ = self.opened.plugin.pump();
            std::thread::sleep(std::time::Duration::from_secs_f64(frames as f64 / self.sample_rate));
        }
    }
}

#[cfg(feature = "host")]
impl Playable for PluginInstrument {
    fn prepare(&mut self, spec: &ProcessSpec) {
        let wide_spec = self.wide.as_ref().map(|w| ProcessSpec { num_channels: w.buffer.num_channels(), ..*spec });
        let spec = wide_spec.as_ref().unwrap_or(spec);
        self.processor.prepare(spec);
        if let Err(w) = self.opened.settle(&mut self.processor, spec) {
            eprintln!("[muser] warning: {w}");
        }
        if self.opened.warmup > 0.0 {
            self.warm_up(self.opened.warmup, spec);
        }
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, _accent: bool) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event { offset: 0, kind: EventKind::NoteOn { note, velocity } });
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event { offset: 0, kind: EventKind::NoteOff { note, velocity } });
    }
    fn midi(&mut self, message: MidiMessage) {
        let _ = self.pending.push(midi_event(message));
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        use manguebyte::prelude::ProcessContext;
        let transport = self.clock.now(self.sample_rate);
        let ctx = ProcessContext::new(&self.pending, &transport);
        Self::run(&mut self.processor, &mut self.wide, buffer, &ctx);
        self.pending.clear();
        self.clock.advance(buffer.num_frames());
        self.blocks = self.blocks.wrapping_add(1);
        if self.blocks.is_multiple_of(16) {
            let _ = self.opened.plugin.pump();
        }
    }
    fn taps(&self) -> &[AudioBuffer<f32>] {
        self.wide.as_ref().map_or(&[], |w| w.taps.as_slice())
    }
    /// 自动化：`name` 为插件参数的数字 id（字符串形式），值为插件自动化刻度。
    fn set_param(&mut self, name: &str, value: f64) {
        if let Ok(id) = name.parse::<u32>() {
            let _ = self.opened.plugin.set_param(manguebyte::ParamId(id), value);
        }
    }
}

/// 解析 `plugin:` 乐器。`spec`：`/path/to.vst3` 或 `/path.vst3#插件id`。
#[cfg(feature = "host")]
fn resolve_plugin(spec: &str, params: &InstrumentParams, ps: &ProcessSpec) -> Result<TrackInstrument, InstrumentError> {
    let (path, id) = crate::plugin::split_spec(spec);
    let load_err = |detail: String| InstrumentError::PluginLoad { path: path.to_string(), detail };
    let mut opened = crate::plugin::Opened::open(path, id).map_err(load_err)?;
    opened.apply(params).map_err(|detail| InstrumentError::Param { detail })?;
    let wide = opened.outputs.clone().map(|mix| {
        let aux = if mix.iter().any(|m| m.aux.is_some()) {
            opened.aux_output_channels().into_iter().map(|c| AudioBuffer::new(c.max(1), ps.max_block_size)).collect()
        } else {
            Vec::new()
        };
        let taps = mix.iter().filter(|m| m.tap.is_some()).map(|_| AudioBuffer::new(2, ps.max_block_size)).collect();
        WideOutput { buffer: AudioBuffer::new(opened.main_output_channels(), ps.max_block_size), aux, mix, taps }
    });
    let processor = opened.activate_any(ps.sample_rate, ps.max_block_size).map_err(load_err)?;
    Ok(TrackInstrument::new(
        PluginInstrument {
            processor,
            opened,
            pending: manguebyte::prelude::EventBuffer::with_capacity(256),
            clock: crate::transport::clock(),
            sample_rate: ps.sample_rate,
            blocks: 0,
            wide,
        },
        0,
    ))
}

#[cfg(not(feature = "host"))]
fn resolve_plugin(_spec: &str, _params: &InstrumentParams, _ps: &ProcessSpec) -> Result<TrackInstrument, InstrumentError> {
    Err(InstrumentError::HostDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_accepted_by_every_instrument() {
        let mut p = InstrumentParams::new();
        p.insert("@label".into(), serde_json::json!("Warm keys"));
        for name in ["poly.wurly", "guitar.steel", "drums.808", "audio"] {
            check(name, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn catalog_has_all_presets_and_unique_names() {
        let cat = catalog();
        assert_eq!(cat.iter().filter(|i| i.family.starts_with("poly/")).count(), 68);
        let mut names: Vec<_> = cat.iter().map(|i| i.name.clone()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "instrument names must be unique");
    }

    #[test]
    fn every_catalog_entry_resolves() {
        for info in catalog() {
            assert!(resolve(&info.name).is_ok(), "{} should resolve", info.name);
        }
    }

    #[test]
    fn aliases_and_case_insensitive() {
        assert!(resolve("808").is_ok());
        assert!(resolve("POLY.MOOG_BASS").is_ok());
        assert_eq!(describe("marimba").unwrap().name, "mallet.marimba");
        assert!(check_name("no.such").is_err());
        // 非 host 只检查语法；host 下会加载插件，不存在的路径报错。
        #[cfg(not(feature = "host"))]
        assert!(check_name("plugin:/x.vst3").is_ok());
        #[cfg(feature = "host")]
        assert!(check_name("plugin:/x.vst3").unwrap_err().to_string().contains("cannot find plugin"));
        assert!(check_name("plugin:").is_err());
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Moog Bass"), "moog_bass");
        assert_eq!(slug("FM Electric Piano"), "fm_electric_piano");
    }
}
