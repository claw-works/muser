# muser tool list (CLI + MCP design)

muser's interface layer is exposed to AI in two forms:

1. **CLI** (implemented): the AI calls `muser <subcommand>` through the shell.
2. **MCP server** (planned): wraps the same capabilities as tools an agent can call directly.

Both share the same semantics, so behavior is consistent.

---

## 1. CLI (implemented)

Executable name: `muser` (from the `muser-cli` crate).

### `muser render <project> [options]`
Read a project JSON and render it offline to WAV.
- `<project>`: project file path (`.json`)
- `-o, --output <path>`: output path (default `out.wav`); a path ending in `.mp3` writes MP3 directly (320 kbit/s). The WAV carries a marker (cue point) at the start of each section
- `--stems <dir>`: export stems instead of the full mix: one 24-bit WAV per unmuted track (with effects and sends, without master effects, all tracks the same length)
- `--sample-rate <hz>`: sample rate (default 48000)
- `--tail <seconds>`: seconds of tail at the end (default 2.0)
- `--seed <n>`: temporarily override the project's random seed (the file is not changed): `humanize` and random automation take another pass
- **Output**: `Rendered '<name>' -> <out>: <duration>s, peak <dBFS>`

```bash
muser render beat.muser.json -o beat.wav
```

### `muser validate <project>`
Validate the project JSON: tempo, clips, notes, pan, instrument names, and whether every effect parameter is within its allowed range.
Error messages give the exact location, e.g. `tracks[0].effects[0]: reverb.mix = 1.8 out of range [0, 1]`.
- **Output**: `OK: '<name>' is valid - tempo <t>, <n> tracks, duration <d> beats`, or a non-zero exit code + the errors.

### `muser instruments [name] [--family <F>] [--json]`
- No argument: list all built-in instruments with descriptions.
- `--family`: list only one family (drums / synth / poly / guitar / bass / organ / mallet / strings / wind / choir).
- `<name>`: show the instrument's family, pitch convention (GM drum keys / pitched notes), recommended range, polyphony, description,
  and its tunable parameters (the keys of `instrument_params`, ranges, units, descriptions, the preset's current values, and whether each can be automated).
- `--json`: machine-readable output.

### `muser effects [name] [--json]`
- No argument: list all effects with a one-line description.
- `<name>`: show the effect's placement advice, each parameter's default / range / unit / sonic description, and a JSON example.
- `--json`: machine-readable output (the effect description table from the code).

### `muser plugin scan|info|params|values`
- `scan [--kind instrument|effect] [--refresh]`: scan the machine's standard plugin directories (each plugin is read in a separate process; results are cached) and give `path#id`.
- `info <plugin>`: name, vendor, type, whether it accepts notes, inputs and outputs, total parameter count and counts grouped by name, other plugins in the same file.
- `params <plugin> [--filter keyword] [--params JSON] [--limit N] [--all]`: parameter name, range, current value and text, id;
  `--params` applies base parameters first (use it when a parameter's meaning changes with them).
- `values <plugin> <param>`: the text values the parameter accepts (for choice parameters, all the options).
- `profile list|show|draft|check|path`: plugin profiles (data files holding common parameter aliases, presets, setup, and experience notes).
  `show <plugin|profile.json> [--markdown]`; `draft <plugin> [--audition] [--save | -o file]`; `check <profile.json> [--plugin <plugin>]`.
- `save-state <plugin> [--params JSON] -o <file>`: save the plugin's full state; load it in a project with `"@state": "<file>"`.
- `edit <plugin> [--params JSON] -o <file>` (macOS): open the plugin's own UI so a person can pick a sound, and save the state when the window closes; the 🎛 panel of `muser view` uses this.
- `<plugin>` is a path and may carry `#plugin-id`. All support `--json`.

### `muser sounds list|describe`

- `list [--kind builtin|sfz|plugin] [--filter word] [--json]`: the sounds available on this machine (built-in instruments, SFZ in `$MUSER_SFZ_PATH`, scanned instrument plugins); ones already listened to carry a one-line card description.
- `presets <plugin> [--filter word] [--json]`: a plugin's built-in sounds (name / sound library / category / tags of VST3 standard presets, Surge `.fxp`, Kontakt `.nki`),
  marked by whether they can be used directly: auto (converted straight to a state) / captured (already captured) / manual (must be loaded once in the plugin window).
- `state <plugin> <sound>`: fetch this sound's state file (in the machine's sound library `~/.config/muser/sound-library/`) and print how to write it in a project.
- `save <plugin> <name> <state-file>`: save a plugin state into "My sounds" in the sound library (same name overwrites); afterward it is available in `presets` and the view drawer.
- `describe <sound> --keys [--from 36 --to 96 --tempo 120 --hold 4]`: key map card - hold each key for a few bars and measure whether it sounds, whether it is a loop that follows the tempo,
  how many bars per cycle, onsets per bar, loudness, brightness (loop kits, drum kits, multi-zone sounds).
- `identify <plugin> <state-file>`: identify which built-in sound the state has loaded (compares the sound file's raw bytes).
- `capture <plugin> <sound>` (macOS): open the plugin window and select the sound file in Finder; a person or an agent (computer use) loads it and closes the window, and the state is saved into the sound library.
- `describe <sound> [--params JSON] [--drums] [--refresh] [--json]`: "listen" to a sound: render probe notes and measure brightness, attack, sustain, decay, range, velocity response, stereo, and pitch,
  write them as a text card, and cache it on this machine (`~/Library/Caches/muser/sounds`, `~/.cache/muser/sounds`). For plugins, use `--params '{"@state": …}'` to specify the sound.

### `muser audition`
Audition what a parameter does: `--instrument <instrument>` (or `--effect '<effect JSON>'` + `--source <instrument>`),
`--params '<base parameter JSON>'`, `--param <param-name> --values=a,b,c` (numbers, or text the plugin can parse),
`--pitch` `--hold` `--tail` `--out <dir>` (write each version's wav) `--json`.
Reports peak, RMS, LUFS, spectral centroid, low / mid / high band shares, attack time, and decay length. Works for built-in and external sounds.
`--record` writes the result into that plugin's user profile.

### `muser audio <file> [--tempo T] [--json]`

An audio file's format, duration, sample rate, and channels; with `--tempo` it converts the duration to beats (use it to write an audio-track clip's `length`). See [skills/audio.md](skills/audio.md).

### `muser routing <project>`

Print the signal flow: processing order, where each track / bus goes (output, send amounts), effect chains, and sidechain relationships. Take a look after changing routing.

### `muser mix-report <project> [--json]`

Mix report: render the whole piece once and list the loudness of every track / bus / master - integrated LUFS, maximum short-term LUFS, and peak of the input
(before the effect chain; for a track, the dry instrument) and the output (after the fader); each track's estimated loudness at the master (adding the `gain_db` of the buses on its output path) and its difference from the loudest track;
plus warnings such as silent, too loud, master overload, and clipping. Use it to balance each track's `gain_db`; for the workflow see [skills/mixing.md](skills/mixing.md).

### `muser view <project> [--port 8737] [--open]`

A playback page for people to listen: starts a local server on `127.0.0.1` and opens `http://127.0.0.1:8737/` in the browser (`--open` opens it automatically).
Timeline (waveform, clips, notes), play / loop / seek (the space bar plays / pauses at any time), a volume fader, mute, solo, and level meter for every track and bus,
signal chain (effect blocks -> destination, send amounts), and an overall signal-flow diagram (output, send, and sidechain connections).

- The page fetches already-mixed audio from the server in segments as playback advances, so the browser holds only a few seconds in memory, however long the piece is.
- Changing volume, mute, or solo is handed to the engine to re-render (instrument output is cached; only effects, buses, and master are rerun; a 2-minute, 15-track piece takes about 1.7 seconds),
  and the new version is swapped in seamlessly from the current playback position; the result is accurate. Solo is for auditioning only and is not written to the project.
- A track with volume automation (`gain_db`): the automation is absolute dB, and a fader change shifts it as a whole by the same amount (trim), written back together on save.
- Clicking an effect block opens its panel: an On switch (bypass), and for built-in effects every parameter as a slider or a choice (double-click a slider for the default), re-rendered as you drag;
  for a plugin effect, "Open the plugin window" opens the plugin's own window (Guitar Rig, Surge XT Effects…): every change there is saved into
  `<project>.<node>.fx<n>.muserstate` and written into that effect's `params."@state"`; "Discard window changes" restores how it was.
- "Export" downloads the current mix as WAV or MP3 (320 kbit/s), MIDI, ABC / ABC (YuE2), a DAW package (zip: MIDI, mix, one stem per track, an import guide; same as `muser export-daw`)
  and visualization data (JSON, same as `muser export-data`).
- "Save to project file" writes back only what was changed on the page (faders, mutes, effect bypass and parameters, recordings, split / moved audio clips, whole-piece transpose); "Undo changes" returns to the values in the file; "Copy changes JSON" hands the changes to the AI.
- **Record vocals** (right panel): record from the playhead or the loop range, with count-in, fine latency adjustment, and "measure latency"; each take is stored at `recordings/<track-name>-take-NN.wav` (manifest `recordings/takes.json`),
  and pasted onto the audio track (pending save); in the list each take has a "Use" checkbox (several can be ticked and overlap, e.g. when recording line by line) and "Delete". The project's `lyrics` are shown at the bottom of the screen as karaoke (the "Lyrics" toggle).
- **Audio clips**: ✂ (or C) splits, drag to move (Shift snaps to 1/16), and with a clip selected ← → nudge by 10 ms each, Delete removes it (e.g. the unwanted piece after a cut; the recording stays in the list). **Transpose − / +**: transpose the whole piece (`transpose`).
- If the project file is changed by someone else (the AI): it re-renders automatically and the page refreshes (unsaved changes on the page are discarded).
- Listens on the local machine only, with no authentication (requests with a non-local Host/Origin are rejected). On a remote machine, access it with SSH port forwarding. Use a release build.

Changes JSON format: `{"changes": [{"node": "tracks[3]", "gain_db": -9}, {"node": "buses[1]", "muted": true}]}`.
Sharing your work: share the project JSON (and the plugin state files) or the rendered WAV.

### `muser poly edit|list|show|schema`

The poly synth's full sound (oscillators, sub / noise, two filters, four envelopes, three LFOs, a 16-slot modulation matrix).
- `edit [--port 8739] [--open]`: a local web sound editor (listens on 127.0.0.1 only). Start from a preset or the machine's sound library; after you change parameters it automatically renders a short
  audition phrase (chord, single note, bass line, arpeggio, legato melody, optional root note) and saves into the machine's sound library `~/.config/muser/poly/<name>.json`.
- `list [--json]`: the sounds in the machine's sound library.
- `show <name> [--sound]`: print the JSON to write into a project track (`"instrument": "synth.poly"` + the full sound in `instrument_params.patch`);
  the name can be one in the sound library, `poly.<preset>`, or `synth.poly`. `--sound` prints the sound file itself.
- `schema`: the structure of the full sound (each field's type, range, options, description); write `instrument_params.patch` according to it.

### `muser play <project> [--seed N] [--from seconds]`

Play the project from the start on the machine's sound card and exit when done: a quick listen to a draft for a person (no web page). `--seed` temporarily takes another pass of the performance (humanize / drummer /
random automation) without changing the file. Needs the device feature (release builds include it; the headless build does not - use `render` and then listen).

### `muser live start|send|status|wait|autoplay|stop`

Live session (for how, see [skills/live.md](skills/live.md)). `start <template> [--detach] [--autoplay 3m] [--record s.muser.json] [--open] [--port 8747]`
is long-running, holds the machine's sound card, and listens on 127.0.0.1 only (`--detach` starts in the background and returns once playing begins; agents use this); the other subcommands control it: `send <section.json|->` submits a section (returns a receipt immediately), `status`,
`wait [--runway 8]` (at most 30 seconds), `mix <track|bus|master> <dB>` (adjust volume during the session with a 0.3-second ramp, without disturbing the form), `autoplay <duration>`, `stop` (write the recorded project and exit). Templates are in `samples/live/`. Needs the device feature.

### `muser loop edit|list|show|play|save|share`

The loop library: drum rhythm patterns (`grid`) and melody / motif / harmony patterns (`notes`). Three locations; when names collide, the nearer wins:
project (the `loops/` next to the project, or the current directory's `loops/` when there is no project) > global (`~/.config/muser/loops/`) > built-in (common drum patterns).
Each command's `--project <project-file>` specifies the project directory.
- `edit [name] [--project P] [--port 8739] [--open]`: the loop / motif editor (local web page, listens on the local machine only). Melodies use a piano roll (click, drag, change length,
  velocity, transpose, undo, snap to the grid or free durations); drums use a step grid; it renders with muser's instruments and auditions in a loop; "● Hum to record" uses the microphone to hum a phrase and converts it directly to notes;
  save to the project or global, or share one from the project to global.
  When the agent changes a loop file, the page re-reads it automatically (and warns if there are unsaved changes).
- `list [--style S] [--json]`: list them (marking project `+` / global `*`), filtered by style or tag.
- `show <name> [--start beat] [--repeat N] [--file | --abc]`: print the clip to write into a project (`grid` / `notes` + inline `humanize`, with `from` noting the source);
  `--file` prints the loop file (with suggested instrument, key, chords); `--abc` prints ABC notation.
- `play <name> [--instrument X] [--tempo BPM] [--repeat N]`: audition on the sound card.
- `save <project> --track I [--clip J] --name N [--style S] [--description D] [--step 0.25] [--global] [--overwrite]`:
  save a clip from the project as a loop (drums: grid, with ordinary notes quantized to the grid; melody: notes), by default into the `loops/` next to the project.
- `share <name> [--overwrite]`: copy a project loop to global so other projects can use it.
- `hum <audio> [--name N] [--tempo 100] [--grid 1/16|1/8|1/8t|0.25|off] [--from-start] [--key Am] [--instrument X] [--global]`:
  convert humming to a score (no AI). A single-melody recording (hummed, sung, whistled, single-note instrument) -> notes, saved as a melody loop, and the ABC is printed. Pitch is detected with the McLeod method (every 10 ms),
  then median-filtered and corrected for overall tuning; notes are split by pauses / pitch jumps / loudness dips on the same pitch; after subtracting the overall early / late offset they are aligned to the grid, and the key is estimated.
  `--from-start` keeps the blank from the start of the recording to the first note (use it when recording along to a metronome). The editor's "● Hum to record" is the same pipeline (the browser records the microphone, optionally with a one-bar count-in and a metronome).
- Loop file format (`format: "muser.loop"`, `version: 2`): `name` `bars` `tempo` `style` `description` `tags` `kit` / `instrument`
  `key` `chords` `grid` / `notes` `humanize`. The drummer's `loop` can also reference a drum loop in the project or global.
For the clip shorthands (`abc` / `grid` / `drummer`) and `humanize`, see [skills/notation.md](skills/notation.md) and [skills/humanize.md](skills/humanize.md).

### `muser midi <project> [-o out.mid] [--seed N]`

Export a standard MIDI file (format 1, 480 ticks per quarter note): the first track holds tempo and time signature, then one MIDI track per instrument track.
Notes match the render exactly: humanized timing and velocity, legato extension, clip trimming, abc / grid / drummer expansion; the sustain pedal is written as CC64,
and `gain_db` / `pan` as CC7 / CC10. Drum kits go on channel 10, and other tracks get a GM sound guessed from the instrument name. Muted tracks are exported as usual (track name marked "(muted)"),
and audio tracks are not exported. The default output is a `.mid` with the project's name. The "Export" in the `muser view` page can also download the MIDI, the full mix (WAV / MP3) and a DAW package.

### `muser analyze <project> [--audio] [--json]`

A listening report, so an agent that cannot hear can self-check an arrangement. By section (each pass of the form when there is a `form`, otherwise every 8 bars) it lists:
density (onsets per bar), each track's onsets per bar / sounding share / register, chords and harmonic rhythm (the written `chords`; when not written, estimated and marked with `~`),
repetition (the share of bars identical to some earlier bar), and clashes (long notes on beats are avoid notes; two tracks sounding long notes together as a minor second / minor ninth).
`--audio` renders once and adds each section's loudness, spectral centroid, and low / mid / high band energy shares. At the end come the suggestions: chorus didn't lift, too much repetition, only one chord,
register masking, clashes, section loudness too flat, too dark / too bright. At the top is the energy curve (0-10 per section); there is also a set of "AI tells" checks (`tells`):
sections all multiples of 8 bars, energy only rising and never falling, unchanged instrumentation, a single harmonic loop / never leaving the key, a fill every 4 bars, everything aligned to the grid, every chorus identical,
and (`--audio`) unchanging dynamics and brightness. See [skills/arrange.md](skills/arrange.md).

### `muser abc <project> [-o out.abc] [--yue2] [options]`

Export ABC notation (printed to stdout by default), writing the notes on the score (before humanize, after clip trimming; abc / grid / drummer expansions all count);
drum kits and audio tracks are not exported. Start times and durations are quantized to a 1/16 (1/32, 1/64 when needed) note grid, and triplets and the like are rounded to the nearest (with a notice).

- Standard form (default): ABC 2.1 multi-voice, one `V:` per instrument track (low register uses the bass clef), polyphony written as `[CEG]`, ties across bars.
  `--track <name>` (repeatable) exports only those tracks; `--chords` adds chord symbols.
- `--yue2`: [YuE2](https://github.com/multimodal-art-projection/YuE)'s native score - two monophonic voices, `Vocal` / `Ins`,
  with chord symbols written in `Vocal` (only the 15 chord types it supports), durations using only 1 2 3 4 6 8 12 16 24 32 48 units (others built with ties),
  and every group of 1-4 bars as one `V: Vocal` block followed by one `V: Ins` block. For `--cot full`; add `--no-chords` for `--cot melody` (cover / adaptation).
  - `--vocal a,b,c`: the tracks that sing the melody; list several when the melody passes between instruments (after merging, the highest note at each moment is taken); by default chosen by name
    (vocal / lead / melody...), otherwise the track that sounds longest in the higher register. `--ins <name>`: the instrumental melody voice (default all rests).
  - `--sections intro:4,verse:8,chorus:8`: sections are written as comments such as `% verse`, corresponding to the `[Verse]` tags in the lyrics.
  - Use at least a `L:1/32` grid (YuE2's native exports commonly use 1/32; third-party testing shows scores with `L:1/16` stop generating halfway).
  - `--vocal-octave 1`: raise Vocal by an octave overall, writing into the lowercase note-name range (third-party testing shows enunciation is clearer this way; the official docs do not say so).
  - The export passes validation with YuE2's `abc_tools.py inspect`.
- `--key G` / `--key F#m`: specify the key signature; by default the `K:` written in the clips' ABC is used (when they all agree), otherwise it is estimated from the pitches of the whole piece.
- Chord symbols are estimated from the accompaniment notes of each bar (a half bar when necessary): reliable when the accompaniment is clear, only a guess when there is just a bass line; edit them by hand before handing to a model.

The "Export" in the `muser view` page can also download ABC and ABC (YuE2) (all with default options).

### `muser export-daw <project> -o <folder> [--sample-rate 48000] [--tail 2]`
A folder for a DAW: `<name>.mid` (all instrument tracks, tempo, time signature, section markers), `stems/NN-trackname.wav` (24-bit, aligned from bar 1), `mix.wav`,
and `IMPORT.md` (import steps for Live, Cubase, Logic, FL Studio, Studio One, Bitwig, REAPER, GarageBand, Pro Tools).

### `muser export-data <project> [-o data.json] [--fps 30]`
JSON for visualization: each track's notes `[start seconds, pitch, velocity, duration seconds]`, sections (start and end seconds, time signature), tracks' and master's automation curves, `bars` / `beats` (the second each bar and beat starts, following tempo and meter changes), and `lyrics` (each line and each syllable with its start / end seconds, timed by the guide melody, as in the `muser view` karaoke); `--fps` adds the per-frame overall / low-frequency level (dB).

### `muser takes <project> [--timing] [--clean]`
Each take of a vocal recording: start, length, level, the part actually sung, and whether the project uses it. `--timing` measures, phrase by phrase, how many milliseconds early / late the vocal is against the guide melody (the track with `lyric`);
`--clean` deletes recordings the project does not use (ask the user first).

### `muser clip split|move <project> --track <track-name> …`
Split / move clips on an audio track: `split --at <beat>`; `move --clip <index> --to <beat>` or `--by-ms <milliseconds>` (negative moves earlier). Only that track's clips are changed.

### `muser migrate <project…> --from <version> [--write]`
Convert projects written for an older version to the current one (e.g. 0.3.0 -> current: the built-in instrument volume calibration). By default it only lists the changes.

### `muser self-update [--check] [--version vX.Y.Z]`
Update to the latest version on GitHub using the official install script (needs curl).

### `muser new [-o <path>]`
Generate a sample project JSON (the AI can use it as a reference for the standard format). Prints to stdout by default.

---

## 2. MCP Server (implemented: `muser-mcp`)

The crate `muser-mcp` runs an MCP server over **stdio**, so agents like Claude/Cursor
can drive the engine with structured tool calls **without a shell**. It depends on `muser-core` + `muser-engine`
and shares its source with the CLI.

### Running

```bash
muser-mcp          # start the MCP server over stdio (during development: cargo run -p muser-mcp)
```

In an MCP client's configuration (e.g. Claude Desktop), just register the `muser-mcp` executable as a
stdio server. On initialization the server returns usage instructions telling the agent
how to use beats as time, GM drum keys, semantic instrument names, and so on.

### Implemented tools (48)

| tool | purpose |
|---|---|
| `create_project` | Create an empty project, returns `project_id` |
| `get_project` | Get the project's full JSON |
| `set_project` | Replace the whole project with a full JSON |
| `validate_project` | Validate the project |
| `load_project` | Read a project JSON from disk into the session, returns a new `project_id` |
| `save_project` | Write the session's project to a JSON on disk |
| `set_tempo` | Set the tempo |
| `add_track` | Add a track (can carry `instrument_params`, `output`, `sends`), returns `track_index`; instrument name, parameters, and routing are validated immediately |
| `remove_track` | Delete a track |
| `update_track` | Update gain / pan / mute, change the instrument, `output`, merge-update `instrument_params` and `sends` (a null value deletes that key); validated immediately |
| `set_bus` | Add or replace (same name) a bus: effects / gain_db / pan / muted / output / sends / automation; name, references, and cycles are validated immediately |
| `remove_bus` | Delete a bus; if it is still referenced, errors and points out where |
| `get_routing` | View the signal flow: processing order, destinations and send amounts, sidechain relationships |
| `add_clip` | Add a clip (can carry the sustain pedal `pedal`), returns `clip_index` |
| `set_pedal` | Set a clip's sustain pedal ranges (CC64; keys released while the pedal is down keep sounding) |
| `add_notes` | Add a batch of notes to a clip (supports `velocity`, `accent`) |
| `clear_clip` | Clear all notes in a clip |
| `set_automation` | Set the automation of a track (`track_index`), bus (`bus`), or master (replace or append), validated immediately |
| `set_seed` | Set the project's random seed (determines random automation) |
| `render` | Render offline to WAV, returns duration / peak |
| `export_midi` | Export a standard MIDI file (notes match the render; audio tracks are not exported) |
| `export_daw` | Export for a DAW: MIDI + per-track stem WAVs (24-bit) + mix + import notes (steps for Live / Cubase / Logic) |
| `export_data` | Export JSON for visualization: each track's notes (seconds), sections, automation curves, optionally a per-frame volume envelope |
| `export_stems` | Export stems: one WAV per unmuted track (with effects and sends, without master effects, all tracks the same length) |
| `analyze` | Listening report: by section, density, register, chords, repetition, clashes, and suggestions (`audio` adds loudness and spectral centroid) |
| `export_abc` | Export ABC notation: `standard` (ABC 2.1 multi-voice) or `yue2` (YuE2 native score) |
| `mix_report` | Mix report: each track / bus / master's dry and post-fader loudness, relative differences, and warnings (instrument output is cached per project) |
| `scan_plugins` | Scan the machine's external plugins |
| `plugin_info` | External plugin info and parameter groups |
| `list_plugin_params` | Search an external plugin's parameters (filter keyword; base parameters can be applied first) |
| `plugin_param_values` | The text values an external plugin parameter accepts |
| `audition` | Audition: render a test note for each parameter value and return loudness / brightness / bands / attack / decay; `record` writes into the profile |
| `list_profiles` | List plugin profiles |
| `get_profile` | Get a plugin's profile (JSON or Markdown usage notes) |
| `draft_profile` | Auto-generate a profile draft from a plugin |
| `save_profile` | Validate and save a profile to the user directory (can be checked against the plugin) |
| `save_plugin_state` | Save a plugin's full state (sound) file |
| `list_instruments` | List instruments (can filter by `family`), returns name / family / pitch convention / range / polyphony / description |
| `describe_instrument` | View an instrument's details and tunable parameters (including the preset's current values) |
| `list_effects` | List all effects with descriptions |
| `describe_effect` | View an effect's parameters (range / default / unit / sonic description), placement advice, and JSON example |
| `live_start` | Start a live session (starts `muser live start` in the background, in its own process group, so the music keeps playing after the agent session ends) |
| `live_send` | Submit a section (bar count, time signature, each track's clips / effects / automation, with times relative to the start of this section); immediately returns the effective bars and seconds |
| `live_status` | Which bar it is now, how many seconds are queued, whether it is looping, the tracks, how the last section was written, how long autoplay has left |
| `live_wait` | Wait until the runway is shorter than N seconds or the autoplay time is up (at most 30 seconds); returns the reason |
| `live_mix` | Adjust the volume of a track / bus / master during the session (0.3-second ramp, without writing a new section) |
| `live_autoplay` | Reset the autoplay duration |
| `live_stop` | End the session and write the recorded project |

Projects are managed as an **in-memory session** (`project_id → Project`). An agent's standard usage is **sequential** calls
(wait for each tool to return before sending the next), which matches real MCP client behavior.

> Note: the server holds a short lock for each tool call, so normal request/response agent interaction is safe;
> if a client abnormally issues several edits concurrently, they may interleave. In the future this could become a session-level lock per `project_id`.

### Design principles (original design, for reference when extending)

- **Projects are managed by handle + in-memory state**, avoiding reading and writing a large JSON on disk every time; `load`/`save` to disk are also supported.
- Each tool's input / output is **structured JSON**, with fields mapping one-to-one to the `muser-core` model.
- **Render returns metadata** (duration, peak, file path); the audio itself is written to a file, and the path is returned rather than binary.
- The tool granularity serves both agent working styles: "submit it all at once" and "incremental editing".

### Extensible tools (not yet implemented)

#### Project lifecycle

| tool | input | output | notes |
|---|---|---|---|
| `create_project` | `{ name?, tempo, time_signature? }` | `{ project_id }` | Create an empty project, returns a handle |
| `load_project` | `{ path }` | `{ project_id, project }` | Load from a JSON on disk |
| `save_project` | `{ project_id, path }` | `{ path }` | Save to disk as JSON |
| `get_project` | `{ project_id }` | `{ project }` | Get the current full project JSON |
| `set_project` | `{ project_id, project }` | `{ ok }` | Replace as a whole (good for "submit the full idea at once") |
| `validate_project` | `{ project_id }` | `{ ok, errors? }` | Validate |

#### Incremental editing (good for "conversational fine-tuning")

| tool | input | output |
|---|---|---|
| `add_track` | `{ project_id, name?, instrument, gain_db?, pan? }` | `{ track_index }` |
| `remove_track` | `{ project_id, track_index }` | `{ ok }` |
| `update_track` | `{ project_id, track_index, gain_db?, pan?, muted? }` | `{ ok }` |
| `add_clip` | `{ project_id, track_index, start, length }` | `{ clip_index }` |
| `add_notes` | `{ project_id, track_index, clip_index, notes: [...] }` | `{ added }` |
| `clear_clip` | `{ project_id, track_index, clip_index }` | `{ ok }` |
| `set_tempo` | `{ project_id, tempo }` | `{ ok }` |

#### Rendering and discovery

| tool | input | output |
|---|---|---|
| `render` | `{ project_id, output_path, sample_rate?, tail?}` | `{ path, seconds, peak_dbfs }` |
| `list_instruments` | `{}` | `{ instruments: [...] }` |
| `describe_instrument` | `{ name }` | `{ kind, note_convention, hint }` |

### Implementation path (reuse existing code with minimal change)

```
muser-mcp (new crate)
  ├─ maintains HashMap<ProjectId, muser_core::Project>  // in-memory session state
  ├─ each tool = one structured mutation of a Project / one muser_engine::render call
  └─ exposed as a stdio / SSE server with any Rust MCP SDK
```

Because the `muser-core` models are all serde, almost every tool's I/O can be derived directly from the model types,
and `render` calls `muser_engine::render_to_wav` directly - fully the same source as the CLI, with zero behavior divergence.

### Relationship to the CLI

The CLI and MCP are two faces of the same engine:
- The CLI suits **scripting / batch processing** and the "AI works through the shell" scenario.
- MCP suits **conversational agents** calling tools directly and keeping project state across a multi-turn session.

Both are only thin wrappers over `muser-core` + `muser-engine`.
