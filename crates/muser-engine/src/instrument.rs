//! 乐器注册表与描述表。
//!
//! 音轨的 `instrument` 字段有三种写法：
//! - 内置乐器名，如 `"drums.808"`、`"guitar.nylon"`、`"strings.cello"`；
//! - poly 合成器预设，如 `"poly.moog_bass"`、`"poly.warm_analog_pad"`（共 68 个）；
//! - `"plugin:/path/to.vst3"`（可带 `#插件id`）加载第三方 VSTi/CLAP 乐器（需 `host` 特性）。
//!
//! 每个内置乐器的名称、类别、音高约定、推荐音域、复音数、说明都写在 [`catalog`] 里，
//! CLI (`muser instruments`)、MCP (`list_instruments` / `describe_instrument`) 与
//! 校验都从这里取数据。

use manguebyte::dsp::instruments::{
    poly, AcidParams, AcidVoice, BowedParams, BowedString, ChoirParams, ChoirVoice, DrumKit,
    DrumKitVoice, FmParams, FmVoice, MalletParams, MalletVoice, OrganParams, OrganVoice,
    PluckedString, PolySynthParams, PolySynthVoice, StringParams, WindParams, WindVoice,
};
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth, Voice};
use serde::Serialize;

// ---------------------------------------------------------------------------
// 统一的可演奏接口
// ---------------------------------------------------------------------------

/// 任何能接收音符并输出音频的东西。
trait Playable {
    fn prepare(&mut self, spec: &ProcessSpec);
    fn note_on(&mut self, note: NoteRef, velocity: f32);
    fn note_off(&mut self, note: NoteRef, velocity: f32);
    fn process(&mut self, buffer: &mut AudioBuffer<f32>);
}

impl<V: Voice<f32>> Playable for Synth<V, f32> {
    fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(self, spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        Synth::note_on(self, note, velocity);
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
    pub fn note_on(&mut self, note: NoteRef, velocity: f32) {
        self.inner.note_on(note, velocity);
    }
    pub fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.inner.note_off(note, velocity);
    }
    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.inner.process(buffer);
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
}

/// 内置（非 poly 预设）乐器条目。
struct Builtin {
    name: &'static str,
    aliases: &'static [&'static str],
    family: &'static str,
    drums: bool,
    range: (u8, u8),
    polyphony: usize,
    description: &'static str,
    build: fn() -> TrackInstrument,
}

const DRUM_RANGE: (u8, u8) = (35, 81);

fn drums(kit: DrumKit) -> TrackInstrument {
    TrackInstrument::new(Synth::new(DrumKitVoice::voices(24, kit)), 9)
}
fn pitched<V: Voice<f32> + 'static>(count: usize, make: impl FnMut() -> V) -> TrackInstrument {
    TrackInstrument::new(Synth::with_voices(count, make), 0)
}
fn mono<V: Voice<f32> + 'static>(make: impl FnMut() -> V) -> TrackInstrument {
    let mut s = Synth::with_voices(1, make);
    s.set_legato(true);
    TrackInstrument::new(s, 0)
}

macro_rules! b {
    ($name:expr, [$($alias:expr),*], $family:expr, $drums:expr, $range:expr, $poly:expr, $desc:expr, $build:expr) => {
        Builtin {
            name: $name,
            aliases: &[$($alias),*],
            family: $family,
            drums: $drums,
            range: $range,
            polyphony: $poly,
            description: $desc,
            build: $build,
        }
    };
}

static BUILTINS: &[Builtin] = &[
    // —— 鼓组 ——
    b!("drums.808", ["808"], "drums", true, DRUM_RANGE, 24, "TR-808 风格：长尾音的低频底鼓、清脆拍手，hip-hop / trap / 电子", || drums(DrumKit::style_808())),
    b!("drums.909", ["909"], "drums", true, DRUM_RANGE, 24, "TR-909 风格：有冲击力的底鼓与镲片，house / techno", || drums(DrumKit::style_909())),
    b!("drums.606", ["606"], "drums", true, DRUM_RANGE, 24, "TR-606 风格：轻薄干脆，acid / electro", || drums(DrumKit::style_606())),
    b!("drums.cr78", ["cr78"], "drums", true, DRUM_RANGE, 24, "CR-78 风格：柔和复古，含沙锤、牛铃、邦戈", || drums(DrumKit::style_cr78())),
    b!("drums.acoustic", ["acoustic"], "drums", true, DRUM_RANGE, 24, "原声架子鼓（物理建模），pop / rock / jazz", || drums(DrumKit::acoustic())),
    b!("drums.trap", ["trap"], "drums", true, DRUM_RANGE, 24, "trap 套鼓：重底鼓、清脆军鼓与拍手，适合高速闭镲连打", || drums(DrumKit::trap())),
    b!("drums.techno", ["techno"], "drums", true, DRUM_RANGE, 24, "techno 套鼓：带隆隆尾音的底鼓、硬拍手", || drums(DrumKit::techno())),
    b!("drums.house", ["house"], "drums", true, DRUM_RANGE, 24, "house 套鼓：四踩底鼓、拍手、沙锤、康加", || drums(DrumKit::house())),
    b!("drums.dnb", ["dnb", "drums.drum_and_bass"], "drums", true, DRUM_RANGE, 24, "drum & bass 套鼓：紧实的军鼓与深底鼓", || drums(DrumKit::drum_and_bass())),
    b!("drums.lofi", ["lofi", "drums.lo_fi"], "drums", true, DRUM_RANGE, 24, "lo-fi 套鼓：柔软、暗、带 12-bit 采样质感", || drums(DrumKit::lo_fi())),
    // —— 通用合成器 ——
    b!("synth.poly", ["poly", "synth"], "synth", false, (36, 96), 16, "复音合成器的初始音色（较朴素）。想要具体音色请用 poly.* 预设", || pitched(16, || PolySynthVoice::new(PolySynthParams::default()))),
    b!("synth.fm", ["fm"], "synth", false, (24, 96), 16, "四算子 FM 合成器默认音色，偏电子、带金属感，可做贝斯", || pitched(16, || FmVoice::new(FmParams::default()))),
    // —— 吉他 / 贝斯（物理建模拨弦）——
    b!("guitar.nylon", [], "guitar", false, (40, 84), 8, "尼龙弦古典吉他：温暖柔和，适合分解和弦、拉丁、民谣", || pitched(8, || PluckedString::new(StringParams::nylon_guitar()))),
    b!("guitar.steel", [], "guitar", false, (40, 84), 8, "钢弦民谣吉他：明亮清脆，适合扫弦与分解和弦", || pitched(8, || PluckedString::new(StringParams::steel_guitar()))),
    b!("bass.finger", [], "bass", false, (28, 60), 4, "指弹电贝斯：圆润饱满，pop / funk / R&B 的标准贝斯", || pitched(4, || PluckedString::new(StringParams::finger_bass()))),
    b!("bass.slap", [], "bass", false, (28, 60), 4, "击勾弦贝斯：有冲击力的打击感，funk / disco", || pitched(4, || PluckedString::new(StringParams::slap_bass()))),
    b!("bass.acid", ["acid"], "bass", false, (24, 60), 1, "TB-303 风格 acid 贝斯（单音，重叠音符会滑音），acid house / techno", || mono(|| AcidVoice::new(AcidParams::classic()))),
    b!("bass.acid_squelch", [], "bass", false, (24, 60), 1, "acid 贝斯，更高谐振与包络，'吱吱'声更强烈", || mono(|| AcidVoice::new(AcidParams::squelch()))),
    b!("bass.acid_square", [], "bass", false, (24, 60), 1, "acid 贝斯，方波版本，更空心", || mono(|| AcidVoice::new(AcidParams::square()))),
    // —— 风琴 ——
    b!("organ.jazz", [], "organ", false, (36, 96), 16, "拉杆风琴（Hammond 风格）爵士音色：柔和带打击泛音", || TrackInstrument::new(Synth::new(OrganVoice::voices(16, OrganParams::jazz())), 0)),
    b!("organ.gospel", [], "organ", false, (36, 96), 16, "福音风琴：饱满明亮，适合和弦铺底", || TrackInstrument::new(Synth::new(OrganVoice::voices(16, OrganParams::gospel())), 0)),
    b!("organ.full", [], "organ", false, (36, 96), 16, "全拉杆风琴：最饱满响亮，摇滚风琴", || TrackInstrument::new(Synth::new(OrganVoice::voices(16, OrganParams::full())), 0)),
    b!("organ.ballad", [], "organ", false, (36, 96), 16, "抒情风琴：柔和圆润，适合慢歌铺底", || TrackInstrument::new(Synth::new(OrganVoice::voices(16, OrganParams::ballad())), 0)),
    // —— 打击旋律乐器（模态合成）——
    b!("mallet.marimba", ["marimba"], "mallet", false, (45, 96), 12, "马林巴：温暖木质，适合旋律与琶音", || pitched(12, || MalletVoice::new(MalletParams::marimba()))),
    b!("mallet.xylophone", ["xylophone"], "mallet", false, (65, 108), 12, "木琴：明亮短促", || pitched(12, || MalletVoice::new(MalletParams::xylophone()))),
    b!("mallet.vibraphone", ["vibraphone"], "mallet", false, (53, 89), 12, "颤音琴：金属、长余音，爵士与 lo-fi 常用", || pitched(12, || MalletVoice::new(MalletParams::vibraphone()))),
    b!("mallet.glockenspiel", ["glockenspiel"], "mallet", false, (79, 108), 12, "钟琴：高而清亮的铃声", || pitched(12, || MalletVoice::new(MalletParams::glockenspiel()))),
    b!("mallet.celesta", ["celesta"], "mallet", false, (60, 108), 12, "钢片琴：柔和的铃音，梦幻感", || pitched(12, || MalletVoice::new(MalletParams::celesta()))),
    b!("mallet.kalimba", ["kalimba"], "mallet", false, (60, 88), 12, "卡林巴（拇指琴）：清脆温柔", || pitched(12, || MalletVoice::new(MalletParams::kalimba()))),
    b!("mallet.steel_pan", ["steel_pan"], "mallet", false, (60, 84), 12, "钢鼓：加勒比风情的金属音色", || pitched(12, || MalletVoice::new(MalletParams::steel_pan()))),
    b!("mallet.tubular_bells", ["tubular_bells"], "mallet", false, (60, 77), 8, "管钟：庄严的长余音钟声", || pitched(8, || MalletVoice::new(MalletParams::tubular_bells()))),
    // —— 弦乐（弓弦波导模型）——
    b!("strings.violin", ["violin"], "strings", false, (55, 100), 8, "小提琴（拉奏）：旋律主奏", || pitched(8, || BowedString::new(BowedParams::violin()))),
    b!("strings.viola", ["viola"], "strings", false, (48, 88), 8, "中提琴（拉奏）：比小提琴更暗更厚", || pitched(8, || BowedString::new(BowedParams::viola()))),
    b!("strings.cello", ["cello"], "strings", false, (36, 76), 8, "大提琴（拉奏）：温暖深沉", || pitched(8, || BowedString::new(BowedParams::cello()))),
    b!("strings.contrabass", ["contrabass"], "strings", false, (28, 60), 8, "低音提琴（拉奏）：弦乐低音声部", || pitched(8, || BowedString::new(BowedParams::contrabass()))),
    b!("strings.ensemble", ["strings"], "strings", false, (36, 96), 8, "弦乐组：多把琴的合奏，适合和弦铺底与抒情段落", || pitched(8, || BowedString::new(BowedParams::string_ensemble()))),
    b!("strings.violin_pizz", [], "strings", false, (55, 100), 8, "小提琴拨奏：短促跳跃", || pitched(8, || BowedString::new(BowedParams::violin_pizzicato()))),
    b!("strings.cello_pizz", [], "strings", false, (36, 76), 8, "大提琴拨奏", || pitched(8, || BowedString::new(BowedParams::cello_pizzicato()))),
    b!("strings.contrabass_pizz", [], "strings", false, (28, 60), 8, "低音提琴拨奏：爵士 walking bass", || pitched(8, || BowedString::new(BowedParams::contrabass_pizzicato()))),
    // —— 管乐（波导模型）——
    b!("wind.flute", ["flute"], "wind", false, (60, 96), 4, "长笛：轻盈透亮", || pitched(4, || WindVoice::new(WindParams::flute()))),
    b!("wind.clarinet", ["clarinet"], "wind", false, (50, 91), 4, "单簧管：圆润木质", || pitched(4, || WindVoice::new(WindParams::clarinet()))),
    b!("wind.sax", ["sax", "wind.saxophone"], "wind", false, (49, 81), 4, "萨克斯：爵士/流行的主奏", || pitched(4, || WindVoice::new(WindParams::saxophone()))),
    b!("wind.trumpet", ["trumpet"], "wind", false, (54, 82), 4, "小号：明亮有穿透力", || pitched(4, || WindVoice::new(WindParams::trumpet()))),
    b!("wind.trombone", ["trombone"], "wind", false, (40, 72), 4, "长号：浑厚的铜管中低音", || pitched(4, || WindVoice::new(WindParams::trombone()))),
    // —— 人声（共振峰合成）——
    b!("choir.aah", ["choir"], "choir", false, (48, 79), 8, "合唱 'aah'：开阔明亮，适合和弦铺底", || pitched(8, || ChoirVoice::new(ChoirParams::aah_choir()))),
    b!("choir.ooh", [], "choir", false, (48, 79), 8, "合唱 'ooh'：柔和暗淡", || pitched(8, || ChoirVoice::new(ChoirParams::ooh_choir()))),
    b!("choir.solo", ["voice"], "choir", false, (48, 79), 2, "独唱人声（元音）", || pitched(2, || ChoirVoice::new(ChoirParams::solo_voice()))),
];

/// 把 poly 预设名转成 `poly.` 后缀：小写，非字母数字变 `_`。
fn slug(name: &str) -> String {
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

/// 所有内置乐器（含 68 个 poly 预设）的描述。
pub fn catalog() -> Vec<InstrumentInfo> {
    let mut out: Vec<InstrumentInfo> = BUILTINS
        .iter()
        .map(|b| InstrumentInfo {
            name: b.name.to_string(),
            family: b.family.to_string(),
            notes: if b.drums { NoteConvention::GmDrums } else { NoteConvention::Pitched },
            range: b.range,
            polyphony: b.polyphony,
            description: b.description.to_string(),
        })
        .collect();
    for preset in poly::PRESETS.iter() {
        out.push(InstrumentInfo {
            name: format!("poly.{}", slug(preset.name)),
            family: format!("poly/{}", preset.category.name()),
            notes: NoteConvention::Pitched,
            range: poly_range(preset.category),
            polyphony: 16,
            description: format!("{}：{}", preset.name, preset.description),
        });
    }
    out
}

/// 按名称查描述（大小写不敏感，支持别名）。插件乐器返回 `None`。
pub fn describe(name: &str) -> Option<InstrumentInfo> {
    let key = name.trim().to_ascii_lowercase();
    let canonical = BUILTINS
        .iter()
        .find(|b| b.name == key || b.aliases.contains(&key.as_str()))
        .map(|b| b.name.to_string())
        .unwrap_or(key);
    catalog().into_iter().find(|i| i.name == canonical)
}

/// 所有可用的内置乐器名（用于错误提示与文档）。插件乐器用 `plugin:路径`。
pub fn available() -> Vec<String> {
    catalog().into_iter().map(|i| i.name).collect()
}

/// 不加载任何东西，仅检查名称是否可解析（`plugin:` 只检查语法）。
pub fn check_name(name: &str) -> Result<(), InstrumentError> {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        if rest.trim().is_empty() {
            return Err(InstrumentError::PluginLoad {
                path: String::new(),
                detail: "plugin: 后缺少插件路径".into(),
            });
        }
        return Ok(());
    }
    describe(trimmed)
        .map(|_| ())
        .ok_or_else(|| InstrumentError::Unknown { name: name.to_string() })
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

/// 解析失败。
#[derive(Debug, thiserror::Error)]
pub enum InstrumentError {
    #[error("未知乐器 '{name}'。用 `muser instruments` 查看全部；常用：drums.808 / drums.acoustic / synth.poly / poly.moog_bass / guitar.nylon / strings.ensemble / mallet.marimba")]
    Unknown { name: String },
    #[error("插件乐器需要 `host` 特性：请用 `--features host` 构建")]
    HostDisabled,
    #[error("加载插件乐器 '{path}' 失败: {detail}")]
    PluginLoad { path: String, detail: String },
}

/// 兼容旧名。
pub type UnknownInstrument = InstrumentError;

/// 把乐器名解析为就绪的乐器。大小写不敏感。
pub fn resolve(name: &str) -> Result<TrackInstrument, InstrumentError> {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        return resolve_plugin(rest);
    }
    let key = trimmed.to_ascii_lowercase();
    if let Some(b) = BUILTINS.iter().find(|b| b.name == key || b.aliases.contains(&key.as_str())) {
        return Ok((b.build)());
    }
    if let Some(suffix) = key.strip_prefix("poly.") {
        if let Some(preset) = poly::PRESETS.iter().find(|p| slug(p.name) == suffix) {
            let params = preset.params;
            return Ok(pitched(16, move || PolySynthVoice::new(params)));
        }
    }
    Err(InstrumentError::Unknown { name: name.to_string() })
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
    processor: Box<dyn Processor<f32> + Send>,
    _plugin: manguebyte::host::any::Plugin,
    pending: manguebyte::prelude::EventBuffer,
    transport: manguebyte::prelude::Transport,
}

#[cfg(feature = "host")]
impl Playable for PluginInstrument {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor.prepare(spec);
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event { offset: 0, kind: EventKind::NoteOn { note, velocity } });
    }
    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event { offset: 0, kind: EventKind::NoteOff { note, velocity } });
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        use manguebyte::prelude::ProcessContext;
        let ctx = ProcessContext::new(&self.pending, &self.transport);
        self.processor.process_with(buffer, &ctx);
        self.pending.clear();
    }
}

/// 解析 `plugin:` 乐器。`spec`：`/path/to.vst3` 或 `/path.vst3#插件id`。
#[cfg(feature = "host")]
fn resolve_plugin(spec: &str) -> Result<TrackInstrument, InstrumentError> {
    use manguebyte::host::any::Library;

    let (path, id) = match spec.rsplit_once('#') {
        Some((p, i)) => (p.trim(), Some(i.trim())),
        None => (spec.trim(), None),
    };
    let load_err = |detail: String| InstrumentError::PluginLoad { path: path.to_string(), detail };

    // 加载会执行第三方代码，调用方须保证路径可信。
    let library = unsafe { Library::load(path) }.map_err(|e| load_err(format!("{e:?}")))?;
    let plugin_id = match id {
        Some(s) => s.to_string(),
        None => library
            .plugins()
            .first()
            .map(|p| p.id.clone())
            .ok_or_else(|| load_err("库中没有任何插件".into()))?,
    };
    let mut plugin = library
        .create(&plugin_id)
        .map_err(|e| load_err(format!("create('{plugin_id}') 失败: {e:?}")))?;
    let processor = plugin
        .activate(48_000.0, 512)
        .map_err(|e| load_err(format!("activate 失败: {e:?}")))?;

    Ok(TrackInstrument::new(
        PluginInstrument {
            processor,
            _plugin: plugin,
            pending: manguebyte::prelude::EventBuffer::with_capacity(256),
            transport: manguebyte::prelude::Transport::STOPPED,
        },
        0,
    ))
}

#[cfg(not(feature = "host"))]
fn resolve_plugin(_spec: &str) -> Result<TrackInstrument, InstrumentError> {
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
        assert!(check_name("plugin:/x.vst3").is_ok());
    }

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Moog Bass"), "moog_bass");
        assert_eq!(slug("FM Electric Piano"), "fm_electric_piano");
    }
}
