# Shorthand notation: ABC melodies, grid rhythms, the loop library, drummer

Do not write `notes` arrays one by one, and do not write scripts to generate notes: use the shorthands below on a clip, which expand at render time (merged with `notes`,
with all times relative to the clip start). The project file is therefore short, a human can read it, and later you edit these few lines directly. `clip.length` must still be written as usual,
and grid, loop and drummer loop within that length.

## ABC: melody and harmony (`"abc"`)

```json
{ "start": 0, "length": 8, "abc": "L:1/8\nK:Am\nA2 cB e2 dc | B4 z4 |" }
```

- `C` = middle C (60), `c` is an octave higher, `'` raises another octave and `,` lowers an octave; `^` sharp, `_` flat, `=` natural; the `K:` key signature applies automatically (`Am` `Dmix` `Edor` `F#m` `Bb` ...),
  and accidentals last for the rest of the bar.
- `L:` unit note length (default 1/8): `A2` double, `A/2` half, `A3/2`; rest `z`, whole-bar rest `Z` (`Z4`).
- Chord `[CEG]2`, tie `A2-A`, dotted `A>B`, triplet `(3ABc`, repeat `|: ... :|`, first and second endings `[1 ... :|[2 ...`.
- Velocity `!p!` `!mf!` `!f!` (this velocity is used from then on), accent `!>!`, staccato `.A`. Chord symbols `"Am"` and grace notes `{g}` are ignored.
- One voice per clip; the other voices of the harmony (bass, chords) go in the clips of other tracks.
- Lyrics: the `w:` line matches the notes in order (spaces or `-` separate syllables, `*` skips a note, `_` extends the previous syllable), and `choir.*` instruments will sing them, see [instruments/choir.md](instruments/choir.md).
- Mistakes report the location, such as `tracks[0].clips[0].abc: line 2, column 5: chord [ is not closed`.

## grid: step rhythm (`"grid"`)

```json
{ "start": 0, "length": 16, "grid": { "kick": "x...x...x...x...", "snare": "....x.......x...", "hat": "x.x.x.x.x.x.x.x." } }
```

- One character per step, `step` beats per step (default 0.25 = a sixteenth). Each row loops within the clip at its own length.
- Row names: drum pieces (`kick` `snare` `clap` `rim` `hat` `open_hat` `pedal_hat` `crash` `ride` `tom_high` `tom_mid` `tom_low` `floor_tom`
  `shaker` `cowbell` ...), MIDI numbers (`"36"`), or note names (`"c2"` `"eb3"`, C4 = 60, for pitched instruments such as bass lines).
- `x` hit (0.8), `X` accent (1.0), `o` soft / ghost note (0.45), `1`-`9` velocity 0.1-0.9; `=` or `~` extends the previous note by one step; `.` `-` rest; spaces and `|` are ignored.

## Loop library: rhythm patterns and motifs

```bash
muser loop list                 # all (project + global + built-in), filter with --style dnb
muser loop play dnb_two_step    # audition on the sound card (--instrument drums.909 to change the kit, --tempo 170)
muser loop show dnb_two_step --start 16 --repeat 8   # output clip JSON, to put straight into a drum track's clips
muser loop show hook --abc      # the ABC of a melodic loop
muser loop edit --open          # editor: a person draws motifs and drum patterns in the piano roll / step grid
muser loop save song.muser.json --track 0 --clip 2 --name my_break --style dnb   # save a clip in the project as a loop (stored in the project)
muser loop share hook           # share one from the project to global
muser loop hum hum.m4a --tempo 96 --name hook   # turn a humming / whistling recording into a melodic loop
```

- Drum loops store `grid`; melody / motif / harmony patterns store `notes`, and may carry a suggested instrument, key (`key`) and chords (`chords`).
- The clip output by `show` embeds the content and the groove `humanize`, and is marked `"from": "loop:<name>"`. The project is therefore self-contained, and later changes to the loop do not affect old projects.
  Drum pieces are written by name, so any `drums.*` kit can play them (`kit` is only a suggestion).
- Three locations; when names collide, the nearer wins: project (`loops/` beside the project) > global (`~/.config/muser/loops/`) > built-in.

### The person gives a motif, the agent continues / extends it

The person draws a few bars of motif in `muser loop edit`, or clicks "● Hum to notes" and hums into the microphone (converted automatically into notes, which can be edited in the piano roll), and saves it to the project;
if a recording file already exists, use `muser loop hum hum.wav --tempo 96 --name hook` (no AI, pure signal processing; it prints ABC). The agent then continues:
1. Read it with `muser loop show <name> --file` (or `--abc`): pitches, rhythm, key, chords.
2. Continue / extend: develop it with the arrangement layer's `transform` (`sequence`, `fragment`, `vary`, `invert`, `stretch`, see [arrange.md](arrange.md)),
   or write the phrases that follow (question -> answer with a changed cadence; or motif -> repeat / sequence -> fragment and speed up -> cadence).
3. Give 2-3 versions at a time, each saved as a loop in the project (such as `hook-a`, `hook-b`); once the files are written to `loops/` they appear in the editor automatically and can be auditioned and modified;
   tell the person what technique each version used, and let them choose.
4. Use the decided one in the project: put the notes from `loop show` into parts / clips, and write `chords` into sections.

## drummer: a drummer that varies and plays fills (`"drummer"`)

```json
{ "start": 0, "length": 32, "drummer": { "loop": "rock_basic", "complexity": 0.6, "intensity": 0.6 } }
```

| Field | Meaning |
|---|---|
| `loop` | The base (a name from `muser loop list`) |
| `complexity` | 0-1: 0 plays the loop as is; the higher it is, the more snare ghost notes, kick syncopation and hi-hat ornaments, and the longer the fills |
| `intensity` | 0-1: how hard it plays (0.5 is unchanged) |
| `fill_every` | Default is automatic: it always plays one in the last bar of the clip, and in between plays at the end of 4-bar phrases at random (about one in three), 1 or 2 beats long -- not mechanically every 4 bars; with N, one every N bars, and 0 plays none |
| `crash` | Add a cymbal on the first beat of the bar after a fill, default true |
| `seed` | When omitted it is derived from the project seed; change the seed for a different way of playing |

- No two bars are exactly alike; when `humanize` is not written, the base loop's groove is used.
- Use `grid` when you need precise control note by note; use `drummer` when you want "play a passage like a drummer".

## Following the chords: chordal

Use `abc` for melody; for parts that "follow the chords" such as bass, chords and arpeggios, use `chordal` in the arrangement layer (`"1 . 5 . 8 . 5 3"`, `"c - - -"`),
with the chords written in the section's `chords`, so they change together when the chords change. See [arrange.md](arrange.md).

## Mixing them

In one clip, `notes`, `abc`, `grid` and `drummer` can all be written at once, and the notes are merged: for example `drummer` as the base, with `grid` adding a `shaker` row.
The timeline in `muser view` shows the expanded notes.

## The other way: export ABC / MIDI

- `muser abc <project>`: writes the whole piece as standard multi-voice ABC (one voice per instrument track), for models that can read ABC to analyze, continue and adapt.
- `muser abc <project> --yue2 --vocal <melody track> --sections verse:8,chorus:8`: YuE2's native score (Vocal / Ins + chord symbols),
  to use directly as its `--abc-file` (`--cot full`; add `--no-chords` for `--cot melody`).
- `muser midi <project>`: a standard MIDI file, to hand to a DAW.
- Exports write the notes on the score (before humanize); drum kits and audio tracks are not exported; chord symbols are estimated, so take a look before handing them out. See [TOOLS.md](../TOOLS.md) for details.
