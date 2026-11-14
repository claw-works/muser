//! `muser export-daw`：把工程导出成一个文件夹，拖进 Ableton Live、Cubase、Logic 等 DAW 就能接着做：
//!
//! - `<名字>.mid`：所有乐器音轨的音符（与渲染一致），速度、拍号、段落标记；在 DAW 里可以换成自己的音源；
//! - `stems/NN-轨名.wav`：每条轨的声音（24 位，带效果器与发送，不带主总线效果器），各轨同长、从第 1 小节对齐；
//! - `mix.wav`：整首混音（对照用）；
//! - `IMPORT.md`：只是说明——各 DAW（Live、Cubase、Logic、FL Studio、Studio One、Bitwig、REAPER、GarageBand、Pro Tools）的导入步骤，以及速度、拍号、采样率。

use std::path::{Path, PathBuf};

use muser_core::Project;

use crate::render::{render_stems, render_to_wav, EngineError, RenderSettings, Stem};

pub struct DawExport {
    pub midi: PathBuf,
    pub mix: PathBuf,
    pub guide: PathBuf,
    pub stems: Vec<Stem>,
    /// MIDI 里没有的音轨（audio 音轨没有音符，只在 stems 里）。
    pub midi_skipped: Vec<String>,
}

pub fn export_daw(project: &Project, dir: &Path, settings: RenderSettings, progress: &mut dyn FnMut(&str, usize, usize)) -> Result<DawExport, EngineError> {
    let io = |e: std::io::Error| EngineError::Wav(format!("cannot write {}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    let base: String = if project.name.is_empty() { "muser".into() } else { project.name.rsplit('/').next().unwrap_or("muser").into() };
    let base: String = base.chars().map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();

    let midi = crate::midi::export(project)?;
    let midi_path = dir.join(format!("{base}.mid"));
    std::fs::write(&midi_path, &midi.bytes).map_err(io)?;

    let stems = render_stems(project, &dir.join("stems"), settings, progress)?;
    let mix = dir.join("mix.wav");
    render_to_wav(project, &mix.to_string_lossy(), settings)?;

    let guide = dir.join("IMPORT.md");
    std::fs::write(&guide, guide_text(project, &base, settings.sample_rate, &stems, &midi.skipped)).map_err(io)?;
    Ok(DawExport { midi: midi_path, mix, guide, stems, midi_skipped: midi.skipped })
}

fn guide_text(p: &Project, base: &str, sr: u32, stems: &[Stem], skipped: &[String]) -> String {
    let ts = format!("{}/{}", p.time_signature.numerator, p.time_signature.denominator);
    let mut tempo = if p.tempo.fract() == 0.0 { format!("{}", p.tempo) } else { format!("{:.3}", p.tempo) };
    if !p.tempo_map().is_constant() {
        tempo += " -> variable (the MIDI file carries the tempo map)";
    }
    let stem_list: String =
        stems.iter().map(|s| format!("- `stems/{}`: {}\n", s.path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default(), s.track)).collect();
    let skipped = if skipped.is_empty() { String::new() } else { format!("\n({} are audio tracks: no MIDI, stems only.)\n", skipped.join(", ")) };
    format!(
        "# {base} - Importing into a DAW\n\n\
         Exported by muser {ver}. Tempo **{tempo} BPM**, time signature **{ts}**, sample rate **{sr} Hz** (24-bit WAV).\n\n\
         ## Files\n\n\
         - `{base}.mid`: MIDI of every instrument track, with tempo, meter and section markers\n\
         - `mix.wav`: the full mix, for reference\n\
         {stem_list}{skipped}\n\
         Stems include each track's effects and sends but not the master effects (leave mastering to the DAW); all are as long as the song and **start at bar 1, beat 1**.\n\n\
         Set the DAW project to {sr} Hz, {tempo} BPM and {ts} before importing.\n\n\
         ## Ableton Live\n\n\
         1. In Arrangement view set the tempo to {tempo} (Live does not change the tempo when you drop in a MIDI file).\n\
         2. Drag the WAVs from `stems/` to bar 1 together; then turn **Warp** off on every clip (or first turn off Auto-Warp Long Samples in Preferences -> Record/Warp/Launch).\n\
         3. For MIDI, drag in `{base}.mid` and put your own instrument on each track.\n\n\
         ## Cubase\n\n\
         1. File -> Import -> MIDI File..., pick `{base}.mid`: tempo, time signature and markers come in together.\n\
         2. File -> Import -> Audio File..., select all files in `stems/`, choose **Different Tracks**, and place them at the project start (bar 1).\n\
         3. Cubase 14 and later can also use DAWproject, but this export is generic MIDI + WAV.\n\n\
         ## Logic Pro\n\n\
         1. File -> Import -> MIDI File..., pick `{base}.mid`, and choose to import the tempo when prompted.\n\
         2. Drag the files in `stems/` to bar 1 together and choose **Create new tracks**.\n\
         3. If Logic wants to change the tempo to match the audio (Smart Tempo), set Project Tempo mode to **Keep**.\n\n\
         ## FL Studio\n\n\
         1. Set the project tempo to {tempo} first.\n\
         2. File -> Import -> MIDI file..., pick `{base}.mid`; each MIDI track becomes a channel.\n\
         3. Drag the WAVs from `stems/` out of the browser into the Playlist, one per track, all at the start of bar 1; if FL offers to stretch the audio to the tempo, choose not to stretch (keep as is).\n\n\
         ## Studio One\n\n\
         1. Drag `{base}.mid` into the arrangement window and choose to import the tempo when prompted.\n\
         2. Drag the files in `stems/` to bar 1 together (one track per file). Studio One 7 and later can also open DAWproject.\n\n\
         ## Bitwig Studio\n\n\
         1. Set the tempo to {tempo}, then drag `{base}.mid` into the arranger (one track per MIDI track).\n\
         2. Drag the stems to bar 1; in each clip's Stretch settings choose Off (do not stretch to the tempo).\n\n\
         ## REAPER\n\n\
         1. Insert -> Media file..., pick `{base}.mid`, and tick importing the tempo map and splitting into multiple tracks.\n\
         2. With the cursor at the start, Insert -> Media file... and select all the stems, choosing one track per file.\n\n\
         ## GarageBand\n\n\
         1. Create an empty project and drag `{base}.mid` into the track area (the first MIDI file dropped in brings its tempo).\n\
         2. Drag the stems to bar 1; turn off **Follow Tempo & Pitch** on the audio regions.\n\n\
         ## Pro Tools\n\n\
         1. File -> Import -> MIDI..., choose to import the tempo map, and put it on new tracks.\n\
         2. File -> Import -> Audio..., select the stems and import them to new tracks, placed at the start of the session.\n\n\
         ## Other DAWs\n\n\
         Set the tempo and sample rate, import the MIDI file, place all stems at the very start of bar 1, and turn off audio time-stretching.\n\n\
         Menu names vary between versions; search your DAW's help for \"import MIDI file\" and \"import audio to new tracks\".\n",
        ver = env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_midi_stems_mix_and_guide() {
        let p: Project = serde_json::from_value(serde_json::json!({ "name": "songs/daw test", "tempo": 100,
            "tracks": [
                { "name": "k", "instrument": "drums.909", "clips": [{ "start": 0, "length": 4, "grid": { "kick": "x..." } }] },
                { "name": "b", "instrument": "poly.juno_keys", "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 48, "start": 0, "length": 2 }] }] }
            ] }))
        .unwrap();
        let dir = std::env::temp_dir().join(format!("muser-daw-{}", std::process::id()));
        let r = export_daw(&p, &dir, RenderSettings::default(), &mut |_, _, _| {}).unwrap();
        assert!(r.midi.ends_with("daw test.mid") && r.midi.exists());
        assert_eq!(r.stems.len(), 2);
        let a = manguebyte::prelude::wav::read_file(&r.stems[0].path).unwrap();
        assert_eq!(a.bits_per_sample, 24);
        assert!(r.mix.exists());
        let guide = std::fs::read_to_string(&r.guide).unwrap();
        assert!(guide.contains("100 BPM") && guide.contains("stems/02-b.wav"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
