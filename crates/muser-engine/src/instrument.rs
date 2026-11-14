//! 乐器注册表与描述表。
//!
//! 音轨的 `instrument` 字段有这几种写法：
//! - 内置乐器名，如 `"drums.808"`、`"guitar.nylon"`、`"strings.cello"`（manguebyte 自带的音色）；
//! - muser 自己做的乐器（yinse），都在 `yinse.<发声方式>.<名字>` 下，如 `"yinse.pluck.electric_guitar"`；
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
    poly, AcidParams, AcidVoice, BowedParams, BowedString, ChoirParams, ChoirVoice, DrumKit, DrumKitVoice, FmParams, FmVoice, MalletParams, MalletVoice,
    OrganParams, OrganVoice, PluckedString, PolySynthParams, PolySynthVoice, StringParams, WindParams, WindVoice,
};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{decibels, AudioBuffer, Filter, FilterShape, NoteRef, ProcessSpec, Processor, Synth, Voice};
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
    /// 下一个音唱的字与这个音的长度（帧）：`yinse.voice.*` 用，其它乐器忽略。
    fn lyric(&mut self, _text: &str, _frames: u64, _lead: u64) {}
    /// 滑到 `key` 的音高（`frames` 帧里）：`yinse.voice.*` 用，其它乐器忽略。
    fn slide(&mut self, _key: u8, _frames: u64) {}
    /// 一声吸气（`frames` 帧）：`yinse.voice.*` 用，其它乐器忽略。
    fn inhale(&mut self, _frames: u64) {}
    /// 调律（工程的 `tuning`）。默认忽略（鼓组、插件）。
    fn set_tuning(&mut self, _tuning: &manguebyte::dsp::Tuning) {}
    /// 古琴逐音奏法，在同帧起音前按通道和键位交给对应音符。
    fn technique(&mut self, _note: NoteRef, _technique: crate::zither::NoteTag) {}
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
    /// 下一个音唱的字（`yinse.voice.*`）。
    pub fn slide(&mut self, key: u8, frames: u64) {
        self.inner.slide(key, frames);
    }
    pub fn inhale(&mut self, frames: u64) {
        self.inner.inhale(frames);
    }
    pub fn lyric(&mut self, text: &str, frames: u64, lead: u64) {
        self.inner.lyric(text, frames, lead);
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
    pub(crate) fn technique(&mut self, note: NoteRef, technique: crate::zither::NoteTag) {
        self.inner.technique(note, technique);
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

impl Playable for crate::zither::Zither {
    fn technique(&mut self, note: NoteRef, technique: crate::zither::NoteTag) {
        crate::zither::Zither::playing(self, note, technique);
    }
    fn prepare(&mut self, spec: &ProcessSpec) {
        crate::zither::Zither::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        crate::zither::Zither::note_on(self, note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        crate::zither::Zither::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        crate::zither::Zither::process(self, buffer);
    }
    fn midi(&mut self, message: MidiMessage) {
        crate::zither::Zither::midi(self, message);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        crate::zither::Zither::set_param(self, name, value);
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        crate::zither::Zither::set_tuning(self, tuning);
    }
}

impl Playable for crate::chinese::Dizi {
    fn technique(&mut self, _note: NoteRef, tag: crate::zither::NoteTag) {
        crate::chinese::Dizi::flutter_next(self, tag.flutter);
        crate::chinese::Dizi::slide_next(self, tag.slide);
    }
    fn prepare(&mut self, spec: &ProcessSpec) {
        crate::chinese::Dizi::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        crate::chinese::Dizi::note_on(self, note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        crate::chinese::Dizi::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        crate::chinese::Dizi::process(self, buffer);
    }
    fn midi(&mut self, message: MidiMessage) {
        crate::chinese::Dizi::midi(self, message);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        crate::chinese::Dizi::set_param(self, name, value);
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        crate::chinese::Dizi::set_tuning(self, tuning);
    }
}

impl Playable for crate::chinese::Erhu {
    fn prepare(&mut self, spec: &ProcessSpec) {
        crate::chinese::Erhu::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        crate::chinese::Erhu::note_on(self, note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        crate::chinese::Erhu::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        crate::chinese::Erhu::process(self, buffer);
    }
    fn midi(&mut self, message: MidiMessage) {
        crate::chinese::Erhu::midi(self, message);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        crate::chinese::Erhu::set_param(self, name, value);
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        crate::chinese::Erhu::set_tuning(self, tuning);
    }
}

/// 歌声：一个单音的声部，歌词交给它。不经过 Synth：Synth 每个新音前会把还在收尾的声音一下切掉（choke），
/// 人声上听得出"破音"；这里新音直接接着上一个音（声门、声道都不断），像歌手换字。
struct Singer {
    voice: crate::singer::SingerVoice,
    params: crate::singer::SingerParams,
    key: Option<u8>,
    /// 当前这个键按下了几次没松（辅音提前时，同一个音高的下一个字会在上一个字松开之前开始）。
    downs: u32,
}

impl Playable for Singer {
    fn prepare(&mut self, spec: &ProcessSpec) {
        manguebyte::prelude::Voice::prepare(&mut self.voice, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, _accent: bool) {
        manguebyte::prelude::Voice::note_on(&mut self.voice, note, velocity);
        self.downs = if self.key == Some(note.key) { self.downs + 1 } else { 1 };
        self.key = Some(note.key);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        if self.key == Some(note.key) {
            self.downs = self.downs.saturating_sub(1);
            if self.downs == 0 {
                manguebyte::prelude::Voice::note_off(&mut self.voice, velocity);
                self.key = None;
            }
        }
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        buffer.clear();
        manguebyte::prelude::Voice::render(&mut self.voice, buffer);
    }
    fn midi(&mut self, message: MidiMessage) {
        if let MidiMessage::PitchBend { value, .. } = message {
            manguebyte::prelude::Voice::pitch_bend(&mut self.voice, f64::from(value) / 8192.0 * 2.0);
        }
    }
    fn slide(&mut self, key: u8, frames: u64) {
        self.voice.slide(key, frames);
    }
    fn inhale(&mut self, frames: u64) {
        self.voice.inhale(frames);
    }
    fn lyric(&mut self, text: &str, frames: u64, lead: u64) {
        self.voice.lyric(text, frames, lead);
    }
    fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::SINGER, &mut self.params, name, value) {
            self.voice.set_params(self.params);
        }
    }
    fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        manguebyte::prelude::Voice::set_tuning(&mut self.voice, tuning);
    }
}

/// 鼓组：参数名为 `鼓件.参数`，如 `kick.tune`；改动从下一次击打生效。
struct DrumsTunable {
    synth: Synth<DrumKitVoice, f32>,
    kit: DrumKit,
}

impl Playable for crate::percussion::Kit {
    fn prepare(&mut self, spec: &ProcessSpec) {
        crate::percussion::Kit::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        crate::percussion::Kit::note_on(self, note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        crate::percussion::Kit::note_off(self, note, velocity);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        crate::percussion::Kit::process(self, buffer);
    }
    fn midi(&mut self, _message: MidiMessage) {}
    fn set_param(&mut self, name: &str, value: f64) {
        crate::percussion::Kit::set_param(self, name, value);
    }
    fn set_tuning(&mut self, _tuning: &manguebyte::dsp::Tuning) {}
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
    /// `yinse.strike.drums`：同 `Drums`，底鼓另走一路 EQ（[`YinseDrums`]）。
    YinseDrums(DrumKit),
    Kit(crate::percussion::KitParams),
    Poly(PolySynthParams),
    Fm(FmParams),
    String(StringParams, usize),
    Electric(crate::electric::ElectricParams, usize),
    Accordion(crate::accordion::AccordionParams),
    Zither(crate::zither::ZitherParams),
    Dizi(crate::chinese::DiziParams),
    Erhu(BowedParams),
    Singer(crate::singer::SingerParams),
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
            Patch::Accordion(p) => params::apply(params::ACCORDION, p, params),
            Patch::Zither(p) => params::apply(params::ZITHER, p, params),
            Patch::Dizi(p) => params::apply(params::DIZI, p, params),
            Patch::Erhu(p) => params::apply(params::BOWED, p, params),
            Patch::Singer(p) => params::apply(params::SINGER, p, params),
            Patch::Acid(p) => params::apply(params::ACID, p, params),
            Patch::Organ(p) => params::apply(params::ORGAN, p, params),
            Patch::Mallet(p, _) => params::apply(params::MALLET, p, params),
            Patch::Bowed(p, _) => params::apply(params::BOWED, p, params),
            Patch::Wind(p, _) => params::apply(params::WIND, p, params),
            Patch::Choir(p, _) => params::apply(params::CHOIR, p, params),
            Patch::Kit(kit) => {
                for (piece, value) in params {
                    let i = kit.index(piece).ok_or_else(|| format!("unknown piece '{piece}': this kit has {}", kit.keys()))?;
                    let obj = value.as_object().ok_or_else(|| format!("{piece}: expected an object, e.g. {{ \"tune\": -2 }}"))?;
                    params::apply(params::KIT_PIECE, &mut kit.set[i], obj).map_err(|e| format!("{piece}.{e}"))?;
                }
                Ok(())
            }
            Patch::Drums(kit) | Patch::YinseDrums(kit) => apply_drum_params(kit, params),
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
            Patch::Singer(p) => (
                params::describe(params::SINGER, p, params::NEXT_SINGER),
                "Sings the note's `lyric` (pinyin, e.g. \"ming\", \"yue\"; English spellings approximately). One note at a time: overlapping notes are sung legato and glide. \
                 The initial consonant starts a little before the note so the vowel lands on the beat; -n / -ng close the note.".into(),
            ),
            Patch::Erhu(p) => (params::describe(params::BOWED, p, params::NEXT_BOWED), "One note at a time: a note that overlaps the next slides into it under one bow; a small gap is a new bow.".into()),
            Patch::Dizi(p) => (params::describe(params::DIZI, p, params::NEXT_DIZI), "One note at a time. Overlapping notes are slurred (连奏: the fingers change the note at once, no new tongue stroke); a note with `\"slide\": true` glides from the previous one (滑音); a small gap is a new tongued note (断奏, harder with `articulation`). Ornaments are short notes or `trill` / `half_trill`; `flutter` is 花舌, `accent` 剁音; shape the tone over a phrase by automating `breath` and `embouchure`.".into()),
            Patch::Zither(p) if p.table.is_some() => (
                params::describe(params::ZITHER, p, params::NEXT_ZITHER),
                "Extracted from recordings (`muser yinse extract`): each note's partials, two-stage decay, beating, stereo and attack noise come from the nearest recorded pitches and velocities. \
                 Struck open, no dampers by default (lower `release` to damp); `decay`, `high_decay`, `pluck_position` and the body parameters do not apply.".into(),
            ),
            Patch::Zither(p) if p.profile == &crate::zither::YANGQIN => (
                params::describe(params::ZITHER, p, params::NEXT_ZITHER),
                "Every pitch has its own course of strings, struck open (no pressing, no bends); keep to the pentatonic scale for the traditional sound (e.g. D E F# A B). \
                 There are no dampers: a note rings on after it ends, so `length` only matters for tremolo. Fast runs, alternating octaves and tremolo (轮音, the note's `tremolo`) suit it.".into(),
            ),
            Patch::Zither(p) if p.profile == &crate::zither::GUZHENG => (
                params::describe(params::ZITHER, p, params::NEXT_ZITHER),
                "Tuned to D pentatonic (D E F# A B); write the notes you want: other pitches are pressed (按音) from the string below. \
                 Every note follows its `bend` (按音、滑音, up to +/-24 semitones) and the automatic left-hand vibrato (揉弦) with continuous phase. \
                 Per note, \"technique\": \"harmonic\", \"harmonic\": 2 is a harmonic (泛音); glissandi (刮奏) are fast runs of pentatonic notes; tremolo (摇指) is the note's `tremolo`.".into(),
            ),
            Patch::Zither(p) => (
                params::describe(params::ZITHER, p, params::NEXT_ZITHER),
                "Choose technique: \"open\" (散音), \"pressed\" (按音), or \"harmonic\" (泛音); \"auto\" uses open notes at C2 D2 F2 G2 A2 C3 D3 and pressed notes elsewhere. \
                 Pitch is always the sounding pitch; harmonic=4 touches at one quarter of the string and preserves every fourth mode. \
                 Only pressed notes follow bend curves (up to +/-24 semitones), with continuous phase and hand-motion noise. \
                 Override individual notes with \"technique\": \"harmonic\", \"harmonic\": 2, or choose a track default. \
                 Open notes at other pitches extend the model beyond the seven-string tuning.".into(),
            ),
            Patch::Accordion(p) => (
                params::describe(params::ACCORDION, p, params::NEXT_ACCORDION),
                "Chords and bass in the left hand are plain notes in the low register. Shape phrases with `bellows` automation (louder and brighter together); `velocity` also pushes the bellows a little.".into(),
            ),
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
            Patch::Kit(kit) => (
                params::specs(params::KIT_PIECE, params::NEXT_KIT_PIECE),
                format!("Set per piece, by name or key: {{ \"daluo\": {{ \"tune\": -2, \"decay\": 1.5 }}, \"52\": {{ \"level\": 0.8 }} }}; automate as `piece.param` (e.g. \"daluo.tune\"). Pieces: {}", kit.keys()),
            ),
            Patch::Drums(kit) | Patch::YinseDrums(kit) => (
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
            Patch::Kit(kit) => TrackInstrument::new(crate::percussion::Kit::new(kit), 9),
            Patch::Drums(kit) => TrackInstrument::new(DrumsTunable { synth: Synth::new(DrumKitVoice::voices(24, kit)), kit }, 9),
            Patch::YinseDrums(kit) => TrackInstrument::new(YinseDrums::new(kit), 9),
            Patch::Poly(p) => tunable(Synth::with_voices(16, move || PolySynthVoice::new(p)), p, params::POLY, |v, p| v.set_params(p)),
            Patch::Fm(p) => tunable(Synth::with_voices(16, move || FmVoice::new(p)), p, params::FM, |v, p| v.set_params(p)),
            Patch::String(p, n) => tunable(Synth::with_voices(n, move || PluckedString::new(p)), p, params::STRING, |v, p| v.set_params(p)),
            Patch::Electric(p, n) => TrackInstrument::new(crate::electric::Electric::new(p, n), 0),
            Patch::Zither(p) => TrackInstrument::new(crate::zither::Zither::new(p), 0),
            Patch::Dizi(p) => TrackInstrument::new(crate::chinese::Dizi::new(p), 0),
            Patch::Erhu(p) => TrackInstrument::new(crate::chinese::Erhu::new(p), 0),
            Patch::Singer(p) => TrackInstrument::new(Singer { voice: crate::singer::SingerVoice::new(p), params: p, key: None, downs: 0 }, 0),
            Patch::Accordion(p) => {
                tunable(Synth::with_voices(16, move || crate::accordion::AccordionVoice::new(p)), p, params::ACCORDION, |v, p| v.set_params(p))
            }
            Patch::Acid(p) => {
                let mut s = Synth::with_voices(1, move || AcidVoice::new(p));
                s.set_legato(true);
                tunable(s, p, params::ACID, |v, p| v.set_params(p))
            }
            Patch::Organ(p) => tunable(Synth::new(OrganVoice::voices(16, p)), p, params::ORGAN, |v, p| v.set_params(p)),
            Patch::Mallet(p, n) => tunable(Synth::with_voices(n, move || MalletVoice::new(p)), p, params::MALLET, |v, p| v.set_params(p)),
            Patch::Bowed(p, n) => {
                let mut s = Synth::with_voices(n, move || BowedString::new(p));
                // 单音的拉弦乐器（二胡）：连着的音是一弓里的换指，按 portamento 滑过去。
                s.set_legato(n == 1);
                tunable(s, p, params::BOWED, |v, p| v.set_params(p))
            }
            Patch::Wind(p, n) => {
                let mut s = Synth::with_voices(n, move || WindVoice::new(p));
                // 单音的吹管（箫）：连着的音一口气吹下来，按 portamento 滑过去。
                s.set_legato(n == 1);
                tunable(s, p, params::WIND, |v, p| v.set_params(p))
            }
            Patch::Choir(p, n) => tunable(Synth::with_voices(n, move || ChoirVoice::new(p)), p, params::CHOIR, |v, p| v.set_params(p)),
            Patch::Sfz(path, cc) => sfz_instrument(&path, &cc)?,
        })
    }
}

/// 鼓组乐器（`drums.*`）有哪些鼓键；不是鼓组时为 `None`。
pub fn drum_kit_keys(name: &str) -> Option<Vec<u8>> {
    match lookup(name)?.1 {
        Patch::Drums(kit) | Patch::YinseDrums(kit) => Some(kit.keys().collect()),
        Patch::Kit(kit) => Some(kit.defs.iter().map(|d| d.key).collect()),
        _ => None,
    }
}

fn apply_drum_params(kit: &mut DrumKit, params: &InstrumentParams) -> Result<(), String> {
    for (piece, value) in params {
        let key = params::piece_key(piece).ok_or_else(|| {
            format!(
                "unknown drum piece '{piece}': use a GM drum key number or an alias ({})",
                params::PIECE_ALIASES.iter().map(|(a, _)| *a).collect::<Vec<_>>().join(" ")
            )
        })?;
        let mut p = *kit.piece(key).ok_or_else(|| format!("this kit has no drum key {key} ({piece}), supported: {}", kit_keys(kit)))?;
        let obj = value.as_object().ok_or_else(|| format!("{piece}: expected an object, e.g. {{ \"tune\": -2 }}"))?;
        params::apply(params::PIECE, &mut p, obj).map_err(|e| format!("{piece}.{e}"))?;
        kit.set(key, Some(p));
    }
    Ok(())
}

/// yinse 的原声套鼓：manguebyte 的原声鼓组，每件按一套放克鼓的采样调过（音高、衰减、亮度、起音、质感、音量，
/// 自动拟合对数梅尔谱）。
fn yinse_drums() -> DrumKit {
    let mut kit = DrumKit::acoustic();
    let tuned: InstrumentParams = serde_json::from_str(YINSE_DRUMS).expect("valid built-in drum settings");
    apply_drum_params(&mut kit, &tuned).expect("built-in drum settings fit the acoustic kit");
    kit
}

const YINSE_DRUMS: &str = r#"{"36":{"tune":4.0,"decay":0.852,"tone":0.0,"snap":1.0,"texture":0.93,"level":1.05},"37":{"tune":-8.723,"decay":0.3633,"tone":0.0073,"snap":0.9952,"texture":0.382,"level":0.6308},"38":{"tune":3.3,"decay":0.6,"tone":0.3,"snap":0.55,"texture":0.3276,"level":1.1},"39":{"tune":-11.799,"decay":2.5995,"tone":0.0078,"snap":0.9599,"texture":0.9957,"level":1.9046},"40":{"tune":3.3,"decay":0.5,"tone":0.55,"snap":0.8,"texture":0.15,"level":1.9},"41":{"tune":-6.083,"decay":3.4674,"tone":0.1653,"snap":1.0,"texture":0.3728,"level":0.3429},"42":{"tune":-10.65,"decay":2.6952,"tone":0.3175,"snap":0.6453,"texture":0.9952,"level":0.1534},"43":{"tune":-6.119,"decay":3.2556,"tone":0.2629,"snap":0.9974,"texture":0.3484,"level":0.3113},"44":{"tune":-6.334,"decay":3.1418,"tone":0.8249,"snap":0.0632,"texture":0.9952,"level":0.6628},"45":{"tune":-0.895,"decay":2.8685,"tone":0.5505,"snap":0.9973,"texture":0.4516,"level":0.318},"46":{"tune":-7.638,"decay":0.2162,"tone":0.9832,"snap":0.1947,"texture":0.4913,"level":0.7299},"47":{"tune":-5.193,"decay":3.1269,"tone":1.0,"snap":0.9957,"texture":0.3856,"level":0.4311},"48":{"tune":1.481,"decay":1.8786,"tone":0.2739,"snap":0.9951,"texture":0.7409,"level":0.362},"49":{"tune":-6.299,"decay":0.9317,"tone":0.9939,"snap":0.9952,"texture":0.1799,"level":0.4995},"50":{"tune":-0.636,"decay":2.3253,"tone":0.5702,"snap":1.0,"texture":0.3861,"level":0.3744}}"#;

/// 力度的指数（1 不变，越小轻敲越响）：对着采样套鼓 0.35 / 0.95 两个力度的音量差量的。
fn velocity_curve(key: u8) -> f32 {
    match key {
        39 => 0.65,
        42 => 1.1,
        44 => 0.55,
        46 => 0.73,
        55 => 0.15,
        51 => 0.9,
        54 => 0.35,
        56 => 1.6,
        59 => 0.7,
        _ => 0.8,
    }
}

/// 军鼓随力度差多少（dB，相对力度 1.0）：采样军鼓八个力度量出来的，中间连直线、两头顺着延出去；
/// 换成 [`crate::percussion::Kit`] 的力度（它的音量是力度的 1.4 次方）。鼓皮（320 Hz 以下）轻敲时掉得比响弦快。
const SNARE_BODY_DB: &[(f64, f64)] =
    &[(0.126, -31.9), (0.252, -26.0), (0.378, -20.7), (0.5039, -17.1), (0.6299, -10.4), (0.7559, -5.4), (0.8819, -1.5), (1.0, 0.0)];
/// 鼓腔的余音（600 Hz … 2 kHz 的那一圈"嗡"）：轻敲时掉得最少。
const SNARE_RING_DB: &[(f64, f64)] =
    &[(0.126, -41.9), (0.252, -31.4), (0.378, -21.7), (0.5039, -16.1), (0.6299, -12.4), (0.7559, -7.4), (0.8819, -1.6), (1.0, 0.0)];
/// 响弦头二十毫秒的"唰"，2.8 kHz 以下与以上：轻敲时中频掉得多，高频掉得少。
const SNARE_TOP_LOW_DB: &[(f64, f64)] =
    &[(0.126, -29.1), (0.252, -23.7), (0.378, -19.9), (0.5039, -14.1), (0.6299, -10.8), (0.7559, -6.9), (0.8819, -2.0), (1.0, 0.0)];
const SNARE_TOP_HIGH_DB: &[(f64, f64)] =
    &[(0.126, -14.1), (0.252, -11.3), (0.378, -7.8), (0.5039, -7.8), (0.6299, -7.8), (0.7559, -7.8), (0.8819, 0.0), (1.0, 0.0)];
/// 响弦的尾巴（50 ms 以后），同样分两段：轻敲时掉得更多。
const SNARE_TAIL_LOW_DB: &[(f64, f64)] =
    &[(0.126, -50.0), (0.252, -43.2), (0.378, -38.2), (0.5039, -38.2), (0.6299, -26.0), (0.7559, -26.0), (0.8819, -26.0), (1.0, 0.0)];
const SNARE_TAIL_HIGH_DB: &[(f64, f64)] =
    &[(0.126, -49.2), (0.252, -34.3), (0.378, -16.8), (0.5039, -12.6), (0.6299, -4.4), (0.7559, -2.0), (0.8819, -0.9), (1.0, 0.0)];
/// 鼓槌那一下"啪"（头 2 ms）：重击才有，力度 0.75 以下几乎没有（鬼音是软的）。
const SNARE_CLICK_DB: &[(f64, f64)] =
    &[(0.126, -50.0), (0.252, -50.0), (0.378, -50.0), (0.5039, -50.0), (0.6299, -50.0), (0.7559, -45.0), (0.8819, 0.0), (1.0, 0.0)];

/// 军鼓的七套与各自的力度表。
const SNARE_PARTS: [(&[crate::percussion::PieceDef], &[(f64, f64)]); 7] = [
    (crate::percussion::SNARE_BODY, SNARE_BODY_DB),
    (crate::percussion::SNARE_RING, SNARE_RING_DB),
    (crate::percussion::SNARE_TOP_LOW, SNARE_TOP_LOW_DB),
    (crate::percussion::SNARE_TOP_HIGH, SNARE_TOP_HIGH_DB),
    (crate::percussion::SNARE_TAIL_LOW, SNARE_TAIL_LOW_DB),
    (crate::percussion::SNARE_TAIL_HIGH, SNARE_TAIL_HIGH_DB),
    (crate::percussion::SNARE_CLICK, SNARE_CLICK_DB),
];

/// rimshot（40）的七套与各自的力度表（采样里三个力度：0.35、0.6、0.95）。
const RIM_BODY_DB: &[(f64, f64)] = &[(0.35, -23.3), (0.6, -11.8), (0.95, 0.0)];
const RIM_RING_DB: &[(f64, f64)] = &[(0.35, -34.2), (0.6, -19.3), (0.95, 0.0)];
const RIM_TOP_LOW_DB: &[(f64, f64)] = &[(0.35, -22.5), (0.6, -13.7), (0.95, 0.0)];
const RIM_TOP_HIGH_DB: &[(f64, f64)] = &[(0.35, 0.0), (0.6, 0.0), (0.95, 0.0)];
const RIM_TAIL_LOW_DB: &[(f64, f64)] = &[(0.35, -25.6), (0.6, -12.4), (0.95, 0.0)];
const RIM_TAIL_HIGH_DB: &[(f64, f64)] = &[(0.35, -26.7), (0.6, -6.8), (0.95, 0.0)];
const RIM_CLICK_DB: &[(f64, f64)] = &[(0.35, -50.0), (0.6, -47.0), (0.95, 0.0)];
const RIM_PARTS: [(&[crate::percussion::PieceDef], &[(f64, f64)]); 7] = [
    (crate::percussion::RIM_BODY, RIM_BODY_DB),
    (crate::percussion::RIM_RING, RIM_RING_DB),
    (crate::percussion::RIM_TOP_LOW, RIM_TOP_LOW_DB),
    (crate::percussion::RIM_TOP_HIGH, RIM_TOP_HIGH_DB),
    (crate::percussion::RIM_TAIL_LOW, RIM_TAIL_LOW_DB),
    (crate::percussion::RIM_TAIL_HIGH, RIM_TAIL_HIGH_DB),
    (crate::percussion::RIM_CLICK, RIM_CLICK_DB),
];

/// 边击（37）随力度差多少（dB）：采样的三个力度量出来的。
const SIDE_STICK_DB: [(f64, f64); 3] = [(0.35, -12.8), (0.6, -5.7), (0.95, 0.0)];

/// 吊镲（49）随力度差多少（dB）：采样的三个力度量出来的（先把采样里叠在这个键上的那个有音高的声音去掉）。
const CRASH_DB: [(f64, f64); 3] = [(0.35, -12.7), (0.6, -8.8), (0.95, 0.0)];

/// 吊镲二（57，sizzle）随力度差多少（dB）：采样的三个力度层量出来的，中等力度比重击轻 12 dB。
const CRASH2_DB: [(f64, f64); 3] = [(0.197, -16.4), (0.504, -12.1), (0.9055, 0.0)];

fn measured_velocity(points: &[(f64, f64)], raw: f32) -> f32 {
    let raw = f64::from(raw);
    let i = points.windows(2).position(|w| raw < w[1].0).unwrap_or(points.len() - 2);
    let ((x0, y0), (x1, y1)) = (points[i], points[i + 1]);
    let db = y0 + (raw - x0) * (y1 - y0) / (x1 - x0);
    // 比表里最轻的还轻：至少跟着力度一起小下去（表的头一段有的很平，照直线延出去的话再轻也不变小）。
    let db = if raw < points[0].0 { db.min(points[0].1 + 20.0 * (raw.max(1e-3) / points[0].0).log10()) } else { db };
    // 表外顺着延出去，但不超过 +3 dB（鼓槌那一层在 0.7 … 0.95 之间升得很陡）。
    10f64.powf(db.clamp(-50.0, 3.0) / 20.0 / 1.4) as f32
}

/// 边击、拍手、踩镲、吊镲与 ride（51，钟面 53）另走一路的键（各一套模态 + 噪声，各自的 EQ）。
const HAT_KEYS: [u8; 14] = [37, 39, 42, 44, 46, 49, 51, 53, 54, 55, 56, 57, 58, 59];

/// 镲那一路每件的 EQ：(2.2 kHz 压多少, 9 kHz 以上提多少, 300 Hz 提多少) dB。
fn hat_eq(key: u8) -> (f64, f64, f64) {
    match key {
        39 => (0.00, -6.00, 0.00),
        // 铃鼓：铃片的声音几乎都在 3 kHz 以上，高处那几层噪声的下沿漏下来的压掉。
        54 => (-15.00, 0.00, -20.00),
        55 | 57 => (0.00, 1.00, 4.72),
        _ => (0.0, 0.0, 0.0),
    }
}

/// 通鼓的键（落地通鼓到高音通鼓）。
const TOM_KEYS: [u8; 6] = [41, 43, 45, 47, 48, 50];

/// 底鼓另走一路的键（35、36）。
const KICK_KEYS: [u8; 2] = [35, 36];

/// 底鼓的鼓腔（160 Hz 以下）随力度差多少：采样底鼓轻、中、重三下量出来的（力度 0.4 比 0.95 轻 7.9 dB，0.7 只轻 1.1 dB），
/// 中间按 dB 连直线、两头顺着延出去；换成 [`crate::percussion::Kit`] 的力度（它的音量是力度的 1.4 次方）。
fn kick_body_velocity(raw: f32) -> f32 {
    let raw = f64::from(raw);
    let db = if raw < 0.7 { -1.1 + (raw - 0.7) * (6.8 / 0.3) } else { -1.1 + (raw - 0.7) * (1.1 / 0.25) };
    10f64.powf(db.max(-40.0) / 20.0 / 1.4) as f32
}

/// 底鼓的鼓槌声与带起来的响弦（160 Hz 以上）随力度的 2.4 次方：轻踩几乎只剩鼓腔的"咚"。
fn kick_top_velocity(raw: f32) -> f32 {
    (f64::from(raw) / 0.95).powf(2.4 / 1.4) as f32
}

/// `yinse.strike.drums`：底鼓、军鼓、通鼓、镲各走各的一路，其余的鼓件用 manguebyte 的原声鼓组。
struct YinseDrums {
    drums: DrumsTunable,
    /// 底鼓（35、36）：鼓腔一套、鼓槌与响弦一套（[`crate::percussion::KICK_BODY`]、[`crate::percussion::KICK_TOP`]），
    /// 一起敲，随力度各变各的。
    kick: [crate::percussion::Kit; 2],
    /// 军鼓（38）：鼓皮、鼓腔的余音、响弦（"唰"与尾巴、中频与高频共四套）、鼓槌的"啪"各一套（[`SNARE_PARTS`]），同上。
    snare38: [crate::percussion::Kit; 7],
    /// 踩镲（42 44 46）另走一路 EQ：这个物理模型的踩镲 2 kHz 一带太多、9 kHz 以上太少。
    /// 每件踩镲各一套（各自的 EQ）；闭镲、踩镲掐断开镲。
    hats: Vec<(u8, crate::percussion::Kit, Vec<Filter<f32>>)>,
    /// rimshot（40）：同一只军鼓重一些的打法，也是七套（[`RIM_PARTS`]）。
    rim40: [crate::percussion::Kit; 7],
    scratch: AudioBuffer<f32>,
    /// 整套鼓的压缩（采样套鼓是压过的：峰值只比均方根高十几 dB，听着有气势）：包络。
    comp_env: f64,
    sr: f64,
    /// 通鼓（[`TOM_KEYS`]）：鼓腔一套、鼓槌与鼓皮的"沙"一套，各六件（[`crate::percussion::TOM_BODY`]、[`crate::percussion::TOM_TOP`]），同底鼓。
    toms: [crate::percussion::Kit; 2],
}

/// 整套鼓的压缩：门限 dB、比例、起、落（秒）。
const KIT_COMP: (f64, f64, f64, f64) = (-18.0, 6.0, 0.00002, 0.08);

impl YinseDrums {
    fn new(kit: DrumKit) -> Self {
        // 底鼓、军鼓不用 manguebyte 的模型了：`instrument_params` 里给它们写的 level / tune / decay / tone / pan
        // （相对内置的那套）折算到各自的两套上。
        let built_in = yinse_drums();
        fn kits<const N: usize>(kit: &DrumKit, built_in: &DrumKit, defs: [&'static [crate::percussion::PieceDef]; N]) -> [crate::percussion::Kit; N] {
            let mut params = defs.map(crate::percussion::KitParams::of);
            for k in &mut params {
                for (def, set) in k.defs.iter().zip(&mut k.set) {
                    let (Some(now), Some(base)) = (kit.piece(def.key), built_in.piece(def.key)) else { continue };
                    set.level = now.level / base.level.max(1e-6);
                    set.tune = now.tune - base.tune;
                    set.decay = now.decay / base.decay.max(1e-6);
                    set.tone = (0.5 + now.tone - base.tone).clamp(0.0, 1.0);
                    // 没改声像的用量出来的（军鼓略偏一点，通鼓从中间排到右边）。
                    if (now.pan - base.pan).abs() > 1e-9 {
                        set.pan = now.pan;
                    }
                }
            }
            params.map(crate::percussion::Kit::new)
        }
        let kick = kits(&kit, &built_in, [crate::percussion::KICK_BODY, crate::percussion::KICK_TOP]);
        let toms = kits(&kit, &built_in, [crate::percussion::TOM_BODY, crate::percussion::TOM_TOP]);
        let snare38 = kits(&kit, &built_in, SNARE_PARTS.map(|(defs, _)| defs));
        let rim40 = kits(&kit, &built_in, RIM_PARTS.map(|(defs, _)| defs));
        Self {
            drums: DrumsTunable { synth: Synth::new(DrumKitVoice::voices(24, kit)), kit },
            kick,
            snare38,
            rim40,
            hats: HAT_KEYS.iter().map(|&k| (k, crate::percussion::Kit::new(crate::percussion::KitParams::hats()), Vec::new())).collect(),
            scratch: AudioBuffer::new(2, 0),
            comp_env: 0.0,
            sr: 48_000.0,
            toms,
        }
    }
    fn part(&mut self, key: u8) -> &mut dyn Playable {
        if let Some(i) = self.hats.iter().position(|h| h.0 == key) {
            &mut self.hats[i].1
        } else {
            // 未调教（原始音色）：52、60 … 81 还是 manguebyte 原声鼓组的声音，没有对着采样量过。
            &mut self.drums
        }
    }
}

impl Playable for YinseDrums {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.drums.prepare(spec);
        self.kick.iter_mut().chain(&mut self.snare38).chain(&mut self.rim40).for_each(|k| Playable::prepare(k, spec));
        for (key, kit, eq) in &mut self.hats {
            Playable::prepare(kit, spec);
            let (mid, high, low) = hat_eq(*key);
            *eq = vec![
                Filter::new(FilterShape::Peak { gain_db: mid }, 2_200.0, 0.8),
                Filter::new(FilterShape::HighShelf { gain_db: high }, 9_000.0, 0.7),
                Filter::new(FilterShape::Peak { gain_db: low }, 300.0, 0.8),
            ];
            eq.iter_mut().for_each(|f| f.prepare(spec));
        }
        self.scratch = AudioBuffer::new(spec.num_channels.max(1), spec.max_block_size);
        self.sr = spec.sample_rate;
        self.toms.iter_mut().for_each(|k| Playable::prepare(k, spec));
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        // 采样套鼓的力度反应比这个物理模型平得多（底鼓、通鼓轻敲也有劲）：每件按量到的比例压平。
        let raw = velocity.clamp(0.0, 1.0);
        if KICK_KEYS.contains(&note.key) {
            let key = NoteRef::new(note.channel, 36);
            Playable::note_on(&mut self.kick[0], key, kick_body_velocity(raw), accent);
            Playable::note_on(&mut self.kick[1], key, kick_top_velocity(raw), accent);
            return;
        }
        if note.key == 38 {
            for (kit, (_, table)) in self.snare38.iter_mut().zip(SNARE_PARTS) {
                Playable::note_on(kit, note, measured_velocity(table, raw), accent);
            }
            return;
        }
        if note.key == 40 {
            for (kit, (_, table)) in self.rim40.iter_mut().zip(RIM_PARTS) {
                Playable::note_on(kit, note, measured_velocity(table, raw), accent);
            }
            return;
        }
        let velocity = match note.key {
            37 => measured_velocity(&SIDE_STICK_DB, raw),
            49 => measured_velocity(&CRASH_DB, raw),
            57 => measured_velocity(&CRASH2_DB, raw),
            _ => raw.powf(velocity_curve(note.key)),
        };
        if TOM_KEYS.contains(&note.key) {
            // 采样通鼓的力度反应很平：鼓腔按力度的 0.55 次方，鼓槌与"沙"按 0.8 次方（力度 0.6 比 0.95 各轻 2、3 dB）。
            let part = |power: f64| (f64::from(raw) / 0.95).powf(power / 1.4) as f32;
            Playable::note_on(&mut self.toms[0], note, part(0.55), accent);
            Playable::note_on(&mut self.toms[1], note, part(0.8), accent);
            return;
        }
        if note.key == 42 || note.key == 44 {
            // 踩下去：开着的镲被夹住。
            if let Some(open) = self.hats.iter_mut().find(|h| h.0 == 46) {
                open.1.choke();
            }
        }
        self.part(note.key).note_on(note, velocity, accent);
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        if !KICK_KEYS.contains(&note.key) && note.key != 38 && note.key != 40 && !TOM_KEYS.contains(&note.key) {
            self.part(note.key).note_off(note, velocity);
        }
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.drums.process(buffer);
        let n = buffer.num_frames();
        if self.scratch.num_frames() < n || self.scratch.num_channels() < buffer.num_channels() {
            self.scratch = AudioBuffer::new(buffer.num_channels(), n);
        }
        self.scratch.set_num_frames(n);
        for (_, kit, eq) in &mut self.hats {
            self.scratch.clear();
            Playable::process(kit, &mut self.scratch);
            eq.iter_mut().for_each(|f| f.process(&mut self.scratch));
            for c in 0..buffer.num_channels() {
                for (y, x) in buffer.channel_mut(c).iter_mut().zip(self.scratch.channel(c)) {
                    *y += *x;
                }
            }
        }
        for kit in self.kick.iter_mut().chain(&mut self.snare38).chain(&mut self.rim40).chain(&mut self.toms) {
            self.scratch.set_num_frames(n);
            self.scratch.clear();
            Playable::process(kit, &mut self.scratch);
            for c in 0..buffer.num_channels() {
                for (y, x) in buffer.channel_mut(c).iter_mut().zip(self.scratch.channel(c)) {
                    *y += *x;
                }
            }
        }
        // 压缩：峰值包络（快起慢落），超过门限的按比例压下去。
        let (thr_db, ratio, attack, release) = KIT_COMP;
        if ratio > 1.0 {
            let thr = 10f64.powf(thr_db / 20.0);
            let (ka, kr) = (1.0 - (-1.0 / (attack * self.sr)).exp(), 1.0 - (-1.0 / (release * self.sr)).exp());
            for i in 0..n {
                let peak = (0..buffer.num_channels()).map(|c| f64::from(buffer.channel(c)[i].abs())).fold(0.0, f64::max);
                let k = if peak > self.comp_env { ka } else { kr };
                self.comp_env += (peak - self.comp_env) * k;
                let g = if self.comp_env > thr { (thr / self.comp_env).powf(1.0 - 1.0 / ratio) } else { 1.0 } as f32;
                for c in 0..buffer.num_channels() {
                    buffer.channel_mut(c)[i] *= g;
                }
            }
        }
    }
    fn set_param(&mut self, name: &str, value: f64) {
        let key = name.split_once('.').and_then(|(piece, _)| params::piece_key(piece));
        match key {
            Some(k) if KICK_KEYS.contains(&k) => {
                // 两套里那一件都叫 kick（键 36）。
                let name = name.split_once('.').map_or_else(|| name.to_string(), |(_, param)| format!("kick.{param}"));
                self.kick.iter_mut().for_each(|kit| Playable::set_param(kit, &name, value));
            }
            Some(38) => {
                let name = name.split_once('.').map_or_else(|| name.to_string(), |(_, param)| format!("snare.{param}"));
                self.snare38.iter_mut().for_each(|kit| Playable::set_param(kit, &name, value));
            }
            Some(k) if TOM_KEYS.contains(&k) => {
                // 两套里每件按键号找。
                let name = name.split_once('.').map_or_else(|| name.to_string(), |(_, param)| format!("{k}.{param}"));
                self.toms.iter_mut().for_each(|kit| Playable::set_param(kit, &name, value));
            }
            Some(40) => {
                let name = name.split_once('.').map_or_else(|| name.to_string(), |(_, param)| format!("40.{param}"));
                self.rim40.iter_mut().for_each(|kit| Playable::set_param(kit, &name, value));
            }
            Some(k) if HAT_KEYS.contains(&k) => {
                for (_, kit, _) in &mut self.hats {
                    Playable::set_param(kit, name, value);
                }
            }
            _ => self.drums.set_param(name, value),
        }
    }
}

fn kit_keys(kit: &DrumKit) -> String {
    kit.keys().map(|k| k.to_string()).collect::<Vec<_>>().join(",")
}

/// SFZ 采样器乐器（内置，manguebyte-sampler）。
/// `muser instruments sfz:…` 的说明：列出 .sfz 声明的控制器。
fn sfz_controls_note(path: &std::path::Path) -> String {
    let base =
        "The sound is defined by the .sfz file. Set controller values with instrument_params \"cc\" (applied at load), e.g. { \"cc\": { \"105\": 127 } }.";
    let mut out = base.to_string();
    if let Ok(c) = crate::sfz::read_controls(path) {
        if !c.is_empty() {
            let list: Vec<String> =
                c.iter().map(|(n, (label, v))| format!("cc{n}{} = {v}", if label.is_empty() { String::new() } else { format!(" ({label})") })).collect();
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
    // —— muser 自己的打击乐（yinse）——
    b!("yinse.strike.luogu", ["luogu"], "drums", true, (36, 81), 48, "Chinese percussion (锣鼓) for opera, folk bands and festivals: big drum 大鼓, tanggu 堂鼓 and its rim, bangu 板鼓, clapper 拍板, big gong 大锣 (falls in pitch), small gong 小锣 (rises), cymbals 大钹 (and muted) and 小钹, wood blocks 木鱼, bangzi 梆子, bells 碰铃", || Patch::Kit(crate::percussion::KitParams::luogu())),
    b!("yinse.strike.drums", [], "drums", true, DRUM_RANGE, 24, "Acoustic drum kit resynthesized from sampled kits (each piece's measured resonances and noise): punchy kick, snare with wires, rimshot, side stick, clap, six toms, hi-hats, crash, splash, sizzle crash, ride and bell, flat ride, tambourine, cowbell, vibraslap (GM keys; Chinese cymbal (52) and percussion above 59 are the plain acoustic model); pop / funk / rock / jazz", || Patch::YinseDrums(yinse_drums())),
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
    b!("yinse.pluck.electric_guitar", [], "guitar", false, (40, 88), 8, "Electric guitar, humbucker on the bridge pickup: long sustain, thick and biting; made for distortion (rock / metal riffs, power chords, leads). Palm mute per note", || Patch::Electric(crate::electric::ElectricParams::humbucker_bridge(), 8)),
    b!("yinse.pluck.electric_guitar_clean", [], "guitar", false, (40, 88), 8, "Electric guitar, single coil on the neck pickup through a built-in clean amp: clean, glassy and warm; funk, pop, blues and jazz comping, arpeggios", || Patch::Electric(crate::electric::ElectricParams::single_coil_neck(), 8)),
    b!("bass.finger", [], "bass", false, (28, 60), 4, "Fingerstyle electric bass: round and full; the standard bass for pop / funk / R&B", || Patch::String(StringParams::finger_bass(), 4)),
    b!("bass.slap", [], "bass", false, (28, 60), 4, "Slap bass: percussive and punchy; funk / disco", || Patch::String(StringParams::slap_bass(), 4)),
    b!("bass.acid", ["acid"], "bass", false, (24, 60), 1, "TB-303 style acid bass (monophonic, overlapping notes glide); acid house / techno", || Patch::Acid(AcidParams::classic())),
    b!("bass.acid_squelch", [], "bass", false, (24, 60), 1, "acid bass with higher resonance and envelope, a stronger squelch", || Patch::Acid(AcidParams::squelch())),
    b!("bass.acid_square", [], "bass", false, (24, 60), 1, "acid bass, square-wave version, more hollow", || Patch::Acid(AcidParams::square())),
    // —— 歌声（muser 自己的，yinse）——
    b!("yinse.voice.soprano", [], "voice", false, (60, 84), 1, "Singing voice, soprano: sings each note's lyric (pinyin or English) with a formant voice; legato glides, vibrato, slides and breaths", || Patch::Singer(crate::singer::SingerParams::preset(0))),
    b!("yinse.voice.alto", [], "voice", false, (55, 79), 1, "Singing voice, alto: warmer and lower than the soprano", || Patch::Singer(crate::singer::SingerParams::preset(1))),
    b!("yinse.voice.tenor", [], "voice", false, (48, 72), 1, "Singing voice, tenor", || Patch::Singer(crate::singer::SingerParams::preset(2))),
    b!("yinse.voice.bass", [], "voice", false, (40, 64), 1, "Singing voice, bass: deep and dark", || Patch::Singer(crate::singer::SingerParams::preset(3))),
    b!("yinse.voice.child", [], "voice", false, (60, 81), 1, "Singing voice, child: small and bright", || Patch::Singer(crate::singer::SingerParams::preset(4))),
    // —— 中国民族乐器（muser 自己的，yinse）——
    b!("yinse.pluck.pipa", [], "chinese", false, (45, 88), 12, "Pipa (琵琶): pear-shaped lute plucked with picks, four strings A2 D3 E3 A3, crisp and bright with about a second of ring; tremolo (轮指, ~16 a second) with the note's `tremolo`, bends (推拉) on fretted notes with `bend`, harmonics with `technique`", || Patch::Zither(crate::zither::ZitherParams::pipa())),
    b!("yinse.pluck.guqin", [], "chinese", false, (36, 84), 12, "Guqin (古琴): partials measured from a solo recording, with distinct open (散音), stopped (按音) and harmonic (泛音) techniques, sympathetic strings; continuous stopped-string slides (走手音) with `bend`", || Patch::Zither(crate::zither::ZitherParams::default())),
    b!("yinse.pluck.guzheng", [], "chinese", false, (38, 88), 24, "Guzheng (古筝): picked strings with partials measured from a solo recording, sympathetic strings; 21 strings tuned to D pentatonic D2-D6; other pitches are pressed (按音) from the string below; every plucked or pressed note follows bends and the left-hand vibrato (揉弦); harmonics (泛音) with `technique`", || Patch::Zither(crate::zither::ZitherParams::guzheng())),
    b!("yinse.strike.yangqin", [], "chinese", false, (43, 93), 24, "Yangqin (扬琴): hammered dulcimer struck with soft bamboo hammers, a crisp attack and a long, undamped ring; each note a slightly detuned course of strings, the other strings ringing along, in stereo; tremolo (轮音) with the note's `tremolo`", || Patch::Zither(crate::zither::ZitherParams::yangqin())),
    b!("yinse.blow.bangdi", ["yinse.blow.dizi"], "chinese", false, (67, 96), 1, "Bangdi (梆笛), the northern dizi (笛子; also yinse.blow.dizi): high bamboo transverse flute with a buzzing membrane (笛膜), bright and lively, made for tongue techniques (吐音、剁音、花舌、滑音); one note at a time, overlapping notes slur (a note with `slide` glides)", || Patch::Dizi(crate::chinese::DiziParams::dizi())),
    b!("yinse.blow.qudi", [], "chinese", false, (64, 91), 1, "Qudi (曲笛): the southern bamboo flute, larger and lower than the dizi, round, mellow and breathier with a lighter membrane; made for finger ornaments (颤音、叠音、打音) and Kunqu / Jiangnan tunes; one note at a time, overlapping notes slur (a note with `slide` glides)", || Patch::Dizi(crate::chinese::DiziParams::qudi())),
    b!("yinse.blow.hulusi", [], "chinese", false, (67, 81), 1, "Hulusi (葫芦丝): the Dai free-reed gourd pipe of Yunnan, soft, round and reedy like a gentle clarinet, its long notes pulsing in the breath; a drone pipe (`drone`) can sound under the melody; one note at a time, overlapping notes slur (a note with `slide` glides)", || Patch::Dizi(crate::chinese::hulusi())),
    b!("yinse.blow.suona", [], "chinese", false, (67, 95), 1, "Suona (唢呐): the loud double-reed horn with a brass bell, bright and raspy (rounder low, sharper high), a wide vibrato; weddings, festivals, 百鸟朝凤; one note at a time, overlapping notes slur, `scoop` / `fall` slide in from below and drop off the end", || Patch::Dizi(crate::chinese::suona())),
    b!("yinse.blow.suona_big", [], "chinese", false, (55, 84), 1, "Big suona (大唢呐): longer, with a big bell, a fourth or fifth below the suona; thick, rough and full, more low-mids; northern folk bands, weddings and funerals, opera accompaniment", || Patch::Dizi(crate::chinese::suona_big())),
    b!("yinse.blow.haidi", [], "chinese", false, (79, 103), 1, "Haidi (海笛, the small suona): short, with a tiny reed, a fourth to an octave above the suona; shrill, thin and the brightest, for bird calls and ornate high lines", || Patch::Dizi(crate::chinese::haidi())),
    b!("yinse.blow.xiao", [], "chinese", false, (55, 86), 1, "Xiao (箫, 洞箫): end-blown bamboo flute, low, dark and nearly pure, a soft attack; one note at a time, overlapping notes slur (a note with `slide` glides)", || Patch::Dizi(crate::chinese::xiao())),
    b!("yinse.bow.erhu", [], "chinese", false, (62, 96), 1, "Erhu (二胡): two-string fiddle with a python-skin resonator, nasal and singing, wide vibrato; slurred notes (legato clips) slide into each other. One note at a time", || Patch::Erhu(crate::chinese::erhu())),
    // —— 手风琴（muser 自己的，yinse）——
    b!("yinse.blow.accordion", [], "accordion", false, (41, 93), 16, "Accordion, musette register: three middle reeds tuned apart, the shimmering French sound; chanson, waltz musette, folk", || Patch::Accordion(crate::accordion::AccordionParams::musette())),
    b!("yinse.blow.accordion_dry", [], "accordion", false, (41, 93), 16, "Accordion, master register with the reeds nearly in tune: full and dry; tango, folk, polka, accompaniment", || Patch::Accordion(crate::accordion::AccordionParams::dry())),
    b!("yinse.blow.bandoneon", [], "accordion", false, (36, 88), 16, "Bandoneon: low and middle reeds an octave apart, dark and reedy; the tango sound", || Patch::Accordion(crate::accordion::AccordionParams::bandoneon())),
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
    if let Some(path) = trimmed.strip_prefix("yinse:") {
        let (table, profile) = crate::modal::load(path).ok()?;
        let (lo, hi) = table.range();
        return Some((
            InstrumentInfo {
                name: trimmed.to_string(),
                family: "yinse".into(),
                notes: NoteConvention::Pitched,
                range: ((lo - 3.0).max(0.0).round() as u8, (hi + 3.0).min(127.0).round() as u8),
                polyphony: profile.voices(),
                description: format!(
                    "{} (yinse modal instrument extracted from {} recordings: {}); articulations: {}",
                    table.name,
                    table.notes.len(),
                    crate::modal::resolve(path).display(),
                    Some(table.articulations().join(", ")).filter(|a| !a.is_empty()).unwrap_or_else(|| "none".into())
                ),
                params: Vec::new(),
                params_note: String::new(),
            },
            Patch::Zither(crate::zither::ZitherParams::from_table(table, profile)),
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
    BUILTINS.iter().map(builtin_info).chain(poly::PRESETS.iter().map(preset_info)).collect()
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
            false => Err(InstrumentError::Param {
                detail: "an audio track has no instrument parameters (set volume with gain_db or the clip's audio.gain_db)".into(),
            }),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        if rest.trim().is_empty() {
            return Err(InstrumentError::PluginLoad { path: String::new(), detail: "missing plugin path after plugin:".into() });
        }
        let (path, id) = crate::plugin::split_spec(rest);
        return crate::plugin::check_params(path, id, params).map_err(|detail| InstrumentError::Param { detail });
    }
    if let Some(path) = trimmed.strip_prefix("yinse:") {
        crate::modal::load(path).map_err(|detail| InstrumentError::Load { detail })?;
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })?;
    if let Patch::Sfz(path, _) = &patch {
        if !path.is_file() {
            let hint = if path.is_relative() {
                "(relative paths are searched in the sound library directories of MUSER_SFZ_PATH and two levels of subdirectories)"
            } else {
                ""
            };
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

    fn find<P>(defs: &[params::ParamDef<P>], p: &P, param: &str, next: &[&str]) -> Result<(params::InstParamSpec, params::Automatable, f64), String> {
        let list = || defs.iter().filter(|d| params::automatable(&d.spec, next) != params::Automatable::No).map(|d| d.spec.name).collect::<Vec<_>>().join(", ");
        let d = defs.iter().find(|d| d.spec.name == param).ok_or_else(|| format!("unknown parameter '{param}', automatable: {}", list()))?;
        let a = params::automatable(&d.spec, next);
        if a == params::Automatable::No {
            return Err(format!("parameter '{param}' cannot be automated (switches, choices, integers); automatable: {}", list()));
        }
        let base = (d.get)(p).as_f64().ok_or_else(|| format!("parameter '{param}' has no effect on this sound (no filter enabled)"))?;
        Ok((d.spec, a, base))
    }

    match &patch {
        Patch::Poly(p) => find(params::POLY, p, param, &[]),
        Patch::Fm(p) => find(params::FM, p, param, params::NEXT_FM),
        Patch::String(p, _) => find(params::STRING, p, param, params::NEXT_STRING),
        Patch::Electric(p, _) => find(params::ELECTRIC, p, param, params::NEXT_ELECTRIC),
        Patch::Accordion(p) => find(params::ACCORDION, p, param, params::NEXT_ACCORDION),
        Patch::Zither(p) => find(params::ZITHER, p, param, params::NEXT_ZITHER),
        Patch::Dizi(p) => find(params::DIZI, p, param, params::NEXT_DIZI),
        Patch::Erhu(p) => find(params::BOWED, p, param, params::NEXT_BOWED),
        Patch::Singer(p) => find(params::SINGER, p, param, params::NEXT_SINGER),
        Patch::Acid(p) => find(params::ACID, p, param, &[]),
        Patch::Organ(p) => find(params::ORGAN, p, param, &[]),
        Patch::Mallet(p, _) => find(params::MALLET, p, param, params::NEXT_MALLET),
        Patch::Bowed(p, _) => find(params::BOWED, p, param, params::NEXT_BOWED),
        Patch::Wind(p, _) => find(params::WIND, p, param, &[]),
        Patch::Choir(p, _) => find(params::CHOIR, p, param, &[]),
        Patch::Kit(kit) => {
            let (piece, field) =
                param.split_once('.').ok_or_else(|| format!("percussion parameters are written as `piece.param`, e.g. \"daluo.tune\", got '{param}'"))?;
            let i = kit.index(piece).ok_or_else(|| format!("unknown piece '{piece}'"))?;
            find(params::KIT_PIECE, &kit.set[i], field, params::NEXT_KIT_PIECE)
        }
        Patch::Drums(kit) | Patch::YinseDrums(kit) => {
            let (piece, field) =
                param.split_once('.').ok_or_else(|| format!("drum kit parameters are written as `piece.param`, e.g. \"kick.tune\", got '{param}'"))?;
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
