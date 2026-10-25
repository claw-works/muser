//! 乐器注册表：把 core 模型里的语义乐器名解析为具体的 manguebyte 乐器。
//!
//! AI 在 Project 里写 `"drums.808"` / `"synth.poly"` / `"fm"` 这类内置名，
//! 或 `"plugin:/path/to.vst3"`（可带 `#插件id`）加载第三方 **VSTi/CLAP 乐器**。
//! 这里负责把它们变成一个可渲染的 [`TrackInstrument`]。
//!
//! 内置乐器的 Voice 是不同 Rust 类型，用枚举做类型擦除；插件乐器（`host` 特性下）
//! 持有宿主激活的 processor，音符通过事件（`process_with` + `ProcessContext`）喂入。

use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth};

use manguebyte::dsp::instruments::{
    DrumKit, DrumKitVoice, FmParams, FmVoice, PolySynthParams, PolySynthVoice,
};

/// 一件已就绪、可渲染的音轨乐器。
pub enum TrackInstrument {
    /// 鼓组（打击乐），音符走 MIDI 通道 9，key = GM 鼓键。
    Drums(Synth<DrumKitVoice>),
    /// 复音合成器。
    Poly(Synth<PolySynthVoice>),
    /// FM 合成器。
    Fm(Synth<FmVoice>),
    /// 第三方插件乐器（VSTi/CLAP），仅 `host` 特性下可用。
    #[cfg(feature = "host")]
    Plugin(PluginInstrument),
}

impl TrackInstrument {
    /// 该乐器约定使用的 MIDI 通道（鼓组按惯例用 9）。
    pub fn channel(&self) -> u8 {
        match self {
            TrackInstrument::Drums(_) => 9,
            _ => 0,
        }
    }

    pub fn prepare(&mut self, spec: &ProcessSpec) {
        match self {
            TrackInstrument::Drums(s) => s.prepare(spec),
            TrackInstrument::Poly(s) => s.prepare(spec),
            TrackInstrument::Fm(s) => s.prepare(spec),
            #[cfg(feature = "host")]
            TrackInstrument::Plugin(p) => p.prepare(spec),
        }
    }

    pub fn note_on(&mut self, note: NoteRef, velocity: f32) {
        match self {
            TrackInstrument::Drums(s) => s.note_on(note, velocity),
            TrackInstrument::Poly(s) => s.note_on(note, velocity),
            TrackInstrument::Fm(s) => s.note_on(note, velocity),
            #[cfg(feature = "host")]
            TrackInstrument::Plugin(p) => p.note_on(note, velocity),
        }
    }

    pub fn note_off(&mut self, note: NoteRef, velocity: f32) {
        match self {
            TrackInstrument::Drums(s) => s.note_off(note, velocity),
            TrackInstrument::Poly(s) => s.note_off(note, velocity),
            TrackInstrument::Fm(s) => s.note_off(note, velocity),
            #[cfg(feature = "host")]
            TrackInstrument::Plugin(p) => p.note_off(note, velocity),
        }
    }

    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        match self {
            TrackInstrument::Drums(s) => s.process(buffer),
            TrackInstrument::Poly(s) => s.process(buffer),
            TrackInstrument::Fm(s) => s.process(buffer),
            #[cfg(feature = "host")]
            TrackInstrument::Plugin(p) => p.process(buffer),
        }
    }
}

/// 第三方插件乐器：持有宿主激活的 processor，把音符累积成事件，
/// 在每个 `process` 块通过 `process_with` 喂给插件。
///
/// 渲染循环会在每个子块开始前触发到点的 note_on/note_off，再 `process`；
/// 由于子块恰好在事件处切分，这里把事件记为 offset 0 即可做到样本精确。
#[cfg(feature = "host")]
pub struct PluginInstrument {
    processor: Box<dyn Processor<f32> + Send>,
    _plugin: manguebyte::host::any::Plugin,
    pending: manguebyte::prelude::EventBuffer,
    transport: manguebyte::prelude::Transport,
}

#[cfg(feature = "host")]
impl PluginInstrument {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor.prepare(spec);
    }

    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event {
            offset: 0,
            kind: EventKind::NoteOn { note, velocity },
        });
    }

    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        use manguebyte::prelude::{Event, EventKind};
        let _ = self.pending.push(Event {
            offset: 0,
            kind: EventKind::NoteOff { note, velocity },
        });
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        use manguebyte::prelude::ProcessContext;
        let ctx = ProcessContext::new(&self.pending, &self.transport);
        self.processor.process_with(buffer, &ctx);
        self.pending.clear();
    }
}

/// 解析失败。
#[derive(Debug, thiserror::Error)]
pub enum InstrumentError {
    #[error("未知乐器 '{name}'。可用内置：{}", available().join(", "))]
    Unknown { name: String },
    #[error("插件乐器需要 `host` 特性：请用 `--features host` 构建")]
    HostDisabled,
    #[error("加载插件乐器 '{path}' 失败: {detail}")]
    PluginLoad { path: String, detail: String },
}

/// 兼容旧名。
pub type UnknownInstrument = InstrumentError;

const POLY_VOICES: usize = 16;
const DRUM_VOICES: usize = 24;

/// 把语义乐器名解析为就绪的乐器。大小写不敏感。
/// `plugin:/path/to.vst3` 或 `plugin:/path.vst3#id` 加载第三方乐器插件。
pub fn resolve(name: &str) -> Result<TrackInstrument, InstrumentError> {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("plugin:") {
        return resolve_plugin(rest);
    }
    let key = trimmed.to_ascii_lowercase();
    let inst = match key.as_str() {
        "drums.808" | "808" => drums(DrumKit::style_808()),
        "drums.909" | "909" => drums(DrumKit::style_909()),
        "drums.606" | "606" => drums(DrumKit::style_606()),
        "drums.cr78" | "cr78" => drums(DrumKit::style_cr78()),
        "drums.acoustic" | "acoustic" => drums(DrumKit::acoustic()),
        "drums.trap" | "trap" => drums(DrumKit::trap()),
        "drums.techno" | "techno" => drums(DrumKit::techno()),
        "drums.house" | "house" => drums(DrumKit::house()),
        "drums.dnb" | "drums.drum_and_bass" | "dnb" => drums(DrumKit::drum_and_bass()),
        "drums.lofi" | "drums.lo_fi" | "lofi" => drums(DrumKit::lo_fi()),
        "synth.poly" | "poly" | "synth" => TrackInstrument::Poly(Synth::with_voices(
            POLY_VOICES,
            || PolySynthVoice::new(PolySynthParams::default()),
        )),
        "synth.fm" | "fm" => TrackInstrument::Fm(Synth::with_voices(POLY_VOICES, || {
            FmVoice::new(FmParams::default())
        })),
        _ => return Err(InstrumentError::Unknown { name: name.to_string() }),
    };
    Ok(inst)
}

/// 解析 `plugin:` 乐器。`spec`：`/path/to.vst3` 或 `/path.vst3#插件id`。
#[cfg(feature = "host")]
fn resolve_plugin(spec: &str) -> Result<TrackInstrument, InstrumentError> {
    use manguebyte::host::any::Library;

    let (path, id) = match spec.rsplit_once('#') {
        Some((p, i)) => (p.trim(), Some(i.trim())),
        None => (spec.trim(), None),
    };

    let library = unsafe { Library::load(path) }.map_err(|e| InstrumentError::PluginLoad {
        path: path.to_string(),
        detail: format!("{e:?}"),
    })?;

    let plugin_id = match id {
        Some(s) => s.to_string(),
        None => library
            .plugins()
            .first()
            .map(|p| p.id.clone())
            .ok_or_else(|| InstrumentError::PluginLoad {
                path: path.to_string(),
                detail: "库中没有任何插件".to_string(),
            })?,
    };

    let mut plugin = library
        .create(&plugin_id)
        .map_err(|e| InstrumentError::PluginLoad {
            path: path.to_string(),
            detail: format!("create('{plugin_id}') 失败: {e:?}"),
        })?;

    // 采样率/块大小在 prepare 时还会再确认；activate 需要一个初始值。
    let processor = plugin
        .activate(48_000.0, 512)
        .map_err(|e| InstrumentError::PluginLoad {
            path: path.to_string(),
            detail: format!("activate 失败: {e:?}"),
        })?;

    Ok(TrackInstrument::Plugin(PluginInstrument {
        processor,
        _plugin: plugin,
        pending: manguebyte::prelude::EventBuffer::with_capacity(256),
        transport: manguebyte::prelude::Transport::STOPPED,
    }))
}

#[cfg(not(feature = "host"))]
fn resolve_plugin(_spec: &str) -> Result<TrackInstrument, InstrumentError> {
    Err(InstrumentError::HostDisabled)
}

fn drums(kit: DrumKit) -> TrackInstrument {
    TrackInstrument::Drums(Synth::new(DrumKitVoice::voices(DRUM_VOICES, kit)))
}

/// 所有可用的内置乐器名（用于错误提示与文档）。插件乐器用 `plugin:路径`。
pub fn available() -> Vec<&'static str> {
    vec![
        "drums.808",
        "drums.909",
        "drums.606",
        "drums.cr78",
        "drums.acoustic",
        "drums.trap",
        "drums.techno",
        "drums.house",
        "drums.dnb",
        "drums.lofi",
        "synth.poly",
        "synth.fm",
    ]
}
