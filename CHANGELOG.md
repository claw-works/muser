# Changelog

Each release's notes on GitHub are the section for its tag below (the release workflow copies it).
Write the next version's section here before tagging; the heading must be `## vX.Y.Z`.

## v0.5.0

### Upgrade notes

- **muser speaks English now**: CLI output, `--help`, error and validation messages, `analyze` / `mix-report` reports, MCP tool descriptions and results, built-in loop and plugin profile descriptions, and the docs (`SKILLS.md`, `TOOLS.md`, `skills/`) are English only (the `muser view` / live pages keep their 中文 / EN switch; `README.zh-CN.md` stays). Agents answer in the user's language as before; scripts that matched the old Chinese messages need updating. Cached sound cards are re-probed once (their summaries were Chinese).
- SFZ instruments that use the built-in controller curves (`tune_curveccN=1` and the like) now play as designed: Ixox Flute, for one, was 50 cents sharp before and is in tune now, so projects using such libraries will sound different (better) after upgrading.

### Live performance (`muser live`)

- **Live performance (`muser live`)**: `muser live start <template>` keeps playing through this machine's sound card while an agent writes the music a section at a time (`send`), and never goes silent: when nothing new has arrived, the last section loops. Each section has its own bars and time signature; tracks, effects (`effects_add` / `effects_off`) and automation can join mid-song without a gap, since the engine swaps in the grown project while keeping every instrument's and effect's state. `status` / `wait` (runway, autoplay time limit) let an agent play on its own for a while and stop on a loop-friendly phrase; a page shows the playhead, the queued sections and levels; the whole session is written to `--record`. Six MCP tools (`live_start` … `live_stop`), three templates in `samples/live/`, and `skills/live.md`.
- **`muser live`, after a first real session**: `start --detach` runs the session fully detached and returns once it plays (the MCP `live_start` uses it), since some agent environments end everything a finished command started; `stop` silences at once, the page shows that the session is over (and has a Stop button); sections can carry a `name` (kept in the recorded project's section names, with repeats and cut sections marked) and change buses and the master (`buses`, `master`: effects and automation such as a `master_gain_db` fade, applied on a section's first pass only); receipts and status give `bar`, `beat_in_bar`, `absolute_beat`, `starts_in_seconds`; `status` / `wait` warn after a dropout and name the heaviest tracks (`load`); every submission is logged next to the recording in `<name>.changes.jsonl`.
- **Karaoke on the live page**: a section can carry `"lyrics"`, timed by a guide track whose notes have syllables (ABC `w:` lines) as in `muser view`; the page shows the current line in large type (sung words lit, the one being sung glowing), the next line under it, and a "get ready" cue just before a line starts.
- **Patching the section that is playing**: `"at": "patch"` replaces the current section from the next bar while staying at the same place in its form — write the whole section as before (adding drums, a new part), muser works out which bar it is up to; clips that cross the join keep only the notes that start after it; the receipt's `section_bar` says where it took over, and the loop afterwards repeats the patched section from its top. For when someone is singing along.
- **Mixing during a live session**: `muser live mix <track|bus|master> <dB>` (MCP `live_mix`) moves a fader within 0.3 s without a new section and without disturbing the form, recorded as automation from that moment; `status` lists every fader and its loudness right now (`mix`: `gain_db`, `rms_db`), so an agent can see that the sax is 10 dB under the piano before anyone says so.
- **`muser live` page and robustness**: the page puts the session's title on top (`start --title`, or `"title"` in a section), then what is playing now and what comes next with each section's one-line `note` (it flashes when a new section starts), the timeline, and a fixed status bar so nothing jumps; the playback queue is longer in live mode (about 340 ms, changes are scheduled 1.5 s ahead anyway); tracks and effect chains that have been silent for a second, with nothing coming up, are skipped; `status` lists slow moments in the playback thread (`slow`) and warns about dropouts since the agent last looked.
- **Tempo changes**: a section (in a project's `sections` or a live section) can carry `"tempo"` — the new BPM from its first beat on, held by the sections after it — and `"tempo_ramp_bars"` to get there gradually over its first bars (accelerando / ritardando, linear in time as CLAP's `tempo_inc`; 0 or omitted switches at the bar line). Clips and automation stay in beats and follow; plugins get the tempo map in their transport, MIDI export writes it (a ramp as one tempo per beat), ABC export writes inline `[Q:]`, and `muser view`, karaoke, takes and `live status` (`tempo` is the tempo right now) use it. A looping live section stays at the tempo it reached. Projects without tempo changes render bit-identically.
- **Effects can start and stop mid-song**: `"from"` / `"until"` (beats) on an effect — untouched before (empty state, as if plugged in then), cut after.

### Recording

- **Recording with `muser view --device`**: the backing plays through this machine's sound card and muser records the default microphone in the same audio callback, lined up by the driver's reported round-trip latency plus the latency nudge; **Measure latency** works there too (clicks out of the speakers, heard back through the mic). The nudge is remembered separately for in-browser and sound-card recording.

### SFZ

- **SFZ in live sessions**: looking an `sfz:` instrument up no longer parses the whole file (only `describe` needs its controls), so a reconcile with a large library takes ~5 ms instead of ~250 ms — the dropouts at the party; SFZ files with stray lines (such as `A0 bankname: …`) load with a warning instead of failing.
- **Built-in controller curves**: curves 1–3 (bipolar, inverted, inverted bipolar) on `tune` / `volume` / `pan` / `amplitude` controller modulations are applied — Ixox Flute's `tune_curvecc81=1` at its 0.5 default played 50 cents sharp.

### Export and plugins

- **ABC export**: sections with their own time signature get bars of the right length and an inline `[M:7/8]` where the meter changes; tracks with lyrics get `w:` lines (one syllable per sung note, `*` for notes without one). muser's ABC reader understands inline `[M:]` too.
- **Plugins follow per-section meters**: hosted instruments and effects see the time signature, bar start and bar number of the section that is playing, and after a jump in `--device` playback they see the position they jumped to (they were told the song started there).

### Engine

- **manguebyte follows upstream 66ef3dc**: partial voicing (air, attacks, wind jitter) for `PartialBank`, an organ's space (windchest, early reflections, swell box), plugin tail reporting, render pool and MIDI port fixes. Renders of the built-in samples are bit-identical.

## v0.4.0

### Upgrade notes

- Stems (`render --stems`, `export-daw`) are 24-bit WAV now (16-bit before). Projects from 0.3.0: run `muser migrate <project> --from 0.3.0 --write` to keep their balance under the calibrated instrument levels.

### Install and upgrade

- **One-line install**: `curl -fsSL https://raw.githubusercontent.com/claw-works/muser/main/scripts/install.sh | sh` (Windows: `irm …/scripts/install.ps1 | iex`) downloads the right release, checks it against `SHA256SUMS.txt`, installs it under `~/.local/share/muser/<version>` (`%LOCALAPPDATA%\muser\<version>` on Windows) and puts `muser` and `muser-mcp` on `PATH`; Linux without ALSA gets the headless build.
- **`muser self-update`** updates to the latest release through the same script (`--check` only looks, `--version` picks one).
- **`muser migrate <project> --from 0.3.0`** converts a project written for an older muser so it sounds the same now: for 0.3.0 projects it shifts the `gain_db` (and `gain_db` automation) of built-in instrument tracks by their calibration. It lists the changes; `--write` writes them back.

### Recording and vocals

- **Record a voice in `muser view`**: a "Record a voice" panel records from the playhead (or just the loop range) over the backing in the browser, with a count-in and latency compensation (plus a manual nudge). Every take is saved at once as `recordings/<track>-take-NN.wav` and listed in `recordings/takes.json`; the take is laid on an audio track (created if needed, replacing what it overlaps) as a pending change — the project file only changes when you press Save. Takes can be swapped in or deleted from the panel. `muser takes <project>` shows each take's length, level, where the singing actually starts and ends, and whether the project uses it, so an agent can trim and align them (`--clean` removes unused takes). A project's `lyrics` text is shown next to the recorder, and as karaoke at the bottom of the screen: each line highlights character by character in time with the sung syllables (Chinese characters map one-to-one onto the `lyric` syllables, English by word), with the next line below; a Lyrics switch in the transport bar turns it off.
- **Measure latency** in the recorder: with headphones off, muser plays 8 clicks, hears them through the microphone and fills in the latency nudge; the nudge is remembered on that computer.
- **`muser takes <project> --timing`** measures each sung phrase against the guide melody (the track whose notes carry `lyric`) and says how many milliseconds early or late it is, and whether a constant offset looks like recording latency.
- **Cut and move audio clips in `muser view`**: the ✂ button (or C) turns on scissors — click an audio clip to cut it there into two clips with short fades; drag an audio clip to move it (Shift snaps to 1/16 notes); click one to select it and nudge it with ← / → by 10 ms (Shift: a 1/16 note). Edits are pending changes until Save.
- **`muser clip split <project> --track vocal --at <beat>` / `muser clip move … --clip N --to <beat> | --by-ms -70`**: the view's scissors and nudges for agents, editing only that track's clips in the project file.
- **Humming lyrics**: notes on `choir.*` instruments can carry a `lyric` (pinyin or English spelling), or an ABC `w:` lyric line maps syllables onto the notes in order. Each note sings the syllable's vowel (diphthongs glide within the note, -n / -ng close to a hum), so a line hums with the vowels of its words; consonants are not sung in 0.4. Lyrics go into MIDI exports as lyric events. Sample: `samples/v0.4/moon-rising`.
- **Transpose the whole song**: a project-level `"transpose": 3` (−24 to 24 semitones) shifts every pitched instrument track as it plays — the written notes stay as they are, drums and audio tracks don't move, and chords in `analyze` follow. `muser view` has a Key − / + control for trying keys for a different singer before recording; Save writes it.

### Writing music

- **A time signature per section**: `"sections": { "bridge": { "bars": 4, "time_signature": { "numerator": 7, "denominator": 8 } } }`. Chords follow the section's bar length; `analyze` bar numbers, warnings, MIDI export (a time-signature event at each change), `export-data` and the bar lines in `muser view` follow the meter changes. The drummer still writes 4-beat grooves, and ABC export lays bars out in the project meter (with a note).
- **Loops that run across sections**: a part with `"align": "global"` counts its loop from the start of the song instead of restarting at every section, so polymeter layers (7, 13, 17 steps …) keep drifting through the whole form without rewriting them as absolute clips. Consecutive sections that use the same global part become one clip, so long notes sound across section lines, and `muser view` shows it as one continuous block.
- `muser view` draws parts that are used more than once as rounded blocks, the same shade and the part's name for the same part — like patterns in FL Studio — and its bar lines follow the real bar lengths (they were wrong for meters like 6/8).

### Hand-off and export

- **Export for a DAW**: `muser export-daw <project> -o <folder>` (MCP `export_daw`) writes everything Ableton Live, Cubase or Logic needs to carry on: a MIDI file of every instrument track (tempo, meter, section markers), 24-bit stems aligned at bar 1, the mix, and IMPORT.md, a guide with the steps for Live, Cubase, Logic, FL Studio, Studio One, Bitwig, REAPER, GarageBand and Pro Tools.
- **Stems**: `muser render <project> --stems <dir>` (MCP `export_stems`) writes one WAV per unmuted track — with its effects, fader, pan and what it sends to buses (reverb follows the track), sidechains intact, without the master effects — all the same length, for mixing on in a DAW.
- **`muser render -o song.mp3`** writes MP3 directly (320 kbit/s, with title and tempo tags) with manguebyte's encoder.
- **Section markers in WAV files**: `muser render` (and stems) write a cue point at the start of every section of the `form` (`verse`, `chorus`, `chorus#2`…), plus the title and tempo, so DAWs and players can jump by section.
- **Data for visuals**: `muser export-data <project> [-o data.json] [--fps 30]` (MCP `export_data`) writes one JSON with every track's notes in seconds (as rendered), the sections, the automation curves of tracks and master, and with `--fps` a per-frame level and low-band envelope — everything the *Electric Sheep* videos had to scrape by hand.

### Better feedback for agents

- `analyze --audio` warns about sections that are almost silent (average level below −45 dB, or no sound at all) and says what it can see in the project: no notes there, the master volume curve low there, the playing tracks' volume low there, or both stacked (the master curve and track fades together, as in the dreamer songs). Intentional silence can be ignored.

### Fixes

- Plugins that print their own log lines (for example Kontakt 8's `[…] [error] PresetSlotManager::selectSlot: slot not found …`, once per instance) no longer flood the terminal during `render`, `view` and the other commands. Lines that start with a log timestamp go to `muser-plugin.log` in the temp folder, with one note saying where; muser's own output is unchanged. Set `MUSER_PLUGIN_LOG=show` to see them again (macOS and Linux).
- `muser view` drew bar lines a numerator's worth of beats apart, which was wrong for meters like 6/8; the pending-changes panel kept showing "matches the project file" next to a list of changes.

## v0.3.1

### Upgrade notes

- **Built-in instrument levels are calibrated.** `gain_db 0` now means about −20 LUFS on a reference phrase for every built-in instrument, so instruments that used to be much louder or quieter than the rest now sit together. A project written for 0.3.0 may come out with a different balance; check it with `muser mix-report` and adjust `gain_db`. Custom `synth.poly` patches are not trimmed.

### New

- **12 new effects**: `bitcrusher`, `tremolo`, `auto_pan`, `ring_mod`, `frequency_shifter`, `widener`, `transient_shaper`, `multiband_compressor`, `tape` (saturation with wow / flutter), `vocoder` (modulated by another track through `sidechain`), `granular_delay`, `formant_filter`. All have ranges, automation and skill pages (`muser effects <name>`).
- **Tuning**: a project-level `tuning` for pitched built-in instruments and the SFZ sampler — just intonation, Pythagorean, meantone, Werckmeister III, Kirnberger III, Vallotti, Young, custom deviations, N-EDO, Scala `.scl` / `.kbm`, reference pitch, detune. Drum kits and plugins keep standard pitch. Samples: `tuning/just-chorale`, `tuning/maqam-rast`.
- **Plugin multi-out**: `@outputs` entries can take a plugin's auxiliary output bus (`aux`) and send a channel or bus straight to a muser bus (`bus`), so one plugin instance (e.g. a drum sampler) feeds several mixer buses. `muser routing` lists these taps.
- **Multichannel plugin instruments**: `@outputs` mixes multichannel outputs down to stereo; `@warmup` waits for plugins that load samples in the background (e.g. DrumGizmo).
- **Large SFZ libraries** (over 1 GB) stream samples from disk instead of loading everything into memory; the rendered output is identical.
- `muser sounds describe` reports a sound's volume against the built-ins.

### Better feedback for agents

- `validate` and `render` (CLI and MCP) list warnings without failing: ABC bars that don't match the meter, parts that cover only part of their section, notes far outside an instrument's range, drum notes on keys the kit doesn't have (they make no sound), and clipping on render.
- `analyze` adds an ungated `rms_db` per section (near-silent sections no longer hide behind LUFS gating), names the section, bars and a `form` mute snippet in its hints, and merges masking hints per track.
- A `grid` with `step` in the wrong place points to `grid.step`.

### Fixes

- `muser view`, `loop edit` and `poly edit` move to the next free port when the default one is taken.
- Updated the vendored manguebyte to 38eb44b.
