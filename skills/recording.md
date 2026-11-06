# Helping the user record vocals

How to work when the user wants to sing themselves and have the agent help record, edit and mix. This is the **method and a starting point**: all specific numbers are only a first-version starting point,
music is taste, and in the end the user's intent and ears decide -- give a version first, say clearly what you did, then change it according to feedback.

## Preparation

- **Record in the user's project directory**: recordings are written to `recordings/` beside the project file. Copy example songs and templates into the user's working directory first and then record,
  and do not write anything into muser's own `samples/`.
- **Lyrics**: write the original lyrics in the project's `lyrics` (section markers like `[A]` are ignored); `muser view` shows them at the bottom as karaoke.
- **Guide melody**: only when there is a track whose notes carry `lyric` (the `w:` line in ABC) does karaoke know when each word is sung,
  and the later timing check also compares against it. You can write it with `choir.solo`, and after the user has recorded, mute it (do not delete it).
  One Chinese character per syllable: the number of characters to sing in `lyrics` must equal the number of syllables (do not mix in titles or annotation text), otherwise it can only show by syllable.

## Recording

`muser view <project> --open` (play in the browser, record in the browser) or add `--device` (the backing track goes out of this machine's sound card, and muser records directly from the default microphone,
with the latency reported by the sound card driver compensated automatically; more accurate with an external sound card). Do not add `--device` when muser runs on a remote machine. Tell the user:

- Wear headphones, so the backing track does not leak into the microphone.
- Right-drag on the time ruler to mark the section to record, and click "● Record the loop range" (with a count-in); if no region is marked, it records from the playhead until stop.
- The same section can be recorded many times: a new take replaces a take it mostly covers (a retake of the same line), and old ones stay in the list. Each take has a "Use" checkbox: several can be ticked at once and may overlap (recording line by line, the end of one take and the start of the next overlap; both are kept), and "Delete".
- Scissors (✂ / C) cut at breath points, drag to move position, and use the left/right arrows to nudge once selected.
- Click "Save" when satisfied; until then the project file is unchanged.

Give the user a table of the song's sections, bar numbers and the corresponding lyrics; recording section by section is much easier than recording in one go.

**Key does not fit** (a different person singing, female / male voice, cannot reach the notes): first try "Transpose - / +" on the view playback bar, shifting the whole song by some semitones without changing the notes;
when it fits, click "Save" (the project's `"transpose": N`) and then record. Vocals already recorded do not transpose with it, so re-record after changing the key.

## Checking and alignment

- `muser takes <project>` shows each take: start, length, level, the part actually sung, and whether the project uses it.
- **Timing**: `muser takes <project> --timing` gives, for each phrase, how many milliseconds early / late it is relative to the guide melody (it compares the recording's pitch contour with the guide melody by relative pitch, so a different octave is fine);
  when the match is poor (average error over a semitone or two) the conclusion is not trustworthy, so look at it a different way or ask the user directly.
  - Every take and every phrase off in the same direction by about the same amount -> most likely latency in the recording chain; have the user compensate in "latency fine-tune" and record again.
    The most direct is "Measure latency" in the recording panel (take off the headphones, turn the volume up, and it measures and fills in automatically); you can also have the user clap along to the beat for a recording, since clapping is more accurate than singing.
  - Individual phrases off -> it is the singing. The human ear notices a vocal and the beat differing by a few tens of milliseconds, but tighter is not always better: slight ahead/behind is the human feel.
    Where the user says something is slow, measure there; do not "correct" what they did not mention on your own initiative.
  - Method of moving: cut at the breath before that phrase (`muser clip split <project> --track vocal --at <beat>`), and move the latter half (`muser clip move ... --clip N --by-ms -60`);
    do not let the two pieces overlap (the overlap would play the same phrase twice), and add a `fade_in` / `fade_out` of a few tens of milliseconds at the seam. Measure again after moving, and tell the user what was changed and by how many milliseconds.
- **Choosing a take**: if the user said which take to use, use that one; if not, you can choose by "the sung part is complete, level is normal, timing deviation is small" and give your reasons.

## Mixing

- Vocals recorded with a home microphone are often quiet and dry; common starting points:
  - Effect chain: highpass (cut plosives and low rumble) -> cut a little at 200-400 Hz (remove muddiness) -> compression (so soft words are also clear) ->
    boost a little at 2-5 kHz (diction) and the high frequencies (air) as needed.
  - Space: a dedicated reverb bus for the vocal (cut the lows and the highest frequencies in the reverb so it does not get muddy or harsh), plus a little echo that follows the beat;
    also send a little to the reverb used by the backing, so the vocal and instruments feel like they are in the same space.
  - Volume: the lead vocal is generally at the front of the mix; use `mix-report` and set it against the loudest instrument.
- **Wetness, volume, and brightness are all taste**: ballads, pop, rap and experimental music want completely different things. Give a middle-of-the-road version first, say roughly how many dB the reverb is below the dry signal,
  let the user say "too wet / too dry / a bit louder", and adjust to the feedback.

## Ambience and endings

- Before adding ambience (wind, rain, crowd, environment), work out its role:
  - **Background**: only creates a feeling, usually far below the lead vocal (tens of LU), with the bands that fight with the vocal and main instruments cut out, and its loudness rising and falling with the sections.
  - **Foreground**: a part of the expression itself (storm, thunderstorm, a deliberate wall of noise); follow the degree the user wants.
  - When unsure, ask, or give a lighter version first and then ask whether to strengthen it.
- **Ending**: when the last note of a song is abruptly cut off, the common remedy is to add an ending section -- a long tonic chord, a strum or arpeggio, with the bed parts fading out --
  and leave tail when rendering (`--tail`). When the user wants a sudden stop (hard cut), keep the hard cut.

## Delivery

- Before changing the user's project, back it up first and tell the user where the backup is.
- Export: `muser render <project> -o song.wav`, or `-o song.mp3` to output an MP3 directly. To compare versions (for example synthesized humming vs. a real person), save a temporary project and render that, leaving the user's project untouched.
