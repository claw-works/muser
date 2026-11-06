# Live session (`muser live`)

Use when the user says "play something live", "improvise some music for me", "change it as I listen". You write one section after another, muser plays them on this machine's sound card and keeps the music going;
the human can change direction in the conversation at any time. This is the **method**: what music to write and how long, follow the user's style and feedback.

## How it works

- The whole session is one project that grows forward: a template (tracks, effects, buses, tempo) plus the **sections** you submit one after another. Each section has its own bar count and time signature;
  clip and automation times are all **relative to the start of this section**.
- **No dead air**: when the queued music has played out and there is no new section yet, **the last submitted section loops as a whole** (reverb and delay tails carry into the next pass).
  So you do not need to stay online all the time: the music keeps playing after you stop.
- A new section is queued after the ones already submitted by default; while looping, the current pass finishes and then the new section follows, so phrases are never cut in half.
- Changes take effect in about 1.5 seconds at the earliest (the engine prepares ahead). Receipt: `bar` (which bar), `beat_in_bar` (which beat in the bar; a section always starts on a bar line, so this is 1),
  `starts_in_seconds` (seconds until it starts), `absolute_beat` (beat count from the start of the session, rarely needed), `queued_until_bar` (bar up to which music is queued).

## Starting

1. Pick a template: the release's `samples/live/` has `electronic` (124 BPM, drums / bass / chords / pad / lead, reverb and echo buses),
   `ambient` (72 BPM, drone / pad / bells / bass / flute / lo-fi drums), `band` (96 BPM, acoustic drums / bass / electric piano / guitar / organ / saxophone).
   You can also write your own: an ordinary project where **every track must have a name**, the buses must be created up front (buses cannot be added midway), and there are no audio tracks. The clips in the template are the opening, and loop at first.
2. Start: `muser live start <template> --detach --title "<a title that fits what the person wants>" --record session.muser.json --autoplay 3m --open` (or MCP `live_start`).
   `--detach` fully separates it from your shell and returns once playback starts -- **always add it**; do not use `nohup ... &` (some agent environments kill the processes a command left behind when it finishes).
   `--open` opens the live page (position, queued sections, per-track levels, a stop button); tell the user they can watch it.
   The sound card and local connections (127.0.0.1) may be unavailable in a sandbox: if you get an error, ask the user to allow this command to run outside the sandbox.
3. Tell the user in plain words: the music comes out of this computer's speakers; to change something, just say so in the conversation; and how long you will keep writing on your own.

## Writing a section

```json
{ "name": "groove A", "bars": 8,
  "time_signature": { "numerator": 4, "denominator": 4 },
  "tracks": [
    { "name": "drums", "clips": [{ "start": 0, "length": 32, "grid": { "kick": "x...x...x...x...", "hat": "..x...x...x...x." } }] },
    { "name": "bass",  "clips": [{ "start": 0, "length": 32, "abc": "L:1/8\nK:Am\nA,,2 A,,2 C,2 E,2 | …" }],
      "automation": [{ "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 400], [32, 2400]] }] },
    { "name": "pad", "instrument": "poly.glass_pad", "output": "verb", "gain_db": -16,
      "clips": [{ "start": 0, "length": 32, "notes": [{ "pitch": 57, "start": 0, "length": 32 }] }] },
    { "name": "lead", "effects_add": [{ "type": "delay", "time_seconds": 0.36, "feedback": 0.4, "mix": 0.25 }] }
  ] }
```

- Submit: `muser live send seg.json` (`-` reads standard input) or MCP `live_send`.
- `name`: give the section a name ("intro", "blues 2"); it is shown on the page, and the section name is also carried into the recorded project.
- `note`: one sentence on what changed in this section ("switch to 6/8 campus folk: fingerpicked acoustic guitar, flute comes in"). It is shown on the page when the section changes, so the listener knows what happened -- **write it for every section**.
- `title`: when the person changes direction ("give me some country"), you can also update the title of the whole session.
- **Read the description before using a sound**: `muser instruments <name>`. Some sounds map one key to one whole chord (`poly.rave_chord_stab` and other chord stabs);
  writing a triad for them stacks into a harsh clump -- they take single notes only; some are monophonic (acid bass, solo voice), and overlapping notes glide.
- The built-in drum kit's **ride cymbal sounds bad right now** (like a cracked gong): for jazz and country swing grooves, play `hat` (closed hat) or `pedal_hat` first;
  for a real ride, use a local SFZ / plugin drum kit (`muser sounds list`).
- **Keep drum patterns short**: each row of a `grid` loops by itself across the whole clip, so 16 steps for one bar is enough; no need to write out all 8 bars. But the **clip's `length` must cover the whole section** (in an 8-bar 7/8 section: `"length": 28`): the rows loop only inside the clip, so a one-bar clip plays one bar and then the drums stop.
  Ready-made grooves are in `muser loop list` / `muser loop show <name>` (disco, funk, house...); adapting one is faster than writing from scratch.
- **Tracks you do not write rest during this section.** To keep a track going, give it a clip in this section too (`last_change` in `live status` is the full text of the previous section;
  editing on top of it is easiest: keep what should continue, change what should vary).
- Clips are written as in an ordinary project: `notes`, `abc`, `grid`, `drummer`, `humanize` all work (`chordal` and `parts` belong to the arrangement layer and are not used live); anything beyond the end of the section is cut off.
- **Big sampled sounds** (`sfz:` pianos and the like): the first `send` that brings one in loads its samples first and can take ~10 seconds to answer; the music keeps playing, and the section starts on time after that. Bring such a sound in a section early (or put it in the template), not on the bar where it must be heard.
- **New track**: write `instrument` (optionally `instrument_params`, `output`, `sends`, `gain_db`, `pan`).
- **Existing tracks** cannot change instrument, parameters, volume or pan midway: to change instrument, **open a new track**; changes to parameters, volume and pan go in `automation`.
- **Effects**: `effects_add` connects from the start of this section (appended to the end of the effect chain, indexes continue after the existing ones), `effects_off: [index]` removes from the start of this section.
  Effects cannot be deleted or inserted in the middle; effects with latency (such as a limiter) cannot be added midway and must be put in the template.
- **Automation**: takes effect from the start of this section until overridden by a later write of the same parameter; if you write `end` (relative to the start of this section) it stops there.
- **Buses and master**: `"buses": [{ "name": "verb", "automation": [...], "effects_add": [...], "effects_off": [...] }]` (the bus must exist in the template),
  `"master": { "automation": [{ "target": "master_gain_db", … }], "effects_add": [...] }`. Put overall fade-outs and reverb slowly getting wetter here.
  They take effect only the first time this section plays: when the section loops, it stays at the changed state (once faded, it stays faded) and does not restart each pass; automation on tracks repeats on every pass.
- **Changing sounds (crossfade)**: an existing track cannot change instrument. Open a new track with the new sound and fade it in with `gain_db` automation over a bar or two;
  in the same section give the old track an automation fading out to -60, and from the next section on give the old track no clips. More tracks cost more resources, so let unused tracks rest as early as possible.
- `"at": "now"`: switch at the next bar line (cutting off the section that is playing). The default is `"after"`. Use `now` when the human says "stop right now" or "change immediately".
- `"at": "patch"`: **modify the section that is playing** (add drums, change the accompaniment pattern, add something to a track) without disturbing the form -- when someone is singing along, always use it, not `now`.
  Write a complete section (written from the start of the section as before, with the same `bars`); muser continues from the next bar at the same position in this section;
  `section_bar` in the receipt says from which bar of the section the change takes over. Afterwards, when looping, the modified version repeats whole from the start.
  To adjust only volume, do not use patch; use `muser live mix`.

## Someone singing along: cues and karaoke lyrics

- **Cue**: before the singer comes in, play a 1-2 bar intro with a clear percussion sound (claves, cowbell) counting "1, 2, 3, 4"; make the intro its own section,
  with `note` saying "start singing when you hear 1, 2, 3, 4", and the `note` of the section where singing starts saying "Sing!" -- the page flashes at the moment singing starts.
- **Karaoke**: in a section write `"lyrics": "first line\nsecond line"`, then give one track a guide melody whose notes carry syllables (the `w:` line in ABC, pinyin or English, one syllable per note).
  The page shows the current line in large type (sung words change color, the word being sung lights up) and the next line, and shows "▶ Get ready" before the singing starts.
  The guide melody can play quietly to lead the singing (vibraphone, flute), or you can open a separate track at `gain_db` -80 used only to time the karaoke.
  Each Chinese character maps to one syllable; when the number of characters and syllables do not match, the page shows the syllables themselves.
- Use only public-domain lyrics (nursery rhymes, folk songs) or ones the user supplies; for copyrighted songs, make only the accompaniment and write no lyrics or melody.

## Adjusting volume while playing

- When the person says "the saxophone is too quiet" or "the drums are too loud": `muser live mix <track> <dB>` (MCP `live_mix`); it starts after about 0.4 seconds and ramps to that fader value within 0.3 seconds,
  **no new section needed, and the form is unaffected**. For a bus write the bus name, for the master write `master`; for negative values write `muser live mix master -- -3`.
- Check first yourself: the `mix` in `status` lists the current fader value `gain_db` and loudness `rms_db` of every track / bus / master (RMS about 1.5 seconds after the fader, sorted loud to quiet).
  The lead (saxophone, flute, the guide melody of a voice) usually needs to be a few dB louder than the accompaniment; after a new part comes in, take a look, and if the gap is too big, adjust proactively and tell the person what you adjusted. After moving a fader, **look at `mix` once more** about 2 seconds later to confirm the balance is what you wanted.
- If a later section writes volume automation for that track, from that section on that section's automation governs.

## The autoplay loop

During autoplay (within the duration of `--autoplay`):

1. Write a section -> submit -> `muser live wait --runway N` (MCP `live_wait`).
2. `wait` returns a `reason`:
   - `runway`: the queued music is about to run out; write the next section.
   - `idle`: still early; call `wait` once more (at most 30 seconds per call).
   - `autoplay_over`: time is up. **Submit a last section -- a phrase that is good to hear repeatedly** (do not end on a fill, a fade-out or a climax), then end this round,
     and tell the user "I'll stop here for now; the music will keep looping. Say so if you want me to continue or change anything." Do not `wait` again.
3. Set `N` longer than the time it takes you to write a section: `last_write_seconds` in `status` / `wait` is how many seconds it took from the last return of `wait` to the submission;
   N should be at least 1.5 times that. Writing slowly is fine -- when the runway runs out the last section loops and there is no dead air, it just repeats a few more times.
4. Each section is 4-16 bars, and the whole should develop: introduce, add layers, vary, leave space, return. **The runway matters more than length**: if writing a section will take more than half a minute,
   first submit a short one (a few edits on the previous section) as a cushion, then write the long one; otherwise the previous section repeats many times.
   **Don't queue far ahead**: one or two sections, then `wait`. The person can change direction at any moment, and sections queued for later then have to be cut off with `now` (whole planned passages never get played).
5. When the user names a well-known tune ("Twinkle Twinkle Little Star", "Two Tigers"): **write the melody exactly as it is** (intervals and rhythm unchanged; key and register may change),
   and put the stylistic change in the accompaniment, drums and sounds; touch the melody only when the user asks for a "variation" or "arrangement".
   If you are not sure of the exact melody, say so before playing it and ask someone who knows the song to listen to the first line; someone singing along to a wrong tune is worse than no tune. Count syllables against notes: one note per syllable (or a slur for a melisma), or the karaoke drifts.

When the human says something in the conversation (in Claude Code you see it at your next tool call): **do what the person says first**, so the next section reflects it,
then go on with the loop. For agents that cannot take interjections mid-turn, the user will press Esc to interrupt; after answering and making the change, return to the autoplay loop (if still within the duration).
When the user says "develop on your own for another 5 minutes": `muser live autoplay 5m` (MCP `live_autoplay`), and return to the loop.

## Changing tempo

- Write `"tempo": 140` in a section: from the start of this section it switches to 140 BPM, and later sections that do not write it stay there. Add `"tempo_ramp_bars": 4`:
  over the first 4 bars of this section it ramps from the current tempo (accelerando, ritardando; omitted or 0 means switching directly on the bar line).
  When this section loops it stays at the new tempo and does not restart each pass. `tempo` in `status` is the current tempo (during a ramp, the value at that moment).
- For dancing, do not change too much at once (within +/-10 BPM, or use a 4-8 bar ramp); for a big style change (ballad -> drum & bass), switch directly with a fill.
- Do not change tempo while someone is singing along.

## Changing tuning

- A section can write `"tuning"` (same as the project's `tuning`, see SKILLS.md "Tuning"): from the start of this section the new tuning applies, later sections keep it; `{ "scale": "equal" }` goes back to equal temperament.
  Notes already sounding keep their pitch. Use it for styles whose scales are not 12-TET: maqam (`custom`, E and B -50), raga shruti (`custom` deviations, or `just`), gamelan (`scala`), Baroque pitch (`reference_hz` 415).
  Ornaments (meend, andolan, slides) still go in per-note `bend`, on top of the tuning.

### Perceived tempo change (BPM unchanged)

Without changing BPM it can still sound faster / slower: change **the note length the groove's accents fall on**:

- Perceived beat = `BPM x 1/4 / accent interval` (the interval as a fraction of a whole note; a quarter note = 1/4).
  Example: at 120 BPM, with an accent every dotted quarter (3/8) -> 120 x (1/4) / (3/8) = 80; an accent every eighth (1/8) -> 240 (double-time feel).
- Common ratios: 2:1 (half / double speed: move the snare from beats 2 and 4 to beat 3, or make the drum hits twice as dense), 3:2 (triplet / dotted groove), 4:3.
- The time signature itself does not change the speed (7/8 is just a bar half a beat shorter); changing the time signature and accent positions together is what gives "a different feel of tempo".

## Ending

- The user says "end gradually": close with two or three sections -- each removes some layers, thins the groove, lengthens the notes (a ritardando by ear), and the last section is a long tonic chord,
  while on `master` you write a fade-out of `master_gain_db` from 0 down to about -40, and the reverb can gradually increase in `buses`.
- Wait for the last section to finish playing (`segment.seconds_to_end` in `status`), then `muser live stop` (MCP `live_stop`): the sound stops, the recorded project is written, and the page shows "Performance ended".
  If the user says "stop right now", just `stop`.
- Tell the user where the recorded project is: it can be exported with `muser render session.muser.json -o live.mp3`, or edited further with `muser view`;
  the `session.changes.jsonl` beside it holds the original text of every submission and the bar where it took effect, for review.

## When something goes wrong

- `send` errors: fix as the error message says (most often: changed the instrument / parameters of an existing track, deleted an effect, or referenced a bus that does not exist), then resubmit; the music is unaffected.
- `warning` in `status` / `wait` (a dropout since you last looked): look at `slow` (where the playback thread is slow: rendering a block, swapping in the new project, or being delayed by the system)
  and `load.heaviest` (the most resource-hungry tracks, in number of cores). If a track is too heavy, let it rest or swap in a lighter instrument in the next section; if "delayed by the system", it is mostly other programs busy on the computer; tell the user.
  Resting tracks (no clips, and the tail is gone) use no CPU; do not delete tracks to save resources.
- Cannot connect: `muser live status` reports "no live session in progress" -- the process has stopped; ask the user whether to start again.
