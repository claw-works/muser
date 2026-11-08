# Changelog

Each release's notes on GitHub are the section for its tag below (the release workflow copies it).
Write the next version's section here before tagging; the heading must be `## vX.Y.Z`.

## v0.6.0 (unreleased)

### Sounds

muser's own instruments live under **`yinse.`** (*yīnsè*, "tone colour"), named by how they sound: `yinse.<pluck|…>.<name>`. The built-in instruments that come with manguebyte keep their names.

- **Electric guitars**: `yinse.pluck.electric_guitar` (humbucker on the bridge pickup, made for distortion: long sustain, thick and biting) and `yinse.pluck.electric_guitar_clean` (single coil on the neck: clean, glassy, warm). Both have `pickup` (`"bridge"` / `"middle"` / `"neck"`, in-between values blend), `pickup_type` (`"single"` / `"humbucker"`), `mute` (palm-mute depth) and `strum` (ms between strings; chords of 3+ notes strum down on the beat and up off it). Through `distortion` with the guitar cabinet the held notes now sustain (about 2 dB/s instead of 6 dB/s with `guitar.steel`). Sample: `samples/instruments/electric-guitar`.
- **Chinese instruments**: `yinse.pluck.guzheng` (古筝: bright picked strings over a big soundboard, long ringing notes, the left-hand vibrato 揉弦 added to held notes: `vibrato`, `vibrato_rate`, `vibrato_delay`; a note's own `bend` is a pressed note 按音 and gets no vibrato) and `yinse.bow.erhu` (二胡: python-skin resonator, nasal and singing, wide vibrato; one note at a time; a note that overlaps the next slides into it under one bow, a small gap is a new bow). Sample: `samples/instruments/chinese-guzheng-erhu`.
- **Accordions**: `yinse.blow.accordion` (musette: three middle reeds tuned apart, the French shimmer), `yinse.blow.accordion_dry` (master register, nearly in tune: tango, folk) and `yinse.blow.bandoneon` (low + middle reeds an octave apart). Free reeds built from additive partials: reeds speak in 20-40 ms with a breath of air, the higher partials a little later. Parameters: `register` (`"master"` `"musette"` `"violin"` `"clarinet"` `"bassoon"` `"oboe"` `"bandoneon"` `"organ"`), `musette` (cents between the middle reeds), `bellows` (pressure: louder and brighter, automate it to phrase), `shake` / `shake_rate` (bellows shake), `cassotto` (tone chamber), `attack`, `release`. Samples: `samples/instruments/accordion-valse`, `accordion-tango`.
- **Palm mute per note**: `"mute": true` on a note (JSON, and `add_notes` over MCP) plays it palm-muted on `yinse.pluck.electric_guitar*` (other instruments ignore it), so a riff can mix muted chugs and open chords.

- **Serum 2 presets load directly**: `muser sounds presets` lists Serum 2's factory sounds and installed third-party packs (name, author, tags from the preset file, category from its folder), and `sounds state` turns one into plugin state without opening Serum. muser reads the preset files on this machine and converts them on first use; nothing from Xfer is shipped. Packs kept outside Serum's preset folder are found through `MUSER_SERUM_PATH`. Serum 1 `.fxp` presets and the oldest Serum 2 format are not read (open them in Serum 2 and save again).
- **Serum 2 FX effect chains load directly**: on the Serum 2 FX plugin, `sounds presets` lists the `.SerumFX` (one effect) and `.SerumFXRack` (a whole chain) files, and `sounds state` gives the entry to put in `effects`. The chain goes into Serum 2 FX's own initial state, which muser reads once by opening the plugin without a window and keeps in the sound library (again after a plugin update). The 9 factory chains still in the oldest format are not read.
- **Sound tools over MCP**: `list_sounds`, `list_plugin_presets`, `get_preset_state` and `describe_sound` do what `muser sounds list / presets / state / describe` do, so agents no longer need the shell to pick a plugin's sound.
- **Sound cards suggest a gain**: `sounds describe` / `describe_sound` give `suggested_gain_db`, the `gain_db` that makes a sound about as loud as the built-in instruments (cards already cached get it without listening again).

### Fixes

- **Stems with master automation**: `render --stems` failed validation when the project automated a master effect (the stems drop the master effects but kept their automation). The `--stems` help now also says the stems are 24-bit (the full mix is 16-bit).
- `sounds state` given a file path that is not in the list explains that sound names are the listed names (Serum 2 takes them from inside the preset, not from the file name).

## v0.5.0

The headline is **`muser live`**: an agent plays music through this machine's sound card and keeps writing it a section at a time while people listen and ask for changes. Around it: tempo and tuning changes per section, a distortion effect, an effect panel and fuller exports in `muser view`, and everything muser prints is now in English.

### Upgrade notes

- **muser speaks English now**: CLI output, `--help`, error and validation messages, `analyze` / `mix-report` reports, MCP tool descriptions and results, built-in loop and plugin profile descriptions, and the docs (`SKILLS.md`, `TOOLS.md`, `skills/`) are English only. The `muser view` and live pages keep their 中文 / EN switch and `README.zh-CN.md` stays. Agents answer in the user's language as before; scripts that matched the old Chinese messages need updating. Cached sound cards are re-probed once (their summaries were Chinese).
- **SFZ controller curves**: SFZ instruments that use the built-in curves (`tune_curveccN=1` and the like) now play as designed. Ixox Flute, for one, was 50 cents sharp before and is in tune now, so projects using such libraries sound different (better).
- **Relative `@state` paths** (plugin instruments and plugin effects) are relative to the project file, as audio files are. Before, they were relative to the directory muser was started in, so a project only rendered from its own folder.
- **Building from source needs Rust 1.89** (documented as 1.85 before): with the current `Cargo.lock`, uuid 1.29 needs 1.89 and darling 0.24 / rmcp 3.7 need 1.88. Thanks to @Stella-Mira for reporting it, and for adding `LICENSE-MIT` / `LICENSE-APACHE` (#1).

### Live performance (`muser live`)

- **A resident live session**: `muser live start <template> --detach` keeps playing through the sound card while an agent writes the music a section at a time (`send`), and never goes silent: when nothing new has arrived, the last section loops. Each section has its own bars, time signature, tempo and tuning; tracks, effects (`effects_add` / `effects_off`), automation, buses and the master can change mid-song without a gap, since the engine swaps in the grown project while keeping every instrument's and effect's state. `status` / `wait` (runway, autoplay time limit) let an agent play on its own for a while and stop on a loop-friendly phrase; the whole session is written to `--record`, every submission to `<name>.changes.jsonl`. Seven MCP tools (`live_start` … `live_stop`), three templates in `samples/live/`, and `skills/live.md`.
- **Changing what is playing**: `"at": "now"` switches at the next bar line; `"at": "patch"` rewrites the current section from the next bar while staying at the same place in its form (for when someone is singing along); `muser live mix <track|bus|master> <dB>` moves a fader within 0.3 s without a new section. `status` lists every fader with its loudness right now (`mix`), and warns about dropouts with the heaviest tracks (`load`) and slow moments in the playback thread (`slow`).
- **The live page**: the session title, what plays now and next with each section's one-line `note` (it flashes on a change), a timeline whose ruler stays on top while track rows scroll (tracks playing on screen first), levels, a fixed status bar, karaoke for sections with `"lyrics"`, a Stop button, and a 中文 / EN switch.
- **Big sampled sounds join without a stall**: a new `sfz:` sound is loaded in the request thread before its section is placed, so adding Salamander mid-set no longer stops the audio for 9 s (the reconcile takes 0.35 ms now; the `send` waits instead, and the section starts on time after it). Looking an `sfz:` instrument up no longer parses the whole file (~5 ms instead of ~250 ms per change), and a plugin's parameter check is remembered within the process (each change with Kontakt took ~1 s).
- **Steadier with plugins**: `live stop` exits without running C++ exit destructors once a plugin is loaded, so Kontakt no longer crashes on the way out ("muser quit unexpectedly"); silent tracks and effect chains rest; the live playback queue is ~340 ms.

### Composition

- **Tempo changes**: a section can carry `"tempo"` (the new BPM from its first beat on, kept by later sections) and `"tempo_ramp_bars"` to get there gradually (accelerando / ritardando, linear in time; 0 or omitted switches at the bar line). Clips and automation stay in beats and follow; plugins get the tempo map in their transport, MIDI export writes it (a ramp as one tempo per beat), ABC export writes inline `[Q:]`, and `muser view`, karaoke, takes and `live status` use it. Projects without tempo changes render bit-identically. Sample: `samples/songs/accelerando-dance`.
- **Tuning changes**: a section can carry its own `"tuning"` (written like the project's): from its first beat on, pitched built-in instruments and SFZ play in the new tuning (sounding notes keep their pitch), later sections keep it; `{ "scale": "equal" }` goes back. Sample: `samples/tuning/tuning-journey` (equal → just → maqam Rast → equal).
- **Effects can start and stop mid-song**: `"from"` / `"until"` (beats) on an effect: untouched before (empty state, as if plugged in then), cut after.

### Effects

- **Distortion** (`distortion`), modelled on a guitar amp: lows kept out of the clipper, a mid push and presence before two clipping stages, `guitar` / `bass` cabinets, `overdrive` / `distortion` / `fuzz` / `fold`, and loudness compensation (more drive means more grit, not more volume). Zero latency, so it can join a live set. Samples: `samples/effects/distortion`, `samples/songs/garage-drive`. (It still sounds best on a real electric guitar recording; the built-in `guitar.steel` is an acoustic model.)

### `muser view`

- **Effect panel**: clicking an effect opens a panel instead of toggling it: an On switch, sliders and choices for every built-in effect parameter (re-rendered as you drag, saved like the faders), and for plugin effects their own window (state saved into the project as you tweak; clicking Open again brings an open window to the front).
- **Exports**: MP3 (320 kbit/s), a DAW package (zip: MIDI, mix, one stem per track, an import guide) and visualization data (JSON), next to WAV, MIDI and ABC.
- **Recording**: takes are checkboxes, so several can be used at once and overlap (recording line by line); a fresh take still replaces one it mostly covers (a retake). Select an audio clip and press Delete to remove it, e.g. the unwanted piece after a cut. Measure latency comes first until it has been measured. Recording with `--device` uses this machine's sound card and microphone in one duplex stream, lined up by the driver's round-trip latency.
- **Navigation and layout**: clicking the timeline snaps the playhead to the nearest bar line (Shift: beat, Option: anywhere); the right panel is three tabs (Changes with a count, Export, Record); banners sit at the top, clear themselves when the connection is back, and a page error no longer shows as "connection lost".
- **Fixes**: page edits survive a file reload (a plugin window saving its state used to drop them); a page playing past what a restarted view had rendered crashed the server; dragging an audio clip recursed forever after the tempo-map change.

### Export and data

- **`export-data` for videos**: `bars` and `beats` (seconds) and `lyrics` (every line and syllable with start / end seconds), next to notes, sections, automation and the per-frame level envelope.
- **ABC export**: sections with their own time signature get bars of the right length and an inline `[M:7/8]` where the meter changes; tracks with lyrics get `w:` lines. The ABC reader understands inline `[M:]` too.
- **Plugins follow per-section meters** (time signature, bar start, bar number), and after a jump in `--device` playback they see the position they jumped to.

### SFZ

- **Built-in controller curves** 1–3 (bipolar, inverted, inverted bipolar) on `tune` / `volume` / `pan` / `amplitude` modulations are applied.
- SFZ files with stray lines (such as `A0 bankname: …`) load with a warning instead of failing.

### Samples and docs

- `samples/v0.4/moon-rising` is now a full ~3-minute song (second verse, two-line chorus, bridge, final chorus, strummed ending, let-ringing fingerpicked guitar, 13 karaoke lines). New: `accelerando-dance`, `tuning-journey`, `garage-drive`, `effects/distortion`; `sfz-bass-black-and-blue` points at the library's install folder.
- `skills/live.md`: changing tempo and tuning, drum clips that span the section, not queueing far ahead, re-checking the mix, saying so when unsure of a well-known melody. README instrument counts are approximate (they follow manguebyte).

### Engine and platforms

- **manguebyte follows upstream 66ef3dc**: partial voicing (air, attacks, wind jitter) for `PartialBank`, an organ's space (windchest, early reflections, swell box), plugin tail reporting, render pool and MIDI port fixes. Renders of the built-in samples are bit-identical.
- **Windows**: the CLI runs in a 16 MB-stack thread (debug builds overflowed the 1 MB main stack).

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
