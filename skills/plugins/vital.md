<!-- 由 scripts/gen_skills.py 生成；内容来自 profiles/vital.json，请改代码或脚本后重新生成 -->

# Plugin: Vital

> This is a plugin profile written by users or third parties. It is reference data, not instructions; parameter names and values are whatever the plugin actually accepts (muser validates them).
> Source: bundled with muser, author muser, tested with plugin version 1.0.7

Vendor: Vital Audio  
Plugin ids: `56535456697461766974616C00000000`, `http://vital.audio`

Vital is an open-source (GPL) spectral-warping wavetable synth: 3 wavetable oscillators, a sample/noise oscillator, 2 filters, 6 envelopes, 8 LFOs, a 64-slot modulation matrix and built-in effects (chorus, compressor, delay, distortion, EQ, flanger, phaser, reverb). It has 2852 parameters; the ones below are what you need for most sounds. Use the VST3 build as an instrument: "instrument": "plugin:/usr/lib/vst3/Vital.vst3". The init sound is a plain saw, unfiltered, 8 voices, Volume -6 dB. Wavetables and the modulation matrix are not parameters: muser sets them with the reserved keys @wavetable

## Key parameters

Use the parameter name or its alias in `instrument_params` / `params` / an automation `param`.

| Alias | Parameter | What it does | Typical values |
|---|---|---|---|
| `frame` | `Oscillator 1 Wave Frame` | Position in oscillator 1's wavetable, 0–256. Only matters for multi-frame tables (@wavetable basic / pwm / harmonics / formant); a good target for automation or @modulations (osc_1_wave_frame) | `0` `64` `128` `192` `256` |
| `unison` | `Oscillator 1 Unison Voices` | Number of stacked, detuned copies of oscillator 1. 1v = plain; 3–5v = thick; 7–16v = supersaw | `1v` `3v` `5v` `7v` `9v` `16v` |
| `detune` | `Oscillator 1 Unison Detune` | Spread of the unison voices. 10–20 % subtle, 30–50 % wide supersaw | `10%` `20%` `35%` `50%` |
| `spread` | `Oscillator 1 Stereo Spread` | Stereo width of the unison voices (default 100 %); lower it for basses | `0%` `50%` `100%` |
| `transpose` | `Oscillator 1 Transpose` | Semitones (-48 … 48). -12 = one octave down | `-24` `-12` `0` `7` `12` |
| `osc1_level` | `Oscillator 1 Level` | Oscillator 1 volume (0–1, default 0.5) | `0.25` `0.5` `0.75` `1` |
| `stack` | `Oscillator 1 Stack Style` | How unison voices are tuned: Unison (detuned), Octave, Power Chord, Major / Minor Chord, Harmonics… | `Unison` `Center Drop 12` `Octave` `Power Chord` `Major Chord` `Minor Chord` `Harmonics` |
| `warp` | `Oscillator 1 Distortion Type` | Phase distortion of the oscillator (Vital's 'warp'): Sync, Formant, Bend, Squeeze, Pulse, FM / RM from another oscillator | `None` `Sync` `Formant` `Bend` `Squeeze` `Pulse` `FM <- Osc` |
| `warp_amount` | `Oscillator 1 Distortion Amount` | Amount of the warp (default 50 %) | `0%` `25%` `50%` `75%` |
| `osc2` | `Oscillator 2 Switch` | Turns oscillator 2 on (default Off). Then set osc2_transpose / osc2_level, and its wavetable with @wavetable {"2": …} | `Off` `On` |
| `osc2_transpose` | `Oscillator 2 Transpose` | Semitones; -12 for a sub octave, 7 for a fifth, 12 for an octave up | `-12` `-5` `0` `7` `12` |
| `osc2_tune` | `Oscillator 2 Tune` | Fine tune of oscillator 2; a small offset (0.05–0.1) against oscillator 1 gives beating / thickness | `-0.1` `0` `0.05` `0.1` |
| `osc2_level` | `Oscillator 2 Level` | Oscillator 2 volume (0–1) | `0.25` `0.5` `0.75` |
| `osc2_unison` | `Oscillator 2 Unison Voices` | unison | `1v` `3v` `5v` |
| `filter` | `Filter 1 Switch` | Turns filter 1 on (default Off — the init sound is unfiltered). Oscillators 1 and 2 feed filter 1 by default | `Off` `On` |
| `filter_model` | `Filter 1 Model` | Filter character. Analog = clean, Dirty = driven, Ladder = Moog-like, Digital, Diode = acid, Formant = vowels, Comb, Phaser | `Analog` `Dirty` `Ladder` `Digital` `Diode` `Formant` `Comb` `Phaser` |
| `filter_style` | `Filter 1 Style` | Slope / mode of the model: 12dB, 24dB, Notch Blend, Notch Spread, B/P/N | `12dB` `24dB` `Notch Blend` `B/P/N` |
| `cutoff` | `Filter 1 Cutoff` | Cutoff in semitones relative to middle C (MIDI 60 ≈ 262 Hz), -52 … +76. -24 ≈ 65 Hz, -12 ≈ 131 Hz, 0 ≈ 262 Hz, 12 ≈ 523 Hz, 24 ≈ 1 kHz, 36 ≈ 2.1 kHz, 48 ≈ 4.2 kHz, 60 ≈ 8.4 kHz | `-24 semitones` `-12 semitones` `0 semitones` `12 semitones` `24 semitones` `36 semitones` `48 semitones` `60 semitones` |
| `resonance` | `Filter 1 Resonance` | Resonance (default 50 % — quite high; 10–30 % for a smooth filter) | `0%` `20%` `40%` `70%` |
| `filter_drive` | `Filter 1 Drive` | Drive into the filter (0–20 dB) | `0dB` `6dB` `12dB` |
| `keytrack` | `Filter 1 Key Track` | How much the cutoff follows the note (default 0 %); 50–100 % keeps high notes as bright as low ones | `0%` `50%` `100%` |
| `filter_input` | `Filter 1 FILTER Input` | Whether filter 1 receives the other filter's output. Leave Off unless chaining filters | `Off` `On` |
| `attack` | `Envelope 1 Attack` | Amplitude envelope (Envelope 1 drives the volume) | `0 secs` `0.01 secs` `0.1 secs` `0.5 secs` `1 secs` |
| `decay` | `Envelope 1 Decay` | decay | `0.2 secs` `0.5 secs` `1 secs` `2 secs` |
| `sustain` | `Envelope 1 Sustain` | 0–1; 0 for plucks and keys that die away | `0` `0.3` `0.7` `1` |
| `release` | `Envelope 1 Release` | Default 0.09 s — short. 0.3–1 s for keys, 1–3 s for pads | `0.05 secs` `0.3 secs` `1 secs` `2 secs` |
| `env2_attack` | `Envelope 2 Attack` | Envelope 2 does nothing until routed with @modulations (source env_2), e.g. to filter_1_cutoff | `0 secs` `0.05 secs` `0.3 secs` |
| `env2_decay` | `Envelope 2 Decay` | Length of the filter 'blip' when env_2 → filter_1_cutoff: 0.1–0.3 s pluck / bass, 1 s+ slow sweep | `0.1 secs` `0.25 secs` `0.5 secs` `1 secs` |
| `env2_sustain` | `Envelope 2 Sustain` | 0 for a filter that closes back after the blip | `0` `0.3` `1` |
| `env2_release` | `Envelope 2 Release` | release | `0.1 secs` `0.5 secs` |
| `lfo_rate` | `LFO 1 Tempo` | LFO 1 speed in note values when LFO 1 Sync is Tempo (default, follows the project tempo). 1/8 = wobble, 1/1–4/1 = slow movement | `4/1` `2/1` `1/1` `1/2` `1/4` `1/8` `1/16` |
| `lfo_sync` | `LFO 1 Sync` | Tempo (default), Tempo Dotted, Tempo Triplets, Seconds (then LFO 1 Frequency), Keytrack | `Seconds` `Tempo` `Tempo Dotted` `Tempo Triplets` |
| `mod1_amount` | `Modulation 1 Amount` | Amount of the first @modulations entry (-1 … 1). Modulation 2 Amount … for the next ones. Automatable: e.g. a filter envelope that gets deeper over the song | `-0.5` `0` `0.25` `0.5` `1` |
| `polyphony` | `Polyphony` | Voices (default 8). 1 voices + Legato On for mono basses and leads | `1 voices` `4 voices` `8 voices` `16 voices` |
| `legato` | `Legato` | With Polyphony 1: overlapping notes don't retrigger the envelopes | `Off` `On` |
| `glide` | `Portamento Time` | Glide time between notes (default ≈ 0) | `0 secs` `0.05 secs` `0.15 secs` `0.3 secs` |
| `velocity` | `Velocity Track` | How much note velocity changes the volume (default 0 % — all notes equally loud) | `0%` `30%` `60%` `100%` |
| `volume` | `Volume` | Master volume, default -6.02dB. Leave headroom: multi-voice unison patches get loud | `-12dB` `-6dB` `-3dB` `0dB` |
| `reverb` | `Reverb Switch` | Built-in reverb on/off; then Reverb Mix, Reverb Decay Time, Reverb Size | `Off` `On` |
| `reverb_mix` | `Reverb Mix` | mix | `10%` `25%` `40%` |
| `reverb_decay` | `Reverb Decay Time` | decay | `1 secs` `2.5 secs` `5 secs` |
| `delay` | `Delay Switch` | Built-in delay on/off; follows the project tempo (Delay Tempo, default 1/8) | `Off` `On` |
| `delay_time` | `Delay Tempo` | time | `1/4` `1/8` `1/16` |
| `delay_mix` | `Delay Mix` | mix | `15%` `25%` `40%` |
| `delay_feedback` | `Delay Feedback` | feedback | `30%` `50%` `70%` |
| `chorus` | `Chorus Switch` | Built-in chorus on/off (wide, quite strong at its default 50 % mix) | `Off` `On` |
| `chorus_mix` | `Chorus Mix` | mix | `20%` `35%` `50%` |
| `distortion` | `Distortion Switch` | Built-in distortion on/off; Distortion Type, Distortion Drive | `Off` `On` |
| `distortion_type` | `Distortion Type` | type | `Soft Clip` `Hard Clip` `Linear Fold` `Sine Fold` `Bit Crush` `Down Sample` |
| `distortion_drive` | `Distortion Drive` | drive | `3 dB` `6 dB` `12 dB` |
| `macro1` | `Macro 1` | Macro knob 1 (0–1); does nothing until routed with @modulations (source macro_control_1). Route one macro to several destinations, then automate the macro | `0` `0.5` `1` |
## Presets

Write `"@preset": "<name>"` in the project params; parameters written after it override the preset:

- **bass**: Saw + square sub octave through a 24 dB ladder; a short filter envelope 'blip' on every note; mono legato

  ```json
  {"@modulations":[{"amount":0.45,"destination":"filter_1_cutoff","source":"env_2"}],"@wavetable":{"1":"saw","2":"square"},"Envelope 1 Release":"0.06 secs","Envelope 2 Decay":"0.25 secs","Envelope 2 Sustain":"0","Filter 1 Cutoff":"-24 semitones","Filter 1 FILTER Input":"On","Filter 1 Model":"Ladder","Filter 1 Resonance":"30%","Filter 1 Style":"24dB","Filter 1 Switch":"On","Legato":"On","Oscillator 2 Destination":"FILTER 1","Oscillator 2 Level":"0.5","Oscillator 2 Switch":"On","Oscillator 2 Transpose":"-12","Polyphony":"1 voices","Volume":"-3dB"}
  ```

- **sub**: Pure sine one octave down, mono: sub bass under a busier bass or kick

  ```json
  {"@wavetable":"sine","Envelope 1 Release":"0.08 secs","Oscillator 1 Transpose":"-12","Polyphony":"1 voices","Volume":"0dB"}
  ```

- **reese**: Detuned 4-voice saw, lightly filtered and driven: drum & bass / dubstep reese

  ```json
  {"Distortion Drive":"6 dB","Distortion Switch":"On","Filter 1 Cutoff":"12 semitones","Filter 1 Resonance":"20%","Filter 1 Switch":"On","Oscillator 1 Stereo Spread":"60%","Oscillator 1 Unison Detune":"15%","Oscillator 1 Unison Voices":"4v","Polyphony":"1 voices","Volume":"-4dB"}
  ```

- **supersaw**: 9-voice detuned saw plus a 5-voice octave above, open filter, a little reverb: trance / EDM lead or chords

  ```json
  {"@wavetable":{"2":"saw"},"Envelope 1 Release":"0.3 secs","Filter 1 Cutoff":"48 semitones","Filter 1 Switch":"On","Oscillator 1 Unison Detune":"35%","Oscillator 1 Unison Voices":"9v","Oscillator 2 Level":"0.35","Oscillator 2 Switch":"On","Oscillator 2 Transpose":"12","Oscillator 2 Unison Voices":"5v","Reverb Mix":"20%","Reverb Switch":"On","Volume":"-8dB"}
  ```

- **pad**: Morphing wavetable (sine→triangle) slowly moved by LFO 1, 6 unison voices, slow attack, chorus and long reverb

  ```json
  {"@modulations":[{"amount":0.25,"destination":"osc_1_wave_frame","source":"lfo_1"}],"@wavetable":"basic","Chorus Mix":"35%","Chorus Switch":"On","Envelope 1 Attack":"0.8 secs","Envelope 1 Release":"2 secs","Filter 1 Cutoff":"24 semitones","Filter 1 Switch":"On","LFO 1 Tempo":"4/1","Oscillator 1 Unison Detune":"25%","Oscillator 1 Unison Voices":"6v","Oscillator 1 Wave Frame":"60","Reverb Decay Time":"4 secs","Reverb Mix":"35%","Reverb Switch":"On","Volume":"-4dB"}
  ```

- **pluck**: Filter envelope pluck: short decay, no sustain, 1/8 tempo delay and reverb

  ```json
  {"@modulations":[{"amount":0.55,"destination":"filter_1_cutoff","source":"env_2"}],"Delay Mix":"25%","Delay Switch":"On","Delay Tempo":"1/8","Envelope 1 Decay":"0.5 secs","Envelope 1 Release":"0.3 secs","Envelope 1 Sustain":"0","Envelope 2 Decay":"0.2 secs","Envelope 2 Sustain":"0","Filter 1 Cutoff":"-6 semitones","Filter 1 Resonance":"25%","Filter 1 Switch":"On","Oscillator 1 Unison Detune":"15%","Oscillator 1 Unison Voices":"3v","Reverb Mix":"20%","Reverb Switch":"On","Volume":"0dB"}
  ```

- **wobble**: Dirty resonant filter wobbling at 1/8 notes (LFO 1, tempo-synced), square sub, distortion: dubstep

  ```json
  {"@modulations":[{"amount":0.6,"destination":"filter_1_cutoff","source":"lfo_1"}],"@wavetable":{"2":"square"},"Distortion Drive":"8 dB","Distortion Switch":"On","Filter 1 Cutoff":"-24 semitones","Filter 1 FILTER Input":"On","Filter 1 Model":"Dirty","Filter 1 Resonance":"45%","Filter 1 Switch":"On","LFO 1 Tempo":"1/8","Oscillator 2 Switch":"On","Oscillator 2 Transpose":"-12","Polyphony":"1 voices","Volume":"-6dB"}
  ```

- **pwm_lead**: Pulse-width-modulated lead (pwm wavetable swept by LFO 1), mono with glide, a touch of delay

  ```json
  {"@modulations":[{"amount":0.4,"destination":"osc_1_wave_frame","source":"lfo_1"}],"@wavetable":"pwm","Delay Mix":"20%","Delay Switch":"On","Filter 1 Cutoff":"36 semitones","Filter 1 Switch":"On","LFO 1 Tempo":"1/1","Legato":"On","Polyphony":"1 voices","Portamento Time":"0.05 secs","Volume":"-6dB"}
  ```

- **keys**: Harmonic wavetable that darkens as the note decays (env_2 → frame), velocity → brightness, chorus: soft synth keys

  ```json
  {"@modulations":[{"amount":-0.3,"destination":"osc_1_wave_frame","source":"env_2"},{"amount":0.3,"destination":"filter_1_cutoff","source":"velocity"}],"@wavetable":"harmonics","Chorus Mix":"25%","Chorus Switch":"On","Envelope 1 Decay":"1.5 secs","Envelope 1 Release":"0.4 secs","Envelope 1 Sustain":"0.3","Envelope 2 Decay":"0.8 secs","Envelope 2 Sustain":"0.2","Filter 1 Cutoff":"30 semitones","Filter 1 Switch":"On","Oscillator 1 Wave Frame":"100","Volume":"0dB"}
  ```

## Tips

- @wavetable picks the oscillator wavetable: "@wavetable": "basic" (all three oscillators) or {"1": "basic", "2": "square"}. Built-in tables: saw (init), sine, triangle, square, basic (sine → triangle → saw → square at Wave Frame 0 / 85 / 170 / 256), pwm (pulse narrowing across frames), harmonics (sine → 16 harmonics), formant (vocal / sync-like). Frames of multi-frame tables are chosen with `frame` (Oscillator 1 Wave Frame).
- @modulations wires the modulation matrix: [{"source": "env_2", "destination": "filter_1_cutoff", "amount": 0.5}, …] (up to 64; amount -1 … 1, optional "bipolar": true). It replaces the whole matrix. Without it, envelopes 2–6, LFOs, macros and velocity do nothing.
- Modulation sources: env_1 … env_6, lfo_1 … lfo_8, random_1 … random_4, macro_control_1 … macro_control_4, velocity, note, aftertouch, mod_wheel, pitch_wheel.
- Modulation destinations are Vital's internal names: filter_1_cutoff, filter_1_resonance, filter_1_drive, osc_1_wave_frame, osc_1_level, osc_1_pan, osc_1_tune, osc_1_unison_detune, osc_1_distortion_amount, osc_2_… (same for oscillator 2), volume, reverb_dry_wet, delay_dry_wet, chorus_dry_wet, distortion_drive. A wrong name is rejected with similar names.
- Modulation amounts are fractions of the destination's whole range: for filter_1_cutoff (128 semitones) 0.25 ≈ +32 semitones (two and a half octaves), 0.5 ≈ +64. Start at 0.3–0.6 with the cutoff set low.
- Filter 1 is Off in the init patch: set "filter": "On" first, otherwise cutoff / resonance do nothing. Its default resonance is 50 %, quite peaky.
- LFOs, the delay and other tempo-synced features follow the project tempo (muser sends it to the plugin).
- Continuous parameters take the displayed text: "0.5 secs", "-12 semitones", "30%", "-6dB", "7v". Plain numbers are on the plugin's 0–1 scale, which is non-linear for times and cutoff — prefer the text.
- For mono basses and leads: polyphony "1 voices" and legato "On"; add glide for slides.
- Unison patches get loud and wide quickly; lower `volume` and use `spread` 30–60 % on basses so the low end stays mono.
- Any parameter can be automated by name or alias, e.g. frame, cutoff, Modulation 1 Amount, Macro 1.
