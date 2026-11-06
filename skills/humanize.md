# Humanized playing (humanize)

A machine-like "every note on the grid, all at the same velocity" sounds stiff. `humanize` adds human-like offsets to the timing, velocity and duration of notes,
plus groove (swing, overall push ahead / lay back). Written on a **track** it is the default for the whole track, and written on a **clip** it overrides the same fields of the track; write only the items you want.

```json
{ "name": "hats", "instrument": "drums.909",
  "humanize": { "timing_ms": 6, "drift": 0.6, "velocity": 0.08, "swing": 58, "swing_unit": 0.25 },
  "clips": [ … ] }
```

| Field | Meaning | Typical values |
|---|---|---|
| `timing_ms` | Standard deviation of the timing offset (milliseconds, normal distribution, limited to within +/-3 sigma) | drums 3-8; keyboards, guitar 5-12; relaxed lo-fi 10-20 |
| `drift` | How coherent the offsets are: 0 means each note is random on its own; closer to 1 is more like a human drifting slowly (sometimes fast, sometimes slow) | 0.5-0.8 sounds most human; 0 sounds like jitter |
| `velocity` | Standard deviation of the velocity offset (velocity 0-1) | 0.03-0.12 |
| `length` | Standard deviation of the duration change (as a ratio) | 0-0.15 |
| `swing` | Swing (%): 50 is straight; 66.7 is a triplet feel; notes on the off-beats are pushed later | house / hip-hop 54-62; shuffle 66 |
| `swing_unit` | Unit of the swing (beats): 0.5 eighth notes (default), 0.25 sixteenth notes | use 0.25 for hats and 16th-note rhythms |
| `push_ms` | Push the whole track ahead (negative) or lay it back (positive) | snare lay back +5 to +15; hurried bass -5 |
| `seed` | Random seed. If omitted, it is derived from the project `seed` and the track and clip positions | — |

- **The same seed renders exactly the same every time**: when you hear a take you like, you can keep it (write the seed into `humanize.seed` to fix it).
- **Try another take**: change the project `seed` (MCP `set_seed`), or temporarily change it while auditioning with `muser play song.muser.json --seed 7` (`render` also has `--seed`), without changing the file.
- Notes pressed at the same moment (chords) share one timing offset, with velocities varying individually; swing is computed on the project's beat grid (aligned wherever the clip starts).
- Quantized swing is "groove", not randomness: only a uniform swing across the whole piece (the same `swing` / `swing_unit`) makes the tracks lock together;
  random offsets (`timing_ms`) can differ per track, smallest for drums and slightly larger for harmony instruments.

## Starting points

| Want | humanize |
|---|---|
| Drums: less mechanical but still tight | `{ "timing_ms": 4, "drift": 0.6, "velocity": 0.06 }` |
| Relaxed hip-hop / lo-fi drums | `{ "timing_ms": 10, "drift": 0.7, "velocity": 0.1, "swing": 60, "swing_unit": 0.25 }`, and add `"push_ms": 12` on the snare track |
| House hats | `{ "timing_ms": 3, "velocity": 0.08, "swing": 56, "swing_unit": 0.25 }` |
| Played piano, guitar | `{ "timing_ms": 8, "drift": 0.5, "velocity": 0.08, "length": 0.08 }` |
| Pad bed, long notes | Generally not needed; at most `{ "timing_ms": 15 }` so the notes in a chord are not perfectly simultaneous |

Precise electronic music (trance arpeggios, techno 16th hats) usually needs no random offsets; use only `swing`, or none.
