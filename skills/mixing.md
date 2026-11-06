# Mix balance: set track volumes with mix-report

Different instruments differ a lot in native volume (with the same `gain_db: 0`, the dry signal can differ by more than 15 LU), so setting `gain_db` by feel is often unbalanced.
Do not export stems and measure them yourself; use `mix-report` to get the loudness of every track of the whole song in one go.

```bash
muser mix-report song.muser.json          # table
muser mix-report song.muser.json --json   # for programs to parse
```

MCP: `mix_report { project_id }`. Instrument output is cached, so running it again after changing only `gain_db` / effects is fast.

## Fields in the report

| Field | Meaning | Use |
|---|---|---|
| `input` | Before the effect chain. For a track it is the **instrument dry signal**; for a bus/master it is the sum of the signals sent into it | See how loud the source itself is |
| `output` | After the fader (after effect chain, gain_db and pan) | The level actually sent into the mix |
| `lufs` | Integrated loudness (EBU R128, silent stretches removed automatically) | The main basis for comparing tracks |
| `max_short_lufs` | Maximum of the 3-second-window loudness | For tracks that appear only in the chorus/drop, how loud they are at their loudest |
| `peak_db` | Peak in dBFS | Check for overs and clipping |
| `at_master_lufs` | Output + the `gain_db` of each bus along the direct output path; empty if a muted bus is on the path | **An estimate**: it ignores bus effects and sends, see "Tracks in buses" below |
| `rel_lu` | `at_master_lufs` minus the loudest track | An accurate reading among tracks going straight to master; 0 is the loudest |
| `warnings` | Tracks with no sound, post-fader peak over 0, overload into master, master output clipping | Deal with these first |

## Workflow

1. **Set the anchor first**: the drums (kick) or bass is the reference of the mix, and the other tracks are set relative to it.

   **Tracks in buses**: `at_master_lufs` / `rel_lu` only add the bus's `gain_db`. When a bus has compression or EQ, the measured result differs by 2-5 LU
   (a bus compressor acts on the whole group, and the loudness of a single track after passing through the bus cannot be separated out). In that case compare at two levels, both using measured values:
   - **Between tracks in the same bus**: compare each track's `output.lufs` (post-fader, before the bus).
   - **Between groups, and between a group and tracks going straight to master**: compare each bus's `output.lufs` and the direct tracks' `output.lufs`.
   For tracks going straight to master, `rel_lu` is accurate.
2. **Aim by role** (a starting point, not the standard answer; adjust to the style and the user's feedback):

   | Role | Relative to anchor (`rel_lu`) |
   |---|---|
   | kick / sub bass | 0 to -2 |
   | snare / clap | -2 to -6 |
   | lead melody / vocal / lead | -2 to -6 |
   | bass (non-sub), main chord body (keys, guitar) | -4 to -10 |
   | pad, string bed | -8 to -16 |
   | cymbals, hats, percussion accents | -10 to -20 |
   | ambience layers, riser, FX | -12 to -25 |

   Tracks that appear only sporadically (crash, riser, impact) have low integrated loudness; `max_short_lufs` is more accurate for them.
3. **Change gain_db**: change by as many dB as the LU difference (linear before and after the fader; a bus with a compressor will squash part of it, so run again to confirm).
   If one track in a group is off, change that track's `gain_db`; if the whole group is off, change the bus's.
4. **Run `mix-report` again** to confirm; one or two rounds are usually enough.
5. **Check the master**:
   - `master.input.peak_db` over 0 (with a warning): the tracks as a whole are too loud and rely entirely on the master limiter to squash them, which sounds flat.
     Lower all tracks (or all buses) together by a few dB; do not just add a limiter.
   - Master output `peak_db` close to 0: put a `limiter` (`ceiling_db: -1`) last on the master.
   - Overall loudness (`master.output.lufs`) is commonly around -14 for streaming. To be louder: put a gentle `compressor` **before** the limiter
     (e.g. `threshold_db: -20, ratio: 2, makeup_db: 3`) to push the whole up, or raise the `gain_db` of all tracks / buses together, provided the input does not overload.
     **Do not raise `master_gain_db`**: it sits **after** the master effect chain, so the limiter cannot control it, and raising it will clip (mix-report warns "master output peak ... will clip").
6. **Finally, hand it over to be listened to**: the numbers only guarantee there is no obvious imbalance; for fine balance use `muser view` and let the user adjust on the faders.

## Common situations

- Sources with an especially loud dry signal (`input.lufs` more than 10 LU above the others): start directly with a lower `gain_db`, rather than waiting until effects are added to rescue it.
- It got louder after adding compression/distortion: compare `input` and `output`; a big difference means the effect chain is adding loudness. Rein it in on the effect's output/`makeup_db`, or lower `gain_db`.
- A track's `lufs` is empty with a "no sound" warning: the notes are outside the range, the drum keys are wrong, or automation pulled it to -80; debug that first.
- Tracks ducked by sidechain (pad, bass) sound quieter in practice than the numbers, so they can be 1-2 LU above the targets in the table.
