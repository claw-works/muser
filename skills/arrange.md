# Arrangement layer: sections, form, chords, chord-following patterns, transforms

To write a whole song, do not compute each clip's start by hand: **define a section once and order the form by name; write chords on the section, and every part follows the chords**.
A song with 9 sections, 6 tracks and 56 bars takes about 40 lines (example: [samples/arrange/night-drive](../samples/arrange/night-drive.muser.json)).
Before rendering, everything is expanded into ordinary clips; view, `mix-report`, and MIDI / ABC export all see the expanded form.

```json
"sections": {
  "verse":  { "bars": 8, "chords": "Am7 Fmaj7 C G" },
  "chorus": { "bars": 8, "chords": "F G | Em Am" }
},
"form": ["verse", "chorus", "verse", { "section": "chorus", "transpose": 2, "mute": ["pad"], "seed": 3 }],
"tracks": [
  { "name": "bass", "instrument": "bass.finger", "parts": {
      "verse":    { "chordal": "1 . . 1 . 5 8 .", "octave": 2 },
      "chorus":   { "chordal": "1 . 1 5 8 . 5 3", "octave": 2, "end": { "beats": 2, "chordal": "5 4 3 2", "octave": 2 } },
      "chorus#3": { "chordal": "1 - - - - - - -", "octave": 2 }
  } },
  { "name": "lead", "instrument": "wind.flute", "parts": {
      "chorus": { "abc": "L:1/8\nK:Am\nc4A2c2 | d4B2G2 | ...", "legato": true } } }
]
```

## sections / form

- `sections`: section name -> `bars` (number of bars) and `chords` (optional), plus an optional `time_signature`: this section's own time signature,
  such as `"bridge": { "bars": 4, "time_signature": { "numerator": 7, "denominator": 8 } }` (if omitted, the project's is used). Chords are laid out by this section's bar length;
  the bar numbers in `analyze`, the MIDI export (a time signature event is written at each change), and the bar lines in `muser view` all follow.
  Note: the drum patterns of `drummer` are written for 4 beats per bar; in other time signatures use `grid` / `abc`; the ABC export writes an inline `[M:]` where the time signature changes.
- A section can also have its own tempo: `"tempo": 140` switches to 140 BPM from the start of this section, and later sections that do not write it keep it;
  adding `"tempo_ramp_bars": 4` ramps from the current tempo over the first 4 bars of this section (accelerando / ritardando, uniform in time; omitted or 0 switches directly).
  Clip and automation times are still written in beats, and follow the beats when the tempo changes; the transport that plugins receive, the tempo map in MIDI export (a ramp is written as one tempo per beat),
  the inline `[Q:]` in ABC, and the bar lines in `muser view` all follow.
- A section can also change the tuning: `"tuning": { "scale": "just", "key": "D" }` (written like the project's `tuning`, see SKILLS.md) takes effect from the start of this section, and later sections that do not write it keep it;
  `{ "scale": "equal" }` goes back to 12-tone equal temperament. Notes already sounding keep their pitch; the next note on uses the new tuning (pitched built-in instruments and SFZ; drum kits and plugins are unaffected; MIDI export carries no tuning).
- `form`: the sections played in order from the start of the project. An entry can be a name, or carry parameters for this pass:
  - `transpose`: transpose this pass (in semitones; chords, chordal and melody move together, the drum kit does not). Use it for a key change on the last chorus.
  - `mute`: names of tracks that rest during this pass.
  - `seed`: use a different randomization for this pass (drummer, `vary`, humanize).
- The original `clips` (absolute beats) still work as usual, and can be mixed with parts.

## parts: what this track plays in each section

- Keys: `"verse"`; `"verse#2"` (only the 2nd verse, overrides `"verse"`); `"*"` (used by every section not written separately). If no section matches, the track rests.
- The content is written as a clip: `notes` / `abc` / `grid` / `drummer` / `chordal`, plus `humanize`, `legato`, `pedal`, `transform`.
- `length` (beats): the content loops at this length to fill the section. Default: ordinary content = the whole section (no loop), chordal = the pattern length.
- `"align": "global"`: the loop is counted from the start of the project (beat 0) and does not reset across sections (default `"section"`: each section starts over). Use it for polymeter:
  `"*": { "grid": { "hat": "x......" }, "align": "global" }` (7 steps) against another track of 13 steps keeps drifting however the sections change, with no need to rewrite as absolute-beat clips. When several consecutive sections use the same global part they are merged into one (long notes crossing a section line sound as usual, and in `muser view` it is one whole strip).
- `end`: the last `beats` beats of every pass of the section are replaced by this content (fill, turnaround, phrase-end variation); written like a part.
- When `drummer` writes the whole section (no `length`), it plays fills according to the section length; each pass of the section has a different seed, so no two passes are exactly the same.

## chords: chord progression

- Chords separated by spaces, each taking one bar: `"Am7 F C G"`.
- When bars are separated with `|`, the chords within a bar split that bar equally: `"F G | Em Am | Dm7 G7 C"`.
- A progression shorter than the section loops (an 8-bar section with 4 chords = two passes).
- Notation: root (`C` `F#` `Bb`) + quality: `m` `7` `maj7` `m7` `dim` `dim7` `m7b5` `aug` `sus2` `sus4` `7sus4` `6` `m6`
  `9` `maj9` `m9` `add9` `11` `13` `5` `m(maj7)`, altered tones `b9` `#9` `#11` `b13` `b5` `#5` (such as `G7b9`, `C7#11`), slash bass `C/E`.

## chordal: patterns written against the current chord

```json
{ "chordal": "1 . 5 . 8 . 5 3", "step": 0.5, "octave": 2, "velocity": 0.8 }
```

Each cell is `step` beats (default 0.5 = an eighth note), and notes are picked from the chord of that moment:

| Symbol | Meaning |
|---|---|
| `1` `3` `5` `7` | The chord's actual root, third, fifth and seventh (the 3 of a minor chord is a minor third, the 5 of a diminished chord is a diminished fifth; for chords with no seventh, `7` takes the octave) |
| `8` `9` `11` `13` | Octave root, ninth, eleventh, thirteenth (if the chord names b9 / #11 etc., those are used) |
| `2` `4` `6` | Major second, perfect fourth, major sixth (passing tones) |
| `b7` `#11` `b9`... | Explicit flat / sharp: raised or lowered from the major-scale degree (ignores the chord) |
| `B` | Bass: the bass of a slash chord (the E of `C/E`), otherwise the root |
| `c` | The whole chord: close voicing, automatically choosing the inversion nearest to the previous chord (voice leading), avoiding voicings with a minor second |
| `'` `,` | Appended to raise / lower an octave: `3'`, `1,`, `c'` |
| `!` | Appended for an accent: `1!`, `c!` |
| `.` / `-` | Rest / extend the previous cell |

- `octave`: register. The root is placed in this octave (C4 = middle C), default 3; `c` voices inversions centered on the G of this octave. Bass 2, chords 3-4, arpeggios 5.
- Common ones: bass `1 . . 1 . 5 8 .`, `1 1 5 5 8 8 5 5`; walking bass `1 3 5 6`; arpeggio `1 5 8 3' 5' 3' 8 5` (`step: 0.25`);
  pad `c - - -` (`step: 1`, one per bar); syncopated strum `c . c c . c . .`; grace-note style `B . 5 .`.
- **Changing chords**: only change `sections.<section>.chords`, and all chordal parts change together; change the melody separately.

## transform: transformations

Written on a part (applied on every loop pass) or an ordinary clip, applied in order, with only one kind per object:

```json
"transform": [{ "transpose": 5 }, { "every": 4, "then": [{ "reverse": true }] }, { "vary": 0.2 }]
```

| Transform | Effect |
|---|---|
| `transpose: n` / `octave: n` | Transpose by n semitones / raise or lower by n octaves (the drum kit is unaffected) |
| `reverse: true` | Retrograde: time runs backward |
| `invert: 60` | Inversion: flip up/down around this pitch as the axis |
| `stretch: 2` | Double the durations (0.5 = halve), and the loop length changes with them |
| `shift: 0.5` | Offset: move half a beat later, with what moves past the end of the loop wrapping to the start |
| `sequence: [0, -1, -2, -3]` | Sequence: on the k-th loop pass, shift by the k-th number of scale degrees (the scale is taken from the notes in this section's chords). The motif steps down / up -- the most common development technique in pop songs |
| `fragment: 2` | Truncate: keep only the first 2 beats, and the loop length also becomes 2 beats. Often combined with `sequence`: `[{ "fragment": 2 }, { "sequence": [0, 1, 2] }]` pushes the start of the motif up step by step |
| `every: 4, then: [...]` | Apply the transforms in `then` only on every 4th pass (the 4th, 8th...): phrase-end variation, ornament on bar 4 |
| `vary: 0.2` | Random variation: each note has a 20% chance of becoming a neighboring chord tone (when no chords are written, a neighboring pitch that appears in the pattern), and a few are dropped; for the drum kit, notes are dropped and velocities vary. The same seed gives the same result; change it with `seed` or the `seed` in `form` |

## After writing: `muser analyze`

`muser analyze song.muser.json` (MCP `analyze`) reports, per section, density, each track's register, chords, repetition and conflicts, and ends with suggestions;
adding `--audio` renders once more and gives the loudness and spectral centroid of each section. After editing per the suggestions, run it again:

- **The chorus does not lift**: the chorus's density / loudness is not higher than the verse -> add parts (arp, pad), raise the melody's register, raise the drummer's `intensity` / `complexity`.
- **Too much repetition**: most bars are identical to an earlier bar -> `vary`, `every`, an `end` fill, a different way of writing `verse#2`.
- **Only one chord**: write `chords`.
- **Register masking**: two tracks sound at the same time in the same register for a long stretch -> change the `octave` of one of them, or let it rest in this section (do not write this section's part).
- **Avoid notes**: a long note on a beat is a semitone above a chord tone (b9, the 4 over a major chord, b6) -> change it to a chord tone, or change the chord.
- **Minor second / minor ninth**: two tracks hold notes only a semitone apart (including a compound interval across an octave) -> change the inversion, spread the registers apart, or change the chord (e.g. Fmaj7 -> F).
- **Too dark / too bright** (`--audio`): spectral centroid below 1000 Hz / above 4500 Hz.

### Energy curve and the "sounds like AI" check

At the top of the report is the energy curve (0-10 per section, from density, the number of parts present, and velocity, plus loudness with `--audio`), followed by the "sounds like AI" check --
features that real arrangements almost always have and machine defaults often lack; those that hit (marked x) are written into the suggestions:

| # | Triggered when | How to fix |
|---|---|---|
| 1 | Three or more sections, and every section is a multiple of 8 bars | Change the intro / pre-chorus / outro to 2, 4, 6, 10 or 12 bars, or add a 2-bar transition |
| 2 | Energy never drops | Before the interlude, bridge and last chorus, take out a few instruments (`mute` in `form`, or do not write the part in that section) |
| 3 | Every track is present in every section | Let instruments come and go: fewer in the intro, bring them in at the chorus |
| 4 | Every section has the same progression, or the whole song uses only diatonic chords with no key change | Write different `chords` for pre / bridge, use a borrowed or non-diatonic chord (borrowed iv, bVII, secondary dominant V/vi), or `transpose` the last pass |
| 5 | A fill nearly every 4 bars | Do not write `fill_every` on the drummer (automatic), or write fills with `end` |
| 7 | (`--audio`) Sections differ by less than 1.5 LU in loudness, or the spectral centroid barely changes | Make the intro / interlude emptier and darker, and the chorus fuller and brighter |
| 8 | None of the melody / harmony tracks has humanize | Add `humanize` (the drum kit can stay aligned) |
| 11 | The chorus has identical notes on every pass | Write `chorus#2` differently, add parts, `vary`, or `transpose` the last pass |

These are common traits of "sounding human-made", not iron rules: if it is intentional (for example a loop-based electronic piece using a single progression), keep it, just be aware of it.
