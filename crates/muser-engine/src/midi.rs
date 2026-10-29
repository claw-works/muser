//! 导出标准 MIDI 文件（format 1）：每条乐器音轨一条 MIDI 轨，第一条轨放速度与拍号。
//!
//! - 音符与渲染完全一致：humanize 之后的时间与力度、连奏延长、clip 裁剪、`drummer` / `abc` / `grid` 展开的音符；
//!   延音踏板写成 CC64。
//! - 鼓组（`drums.*`）用通道 10（GM 鼓）；其它音轨依次分配通道，并按乐器名猜一个 GM 音色（program change），
//!   只为在 DAW 里打开时有个像样的默认音色。
//! - 音轨的 `gain_db` / `pan` 写成 CC7 / CC10。静音的音轨照常导出，轨名后标"(muted)"。
//! - audio 音轨（音频文件）没有音符，不导出。

use manguebyte::midi::MidiMessage;
use manguebyte::midi::file::{MetaEvent, MidiFile, Track};
use muser_core::Project;

use crate::render::{Act, EngineError, schedule_ticks};

/// 每四分音符的 tick 数。
pub const TICKS_PER_QUARTER: u16 = 480;

/// 导出结果：文件内容，与没有导出的音轨（audio 音轨）名字。
pub struct MidiExport {
    pub bytes: Vec<u8>,
    pub skipped: Vec<String>,
}

/// 按乐器名（含 sfz 路径）里的关键词猜 GM 音色（0 起）。先匹配的优先。
/// shortcut: 关键词表很粗，只求 DAW 里打开时大致对路；需要精确时再给音轨加一个显式的 program 字段。
fn gm_program(instrument: &str) -> u8 {
    const TABLE: &[(&str, u8)] = &[
        ("pizz", 45), ("contrabass", 43), ("cello", 42), ("viola", 41), ("violin", 40), ("ensemble", 48), ("string", 48),
        ("nylon", 24), ("steel_pan", 114), ("steel", 25), ("guitar", 25),
        ("acid", 38), ("slap", 36), ("finger", 33), ("bass", 38), ("sub", 38),
        ("organ", 16), ("marimba", 12), ("xylophone", 13), ("vibraphone", 11), ("glockenspiel", 9), ("celesta", 8),
        ("kalimba", 108), ("tubular", 14), ("bell", 14),
        ("flute", 73), ("clarinet", 71), ("sax", 65), ("trumpet", 56), ("trombone", 57), ("brass", 61), ("horn", 60), ("fanfare", 61),
        ("choir.solo", 53), ("choir", 52), ("aah", 52), ("ooh", 53),
        ("clavinet", 7), ("wurly", 4), ("electric_piano", 5), ("keys", 4), ("piano", 0),
        ("pad", 89), ("drone", 89), ("spectra", 89), ("wash", 89), ("pluck", 84),
        ("lead", 81), ("saw", 81), ("square", 80), ("chiptune", 80), ("hoover", 81), ("screamer", 81),
        ("riser", 98), ("downlifter", 98), ("noise", 122), ("wind", 122), ("impact", 127), ("siren", 125), ("laser", 98), ("bleep", 98),
    ];
    let name = instrument.to_ascii_lowercase();
    if let Some(&(_, p)) = TABLE.iter().find(|(k, _)| name.contains(k)) {
        return p;
    }
    if name.starts_with("poly.") || name.starts_with("synth.") { 81 } else { 0 }
}

/// 把工程导出为 MIDI 文件。
pub fn export(project: &Project) -> Result<MidiExport, EngineError> {
    crate::validate_score(project)?;
    let sections = crate::arrange::timeline(project).map_err(|e| EngineError::Core(muser_core::CoreError::Invalid(e)))?;
    let arranged = crate::arrange::arranged(project).map_err(|e| EngineError::Core(muser_core::CoreError::Invalid(e)))?;
    let project = &*arranged;
    let ts = project.time_signature;
    let mut conductor = Track::new();
    conductor.push_meta(0, MetaEvent::TrackName(if project.name.is_empty() { "muser".into() } else { project.name.clone() }));
    conductor.push_meta(0, MetaEvent::Tempo((60_000_000.0 / project.tempo).round() as u32));
    // 段落名写成 marker（DAW 里的段落标记）。
    for s in &sections {
        let tick = (s.start * f64::from(TICKS_PER_QUARTER)).round() as u64;
        let label = if sections.iter().filter(|x| x.name == s.name).count() > 1 { format!("{} {}", s.name, s.occurrence) } else { s.name.clone() };
        conductor.push_meta(tick, MetaEvent::Marker(label));
    }
    conductor.push_meta(0, MetaEvent::TimeSignature {
        numerator: ts.numerator,
        denominator_power: ts.denominator.max(1).trailing_zeros() as u8,
        clocks_per_click: 24,
        thirty_seconds_per_quarter: 8,
    });
    let mut tracks = vec![conductor];
    let mut skipped = Vec::new();
    let mut melodic = (0u8..16).filter(|&c| c != 9).cycle();
    for (ti, t) in project.tracks.iter().enumerate() {
        let name = if t.name.is_empty() { t.instrument.clone() } else { t.name.clone() };
        if t.instrument.trim() == muser_core::audio::INSTRUMENT {
            skipped.push(name);
            continue;
        }
        let drums = t.instrument.trim().starts_with("drums.");
        let channel = if drums { 9 } else { melodic.next().unwrap_or(0) };
        let mut track = Track::new();
        track.push_meta(0, MetaEvent::TrackName(if t.muted { format!("{name} (muted)") } else { name }));
        track.push_meta(0, MetaEvent::InstrumentName(t.instrument.clone()));
        if !drums {
            track.push_midi(0, MidiMessage::ProgramChange { channel, program: gm_program(&t.instrument) });
        }
        // 音量：线性增益 = (cc / 127)²（DAW 常见的推子曲线），0 dB 对应 127。
        let volume = (127.0 * 10f64.powf(t.gain_db / 40.0)).round().clamp(0.0, 127.0) as u8;
        let pan = (64.0 + t.pan * 63.0).round().clamp(0.0, 127.0) as u8;
        track.push_midi(0, MidiMessage::ControlChange { channel, controller: 7, value: volume });
        track.push_midi(0, MidiMessage::ControlChange { channel, controller: 10, value: pan });
        for (tick, act) in schedule_ticks(project, ti, TICKS_PER_QUARTER) {
            let tick = tick as u64;
            let message = match act {
                Act::On { key, velocity, .. } => {
                    MidiMessage::NoteOn { channel, note: key, velocity: (velocity * 127.0).round().clamp(1.0, 127.0) as u8 }
                }
                Act::Off { key, .. } => MidiMessage::NoteOff { channel, note: key, velocity: 0 },
                Act::Pedal { down } => MidiMessage::ControlChange { channel, controller: 64, value: if down { 127 } else { 0 } },
                // shortcut: 一个音轨一个通道，弯音会带上同时响着的其它音；要分开得按 MPE 每音一个通道导出。
                Act::Bend { value, .. } => MidiMessage::PitchBend { channel, value },
            };
            track.push_midi(tick, message);
        }
        tracks.push(track);
    }
    Ok(MidiExport { bytes: MidiFile::multi_track(TICKS_PER_QUARTER, tracks).to_bytes(), skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use manguebyte::midi::file::TrackEventKind;

    #[test]
    fn notes_tempo_channels_and_pedal() {
        let p: Project = serde_json::from_value(serde_json::json!({
            "name": "t", "tempo": 90, "time_signature": { "numerator": 3, "denominator": 4 },
            "tracks": [
                { "name": "drums", "instrument": "drums.808", "clips": [{ "start": 1, "length": 2, "grid": { "kick": "x...x..." } }] },
                { "name": "keys", "instrument": "sfz:Piano.sfz", "gain_db": -6, "pan": -1, "muted": true,
                  "clips": [{ "start": 3, "length": 2, "pedal": [[0, 1.5]], "notes": [{ "pitch": 60, "start": 0.5, "length": 1, "velocity": 0.5 }] }] },
                { "name": "vox", "instrument": "audio", "clips": [] }
            ] }))
        .unwrap();
        // sfz 文件不存在时校验会报错：测试只看 MIDI，换成内置乐器名再导出。
        let mut p = p;
        p.tracks[1].instrument = "guitar.nylon".into();
        let out = export(&p).unwrap();
        assert_eq!(out.skipped, ["vox"]);
        let f = MidiFile::from_bytes(&out.bytes).unwrap();
        assert_eq!(f.tracks.len(), 3);
        assert!(f.tracks[0].events().iter().any(|e| e.kind == TrackEventKind::Meta(MetaEvent::Tempo(666_667))));
        let on = |t: usize| -> Vec<(u64, u8, u8)> {
            f.tracks[t].events().iter().filter_map(|e| match e.kind {
                TrackEventKind::Midi(MidiMessage::NoteOn { channel, note, .. }) => Some((e.tick, channel, note)),
                _ => None,
            }).collect()
        };
        // 鼓：通道 10，第 1 拍与第 2 拍（clip 从第 1 拍开始，四个十六分一下）。
        assert_eq!(on(1), [(480, 9, 36), (960, 9, 36)]);
        assert_eq!(on(2), [(3 * 480 + 240, 0, 60)]);
        let ev = f.tracks[2].events();
        assert_eq!(f.tracks[2].name(), Some("keys (muted)"));
        assert!(ev.iter().any(|e| e.kind == TrackEventKind::Midi(MidiMessage::ProgramChange { channel: 0, program: 24 })));
        assert!(ev.iter().any(|e| e.kind == TrackEventKind::Midi(MidiMessage::ControlChange { channel: 0, controller: 10, value: 1 })));
        assert!(ev.iter().any(|e| e.tick == 3 * 480 && e.kind == TrackEventKind::Midi(MidiMessage::ControlChange { channel: 0, controller: 64, value: 127 })));
        assert!(ev.iter().any(|e| e.tick == 3 * 480 + 240 + 480 && matches!(e.kind, TrackEventKind::Midi(MidiMessage::NoteOff { note: 60, .. }))));
    }

    /// 不整的速度下，十六分音符网格也精确落在 tick 上。
    #[test]
    fn odd_tempo_lands_on_ticks() {
        for tempo in [97.3, 123.7, 61.1, 173.9] {
            let p: Project = serde_json::from_value(serde_json::json!({ "tempo": tempo, "tracks": [
                { "instrument": "drums.909", "clips": [{ "start": 3, "length": 64, "grid": { "hat": "x" } }] }] })).unwrap();
            let f = MidiFile::from_bytes(&export(&p).unwrap().bytes).unwrap();
            let ticks: Vec<u64> = f.tracks[1].events().iter().filter(|e| matches!(e.kind, TrackEventKind::Midi(MidiMessage::NoteOn { .. }))).map(|e| e.tick).collect();
            assert_eq!(ticks, (0..256).map(|k| 3 * 480 + k * 120).collect::<Vec<u64>>(), "tempo {tempo}");
        }
    }
}
