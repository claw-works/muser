//! 乐器注册表：把 core 模型里的语义乐器名解析为具体的 manguebyte 乐器。
//!
//! AI 在 Project 里只写 `"drums.808"` / `"synth.poly"` / `"fm"` 这类名字，
//! 这里负责把它们变成一个可渲染的 [`TrackInstrument`]。
//!
//! 不同乐器的 Voice 是不同的 Rust 类型（`DrumKitVoice` vs `PolySynthVoice`…），
//! 无法放进同一个 `Synth<V>`，因此用枚举 [`TrackInstrument`] 做类型擦除，
//! 对外暴露统一的 `prepare` / `note_on` / `note_off` / `process`。

use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth};

use manguebyte::dsp::instruments::{DrumKit, DrumKitVoice, FmParams, FmVoice, PolySynthParams, PolySynthVoice};

/// 一件已就绪、可渲染的音轨乐器。
pub enum TrackInstrument {
    /// 鼓组（打击乐），音符走 MIDI 通道 9，key = GM 鼓键。
    Drums(Synth<DrumKitVoice>),
    /// 复音合成器。
    Poly(Synth<PolySynthVoice>),
    /// FM 合成器。
    Fm(Synth<FmVoice>),
}

/// 该乐器约定使用的 MIDI 通道（鼓组按惯例用 9）。
impl TrackInstrument {
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
        }
    }

    pub fn note_on(&mut self, note: NoteRef, velocity: f32) {
        match self {
            TrackInstrument::Drums(s) => s.note_on(note, velocity),
            TrackInstrument::Poly(s) => s.note_on(note, velocity),
            TrackInstrument::Fm(s) => s.note_on(note, velocity),
        }
    }

    pub fn note_off(&mut self, note: NoteRef, velocity: f32) {
        match self {
            TrackInstrument::Drums(s) => s.note_off(note, velocity),
            TrackInstrument::Poly(s) => s.note_off(note, velocity),
            TrackInstrument::Fm(s) => s.note_off(note, velocity),
        }
    }

    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        match self {
            TrackInstrument::Drums(s) => s.process(buffer),
            TrackInstrument::Poly(s) => s.process(buffer),
            TrackInstrument::Fm(s) => s.process(buffer),
        }
    }
}

/// 解析失败（未知乐器名）。
#[derive(Debug, thiserror::Error)]
#[error("未知乐器 '{name}'。可用：{}", available().join(", "))]
pub struct UnknownInstrument {
    pub name: String,
}

/// 默认复音数（同时可发声的音符数）。
const POLY_VOICES: usize = 16;
const DRUM_VOICES: usize = 24;

/// 把语义乐器名解析为就绪的乐器。大小写不敏感。
pub fn resolve(name: &str) -> Result<TrackInstrument, UnknownInstrument> {
    let key = name.trim().to_ascii_lowercase();
    let inst = match key.as_str() {
        // —— 鼓组（manguebyte 的预设鼓机/风格套鼓）——
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
        // —— 合成器 ——
        "synth.poly" | "poly" | "synth" => {
            TrackInstrument::Poly(Synth::with_voices(POLY_VOICES, || {
                PolySynthVoice::new(PolySynthParams::default())
            }))
        }
        "synth.fm" | "fm" => TrackInstrument::Fm(Synth::with_voices(POLY_VOICES, || {
            FmVoice::new(FmParams::default())
        })),
        _ => return Err(UnknownInstrument { name: name.to_string() }),
    };
    Ok(inst)
}

fn drums(kit: DrumKit) -> TrackInstrument {
    TrackInstrument::Drums(Synth::new(DrumKitVoice::voices(DRUM_VOICES, kit)))
}

/// 所有可用的乐器名（用于错误提示与文档）。
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
