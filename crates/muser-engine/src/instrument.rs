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
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth, Voice};
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
    fn note_off(&mut self, note: NoteRef, velocity: f32);
    fn process(&mut self, buffer: &mut AudioBuffer<f32>);
    /// 通道消息：延音踏板（CC64）、弯音轮。默认忽略（鼓组）。
    fn midi(&mut self, _message: MidiMessage) {}
    /// 自动化：运行中改一个参数（名称与 `instrument_params` 相同，鼓组为 `鼓件.参数`）。
    fn set_param(&mut self, _name: &str, _value: f64) {}
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
}

impl TrackInstrument {
    fn new(inner: impl Playable + 'static, channel: u8) -> Self {
        Self { inner: Box::new(inner), channel }
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
    }
    pub fn set_param(&mut self, name: &str, value: f64) {
        self.inner.set_param(name, value);
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
}

fn tunable<V: Voice<f32> + 'static, P: Copy + 'static>(
    synth: Synth<V, f32>,
    params: P,
    defs: &'static [params::ParamDef<P>],
    push: fn(&mut V, P),
) -> TrackInstrument {
    TrackInstrument::new(Tunable { synth, params, defs, push }, 0)
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
                            "未知鼓件 '{piece}'：用 GM 鼓键数字或别名（{}）",
                            params::PIECE_ALIASES.iter().map(|(a, _)| *a).collect::<Vec<_>>().join(" ")
                        )
                    })?;
                    let mut p = *kit.piece(key).ok_or_else(|| {
                        format!("这套鼓没有鼓键 {key}（{piece}），支持: {}", kit_keys(kit))
                    })?;
                    let obj = value
                        .as_object()
                        .ok_or_else(|| format!("{piece}: 应为对象，如 {{ \"tune\": -2 }}"))?;
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
                "完整音色（振荡器、滤波器、包络、LFO、调制矩阵）用 instrument_params 的 \"patch\" 写，结构见 `muser poly schema`；\
                 本机音色库的音色用 `muser poly show <名字>` 取出完整 JSON。".into(),
            ),
            Patch::Fm(p) => (params::describe(params::FM, p, params::NEXT_FM), String::new()),
            Patch::String(p, _) => (params::describe(params::STRING, p, params::NEXT_STRING), String::new()),
            Patch::Acid(p) => (
                params::describe(params::ACID, p, &[]),
                "单音乐器：前一个音还没结束就开始下一个音，会按 slide 时间滑过去".into(),
            ),
            Patch::Organ(p) => (params::describe(params::ORGAN, p, &[]), String::new()),
            Patch::Mallet(p, _) => (params::describe(params::MALLET, p, params::NEXT_MALLET), String::new()),
            Patch::Bowed(p, _) => (params::describe(params::BOWED, p, params::NEXT_BOWED), String::new()),
            Patch::Wind(p, _) => (params::describe(params::WIND, p, &[]), String::new()),
            Patch::Choir(p, _) => (params::describe(params::CHOIR, p, &[]), String::new()),
            Patch::Drums(kit) => (
                params::specs(params::PIECE, params::NEXT_PIECE),
                format!(
                    "按鼓件设置：{{ \"kick\": {{ \"tune\": -2, \"decay\": 1.5 }}, \"38\": {{ \"snap\": 0.8 }} }}。\
                     键可用 GM 鼓键数字或别名（kick snare clap closed_hat open_hat crash ride tom_low tom_mid tom_high …）。\
                     这套鼓支持的鼓键: {}",
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
    let base = "音色由 .sfz 文件定义。可用 instrument_params 的 \"cc\" 设控制器值（加载时生效），如 { \"cc\": { \"105\": 127 } }。";
    let mut out = base.to_string();
    if let Ok(c) = crate::sfz::read_controls(path) {
        if !c.is_empty() {
            let list: Vec<String> = c.iter().map(|(n, (label, v))| format!("cc{n}{} = {v}", if label.is_empty() { String::new() } else { format!("（{label}）") })).collect();
            out.push_str(&format!("这个音色声明的控制器（默认值）：{}。", list.join("，")));
        }
    }
    if let Ok((sw, def)) = crate::sfz::read_keyswitches(path) {
        if !sw.is_empty() {
            let list: Vec<String> = sw.iter().map(|(n, l)| if l.is_empty() { n.to_string() } else { format!("{n}（{l}）") }).collect();
            out.push_str(&format!(
                "键位切换奏法（instrument_params 的 \"keyswitch\" 选择，默认 {}）：{}。",
                def.map_or("第一个".to_string(), |d| d.to_string()),
                list.join("，")
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
            "[muser] sfz '{}': 采样器不支持（被忽略）的 opcode：{}。音色可能与原设计略有差异",
            path.display(),
            crate::sfz::summary(&loaded.warnings)
        );
    }
    let (sampler, loader) = Sampler::with_instrument(loaded.instrument.clone(), SFZ_VOICES);
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
        Processor::process(&mut self.sampler, buffer);
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
    b!("drums.808", ["808"], "drums", true, DRUM_RANGE, 24, "TR-808 风格：长尾音的低频底鼓、清脆拍手，hip-hop / trap / 电子", || Patch::Drums(DrumKit::style_808())),
    b!("drums.909", ["909"], "drums", true, DRUM_RANGE, 24, "TR-909 风格：有冲击力的底鼓与镲片，house / techno", || Patch::Drums(DrumKit::style_909())),
    b!("drums.606", ["606"], "drums", true, DRUM_RANGE, 24, "TR-606 风格：轻薄干脆，acid / electro", || Patch::Drums(DrumKit::style_606())),
    b!("drums.cr78", ["cr78"], "drums", true, DRUM_RANGE, 24, "CR-78 风格：柔和复古，含沙锤、牛铃、邦戈", || Patch::Drums(DrumKit::style_cr78())),
    b!("drums.acoustic", ["acoustic"], "drums", true, DRUM_RANGE, 24, "原声架子鼓（物理建模），pop / rock / jazz", || Patch::Drums(DrumKit::acoustic())),
    b!("drums.trap", ["trap"], "drums", true, DRUM_RANGE, 24, "trap 套鼓：重底鼓、清脆军鼓与拍手，适合高速闭镲连打", || Patch::Drums(DrumKit::trap())),
    b!("drums.techno", ["techno"], "drums", true, DRUM_RANGE, 24, "techno 套鼓：带隆隆尾音的底鼓、硬拍手", || Patch::Drums(DrumKit::techno())),
    b!("drums.house", ["house"], "drums", true, DRUM_RANGE, 24, "house 套鼓：四踩底鼓、拍手、沙锤、康加", || Patch::Drums(DrumKit::house())),
    b!("drums.dnb", ["dnb", "drums.drum_and_bass"], "drums", true, DRUM_RANGE, 24, "drum & bass 套鼓：紧实的军鼓与深底鼓", || Patch::Drums(DrumKit::drum_and_bass())),
    b!("drums.lofi", ["lofi", "drums.lo_fi"], "drums", true, DRUM_RANGE, 24, "lo-fi 套鼓：柔软、暗、带 12-bit 采样质感", || Patch::Drums(DrumKit::lo_fi())),
    // —— 通用合成器 ——
    b!("synth.poly", ["poly", "synth"], "synth", false, (36, 96), 16, "复音合成器的初始音色（较朴素）。想要具体音色请用 poly.* 预设", || Patch::Poly(PolySynthParams::default())),
    b!("synth.fm", ["fm"], "synth", false, (24, 96), 16, "四算子 FM 合成器默认音色，偏电子、带金属感，可做贝斯", || Patch::Fm(FmParams::default())),
    // —— 吉他 / 贝斯（物理建模拨弦）——
    b!("guitar.nylon", [], "guitar", false, (40, 84), 8, "尼龙弦古典吉他：温暖柔和，适合分解和弦、拉丁、民谣", || Patch::String(StringParams::nylon_guitar(), 8)),
    b!("guitar.steel", [], "guitar", false, (40, 84), 8, "钢弦民谣吉他：明亮清脆，适合扫弦与分解和弦", || Patch::String(StringParams::steel_guitar(), 8)),
    b!("bass.finger", [], "bass", false, (28, 60), 4, "指弹电贝斯：圆润饱满，pop / funk / R&B 的标准贝斯", || Patch::String(StringParams::finger_bass(), 4)),
    b!("bass.slap", [], "bass", false, (28, 60), 4, "击勾弦贝斯：有冲击力的打击感，funk / disco", || Patch::String(StringParams::slap_bass(), 4)),
    b!("bass.acid", ["acid"], "bass", false, (24, 60), 1, "TB-303 风格 acid 贝斯（单音，重叠音符会滑音），acid house / techno", || Patch::Acid(AcidParams::classic())),
    b!("bass.acid_squelch", [], "bass", false, (24, 60), 1, "acid 贝斯，更高谐振与包络，'吱吱'声更强烈", || Patch::Acid(AcidParams::squelch())),
    b!("bass.acid_square", [], "bass", false, (24, 60), 1, "acid 贝斯，方波版本，更空心", || Patch::Acid(AcidParams::square())),
    // —— 风琴 ——
    b!("organ.jazz", [], "organ", false, (36, 96), 16, "拉杆风琴（Hammond 风格）爵士音色：柔和带打击泛音", || Patch::Organ(OrganParams::jazz())),
    b!("organ.gospel", [], "organ", false, (36, 96), 16, "福音风琴：饱满明亮，适合和弦铺底", || Patch::Organ(OrganParams::gospel())),
    b!("organ.full", [], "organ", false, (36, 96), 16, "全拉杆风琴：最饱满响亮，摇滚风琴", || Patch::Organ(OrganParams::full())),
    b!("organ.ballad", [], "organ", false, (36, 96), 16, "抒情风琴：柔和圆润，适合慢歌铺底", || Patch::Organ(OrganParams::ballad())),
    // —— 打击旋律乐器（模态合成）——
    b!("mallet.marimba", ["marimba"], "mallet", false, (45, 96), 12, "马林巴：温暖木质，适合旋律与琶音", || Patch::Mallet(MalletParams::marimba(), 12)),
    b!("mallet.xylophone", ["xylophone"], "mallet", false, (65, 108), 12, "木琴：明亮短促", || Patch::Mallet(MalletParams::xylophone(), 12)),
    b!("mallet.vibraphone", ["vibraphone"], "mallet", false, (53, 89), 12, "颤音琴：金属、长余音，爵士与 lo-fi 常用", || Patch::Mallet(MalletParams::vibraphone(), 12)),
    b!("mallet.glockenspiel", ["glockenspiel"], "mallet", false, (79, 108), 12, "钟琴：高而清亮的铃声", || Patch::Mallet(MalletParams::glockenspiel(), 12)),
    b!("mallet.celesta", ["celesta"], "mallet", false, (60, 108), 12, "钢片琴：柔和的铃音，梦幻感", || Patch::Mallet(MalletParams::celesta(), 12)),
    b!("mallet.kalimba", ["kalimba"], "mallet", false, (60, 88), 12, "卡林巴（拇指琴）：清脆温柔", || Patch::Mallet(MalletParams::kalimba(), 12)),
    b!("mallet.steel_pan", ["steel_pan"], "mallet", false, (60, 84), 12, "钢鼓：加勒比风情的金属音色", || Patch::Mallet(MalletParams::steel_pan(), 12)),
    b!("mallet.tubular_bells", ["tubular_bells"], "mallet", false, (60, 77), 8, "管钟：庄严的长余音钟声", || Patch::Mallet(MalletParams::tubular_bells(), 8)),
    // —— 弦乐（弓弦波导模型）——
    b!("strings.violin", ["violin"], "strings", false, (55, 100), 8, "小提琴（拉奏）：旋律主奏", || Patch::Bowed(BowedParams::violin(), 8)),
    b!("strings.viola", ["viola"], "strings", false, (48, 88), 8, "中提琴（拉奏）：比小提琴更暗更厚", || Patch::Bowed(BowedParams::viola(), 8)),
    b!("strings.cello", ["cello"], "strings", false, (36, 76), 8, "大提琴（拉奏）：温暖深沉", || Patch::Bowed(BowedParams::cello(), 8)),
    b!("strings.contrabass", ["contrabass"], "strings", false, (28, 60), 8, "低音提琴（拉奏）：弦乐低音声部", || Patch::Bowed(BowedParams::contrabass(), 8)),
    b!("strings.ensemble", ["strings"], "strings", false, (36, 96), 8, "弦乐组：多把琴的合奏，适合和弦铺底与抒情段落", || Patch::Bowed(BowedParams::string_ensemble(), 8)),
    b!("strings.violin_pizz", [], "strings", false, (55, 100), 8, "小提琴拨奏：短促跳跃", || Patch::Bowed(BowedParams::violin_pizzicato(), 8)),
    b!("strings.cello_pizz", [], "strings", false, (36, 76), 8, "大提琴拨奏", || Patch::Bowed(BowedParams::cello_pizzicato(), 8)),
    b!("strings.contrabass_pizz", [], "strings", false, (28, 60), 8, "低音提琴拨奏：爵士 walking bass", || Patch::Bowed(BowedParams::contrabass_pizzicato(), 8)),
    // —— 管乐（波导模型）——
    b!("wind.flute", ["flute"], "wind", false, (60, 96), 4, "长笛：轻盈透亮", || Patch::Wind(WindParams::flute(), 4)),
    b!("wind.clarinet", ["clarinet"], "wind", false, (50, 91), 4, "单簧管：圆润木质", || Patch::Wind(WindParams::clarinet(), 4)),
    b!("wind.sax", ["sax", "wind.saxophone"], "wind", false, (49, 81), 4, "萨克斯：爵士/流行的主奏", || Patch::Wind(WindParams::saxophone(), 4)),
    b!("wind.trumpet", ["trumpet"], "wind", false, (54, 82), 4, "小号：明亮有穿透力", || Patch::Wind(WindParams::trumpet(), 4)),
    b!("wind.trombone", ["trombone"], "wind", false, (40, 72), 4, "长号：浑厚的铜管中低音", || Patch::Wind(WindParams::trombone(), 4)),
    // —— 人声（共振峰合成）——
    b!("choir.aah", ["choir"], "choir", false, (48, 79), 8, "合唱 'aah'：开阔明亮，适合和弦铺底", || Patch::Choir(ChoirParams::aah_choir(), 8)),
    b!("choir.ooh", [], "choir", false, (48, 79), 8, "合唱 'ooh'：柔和暗淡", || Patch::Choir(ChoirParams::ooh_choir(), 8)),
    b!("choir.solo", ["voice"], "choir", false, (48, 79), 2, "独唱人声（元音）", || Patch::Choir(ChoirParams::solo_voice(), 2)),
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
                description: format!("SFZ 采样器：{}", path.display()),
                params: Vec::new(),
                params_note: sfz_controls_note(&path),
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
pub fn check(name: &str, params: &InstrumentParams) -> Result<(), InstrumentError> {
    let trimmed = name.trim();
    // 音频音轨：没有乐器，声音来自 clip 上的音频文件（crate::audio）。
    if trimmed == muser_core::audio::INSTRUMENT {
        return match params.is_empty() {
            true => Ok(()),
            false => Err(InstrumentError::Param { detail: "audio 音轨没有乐器参数（音量用 gain_db 或 clip 的 audio.gain_db）".into() }),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        if rest.trim().is_empty() {
            return Err(InstrumentError::PluginLoad { path: String::new(), detail: "plugin: 后缺少插件路径".into() });
        }
        let (path, id) = crate::plugin::split_spec(rest);
        return crate::plugin::check_params(path, id, params).map_err(|detail| InstrumentError::Param { detail });
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })?;
    if let Patch::Sfz(path, _) = &patch {
        if !path.is_file() {
            let hint = if path.is_relative() { "（相对路径会在 MUSER_SFZ_PATH 的音色库目录及其两层子目录里查找）" } else { "" };
            return Err(InstrumentError::Load { detail: format!("找不到 SFZ 文件 '{}'{hint}", path.display()) });
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
        return Err("audio 音轨没有乐器参数可自动化（音量用 target gain_db）".into());
    }
    if trimmed.starts_with("sfz:") {
        return Err("sfz 乐器的参数自动化尚未支持".into());
    }
    if trimmed.starts_with("plugin:") {
        return Err("内部错误：插件参数自动化应走 automate::plugin_rule".into());
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| format!("未知乐器 '{name}'"))?;
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
            .ok_or_else(|| format!("未知参数 '{param}'，可自动化: {}", list()))?;
        let a = params::automatable(&d.spec, next);
        if a == params::Automatable::No {
            return Err(format!("参数 '{param}' 不能自动化（开关、选项、整数类），可自动化: {}", list()));
        }
        let base = (d.get)(p)
            .as_f64()
            .ok_or_else(|| format!("参数 '{param}' 对该音色无效（没有启用滤波器）"))?;
        Ok((d.spec, a, base))
    }

    match &patch {
        Patch::Poly(p) => find(params::POLY, p, param, &[]),
        Patch::Fm(p) => find(params::FM, p, param, params::NEXT_FM),
        Patch::String(p, _) => find(params::STRING, p, param, params::NEXT_STRING),
        Patch::Acid(p) => find(params::ACID, p, param, &[]),
        Patch::Organ(p) => find(params::ORGAN, p, param, &[]),
        Patch::Mallet(p, _) => find(params::MALLET, p, param, params::NEXT_MALLET),
        Patch::Bowed(p, _) => find(params::BOWED, p, param, params::NEXT_BOWED),
        Patch::Wind(p, _) => find(params::WIND, p, param, &[]),
        Patch::Choir(p, _) => find(params::CHOIR, p, param, &[]),
        Patch::Drums(kit) => {
            let (piece, field) = param
                .split_once('.')
                .ok_or_else(|| format!("鼓组参数写成 `鼓件.参数`，如 \"kick.tune\"，实际 '{param}'"))?;
            let key = params::piece_key(piece).ok_or_else(|| format!("未知鼓件 '{piece}'"))?;
            let p = kit.piece(key).ok_or_else(|| format!("这套鼓没有鼓键 {key}（{piece}）"))?;
            find(params::PIECE, p, field, params::NEXT_PIECE)
        }
        Patch::Sfz(..) => Err("sfz 乐器的参数自动化尚未支持".into()),
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
    #[error("未知乐器 '{name}'。用 `muser instruments` 查看全部；常用：drums.808 / drums.acoustic / synth.poly / poly.moog_bass / guitar.nylon / strings.ensemble / mallet.marimba；另可用 sfz:/path/to.sfz")]
    Unknown { name: String },
    #[error("instrument_params 无效: {detail}（内置乐器用 `muser instruments <名称>`，插件用 `muser plugin params <插件>` 查看可调参数）")]
    Param { detail: String },
    #[error("{detail}")]
    Load { detail: String },
    #[error("插件乐器不可用：这个 muser 构建没有启用插件宿主（`host` 特性）；发行版与默认构建都带，别用 --no-default-features 去掉它")]
    HostDisabled,
    #[error("加载插件乐器 '{path}' 失败: {detail}")]
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
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        return resolve_plugin(rest, params, spec);
    }
    let (_, mut patch) = lookup(trimmed).ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })?;
    patch.apply(params).map_err(|detail| InstrumentError::Param { detail })?;
    patch.instantiate().map_err(|detail| InstrumentError::Load { detail })
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
    processor: Box<dyn Processor<f32> + Send>,
    opened: crate::plugin::Opened,
    pending: manguebyte::prelude::EventBuffer,
    transport: manguebyte::prelude::Transport,
    sample_rate: f64,
    blocks: u32,
}

#[cfg(feature = "host")]
impl Playable for PluginInstrument {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor.prepare(spec);
        if let Err(w) = self.opened.settle(self.processor.as_mut(), spec) {
            eprintln!("[muser] 警告: {w}");
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
        let ctx = ProcessContext::new(&self.pending, &self.transport);
        self.processor.process_with(buffer, &ctx);
        self.pending.clear();
        self.transport.advance(buffer.num_frames(), self.sample_rate);
        self.blocks = self.blocks.wrapping_add(1);
        if self.blocks % 16 == 0 {
            let _ = self.opened.plugin.pump();
        }
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
    let processor = opened.activate(ps.sample_rate, ps.max_block_size).map_err(load_err)?;
    Ok(TrackInstrument::new(
        PluginInstrument {
            processor,
            opened,
            pending: manguebyte::prelude::EventBuffer::with_capacity(256),
            transport: crate::transport::start(),
            sample_rate: ps.sample_rate,
            blocks: 0,
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
    fn catalog_has_all_presets_and_unique_names() {
        let cat = catalog();
        assert_eq!(cat.iter().filter(|i| i.family.starts_with("poly/")).count(), 68);
        let mut names: Vec<_> = cat.iter().map(|i| i.name.clone()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "乐器名必须唯一");
    }

    #[test]
    fn every_catalog_entry_resolves() {
        for info in catalog() {
            assert!(resolve(&info.name).is_ok(), "{} 应能解析", info.name);
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
        assert!(check_name("plugin:/x.vst3").unwrap_err().to_string().contains("找不到插件"));
        assert!(check_name("plugin:").is_err());
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Moog Bass"), "moog_bass");
        assert_eq!(slug("FM Electric Piano"), "fm_electric_piano");
    }
}
