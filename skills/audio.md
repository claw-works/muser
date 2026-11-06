# Audio tracks: play recordings / sample files directly

Vocals, guitar recordings, ready-made loops, sound effects, a whole backing track -- no need to convert to notes; put them straight on a track and mix them with the other tracks.

```json
{ "name": "vocal", "instrument": "audio", "gain_db": -2,
  "effects": [{ "type": "eq", "shape": "highpass", "frequency": 120 }, { "type": "compressor", "threshold_db": -20, "ratio": 3 }],
  "sends": { "reverb": -10 },
  "clips": [
    { "start": 8,  "length": 16, "audio": { "file": "vocal.wav", "offset": 1.5, "fade_out": 0.5 } },
    { "start": 32, "length": 8,  "audio": { "file": "vocal.wav", "offset": 30, "fade_in": 0.05, "gain_db": -3 } }
  ] }
```

- Write `"instrument": "audio"` on the track; every clip of it carries `audio` and no notes (notes / abc / grid / drummer / pedal).
  Audio clips cannot be placed on an instrument track, and clips of an instrument track cannot mix in audio.
- After that it is like any other track: `effects`, `gain_db`, `pan`, `output`, `sends`, `sidechain` (it can be a trigger source, and can also be ducked by other tracks),
  `automation` of `gain_db` / `pan` / `sends` / effects, `mix-report`, and the faders in `muser view` all work as usual. An audio track has no instrument parameters.

## Fields

| Field | Unit | Default | Meaning |
|---|---|---|---|
| `file` | path | required | The audio file. A relative path is relative to the **directory of the project file**; an absolute path or `~/` also works |
| `offset` | seconds | 0 | Which second of the file to start playing from (trims the beginning) |
| `gain_db` | dB | 0 | Clip gain (-60 to 24), before the track fader |
| `fade_in` | seconds | 0.005 | Fade-in |
| `fade_out` | seconds | 0.005 | Fade-out, ending at the clip end (the clip length or the end of the file, whichever comes first) |

- The clip's `start` / `length` are still in **beats**: the file starts playing at `start` and stops at `length`; if the file is shorter than the clip, it ends early.
- Times inside the file (`offset`, fades) are in **seconds**. It plays at the original speed: **it is not stretched with tempo and not pitch-shifted**. After a tempo change, the start of the audio still lands on
  the same beat, but its own length (in seconds) does not change -- beat-aligned loop material must be at the same tempo as the project.
- Formats: WAV (including RF64), AIFF, FLAC, Ogg Vorbis, Opus, MP3; on macOS / Windows also AAC and ALAC (m4a, through the system decoder, not supported on Linux).
  Sample rates are converted automatically, and mono is placed on both sides.

## Look at the file before writing the clip

```bash
muser audio vocal.wav --tempo 92      # vocal.wav: Wav, 48.250 seconds, 44100 Hz, 2 channels, 73.983 beats at 92 bpm
muser audio vocal.wav --json
```

Write `length` as the number of beats you want it to play for; to play the whole file, write the beats converted from the file duration (given by `--tempo`).

## Tips

- A recording usually already has its own loudness: first look at its loudness relative to the other tracks with `mix-report`, then adjust `gain_db`, rather than going by feel.
- Commonly for vocal / recording tracks: `eq` highpass 80-150 Hz to remove low rumble, light compression, and a little send to the reverb bus.
- Cutting a passage: place several clips of the same file, use `offset` to choose where to start and `length` to choose how long; give the joins a `fade_in` / `fade_out` of a few tens of milliseconds to avoid clicks.
- To add effects only to certain phrases: effects hang on the track, not on the clip. For whole-phrase processing (echo, flanger, telephone), put those phrases on another audio track
  and hang its own effect chain on it; for a throw at one moment only (phrase-end reverb, echo), on the original track use `automation` to move `sends.<bus>` or the `mix` of `effects[i]`.
  Example: [samples/audio/spoken-groove](../samples/audio/spoken-groove.muser.json).
- A drum loop file can be used as a sidechain trigger source: `"sidechain": "<audio track name>"`.

## Common vocal / recording effects (effects on an audio track)

To apply an effect to only certain phrases: put those phrases on a separate audio track (see the item above). Example: [samples/audio/voice-fx](../samples/audio/voice-fx.muser.json).

| Effect | effects (in order) | Key points |
|---|---|---|
| Radio / walkie-talkie | `eq` highpass 500 x2, `eq` lowpass 3500 x2, `eq` peak 1600 Hz +8 dB (q 1.2), `compressor` -30 dB 8:1 fast attack fast release + makeup | Two stacked filters are steeper; keep only the mids (< 300 Hz and > 4 kHz are both about 1%). A telephone is narrower: 300-3000 Hz |
| Underwater | `eq` lowpass 1000 Hz (q 1.4, plus another at q 0.7), `chorus` 0.6 Hz, 12 ms, mix 0.6, `reverb` mix 0.45, decay 2.5, damping 0.9, and raise `gain_db` | Muffled, slow wobble, dark tail. Do not set the lowpass below ~800 Hz: when all the energy is below 300 Hz, laptop / phone speakers can hardly reproduce it, and it sounds like there is no sound |
| Tunnel / big space | `eq` highpass 150, `delay` 0.09 s, feedback 0.45, mix 0.35, `reverb` mix 0.55, decay 4.5, damping 0.3, size 0.95 | Short echo over a long reverb; the reverb should be long and bright (low damping) |
| Distant / behind a wall | `eq` lowpass 1200-2000, `eq` highpass 200, `reverb` mix 0.5 | Lowering `gain_db` by 6-10 dB sounds more distant |
| Distortion / loudspeaker | The built-in effects have no distortion: use Airwindows `Drive` / `Loud` / `Mackity`, or [Plasma](plugins/plasma.md), followed by the radio EQ | |

### More sound effects: Airwindows Consolidated

With [Airwindows Consolidated](plugins/airwindows-consolidated.md) installed (free, about 470 effects, use the CLAP version) there are far more sound effects to choose from.
One `plugin` effect entry is one effect, `"effect"` selects the name, and parameters are 0-1; first look at the parameter names with `muser plugin params` and audition with `muser audition`:

```json
{ "type": "plugin", "path": "~/.clap/Airwindows Consolidated.clap", "params": { "effect": "Drive", "Drive": 0.6, "Dry/Wet": 1.0 } }
```

| Want | Effect |
|---|---|
| Loudspeaker, walkie-talkie breakup | `Drive`, `Loud`, `Mackity`, `HardVacuum`, followed by the radio EQ |
| Old radio / grain of old recordings | `DeRez5`, `Pockey2`, `BitGlitter`, `GrooveWear`, `ToVinyl4` |
| Tape wobble, unstable transport | `Flutter2`, `Vibrato` (the wobble for underwater can also use it in place of chorus) |
| Robot / monster voice, pitch shifting | `PitchNasty`, `GlitchShifter` (`Note` in semitones), `Gringer` |
| Signal cutting in and out, walkie-talkie switching | `TremoSquare` (square-wave switching, only at zero crossings) |
| Cathedral, hall, space | `kCathedral5`, `Galactic` (on a track, lower Dry/Wet to 0.2-0.35) |
| Blur, dream | `CloudCoat`, `TakeCare` |

Without the plugin installed, validation reports "plugin not found": first confirm with `muser plugin scan`, or fall back to the built-in recipes above.

- Filters and compression change loudness: after adding effects, run `mix-report` and bring each to roughly the same loudness before comparing.
- For an effect that appears only at one moment (the end of a phrase going distant, suddenly entering a tunnel): on the original track use `automation` to move the `mix` of `effects[i]` or the `frequency` of an eq.
