//! `muser export-daw`：把工程导出成一个文件夹，拖进 Ableton Live、Cubase、Logic 等 DAW 就能接着做：
//!
//! - `<名字>.mid`：所有乐器音轨的音符（与渲染一致），速度、拍号、段落标记；在 DAW 里可以换成自己的音源；
//! - `stems/NN-轨名.wav`：每条轨的声音（24 位，带效果器与发送，不带主总线效果器），各轨同长、从第 1 小节对齐；
//! - `mix.wav`：整首混音（对照用）；
//! - `IMPORT.md`：只是说明——各 DAW（Live、Cubase、Logic、FL Studio、Studio One、Bitwig、REAPER、GarageBand、Pro Tools）的导入步骤，以及速度、拍号、采样率。

use std::path::{Path, PathBuf};

use muser_core::Project;

use crate::render::{EngineError, RenderSettings, Stem, render_stems, render_to_wav};

pub struct DawExport {
    pub midi: PathBuf,
    pub mix: PathBuf,
    pub guide: PathBuf,
    pub stems: Vec<Stem>,
    /// MIDI 里没有的音轨（audio 音轨没有音符，只在 stems 里）。
    pub midi_skipped: Vec<String>,
}

pub fn export_daw(project: &Project, dir: &Path, settings: RenderSettings, progress: &mut dyn FnMut(&str, usize, usize)) -> Result<DawExport, EngineError> {
    let io = |e: std::io::Error| EngineError::Wav(format!("写不了 {}: {e}", dir.display()));
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
        tempo += " → variable / 中途变速（the MIDI file carries the tempo map / 速度表在 MIDI 里）";
    }
    let stem_list: String = stems.iter().map(|s| format!("- `stems/{}`：{}\n", s.path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default(), s.track)).collect();
    let skipped = if skipped.is_empty() { String::new() } else { format!("\n（{} 是音频轨，没有 MIDI，只在 stems 里。）\n", skipped.join("、")) };
    format!(
        "# {base} · 导入 DAW / Importing into a DAW\n\n\
         由 muser {ver} 导出。速度 **{tempo} BPM**，拍号 **{ts}**，采样率 **{sr} Hz**（音频是 24 位 WAV）。\n\
         Tempo **{tempo} BPM**, time signature **{ts}**, sample rate **{sr} Hz** (24-bit WAV).\n\n\
         ## 文件 / Files\n\n\
         - `{base}.mid`：所有乐器轨的 MIDI（含速度、拍号、段落标记）/ MIDI of every instrument track, with tempo, meter and section markers\n\
         - `mix.wav`：整首混音，对照用 / the full mix, for reference\n\
         {stem_list}{skipped}\n\
         stems 带着各自的效果器和混响发送，不带主总线效果器（母带留给 DAW）；每个文件和整首一样长，**都从第 1 小节第 1 拍开始**。\n\
         Stems include each track's effects and sends but not the master effects; all are as long as the song and **start at bar 1, beat 1**.\n\n\
         先把 DAW 工程的采样率设成 {sr} Hz、速度设成 {tempo}、拍号 {ts}，再导入。\n\
         Set the DAW project to {sr} Hz, {tempo} BPM and {ts} before importing.\n\n\
         ## Ableton Live\n\n\
         1. 在 Arrangement 视图把速度设成 {tempo}（Live 拖入 MIDI 文件时不改速度）。\n\
         2. 把 `stems/` 里的 WAV 一起拖到第 1 小节；拖进来后在每个片段上关掉 **Warp**（或先在 Preferences → Record/Warp/Launch 关掉 Auto-Warp Long Samples）。\n\
         3. 需要 MIDI 时把 `{base}.mid` 拖进来，每条轨换上你的乐器。\n\n\
         ## Cubase\n\n\
         1. File → Import → MIDI File…，选 `{base}.mid`：速度、拍号和标记一起进来。\n\
         2. File → Import → Audio File…，多选 `stems/` 里的文件，选 **Different Tracks**，放在工程开头（第 1 小节）。\n\
         3. Cubase 14 起也能用 DAWproject，但这里给的是通用的 MIDI + WAV。\n\n\
         ## Logic Pro\n\n\
         1. File → Import → MIDI File…，选 `{base}.mid`，提示时选择导入速度。\n\
         2. 把 `stems/` 里的文件一起拖到第 1 小节，选 **Create new tracks**。\n\
         3. 如果 Logic 想按音频改速度（Smart Tempo），把 Project Tempo 模式设成 **Keep**。\n\n\
         ## FL Studio\n\n\
         1. 先把工程速度设成 {tempo}。\n\
         2. File → Import → MIDI file…，选 `{base}.mid`，每条 MIDI 轨会变成一个通道（channel）。\n\
         3. 把 `stems/` 里的 WAV 从浏览器拖进 Playlist，每个放一条轨、都放在第 1 小节开头；如果 FL 提示按速度拉伸音频，选不拉伸（保持原样）。\n\n\
         ## Studio One\n\n\
         1. 把 `{base}.mid` 拖进编曲窗口，提示时选择导入速度。\n\
         2. 把 `stems/` 里的文件一起拖到第 1 小节（每个文件一条轨）。Studio One 7 起也能打开 DAWproject。\n\n\
         ## Bitwig Studio\n\n\
         1. 把速度设成 {tempo}，再把 `{base}.mid` 拖进编曲视图（每条 MIDI 轨一条轨）。\n\
         2. 把 stems 拖到第 1 小节；在片段的 Stretch 设置里选 Off（不随速度拉伸）。\n\n\
         ## REAPER\n\n\
         1. Insert → Media file…，选 `{base}.mid`，勾上导入速度表（tempo map）和拆成多条轨。\n\
         2. 光标放在开头，Insert → Media file… 多选 stems，选“每个文件一条轨”。\n\n\
         ## GarageBand\n\n\
         1. 新建空工程，把 `{base}.mid` 拖进轨道区（第一个拖进来的 MIDI 文件会带上速度）。\n\
         2. 把 stems 拖到第 1 小节；在音频区域上关掉 **Follow Tempo & Pitch**。\n\n\
         ## Pro Tools\n\n\
         1. File → Import → MIDI…，选择导入速度表（tempo map），放到新轨。\n\
         2. File → Import → Audio…，选中 stems 导入到新轨，放在 session 开头。\n\n\
         ## 其它 DAW / Other DAWs\n\n\
         设好速度和采样率，导入 MIDI 文件，再把 stems 全部放在第 1 小节开头，关掉音频的自动拉伸。\n\
         Set tempo and sample rate, import the MIDI file, place all stems at the very start of bar 1, and turn off audio time-stretching.\n\n\
         菜单名称随版本略有不同；找不到时搜 DAW 的“import MIDI file”和“import audio to new tracks”。\n\
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
