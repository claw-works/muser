# muser Skill: how to compose with muser

> An operating manual for **AI agents**: translate the user's musical intent into a muser project JSON, then render it to audio.
> This is the main entry point; the details of each instrument family and each effect are in the sub-skills under [`skills/`](skills/) - consult them as needed.

## Your role

The user describes the music they want in natural language ("make a 90bpm lo-fi beat, piano + drums"). Your job:

1. Pick instruments, write notes, and set up effects, producing a **Project JSON**.
2. `validate` first, then render to WAV.
3. **Modify the JSON incrementally** based on feedback, and render again.

You work only at the Project JSON level and do not need to understand the underlying DSP.

When the user asks you to install or update muser: if it is not installed, run the one-line install script (macOS / Linux:
`curl -fsSL https://raw.githubusercontent.com/claw-works/muser/main/scripts/install.sh | sh`;
Windows: `irm https://raw.githubusercontent.com/claw-works/muser/main/scripts/install.ps1 | iex`); if it is installed, run `muser self-update`
(`--check` only checks without installing). After installing, relay in plain language the few lines the script prints at the end (the version, where the command is, whether to `export PATH` first).
After an update, projects written with an older version may sound different (for example, 0.3.1 calibrated the built-in instrument volumes): ask which version the project was written for,
run `muser migrate <project> --from 0.3.0` to see what would change, then add `--write` to write it back.

## Listen first, write later: the workflow for creating with a person

Making music is subjective: **play a draft for the person first, then write the whole piece**. Do not start by arranging a complete song.

1. **Make a draft**: after the user describes an idea, first write a 4-8 bar draft, preferably 2-3 versions at once (A / B / C: the same melody with different rhythms,
   different loops, different tempos or sounds). Use `abc` for the melody, a loop / `drummer` for the drums, and `humanize` for the groove (see [skills/notation.md](skills/notation.md)),
   a few lines of JSON per version; do not write scripts to generate notes.
2. **One sentence + one command**: for each version, say in one sentence what it feels like, and give the audition command `muser play sketch-a.muser.json`
   (plays it once on the sound card and exits). To audition a drum pattern alone, use `muser loop play <name>`.
3. **Ask specific questions**, not "what do you think?": faster or slower? Is the melody more bouncy or flatter? Busier or sparser drums? Brighter or darker?
   Should the instrument change? Which version is closest?
4. **Revise the draft per the feedback**, then listen again; for another take of the same version: `muser play sketch-a.muser.json --seed 2`.
5. **Expand once they are happy**: arrange the whole piece on top of the settled draft - write with the arrangement layer (`sections` + `form` + each track's `parts`, with chords written on the sections;
   bass, harmony, and arpeggios follow the chords with `chordal`, see [skills/arrange.md](skills/arrange.md)), and do not hand-calculate each clip's start.
   When done, self-check with `muser analyze` (does the energy curve rise and fall, does the chorus lift, repetition, masking, clashes, the "AI tells" checks) and revise per the suggestions; finally balance volumes with `mix-report`, and hand it to a person with `muser view` for fine-tuning.

A person can supply the motif first: `muser loop edit --open` opens the editor, where the person draws a few bars on the piano roll or hums a phrase into the microphone (automatically converted to notes) and saves it to the project's `loops/`; you read it out and continue / expand it into several versions
(saved as new loops that can be auditioned and modified directly in the editor); see "A person supplies the motif, the agent continues it" in [skills/notation.md](skills/notation.md).

Enter the draft workflow (do not edit the whole piece directly) when the user says things like: "let me hear first", "give me a few versions", "hum something", "it doesn't feel right", "make this part livelier", "change the feel".

## Look it up before writing: discoverability

Do not write instrument names and effect parameters from memory; look them up first:

| To find out | CLI | MCP tool |
|---|---|---|
| Which instruments exist | `muser instruments [--family F]` | `list_instruments` |
| All sounds usable on this machine (built-in, SFZ, instrument plugins) | `muser sounds list [--kind builtin\|sfz\|plugin] [--filter word]` | - |
| What a sound is like (brightness, attack, sustain, range, velocity, stereo, pitch) | `muser sounds describe <sound> [--params JSON]` | - |
| An instrument's pitch convention, range, polyphony, **tunable parameters and the preset's current values** | `muser instruments <name>` | `describe_instrument` |
| Which effects exist | `muser effects` | `list_effects` |
| An effect's parameters, ranges, defaults, examples | `muser effects <name>` | `describe_effect` |
| Whether the project is valid, and where it is wrong | `muser validate <file>` | `validate_project` |
| Which external plugins are on this machine | `muser plugin scan` | `scan_plugins` |
| A plugin's parameters and option text | `muser plugin params <plugin> --filter <word>`, `muser plugin values <plugin> <param>` | `list_plugin_params`, `plugin_param_values` |
| What a parameter actually does | `muser audition ... --param <name> --values ...` | `audition` |
| Whether track volumes are balanced | `muser mix-report <file>` | `mix_report` |
| How the signal flows: where each track / bus goes, send amounts, sidechain, processing order | `muser routing <file>` | `get_routing` |
| Whether the arrangement is good: density, register, chords, repetition, clashes, and suggestions | `muser analyze <file> [--audio]` | `analyze` |
| Which ready-made drum patterns / motifs (loops) exist, and put one into the project | `muser loop list [--style S]`, `muser loop show <name> [--abc]` | - |
| Let a person draw a motif / drum pattern, or hum into the microphone (editor, saved to the project) | `muser loop edit --open` | - |
| Convert a humming / whistling recording into notes | `muser loop hum <audio> --tempo T --name N` | - |
| The poly sounds in this machine's sound library | `muser poly list`, `muser poly show <name>` | - |
| An audio file's format and duration (converted to beats) | `muser audio <file> [--tempo T]` | - |
| Hand the arrangement to a DAW / analysis tool | `muser midi <project> [-o x.mid]` | `export_midi` |
| Change the key of the whole piece (for another singer, to find a suitable range) | Write `"transpose": N` in the project (semitones; notes unchanged, drums and audio tracks do not move); the view playback bar has "Transpose − / +" | - |
| The user wants to sing / record vocals themselves (workflow in [skills/recording.md](skills/recording.md)) | `muser view <project> --open`, the "Record vocals" panel on the right (the project's `lyrics` are shown when written); after recording, `muser takes <project>` shows each take, and you edit and align by changing the audio clip's `offset` / `start` / `length` / `fade_in` / `fade_out` (the user can also split with ✂ in view, drag to move, and nudge with ← →) | - |
| Hand off to a DAW (Live / Cubase / Logic) to continue: MIDI + stems + mix + import notes | `muser export-daw <project> -o <folder>` | `export_daw` |
| Stems only (one WAV per track) | `muser render <project> --stems <dir>` | `export_stems` |
| Score visuals / make a visualization (notes, sections, automation, volume envelope) | `muser export-data <project> -o data.json [--fps 30]` | `export_data` |
| Hand the score to a model that reads ABC (such as YuE2) | `muser abc <project> [--yue2 --vocal melody-track --sections verse:8,chorus:8]` | `export_abc` |
| Let a person quickly hear a draft | `muser play <file> [--seed N]`, `muser loop play <name>` | - |
| Live session: write one section after another while muser keeps playing on this machine's sound card (workflow in [skills/live.md](skills/live.md)) | `muser live start <template> --record s.muser.json --open`, then `muser live send / status / wait / autoplay / stop` | `live_start` `live_send` `live_status` `live_wait` `live_autoplay` `live_stop` |
| Whether a plugin already has usage experience (aliases, presets) | `muser plugin profile show <plugin> --markdown` | `get_profile` / `list_profiles` |

Add `--json` for machine-readable output. `validate` points to the exact location of out-of-range parameters and unknown instruments, for example
`tracks[0].effects[0]: reverb.mix = 1.8 out of range [0, 1]`.
`validate` and `render` also list with `⚠` things that are legal but probably not what was intended (the exit code is unchanged): ABC bar durations that do not match the time signature,
section content that fills only a short stretch with no `length` written (so it goes silent afterward), notes far outside the instrument's recommended range, and master clipping after `render`. **When you see a ⚠, confirm before moving on.**

## Core mental model

- **Project**: one `tempo` + several **Tracks** + optional `master_effects`.
- **Track**: one `instrument` + optional `instrument_params` (shape the sound) + optional `effects` + optional `automation` (change over time) + several **Clips**.
- **Clip**: has a `start` and `length` on the timeline, and contains several **Notes**.
- **The time unit is beats**: `1.0` = one quarter note, independent of tempo.
- A clip's `start` is counted from the **start of the project**; a note's `start` is counted from the **start of its clip**.

## Time cheat sheet (4/4)

| Note | Whole | Half | Quarter | Eighth | Sixteenth | One bar |
|---|---|---|---|---|---|---|
| beats | 4.0 | 2.0 | 1.0 | 0.5 | 0.25 | 4.0 |

## Pitch cheat sheet

- Melodic instruments use **MIDI pitch**: `60` = middle C (C4), +12 is an octave up, +1 is a semitone up.
  `C4=60 D4=62 E4=64 F4=65 G4=67 A4=69 B4=71 C5=72`; bass commonly uses `C2=36`, `E1=28`.
- Drum kits (`drums.*`) use **GM drum keys**: 36=kick, 38=snare, 39=clap, 42=closed hat, 46=open hat, 49=crash, 51=ride.
  For the full key map and the keys each kit supports, see [skills/instruments/drums.md](skills/instruments/drums.md).
- `velocity` is 0-1, default 0.8.
- `accent` is true/false, default false. For drum kits and `bass.acid*` an accent is louder and brighter (acid's 303 character mostly comes from it); other instruments ignore it.
- `bend` (pitch bend, guitar string bends, etc.) is `[[beat, semitones], ...]`; see "Pitch bend: bend" below.
- Chinese instruments take their playing techniques as note fields: `"technique": "open"` / `"pressed"` / `"harmonic"` with `"harmonic": 2` (guqin, guzheng; `pitch` is the sounding pitch), `"damp": true` (止音), `"flutter": true` (dizi 花舌), `"trill": true` (颤音), `"half_trill": true` (虚颤音), `"tremolo": true`. See [skills/instruments/chinese.md](skills/instruments/chinese.md) and the page of the instrument you use.

## Instrument index (built-in + SFZ sound libraries + external VSTi)

| Family | Sub-skill | Representatives |
|---|---|---|
| Drum kits | [drums.md](skills/instruments/drums.md) | `drums.808` `drums.909` `drums.acoustic` `drums.trap` `drums.lofi` ... 10 kits, and the Chinese percussion `yinse.strike.luogu` (锣鼓) |
| Synth presets | [poly.md](skills/instruments/poly.md) | 68: `poly.moog_bass` `poly.trance_supersaw` `poly.warm_analog_pad` `poly.juno_keys` ...; the full sound `patch` and the machine's sound library (`muser poly list / show`) |
| General synths | [synth.md](skills/instruments/synth.md) | `synth.poly` `synth.fm` |
| Guitar | [guitar.md](skills/instruments/guitar.md) | `guitar.nylon` `guitar.steel` `yinse.pluck.electric_guitar` `yinse.pluck.electric_guitar_clean` |
| Singing voice (yinse) | [voice.md](skills/instruments/voice.md) | `yinse.voice.soprano` `yinse.voice.alto` `yinse.voice.tenor` `yinse.voice.bass` `yinse.voice.child` |
| Chinese traditional | [chinese.md](skills/instruments/chinese.md) (overview), one page each in [chinese/](skills/instruments/chinese/) | `yinse.pluck.guzheng` `yinse.pluck.pipa` `yinse.pluck.guqin` `yinse.strike.yangqin` `yinse.bow.erhu` `yinse.blow.bangdi` `yinse.blow.qudi` `yinse.blow.xiao` `yinse.blow.hulusi` `yinse.blow.suona` `yinse.blow.suona_big` `yinse.blow.haidi` |
| Accordion | [accordion.md](skills/instruments/accordion.md) | `yinse.blow.accordion` `yinse.blow.accordion_dry` `yinse.blow.bandoneon` |
| Bass | [bass.md](skills/instruments/bass.md) | `bass.finger` `bass.slap` `bass.acid` |
| Organ | [organ.md](skills/instruments/organ.md) | `organ.jazz` `organ.gospel` `organ.full` `organ.ballad` |
| Mallet percussion | [mallet.md](skills/instruments/mallet.md) | `mallet.marimba` `mallet.vibraphone` `mallet.kalimba` ... |
| Strings | [strings.md](skills/instruments/strings.md) | `strings.violin` `strings.cello` `strings.ensemble` `*_pizz` ... |
| Winds | [wind.md](skills/instruments/wind.md) | `wind.flute` `wind.sax` `wind.trumpet` ... |
| Voice | [choir.md](skills/instruments/choir.md) | `choir.aah` `choir.ooh` `choir.solo` |
| Audio files (recordings, samples, loops) | [audio.md](skills/audio.md) | `"instrument": "audio"`, with the clip written as `"audio": { "file": "vocal.wav" }` |
| Helping the user record vocals (how to prepare, record, check timing, mix, add atmosphere, finish, deliver) | [recording.md](skills/recording.md) | `muser view`'s record-vocals panel, `muser takes`, the project's `lyrics` |
| Live session (starting, how to write a section, the autoplay loop, tempo changes with unchanged BPM, finishing) | [live.md](skills/live.md) | `muser live`, templates in `samples/live/` |
| Your own instrument from recordings (struck or plucked) | [yinse-extract.md](skills/instruments/yinse-extract.md) | `muser yinse extract <wav files> -o x.yinse.json`, then `yinse:x.yinse.json` |
| SFZ sound libraries | [sfz.md](skills/instruments/sfz.md) | `sfz:/path/to/Piano.sfz` or a path relative to `MUSER_SFZ_PATH` (built-in sampler, no plugin needed) |
| External VSTi | [plugin.md](skills/instruments/plugin.md) | `plugin:/usr/lib/vst3/Surge XT.vst3` |

All built-in sound sources are synthesized in real time by manguebyte (physical modeling, modal synthesis, FM, subtractive synthesis, etc.) and do not depend on the system MIDI synth.

## Choosing sounds: "listen" first, then record it as the user's own skill

You cannot hear sound, but muser can listen for you: `muser sounds describe` renders a set of probe notes (pitched instruments play one note each from C2 to C6, then a light tap;
drum kits hit kick, snare, and cymbals), writes the measurements as a text card, and caches it on the user's machine (when the sound, parameters, and files are unchanged it reads the cache; `--refresh` listens again).

```bash
muser sounds list --filter "piano"                 # candidates: built-in, SFZ in $MUSER_SFZ_PATH, instrument plugins; ♪ means already listened to
muser sounds describe poly.wurly                   # very dark, soft attack, slow decay, stops when released; range about C2-C6 + item-by-item data
muser sounds describe "sfz:Salamander…/Salamander….sfz"
muser sounds describe "plugin:<path>#<id>" --params '{"@state": "<state-file>"}'   # plugin: listen with this state
```

1. **List 2-5 candidates per the requirement**, listen to each, and pick by comparing the cards:
   | Want | Look at on the card |
   |---|---|
   | Lyrical, warm chords / pads | brightness "dark, warm", attack "soft" or "slow", sustain "sustained" or "slow decay" |
   | A penetrating lead melody | brightness "bright", attack "fast", range covering the register the melody sits in |
   | Percussive, rhythmic accompaniment | attack "instant", "fast decay" |
   | Bass line | sounds at C2 in the range, brightness not too bright |
   | A solo that conveys dynamics | velocity "highly expressive", harder notes clearly brighter |
   | To be summed to mono / layered with other wide parts | stereo not "side signal stronger than mid" |
   The card comes from measurement, not true listening: after choosing, put it in the piece and render, check the result with `muser analyze --audio`, and finally ask a person to listen.
2. **A plugin's sound is determined by its state**, so do not treat a plugin as having only one sound. First see what sounds the plugin ships with:
   ```bash
   muser sounds presets "<plugin>" --filter "pad evolving"   # name, sound library, category, tags; [auto] [captured] [needs loading]
   muser sounds state "<plugin>" "<sound-name>"                # [auto] / [captured]: gives the @state file and how to write it in the project
   ```
   - **[auto]**: VST3 standard presets (`.vstpreset`), Surge's `.fxp`, Serum 2's `.SerumPreset` (factory sounds and installed sound packs), and the like can be converted straight into plugin state, and `sounds state` works on them.
     Over MCP the same steps are `list_plugin_presets` → `get_preset_state` → `describe_sound`.
   - **Serum 2 FX effect chains**: `sounds presets` on the Serum 2 FX plugin (its `#<id>`, since Serum 2 and Serum 2 FX live in one file) lists the factory and pack effect chains;
     `sounds state` gives an entry for `effects` (`"type": "plugin"` with `path` and `id` written separately). The first one opens Serum 2 FX once, without a window, to read its initial state.
   - **Serum 2 sound packs**: muser reads Serum 2's preset folder; packs kept somewhere else are found when that folder is on `MUSER_SERUM_PATH`.
     Only Serum 2 presets work: Serum 1 `.fxp` packs must be opened in Serum 2 and saved again. A pack that comes with its own wavetables or samples
     needs them installed where its instructions say: without them Serum gives no error, and the sound is wrong or silent (a missing main wavetable makes the patch silent).
     When a pack sound renders silent, ask the person to check the pack's install steps; tell them this in plain words.
   - Two sounds with the same name (a pack's and a factory one, say) make `sounds state` / `get_preset_state` ask for the file path instead; pass the `file` from the list.
   - **[needs loading]**: Kontakt instruments (`.nki`) can only be loaded in its own UI. After loading once, the state is stored in the user's machine-local sound library
     (`~/.config/muser/sound-library/`) and becomes [captured], used directly just like the automatic ones. For how to load, see "Loading with computer use" below;
     without computer use, ask the person to load it from `muser view`'s 🎛 -> plugin -> click this sound, or in `muser sounds capture`.
   - **My sounds**: a plugin sound that the user tweaks in `muser view`'s 🎛 and gives a name is also saved to the sound library on completion,
     appearing at the very top of `sounds presets` (category "My sounds"), and usable directly in another project.
   - To handle a state of unknown origin: `muser sounds identify "<plugin>" <state-file>` recognizes which sound is loaded in it
     (it compares the sound file's raw bytes; it can recognize sound libraries and instruments, but cannot easily tell apart several variants of the same instrument).
   - Once you have a state you can listen with `muser sounds describe "<plugin>" --params '{"@state": "<file>"}'`, then adjust the parameters the plugin exposes
     (`muser plugin params "<plugin>" --params '{"@state": …}'`, such as a Kontakt instrument's knobs).

   **Loading with computer use ([needs loading] sounds, macOS)** - do this only if you have computer use (can take screenshots, click the mouse, and type):
   1. Run `muser sounds capture "<plugin>" "<sound-name-or-file>"` in the background (it waits until the window closes). It opens the plugin window, with the title
      "Load: <sound> (<library>) - close the window after loading · muser".
   2. **At every step, take a screenshot and look carefully before clicking**; do not chain operations with fixed coordinates: the plugin's layout may differ each time it opens (Kontakt is sometimes a large browser,
      sometimes a left column), and a misplaced click will load a different sound from the list.
   3. Load with the plugin's own browser: in Kontakt click the Search box at the top, type the sound name, take a screenshot to confirm only this one remains in the results, and double-click it.
   4. Wait for it to finish loading, **take a screenshot to verify that the instrument name shown in the rack is the one you want**, then close the window (the red button at the top left). In the state of an encrypted sound library (such as Hypha),
      muser cannot tell which sound is loaded and cannot check for you; only a screenshot can confirm. For an unencrypted one (such as Piano Uno) muser will recognize it and correct the name.
   5. The command listens once first: only if there is sound does it print "Captured …" and a line on how it sounds; with no sound (closed before loading finished) it reports an error and does not record it in the sound library, so try again.
   6. Load only one at a time. For dialogs such as licensing, activation, "sound library not found", or update prompts, do not dismiss them yourself - stop and tell the user.
      Do not change the plugin's settings and do not delete anything.
   When capturing in bulk, do them one at a time and confirm each does not sound like "no sound"; when done, record the captured sounds and how they sound in the user's own skill (the next item).
   **Loop kits (Kontakt's Leap kit and the like): holding one key is one loop that follows the project tempo**. Audio tracks do no time stretching,
   so to use a ready-made drum loop at a different tempo, use this kind of kit: the plugin stretches to the project's tempo, a note plays for as long as it is held, and a start aligned to the bar line is aligned to the beat.
   - Loading: in Kontakt's **Instruments**, search for the kit name (such as "Alter Kit") and double-click - the whole set of loops is spread across different keys. Do not
     double-click a single loop in the Loops tab: that gives only a Leap custom kit of one loop. After loading, use `muser sounds save "<plugin>" "<name>" <state-file>`
     to store it in "My sounds" (a state saved with `muser plugin edit … -o <file>`, or saved automatically when "load new sound" finishes in view).
   - Mapping the keys: `muser sounds describe "<plugin>" --params '{"@state": "<file>"}' --keys --from 57 --to 89 --tempo <project-tempo>`
     holds each key for 4 bars and lists which keys sound, how many bars per cycle, onsets per bar, loudness, and brightness. Example: Acoustic Drums' Alter Kit is
     16 white keys from C4 to D6 (Kontakt labels them C3-D5), where F4, A4, and B4 are 2-bar loops and the rest are 1 bar; the higher the key, the denser and louder.
   - Another kind is the **stem kit** (Lo-Fi Vibes' Cozy Kit and the like): one song split into 16 stems, one per white key - in Cozy Kit C4-G4 are
     kick, snare, hi-hat, open hat, and brushes; from A4 are bass, chords, guitar, keys, flute, strings, glockenspiel, sax, and vocals; D6 is vinyl noise;
     holding several keys at once plays that many stems together (the stems are the same song, so key and tempo match); to adjust their volumes separately, put them on several tracks that use the same state.
   - The "bars per cycle" on the key map card is the period at which the sound repeats: in some loops the second half equals the first, so it will report shorter than the loop file (a 4-bar loop reported as 2);
     notes written to this length still stay aligned to the beat. A 4-bar phrase needs `--hold 8` to be measured.
   - How to write: one note is one loop, and `length` is the number of beats to play, best as an integer multiple of the loop length (for a 1-bar loop write 4, 8, 16 beats).
     To change loops, change the pitch: use sparse keys in the verse and dense ones in the chorus, and in the last bar of every 4 / 8 bars switch to another key as a fill.
   - When layering over other drums, add an `eq` highpass (a few hundred Hz) to keep only the cymbals and the air of the snare, then connect it to the kick's sidechain so they "breathe" together.

3. **After listening, record the conclusions as the user's own skill** so next time you can use them directly without listening again. Write it in your own (the agent's) skill / memory directory, for example Claude Code's
   `~/.claude/skills/my-sounds/SKILL.md`; **do not** write it into the muser repo or the project directory - it holds the user's machine-local sound library, plugins, and paths. If one already exists, update it rather than making another:

   ```markdown
   ---
   name: my-sounds
   description: Sounds listened to with muser on this machine (built-in, SFZ, plugin states) and what they suit. Check here before choosing a sound.
   ---
   | Sound (instrument) | Parameters / state | Character (muser sounds describe) | Suits | Notes |
   |---|---|---|---|---|
   | `sfz:Salamander…/Salamander….sfz` | - | dark, fast attack, fast decay, wide stereo; C2-C6 | classical piano, lyrical accompaniment | loads in about 10 seconds |
   | `plugin:…/Kontakt 8.vst3#…` | `@state: ~/…/song.piano.muserstate` (`@label` Piano Uno) | dark, instant attack, very wide | pop piano | chosen by a person in 🎛 |
   ```
   Record only descriptions and uses; do not copy the contents of samples or plugin states. A skill like this that describes generic sound sources (a free SFZ library, a plugin's factory presets) can be shared with others later.

## Legato: legato

Write `"legato": true` on a clip: each note is automatically extended to overlap the next note of a **different pitch** by about 40 milliseconds, and the instrument plays legato -
SFZ sounds' legato samples (`trigger=legato`, such as Sonatina's `* Legato.sfz`) switch to the attackless transition sample while the previous note is still held,
and monophonic synths (plugins with `Polyphony 1` + `Legato On`, acid bass) glide across.

- One phrase per clip: connected within a phrase, broken between phrases (a gap over 1/4 beat counts as a rest and is not connected; a repeated same pitch is re-articulated).
- Use it when strings, woodwinds, and brass sing a melody; do not use it for chords, accompaniment, or staccato. The first note of a legato sound has a slower attack, so the start of a phrase can be slightly early.
- To avoid connecting everything, do not write `legato`; instead overlap the two notes you want connected by hand (the previous note ends later than the next one starts).
- MCP: `add_clip` can carry `legato`.

## Pitch bend: bend (guitar string bends, slides into notes, falling tails)

A note's `bend` is a `[[beat, semitones], ...]` polyline: beats are relative to the note start and increasing, semitones within ±2 (the standard pitch wheel range), or ±24 for pressed notes on `yinse.pluck.guqin`;
before the first point the first point's value applies, and between points it moves in straight lines. Each note with `bend` bends only itself; other notes sounding at the same time are unaffected.

```json
{ "pitch": 74, "start": 1, "length": 1.5, "bend": [[0, 0], [0.3, 2]] }
```

- Full-tone bend: `[[0, 0], [0.3, 2]]`; semitone bend: `[[0, 0], [0.15, 1]]`. The faster the bend, the more it "snaps"; 0.2-0.4 beats is a common feel.
- Pre-bend and release: `[[0, 2], [0.6, 2], [1.1, 0]]` (bend first, then pluck, slowly releasing back to the original pitch).
- Bend vibrato: after reaching pitch, oscillate between 1.7 and 2, such as `[[0, 0], [0.3, 2], [0.6, 1.7], [0.9, 2], [1.2, 1.7]]`.
- Slide into a note (a scoop for voice or sax): `[[0, -1], [0.15, 0]]`; falling tail (fall): `[[0, 0], [0.7, 0], [1, -2]]`.
- Supported: built-in instruments (guitar, synths, orchestral, etc.), the SFZ sampler (per-region `bend_up` / `bend_down`, default ±2 semitones), and plugin instruments (sends pitch wheel; the range is up to the plugin, mostly ±2). Drum kits ignore it.
- MIDI export writes pitch wheel events (one channel per track, so the export carries along other notes sounding at the same time).
- MCP: notes in `add_notes` can carry `bend`.

## Tuning: tuning (just intonation, historical temperaments, microtones, Scala)

Write `tuning` on the project, and pitched built-in instruments and the SFZ sampler sound according to it; drum kits keep standard pitch, and plugin instruments are unaffected (they use the plugin's own tuning settings).

```json
"tuning": { "scale": "werckmeister_iii" }
"tuning": { "scale": "just", "key": "D" }
"tuning": { "scale": "custom", "key": "C", "deviations": [0, 0, 0, 0, -50, 0, 0, 0, 0, 0, 0, -50] }
"tuning": { "scale": "equal", "divisions": 24 }
"tuning": { "scale": "scala", "scala": "slendro.scl" }
"tuning": { "reference_hz": 415 }
```

- Twelve-tone temperaments: notes are written as usual (ABC, `pitch`, and chords are all unchanged), only each pitch is a little lower or higher:
  `just` (pure fifths, pure 5/4 major third; `key` is the tonic, and it goes out of tune after modulating), `pythagorean`, `meantone` (quarter-comma meantone, Renaissance and early Baroque),
  `werckmeister_iii` / `kirnberger_iii` / `vallotti` / `young` (Bach-era good temperaments: every key is playable, each key with its own color).
- `custom`: `deviations` is 12 numbers, the cents of deviation of each semitone starting from `key`. Arabic maqam Rast: E and B each -50 (three-quarter tones); Persian and Turkish music likewise.
- `equal` + `divisions`: equal temperament with microtones, **one step per MIDI key** (in 24-EDO adjacent keys differ by a quarter tone), so do not use ABC / chords here - write `pitch` directly.
- `scala`: a `.scl` file (relative to the directory of the project file), optionally with a `kbm` key mapping; without a `kbm`, scale degree 0 is at middle C (60), one step per key. There are plenty of ready-made gamelan and experimental-tuning files (the Scala archive).
- `reference_hz` (default 440, 415 is common for Baroque), `reference_key` (default 69 = A4), and `detune_cents` shift everything.
- MIDI / ABC export writes keys, without the tuning.
- **Changing tuning mid-song**: a section in `sections` (or a live section) can carry its own `"tuning"`: from that section on the new tuning applies (sustained notes keep their pitch), later sections keep it until another one writes a tuning; `{ "scale": "equal" }` returns to 12-TET. E.g. a jazz chorus in equal temperament, then a raga section in `custom` shruti deviations, then back.

## Sustain pedal: pedal

A clip's `pedal` is the ranges `[[start, end], ...]` during which the sustain pedal (CC64) is down (in beats, relative to the clip start, increasing, non-overlapping). Keys released while it is down keep sounding,
and they are released together when it comes up. Built-in synths, the SFZ sampler, and plugin instruments all support it; drum kits ignore it.

```json
{ "start": 0, "length": 8, "pedal": [[0.1, 4.03], [4.1, 8.0]], "notes": [ ... ] }
```

- Piano writing: write notes at their actual written durations (the fingers release) and leave the sustain to the pedal; do not lengthen every note to imitate the pedal.
- **Change the pedal on every chord change**, using "syncopated pedaling": press down after the first note of the new chord is played (about 0.1 beat later), and the previous range continues until slightly past that moment before lifting,
  so the new notes do not blend into the old chord and nothing is cut off. Without changing the pedal, different chords blur together.
- MCP: `add_clip` can carry `pedal`, or set it with `set_pedal`.
- Full syntax, how to pedal, and examples: [skills/pedal.md](skills/pedal.md).

### Humanize: humanize

Write `"humanize": { "timing_ms": 6, "drift": 0.6, "velocity": 0.08, "swing": 58 }` on a track or clip, and timing, velocity, and duration are offset like a real player's,
with swing and an overall push-ahead / lay-back added. The same seed renders identically every time; change the project `seed` (or `muser render --seed N`) for another take.
- Fields, what the quantization means, and the starting points for each style: [skills/humanize.md](skills/humanize.md).

## Writing a whole piece: sections, form, chords (the arrangement layer)

```json
"sections": { "verse": { "bars": 8, "chords": "Am7 Fmaj7 C G" }, "chorus": { "bars": 8, "chords": "F G | Em Am" } },
"form": ["verse", "chorus", "verse", { "section": "chorus", "transpose": 2 }],
"tracks": [{ "name": "bass", "instrument": "bass.finger", "parts": {
    "verse": { "chordal": "1 . . 1 . 5 8 .", "octave": 2 }, "chorus": { "chordal": "1 1 5 5 8 8 5 5", "octave": 2 } } }]
```

- `sections` defines the sections (bar count, chord progression), and `form` orders them by name (can carry `transpose` / `mute` / `seed`);
  each track's `parts` writes what it plays in each section (`"verse#2"` only the 2nd time, `"*"` the remaining sections), and `end` replaces the last few beats of a section (a fill).
- `chordal` picks notes from the current chord: `1 3 5 7` chord tones, `8` octave, `c` the whole chord (auto-inverted), `B` bass, `.` rest, `-` sustain. To change the chords, change only `chords`.
- `transform`: `transpose` `octave` `reverse` `invert` `stretch` `shift` `sequence` (sequencing) `fragment` (trimming) `every`+`then` `vary` (seeded random variation).
- When done, check the suggestions with `muser analyze`. Full syntax: [skills/arrange.md](skills/arrange.md).

## Placing audio directly: audio tracks

Vocals, recordings, ready-made loops, and sound effects go straight onto a track: write `"instrument": "audio"` on the track, and write `audio` instead of notes on the clip.

```json
{ "name": "vocal", "instrument": "audio", "clips": [{ "start": 8, "length": 16, "audio": { "file": "vocal.wav", "offset": 1.5, "fade_out": 0.5 } }] }
```

- `file` is relative to the directory of the project file; `offset` and the fades are in seconds; the clip's `start` / `length` are still beats.
- It plays at its original speed, with no tempo stretching and no pitch change. First run `muser audio <file> --tempo T` to see the duration (in beats) and then write `length`.
- After that, effects, faders, buses, sends, sidechain, and `mix-report` work as usual. Fields and formats: [skills/audio.md](skills/audio.md).
- **Adding effects to a recording** (radio, telephone, underwater, tunnel, distant, loudspeaker, robot voice...): just chain effects on the audio track; recipes are in [skills/audio.md](skills/audio.md);
  to apply one to only a few lines, put them on a separate audio track. With the Airwindows plugin installed there are also hundreds more, such as distortion, lo-fi, and pitch shifting.

## Shaping sounds: instrument_params

Every built-in instrument exposes a set of common parameters, which **override** the sound of the chosen instrument/preset; write only the items you want to change:

```json
{ "instrument": "poly.moog_bass", "instrument_params": { "cutoff": 900, "resonance": 0.5, "release": 0.15 } }
{ "instrument": "strings.cello",  "instrument_params": { "attack": 0.4, "vibrato_depth": 25 } }
{ "instrument": "organ.jazz",     "instrument_params": { "drawbars": "838000000", "percussion": "second" } }
{ "instrument": "drums.808",      "instrument_params": { "kick": { "tune": -3, "decay": 1.8 }, "snare": { "snap": 0.8 } } }
```

- Parameter names are uniform across instruments: `attack` / `decay` / `sustain` / `release` (seconds), `tone` (Hz, the final brightness),
  `cutoff` / `resonance` (filter), `vibrato_rate` (Hz) / `vibrato_depth` (cents). Real units are used, not 0-1.
- Which parameters each instrument has, their ranges, and the **preset's current values**: `muser instruments <name>` (MCP `describe_instrument`).
  Looking at the current values before changing them is steadier than writing numbers from thin air.
- Drum kits are set per drum piece, with keys as GM drum key numbers or aliases (kick / snare / clap / closed_hat / open_hat / crash / ride ...).
- Misspelled parameter names, out-of-range values, and wrong types are all pointed out by `validate`, for example
  `instrument_params invalid: cutoff: 99999 out of range [30, 8000] Hz`.
- Parameter tables and tuning examples for each family are in the matching instrument sub-skill.
- External plugins (`plugin:` instruments, `plugin` effects) likewise use `instrument_params` / `params`, with the keys being the plugin's parameter names
  and the values being the text the plugin displays (such as `"2000 Hz"`, `"LP 24 dB"`). For an unfamiliar plugin, follow the flow in [skills/plugins.md](skills/plugins.md):
  scan -> info -> search parameters -> view options -> audition -> write into the project -> save as a profile.
- A plugin with a profile (`muser plugin profile list`) can use short aliases (`"cutoff"`) and presets (`"@preset": "pad"`),
  and the profile's `setup` is applied automatically (`"@setup": false` turns it off). A full sound can be saved as a state file and loaded with `"@state": "<file>"`.
- muser ships profiles for these open-source plugins (usable directly once installed; English skills):

  | Plugin | Use | Highlights |
  |---|---|---|
  | [Surge XT](skills/plugins/surge-xt.md) | Instrument | Hybrid synth, with bass / pad / pluck / lead presets |
  | [Surge XT Effects](skills/plugins/surge-xt-effects.md) | Effect | `type` selects among 28 effects |
  | [Vital](skills/plugins/vital.md) | Instrument | Wavetable synth: `@wavetable` picks the wavetable, `@modulations` connects modulation; bass / sub / reese / supersaw / pad / pluck / wobble / pwm_lead / keys presets |
  | [Airwindows Consolidated](skills/plugins/airwindows-consolidated.md) | Effect | About 470 effects, chosen with `"effect": "<name>"` (reverb, tape, saturation, console, lo-fi...), with parameters varying by effect |
  | [Plasma](skills/plugins/plasma.md) | Effect | Two-stage distortion: warm / bass_grit / drum_crush / lofi / scream presets |

  Plugin paths may be written with `~/` (such as `~/.clap/Airwindows Consolidated.clap`).

## Making sound move: automation

Parameters can change over time: filter sweeps, fade in / out, pan swings, seeded random walks. Write them in a track's or the project's `automation`:

```json
"automation": [
  { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp" },
  { "target": "pan", "shape": "lfo", "wave": "sine", "period_beats": 8, "min": -0.6, "max": 0.6 },
  { "target": "effects[0]", "param": "mix", "shape": "random", "style": "walk", "step_beats": 1, "min": 0.1, "max": 0.5 }
]
```

- `target`: on a track `instrument` / `effects[i]` / `gain_db` / `pan` / `sends.<bus>`; on a bus `effects[i]` / `gain_db` / `pan` / `sends.<bus>`;
  on the project `master_gain_db` / `master_effects[i]`.
- `shape`: `points` (a curve through time points), `lfo` (periodic swing), `random`. The time unit is always beats.
- `random` is determined by the project `seed`: the same seed renders identically every time, and changing the seed gives a different variation - you can render several versions and pick one.
- Which parameters can be automated: `muser instruments <name>` marks `[automatable]` / `[automatable - takes effect on the next note]`; for plugin parameters use the plugin's parameter names, and values can be written as text.
- Full description and examples: [skills/automation.md](skills/automation.md).

## Let a person listen: muser view

`muser view` starts a local server and opens the playback page (timeline, playback, and a fader / mute / solo for every track and bus; changes are re-rendered by the engine).
You decide when to let a person listen:

- **Let them listen**: when a piece reaches a finished version (structure, instrumentation, and mix are all there); when the mix or arrangement changes a lot; when the user says "let me hear";
  when a person is needed to judge volume balance or choose between A/B versions.
- **No need**: for every small parameter tweak, or when you are self-checking with `muser audition` / `muser render` - first confirm it renders and has no obvious problems before giving it to a person.

```bash
muser view song.muser.json --open      # runs in the background; page at http://127.0.0.1:8737/
```

- While the server keeps running, when you change the project file the page re-renders and refreshes automatically. When the user clicks "Save" on the page, it directly changes the project's `gain_db` / `muted` -
  **re-read the file before changing the project** so you do not overwrite what the user just saved. End the process when done.
- After opening, tell the user what to listen for (such as "compare the bass in Drop 1 and Drop 2"), and that fader changes must be saved with "Save" to be written back to the project.
- A changes JSON the user copies to you looks like `{"changes": [{"node": "tracks[3]", "gain_db": -9}, {"node": "buses[1]", "muted": true}]}`:
  change the matching track / bus fields to the given values by `node` (MCP: `update_track` / `set_bus`).
- In an environment with no GUI (remote / container): start it as usual and have the user open it in a local browser through SSH port forwarding (`ssh -L 8737:127.0.0.1:8737 …`).

## Mix routing: buses, sends, sidechain

By default every track goes straight to the master. To have several tracks "processed together" or "share one effect", use the project's `buses` (multi-output plugins - Kontakt drum kits, multi-mic drums - can also send each output directly to its own bus: `@outputs`' `bus`, see section 9 of [plugins.md](skills/plugins.md)):

```json
"buses": [
  { "name": "reverb", "effects": [{ "type": "eq", "shape": "highpass", "frequency": 300 }, { "type": "reverb", "mix": 1.0, "decay_seconds": 3 }] },
  { "name": "drums",  "effects": [{ "type": "compressor", "threshold_db": -24, "ratio": 4, "attack_ms": 15 }] }
],
"tracks": [
  { "name": "kick", "instrument": "drums.909", "output": "drums" },
  { "name": "pad",  "instrument": "poly.warm_analog_pad", "sends": { "reverb": -6 },
    "effects": [{ "type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick" }] }
]
```

- `output`: where the whole signal goes (`"master"` by default, or a bus name). Use it for drum grouping and group processing.
- `sends`: send an extra copy to an effect bus, `{bus: dB}`, post-fader. Use it for a shared reverb/delay, with the effect in the bus at `mix: 1`.
- `sidechain`: the compressor is triggered by another track's/bus's signal - the kick ducking the pad/bass for a pumping feel. A muted trigger track still triggers.
- A bus can have its own `effects` / `gain_db` / `pan` / `output` / `sends` / `automation`; a bus can send into a bus, but cycles are not allowed.
- `muser routing <project>` prints the signal flow; take a look after changes. Full description and mixing experience: [skills/routing.md](skills/routing.md).
- To balance track volumes: `muser mix-report <project>` gives each track's dry and post-fader loudness and its difference from the loudest track; see [skills/mixing.md](skills/mixing.md).

## Effect index

Effects go in `track.effects` (track level, before that track's gain/pan), a bus's `effects`, or `master_effects` (the master, before the master gain),
chained in array order. Every parameter can be omitted; `{ "type": "reverb" }` is a usable reverb.
Any effect can take `"bypass": true` to be turned off temporarily (bypass: the sound passes through unchanged, parameters and position are kept, and the automation's `effects[i]` stays the same); in `muser view` click the effect's block to open its panel (On switch, parameters, or the plugin's own window).

| type | Purpose | Sub-skill |
|---|---|---|
| `eq` | Equalize / filter a frequency band | [eq.md](skills/effects/eq.md) |
| `compressor` | Compress dynamics | [compressor.md](skills/effects/compressor.md) |
| `chorus` | Thicken, widen | [chorus.md](skills/effects/chorus.md) |
| `phaser` | Sweeping, flowing feel | [phaser.md](skills/effects/phaser.md) |
| `flanger` | Metallic sweep | [flanger.md](skills/effects/flanger.md) |
| `delay` | Echo | [delay.md](skills/effects/delay.md) |
| `reverb` | Sense of space | [reverb.md](skills/effects/reverb.md) |
| `limiter` | Peak limiting against overs (last on the master) | [limiter.md](skills/effects/limiter.md) |
| `multiband_compressor` | Three-band compression (mastering, drum bus) | [multiband_compressor.md](skills/effects/multiband_compressor.md) |
| `transient_shaper` | Shape attack / sustain (drums) | [transient_shaper.md](skills/effects/transient_shaper.md) |
| `tape` | Tape saturation and wobble (warm, lo-fi) | [tape.md](skills/effects/tape.md) |
| `distortion` | Overdrive / distortion / fuzz / fold with cabinet simulation (rock and metal guitars, gritty bass) | [distortion.md](skills/effects/distortion.md) |
| `bitcrusher` | Bit and sample-rate reduction (8-bit, old samplers) | [bitcrusher.md](skills/effects/bitcrusher.md) |
| `tremolo` | Volume wobble, chopping | [tremolo.md](skills/effects/tremolo.md) |
| `auto_pan` | Automatic left-right swing | [auto_pan.md](skills/effects/auto_pan.md) |
| `widener` | Stereo widening (lows stay mono) | [widener.md](skills/effects/widener.md) |
| `ring_mod` | Metallic, bell, robot | [ring_mod.md](skills/effects/ring_mod.md) |
| `frequency_shifter` | Frequency shifting, endless spiral | [frequency_shifter.md](skills/effects/frequency_shifter.md) |
| `formant_filter` | Vowel filter, making a synth "talk" | [formant_filter.md](skills/effects/formant_filter.md) |
| `granular_delay` | Granular delay, shimmer atmosphere | [granular_delay.md](skills/effects/granular_delay.md) |
| `vocoder` | Vocoder (`sidechain` names the vocal track) | [vocoder.md](skills/effects/vocoder.md) |
| `plugin` | Third-party VST3/CLAP/LV2 | [plugin.md](skills/effects/plugin.md) |

### Recommended chain order

- Single track: `eq → transient_shaper → compressor → distortion/tape/bitcrusher → chorus/phaser/flanger/tremolo → delay → reverb` (skip what you do not need).
- Master: `eq → compressor or multiband_compressor → limiter`, with the limiter always last.
- Bass and kick generally get no reverb / chorus; for other tracks use an `eq` highpass around 150Hz to leave the low end to them.

## Workflow example

**User**: "Make a 90bpm lo-fi beat, a 4-beat loop, drums + bass + electric piano chords."

1. Look up instruments: `muser instruments --family drums`, `muser instruments --family poly`,
   and pick `drums.lofi`, `bass.finger`, `poly.wurly`.
2. Look up effects: `muser effects reverb`, and add a little reverb to the electric piano.
3. Write the project:

```json
{
  "tempo": 90.0,
  "master_effects": [
    { "type": "compressor", "threshold_db": -14, "ratio": 2, "makeup_db": 2 },
    { "type": "limiter", "ceiling_db": -1.0 }
  ],
  "tracks": [
    {
      "name": "drums",
      "instrument": "drums.lofi",
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 36, "start": 0.0, "length": 0.25, "velocity": 0.9 },
        { "pitch": 38, "start": 1.0, "length": 0.25 },
        { "pitch": 36, "start": 2.5, "length": 0.25, "velocity": 0.7 },
        { "pitch": 38, "start": 3.0, "length": 0.25 },
        { "pitch": 42, "start": 0.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 0.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 1.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 1.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 2.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 2.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 3.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 3.5, "length": 0.1, "velocity": 0.35 }
      ]}]
    },
    {
      "name": "bass",
      "instrument": "bass.finger",
      "gain_db": -2.0,
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 33, "start": 0.0, "length": 1.5 },
        { "pitch": 29, "start": 2.0, "length": 1.5 }
      ]}]
    },
    {
      "name": "keys",
      "instrument": "poly.wurly",
      "gain_db": -6.0,
      "effects": [
        { "type": "eq", "shape": "highpass", "frequency": 150 },
        { "type": "reverb", "mix": 0.3, "decay_seconds": 2.0, "size": 0.6 }
      ],
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 57, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 60, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 64, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 53, "start": 2.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 57, "start": 2.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 60, "start": 2.0, "length": 2.0, "velocity": 0.6 }
      ]}]
    }
  ]
}
```

4. `muser validate beat.muser.json` → `muser render beat.muser.json -o beat.wav`.
5. Tell the user the wav path, duration, and peak, and modify incrementally per their feedback.

## Common arrangement patterns

- **Repeat several bars**: copy the clip and move `start` back by 4 beats (bar 2 is `start: 4.0`), or fill one long clip.
- **Chords**: several notes with the same `start` and different `pitch`. C major triad = 60/64/67, A minor triad = 57/60/64.
- **Layers**: put different roles on different tracks (drums / bass / chords / melody), balance with `gain_db`, and separate with `pan` (-1 left to 1 right).
  Built-in instruments are volume-calibrated: at `gain_db` 0 the instruments are roughly equally loud, so `gain_db` directly expresses "how much quieter/louder than the others" (accompaniment is often -4 to -10).
  SFZ and plugins are not calibrated: the "volume" line of `muser sounds describe` says how much louder/quieter it is than the built-in instruments and where to start `gain_db`; finally verify with `muser mix-report`.
- **Volume**: the instruments' own volumes differ a lot, so do not set `gain_db` by feel. After writing a version, run `muser mix-report` and tune each track to the target relative loudness for its role,
  with a `limiter` last on the master. For the workflow and the targets for each role, see [skills/mixing.md](skills/mixing.md).

## Troubleshooting

- **Always** `validate` before rendering; it gives the exact field that is wrong and the allowed values.
- A drum kit with no sound or wrong sound -> pitched notes were used, or a key the kit does not support (see drums.md).
- Wrong sound -> first look at the preset's current values with `muser instruments <name>`, then adjust in small steps with `instrument_params`.
- Render reports "output values diverged" -> some instrument parameter took an extreme value (such as a very short release); move it back to a normal range.
- Short bowed string / wind notes sound weak -> the notes are too short; for short string notes use `*_pizz` instead.
- Reverb tail is cut off -> increase `--tail` when rendering (default 2 seconds).
- Overall muddy / murky -> add an `eq` highpass on non-bass tracks and reduce the reverb on many tracks.
