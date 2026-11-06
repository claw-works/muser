//! `muser play`：用本机声卡把工程从头放一遍，放完退出（给人快速试听草稿；不开网页）。
//! `muser loop`：loop 库（列出、取出写进工程的 clip、存、试听）。

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use muser_core::Project;
use muser_engine::loop_library;
#[cfg(feature = "device")]
use muser_engine::RenderSettings;

/// Play the project once (local sound card), starting `from` seconds in.
pub fn play(project: Project, from: f64) -> Result<()> {
    muser_engine::validate(&project)?;
    #[cfg(feature = "device")]
    {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;
        let cache = Arc::new(Mutex::new(muser_engine::RenderCache::new()));
        let player = muser_engine::playback::Player::start(RenderSettings { tail_seconds: 1.5, ..RenderSettings::default() }, cache).map_err(|e| anyhow!(e))?;
        player.load(project);
        if from > 0.0 {
            player.seek(from);
        }
        player.play();
        let mut shown = -1i64;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let s = player.status();
            if let Some(e) = s.error {
                bail!("playback failed: {e}");
            }
            if (s.position as i64) != shown && s.duration > 0.0 {
                shown = s.position as i64;
                eprint!("\rplaying {:5.1}s / {:.1}s ({})  Ctrl+C to stop", s.position, s.duration, s.device);
            }
            if s.duration > 0.0 && !s.playing && !s.seeking && s.position >= s.duration - 0.01 {
                eprintln!();
                return Ok(());
            }
        }
    }
    #[cfg(not(feature = "device"))]
    {
        let _ = from;
        bail!("this muser build has no sound card playback (the device feature); render to WAV with `muser render` and listen to that")
    }
}

#[derive(Subcommand)]
pub enum LoopCommand {
    /// List loops: project (loops/ next to the project), global (~/.config/muser/loops), built-in.
    List {
        /// Only this style (house, dnb, hiphop ...).
        #[arg(long)]
        style: Option<String>,
        /// Project file (looks in the loops/ next to it; default: the current directory).
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print the clip JSON to put in a project track (grid / notes + humanize embedded, `from` marks the source).
    Show {
        name: String,
        /// Clip start (beats).
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        /// Number of repeats (clip length = repeats x loop length).
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// Print the loop file itself instead (with style, tempo, suggested instrument, key, chords).
        #[arg(long, conflicts_with = "abc")]
        file: bool,
        /// Print ABC notation instead (melodic loops).
        #[arg(long)]
        abc: bool,
        #[arg(long)]
        project: Option<String>,
    },
    /// Audition a loop on the local sound card.
    Play {
        name: String,
        /// Use this instrument / drum kit (default: the loop's suggestion).
        #[arg(long, alias = "kit")]
        instrument: Option<String>,
        /// Tempo (default: the middle of the loop's tempo range).
        #[arg(long)]
        tempo: Option<f64>,
        /// Number of repeats.
        #[arg(long, default_value_t = 4)]
        repeat: u32,
        #[arg(long)]
        project: Option<String>,
    },
    /// Save a clip from a project as a loop: drums as a grid, melody as notes. Default: the loops/ next to the project.
    Save {
        /// Project file.
        project: String,
        /// Track index (from 0).
        #[arg(long)]
        track: usize,
        /// Clip index (from 0).
        #[arg(long, default_value_t = 0)]
        clip: usize,
        /// Loop name (lowercase letters, digits, _, -).
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "")]
        style: String,
        #[arg(long, default_value = "")]
        description: String,
        /// Drum quantization grid (beats); only used when the drum clip is plain notes.
        #[arg(long, default_value_t = 0.25)]
        step: f64,
        /// Save to global (~/.config/muser/loops) instead of the project.
        #[arg(long)]
        global: bool,
        /// Overwrite if the name already exists.
        #[arg(long)]
        overwrite: bool,
    },
    /// Hum to notes: turn a monophonic recording (hummed, sung, whistled, a single instrument) into notes and save it as a melodic loop (no AI).
    Hum {
        /// Audio file (WAV, AIFF, FLAC, Ogg, Opus, MP3 ...).
        audio: String,
        /// Loop name (default: the audio file name).
        #[arg(long)]
        name: Option<String>,
        /// Tempo (BPM): used to convert seconds to beats.
        #[arg(long, default_value_t = 100.0)]
        tempo: f64,
        /// Alignment grid: 1/16, 1/8, 1/8t, 0.25 (beats); off for none.
        #[arg(long, default_value = "1/16")]
        grid: String,
        /// Keep the silence from the start of the recording to the first note (for recordings made to a metronome); otherwise the first note moves to beat 0.
        #[arg(long)]
        from_start: bool,
        /// Suggested instrument (default poly.juno_keys).
        #[arg(long)]
        instrument: Option<String>,
        /// Key (e.g. Am); default: estimated from the pitch.
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        project: Option<String>,
        /// Save to global instead of the project.
        #[arg(long)]
        global: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// Copy a project loop to global (usable from other projects).
    Share {
        name: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        overwrite: bool,
    },
    /// Open the loop / motif editor (a local web page: piano roll for melody, step grid for drums); saves to the project or global.
    Edit {
        /// Open this loop (default: a new one).
        name: Option<String>,
        /// Project file (loops are saved in the loops/ next to it; default: the current directory).
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 8739)]
        port: u16,
        /// Open in the system browser.
        #[arg(long)]
        open: bool,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
    },
}

/// `--project` 的工程文件所在目录。
fn base_of(project: &Option<String>) -> Option<std::path::PathBuf> {
    project.as_ref().map(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf).unwrap_or_default())
}

pub fn run_loop(command: LoopCommand) -> Result<()> {
    let fail = |e: String| anyhow!(e);
    match command {
        LoopCommand::List { style, project, json } => {
            let base = base_of(&project);
            let mut all = loop_library::list_for(base.as_deref());
            if let Some(s) = &style {
                all.retain(|e| e.item.style.eq_ignore_ascii_case(s) || e.item.tags.iter().any(|t| t.eq_ignore_ascii_case(s)));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(());
            }
            for e in &all {
                let l = &e.item;
                let tempo = l.tempo.map_or(String::new(), |[a, b]| format!("{a:.0}–{b:.0} BPM"));
                let mark = match e.source { "project" => " +", "user" => " *", _ => "" };
                let kind = if l.is_drums() { "drums" } else { "melody" };
                println!("{:<18} {:<6} {:<10} {:>12}  {} bars  {}{mark}", l.name, kind, l.style, tempo, l.bars, l.description);
            }
            println!(
                "\n{} loop(s) (+ = project {}, * = global {}). `muser loop show <name>` prints the clip to put in a project; `muser loop play <name>` auditions; `muser loop edit` edits.",
                all.len(),
                loop_library::project_dir(base.as_deref()).display(),
                loop_library::dir().display()
            );
            Ok(())
        }
        LoopCommand::Show { name, start, repeat, file, abc, project } => {
            let l = loop_library::load_for(&name, base_of(&project).as_deref()).map_err(fail)?;
            if abc {
                let p = l.project(None, None, repeat).map_err(fail)?;
                let mut opts = muser_engine::abc_export::AbcOptions::new(muser_engine::abc_export::Dialect::Standard);
                opts.key = l.key.clone();
                let out = muser_engine::abc_export::export(&p, &opts)?;
                print!("{}", out.text);
                return Ok(());
            }
            let out = if file { serde_json::to_value(&l)? } else { l.clip(start, repeat) };
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        LoopCommand::Play { name, instrument, tempo, repeat, project } => {
            let l = loop_library::load_for(&name, base_of(&project).as_deref()).map_err(fail)?;
            let p = l.project(instrument.as_deref(), tempo, repeat).map_err(fail)?;
            eprintln!("{}: {}, {} BPM, {repeat} repeat(s)", l.name, p.tracks[0].instrument, p.tempo);
            play(p, 0.0)
        }
        LoopCommand::Save { project, track, clip, name, style, description, step, global, overwrite } => {
            let p = crate::load_project(&project)?;
            let t = p.tracks.get(track).ok_or_else(|| anyhow!("no tracks[{track}]"))?;
            let c = t.clips.get(clip).ok_or_else(|| anyhow!("tracks[{track}] has no clips[{clip}]"))?;
            let drums = t.instrument.trim().starts_with("drums.");
            let notes = muser_engine::clip_notes(&p, track, clip).map_err(fail)?;
            let grid = match (drums, &c.grid, c.notes.is_empty() && c.abc.is_none() && c.drummer.is_none()) {
                (false, _, _) => muser_core::grid::Grid::default(),
                (true, Some(g), true) => g.clone(),
                (true, _, _) => loop_library::grid_from_notes(&notes, c.length.0, step).map_err(fail)?,
            };
            let bars = (c.length.0 / 4.0).ceil().max(1.0) as u32;
            let l = loop_library::Loop {
                format: loop_library::FORMAT.into(),
                version: loop_library::VERSION,
                name,
                style,
                description,
                tags: Vec::new(),
                tempo: Some([p.tempo, p.tempo]),
                bars,
                meter: "4/4".into(),
                kit: drums.then(|| t.instrument.clone()),
                instrument: (!drums).then(|| t.instrument.clone()),
                key: None,
                chords: None,
                grid,
                notes: if drums { Vec::new() } else { notes.into_iter().filter(|n| n.start.0 < f64::from(bars) * 4.0).collect() },
                humanize: muser_core::humanize::Humanize::merged(t.humanize.as_ref(), c.humanize.as_ref()),
            };
            let path = if global { loop_library::save(&l, overwrite) } else { loop_library::save_project(&l, p.base_dir.as_deref(), overwrite) }.map_err(fail)?;
            println!("saved loop '{}' -> {}", l.name, path.display());
            Ok(())
        }
        LoopCommand::Hum { audio, name, tempo, grid, from_start, instrument, key, project, global, overwrite } => {
            let step = muser_engine::hum::parse_grid(&grid).map_err(fail)?;
            let path = std::path::Path::new(&audio);
            let mut r = muser_engine::hum::transcribe_file(path, muser_engine::hum::HumOptions { tempo, step, from_start }).map_err(fail)?;
            if let Some(k) = key {
                r.key = k;
            }
            let name = name.unwrap_or_else(|| {
                let stem = path.file_stem().map_or("hum".into(), |s| s.to_string_lossy().to_lowercase());
                stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
            });
            let l = loop_library::Loop::melody(&name, r.bars, tempo, Some(instrument.unwrap_or_else(|| "poly.juno_keys".into())), Some(r.key.clone()), r.notes);
            let base = base_of(&project);
            let saved = if global { loop_library::save(&l, overwrite) } else { loop_library::save_project(&l, base.as_deref(), overwrite) }.map_err(fail)?;
            println!("transcribed {} notes, {} bars, key {} (pitch was {:+} cents off overall; corrected) -> {}", l.notes.len(), l.bars, r.key, r.tuning_cents, saved.display());
            let p = l.project(None, None, 1).map_err(fail)?;
            let mut opts = muser_engine::abc_export::AbcOptions::new(muser_engine::abc_export::Dialect::Standard);
            opts.key = Some(r.key);
            if let Ok(abc) = muser_engine::abc_export::export(&p, &opts) {
                print!("{}", abc.text.lines().skip_while(|l| !l.starts_with("K:")).collect::<Vec<_>>().join("\n"));
                println!();
            }
            println!("`muser loop edit {}` edits it in the piano roll; `muser loop play {}` auditions it.", l.name, l.name);
            Ok(())
        }
        LoopCommand::Share { name, project, overwrite } => {
            let path = loop_library::share(&name, base_of(&project).as_deref(), overwrite).map_err(fail)?;
            println!("copied to global -> {}", path.display());
            Ok(())
        }
        LoopCommand::Edit { name, project, port, open, sample_rate } => crate::loop_edit::serve(name, base_of(&project), port, open, sample_rate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn show_produces_a_valid_clip() {
        let l = loop_library::load("dnb_two_step").unwrap();
        let clip = l.clip(8.0, 2);
        assert_eq!((clip["start"].as_f64(), clip["length"].as_f64(), clip["from"].as_str()), (Some(8.0), Some(8.0), Some("loop:dnb_two_step")));
        let p: Project = serde_json::from_value(json!({ "tempo": 174, "tracks": [{ "instrument": "drums.dnb", "clips": [clip] }] })).unwrap();
        muser_engine::validate(&p).unwrap();
        assert_eq!(p.tracks[0].clips[0].all_notes().unwrap().len(), 2 * l.grid.expand(l.beats()).unwrap().len());
    }
}
