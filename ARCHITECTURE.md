# muser architecture

English | [简体中文](ARCHITECTURE.zh-CN.md)

muser is a **headless arrangement engine for AI**: you arrange with data instead of a mouse. It has no editor window.
It offers a structured data format + a CLI + an MCP server, so an AI agent can drive a deterministic audio engine
by "expressing musical intent" and get audio you can listen to.

Design principle: **the engine owns the deterministic "audio truth", the AI owns the "translation of intent", and the interface layer is the contract between them.**

---

## Three layers

```
┌─────────────────────────────────────────────────────────────┐
│  3. Interface layer (muser-cli + muser-mcp)                   │
│     - CLI subcommands: render / validate / instruments /      │
│       effects / plugin / audition / view / routing / new      │
│     - MCP server (muser-mcp): the same capabilities as tools  │
│     - skill docs: how the AI expresses musical intent         │
│       (SKILLS.md)                                             │
├─────────────────────────────────────────────────────────────┤
│  2. Engine adapter layer (muser-engine)                       │
│     - instrument: semantic instrument name → manguebyte       │
│       instrument                                              │
│     - render: Project → streaming Renderer → WAV / player     │
│       page / sound card                                       │
├─────────────────────────────────────────────────────────────┤
│  1. Domain model layer (muser-core)                           │
│     - Project / Track / Clip / Note / Beats / TimeSignature   │
│     - all serde: the JSON is the format the AI reads and      │
│       writes projects in                                      │
│     - time is in beats, independent of tempo and sample rate  │
├─────────────────────────────────────────────────────────────┤
│  0. Audio engine (manguebyte, from crates.io)                 │
│     - DSP, instruments / drum kits, audio graph, plugin host, │
│       audio file formats                                      │
│     - upstream project, MIT/Apache, a crates.io dependency    │
└─────────────────────────────────────────────────────────────┘
```

The AI only needs to understand **the JSON format of layer 1** and **the commands of layer 3**; layers 0 and 2 are invisible to it.

---

## Crates

| crate | layer | role | key dependencies |
|---|---|---|---|
| `muser-core` | 1 | Domain model + serialization. No audio dependencies; compiles very fast. | serde |
| `muser-engine` | 2 | Compiles the core model into manguebyte instruments; streaming render (offline WAV, play while rendering, local sound card). | muser-core, manguebyte |
| `muser-cli` | 3 | The `muser` CLI executable. | muser-core, muser-engine, clap |
| `muser-mcp` | 3 | MCP server (stdio) exposing the capabilities as agent tools. | muser-core, muser-engine, rmcp |
| `manguebyte` (crates.io) | 0 | The upstream audio engine. | — |

### Why a crates.io dependency (it used to be a git submodule)

- **Upstream publishes releases**: manguebyte 0.2.0 is on crates.io (MIT/Apache); pinning a version gives the same reproducibility as pinning a commit, and nobody has to clone with `--recursive`.
- **Devices only when needed**: the dependency uses `default-features = false`, only what rendering needs; local sound-card playback (`muser view --device`)
  turns on manguebyte's `devices` through muser-engine's `devices` feature (CoreAudio / WASAPI / ALSA; Linux needs libasound2-dev).
- **Changing manguebyte itself**: clone it next to muser and temporarily point the dependency at it (see the comment in the root `Cargo.toml`).

### Updating upstream

```bash
# bump the version in the root Cargo.toml, then
cargo update -p manguebyte
# if the upstream API changed, only muser-engine needs adjusting (the only crate coupled to manguebyte)
```

The code coupled to the engine **all lives in `muser-engine`**. core and cli never touch manguebyte,
so upstream changes affect a single crate.

---

## Data flow

```
natural-language intent
    │  (AI translates)
    ▼
Project JSON  ──Project::from_json──►  muser_core::Project
                                           │  Project::validate()
                                           ▼
                                 muser_engine::Renderer (streaming)
                                           │
                 ┌─────────────────────────┼─────────────────────────┐
                 ▼                          ▼                          ▼
          each Track                instrument::resolve         each clip / note
     semantic instrument  ──►   manguebyte instrument    ──►  turned into sample-accurate
           name                       (Synth<V>)               events (note_on / note_off)
                                           │  each built-in instrument computes a stretch ahead on its own performer thread
                                           ▼
             each process() moves forward one stretch: the nodes are processed in routing-plan order
             (instrument → effect chain → fader → into buses / sends / sidechain keys → master effect chain → master gain)
                                           │
             ┌─────────────────────────────┼─────────────────────────────┐
             ▼                             ▼                             ▼
   render(): from start to end    muser view: plays while        muser view --device:
   → wav::write_file              rendering (sent to the          lock-free queue →
                                  browser in pieces)              local sound card
```

Offline rendering is the streaming renderer run from start to end; the two are sample-identical (the test `streaming_matches_offline` compares them with arbitrary block sizes).
Effect chains process fixed-size blocks counted from the start of the project, and every stretch of streaming is aligned to a common multiple of those blocks, so splitting into stretches gives the same result as processing the whole song at once;
effects with latency (limiter look-ahead, plugins) make a node take its effect chain's output `latency` frames later, and the upstream nodes compute that much more.
During sound-card playback, faders, mute and solo take effect in the playback thread from the next stretch (`Renderer::set_mix`); the delay you hear is about the queue length (tens of milliseconds).
Seeking starts straight from the target position, as a DAW does (`Renderer::new_at`): notes and pedals still held at that point are sent again (chase), effects start from an empty state,
and a track's instrument output is taken from the cache when it is there; an SFZ sound is loaded only once per process (`sfz::load_shared`), so rebuilding does not read the samples again.

### Time model

The core layer uses **beats** as its only unit of time: `1 beat = one quarter note`.
The engine layer converts:

```
seconds = beats × (60 / tempo)
samples = seconds × sample_rate
```

So the AI never has to think about the sample rate, and changing the tempo does not change note times.

### Instrument resolution

A track refers to its instrument by a **semantic name** (such as `"drums.808"`, `"synth.poly"`, `"synth.fm"`),
which `muser_engine::instrument::resolve()` maps to a concrete manguebyte instrument.
Each instrument's Voice is a different Rust type; the `TrackInstrument` enum erases the type
and exposes a uniform `prepare / note_on / note_off / process`. Drum kits use MIDI channel 9 by convention.

---

## Project file format (`*.muser.json`)

The project file is muser's core contract: **this JSON is what the AI produces and edits**.

### Structure

```jsonc
{
  "schema_version": 1,          // format version, for future migrations (optional, default 1)
  "name": "example",            // project name (optional)
  "tempo": 90.0,                // required, quarter notes per minute, > 0
  "time_signature": {           // time signature (optional, default 4/4)
    "numerator": 4,
    "denominator": 4
  },
  "master_gain_db": 0.0,        // master output gain in dB (optional, default 0)
  "master_effects": [           // master effect chain (optional, default empty; applied before the master gain)
    { "type": "compressor", "threshold_db": -16, "ratio": 3 }
  ],
  "seed": 0,                    // random seed (optional): decides random automation that has no seed of its own
  "automation": [               // master automation (optional): targets master_gain_db / master_effects[i]
    { "target": "master_effects[0]", "param": "ratio", "shape": "points", "points": [[0, 2], [16, 4]] }
  ],
  "buses": [                    // buses (optional): sum what is sent in → effects → gain_db/pan → output
    {
      "name": "reverb",         // required, unique, not "master", no "."
      "effects": [{ "type": "reverb", "mix": 1.0 }],
      "gain_db": 0.0, "pan": 0.0, "muted": false,
      "output": "master",       // destination (optional, default master): master or another bus
      "sends": {},              // sends to other buses (dB)
      "automation": []          // targets effects[i] / gain_db / pan / sends.<bus>
    }
  ],
  "tracks": [                   // the tracks
    {
      "name": "drums",          // track name (optional)
      "instrument": "drums.lofi", // required, instrument name (see `muser instruments`), or sfz:/path.sfz, plugin:/path.vst3
      "instrument_params": {    // sound shaping (optional): overrides the preset; keys and ranges in `muser instruments <name>`
        "kick": { "tune": -2 }  //   drum kits per piece; melodic instruments take the parameters directly, e.g. { "cutoff": 800 }
      },
      "gain_db": 0.0,           // track gain in dB (optional, default 0)
      "pan": 0.0,               // pan -1 (left) to 1 (right) (optional, default 0)
      "muted": false,           // muted (optional, default false)
      "effects": [              // track effect chain (optional, default empty; applied before gain / pan)
        { "type": "reverb", "mix": 0.3 }
      ],
      "output": "master",       // destination (optional, default master): master or a bus name
      "sends": { "reverb": -6 }, // post-fader sends to buses (optional): bus name → dB
      "automation": [           // track automation (optional): targets instrument / effects[i] / gain_db / pan / sends.<bus>
        { "target": "pan", "shape": "lfo", "period_beats": 8, "min": -0.5, "max": 0.5 }
      ],
      "clips": [                // the clips
        {
          "start": 0.0,         // required, clip start (beats, from the start of the project)
          "length": 4.0,        // required, clip length (beats), > 0
          "legato": false,      // legato (optional): each note runs into the next note of a different pitch, overlapping about 40 ms
          "pedal": [[0.1, 4.03]], // sustain pedal (CC64) down intervals (optional): [down, up] in beats, relative to the clip start, increasing, not overlapping
          "notes": [            // the notes
            {
              "pitch": 36,      // required, MIDI pitch 0-127 (60 = middle C)
              "start": 0.0,     // required, note start (beats, relative to the clip start)
              "length": 0.25,   // required, note length (beats), > 0
              "velocity": 0.8,  // velocity 0.0-1.0 (optional, default 0.8)
              "accent": false   // accent (optional): drum kits and acid bass play louder and brighter
            }
          ]
        }
      ]
    }
  ]
}
```

### Constraints (checked by `Project::validate`)

- `tempo` must be > 0
- each clip's `start` ≥ 0 and `length` > 0; a note's `start` may be up to 1 beat before its clip's start (grace notes, pickups), but not before the start of the project
- notes running past the clip `length` are cut at the clip boundary when rendering
- drum-kit tracks use GM drum keys for `pitch` (36 = kick, 38 = snare, 42 = closed hi-hat, …)
- routing (`muser_core::routing::plan`): bus names are valid and unique; the buses named by output / sends / a compressor's `sidechain` exist;
  nothing routes to itself and there are no cycles (sidechain keys count as dependencies). Once it passes, the topological order is the processing order

### Time cheat sheet (4/4)

| You want | beats |
|---|---|
| a whole note | 4.0 |
| a quarter note | 1.0 |
| an eighth note | 0.5 |
| a sixteenth note | 0.25 |
| one bar | 4.0 (= `time_signature.beats_per_bar()`) |

---

## Current state (MVP)

The end-to-end loop that has been verified:

```bash
muser new -o demo.muser.json     # generate an example project
muser validate demo.muser.json   # validate
muser render demo.muser.json -o demo.wav   # render offline
# → a valid 16-bit / 48 kHz / stereo WAV
```

Implemented:
- Offline rendering of multiple tracks, clips and notes
- 115 built-in sounds: 10 drum kits, 68 poly synth presets, guitar / bass / organ / mallets / strings / winds / choir
- The instrument and effect description tables (names, parameters, ranges, defaults, descriptions) are written once, in the code:
  core `EFFECT_SPECS`, engine `instrument::catalog()` and `params.rs`; the CLI, MCP, `validate` and the `skills/` docs are all derived from them
- Built-in instrument parameters `instrument_params`: each kind of instrument exposes its common parameters (uniform names, real units), overriding the preset;
  drum kits are set per piece; rendering detects diverging values and reports an error
- `sfz:` built-in SFZ sampler (manguebyte-sampler): loads .sfz sound libraries directly, no external plugin needed
- A generic layer for external plugins (`host` feature): scanning (in a separate process, cached), info, parameter search, option text; plugin parameters are set by **name**,
  and the value can be the text the plugin displays (round-trip checked, with a lookup-table fallback); used in the `instrument_params` of `plugin:` instruments, the `params` of `plugin` effects,
  and automation; after activation the plugin is warmed up silently, and parameters it did not take are sent again
- Plugin profiles: pure-data JSON, matched by plugin id (ids in several formats), with aliases, common values, audition notes, presets, setup, experience,
  sources and the verified version; user directory > `MUSER_PROFILE_PATH` > built-in; structure validation and text cleaning (in preparation for community sharing);
  automatic drafts and audition write-back; rendered as skill docs. Reserved keys in project parameters: `@state` / `@setup` / `@preset`
- Plugin state: saves / loads a plugin's complete sound; the file header records the plugin id and version, and loading it into the wrong plugin is refused
- Profile extensions: `selector` (for plugins whose "type / program" lives only in their state: rewrites a root attribute of the JUCE XML state to switch it and refreshes the parameter table,
  as with Airwindows Consolidated's `effect`); `state_format: "vital"` (parses the JSON preset in Vital's state:
  `@wavetable` generates a wavetable, `@modulations` writes the modulation matrix); `source.lang: "en"` generates an English skill
- Built-in profiles (`profiles/`): Surge XT, Surge XT Effects, Vital, Airwindows Consolidated (its effect list comes from the Airwindows registry,
  intersected with the installed version), Plasma
- Plugin transport: rendering passes the project tempo and time signature to plugins (advancing block by block), so tempo-synced LFOs and delays are right; plugin dynamic libraries stay loaded in the process and are reused
  (some plugins crash on unload while a background thread is still running); paths support `~/`
- Audition: renders a test sound for each value of a parameter and reports loudness, spectral centroid, band shares, attack and tail: an "ear" for the AI
- Automation: three shapes, points / lfo / random (seeded), targeting built-in instrument parameters, effect parameters,
  track gain and pan, master gain and master effects; instruments and effects update every 64 frames, gain and pan every frame
- Track gain, equal-power pan, master gain, a reverb tail at the end
- Effect chains: reverb / delay / compressor / multiband_compressor / chorus / eq (filter) / limiter / phaser / flanger / tremolo / auto_pan / widener / transient_shaper / tape / bitcrusher / ring_mod / frequency_shifter / formant_filter / granular_delay / vocoder (sidechain-modulated), on a track, a bus or the master bus
- Routing: buses (`buses`), the `output` of tracks and buses and post-fader `sends` (automatable), buses into buses; a compressor's `sidechain`
  is triggered by the post-fader signal of any track or bus (a muted key track still triggers); processed offline in topological order, buffers freed as soon as they are used;
  effects with latency (limiter look-ahead, latency reported by plugins) are compensated on their own paths, so parallel signals stay aligned.
  The AI only writes these simple fields and never touches a node / connection graph
- Plugin host (`host` feature): loads third-party CLAP / VST3 / LV2 plugins as **effects**
  (`{ "type": "plugin", "path": "..." }`), activated as a Processor by manguebyte-host.
  Verified with a real plugin: loading the local `Surge XT Effects.vst3` gives output clearly different from the plugin-free control.
- Third-party plugins as **instruments** (`host` feature): a track's `instrument` is `plugin:path[#id]`,
  and notes are fed to the plugin through `process_with` + `ProcessContext`. Verified with a real VSTi:
  `Surge XT.vst3` has an RMS of 895 with notes and 0 without (complete silence), showing it responds to notes.
- Player page `muser view`: a local server (tiny_http, bound to 127.0.0.1 only, checking Host / Origin). A background render thread merges requests and reuses
  a `RenderCache` (each track's instrument output is cached under a key of "what affects the sound", with leading and trailing silence trimmed), so changing volume / mute / solo only re-runs the effect chains, buses and master;
  the page fetches 16-bit PCM in pieces as playback progresses and schedules it with Web Audio, switching to a new version seamlessly from the current position; saving writes back only the changed fields; it polls for file changes
- Streaming render: offline WAV, play while rendering and the local sound card share `Renderer`, with sample-identical results; built-in instruments each play in parallel on a performer thread
  (plugin instruments and effects on the render thread); denormal floats are turned off while rendering (`NoDenormals`), so almost-silent tails no longer slow down filters and reverbs
- Plugin instances are released about a minute later (on the same thread), so plugin background threads (such as JUCE's online update check) do not touch freed objects
- CI / release: GitHub Actions builds and tests on Linux, macOS and Windows (default and plugin-host features); pushing a `v*` tag builds
  `muser` + `muser-mcp` (with the plugin host) for Linux x86_64 / arm64 (each in a desktop and a headless build), macOS arm64 / x86_64 and Windows x86_64, with the docs and SHA256 sums, published to a GitHub Release
- MCP server (`muser-mcp`): 34 tools (project lifecycle + incremental editing + routing + automation + disk read / write + rendering + instrument / effect lookup + external plugins + audition + plugin profiles and state), over stdio

Not implemented yet (roadmap): the 0.6 yinse synthesizer (muser's own sound engine, made for AI), among others.
