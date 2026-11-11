//! muser CLI —— AI/用户驱动引擎的命令行接口。
//!
//! 这是「AI 接口层」的 CLI 形态。每个子命令都是一个明确的、可被 AI 通过 shell
//! 调用的工具。将来 MCP server 会把这些能力再包一层给 agent 直接调用。
//!
//! 子命令：
//! - `render <project.json> [-o out.wav]`  读工程 JSON，离线渲染出 WAV
//! - `validate <project.json>`             校验工程 JSON 是否合法
//! - `instruments [name] [--family F] [--json]`  列出乐器 / 查看某乐器详情
//! - `effects [name] [--json]`              列出效果器 / 查看某效果器参数
//! - `mix-report <project.json> [--json]`  各轨/总线/主总线响度，用来平衡音量
//! - `new [-o project.json]`                生成一个示例工程 JSON（供 AI 参考格式）

mod live;
mod loop_edit;
mod play;
mod poly;
mod clipedit;
mod self_update;
mod takes;
mod view;
mod web;
mod zipfile;

use std::fs;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use muser_core::{Clip, Note, Project, Track};
use muser_engine::{render_to_wav_with_progress, RenderSettings};

#[derive(Parser)]
#[command(name = "muser", version, about = "A headless music-arrangement engine CLI for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read a project JSON and render it offline to WAV.
    Render {
        /// Project file path (.json).
        project: String,
        /// Output path: .wav (16-bit) or .mp3 (320 kbit/s).
        #[arg(short, long, default_value = "out.wav")]
        output: String,
        /// Sample rate.
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Seconds of tail to render at the end.
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
        /// Temporarily override the project's random seed (the file is not changed): re-rolls humanize and random automation.
        #[arg(long)]
        seed: Option<u64>,
        /// Export stems to this directory instead of the full mix (one 24-bit WAV per unmuted track, with its effects and its sends to buses, without master effects or their automation). The full mix (-o .wav) is 16-bit.
        #[arg(long, value_name = "DIR")]
        stems: Option<String>,
    },
    /// Export a standard MIDI file (one MIDI track per instrument track; notes include humanize, legato and drummer expansion; pedal as CC64).
    Midi {
        /// Project file path.
        project: String,
        /// Output .mid path; default: the project's name with .mid.
        #[arg(short, long)]
        output: Option<String>,
        /// Temporarily override the project's random seed (the file is not changed): re-rolls humanize and the drummer.
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Export ABC notation (for models and notation software that read ABC): standard multi-voice ABC 2.1, or YuE2-native with --yue2.
    Abc {
        /// Project file path.
        project: String,
        /// Output .abc path; default: print to stdout.
        #[arg(short, long)]
        output: Option<String>,
        /// YuE2-native notation: two monophonic voices, Vocal / Ins, with chord symbols on Vocal (for cot=full; add --no-chords for cot=melody).
        #[arg(long)]
        yue2: bool,
        /// Write chord symbols (analyzed from the accompaniment; off by default in standard mode, on in YuE2).
        #[arg(long, conflicts_with = "no_chords")]
        chords: bool,
        /// Do not write chord symbols.
        #[arg(long)]
        no_chords: bool,
        /// Standard mode: export only this track (repeatable); default: all instrument tracks.
        #[arg(long = "track")]
        tracks: Vec<String>,
        /// YuE2: the track(s) carrying the sung melody; when the melody passes between several instruments, list them with commas (the highest note at each moment is taken).
        /// Default: picked by name (vocal / lead / melody...), otherwise the one sounding longest in the higher register.
        #[arg(long, value_delimiter = ',')]
        vocal: Vec<String>,
        /// YuE2: the track(s) for the instrumental line (comma-separated; default: all rests).
        #[arg(long, value_delimiter = ',')]
        ins: Vec<String>,
        /// Sections, written as % comments: e.g. intro:4,verse:8,chorus:8 (bar counts).
        #[arg(long)]
        sections: Option<String>,
        /// Key signature (e.g. G, F#m); default: the K: of the ABC in the clip, otherwise estimated from pitch.
        #[arg(long)]
        key: Option<String>,
        /// YuE2: shift the Vocal voice by this many octaves (e.g. 1: written in the lowercase note-name range).
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        vocal_octave: i32,
    },
    /// Listening report: per section, density, register, chords, repetition, clashes and suggestions; --audio also renders for loudness and band shares.
    Analyze {
        /// Project file path.
        project: String,
        /// Also render, adding each section's loudness, spectral centroid and low / mid / high shares (slower).
        #[arg(long)]
        audio: bool,
        #[arg(long)]
        json: bool,
    },
    /// Validate a project JSON.
    Validate {
        /// Project file path.
        project: String,
    },
    /// List the available instruments; with a name, show that instrument's details.
    Instruments {
        /// Instrument name (e.g. guitar.nylon / poly.moog_bass); default: list all.
        name: Option<String>,
        /// Only one family (drums / synth / guitar / bass / organ / mallet / strings / wind / choir / poly).
        #[arg(long)]
        family: Option<String>,
        /// Output as JSON (easy for AI to parse).
        #[arg(long)]
        json: bool,
    },
    /// List the available effects; with a name, show that effect's parameters, ranges, defaults and examples.
    Effects {
        /// Effect name (e.g. reverb); default: list all.
        name: Option<String>,
        /// Output as JSON (easy for AI to parse).
        #[arg(long)]
        json: bool,
    },
    /// Play the project once from the start on the local sound card, then exit: a quick listen to a draft (no web page).
    Play {
        /// Project file path.
        project: String,
        /// Temporarily override the project's random seed (re-rolls humanize / random automation); the file is not changed.
        #[arg(long)]
        seed: Option<u64>,
        /// Start at this many seconds.
        #[arg(long, default_value_t = 0.0)]
        from: f64,
    },
    /// Loop / motif library: edit opens the editor, list lists, show prints a clip to put in a project, play auditions, save stores a project clip as a loop, share copies to global.
    Loop {
        #[command(subcommand)]
        command: play::LoopCommand,
    },
    /// poly synth sounds: edit opens the sound editor, list / show fetch sounds from the sound library (for agents to put in a project).
    Poly {
        #[command(subcommand)]
        command: poly::PolyCommand,
    },
    /// Sounds on this machine: list the usable sounds (built-in, SFZ, instrument plugins), or "listen" to one and write a text card (cached locally).
    Sounds {
        #[command(subcommand)]
        command: SoundsCommand,
    },
    /// Make your own instrument from recordings: extract a small modal instrument from single-note audio files, and compare it with them.
    Yinse {
        #[command(subcommand)]
        command: YinseCommand,
    },
    /// External plugins (VST3 / CLAP / LV2): scan this machine, show info, list parameters.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Audition: set one parameter of an instrument or effect to several values in turn, render a test tone for each, and report loudness, brightness, attack, tail and other features.
    Audition {
        /// Instrument name (e.g. poly.moog_bass, sfz:/x.sfz, plugin:/x.clap#id); give this or --effect.
        #[arg(long)]
        instrument: Option<String>,
        /// Effect JSON (e.g. '{"type":"reverb"}' or '{"type":"plugin","path":"..."}'); give this or --instrument.
        #[arg(long)]
        effect: Option<String>,
        /// Instrument that makes the sound when testing an effect.
        #[arg(long, default_value = "synth.poly")]
        source: String,
        /// Base parameters of the instrument (a JSON object).
        #[arg(long)]
        params: Option<String>,
        /// Parameter to try (a built-in parameter name, an effect field name or a plugin parameter name). Omit to render just the unchanged version.
        #[arg(long)]
        param: Option<String>,
        /// Values, comma-separated; numbers are treated as numbers, the rest is passed to the plugin as text to parse (e.g. "200 Hz,2000 Hz").
        /// Text that a plugin displays as a plain number goes in double quotes (e.g. '"200.0","800.0"'); for negative numbers write --values=-12,0.
        #[arg(long, value_delimiter = ',')]
        values: Vec<String>,
        /// Test pitch (MIDI; GM drum keys for drum kits).
        #[arg(long, default_value_t = 60)]
        pitch: u8,
        /// Velocity 0-1.
        #[arg(long, default_value_t = 0.8)]
        velocity: f32,
        /// Note length (seconds).
        #[arg(long, default_value_t = 1.0)]
        hold: f64,
        /// How long to keep recording after the note ends (seconds).
        #[arg(long, default_value_t = 1.5)]
        tail: f64,
        /// Directory to write each version to as a wav.
        #[arg(long)]
        out: Option<String>,
        /// Record the audition results into the plugin's user profile (external plugins only).
        #[arg(long)]
        record: bool,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// A playback page for people: starts a local server (listening on 127.0.0.1 only); play in the browser, see the timeline, adjust volume / mute / solo,
    /// changes are re-rendered by the engine; Save writes back to the project. If the project file changes it is re-rendered and the page refreshes automatically.
    /// When listening on this machine add --device: sound comes straight from this machine's sound card and volume / mute / solo take effect instantly (the page is only a remote control);
    /// remotely (over SSH port forwarding) use the default browser playback.
    View {
        /// Project file path.
        project: String,
        /// Port.
        #[arg(long, default_value_t = 8737)]
        port: u16,
        /// Open the page in the system browser after starting.
        #[arg(long)]
        open: bool,
        /// Sample rate.
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Seconds of tail to render at the end.
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
        /// Play on the local sound card (sound comes out of the machine running muser).
        #[arg(long)]
        device: bool,
    },

    /// Live session: the agent writes section after section ahead, and muser keeps the music going (out of the local sound card) without gaps.
    /// `muser live start <template>` stays resident (started by the agent, in the background); send / status / wait / autoplay / stop control it.
    Live {
        #[command(subcommand)]
        command: LiveCommand,
    },

    /// Mix report: render the whole piece and list the loudness of every track / bus / master (dry before the effect chain, after the fader),
    /// each track's difference from the loudest track, how much peak the master effect chain removed, and what to watch out for. Use it to balance each track's gain_db.
    MixReport {
        /// Project file path.
        project: String,
        /// Sample rate.
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Seconds of tail to render at the end.
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
        /// Output as JSON (easy for AI to parse).
        #[arg(long)]
        json: bool,
    },
    /// Inspect an audio file: format, duration, sample rate, channels; with --tempo also converts to beats (for the length of an audio clip).
    Audio {
        /// Audio file (WAV, AIFF, FLAC, Ogg Vorbis, Opus, MP3; on macOS / Windows also AAC, ALAC).
        file: String,
        /// Convert the duration to beats at this tempo.
        #[arg(long)]
        tempo: Option<f64>,
        #[arg(long)]
        json: bool,
    },
    /// Print the project's signal flow: where each track / bus goes, send amounts, sidechains, processing order.
    Routing {
        /// Project file path.
        project: String,
    },
    /// Generate an example project JSON (a format reference for AI).
    New {
        /// Output path; default: print to stdout.
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Export for a DAW (Ableton Live, Cubase, Logic...): a folder with the MIDI of all tracks, a stem WAV per track (24-bit), the full mix and import notes.
    ExportDaw {
        /// Project file path (.json).
        project: String,
        /// Output folder.
        #[arg(short, long)]
        output: String,
        /// Sample rate.
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Seconds of tail to render at the end.
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
    },
    /// Export data for visualization (JSON): notes per track (seconds), sections, automation curves; with --fps also renders, adding per-frame overall / low-frequency volume envelopes (dB).
    ExportData {
        /// Project file path (.json).
        project: String,
        /// Output path; default: print to stdout.
        #[arg(short, long)]
        output: Option<String>,
        /// Volume envelope frames per second (e.g. 30); not rendered if omitted.
        #[arg(long)]
        fps: Option<u32>,
    },
    /// Split and move clips on audio tracks (edits the project file): split cuts at a beat, move shifts a clip to a beat or by some milliseconds.
    Clip {
        #[command(subcommand)]
        command: ClipCommand,
    },
    /// The vocal takes (recorded in muser view, in recordings/): length, loudness, the part actually sung, whether the project uses it; --clean deletes unused ones.
    Takes {
        /// Project file path (.json).
        project: String,
        /// Delete recordings the project does not use (ask the user first).
        #[arg(long)]
        clean: bool,
        /// Check timing line by line: how many milliseconds the project's vocals differ from the guide melody (the track with lyric).
        #[arg(long)]
        timing: bool,
    },
    /// Convert a project written for an older muser to the current version (it sounds the same as in the old version). By default only lists the changes; add --write to write them back.
    Migrate {
        /// Project JSON file(s) (one or more).
        #[arg(required = true)]
        projects: Vec<String>,
        /// The version the project was written for, e.g. 0.3.0.
        #[arg(long)]
        from: String,
        /// Write the changes back to the project file.
        #[arg(long)]
        write: bool,
    },
    /// Update to the latest version on GitHub (runs the official install script: download, SHA256 check, version switch; needs curl).
    SelfUpdate {
        /// Only check for a new version; do not install.
        #[arg(long)]
        check: bool,
        /// Install this version (e.g. v0.4.0) instead of the latest.
        #[arg(long)]
        version: Option<String>,
    },
}

#[derive(Subcommand)]
enum ClipCommand {
    /// Split the audio clip at beat `at` into two.
    Split {
        project: String,
        /// Track name.
        #[arg(long)]
        track: String,
        /// Beat to cut at (project beats, from 0).
        #[arg(long)]
        at: f64,
    },
    /// Move an audio clip: --to starts it at a beat, or --by-ms shifts it by milliseconds (negative = earlier).
    Move {
        project: String,
        #[arg(long)]
        track: String,
        /// Clip index (the order of this track's clips, from 0).
        #[arg(long)]
        clip: usize,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long, allow_hyphen_values = true)]
        by_ms: Option<f64>,
    },
}

#[derive(Subcommand)]
enum YinseCommand {
    /// Extract an instrument from single-note recordings (one note per file; struck or plucked sounds that ring away by themselves).
    /// Pitch comes from the file name (A4, Eb3, C#5 or a MIDI number) or is measured; velocity from the name (v80, vel100, v0.7, pp ... ff)
    /// or, per pitch, from the loudness order. The result is a small JSON of partial tables (no audio): play it as "instrument": "yinse:<file>".
    Extract {
        /// Audio files or folders of them (wav / flac / aiff / ogg / mp3).
        #[arg(required = true)]
        inputs: Vec<std::path::PathBuf>,
        /// Output file (suggested extension .yinse.json).
        #[arg(short, long)]
        output: std::path::PathBuf,
        /// Instrument name (default: the output file name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Play every recording's pitch and velocity on an extracted instrument and compare: spectral difference, decay and stereo.
    Compare {
        /// The extracted instrument (.yinse.json).
        instrument: String,
        /// The recordings (files or folders).
        #[arg(required = true)]
        inputs: Vec<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SoundsCommand {
    /// List the sounds usable on this machine: built-in instruments, SFZ in $MUSER_SFZ_PATH, scanned instrument plugins; ones already listened to carry a one-line card description.
    List {
        /// Only one kind: builtin / sfz / plugin.
        #[arg(long)]
        kind: Option<String>,
        /// Keyword filter (name, category, description, path; every space-separated word must appear).
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List an instrument plugin's built-in sounds (VST3 standard presets, Surge .fxp, Kontakt .nki...) and whether they are directly usable (auto / captured / manual).
    Presets {
        /// Plugin: plugin:<path>#<id>, or a path.
        plugin: String,
        /// Keyword filter (name, category, sound library, tags).
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Get the plugin state file for a built-in sound (in the local sound library; generated directly when it can be converted automatically) and print how to write it in a project.
    State {
        plugin: String,
        /// Sound name or file path (listed by `sounds presets`).
        preset: String,
    },
    /// Save a plugin state into the local sound library as one of "my sounds" (e.g. a state saved by `muser plugin edit`, or a tuned loop kit); after that
    /// `sounds presets` and the sound drawer in muser view can use it directly. Overwrites a sound of the same name.
    Save {
        plugin: String,
        /// Name (e.g. "Alter Kit").
        name: String,
        /// State file (.muserstate).
        state: String,
    },
    /// Identify which built-in sound a plugin state has loaded (compares the sound files byte for byte; works for Kontakt .nki).
    Identify {
        plugin: String,
        /// State file (.muserstate).
        state: String,
    },
    /// Capture (macOS): open the plugin window and load this sound in it (with the plugin's own browser / search box);
    /// after the window is closed the state is saved to the local sound library and `sounds state` can use it directly. A person clicks, or the agent uses computer use.
    Capture {
        plugin: String,
        preset: String,
    },
    /// Listen to a sound: render probe tones (5 registers + soft velocity; common drum keys for drum kits) and measure brightness, attack, sustain, tail, range, velocity response, stereo and pitch,
    /// then write a text card and cache it locally (read from the cache while the sound, parameters and files are unchanged).
    Describe {
        /// Sound: a built-in instrument name, sfz:<path> or plugin:<path>#<id>.
        sound: String,
        /// instrument_params (JSON), e.g. {"@state": "<file>"} for a plugin, {"cc": {"1": 64}} for SFZ.
        #[arg(long)]
        params: Option<String>,
        /// Probe as a drum kit (default: detected automatically).
        #[arg(long)]
        drums: bool,
        /// Ignore the cache and listen again.
        #[arg(long)]
        refresh: bool,
        /// Key map: hold each key for some bars and measure whether it sounds, whether it is a loop that follows the tempo, how many bars per cycle, how dense and how loud (loop kits, drum kits, split sounds).
        #[arg(long)]
        keys: bool,
        /// Range of the key map (MIDI pitch, C4 = 60).
        #[arg(long, default_value_t = 36)]
        from: u8,
        #[arg(long, default_value_t = 96)]
        to: u8,
        /// Tempo for the key map (loops are stretched to this tempo).
        #[arg(long, default_value_t = 120.0)]
        tempo: f64,
        /// Bars to hold each key.
        #[arg(long, default_value_t = 4.0)]
        hold: f64,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PluginCommand {
    /// Scan the standard plugin directories on this machine (each plugin is read in a separate process; results are cached).
    Scan {
        /// Only one kind: instrument / effect.
        #[arg(long)]
        kind: Option<String>,
        /// Ignore the cache and rescan.
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Plugin info: name, vendor, kind, inputs and outputs, parameter count and groups.
    Info {
        /// Plugin path, optionally with #plugin-id.
        plugin: String,
        #[arg(long)]
        json: bool,
    },
    /// List plugin parameters (name, range, default, current value and text).
    Params {
        /// Plugin path, optionally with #plugin-id.
        plugin: String,
        /// Keyword filter (space-separated, all must match; case-insensitive), e.g. "filter cutoff".
        #[arg(long)]
        filter: Option<String>,
        /// Include parameters the plugin marks as hidden.
        #[arg(long)]
        all: bool,
        /// Apply these parameters first (a JSON object), e.g. '{"FX Type": "Reverb 2"}', then list (useful when the meaning of the parameters depends on them).
        #[arg(long)]
        params: Option<String>,
        /// Maximum number to show (default 50; 0 = unlimited).
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Plugin profile: a data file of common parameter aliases, presets and know-how (matched by plugin id, shareable).
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Save the plugin's current state (sound) to a file: applies --params first (including the profile setup / @preset); load it in a project with "@state": "<file>".
    SaveState {
        /// Plugin path, optionally with #plugin-id.
        plugin: String,
        /// Parameters to apply (a JSON object).
        #[arg(long)]
        params: Option<String>,
        /// Output file (suggested extension .muserstate).
        #[arg(short, long)]
        output: String,
    },
    /// Open the plugin's own UI (macOS, VST3), pick a sound and adjust parameters in it; the state is saved to a file when the window is closed.
    Edit {
        /// Plugin path, optionally with #plugin-id.
        plugin: String,
        /// Parameters to apply first (a JSON object).
        #[arg(long)]
        params: Option<String>,
        /// Output file (suggested extension .muserstate).
        #[arg(short, long)]
        output: String,
        /// Window title.
        #[arg(long)]
        title: Option<String>,
        /// Driven by muser view (reads JSON commands on stdin, reports on stdout).
        #[arg(long, hide = true)]
        serve: bool,
    },
    /// List the text values a parameter accepts (for choice parameters, all the options, e.g. filter type, waveform).
    Values {
        /// Plugin path, optionally with #plugin-id.
        plugin: String,
        /// Parameter name or id.
        param: String,
        /// Number of points sampled along the range.
        #[arg(long, default_value_t = 400)]
        samples: usize,
        /// Apply these parameters first (a JSON object).
        #[arg(long)]
        params: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// List all available profiles (user directory, MUSER_PROFILE_PATH, built-in).
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show a plugin's profile (a plugin path, or a profile .json file).
    Show {
        target: String,
        /// Output as Markdown (skill document).
        #[arg(long)]
        markdown: bool,
        #[arg(long)]
        json: bool,
    },
    /// Auto-generate a draft profile from the plugin.
    Draft {
        plugin: String,
        /// Audition the common numeric parameters and write the observations into the draft (slower).
        #[arg(long)]
        audition: bool,
        /// Save to the user profile directory (an existing file of the same name is overwritten).
        #[arg(long)]
        save: bool,
        /// Write to this file; if neither is given, print.
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Validate a profile file; when a plugin is given, also check that the parameter names and presets can be applied to it.
    Check {
        file: String,
        #[arg(long)]
        plugin: Option<String>,
    },
    /// Show the profile search paths.
    Path,
}

fn main() {
    // 本进程被插件扫描器以子进程方式启动时，读完插件即退出。
    muser_engine::plugin::serve_scanner_if_asked();
    let code = match run_with_stack() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e:?}");
            1
        }
    };
    muser_engine::plugin::exit(code)
}

/// Windows 主线程只有 1 MB 栈，调试构建解析这么多子命令就会栈溢出：在一个 16 MB 栈的线程里跑。
/// 其它系统留在主线程（macOS 的插件窗口必须在主线程上）。
#[cfg(windows)]
fn run_with_stack() -> Result<()> {
    std::thread::Builder::new().stack_size(16 << 20).spawn(run).context("cannot start the main thread")?.join().unwrap_or_else(|_| Err(anyhow::anyhow!("main thread crashed")))
}

#[cfg(not(windows))]
fn run_with_stack() -> Result<()> {
    run()
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render {
            project,
            output,
            sample_rate,
            tail,
            seed,
            stems,
        } => cmd_render(&project, &output, sample_rate, tail, seed, stems.as_deref()),
        Command::Validate { project } => cmd_validate(&project),
        Command::Instruments { name, family, json } => {
            cmd_instruments(name.as_deref(), family.as_deref(), json)
        }
        Command::Effects { name, json } => cmd_effects(name.as_deref(), json),
        Command::New { output } => cmd_new(output.as_deref()),
        Command::SelfUpdate { check, version } => self_update::run(check, version),
        Command::Migrate { projects, from, write } => cmd_migrate(&projects, &from, write),
        Command::Takes { project, clean, timing } => takes::run(&project, clean, timing),
        Command::Clip { command } => match command {
            ClipCommand::Split { project, track, at } => clipedit::run(&project, &track, Some(at), None, None, None),
            ClipCommand::Move { project, track, clip, to, by_ms } => clipedit::run(&project, &track, None, Some(clip), to, by_ms),
        },
        Command::ExportDaw { project, output, sample_rate, tail } => {
            let p = load_project(&project)?;
            let settings = RenderSettings { sample_rate, tail_seconds: tail, ..RenderSettings::default() };
            let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
            let mut progress = |name: &str, done: usize, total: usize| {
                if tty {
                    eprint!("\rstems {:3.0}% ({name}){:24}", done as f64 * 100.0 / total.max(1) as f64, "");
                }
            };
            let r = muser_engine::daw::export_daw(&p, std::path::Path::new(&output), settings, &mut progress);
            if tty {
                eprint!("\r{:80}\r", "");
            }
            let r = r.with_context(|| format!("export failed: {project}"))?;
            println!("exported for DAW -> {output}/");
            println!("  MIDI: {}", r.midi.display());
            println!("  stems: {} in {}/stems/ (24-bit, aligned from bar 1)", r.stems.len(), output);
            println!("  mix: {}", r.mix.display());
            println!("  import notes: {}", r.guide.display());
            if !r.midi_skipped.is_empty() {
                println!("  ({} are audio tracks: no MIDI, stems only)", r.midi_skipped.join(", "));
            }
            print_warnings(&p);
            Ok(())
        }
        Command::ExportData { project, output, fps } => {
            let p = load_project(&project)?;
            let data = muser_engine::export_data::export(&p, fps, RenderSettings::default()).with_context(|| format!("export failed: {project}"))?;
            let text = serde_json::to_string(&data)? + "\n";
            match output {
                Some(path) => {
                    fs::write(&path, text)?;
                    println!("exported visualization data -> {path}");
                }
                None => print!("{text}"),
            }
            Ok(())
        }
        Command::Routing { project } => cmd_routing(&project),
        Command::Analyze { project, audio, json } => cmd_analyze(&project, audio, json),
        Command::Abc { project, output, yue2, chords, no_chords, tracks, vocal, ins, sections, key, vocal_octave } => {
            use muser_engine::abc_export::{AbcOptions, Dialect};
            let mut opts = AbcOptions::new(if yue2 { Dialect::Yue2 } else { Dialect::Standard });
            opts.chords = (opts.chords || chords) && !no_chords;
            (opts.tracks, opts.vocal, opts.ins, opts.key, opts.vocal_octave) = (tracks, vocal, ins, key, vocal_octave);
            if let Some(s) = sections {
                opts.sections = s
                    .split(',')
                    .map(|part| {
                        let (name, bars) = part.trim().rsplit_once(':').ok_or_else(|| anyhow::anyhow!("--sections takes name:bars, comma-separated, e.g. verse:8,chorus:8"))?;
                        Ok((name.trim().to_string(), bars.trim().parse::<u32>().with_context(|| format!("the bar count in '{part}' is not an integer"))?))
                    })
                    .collect::<Result<_>>()?;
            }
            cmd_abc(&project, output.as_deref(), &opts)
        }
        Command::Midi { project, output, seed } => cmd_midi(&project, output, seed),
        Command::Audio { file, tempo, json } => cmd_audio(&file, tempo, json),
        Command::MixReport { project, sample_rate, tail, json } => cmd_mix_report(&project, sample_rate, tail, json),
        Command::Live { command } => cmd_live(command),
        Command::View { project, port, open, sample_rate, tail, device } => {
            let settings = RenderSettings { sample_rate, tail_seconds: tail, ..RenderSettings::default() };
            // 网页服务在后台线程，主线程留给插件（见 run_with_main_loop）。
            muser_engine::plugin::run_with_main_loop(move || match view::serve(&project, settings, port, open, device) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("Error: {e:?}");
                    1
                }
            })
        }
        Command::Plugin { command } => cmd_plugin(command),
        Command::Sounds { command } => cmd_sounds(command),
        Command::Yinse { command } => cmd_yinse(command),
        Command::Poly { command } => poly::run(command),
        Command::Loop { command } => play::run_loop(command),
        Command::Play { project, seed, from } => {
            let mut p = load_project(&project)?;
            if let Some(s) = seed {
                p.seed = s;
            }
            play::play(p, from)
        }
        Command::Audition { instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, record, json } => {
            cmd_audition(instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, record, json)
        }
    }
}

fn cmd_analyze(project_path: &str, audio: bool, json: bool) -> Result<()> {
    let project = load_project(project_path)?;
    let r = muser_engine::analyze::analyze(&project, audio, RenderSettings::default())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    println!("{} bpm, {} bars, key {}", r.tempo, r.bars, r.key);
    // 能量曲线：每段一格。
    const BLOCKS: [char; 11] = [' ', '▁', '▂', '▃', '▃', '▄', '▅', '▆', '▆', '▇', '█'];
    let curve: String = r.sections.iter().map(|s| BLOCKS[usize::from(s.energy.min(10))]).collect();
    let labels: Vec<String> = r.sections.iter().map(|s| format!("{}{} {}", s.name, if s.occurrence > 1 { format!("#{}", s.occurrence) } else { String::new() }, s.energy)).collect();
    println!("energy |{curve}|  {}", labels.join(" · "));
    for s in &r.sections {
        let label = if s.occurrence > 1 { format!("{} #{}", s.name, s.occurrence) } else { s.name.clone() };
        println!();
        print!("[{label}] bars {}-{}  energy {}  density {} notes/bar  repetition {:.0}%  chords {}/bar", s.bar, s.bar + s.bars - 1, s.energy, s.density, s.repeated * 100.0, s.chord_changes_per_bar);
        match &s.audio {
            Some(a) => {
                println!("  {:.1} LUFS  centroid {} Hz  low/mid/high {}/{}/{}%", a.lufs, a.centroid_hz, a.low_pct, a.mid_pct, a.high_pct);
                if a.rms_db < a.lufs - 6.0 {
                    println!("  average level {:.1} dB: most of this section is very quiet (LUFS only counts the loud parts, which a few hits pulled up)", a.rms_db);
                }
            }
            None => println!(),
        }
        let mut chords = s.chords.clone();
        chords.dedup();
        if !chords.is_empty() {
            println!("  chords: {}", chords.join(" "));
        }
        for t in &s.tracks {
            println!("  {:<14} {:>5} notes/bar  sounding {:>3.0}%  {}", t.name, t.onsets_per_bar, t.sounding * 100.0, t.range.as_deref().unwrap_or(""));
        }
        for c in &s.clashes {
            println!("  ! bar {} beat {}: {}", c.bar, c.beat, c.what);
        }
    }
    if !r.tells.is_empty() {
        println!();
        let hits = r.tells.iter().filter(|t| t.hit).count();
        println!("AI-sounding check ({hits} / {} hit):", r.tells.len());
        for t in &r.tells {
            println!("  {} #{:<2} {}{}", if t.hit { "x" } else { "ok" }, t.id, t.name, if t.hit { String::new() } else { format!(": {}", t.detail) });
        }
    }
    println!();
    println!("suggestions:");
    for f in &r.findings {
        println!("  - {f}");
    }
    Ok(())
}

fn cmd_abc(project_path: &str, output: Option<&str>, opts: &muser_engine::abc_export::AbcOptions) -> Result<()> {
    let project = load_project(project_path)?;
    let out = muser_engine::abc_export::export(&project, opts)?;
    match output {
        Some(path) => {
            fs::write(path, &out.text).with_context(|| format!("failed to write ABC: {path}"))?;
            println!("exported '{}' -> {path}", project.name);
            for n in &out.notes {
                println!("  {n}");
            }
        }
        None => {
            print!("{}", out.text);
            for n in &out.notes {
                eprintln!("{n}");
            }
        }
    }
    Ok(())
}

fn cmd_midi(project_path: &str, output: Option<String>, seed: Option<u64>) -> Result<()> {
    let mut project = load_project(project_path)?;
    if let Some(s) = seed {
        project.seed = s;
    }
    let out = output.unwrap_or_else(|| {
        let stem = project_path.strip_suffix(".json").unwrap_or(project_path);
        format!("{}.mid", stem.strip_suffix(".muser").unwrap_or(stem))
    });
    let m = muser_engine::midi::export(&project)?;
    fs::write(&out, &m.bytes).with_context(|| format!("failed to write MIDI: {out}"))?;
    let n = project.tracks.len() - m.skipped.len();
    println!("exported '{}' -> {out}: {n} MIDI track(s), {} bytes", project.name, m.bytes.len());
    if !m.skipped.is_empty() {
        println!("audio tracks not exported (audio cannot go into MIDI): {}", m.skipped.join(", "));
    }
    Ok(())
}

fn cmd_audio(file: &str, tempo: Option<f64>, json: bool) -> Result<()> {
    let path = std::path::PathBuf::from(&*muser_engine::plugin::expand_home(file));
    let (format, seconds, sample_rate, channels) = muser_engine::audio::info(&path).map_err(|e| anyhow::anyhow!(e))?;
    let beats = tempo.map(|t| seconds * t / 60.0);
    if json {
        let v = serde_json::json!({ "file": file, "format": format, "seconds": seconds, "sample_rate": sample_rate, "channels": channels, "beats": beats });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{file}: {format}, {seconds:.3} s, {sample_rate} Hz, {channels} channel(s)");
        match (beats, tempo) {
            (Some(b), Some(t)) => println!(", {b:.3} beats at {t} bpm"),
            _ => println!(),
        }
    }
    Ok(())
}

fn load_project(path: &str) -> Result<Project> {
    let text = fs::read_to_string(path).with_context(|| format!("failed to read the project file: {path}"))?;
    let project = Project::from_json(&text).with_context(|| format!("failed to parse the project JSON: {path}"))?;
    Ok(project.at_file(path))
}

fn cmd_render(project_path: &str, output: &str, sample_rate: u32, tail: f64, seed: Option<u64>, stems: Option<&str>) -> Result<()> {
    let mut project = load_project(project_path)?;
    if let Some(s) = seed {
        project.seed = s;
    }
    let settings = RenderSettings {
        sample_rate,
        tail_seconds: tail,
        ..RenderSettings::default()
    };
    // 在终端里原地显示进度（大工程、调试构建可能要渲染很久）；输出被管道/AI 捕获时不打印。
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let (sr, t0) = (f64::from(sample_rate), std::time::Instant::now());
    let mut last = std::time::Instant::now();
    let mut progress = |done: usize, total: usize| {
        if tty && (last.elapsed().as_millis() >= 200 || done == total) {
            last = std::time::Instant::now();
            eprint!("\rrendering {:3.0}% ({:.1}s / {:.1}s, elapsed {:.1}s)", done as f64 * 100.0 / total.max(1) as f64, done as f64 / sr, total as f64 / sr, t0.elapsed().as_secs_f64());
        }
    };
    if let Some(dir) = stems {
        let mut stem_progress = |name: &str, done: usize, total: usize| {
            if tty && (last.elapsed().as_millis() >= 200 || done == total) {
                last = std::time::Instant::now();
                eprint!("\rstems {:3.0}% ({name}, elapsed {:.1}s){:20}", done as f64 * 100.0 / total.max(1) as f64, t0.elapsed().as_secs_f64(), "");
            }
        };
        let result = muser_engine::render::render_stems(&project, std::path::Path::new(dir), settings, &mut stem_progress);
        if tty {
            eprint!("\r{:80}\r", "");
        }
        let list = result.with_context(|| format!("stem export failed: {project_path}"))?;
        println!("exported stems of '{}' -> {dir}/ ({} file(s), without master effects, all the same length, aligned from 0):", project.name, list.len());
        for s in &list {
            println!("  {}  {}  peak {:.1} dBFS{}", s.path.display(), s.track, s.peak_db, if s.peak_db > 0.0 { "  WARNING clipping" } else { "" });
        }
        print_warnings(&project);
        return Ok(());
    }
    let result = render_to_wav_with_progress(&project, output, settings, &mut progress);
    if tty {
        // 用空格覆盖进度行（老式 Windows 控制台不认 ANSI 清行）。
        eprint!("\r{:80}\r", "");
    }
    let (seconds, peak_db) = result.with_context(|| format!("render failed: {project_path}"))?;
    println!(
        "rendered '{}' -> {output}: {:.2}s, peak {:.1} dBFS",
        project.name, seconds, peak_db
    );
    if peak_db > 0.0 {
        println!("WARNING master clipping (peak {peak_db:+.1} dBFS): lower master_gain_db or the tracks' gain_db, or put a limiter last on the master (ceiling_db -1). Details: muser mix-report {project_path}");
    }
    print_warnings(&project);
    Ok(())
}

fn cmd_validate(project_path: &str) -> Result<()> {
    let project = load_project(project_path)?;
    muser_engine::validate(&project).with_context(|| "project validation failed")?;
    println!(
        "OK: '{}' is valid - tempo {}, {} track(s), length {:.2} beats",
        project.name,
        project.tempo,
        project.tracks.len(),
        project.duration_beats().0
    );
    print_warnings(&project);
    Ok(())
}

/// 不妨碍渲染、但多半不是本意的地方（不改退出码）。
fn print_warnings(project: &Project) {
    for w in muser_engine::warnings(project) {
        println!("WARNING {w}");
    }
}

fn note_label(n: muser_engine::NoteConvention) -> &'static str {
    match n {
        muser_engine::NoteConvention::GmDrums => "GM drum keys",
        muser_engine::NoteConvention::Pitched => "scale pitches",
    }
}

fn cmd_instruments(name: Option<&str>, family: Option<&str>, json: bool) -> Result<()> {
    if let Some(name) = name {
        let info = muser_engine::describe_instrument(name)
            .with_context(|| format!("unknown instrument '{name}'; see `muser instruments` for all"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&info)?);
        } else {
            println!("{}", info.name);
            println!("  family:      {}", info.family);
            println!("  pitches:     {}", note_label(info.notes));
            println!("  range:       {} - {}", info.range.0, info.range.1);
            println!("  polyphony:   {}", info.polyphony);
            println!("  description: {}", info.description);
            if !info.params.is_empty() {
                println!("  adjustable parameters (instrument_params):");
                for p in &info.params {
                    let spec = serde_json::to_value(p.spec)?;
                    let range = match spec["type"].as_str() {
                        Some("number") => {
                            let unit = spec["unit"].as_str().unwrap_or("");
                            let mut r = format!("{} – {}{}", spec["min"], spec["max"], if unit.is_empty() { String::new() } else { format!(" {unit}") });
                            if spec["integer"] == true { r.push_str(" (integer)"); }
                            if let Some(n) = spec["names"].as_array() {
                                r.push_str(&format!(", or {}", n.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("/")));
                            }
                            r
                        }
                        Some("choice") => spec["choices"].as_array().map(|c| c.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" / ")).unwrap_or_default(),
                        Some("bool") => "true / false".into(),
                        Some("drawbars") => "a 9-digit string, each digit 0-8".into(),
                        _ => String::new(),
                    };
                    let current = if p.current.is_null() { String::new() } else { format!("  current {}", p.current) };
                    let auto = match p.automation {
                        muser_engine::Automatable::Live => "  [automatable]",
                        muser_engine::Automatable::NextNote => "  [automatable; takes effect on the next note]",
                        muser_engine::Automatable::No => "",
                    };
                    println!("    {:<16} {range}{current}{auto}", p.spec.name);
                    println!("    {:<16} {}", "", p.spec.description);
                }
            }
            if !info.params_note.is_empty() {
                println!("  description: {}", info.params_note);
            }
        }
        return Ok(());
    }
    let list: Vec<_> = muser_engine::instrument_catalog()
        .into_iter()
        .filter(|i| match family {
            Some(f) => i.family == f || i.family.starts_with(&format!("{f}/")),
            None => true,
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let mut current = String::new();
    for info in &list {
        let group = info.family.split('/').next().unwrap_or("").to_string();
        if group != current {
            println!("\n[{group}]");
            current = group;
        }
        println!("  {:<28} {}", info.name, info.description);
    }
    println!(
        "\n{} in total, level-calibrated: at gain_db 0 they are roughly equally loud (reference phrase about -20 LUFS). You can also load an SFZ sound library with sfz:/path/to.sfz, or a third-party VSTi with plugin:/path/to.vst3 (not calibrated; `muser sounds describe` tells how much louder/quieter than the built-in instruments it is).\n`muser instruments <name>` shows details and adjustable parameters.",
        list.len()
    );
    Ok(())
}

fn cmd_effects(name: Option<&str>, json: bool) -> Result<()> {
    if let Some(name) = name {
        let spec = muser_core::effect_spec(name)
            .with_context(|| format!("unknown effect '{name}'; see `muser effects` for all"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(spec)?);
            return Ok(());
        }
        println!("{} — {}", spec.name, spec.summary);
        println!("placement: {}", spec.placement);
        if !spec.params.is_empty() || !spec.choices.is_empty() {
            println!("parameters:");
        }
        for c in spec.choices {
            println!("  {:<14} default {:<10} choices {}", c.name, c.default, c.choices.join(" / "));
            println!("  {:<14} {}", "", c.description);
        }
        for p in spec.params {
            let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
            println!(
                "  {:<14} default {:<10} range [{}, {}]{}",
                p.name, p.default, p.min, p.max, unit
            );
            println!("  {:<14} {}", "", p.description);
        }
        println!("example: {}", spec.example);
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(muser_core::EFFECT_SPECS)?);
        return Ok(());
    }
    println!("Available effects (for track.effects or master_effects):");
    for spec in muser_core::EFFECT_SPECS {
        println!("  {:<12} {}", spec.name, spec.summary);
    }
    println!("\n`muser effects <name>` shows parameters, ranges and examples.");
    Ok(())
}

fn cmd_profile(command: ProfileCommand) -> Result<()> {
    use muser_engine::{plugin, profile};
    let fail = |e: String| anyhow::anyhow!(e);
    let load_target = |target: &str| -> Result<profile::Loaded> {
        if target.ends_with(".json") && std::path::Path::new(target).is_file() {
            return Ok(profile::Loaded {
                origin: profile::Origin::File,
                path: Some(target.into()),
                profile: profile::load_file(std::path::Path::new(target)).map_err(fail)?,
            });
        }
        let (id, name, _) = plugin::plugin_identity(target).map_err(fail)?;
        profile::find(&id)
            .map(|l| (*l).clone())
            .ok_or_else(|| anyhow::anyhow!("{name} ({id}) has no profile yet; generate one with `muser plugin profile draft '{target}' --save`"))
    };
    match command {
        ProfileCommand::Path => {
            if let Ok(v) = std::env::var("MUSER_PROFILE_PATH") {
                println!("MUSER_PROFILE_PATH: {v}");
            }
            println!("user profile directory: {}", profile::user_dir().display());
            println!("muser also ships built-in profiles (lowest priority).");
        }
        ProfileCommand::List { json } => {
            let all = profile::all();
            if json {
                let list: Vec<_> = all.iter().map(|l| serde_json::json!({
                    "name": l.profile.plugin.name, "ids": l.profile.plugin.ids, "origin": l.origin, "path": l.path,
                    "tested_version": l.profile.source.tested_version, "params": l.profile.params.len(), "presets": l.profile.presets.len()
                })).collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            for l in &all {
                println!("{:<24} {:>3} params {:>2} presets  {:?}  {}", l.profile.plugin.name, l.profile.params.len(), l.profile.presets.len(), l.origin, l.path.as_deref().unwrap_or(""));
            }
        }
        ProfileCommand::Show { target, markdown, json } => {
            let l = load_target(&target)?;
            if json {
                println!("{}", l.profile.to_json());
            } else if markdown {
                print!("{}", profile::render_markdown(&l));
            } else {
                let p = &l.profile;
                println!("{} - {} ({:?}, {})", p.plugin.name, p.plugin.vendor, l.origin, l.path.as_deref().unwrap_or(""));
                if !p.setup.is_empty() {
                    println!("  setup: {}", serde_json::Value::Object(p.setup.clone()));
                }
                for q in &p.params {
                    println!("  {:<14} {}", q.alias.as_deref().unwrap_or("-"), q.name);
                }
                if !p.presets.is_empty() {
                    println!("  presets: {}", p.presets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", "));
                }
                println!("\n`--markdown` prints the skill document.");
            }
        }
        ProfileCommand::Draft { plugin: spec, audition, save, output } => {
            let p = plugin::draft_profile(&spec, audition).map_err(fail)?;
            if save {
                let path = profile::save_user(&p).map_err(fail)?;
                println!("saved draft -> {} ({} parameter(s))", path.display(), p.params.len());
            } else if let Some(out) = output {
                std::fs::write(&out, p.to_json()).with_context(|| format!("failed to write {out}"))?;
                println!("wrote draft -> {out} ({} parameter(s))", p.params.len());
            } else {
                print!("{}", p.to_json());
            }
        }
        ProfileCommand::Check { file, plugin: spec } => {
            let p = profile::load_file(std::path::Path::new(&file)).map_err(fail)?;
            println!("structure OK: {}, {} params, {} presets", p.plugin.name, p.params.len(), p.presets.len());
            if let Some(spec) = spec {
                let (id, _, version) = plugin::plugin_identity(&spec).map_err(fail)?;
                if !p.matches(&id) {
                    anyhow::bail!("the profile's plugin.ids does not include this plugin's id '{id}'");
                }
                let mut problems = Vec::new();
                let have: std::collections::HashSet<String> =
                    plugin::params(&spec, None, true, &Default::default()).map_err(fail)?.into_iter().map(|x| x.name).collect();
                for q in &p.params {
                    if !have.contains(&q.name) {
                        problems.push(format!("parameter '{}' does not exist in the plugin", q.name));
                    }
                }
                let base = |m: serde_json::Map<String, serde_json::Value>| plugin::check_params_in(&spec, &p, &m);
                if let Err(e) = base(p.setup.clone()) {
                    problems.push(format!("setup: {e}"));
                }
                for pr in &p.presets {
                    if let Err(e) = base(pr.params.clone()) {
                        problems.push(format!("preset '{}': {e}", pr.name));
                    }
                }
                if !p.source.tested_version.is_empty() && p.source.tested_version != version {
                    println!("note: the profile was verified on {}, installed is {version}", p.source.tested_version);
                }
                if problems.is_empty() {
                    println!("plugin check OK: parameter names, setup and {} preset(s) all apply", p.presets.len());
                } else {
                    for e in &problems {
                        println!("  ✗ {e}");
                    }
                    anyhow::bail!("{} problem(s)", problems.len());
                }
            }
        }
    }
    Ok(())
}

fn audio_files(inputs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for p in inputs {
        if p.is_dir() {
            let mut list: Vec<std::path::PathBuf> = std::fs::read_dir(p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).filter(|f| f.is_file()).collect();
            list.sort();
            files.extend(list.into_iter().filter(|f| f.extension().and_then(|e| e.to_str()).is_some_and(|e| ["wav", "flac", "aif", "aiff", "ogg", "mp3"].contains(&e.to_ascii_lowercase().as_str()))));
        } else {
            files.push(p.clone());
        }
    }
    files
}

fn cmd_yinse(command: YinseCommand) -> Result<()> {
    use muser_engine::modal;
    match command {
        YinseCommand::Extract { inputs, output, name } => {
            let name = name.unwrap_or_else(|| output.file_name().map(|n| n.to_string_lossy().trim_end_matches(".json").trim_end_matches(".yinse").to_string()).unwrap_or_default());
            let files = audio_files(&inputs);
            let table = modal::extract(&files, &name, |n| {
                eprintln!("  {}: pitch {:.2}, {} partials, slowest {:.1} s", n.source, n.pitch, n.partials.len(), n.partials.iter().map(|p| p.t60).fold(0.0, f64::max));
            })
            .map_err(|e| anyhow::anyhow!(e))?;
            std::fs::write(&output, serde_json::to_string(&table)?)?;
            let (lo, hi) = table.range();
            let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
            println!(
                "extracted '{}' from {} recordings (pitch {:.0}-{:.0}, string resonance {:.3}) -> {} ({} KB)\nPlay it with \"instrument\": \"yinse:{}\"; check it with `muser yinse compare {} <recordings>`.",
                table.name,
                table.notes.len(),
                lo,
                hi,
                table.resonance,
                output.display(),
                size.div_ceil(1024),
                output.display(),
                output.display()
            );
        }
        YinseCommand::Compare { instrument, inputs, json } => {
            let list = modal::compare(&instrument, &audio_files(&inputs)).map_err(|e| anyhow::anyhow!(e))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            println!("{:<28} {:>5} {:>4} {:>9}  {:<26} {:<26} {:>11}", "recording", "pitch", "vel", "spectral", "decay @0.1/0.5/1/2 s (rec)", "(extracted)", "stereo r/e");
            let f = |d: [f64; 4]| d.iter().map(|v| if v.is_nan() { "  -".into() } else { format!("{v:>4.0}") }).collect::<Vec<_>>().join(" ");
            for c in &list {
                println!("{:<28} {:>5.1} {:>4.2} {:>7.1}dB  {:<26} {:<26} {:>5.2}/{:<5.2}", c.source, c.pitch, c.velocity, c.spectral_db, f(c.decay_ref), f(c.decay_ours), c.stereo_ref, c.stereo_ours);
            }
            let mean = list.iter().map(|c| c.spectral_db).sum::<f64>() / list.len().max(1) as f64;
            println!("mean spectral difference {mean:.2} dB (third-octave bands, 20 ms frames, first 2 s, loudness matched; lower is closer)");
        }
    }
    Ok(())
}

fn cmd_sounds(command: SoundsCommand) -> Result<()> {
    use muser_engine::sounds;
    match command {
        SoundsCommand::List { kind, filter, json } => {
            let list = sounds::search(kind.as_deref(), filter.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            for s in &list {
                let extra = s.card.as_deref().map(|c| format!("  ♪ {c}")).unwrap_or_default();
                println!("{:<44} [{}{}] {}{extra}", s.spec, s.kind, if s.group.is_empty() { String::new() } else { format!(" · {}", s.group) }, s.description.chars().take(60).collect::<String>());
            }
            println!("\n{} in total. `muser sounds describe <sound>` listens to one (a note marker means already listened to).", list.len());
        }
        SoundsCommand::Presets { plugin, filter, json } => {
            use muser_engine::presets;
            let p = presets::plugin_ref(&plugin).map_err(|e| anyhow::anyhow!(e))?;
            let list = presets::search(&p, filter.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            for x in &list {
                let mark = match x.load {
                    presets::Load::Auto => "auto",
                    presets::Load::Captured => "captured",
                    presets::Load::Manual => "manual",
                };
                println!("[{mark}] {:<32} {} · {}{}", x.name, x.library, x.category, if x.tags.is_empty() { String::new() } else { format!(" · {}", x.tags.join(", ")) });
            }
            println!("\n{} in total. `muser sounds state '{}' <sound>` gets the state; for [manual] ones run `muser sounds capture` first.", list.len(), p.spec());
        }
        SoundsCommand::State { plugin, preset } => {
            use muser_engine::presets;
            let p = presets::plugin_ref(&plugin).map_err(|e| anyhow::anyhow!(e))?;
            let x = presets::find(&p, &preset).map_err(|e| anyhow::anyhow!(e))?;
            let state = presets::state_for(&p, &x).map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", state.display());
            let usage = p.usage(&state, &x.name);
            match usage.get("effect") {
                Some(e) => println!("In the project add to a track's (or master_effects') effects: {e}"),
                None => println!("In the project write: \"instrument\": {}, \"instrument_params\": {}", usage["instrument"], usage["instrument_params"]),
            }
        }
        SoundsCommand::Save { plugin, name, state } => {
            use muser_engine::presets;
            let p = presets::plugin_ref(&plugin).map_err(|e| anyhow::anyhow!(e))?;
            muser_engine::plugin::read_state_file(std::path::Path::new(&state)).map_err(|e| anyhow::anyhow!(e))?;
            let out = presets::save_mine(&p, &name, std::path::Path::new(&state)).map_err(|e| anyhow::anyhow!(e))?;
            println!("saved to my sounds: {name} -> {}", out.display());
        }
        SoundsCommand::Identify { plugin, state } => {
            use muser_engine::presets;
            let p = presets::plugin_ref(&plugin).map_err(|e| anyhow::anyhow!(e))?;
            let all: Vec<_> = presets::presets(&p).into_iter().filter(|x| !x.file.ends_with(".muserstate")).collect();
            let ranked = presets::identify(std::path::Path::new(&state), &all);
            match ranked.first() {
                Some((score, x)) if *score >= 0.3 => println!("loaded: {} ({} · {}, match {score:.2})", x.name, x.library, x.category),
                _ => println!("not recognized: no sound file matches this state (all matches are below 0.3)"),
            }
            for (score, x) in ranked.iter().take(5) {
                println!("  {score:.2}  {}", x.name);
            }
        }
        SoundsCommand::Capture { plugin, preset } => {
            #[cfg(target_os = "macos")]
            {
                use muser_engine::presets;
                let p = presets::plugin_ref(&plugin).map_err(|e| anyhow::anyhow!(e))?;
                let x = presets::find(&p, &preset).map_err(|e| anyhow::anyhow!(e))?;
                let out = presets::library_state_path(&p, &x.file);
                if let Some(d) = out.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let title = format!("Load: {} ({}) - close the window after loading · muser", x.name, x.library);
                let (_tx, rx) = std::sync::mpsc::channel();
                match muser_engine::plugin_edit::run(&format!("{}#{}", p.path, p.id), &serde_json::Map::new(), &out, &title, rx, || {}).map_err(|e| anyhow::anyhow!(e))? {
                    muser_engine::plugin_edit::Outcome::Saved(_) => {
                        let mut params = serde_json::Map::new();
                        params.insert("@state".into(), serde_json::json!(out.display().to_string()));
                        let card = muser_engine::sounds::describe(&p.spec(), &params, None, true).map_err(|e| anyhow::anyhow!(e))?;
                        if card.summary.starts_with("Silent:") {
                            // 没载入完就关了窗口：不记进音色库。
                            let _ = std::fs::remove_file(&out);
                            return Err(anyhow::anyhow!("'{}' did not load (the saved state is silent), nothing captured: run capture again and wait for it to finish loading in the plugin window before closing it", x.name));
                        }
                        // 窗口里实际载入的是哪个（可能载入了别的音色）。
                        let all: Vec<_> = presets::presets(&p).into_iter().filter(|y| !y.file.ends_with(".muserstate")).collect();
                        let ranked = presets::identify(&out, &all);
                        let target = ranked.iter().find(|(_, y)| y.file == x.file).map_or(0.0, |r| r.0);
                        let chosen = match ranked.first() {
                            Some((score, best)) if *score >= 0.3 && target < 0.3 => {
                                println!("the window loaded '{}', not '{}': saving as '{}'", best.name, x.name, best.name);
                                best.clone()
                            }
                            _ => x.clone(),
                        };
                        let fin = presets::library_state_path(&p, &chosen.file);
                        if fin != out {
                            std::fs::rename(&out, &fin)?;
                        }
                        presets::record_capture(&p, &chosen.file, &fin).map_err(|e| anyhow::anyhow!(e))?;
                        println!("captured {} -> {}", chosen.name, fin.display());
                        println!("sound: {}", card.summary);
                    }
                    muser_engine::plugin_edit::Outcome::Unchanged => println!("No changes in the window; nothing captured."),
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (plugin, preset);
                return Err(anyhow::anyhow!("capture needs to open the plugin window, which is currently supported on macOS only"));
            }
        }
        SoundsCommand::Describe { sound, params, keys: true, from, to, tempo, hold, refresh, json, .. } => {
            let params = parse_map(params.as_deref())?;
            let map = muser_engine::plugin::with_stdout_to_stderr(|| sounds::describe_keys(&sound, &params, from, to, tempo, hold, refresh)).map_err(|e| anyhow::anyhow!(e))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&map)?);
                return Ok(());
            }
            println!("{}\n  {} ({} BPM, each key held {} bars)", map.spec, map.summary, map.tempo, map.hold_bars);
            for k in map.keys.iter().filter(|k| k.audible) {
                let lp = k.loop_bars.map_or("no loop".to_string(), |b| format!("{b} bars per cycle"));
                println!("  {:<4} {:>3}  {lp:<10} onsets {:>4}/bar  level {:>6.1} dB  brightness {:>5.0} Hz", k.name, k.pitch, k.onsets_per_bar, k.level_db, k.centroid_hz);
            }
        }
        SoundsCommand::Describe { sound, params, drums, refresh, json, .. } => {
            let params = parse_map(params.as_deref())?;
            let card = muser_engine::plugin::with_stdout_to_stderr(|| sounds::describe(&sound, &params, drums.then_some(true), refresh)).map_err(|e| anyhow::anyhow!(e))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&card)?);
                return Ok(());
            }
            println!("{}\n  {}", card.spec, card.summary);
            for t in &card.traits {
                println!("  - {t}");
            }
        }
    }
    Ok(())
}

fn parse_map(s: Option<&str>) -> Result<serde_json::Map<String, serde_json::Value>> {
    match s {
        Some(s) => serde_json::from_str(s).context("--params must be a JSON object"),
        None => Ok(serde_json::Map::new()),
    }
}

fn cmd_plugin(command: PluginCommand) -> Result<()> {
    use muser_engine::plugin;
    let fail = |e: String| anyhow::anyhow!(e);
    match command {
        PluginCommand::Scan { kind, refresh, json } => {
            let (mut found, failures) = plugin::scan(refresh).map_err(fail)?;
            if let Some(k) = &kind {
                found.retain(|p| &p.kind == k);
            }
            if json {
                let failures: Vec<_> = failures.iter().map(|(p, e)| serde_json::json!({ "path": p, "error": e })).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "plugins": found, "failures": failures }))?);
                return Ok(());
            }
            for p in &found {
                println!("[{:<10}] {:<5} {} — {}", p.kind, p.format, p.name, p.vendor);
                println!("             {}#{}", p.path, p.id);
            }
            for (p, e) in &failures {
                println!("[failed] {p}: {e}");
            }
            println!("\n{} plugin(s){}. For an instrument write \"instrument\": \"plugin:<path>#<id>\"; for an effect write {{\"type\": \"plugin\", \"path\": ..., \"id\": ...}}.",
                found.len(), if failures.is_empty() { String::new() } else { format!(", {} failed to read", failures.len()) });
        }
        PluginCommand::Info { plugin: spec, json } => {
            let info = plugin::with_stdout_to_stderr(|| plugin::info(&spec)).map_err(fail)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
                return Ok(());
            }
            println!("{} — {} {}", info.name, info.vendor, info.version);
            println!("  kind:     {} ({})  categories: {}", info.kind, info.format, info.categories.join(", "));
            println!("  id:       {}", info.id);
            println!("  takes notes: {}", if info.takes_notes { "yes" } else { "no" });
            println!("  audio inputs: {}", if info.audio_inputs.is_empty() { "none".into() } else { info.audio_inputs.join("; ") });
            println!("  audio outputs: {}", info.audio_outputs.join("; "));
            if info.latency_samples > 0 {
                println!("  latency:  {} frames", info.latency_samples);
            }
            println!("  params:   {} ({} visible)", info.params_total, info.params_visible);
            let mut groups = info.param_groups.clone();
            groups.sort_by_key(|g| std::cmp::Reverse(g.1));
            let shown: Vec<String> = groups.iter().take(20).map(|(g, n)| format!("{g}({n})")).collect();
            println!("  param groups: {}", shown.join(" "));
            match &info.profile {
                Some(pr) => {
                    println!("  profile:  {:?} {}, {} alias(es), presets: {}", pr.origin, pr.path.as_deref().unwrap_or(""), pr.aliases,
                        if pr.presets.is_empty() { "none".into() } else { pr.presets.join(", ") });
                    if !pr.setup.is_empty() {
                        println!("            setup applied automatically: {} (write \"@setup\": false to turn off)", serde_json::Value::Object(pr.setup.clone()));
                    }
                    println!("            `muser plugin profile show '{spec}' --markdown` shows the usage notes");
                }
                None => println!("  profile:  none (`muser plugin profile draft '{spec}' --save` generates a draft)"),
            }
            if !info.siblings.is_empty() {
                println!("  other plugins in the same file:");
                for (id, name) in &info.siblings {
                    println!("    {name}  →  {}#{id}", info.path);
                }
            }
            println!("\n`muser plugin params '{spec}' --filter <keyword>` shows parameters.");
        }
        PluginCommand::SaveState { plugin: spec, params, output } => {
            let base = parse_map(params.as_deref())?;
            let h = plugin::save_state(&spec, &base, std::path::Path::new(&output)).map_err(fail)?;
            println!("saved the state of {} {} -> {output}", h.plugin_name, h.version);
            println!("In the project write \"instrument_params\": {{ \"@state\": \"{output}\" }} (for an effect, put it in params).");
        }
        PluginCommand::Edit { plugin: spec, params, output, title, serve } => {
            #[cfg(target_os = "macos")]
            {
                use muser_engine::plugin_edit::{run, Cmd, Outcome};
                let base = parse_map(params.as_deref())?;
                let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
                if serve {
                    // 由 muser view 驱动：stdin 每行一条 JSON 命令，stdout 每行一条事件。
                    std::thread::spawn(move || {
                        for line in std::io::stdin().lines().map_while(Result::ok) {
                            match serde_json::from_str::<Cmd>(&line) {
                                Ok(c) => {
                                    let _ = tx.send(c);
                                }
                                Err(e) => eprintln!("[muser] unrecognized command {line}: {e}"),
                            }
                        }
                    });
                } else {
                    std::mem::forget(tx);
                }
                let title = title.unwrap_or_else(|| format!("{} · muser", plugin::split_spec(&spec).0.rsplit('/').next().unwrap_or(&spec)));
                let saved = || {
                    if serve {
                        use std::io::Write;
                        println!("{}", serde_json::json!({ "event": "saved" }));
                        let _ = std::io::stdout().flush();
                    }
                };
                match run(&spec, &base, std::path::Path::new(&output), &title, rx, saved).map_err(fail)? {
                    Outcome::Saved(h) if !serve => {
                        println!("saved the state of {} {} -> {output}", h.plugin_name, h.version);
                        println!("In the project write \"instrument_params\": {{ \"@state\": \"{output}\" }}.");
                    }
                    Outcome::Unchanged if !serve => println!("No changes; not saved."),
                    _ => {}
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (spec, params, output, title, serve);
                return Err(anyhow::anyhow!("the plugin UI is currently supported on macOS only"));
            }
        }
        PluginCommand::Profile { command } => cmd_profile(command)?,
        PluginCommand::Values { plugin: spec, param, samples, params, json } => {
            let base = parse_map(params.as_deref())?;
            let (p, values) = plugin::with_stdout_to_stderr(|| plugin::values(&spec, &param, samples, &base)).map_err(fail)?;
            if json {
                let list: Vec<_> = values.iter().map(|(v, t)| serde_json::json!({ "value": v, "text": t })).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "param": p, "values": list }))?);
                return Ok(());
            }
            println!("{} (id {}, current \"{}\") accepts these values:", p.name, p.id, p.current_text);
            let many = values.len() > 40;
            for (i, (v, t)) in values.iter().enumerate() {
                if many && i >= 20 && i + 10 < values.len() {
                    if i == 20 {
                        println!("  ... ({} in total, a continuous parameter; the middle is omitted)", values.len());
                    }
                    continue;
                }
                println!("  {:<28} = {v:.6}", format!("\"{t}\""));
            }
            println!("\nWrite the text in params / instrument_params, e.g. {{ \"{}\": \"{}\" }}.", p.name, values.last().map_or("", |(_, t)| t.as_str()));
        }
        PluginCommand::Params { plugin: spec, filter, all, params, limit, json } => {
            let base = parse_map(params.as_deref())?;
            let list = plugin::with_stdout_to_stderr(|| plugin::params(&spec, filter.as_deref(), all, &base)).map_err(fail)?;
            let total = list.len();
            let shown: Vec<_> = if limit == 0 { list } else { list.into_iter().take(limit).collect() };
            if json {
                println!("{}", serde_json::to_string_pretty(&shown)?);
                return Ok(());
            }
            for p in &shown {
                let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
                let steps = p.steps.map_or(String::new(), |s| format!(" {s} steps"));
                let flags = format!("{}{}", if p.automatable { "" } else { " not automatable" }, if p.read_only { " read-only" } else { "" });
                let alias = p.alias.as_ref().map_or(String::new(), |a| format!("  alias {a}"));
                println!("  {:<40} {} – {}{unit}{steps}  current {} \"{}\"  id {}{flags}{alias}",
                    p.name, p.min, p.max, p.current, p.current_text, p.id);
            }
            if total > shown.len() {
                println!("  ... {} more; narrow with --filter or show all with --limit 0", total - shown.len());
            }
            println!("\n{total} in total. Use the parameter name as the key; the value can be the text the plugin displays (e.g. \"{}\") or a number.",
                shown.first().map_or("…".into(), |p| p.current_text.clone()));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_audition(
    instrument: Option<String>,
    effect: Option<String>,
    source: String,
    params: Option<String>,
    param: Option<String>,
    values: Vec<String>,
    pitch: u8,
    velocity: f32,
    hold: f64,
    tail: f64,
    out: Option<String>,
    record: bool,
    json: bool,
) -> Result<()> {
    use muser_engine::audition::{audition, parse_value, AuditionSettings, Subject};
    let subject = match (instrument, effect) {
        (Some(name), None) => {
            let params = match params {
                Some(p) => serde_json::from_str(&p).context("--params must be a JSON object")?,
                None => serde_json::Map::new(),
            };
            Subject::Instrument { name, params }
        }
        (None, Some(e)) => Subject::Effect { effect: serde_json::from_str(&e).context("--effect must be effect JSON")?, source },
        _ => anyhow::bail!("give either --instrument or --effect"),
    };
    let values: Vec<serde_json::Value> = values.iter().filter(|v| !v.trim().is_empty()).map(|v| parse_value(v)).collect();
    let settings = AuditionSettings { pitch, velocity, hold_seconds: hold, tail_seconds: tail, out_dir: out.map(Into::into) };
    let takes = audition(&subject, param.as_deref(), &values, &settings)?;
    if record {
        let spec = match &subject {
            Subject::Instrument { name, .. } => name.trim().strip_prefix("plugin:").map(String::from),
            Subject::Effect { effect: muser_core::Effect::Plugin { path, id, .. }, .. } => {
                Some(id.as_ref().map_or(path.clone(), |i| format!("{path}#{i}")))
            }
            _ => None,
        };
        match (spec, &param) {
            (Some(spec), Some(param)) => {
                let path = muser_engine::plugin::record_observations(&spec, param, &takes).map_err(|e| anyhow::anyhow!(e))?;
                eprintln!("recorded into the profile: {}", path.display());
            }
            _ => anyhow::bail!("--record needs an external plugin (a plugin:... instrument or a plugin effect) and --param"),
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&takes)?);
        return Ok(());
    }
    println!("{:<14} {:>7} {:>7} {:>7} {:>9} {:>6} {:>6} {:>6} {:>8} {:>9}",
        param.as_deref().unwrap_or("value"), "peak dB", "RMS dB", "LUFS", "centroid Hz", "low%", "mid%", "high%", "attack ms", "tail s");
    for t in &takes {
        let a = &t.analysis;
        let v = match &t.value { serde_json::Value::Null => "unchanged".to_string(), serde_json::Value::String(s) => s.clone(), v => v.to_string() };
        println!("{:<14} {:>7.1} {:>7.1} {:>7.1} {:>9.0} {:>6.1} {:>6.1} {:>6.1} {:>8.0} {:>8.2}{}",
            v, a.peak_db, a.rms_db, a.lufs, a.centroid_hz, a.low_pct, a.mid_pct, a.high_pct, a.attack_ms, a.release_s,
            if a.release_truncated { "+" } else { " " });
        if let Some(w) = &t.wav {
            println!("{:<14} → {w}", "");
        }
    }
    println!("\nA higher centroid is brighter; a + on the tail means it was cut off by the render length (lengthen it with --tail).");
    Ok(())
}

fn cmd_mix_report(path: &str, sample_rate: u32, tail: f64, json: bool) -> Result<()> {
    let project = load_project(path)?;
    let settings = RenderSettings { sample_rate, tail_seconds: tail, ..RenderSettings::default() };
    let r = muser_engine::mix_report(&project, settings, &mut muser_engine::RenderCache::new())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    let f = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{v:.1}"));
    println!("{:<12} {:<14} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7}",
        "node", "name", "gain_db", "in LUFS", "in peak", "out LUFS", "short max", "out peak", "to master", "rel LU");
    for n in r.tracks.iter().chain(&r.buses).chain(std::iter::once(&r.master)) {
        let name = if n.muted { format!("{} (muted)", n.name) } else { n.name.clone() };
        println!(
            "{:<12} {:<14} {:>7.1} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7}",
            n.node, name, n.gain_db, f(n.input.lufs), f(n.input.peak_db), f(n.output.lufs), f(n.output.max_short_lufs),
            f(n.output.peak_db), f(n.at_master_lufs), f(n.rel_lu)
        );
    }
    println!("in = before the effect chain (for a track, the dry instrument; for a bus / master, the sum of the signals sent in); out = after the fader.");
    println!("to master = out + the gain_db of the buses on the direct output path (an estimate, ignoring bus effects and sends); rel LU = to master minus the loudest track.");
    println!("The estimate can be off by 2-5 LU through buses with compression / EQ: compare tracks' out LUFS within one bus, and buses' out LUFS between groups.");
    for w in &r.warnings {
        println!("WARNING {w}");
    }
    Ok(())
}

fn cmd_routing(path: &str) -> Result<()> {
    use muser_core::routing::{self, Node};
    let p = load_project(path)?;
    let plan = routing::plan(&p).map_err(|e| anyhow::anyhow!(e))?;
    let name = |n: Node| match n {
        Node::Track(i) => {
            let t = &p.tracks[i];
            format!("{}{}", if t.name.is_empty() { format!("tracks[{i}]") } else { t.name.clone() }, if t.muted { " (muted)" } else { "" })
        }
        Node::Bus(i) => format!("[{}]{}", p.buses[i].name, if p.buses[i].muted { " (muted)" } else { "" }),
    };
    let fx = |list: &[muser_core::EffectSlot]| {
        if list.is_empty() {
            String::new()
        } else {
            format!(" → {}", list.iter().map(|e| match e.effect.sidechain() {
                Some(k) => format!("{}⟵{k}", e.type_name()),
                None => e.type_name().to_string(),
            }).collect::<Vec<_>>().join(" → "))
        }
    };
    println!("Processing order and destinations:");
    for n in &plan.order {
        let (src, effects) = match *n {
            Node::Track(i) => (format!("{} ({})", name(*n), p.tracks[i].instrument), &p.tracks[i].effects),
            Node::Bus(i) => (name(*n), &p.buses[i].effects),
        };
        let outs: Vec<String> = plan
            .edges
            .iter()
            .filter(|e| e.from == *n)
            .map(|e| {
                let to = e.to.map_or("master".to_string(), |b| format!("[{}]", p.buses[b].name));
                match &e.send {
                    Some(_) => format!("{to} send {:+.1} dB", e.level_db),
                    None => to,
                }
            })
            .collect();
        println!("  {src}{} ⇒ {}", fx(effects), outs.join(", "));
    }
    println!("  master{} -> master gain {:+.1} dB", fx(&p.master_effects), p.master_gain_db);
    if !plan.taps.is_empty() {
        println!("Plugin direct outputs (the bus of @outputs; they skip the track's effects and fader):");
        for t in &plan.taps {
            let to = t.to.map_or("master".to_string(), |b| format!("[{}]", p.buses[b].name));
            println!("  {} output {} => {to}", name(Node::Track(t.track)), t.index + 1);
        }
    }
    if !plan.sidechains.is_empty() {
        println!("Sidechains:");
        for s in &plan.sidechains {
            println!("  {} triggers effects[{}] of {}", name(s.key), s.effect, name(s.node));
        }
    }
    Ok(())
}

fn cmd_migrate(projects: &[String], from: &str, write: bool) -> Result<()> {
    use muser_engine::migrate;
    for path in projects {
        let text = fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?;
        let mut v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("{path} is not valid JSON"))?;
        let changes = migrate::migrate(&mut v, from).map_err(|e| anyhow::anyhow!(e))?;
        if changes.is_empty() {
            println!("{path}: no change needed from {from} to the current version");
            continue;
        }
        println!("{path}:");
        for c in &changes {
            println!("  {}: {}", c.track, c.detail);
        }
        if write {
            fs::write(path, serde_json::to_string_pretty(&v)? + "\n")?;
            println!("  written back");
        } else {
            println!("  (listing only; add --write to write back)");
        }
    }
    Ok(())
}

fn cmd_new(output: Option<&str>) -> Result<()> {
    // 一个带鼓+贝斯线的示例工程，展示完整格式。
    let mut p = Project::new("example", 90.0);

    let mut drums = Track::new("drums", "drums.lofi");
    let mut dclip = Clip::new(0.0, 4.0);
    // 4/4 lo-fi：kick 在 1、3；snare 在 2、4；闭镲走八分
    dclip.notes.push(Note::new(36, 0.0, 0.25));
    dclip.notes.push(Note::new(38, 1.0, 0.25));
    dclip.notes.push(Note::new(36, 2.0, 0.25));
    dclip.notes.push(Note::new(38, 3.0, 0.25));
    for i in 0..8 {
        dclip.notes.push(Note::new(42, i as f64 * 0.5, 0.1));
    }
    drums.clips.push(dclip);
    p.tracks.push(drums);

    let mut bass = Track::new("bass", "synth.fm");
    bass.gain_db = -3.0;
    let mut bclip = Clip::new(0.0, 4.0);
    // 简单根音走向：C2 - C2 - G1 - A1
    bclip.notes.push(Note::new(36, 0.0, 1.0));
    bclip.notes.push(Note::new(36, 1.0, 1.0));
    bclip.notes.push(Note::new(31, 2.0, 1.0));
    bclip.notes.push(Note::new(33, 3.0, 1.0));
    bass.clips.push(bclip);
    p.tracks.push(bass);

    let json = p.to_json()?;
    match output {
        Some(path) => {
            fs::write(path, &json).with_context(|| format!("write failed: {path}"))?;
            println!("generated an example project: {path}");
        }
        None => println!("{json}"),
    }
    Ok(())
}

#[derive(Subcommand)]
enum LiveCommand {
    /// Start a live session (stays in the foreground until muser live stop). The template is an ordinary project: tracks, effects, buses, tempo, optionally with an opening phrase.
    Start {
        /// Template project.
        template: String,
        /// How long to autoplay (e.g. 3m, 90s; 0 = unlimited). When time is up, wait tells the agent to submit one last section and stop; the music keeps looping.
        #[arg(long, default_value = "3m")]
        autoplay: String,
        /// Record the whole session into this project file (updated continuously; after the session it can be rendered offline and edited further).
        #[arg(long)]
        record: Option<String>,
        #[arg(long, default_value_t = 8747)]
        port: u16,
        /// Open the live page in the system browser (current position, queued sections, per-track levels).
        #[arg(long)]
        open: bool,
        /// Title of the whole session (at the top of the page); write what the person asked for, e.g. "From electronic to country". A later section can change it with "title".
        #[arg(long)]
        title: Option<String>,
        /// Start in the background: fully detached from the current terminal / calling program, returning once it starts playing (agents use this). Output goes to muser-live.log in the temp directory.
        #[arg(long)]
        detach: bool,
        /// Seconds of tail at the end (not used between loops and sections; only affects the very end of the session).
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
    },
    /// Submit a section (a JSON file, - for stdin): { "bars": 8, "time_signature": {...}, "at": "after"|"now", "tracks": [...] }. Returns a receipt immediately.
    Send {
        file: String,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Current bar, how far ahead is queued, whether it is looping, each track, and what the last section wrote.
    Status {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Wait until the queued music is shorter than --runway seconds (or the autoplay time is up), at most 30 seconds.
    Wait {
        #[arg(long, default_value_t = 8.0)]
        runway: f64,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Adjust volume during the session: a track name / bus name / master, and the fader target (dB). Starts after about 0.4 s and ramps into place within 0.3 s; no new section needed, and the form is not disturbed.
    Mix {
        track: String,
        #[arg(allow_hyphen_values = true)]
        gain_db: f64,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Reset the autoplay duration (from now, e.g. 5m; 0 = unlimited).
    Autoplay {
        duration: String,
        #[arg(long)]
        port: Option<u16>,
    },
    /// End the session: write the recorded project and exit the resident process.
    Stop {
        #[arg(long)]
        port: Option<u16>,
    },
}

fn cmd_live(command: LiveCommand) -> Result<()> {
    let print = |v: serde_json::Value| -> Result<()> {
        println!("{}", serde_json::to_string_pretty(&v)?);
        Ok(())
    };
    match command {
        LiveCommand::Start { template, autoplay, record, port, open, title, detach: true, tail } => {
            let mut args = vec![template, "--autoplay".into(), autoplay, "--port".into(), port.to_string(), "--tail".into(), tail.to_string()];
            if let Some(t) = title {
                args.extend(["--title".into(), t]);
            }
            if let Some(r) = record {
                args.extend(["--record".into(), r]);
            }
            if open {
                args.push("--open".into());
            }
            let exe = std::env::current_exe()?;
            let v = muser_engine::live_client::spawn_detached(&exe, &args).map_err(|e| anyhow::anyhow!(e))?;
            println!("muser live started in the background (bar {}); page http://127.0.0.1:{port}/; log {}", v["bar"], std::env::temp_dir().join("muser-live.log").display());
            Ok(())
        }
        LiveCommand::Start { template, autoplay, record, port, open, title, detach: false, tail } => {
            let autoplay = live::parse_duration(&autoplay)?;
            let settings = RenderSettings { tail_seconds: tail, ..RenderSettings::default() };
            muser_engine::plugin::run_with_main_loop(move || match live::serve(&template, settings, port, autoplay, record, open, title) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("Error: {e:?}");
                    1
                }
            })
        }
        LiveCommand::Send { file, port } => {
            let text = if file == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                std::fs::read_to_string(&file).with_context(|| format!("cannot read {file}"))?
            };
            print(live::request(port, "POST", "/api/change", Some(&text))?)
        }
        LiveCommand::Status { port } => print(live::request(port, "GET", "/api/status", None)?),
        LiveCommand::Wait { runway, port } => print(live::request(port, "GET", &format!("/api/wait?runway={runway}"), None)?),
        LiveCommand::Mix { track, gain_db, port } => print(live::request(port, "POST", "/api/mix", Some(&serde_json::json!({ "track": track, "gain_db": gain_db }).to_string()))?),
        LiveCommand::Autoplay { duration, port } => print(live::request(port, "POST", "/api/autoplay", Some(&serde_json::json!({ "duration": duration }).to_string()))?),
        LiveCommand::Stop { port } => print(live::request(port, "POST", "/api/stop", None)?),
    }
}
