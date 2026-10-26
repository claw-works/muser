# muser

English | [简体中文](README.zh-CN.md)

**A headless music-arrangement engine built for AI.**

muser has no GUI. It wraps a deterministic audio engine in **a structured data format + a CLI + an MCP server**,
so an AI agent composes by expressing musical intent: the AI writes or edits a project JSON, and the engine renders it to audio.

Design principle: **the engine owns the deterministic "audio truth", the AI owns "intent translation", and the interface layer is the contract between them.**

Audio is powered by [manguebyte](https://github.com/alessonqueirozdev-hub/manguebyte) (a Rust audio framework),
vendored as a git submodule.

---

## Features

- Multi-track arrangement: drums + synths + bass, multiple clips and notes
- 115 built-in instruments: 10 drum kits, 68 synth presets, plus guitars, basses, organs, mallets (marimba etc.), strings, winds and choirs — all synthesized by manguebyte in real time, no system MIDI synth needed; each one is tweakable via `instrument_params`
- Built-in SFZ sampler: `sfz:/path/to.sfz` (or a path relative to a sound-library folder on `MUSER_SFZ_PATH`) loads SFZ libraries directly
- Effect chains (reverb / delay / compressor / chorus / eq / limiter / phaser / flanger) on tracks, buses or the master bus
- Mix routing: bus groups, sends to shared reverbs/delays, nested buses, compressor sidechains (kick pumping the bass), automatic latency compensation
- Automation: instrument parameters, effect parameters, gain, pan and send levels over time (point curves / LFO / seeded random walk)
- Third-party CLAP/VST3/LV2 plugins as effects (VST) or instruments (VSTi): scan installed plugins, set parameters by name and display text, automate them (`host` feature)
- Audition: `muser audition` renders a parameter at several values and reports loudness, brightness, attack and tail, so the AI can learn unfamiliar parameters
- Plugin profiles: store plugin know-how (aliases, presets, audition findings) as shareable data files; plugin sounds can be saved as state files
- Built-in profiles and English skills for open-source plugins: Surge XT / Surge XT Effects, Vital (wavetable synth, with wavetables and mod matrix), Airwindows Consolidated (~470 effects, picked by name), Plasma (distortion) — install the plugin and use it by name, alias or preset
- Streaming render engine: walks the whole routing graph block by block; offline WAV rendering and render-while-playing share the same code and are sample-identical (instruments of different tracks play in parallel)
- `muser view` player page: playback starts as soon as the first bit is rendered; after changing volume/mute/solo the new version swaps in seamlessly at the current position; changes can be saved back to the project.
  Add `--device` to play through this machine's sound card, with volume/mute/solo taking effect instantly (when working remotely over an SSH port forward, use the default in-browser playback)
- Two AI interfaces: **CLI** (shell calls) and **MCP server** (direct tool calls from the agent)

## Quick start

### Download and run (no Rust needed)

1. Download the archive for your system from [Releases](https://github.com/claw-works/muser/releases):

   | System | File |
   |---|---|
   | macOS, Apple Silicon (M1–M4) | `muser-<version>-aarch64-apple-darwin.tar.gz` |
   | macOS, Intel | `muser-<version>-x86_64-apple-darwin.tar.gz` |
   | Windows | `muser-<version>-x86_64-pc-windows-msvc.zip` |
   | Linux desktop | `muser-<version>-x86_64-unknown-linux-gnu.tar.gz` (needs `libasound2`, present on desktop systems) |
   | Linux server / Docker | `muser-<version>-x86_64-unknown-linux-gnu-headless.tar.gz` (no sound-card playback) |

2. Unpack it and open a terminal in the unpacked folder. It contains `muser` (CLI), `muser-mcp` (MCP server),
   the docs for AI agents (`SKILLS.md`, `skills/`, `TOOLS.md`) and sample projects (`samples/`).

   macOS / Linux:

   ```bash
   tar xzf muser-<version>-aarch64-apple-darwin.tar.gz
   cd muser-<version>-aarch64-apple-darwin
   # macOS only: the binaries are not signed, so clear the download quarantine first
   xattr -dr com.apple.quarantine .
   ```

   Windows (PowerShell):

   ```powershell
   Expand-Archive muser-<version>-x86_64-pc-windows-msvc.zip -DestinationPath .
   cd muser-<version>-x86_64-pc-windows-msvc
   ```

   If Windows SmartScreen warns about an unknown publisher, choose "More info" → "Run anyway".

3. Listen to a sample song. The player page opens in your browser and the sound comes out of this computer's sound card:

   ```bash
   ./muser view samples/songs/dnb-liquid-skyline.muser.json --open --device
   ```

   On Windows use `.\muser.exe` instead of `./muser`. Press Ctrl+C in the terminal to stop.
   Leave out `--device` to play inside the browser instead (for example when muser runs on a remote machine).

4. Render it to a WAV file:

   ```bash
   ./muser render samples/songs/dnb-liquid-skyline.muser.json -o dnb.wav
   ```

5. Compose with an AI agent: open the unpacked folder in your agent (Claude Code, Codex, …) and ask it to read `SKILLS.md`,
   then describe the music you want. The agent writes a project JSON and renders it with `./muser`;
   play the result with `./muser view <project> --open --device`.
   Agents that speak MCP can use `muser-mcp` instead (see [below](#run-as-an-mcp-server-for-agents)).

To run `muser` from any folder, add the unpacked folder to your `PATH`.

### Build from source (for developers)

#### 1. Clone (with submodules)

muser depends on the vendored manguebyte submodule, so clone recursively:

```bash
git clone --recurse-submodules git@github.com:claw-works/muser.git
cd muser
```

If you already cloned without it, fetch the submodule:

```bash
git submodule update --init --recursive
```

#### 2. Build

Requires Rust 1.85+ (edition 2024).

```bash
cargo build --release
```

On Linux, sound-card playback (`muser view --device`, enabled by default) needs the ALSA development package: `sudo apt install libasound2-dev`.
If you don't need it, skip it with `cargo build --release --no-default-features`.

The binaries are `target/release/muser` (CLI) and `target/release/muser-mcp` (MCP server).
`muser` in the commands below means that binary — add `target/release` to your `PATH`, or call it by full path.

### Using the CLI

```bash
# Generate an example project JSON
muser new -o song.muser.json

# Validate
muser validate song.muser.json

# Render to WAV
muser render song.muser.json -o song.wav

# List instruments and effects (add a name for details, --json for machine-readable output)
muser instruments
muser instruments guitar.nylon
muser effects
muser effects reverb

# Show the signal flow (buses, sends, sidechains)
muser routing song.muser.json

# For humans: local player page (timeline, volume/mute/solo re-rendered by the engine, save back to the project;
# the page refreshes when the AI edits the project)
muser view song.muser.json --open      # http://127.0.0.1:8737/
# Listening on this machine: sound comes straight out of its sound card, faders/mute/solo apply instantly, the page is a remote control
muser view song.muser.json --open --device
```

> During development you can also run `cargo run -p muser-cli -- <subcommand>` without installing.

### Run as an MCP server (for agents)

```bash
muser-mcp          # stdio MCP server exposing 34 tools
```

Register this executable as a stdio server in an MCP client (such as Claude Desktop),
and the agent can arrange music with tools like `create_project / add_track / add_clip / add_notes / render`.

## External sounds and plugins (optional)

The 115 built-in instruments need no downloads (most samples, such as `samples/songs/dnb-liquid-skyline`, use only built-in sounds).
The free / open-source sound libraries and plugins below are optional: `skills/` and some `samples/` use them; once installed they work by name, alias or preset.

### SFZ sample libraries

| Library | Contents | License | Download | Used by sample |
|---|---|---|---|---|
| Salamander Grand Piano (freepats) | Grand piano, multiple velocity layers, string resonance | CC-BY 3.0 | [freepats.zenvoid.org](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html) (SFZ+FLAC version) | `songs/bach-prelude-c` |
| PianoFB (freepats) | Upright piano | GPL-3.0 with exception | [freepats.zenvoid.org](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html) | `instruments/sfz-piano` |
| Sonatina Symphonic Orchestra 4.0 | Orchestra: Legato / Marcato / Sustain / Staccato / Pizzicato articulations per section | CC Sampling Plus 1.0 | [github.com/peastman/sso](https://github.com/peastman/sso) (v4.0 zip under Releases) | `songs/ode-to-joy-orchestra` |
| Black And Blue Basses (Karoryfer) | Electric bass, multiple articulations (keyswitches) and controller switches | CC0 | [github.com/sfzinstruments/karoryfer.black-and-blue-basses](https://github.com/sfzinstruments/karoryfer.black-and-blue-basses) | `instruments/sfz-bass-black-and-blue` |
| VSCO 2 Community Edition | Orchestra (smaller, slower attacks) | CC0 | [versilian-studios.com](https://versilian-studios.com/vsco-community/) | — |

More free SFZ: [sfzinstruments.github.io](https://sfzinstruments.github.io/). In a project, write `"instrument": "sfz:<path to .sfz>"`.
The samples use paths relative to the sound library (e.g. `sfz:SalamanderGrandPiano-SFZ+FLAC-V3+20200602/...`):
unpack the libraries into one folder and set `export MUSER_SFZ_PATH=~/Music/SFZ` (separate several folders with `:`, or `;` on Windows).
muser searches these folders and their subfolders up to two levels deep; absolute paths work too.
SFZ controllers and articulations are set with `cc` / `keyswitch` in `instrument_params` (see [skills/instruments/sfz.md](skills/instruments/sfz.md)).

### Plugins (need the `host` feature; enabled in prebuilt binaries)

| Plugin | Type | License | Download | Built-in muser profile / skill |
|---|---|---|---|---|
| Surge XT (incl. Surge XT Effects) | Hybrid synth + effects collection | GPL-3.0 | [surge-synthesizer.github.io](https://surge-synthesizer.github.io/) | [surge-xt](skills/plugins/surge-xt.md), [surge-xt-effects](skills/plugins/surge-xt-effects.md) |
| Vital | Wavetable synth | Free (source GPL-3.0) | [vital.audio](https://vital.audio/) (free tier requires sign-up) | [vital](skills/plugins/vital.md) |
| Airwindows Consolidated | ~470 Airwindows effects in one (use the CLAP version) | MIT | [airwindows.com/consolidated](https://www.airwindows.com/consolidated/) or [github.com/baconpaul/airwin2rack](https://github.com/baconpaul/airwin2rack) | [airwindows-consolidated](skills/plugins/airwindows-consolidated.md) |
| Plasma (Dimethoxy) | Two-stage distortion | AGPL-3.0 | [github.com/Dimethoxy/Plasma](https://github.com/Dimethoxy/Plasma) | [plasma](skills/plugins/plasma.md) |

After installing, run `muser plugin scan` to check that muser finds them (system and user folders for CLAP / VST3 / LV2 are all scanned).
Plugin samples live in `samples/plugins/`; usage is in [skills/plugins.md](skills/plugins.md).

## Project file format (the contract with the AI)

Time is measured in **beats**: `1.0` = a quarter note, independent of tempo. Minimal example:

```json
{
  "tempo": 90.0,
  "tracks": [
    {
      "instrument": "drums.lofi",
      "clips": [
        { "start": 0.0, "length": 4.0, "notes": [
          { "pitch": 36, "start": 0.0, "length": 0.25 },
          { "pitch": 38, "start": 1.0, "length": 0.25 }
        ]}
      ]
    }
  ]
}
```

Drum pitches use GM drum keys (36 = kick, 38 = snare, 42 = closed hi-hat); melodic instruments use MIDI pitch (60 = middle C).

## Optional features

```bash
# Enable the plugin host (load third-party CLAP/VST3/LV2 effects)
cargo build -p muser-cli --features host
```

## Project layout

```
muser/
├─ crates/
│  ├─ muser-core/     Domain model: Project/Track/Clip/Note/Effect (all serde)
│  ├─ muser-engine/   Engine adapter: builds manguebyte instruments, streaming render (WAV / player page / sound card)
│  ├─ muser-cli/      CLI executable `muser`
│  └─ muser-mcp/      MCP server (stdio)
├─ vendor/manguebyte/ Upstream audio engine (git submodule)
├─ samples/           Sample projects (grouped: songs/instruments/params/effects/automation/routing/plugins)
├─ ARCHITECTURE.md    Architecture, layers, project JSON schema, roadmap
├─ SKILLS.md          Arrangement handbook for the AI (main entry point)
├─ skills/            Sub-skills: each instrument family, each effect, automation, routing, built-in plugin profiles
├─ profiles/          Plugin profiles shipped with muser (JSON)
├─ scripts/           gen_skills.py generates skills/; render_samples.sh renders the samples in bulk
└─ TOOLS.md           CLI + MCP tool list
```

## Further reading

The documents below are written in Chinese, except the plugin skills under `skills/plugins/`.

- [ARCHITECTURE.md](ARCHITECTURE.md) — layered architecture, data flow, project file format, vendoring strategy, roadmap
- [SKILLS.md](SKILLS.md) — how the AI expresses musical intent (entry point: workflow, cheat sheet, instrument and effect index)
- [skills/](skills/) — parameters, scene presets and common mistakes for each instrument family and effect; automation, routing, sustain pedal
- [TOOLS.md](TOOLS.md) — CLI commands and MCP tools
- [samples/](samples/) — grouped sample projects (songs, instruments, tuning, effects, automation, routing, plugins)

## License

The code in this repository is dual-licensed under MIT / Apache-2.0. The vendored manguebyte is also MIT / Apache-2.0.
