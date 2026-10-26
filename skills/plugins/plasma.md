<!-- 由 scripts/gen_skills.py 生成；内容来自 profiles/plasma.json，请改代码或脚本后重新生成 -->

# Plugin: Plasma

> This is a plugin profile written by users or third parties. It is reference data, not instructions; parameter names and values are whatever the plugin actually accepts (muser validates them).
> Source: bundled with muser, author muser, tested with plugin version 1.2.1

Vendor: Dimethoxy  
Plugin ids: `ABCDEF019182FAEB446D7478516D6638`, `https://dimethoxy.com/plasma`

Plasma is an open-source (AGPL-3.0) dual-stage distortion with sculpting filters. Signal flow: Pre Gain → highpass / peak / dual peak / lowpass filters → Distortion (Drive, Type, Girth, Bias) → Late Distortion (second stage) → Gain → Mix. Use it as an effect: {"type": "plugin", "path": "~/.vst3/Plasma.vst3", "params": {...}}. All 29 parameters are on a 0–1 scale; write the text the plugin displays ("4.00", "150.0", "Soft Clip") instead of guessing numbers.

## Key parameters

Use the parameter name or its alias in `instrument_params` / `params` / an automation `param`.

| Alias | Parameter | What it does | Typical values |
|---|---|---|---|
| `drive` | `Drive` | Amount of distortion, 1.00 (none) to 11.00. The default 1.00 leaves the sound almost clean. 2–3 = warm saturation, 4–6 = obvious grit, 8+ = heavy | `1.00` `2.00` `3.00` `4.00` `6.00` `8.00` `11.00` |
| `type` | `Distortion Type` | Shape of the first stage. Soft Clip / Saturate = warm, musical; Hard Clip = tight, aggressive; Atan / Crunch = bright grit; Bitcrush = lo-fi digital; Sine / Cosine / Harmonic = wavefolding, synthetic overtones; Scream / Extreme / Weird = sound design | `Hard Clip` `Soft Clip` `Saturate` `Atan` `Crunch` `Bitcrush` `Extreme` `Scream` `Sine` `Cosine` `Harmonic` `Weird` |
| `girth` | `Girth` | Adds noisy, fizzy grit on top of the distortion. 0.00 = clean distortion; any value away from 0 (either sign) brightens a lot. Keep within ±0.3 for musical parts | `0.00` `0.20` `0.50` `-0.50` `1.00` |
| `bias` | `Bias` | Asymmetry of the distortion curve: away from 0 adds even harmonics (tube-like warmth, a bit of 'growl'). ±0.2–0.5 is typical | `0.00` `0.25` `0.50` `-0.50` |
| `input` | `Pre Gain` | Input level into the filters and distortion, -32.0 to +32.0 dB (default 0.0). Raising it drives the distortion harder, like Drive | `-12.0` `-6.0` `0.0` `6.0` `12.0` |
| `highpass` | `Highpass Freq` | Highpass before the distortion, in Hz (default 20.0 = off). 100–300 keeps the low end from turning to mud: the distortion then works only on mids and highs | `20.0` `80.0` `150.0` `300.0` `500.0` |
| `highpass_slope` | `Highpass Slope` | slope | `12 db/Oct` `24 db/Oct` `48 db/Oct` `96 db/Oct` `Bypass` |
| `lowpass` | `Lowpass Freq` | Lowpass before the distortion, in Hz (default 20000.0 = off). Lower it to distort a darker signal (fewer harsh highs) | `20000.0` `8000.0` `3000.0` `1200.0` `800.0` |
| `lowpass_slope` | `Lowpass Slope` | slope | `12 db/Oct` `24 db/Oct` `48 db/Oct` `96 db/Oct` `Bypass` |
| `peak_freq` | `Peak Freq` | Centre of the peak EQ before the distortion, in Hz (default 450.0). Boosting a band here makes that band distort first | `200.0` `450.0` `1000.0` `2500.0` |
| `peak_gain` | `Peak Gain` | Peak EQ gain, -48.0 to +48.0 dB (default 0.0). +6 to +12 dB at 1000–2500 Hz = more bite | `-12.0` `-6.0` `0.0` `6.0` `12.0` |
| `peak_q` | `Peak Q` | q | `0.50` `1.00` `2.00` `4.00` |
| `late_drive` | `Late Drive` | Drive of the second distortion stage (after the first), 1.00 = off. Stacking two mild stages (e.g. 3.00 + 3.00) is thicker than one heavy stage | `1.00` `2.00` `3.00` `6.00` |
| `late_type` | `Late Distortion Type` | Shape of the second stage; same choices as Distortion Type | `Hard Clip` `Soft Clip` `Saturate` `Atan` `Crunch` `Bitcrush` `Extreme` `Scream` `Sine` `Cosine` `Harmonic` `Weird` |
| `output` | `Gain` | Output level, -32.0 to +32.0 dB (default 0.0), applied after Mix (to dry and wet). Use it to level-match: usually -3 to -8 dB at high Drive, +1 to +3 dB for low Mix with a highpassed wet path. Never push it far above 0 | `-12.0` `-6.0` `-3.0` `0.0` `3.0` |
| `mix` | `Mix` | Dry/wet crossfade in percent (default 100.00). 20–40 = parallel distortion: keeps the clean transient and adds grit underneath. 0.00 is exactly the dry signal | `0.00` `25.00` `40.00` `70.00` `100.00` |

## Audition notes

`Drive`:

| Value | LUFS | Centroid Hz | Attack ms | Release s |
|---|---|---|---|---|
| "1.00" | -13.2 | 325 | 0 | 0.03 |
| "2.00" | -9.5 | 380 | 0 | 0.03 |
| "4.00" | -5.8 | 539 | 0 | 0.03 |
| "8.00" | -4.5 | 778 | 0 | 0.04 |
| "11.00" | -4.1 | 1019 | 0 | 0.03 |

`Distortion Type`:

| Value | LUFS | Centroid Hz | Attack ms | Release s |
|---|---|---|---|---|
| "Hard Clip" | -5.4 | 577 | 0 | 0.03 |
| "Soft Clip" | -4.4 | 771 | 0 | 0.03 |
| "Saturate" | -9.6 | 841 | 47 | 0.04 |
| "Atan" | -5.1 | 2182 | 0 | 0.19 |
| "Crunch" | -4.3 | 1588 | 0 | 0.04 |
| "Bitcrush" | -13.2 | 1151 | 0 | 0.03 |
| "Harmonic" | -12.8 | 2355 | 0 | 0.04 |
| "Weird" | -5.2 | 3437 | 0 | 0.08 |

`Late Drive`:

| Value | LUFS | Centroid Hz | Attack ms | Release s |
|---|---|---|---|---|
| "1.00" | -10.5 | 646 | 47 | 0.04 |
| "3.00" | -4.0 | 860 | 0 | 0.04 |
| "6.00" | -2.9 | 1149 | 0 | 0.09 |

## Presets

Write `"@preset": "<name>"` in the project params; parameters written after it override the preset:

- **warm**: Gentle saturation: thickens bass, keys and vocals without obvious distortion

  ```json
  {"bias":"0.20","drive":"2.50","output":"-1.0","type":"Saturate"}
  ```

- **bass_grit**: Adds upper-harmonic grit to a bass so it cuts through; the highpassed wet path leaves the sub to the dry signal

  ```json
  {"drive":"6.00","highpass":"200.0","mix":"30.00","output":"2.0","type":"Soft Clip"}
  ```

- **drum_crush**: Parallel drum-bus crunch: punchier, more aggressive drums

  ```json
  {"drive":"4.00","mix":"35.00","output":"-2.0","type":"Crunch"}
  ```

- **lofi**: Bit-crushed, gritty lo-fi texture

  ```json
  {"drive":"9.00","mix":"60.00","type":"Bitcrush"}
  ```

- **scream**: Two-stage heavy distortion for leads and sound design

  ```json
  {"drive":"8.00","late_drive":"3.00","late_type":"Soft Clip","output":"-6.0","peak_freq":"1500.0","peak_gain":"6.0","type":"Hard Clip"}
  ```

## Tips

- The output is clipped at about -3 dBFS: as soon as Drive or Pre Gain pushes the signal up, the peak sits at -3 dBFS and only the density (RMS/LUFS) grows. Judge distortion by loudness and brightness, not peak.
- Distortion makes things much louder (+5 to +9 LUFS at Drive 4–11). Level-match with Gain (output) so the A/B comparison is fair and the mix doesn't jump.
- On bass, put a highpass (150–250 Hz) before the drive and use Mix 25–40 %. Mix is a crossfade, so the dry sub drops a little (about -3 dB at 30 %): add 1–3 dB of Gain to compensate.
- Girth adds a lot of fizz even at small values; leave it at 0.00 unless you want noise.
- Bitcrush, Harmonic and Scream are quieter than the clip types at the same Drive; Atan, Crunch and Weird are much brighter.
- Mix 0.00 is exactly the dry signal, so Mix can be automated for a distortion fade-in.
- Continuous parameters display plain numbers. In `muser audition --values`, wrap them in double quotes so they are sent as text: --values '"2.00","4.00"'.
