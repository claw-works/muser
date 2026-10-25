<!-- 由 scripts/gen_skills.py 生成；内容来自 profiles/airwindows-consolidated.json，请改代码或脚本后重新生成 -->

# Plugin: Airwindows Consolidated

> This is a plugin profile written by users or third parties. It is reference data, not instructions; parameter names and values are whatever the plugin actually accepts (muser validates them).
> Source: bundled with muser, author muser, tested with plugin version 1.2026.298

Vendor: Airwindows  
Plugin ids: `com.airwindows.consolidated`, `ABCDEF019182FAEB44746872616C4658`, `https://airwindows.com//plugins/Airwindows_Consolidated`

Airwindows Consolidated packs the whole Airwindows collection (free, MIT-licensed effects by Chris Johnson) into one plugin: about 470 effects — reverbs, tape, saturation, consoles, filters, dynamics, lo-fi. Pick one with "effect" (see the list below), then set that effect's own parameters by name. Use the CLAP build: {"type": "plugin", "path": "~/.clap/Airwindows Consolidated.clap", "params": {"effect": "kPlate140", "Wetness": 0.35}}. Effect descriptions come from the Airwindows registry.

## Key parameters

Use the parameter name or its alias in `instrument_params` / `params` / an automation `param`.

| Alias | Parameter | What it does | Typical values |
|---|---|---|---|
| — | `Input Level` | Input trim in dB before the effect (all effects). Drives saturation / tape / console effects harder | `-6.00 dB` `0.00 dB` `3.00 dB` `6.00 dB` |
| — | `Output Level` | Output trim in dB after the effect (all effects); use it to level-match | `-6.00 dB` `-3.00 dB` `0.00 dB` `3.00 dB` |
| — | `Mono Behaviour` | How mono input is handled; leave at the default |  |
## Presets

Write `"@preset": "<name>"` in the project params; parameters written after it override the preset:

- **hall**: kRockstar2: realistic hall that blends well; good as a send-bus reverb at Dry/Wet 1

  ```json
  {"effect":"kRockstar2"}
  ```

- **cathedral**: kCathedral5: huge, long space

  ```json
  {"effect":"kCathedral5"}
  ```

- **plate**: kPlate140: modern plate, bright and smooth on vocals, snares, keys

  ```json
  {"Wetness":0.35,"effect":"kPlate140"}
  ```

- **room**: kStation: small realistic room, like David Bowie's vocal booth

  ```json
  {"effect":"kStation"}
  ```

- **chamber**: Chamber: echo-chamber reverb, bright, medium length

  ```json
  {"effect":"Chamber"}
  ```

- **space**: Galactic: super-reverb for pads and ambient; 100 % wet by default, so best on a send bus

  ```json
  {"Bigness":1.0,"Replace":0.6,"effect":"Galactic"}
  ```

- **tape_echo**: TapeDelay2: tape echo with filtered, decaying repeats

  ```json
  {"Dry/Wet":0.35,"Regen":0.4,"Time":0.4,"effect":"TapeDelay2"}
  ```

- **tape**: ToTape9: full analog tape colour (saturation, head bump, flutter) at defaults

  ```json
  {"effect":"ToTape9"}
  ```

- **tube**: Tube: tube saturation; thicker and louder (+3 dB)

  ```json
  {"TUBE":0.5,"effect":"Tube"}
  ```

- **density**: Density2: dense, fat saturation for bass and drums

  ```json
  {"Density":0.4,"effect":"Density2"}
  ```

- **warm**: PurestWarm2: subtle asymmetric warmth

  ```json
  {"Dry/Pos":0.5,"effect":"PurestWarm2"}
  ```

- **lofi**: DeRez5: retro sample-rate / bit reduction

  ```json
  {"Derez":0.5,"effect":"DeRez5"}
  ```

- **vinyl_wear**: GrooveWear: worn-record high-frequency loss

  ```json
  {"Wear":0.3,"effect":"GrooveWear"}
  ```

- **wide**: Srsly3: stereo widener/shaper

  ```json
  {"effect":"Srsly3"}
  ```

- **tremolo**: TremoSquare: on/off square tremolo

  ```json
  {"Freq":0.5,"effect":"TremoSquare"}
  ```

- **bus_colour**: BussColors4: console bus colour (Color picks the console)

  ```json
  {"effect":"BussColors4"}
  ```

## Choosing the effect

`effect` is not an ordinary parameter of this plugin: write `"effect": "<name>"` in `params` / `instrument_params` and muser switches to it and loads its parameter list (the parameter names change with it); after switching, parameters start at that choice's defaults. Switch first, then look at its parameters: `muser plugin params <plugin> --params '{"effect": "<name>"}'`.

Effects are grouped by Airwindows category. Within each group, effects tagged recommended / basic come first (Chris Johnson's picks), then the rest in his suggested order. Each entry lists the effect's parameters with their default display values. Almost all parameters are plain 0–1 numbers (display = value), so you can write numbers directly, e.g. "Wetness": 0.35. Only one effect per plugin instance: chain several plugin effects for more.

### Reverb

- `kRockstar2` _(recommended)_ — kRockstar2 is a realistic hall reverb using a new matrix! Params (default): Regen 0.5000, Positin 0.7500, DownRez 1.0000, Distnce 0.2500, Dry/Wet 0.5000.
- `kCyberCity` _(recommended)_ — kCyberCity brings live atmosphere to deep reverb. Params (default): Regen 0.5000, Positin 1.0000, Dry/Wet 0.5000.
- `kWoodRoom` _(recommended)_ — kWoodRoom is a small wooden performing space. Params (default): Regen 0.5000, Derez 0.5000, Filter 0.2500, EarlyRF 0.5000, Positin 0.7500, Dry/Wet 0.5000.
- `kStation` _(recommended)_ — kStation is a realistic small room modeled after David Bowie's vocal reverb. Params (default): Regen 0.5000, Derez 0.5000, Filter 0.5000, EarlyRF 0.5000, Positin 0.9000, Dry/Wet 0.5000.
- `kGuitarHall2` _(recommended)_ — kGuitarHall2 puts things in a Loud Room. Params (default): Regen 0.5000, Derez 0.5000, Filter 0.5000, EarlyRF 0.5000, Positin 0.5000, Dry/Wet 0.5000.
- `kCathedral5` _(recommended)_ — kCathedral5 lets you place yourself anywhere you like in the space. Params (default): Regen 0.5000, Derez 0.5000, Filter 0.2400, EarlyRF 0.5000, Positin 0.5000, Dry/Wet 0.5000.
- `VerbTiny` _(basic, recommended)_ — VerbTiny is a classic artificial reverb that expands reverb shape. Params (default): Replace 0.5000, Derez 1.0000, Filter 1.0000, Wider 0.0000, Dry/Wet 1.0000.
- `kAlienSpaceship` _(recommended)_ — kAlienSpaceship is an unreal realistic reverb. Params (default): Regen 0.5000, Derez 0.5000, Filter 0.5000, EarlyRF 0.5000, Positin 0.5000, Dry/Wet 0.5000.
- `kBeyond` _(recommended)_ — kBeyond is a recital hall. Params (default): Regen 0.5000, Derez 1.0000, Filter 0.5000, EarlyRF 0.5000, Predlay 0.0000, Dry/Wet 0.5000.
- `kCosmos` _(recommended)_ — kCosmos is infinite space ambient, or titanic hall. Params (default): Regen 0.5000, Derez 1.0000, Filter 0.5000, EarlyRF 0.5000, Predlay 0.0000, Dry/Wet 0.5000.
- `kPlate140` _(recommended)_ — kPlate140 is a next-generation Airwindows plate reverb. Params (default): Input 1.0000, Regen 0.5000, DeRez 1.0000, Predlay 0.0000, Wetness 0.2500.
- `kPlate240` _(recommended)_ — kPlate240 is for the texture of smaller, gold foil reverb. Params (default): Input 1.0000, Regen 0.5000, DeRez 1.0000, Predlay 0.0000, Wetness 0.2500.
- `CreamCoat` _(basic, recommended)_ — CreamCoat is a swiss army knife reverb with soft undersampling. Params (default): Select 8, Regen 0.5000, DeRez 1.0000, Predlay 0.0000, Wetness 0.2500.
- `CrunchCoat` _(recommended)_ — CrunchCoat is a cursed retro digital reverb! Params (default): Select 8, Regen 0.0000, DeRez 1.0000, Dry/Wet 1.0000.
- `Galactic` _(basic, recommended)_ — Galactic is a super-reverb designed specially for pads and space ambient. Params (default): Replace 0.5000, Brightness 0.5000, Detune 0.5000, Bigness 1.0000, Dry/Wet 1.0000.
- `Chamber` _(basic, recommended)_ — Chamber is a feedforward reverb based on the golden ratio. Params (default): Bigness 0.3500, Longness 0.3500, Liteness 0.3500, Darkness 0.3500, Wetness 0.3500.
- `NonlinearSpace` _(recommended)_ — NonlinearSpace is a flexible reverb plugin. Params (default): SmpRate 44.1K, Livenes 0.5000, Treble 0.5000, Bass 0.5000, Nonlin 0.0000, Dry/Wet 1.0000.
- `VerbSevens` — VerbSevens is a calibrated reference reverb plugin for reverb matrices. No parameters.
- `VerbThic` — VerbThic is a classic artificial reverb for being opaque and textured. Params (default): Replace 0.5000, Derez 1.0000, Filter 1.0000, Wider 0.0000, Dry/Wet 1.0000.
- `kCathedral4` — kCathedral4 is the Cathedral sound expressed as distant space. Params (default): Regen 0.5000, Derez 1.0000, Filter 0.5000, EarlyRF 0.5000, Predlay 0.0000, Dry/Wet 0.5000.
- `VerbSixes` — VerbSixes is a calibrated reference reverb plugin for Householder matrices. No parameters.
- `Galactic3` — Galactic3 is exactly Galactic again, but with better undersampling. Params (default): Replace 0.5000, Brightns 0.5000, Detune 0.5000, Derez 1.0000, Bigness 1.0000, Dry/Wet 1.0000.
- `kCathedral3` — kCathedral3 is a giant cathedral-like space using Bezier undersampling. Params (default): Top dB 105.0000 dB, Regen 0.5000, DeRez 1.0000, Predlay 0.0000, Wetness 0.2500.
- `kRockstar` — kRockstar is a realistic hall reverb that blends very well. Params (default): Regen 0.5000, Positin 1.0000, DownRez 1.0000, Dry/Wet 0.5000.
- `kGuitarHall` — kGuitarHall accentuates loud midrange skronk! Params (default): Top dB 105.0000 dB, Regen 0.5000, DeRez 1.0000, Predlay 0.0000, Wetness 0.2500.
- `kPlateD` — kPlateD is a plate reverb, not unlike its namesake atop Abbey Road. Params (default): InputPad 1.0000, Damping 5.0000, Low Cut 3.0000, Predelay 0.0000 sec, Wetness 0.2500.
- `kPlateB` — kPlateB is a plate reverb, not unlike its namesake atop Abbey Road. Params (default): InputPad 1.0000, Damping 5.0000, Low Cut 3.0000, Predelay 0.0000 sec, Wetness 0.2500.
- `kPlateA` — kPlateA is a plate reverb, not unlike its namesake atop Abbey Road. Params (default): InputPad 1.0000, Damping 5.0000, Low Cut 3.0000, Predelay 0.0000 sec, Wetness 0.2500.
- `kPlateC` — kPlateC is a plate reverb, not unlike its namesake atop Abbey Road. Params (default): InputPad 1.0000, Damping 5.0000, Low Cut 3.0000, Predelay 0.0000 sec, Wetness 0.2500.
- `kCathedral2` — kCathedral2 is a giant reverby space modeled after the Bricasti Cathedral. Params (default): Wetness 1.0000.
- `Verbity2` — Verbity2 adds stereo crossmodulation and expands Verbity's feedforward reverb topology. Params (default): RmSize 0.5000, Sustain 0.5000, Mulch 0.5000, Wetness 1.0000.
- `Galactic2` — Galactic2 is a super-reverb REdesigned specially for pads and space ambient. Params (default): Drive 0.5000, Sustain 0.5000, Darken 0.5000, Wetness 1.0000.
- `Ultralight2` — Ultralight2 is more of the most low-CPU matrix reverb I can make. Params (default): Damping 6, UnSolid 3, UnReflc 3, Predlay 538, Dry/Wet 0.5000, Distnce 0.0000.
- `Verbity` — Verbity is a dual-mono reverb, which uses feedforward reverb topology. Params (default): Bigness 0.2500, Longness 0.0000, Darkness 0.2500, Wetness 0.2500.
- `Chamber2` — Chamber2 is a feedforward reverb, a blur delay, and a glitch topping! Params (default): Delay 0.3400, Regen 0.3100, Thick 0.2800, Wet 0.2500.
- `Infinity2` — Infinity2 expands Infinity with more tone shaping and a much-requested kill-switch! Params (default): Filter 1.0000, Size 0.5000, Damping 0.0000, Allpass 1.0000, Feedback 1.0000, Dry/Wet 1.0000.
- `kCathedral` — kCathedral is a giant reverby space. Params (default): Wetness 1.0000.
- `Infinity` — Infinity is a MatrixVerb specifically designed to sustain and layer sounds forever. Params (default): Filter 1.0000, Damping 0.0000, Size 0.5000, Dry/Wet 1.0000.
- `MatrixVerb` — MatrixVerb is a wildly adjustable, strangely colorful reverb for deep and flexible spaces. Params (default): Filter 1.0000, Damping 0.0000, Speed 0.0000, Vibrato 0.0000, RmSize 0.5000, Flavor 0.5000, Dry/Wet 1.0000.
- `PocketVerbs` — PocketVerbs is my popular old special effects reverbs plugin! Params (default): Type Chamber, Size 0.5000, Gating 0.0000, Dry/Wet 0.5000.
- `Reverb` — Reverb is simplified and tuned MatrixVerb, adjusting everything for each optimum verb size. Params (default): Big 0.5000, Wet 0.5000.
- `Ultralight` — Ultralight is the most low-CPU matrix reverb I can make. Params (default): RT60 6, Furnish 3, SolidWl 3, ReflcWl 3, Predlay 1194, Dry/Wet 0.5000, Distnce 0.0000.

### Ambience

- `ClearCoat` _(basic, recommended)_ — ClearCoat is an array of bright ambience reverbs. Params (default): Select 8, Dry/Wet 1.0000.
- `TapeDelay2` _(recommended)_ — TapeDelay2 is a new implementation of a flexible tape echo. Params (default): Time 1.0000, Regen 0.0000, Freq 0.5000, Reso 0.0000, Flutter 0.0000, Dry/Wet 1.0000.
- `PitchDelay` _(recommended)_ — PitchDelay is TapeDelay2 but with pitch shift instead of flutter. Params (default): Time 1.0000, Regen 0.0000, Freq 0.5000, Reso 0.0000, Pitch 0.5000, Dry/Wet 1.0000.
- `TripleSpread` _(recommended)_ — TripleSpread is a stereo tripler with extra wideness and GlitchShifter processing. Params (default): Spread 0.0000, Tighten 0.5000, Dry/Wet 0.5000.
- `Echojam` — Echojam is an echo effect that is secretly a reverb. Params (default): Select 8, Regen 0.5000, Speed 0.5000, Dry/Wet 0.5000.
- `Doublelay` — Doublelay is StereoDoubler with independent delays and feedback. Params (default): Detune -0.6000, Delay L 0.1000 sec, Delay R 0.2000 sec, Feedbk 0.0000, Dry/Wet 0.6000.
- `SampleDelay` — SampleDelay is three delays combined: millisecond, sample and subsample. Params (default): ms 0, Samples 0, Subsamp 0.0000, Inv/Wet 1.0000.
- `BrightAmbience3` — BrightAmbience3 adds undersampling for high sample rates, and better feedback. Params (default): Start 1.0000, Length 0.0000, Feedback 0.0000, Dry/Wet 1.0000.
- `Melt` — Melt is a wobbly chorusy weird diffuse effect. Params (default): Depth 0.2500, Range 0.7500, Output 1.0000, Dry/Wet 1.0000.
- `MV2` — MV2 is a dual-mono reverb based on BitShiftGain and the old Midiverbs, adapted to high sample rates. Params (default): Depth 0.5000, Bright 0.5000, Regen 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `MV` — MV is a dual-mono reverb based on BitShiftGain and the old Midiverbs. Params (default): Depth 0.5000, Bright 0.5000, Regen 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `ADT` — ADT is a double short delay tap with saturation. Params (default): Headrm 1.0000, A Delay 0.5000, A Level 0.0000, B Delay 0.5000, B Level 0.0000, Output 1.0000.
- `kChamberAR` — kChamberAR is a take on tape echo into chamber echo. Params (default): Input 1.0000, Delay 0.5000, Regen 0.0000, Low Cut 0.0000, Wetness 0.5000.
- `StereoChorus` — StereoChorus is a nice basic stereo chorus. Params (default): Speed 0.0000, Depth 0.0000.
- `PurestEcho` — PurestEcho is optimized Airwindows echo with exactly four evenly spaced taps on tap. Params (default): Time 1.0000, Tap 1 1.0000, Tap 2 0.0000, Tap 3 0.0000, Tap 4 0.0000.
- `TapeDelay` — TapeDelay is an old school tape echo with pitch swerve effects. Params (default): Dry 1.0000, Wet 0.0000, Delay 0.5000, Feedbak 0.0000, Lean/Ft 1.0000, Depth 3.0000 taps.
- `StarChild2` — StarChild2 is a weird digital ambience/echo plugin adapted to high sample rates. Params (default): Sustain 1.0000, Grain 0.7000, Dry/Wet 0.2000.
- `StarChild` — StarChild is a weird digital ambience/echo plugin. Params (default): Sustain 1.0000, Grain 0.7000, Dry/Wet 0.2000.
- `Hombre` — Hombre is atmosphere and texture (through very short delays). Params (default): Voicing 0.4210, Intensity 0.5000.
- `BrightAmbience2` — BrightAmbience2 is more BrightAmbience with better tone and more slapbacky effects. Params (default): Start 0.2000, Length 0.2000, Feedbck 0.0000, Dry/Wet 0.5000.
- `BrightAmbience` — BrightAmbience is a plugin for very bright gated reverbs. Params (default): Dry/Wet 0.0000, Sustain 0.0000, Decay 0.0000.
- `StereoEnsemble` — StereoEnsemble is a sort of hyperchorus blast from the past. Params (default): Depth 0.5000, FXlevel 1.0000.
- `StereoDoubler` — StereoDoubler is like GlitchShifter optimized for pitch shift doubling and tripling. Params (default): Detune 0.2000, Dry/Wet 0.6000.
- `Chorus` — Chorus is a mono chorus, also works as a vibrato. Params (default): Speed 0.5000, Range 0.5000, Dry/Wet 0.5000.
- `ChorusEnsemble` — ChorusEnsemble is a more complex, multi-tap mono chorus. Params (default): Speed 0.5000, Range 0.5000, Dry/Wet 0.8000.
- `Ensemble` — Ensemble is a weird flangey little modulation effect. Params (default): Ensemble 25.0000 vox, Fullness 0.0000, Brighten 1.0000, Dry/Wet 1.0000.

### Tape

- `ToTape9` _(basic, recommended)_ — ToTape9 is Airwindows tape emulation and brightness compression. Params (default): Input 0.5000, Tilt 0.5000, Shape 0.5000, Flutter 0.5000, FlutSpd 0.5000, Bias 0.5000, HeadBmp 0.5000, HeadFrq 68.7500 hz, Output 0.5000.
- `TapeHack2` _(recommended)_ — TapeHack2 brings Airwindows tape to a new level. Params (default): Input 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `ToTape8` — ToTape8 is Airwindows tape emulation with gain staging, bias and optimized Dubly! Params (default): Input 0.5000, Tilt 0.5000, Shape 0.5000, Flutter 0.5000, FlutSpd 0.5000, Bias 0.5000, HeadBmp 0.5000, HeadFrq 68.7500 hz, Output 0.5000.
- `ToTape7` — ToTape7 is Airwindows tape emulation with bias and Dubly. Params (default): EncAmt 0.5000, EncFreq 0.5000, TapeDrv 0.5000, Flutter 0.5000, FlutSpd 0.5000, Bias 0.5000, HeadBmp 0.5000, HeadFrq 68.7500 hz, DecAmt 0.5000, DecFreq 0.5000.
- `ToTape6` — ToTape6 is Airwindows tape emulation for 2020! Params (default): Input 0.0000 dB, Soften 0.5000, Head B 0.5000, Flutter 0.5000, Output 0.0000 dB, Dry/Wet 1.0000.
- `FromTape` — FromTape is a minimalist, cleaner analog tape emulation. Params (default): Louder 1.0000, Softer 0.5000, Weight 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `TapeHack` — TapeHack is a new dimension in tape realism, abstracted to software. Params (default): Input 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `Tape` — Tape is simplified, all-purpose tape mojo: my personal jam. Params (default): Slam 0.0000 dB, Bump 0.5000.
- `IronOxideClassic2` — IronOxideClassic2 is my bandpassy tape sim, updated for high sample rate and aliasing control. Params (default): Input Trim 0.0000 dB, Tape Speed 16.3500 ips, Output Trim 0.0000 dB.
- `IronOxide5` — IronOxide5 is the old school, heavily colored Airwindows tape emulation. Params (default): Input Trim 0.0000 dB, Tape High 16.3500 ips, Tape Low 16.3500 ips, Flutter 0.5000, Noise 0.5000, Output Trim 0.0000 dB, Inv/Dry/Wet 1.0000.
- `ToTape5` — ToTape5 is Airwindows analog tape emulation. Params (default): Louder 0.2500, Softer 0.0000, Fatter 0.0000, Flutter 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `IronOxideClassic` — IronOxideClassic is the purer, simpler, early form of Iron Oxide before all the features. Params (default): Input Trim 0.0000 dB, Tape Speed 16.3500 ips, Output Trim 0.0000 dB.

### Saturation

- `Hypersoft` _(recommended)_ — Hypersoft is a more extreme form of soft-clipper. Params (default): Input 0.5000, Depth 0.5000, Bright 0.5000, Output 0.5000.
- `Tube` _(basic)_ — Tube is a tube style SoundBetterIzer using a new algorithm for analog modeling! Params (default): TUBE 0.0000.
- `PurestSaturation` — PurestSaturation is an experiment in softclipping. Params (default): Input 0.1000, Output 1.0000.
- `Longhand` — Longhand is an experiment in softclipping. Params (default): Input 0.4450, Output 1.0000, Dry/Wet 1.0000.
- `Creature` — Creature is a soft slew saturator, a new class of noisechanger. Params (default): Drive 0.2600, Depth 0.2600, Inv/Wet 1.0000.
- `Huge` — Huge is a dirty, dirty loudenator. Params (default): Huge 0.2000, Nasty 0.2000.
- `NCSeventeen` — NCSeventeen is Dirty Loud! Params (default): LOUDER 0.0000 dB, Output 1.0000.
- `Spiral2` — Spiral2 is Spiral with controls including Presence. Params (default): Input 0.5000, Highpass 0.0000, Presence 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `PurestDrive` — PurestDrive is the magic saturation plugin of subtlety and French House tone. Params (default): Drive 0.0000.
- `Focus` — Focus brings out clarity by distorting. Aggressive, subtle, flexible. Params (default): Boost 0.0000 dB, Focus 0.5000, Mode Spiral, Output 1.0000, Dry/Wet 1.0000.
- `Mojo` — Mojo is a biggenator that also works as a loudenator. Params (default): Input 0.0000 dB.
- `Dyno` — Dyno is a lot like Mojo, but for intensity instead of loudness. Params (default): Input 0.0000 dB.
- `Spiral` — Spiral is the new best smoothest distortion algorithm. No parameters.
- `UnBox` — UnBox is a distortion where only the harmonics that don't alias are allowed to distort. Params (default): Input 1.0000, UnBox 0.0000, Output 1.0000.
- `Desk4` — Desk4 is distinctive analog coloration (a tuneable version of the control-less Desk plugins) Params (default): Overdrive 0.2700, Hi Choke 0.1800, Power Sag 0.2600, Frequency 0.5400, Output Trim 0.8400, Dry/Wet 1.0000.
- `Righteous4` — Righteous4 is a final output stage for targeting dynamic range. Params (default): LTarget -28.0000 dB, BtDepth 16 bit.

### Subtlety

- `PurestWarm3` _(recommended)_ — PurestWarm3 is a subtle tone shaper and warmth adder. Params (default): Asym 0.5000.
- `Mastering2` _(recommended)_ — Mastering2 is Airwindows style, and can do elliptical EQ now! Params (default): Sidepas 0.0000, Glue 0.0000, Scope 0.5000, Skronk 0.5000, Girth 0.5000, Drive 0.5000, Dither Bypass.
- `Discontapeity` _(recommended)_ — Discontapeity combines air modeling with tape-style overdrive. Params (default): More 0.0000.
- `Discontinuity` _(recommended)_ — Discontinuity models air under intense loudness. Params (default): Top dB 105.0000 dB.
- `Shape` _(recommended)_ — Shape is an asymmetrical waveshaper for peak manipulating. Params (default): Shape 0.5000, Neg/Pos 0.5000.
- `Inflamer` _(recommended)_ — Inflamer is an Airwindowsized take on the waveshapers in Oxford Inflator. Params (default): Drive 0.5000, Curve 0.5000, Effect 1.0000.
- `Sweeten` _(basic, recommended)_ — Sweeten is where you can find super-clean second harmonic. Params (default): Sweeten 0.0000.
- `PurestWarm2` _(basic)_ — PurestWarm2 adds dual dry/wet controls for each side of the waveform. Params (default): Dry/Pos 0.0000, Dry/Neg 0.0000.
- `Interstage` _(basic)_ — Interstage is a subtle and sophisticated analogifier. No parameters.
- `SingleEndedTriode` _(recommended)_ — SingleEndedTriode is unusual analog modeling effects. Params (default): Triode 0.0000, Clas AB 0.0000, Clas B 0.0000, Dry/Wet 0.0000.
- `Mastering` — Mastering is Airwindows style, and can do things nothing else can! Params (default): Glue 0.0000, Scope 0.5000, Skronk 0.5000, Girth 0.5000, Drive 0.5000, Dither Bypass.
- `Hype` — Hype is a treble softening experiment gone strangely awry :) No parameters.
- `PurestWarm` — PurestWarm is a subtle tone shaper and warmth adder. Params (default): Polarity Normal.
- `Coils2` — Coils2 is a transformer overdrive emulator. Params (default): Saturate 0.0000, Cheapness 0.0000, Dry/Wet 1.0000.
- `PhaseNudge` — PhaseNudge is a phase rotator/allpass filter. Params (default): Phase Nudge 0.0000, Dry/Wet 1.0000.
- `Remap` — Remap puts the guts back into overloudenated audio! Params (default): Remap 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `Coils` — Coils models the types of distortion you'll find in transformers. Params (default): Saturat 0.0000, Core DC 0.0000, Dry/Wet 1.0000.
- `Desk` — Desk is classic Airwindows subtle analog modeling. No parameters.
- `TransDesk` — TransDesk is more of a transistory, rock desk analog modeling. No parameters.
- `TubeDesk` — TubeDesk is a tube recording console type tone coloring. No parameters.

### Tone Color

- `WoodenBox` — WoodenBox is like a miniature reverb for converting DI to acoustic. Params (default): Select 8, Reso 0.5000, Depth 0.5000.
- `BussColors4` — BussColors4 is the Airwindows console emulations, now working up to 192K correctly. Params (default): Color Dark, Input Trim 0.0000 dB, Output Trim 0.0000 dB, Dry/Wet 1.0000.
- `Channel9` — Channel9 is Channel8 with ultrasonic filtering, and new Teac and Mackie 'low end' settings. Params (default): Console Type Neve, Drive 0 %, Output 1.0000.
- `Apicolypse` — Apicolypse is a re-release of my old API-style color adder, exacly as it was. Params (default): Hardns 0.7000, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Neverland` — Neverland is a re-release of my old Neve-style color adder, exacly as it was. Params (default): Hardns 0.7400, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Elation` — Elation is a re-release of another old Character plugin. Params (default): Dry/Wet 1.0000, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Calibre` — Calibre is a re-release of another old Character plugin. Params (default): Hardns 0.7400, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Cider` — Cider is a re-release of another old Character plugin. Params (default): Hardns 0.7000, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Crystal` — Crystal is a tone shaper and buss soft-clipper. Params (default): Hardnss 0.7000, Personl 1.0000, Drive 0.9990, Output 1.0000.
- `Precious` — Precious is a re-release of another old Character plugin. Params (default): Hardns 0.7000, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Luxor` — Luxor is a re-release of another old Character plugin. Params (default): Hardns 0.7000, Persnlty 1.0000, Drive 1.0000, Output 1.0000.
- `Channel8` — Channel8 is Channel7 with updates from Capacitor2 and Slew3. More and better of what Channel is. Params (default): Console Type Neve, Drive 0 %, Output 1.0000.
- `Channel6` — Channel6 uses the Spiral algorithm. Params (default): Console Type Neve, Drive 0 %, Output 1.0000.
- `Channel5` — Channel5 is Channel for 2018, with new subtleties. Params (default): Console Type Neve, Drive 0 %, Output 1.0000.
- `Channel4` — Channel4 is a tone coloring plugin that gives more of an analog feel. Params (default): Console Type Neve, Drive 0 %.

### Distortion

- `Mackity` _(recommended)_ — Mackity is an emulation of the input stage of a vintage Mackie 1202! Params (default): In Trim 0.1000, Out Pad 1.0000.
- `Distortion` — Distortion is a slightly dark analog-style distortion with several presets, like Focus. Params (default): Input 0.0000 dB, Mode Spiral, Output 0.0000 dB, Dry/Wet 1.0000.
- `Edge` — Edge is a seven-stage distortion with Hypersonic filtering and tone shaping controls. Params (default): Gain 0.1000, Lowpass 1.0000, Highpass 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `Dirt` — Dirt is a soft-clip distortion in the spirit of Edge. Params (default): Gain 0.1000, Lowpass 1.0000, Highpass 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `ZOutputStage` — ZOutputStage is the output clipping from the Emu e6400 style Z filters. Params (default): Input 0.1000, Output 1.0000.
- `Density2` — Density2 is a different color for Density, some old code I had that people wanted. Params (default): Density 0.0000, Highpass 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `Zoom` — Zoom is a bank of Density-like waveshapers for subtly modulating peak energy. Params (default): Zoom 0.5000.
- `Density` — Density is a Swiss Army Knife of saturation/antisaturation. Params (default): Density 0.0000, Highpass 0.0000, Out Level 1.0000, Dry/Wet 1.0000.
- `Drive` — Drive is the angry distortion! Params (default): Drive 0.0000 %, Highpass 0.0000, Out Level 1.0000, Dry/Wet 1.0000.
- `Loud` — Loud is distortion and demolition of air molecules, modeled. Params (default): Boost 0.0000, Dry/Wet 1.0000.
- `HardVacuum` — HardVacuum is tube style saturation effects. Params (default): Drive 0.0000, Warmth 0.0000, Aura 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `HighImpact` — HighImpact is distorted grit and punch without fatness. Params (default): Impact 0.0000, Dry/Wet 0.0000.
- `MultiBandDistortion` — MultiBandDistortion is an old weird gnarly sound wrecker :) Params (default): Freqcy 0.5000, H Gain 0.0000 dB, L Gain 0.0000 dB, H Hard 0.0000, L Hard 0.0000, H Mtrx 1.0000, L Mtrx 1.0000, Stable 1.0000, Output -12.0000 dB.

### Clipping

- `ClipOnly3` _(basic, recommended)_ — ClipOnly3 is just plain stinkin' loud. No parameters.
- `OneCornerClip` _(recommended)_ — OneCornerClip is an ultimate full-bandwidth clipper. Params (default): Input 0.0000, Pos Thr -0.3005, Neg Thr -0.3005, Voicing 0.6180, Dry/Wet 1.0000.
- `ADClip9` — ADClip9 is the ClipOnly3 version of a loudenator/biggenator. Params (default): Boost 0.0000 dB, Match 0.0000 dB, Noise 0.7000, Ceiling 0.7500, Mode Boost.
- `ADClip8` — ADClip8 is a stacked-up Airwindows loudenator based on ClipOnly2. Params (default): Boost 0.0000 dB, Ceiling 0.8830, Mode Normal.
- `ClipOnly2` — ClipOnly2 suppresses the brightness of digital clipping without affecting unclipped samples, at any sample rate. No parameters.
- `ClipSoftly` — ClipSoftly is ClipOnly2 but as a softclipper: a very handy building block. No parameters.
- `ADClip7` — ADClip7 is the original Airwindows loudness maximizer/ peak control. Params (default): Boost 0.0000 dB, Soften 0.5000, Enhance 0.5000, Mode Normal.
- `FinalClip` — FinalClip is one stage of ADClip8 set up to clip +6dB for Final Cut Pro. No parameters.

### Dynamics

- `BeziComp` _(basic, recommended)_ — BeziComp is a radical compressor that eliminates aliasing! Params (default): Comp 0.5000, Speed 0.5000, Dry/Wet 0.5000.
- `Pop3` _(recommended)_ — Pop3 is the dynamics from ConsoleX. Params (default): Thresld 1.0000, C Ratio 0.5000, C Atk 0.5000, C Rls 0.5000, Thresld 0.0000, G Ratio 0.5000, G Sust 0.5000, G Rls 0.5000.
- `Pop2` _(recommended)_ — Pop2 adds control and punch to Pop. Params (default): Compres 0.5000, Attack 0.5000, Release 0.5000, Drive 0.5000, Dry/Wet 1.0000.
- `StoneFireComp` _(recommended)_ — StoneFireComp is some of the dynamics for ConsoleX. Params (default): Fire Th 1.0000, Attack 0.5000, Release 0.5000, Fire 0.5000, StoneTh 1.0000, Attack 0.5000, Release 0.5000, Stone 0.5000, Range 0.5000, Ratio 1.0000.
- `DigitalBlack` _(basic, recommended)_ — DigitalBlack is a quick, staccato gate. Params (default): Thresh 0.0000, Dry/Wet 1.0000.
- `Pyewacket` _(recommended)_ — Pyewacket is an old school compressor for high definition transients. Adds no fatness, just energy. Params (default): Input Gain 0.0000 dB, Release 0.5000, Output Gain 0.0000 dB.
- `SoftGate` _(recommended)_ — SoftGate is a gate that can mute hiss and smooth sample tails. Params (default): Thresh 0.5000, Darken 0.5000, Silence 0.0000.
- `Gatelope` _(recommended)_ — Gatelope is a special gate that applies filters. Params (default): Thresh 0.0000, TrebSus 1.0000, BassSus 0.5000, AttackS 0.0000, Dry/Wet 1.0000.
- `Pressure6` — Pressure6 refines the Pressure compressor and how it moves. Params (default): Compres 0.0000, Ratio 1.0000.
- `Dynamics2` — Dynamics2 is the compressor gate for new Console plugins! Params (default): Thresh 1.0000, Attack 0.5000, Release 0.5000, Gate 0.0000.
- `X2Buss` — X2Buss is an updated version of the buss processing from ConsoleX2. Params (default): High 0.5000 eq, HMid 0.5000, LMid 0.5000, Bass 0.5000, HighF 0.5000 freq, HMidF 0.5000, LMidF 0.5000, BassF 0.5000, Thresh 1.0000 dyn, Fader 0.5000.
- `Dynamics` — Dynamics is a combination compressor gate. Params (default): Comp 0.0000, Speed 0.5000, Gate 0.0000, Dry/Wet 1.0000.
- `Pop` — Pop is a crazy overcompressor with a very exaggerated sound. Params (default): Intenst 0.3000, Output 1.0000, Dry/Wet 1.0000.
- `Logical4` — Logical4 is a classic 2-buss compressor. Params (default): Threshold 0.0000 dB, Ratio 1.6000 /1, Speed 4.6503 ms, MakeupGn 0.0000 dB, Dry/Wet 1.0000.
- `VariMu` — VariMu is a more organic variation on Pressure (a compressor) Params (default): Intensty 0.0000, Speed 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `ButterComp2` — ButterComp2 is improved ButterComp with an output control and sound upgrades. Params (default): Compress 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `curve` — curve is the simplest, purest form of Recurve with no extra boosts. No parameters.
- `Overheads` — Overheads is for compressing only part of the sound, strangely! Params (default): Compr 0.0000, Sharp 0.5000, Dry/Wet 1.0000.
- `Recurve` — Recurve is a special buss compressor with no threshold point. No parameters.
- `BlockParty` — BlockParty is like a moderately saturated analog limiter. Params (default): Pound 0.0000, Dry/Wet 1.0000.
- `Thunder` — Thunder is a compressor that retains or exaggerates subsonic bass when you push it. Params (default): Thunder 0.0000, Output Trim 0.0000 dB.
- `DrumSlam` — DrumSlam is a heavy-processing tape modeler. Params (default): Drive 1.0000, Output 1.0000, Dry/Wet 1.0000.
- `ButterComp` — ButterComp is my softest, smoothest compressor. Params (default): Compress 0.0000, Dry/Wet 1.0000.
- `BrassRider` — BrassRider is One Weird Trick for drum overheads in metal! Params (default): Thresh 0.0000, Dry/Wet 0.0000.
- `Point` — Point is an explosive transient designer. Params (default): Input Trim 0.0000 dB, Point 0.0000, Reaction Speed 0.5000.
- `PodcastDeluxe` — PodcastDeluxe is a pile of compressors (curve style) and phase rotators. Params (default): Boost 0.5000.
- `Podcast` — Podcast is a simpler pile of curve-style compressors with hard clipping. Params (default): Boost 0.5000, Dry/Wet 1.0000.
- `Swell` — Swell is Dial-an-attack, like sidechaining. Params (default): Thresh 0.9000, Swell 0.5000, Dry/Wet 1.0000.
- `PurestSquish` — PurestSquish is an open-sounding compressor with bass bloom. Params (default): Squish 0.0000, BassBlm 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `Pressure4` — Pressure4 is a compressor adjustable between vari-mu and ‘new york’ peak-retaining behaviors. Params (default): Pressure 0.0000, Speed 0.2000, Mewiness 1.0000, Output Gain 1.0000.
- `Surge` — Surge is a compressor for accentuating beats and pulses. Params (default): Surge 0.0000, Dry/Wet 1.0000.
- `SurgeTide` — SurgeTide is a surge and flow dynamics plugin. Params (default): SurgeNode 0.5000, SurgeRate 0.3000, Dry/Wet 1.0000.

### Filter

- `FastDistance` _(recommended)_ — FastDistance is a sort of ultra-efficient lowpass filter. Params (default): Distnce 0.0000.
- `Suzan` _(recommended)_ — Suzan is a new type of ladder filter. Params (default): Freq 0.5000, Reso 0.5000, Output 1.0000.
- `SmoothEQ3` _(basic, recommended)_ — SmoothEQ3 is the most approachable EQ. Params (default): High 0.5000, Mid 0.5000, Bass 0.5000.
- `PearEQ` _(recommended)_ — PearEQ is a six-band Pear-based graphic EQ. Params (default): High 0.5000, HMid 0.5000, Mid 0.5000, LMid 0.5000, Bass 0.5000, Sub 0.5000.
- `PearLiteEQ` _(recommended)_ — PearLiteEQ is a four-band Pear-based graphic EQ. Params (default): High 0.5000, HMid 0.5000, LMid 0.5000, Bass 0.5000.
- `RetroBass` _(recommended)_ — RetroBass is a special kind of highpass. Params (default): Freq 0.5000.
- `FatEQ` _(recommended)_ — FatEQ balances bands only with density. Params (default): High 0.5000, HMid 0.5000, LMid 0.5000, Bass 0.5000, Out 1.0000.
- `SmoothEQ2` _(basic, recommended)_ — SmoothEQ2 is a wildly intuitive vibe EQ with clarity and intensity. Params (default): High 0.5000, HMid 0.5000, LMid 0.5000, Bass 0.5000, HighF 0.5000, HMidF 0.5000, LMidF 0.5000, BassF 0.5000.
- `Parametric` _(recommended)_ — Parametric is three bands of ConsoleX EQ in advance. Params (default): Tr Freq 0.5000, Treble 0.5000, Tr Reso 0.5000, HM Freq 0.5000, HighMid 0.5000, HM Reso 0.5000, LM Freq 0.5000, LowMid 0.5000, LM Reso 0.5000, Dry/Wet 1.0000.
- `Dattorro` _(recommended)_ — Dattorro is the resonant lowpass filter out of Donut. Params (default): Freq 0.5000, Reso 0.5000, Output 1.0000.
- `Silken` _(recommended)_ — Silken is a high frequency boost that gives ambience and texture. Params (default): Silken 0.0000, Freq 0.5000, Window 0.5000.
- `Stonefire` _(recommended)_ — Stonefire is the non-EQ EQ designed for ConsoleX. Params (default): Air 0.5000, Fire 0.5000, Stone 0.5000, Range 0.5000.
- `Isolator3` _(basic, recommended)_ — Isolator3 is Isolator2, but on one slider, with a band-narrower control. Params (default): Iso 0.5000, Q 0.0000.
- `Distance3` _(recommended)_ — Distance3 combines the best parts of Distance and Discontinuity. Params (default): Distance 5.0000 miles, Top dB 105.0000 dB, Dry/Wet 1.0000.
- `Weight` _(recommended)_ — Weight is a very accurate sub-bass boost based on Holt. Params (default): Freq 70.0000 hz, Weight 0.0000.
- `ToneSlant` _(basic, recommended)_ — ToneSlant is a super-transparent ‘tilt EQ’ with very low Q. Params (default): Voicing 1.0000 taps, Highs -1.0000.
- `BezEQ3` — BezEQ3 refines the concept into four vivid, unified bands. Params (default): High 0.5000, HMid 0.5000, LMid 0.5000, Bass 0.5000, HighF 0.5000, HMidF 0.5000, LMidF 0.5000, BassF 0.5000.
- `BezEQ2` — BezEQ2 is a unique, subtle three-band shelving EQ. Params (default): Treble 0.5000, x 0.7500, Mid 0.5000, x 0.2500, Bass 0.5000.
- `SmoothEQ` — SmoothEQ is a sharp, accurate, transparent three-band filter. Params (default): High 0.5000, Mid 0.5000, Low 0.5000, XoverH 0.6000, XoverL 0.4000.
- `Donut` — Donut is in memory of Dilla. Params (default): Attack 0.5000, Release 0.5000, BaseFrq 1.0000, MoveFrq 0.5000, BaseRes 0.0000, MoveRes 0.5000, Output 1.0000.
- `AngleEQ` — AngleEQ is a strange and colorful EQ. Params (default): High 0.5000, Mid 0.4000, Low 0.5000, HiFreq 0.8000, MHiFreq 0.8000, MLoFreq 0.2000, LoFreq 0.2000, Inv/Wet 1.0000.
- `AngleFilter` — AngleFilter is the synth-style extension of AngleEQ. Params (default): Freq 0.5000, Hard 0.5000, Reso 0.0000, Poles 0.5000.
- `BezEQ` — BezEQ is a strange, alien three-band shelving EQ. Params (default): Treble 0.5000, x 0.6180, Mid 0.5000, x 0.3820, Bass 0.5000.
- `Baxandall3` — Baxandall3 is for new tone colors. Params (default): Input 0.5000, Treble 0.5000, Bass 0.5000.
- `Baxandall2` — Baxandall2 is Baxandall, extended and made more powerful. Params (default): Treble 0.0000 dB, Bass 0.0000 dB.
- `Baxandall` — Baxandall is a killer general-purpose EQ. Params (default): Treble 0.0000 dB, Bass 0.0000 dB, Output 0.0000 dB.
- `Pear2` — Pear2 is my Pear filter plus nonlinearity. Params (default): Freq 1.0000, NonLin 0.5000, Poles 0.5000, Inv/Wet 1.0000.
- `Capacitor2` — Capacitor2 is Capacitor with extra analog modeling and mojo. Params (default): Lowpass 1.0000, Highpass 0.0000, NonLin 0.0000, Dry/Wet 1.0000.
- `Pear` — Pear is the testbed for a new filter, implemented as a highpass/lowpass shelf. Params (default): bitFreq 1.0000, Poles 0.5000, Inv/Wet 1.0000.
- `ResEQ2` — ResEQ2 is a single, sharp, sonorous mid peak. Params (default): MSweep 0.5000, MBoost 0.0000.
- `SubTight` — SubTight uses a variation on the Creature algorithm to tighten sub-lows. Params (default): Trim 0.3000, Steep 0.5000.
- `Isolator2` — Isolator2 is Isolator, but with control smoothing and a new resonance booster. Params (default): Freq 1.0000, Reso 0.0000, High 0.0000, Low 1.0000.
- `Kalman` — Kalman is not a real filter! Params (default): Kalman 0.5000, Inv/Wet 1.0000.
- `PrimeFIR` — PrimeFIR is a mostly linear-phase brickwall with a taste for the bizarre! Params (default): Freq 1.0000, Window 0.5000, Prime 0.0000.
- `Holt2` — Holt2 is a bass-cab-like resonant lowpass filter. Params (default): Freq 1.0000, Reso 0.5000, Poles 8.0000, Output 1.0000, Dry/Wet 1.0000.
- `Holt` — Holt is a synth-like resonant lowpass filter focussed on bass frequencies. Params (default): Freq 1.0000, Reso 0.5000, Poles 4.0000, Output 1.0000, Dry/Wet 1.0000.
- `AverMatrix` — AverMatrix turns averaging into a full-featured EQ. Params (default): Average 1.0000 taps, Depth 1.0000 poles, Inv/Wet 1.0000.
- `Average` — Average is a distinctive sort of lowpass filter. Params (default): Average 1.0000 taps, Dry/Wet 1.0000.
- `MackEQ` — MackEQ is the Mackie distortion but with treble and bass controls added. Params (default): Trim 0.1000, Hi 0.5000, Lo 0.5000, Gain 1.0000, Dry/Wet 1.0000.
- `Hull2` — Hull2 is a very clear three-band EQ. Params (default): Treble 0.5000, Mid 0.5000, Bass 0.5000.
- `Hull` — Hull is an alternate form of highpass/lowpass filter. Params (default): Freq 0.0000, Bright 0.0000.
- `EQ` — EQ is just the EQ parts of CStrip. Params (default): Treble 0.0000 dB, Mid 0.0000 dB, Bass 0.0000 dB, Lowpass 16.0000 Khz, TrebFrq 3.4000 Khz, BassFrq 281.2000 hz, Hipass 30.0000 hz, OutGain 0.0000 dB.
- `Capacitor` — Capacitor is a lowpass/highpass filter of a new type. Params (default): Lowpass 1.0000, Highpass 0.0000, Dry/Wet 1.0000.
- `Isolator` — Isolator is a steep highpass or lowpass filter, like you might find in a speaker crossover. Params (default): Freq 1.0000, High 0.0000, Low 1.0000.
- `TapeFat` — TapeFat is the tone control from TapeDelay. Params (default): Lean/Ft 1.0000, Depth 3.0000 taps.
- `ResEQ` — ResEQ is a bank of mostly midrange resonances. Params (default): Reso 1 0.0000, Reso 2 0.0000, Reso 3 0.0000, Reso 4 0.0000, Reso 5 0.0000, Reso 6 0.0000, Reso 7 0.0000, Reso 8 0.0000, Dry/Wet 0.0000.
- `Lowpass2` — Lowpass2 is an unusual-sounding variable-slope lowpass filter. Params (default): Lowpass 0.0000, Sft/Hrd 0.0000, Poles 1.0000, Dry/Wet 1.0000.
- `Highpass2` — Highpass2 is an unusual-sounding variable-slope highpass filter. Params (default): Hipass 0.0000, Ls/Tite 0.0000, Poles 1.0000, Dry/Wet 1.0000.
- `Distance` — Distance is a sound design or reverb far-away-izer. Params (default): Distance 0.0000 miles, Dry/Wet 1.0000.
- `Distance2` — Distance2 is a versatile space shaper for creating depth. Params (default): Atmosph 0.8500, Darken 0.6180, Dry/Wet 0.6180.
- `Lowpass` — Lowpass deepens the tone, leaves a gloss and textural modifications. Params (default): Lowpass 1.0000, Soft/Hard 0.0000, Dry/Wet 1.0000.
- `Highpass` — Highpass is a time warp, for retro midrangey sounds. Params (default): Highpass 0.0000, Loose/Tight 0.0000, Dry/Wet 1.0000.

### XYZ Filters

- `ZBandpass2` _(basic, recommended)_ — ZBandpass2 acts more like the Emu e6400 Ultra bandpass in motion, with control smoothing. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZHighpass2` _(basic, recommended)_ — ZHighpass2 acts more like the Emu e6400 Ultra highpass in motion, with control smoothing. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZRegion2` _(basic, recommended)_ — ZRegion2 is an Emu e6400 style Airwindows Region filter, with control smoothing. Params (default): Input 0.1000, First 0.5000, Last 0.5000, Poles 0.5000, Dry/Wet 1.0000.
- `YBandpass` _(recommended)_ — YBandpass is soft and smooth to nasty, edgy texture-varying filtering. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YHighpass` _(recommended)_ — YHighpass is soft and smooth to nasty, edgy texture-varying filtering. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YLowpass` _(recommended)_ — YLowpass is soft and smooth to nasty, edgy texture-varying filtering. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YNotch` _(recommended)_ — YNotch is soft and smooth to nasty, edgy texture-varying filtering. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `XBandpass` _(recommended)_ — XBandpass is a distorted digital EQ, inspired by retro sampler DSP. Params (default): Gain 0.5000, Freq 0.5000, Nuke 0.0000, Dry/Wet 1.0000.
- `XHighpass` _(recommended)_ — XHighpass is a distorted digital EQ, inspired by retro sampler DSP. Params (default): Gain 0.5000, Freq 1.0000, Nuke 0.0000, Dry/Wet 1.0000.
- `XLowpass` _(recommended)_ — XLowpass is a distorted digital EQ, inspired by retro sampler DSP. Params (default): Gain 0.5000, Freq 1.0000, Nuke 0.0000, Dry/Wet 1.0000.
- `XNotch` _(recommended)_ — XNotch is a distorted digital EQ, not as glitchy as the others. Params (default): Gain 0.5000, Freq 1.0000, Nuke 0.0000, Dry/Wet 1.0000.
- `XRegion` _(recommended)_ — XRegion is distorted staggered bandpasses, for extreme soundmangling. Params (default): Gain 0.5000, First 0.5000, Last 0.5000, Nuke 0.0000, Dry/Wet 1.0000.
- `ZAcidLowpass` — ZAcidLowpass is like if an e6400 went insane and grew shark teeth. Params (default): Cutoff 0.5000, Over 0.0000, Under 1.0000, Meltdwn 0.0000, Drive 0.1000, Output 1.0000.
- `ZBandpass` — ZBandpass is a bandpass made to sound and act like the Emu e6400 Ultra bandpass. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZHighpass` — ZHighpass is a highpass made to sound and act like the Emu e6400 Ultra highpass. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZLowpass` — ZLowpass is a lowpass made to sound and act like the Emu e6400 Ultra lowpass. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZNotch` — ZNotch is a notch filter made to sound and act like the Emu e6400 Phaser. Params (default): Input 0.1000, Freq 0.5000, Output 1.0000, Poles 0.5000.
- `ZRegion` — ZRegion is an Emu e6400 style Airwindows Region filter. Params (default): Input 0.1000, First 0.5000, Last 0.5000, Poles 0.5000, Dry/Wet 1.0000.
- `YNotBandpass` — YNotBandpass is soft and smooth to nasty, edgy texture-varying filtering, no control smoothing. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YNotHighpass` — YNotHighpass is soft and smooth to nasty, edgy texture-varying filtering, no control smoothing. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YNotLowpass` — YNotLowpass is soft and smooth to nasty, edgy texture-varying filtering, no control smoothing. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.
- `YNotNotch` — YNotNotch is soft and smooth to nasty, edgy texture-varying filtering, no control smoothing. Params (default): Gain 0.1000, Freq 0.5000, Reson8 0.1000, ResEdge 0.1000, Output 1.0000, Dry/Wet 1.0000.

### Biquads

- `BiquadStack` _(recommended)_ — BiquadStack is a way of making a parametric EQ out of stacked biquad filters. Params (default): Freq 0.5000, Level 0.5000, Reso 0.5000.
- `BiquadNonLin` — BiquadNonLin is Capacitor2, but for biquad filtering. Params (default): Type 0.0000, Freq 0.1251, Q 10.6128, NonLin 0.0000, Inv/Wet 1.0000.
- `BiquadHiLo` — BiquadHiLo is the highpass and lowpass filter in ConsoleX. Params (default): Highpas 0.0000, Lowpass 0.0000.
- `BiquadPlus` — BiquadPlus is Biquad plus zipper noise suppression! For twiddling the controls. Params (default): Type 4.0000, Freq 0.1251, Q 3.7587, Inv/Wet 1.0000.
- `Biquad` — Biquad is the Airwindows implementation of a biquad filter. Params (default): Type 4.0000, Freq 0.1251, Q 3.7587, Inv/Wet 1.0000.
- `BiquadDouble` — BiquadDouble is a handy Airwindows cascaded biquad filter: steeper roll-off before resonance. Params (default): Type 4.0000, Freq 0.1251, Q 3.7587, Inv/Wet 1.0000.
- `BiquadOneHalf` — BiquadOneHalf is an interleaved biquad filter like Biquad. Params (default): Type 1.0000, Freq 0.0001, Q 0.0100, Inv/Wet -1.0000.
- `BiquadTriple` — BiquadTriple is a handy Airwindows cascaded biquad filter: steeper roll-off before resonance. Params (default): Type 4.0000, Freq 0.1251, Q 3.7587, Inv/Wet 1.0000.
- `Biquad2` — Biquad2 is the Airwindows biquad filter that's more sweepable and synthy. Params (default): Type 4.0000, Freq 0.2501, Q 12.5075, Output 1.0000, Inv/Wet 1.0000.

### Brightness

- `Slew4` _(recommended)_ — Slew4 is tape compression without the tape, for brightness control. Params (default): Compres 0.5000.
- `DeBess` _(recommended)_ — DeBess is an improved DeEss, with perfect rejection of non-ess audio. Params (default): Intense 0.0000, Sharp 0.5000, Depth 0.5000, Filter 0.5000, Sense 0.0000 Ess.
- `Sinew` _(basic, recommended)_ — Sinew combines sines and slew clipping for a tape bias effect! Params (default): Sinew 0.5000.
- `SlewSonic` _(recommended)_ — SlewSonic combines SlewOnly with ultrasonic filtering to solo brightness. Params (default): Mute 15.0000 kHz, Bright 0.0000.
- `Smooth` _(basic, recommended)_ — Smooth can tame pointy sounds or make drums explode. Params (default): Smooth 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `PlatinumSlew` — PlatinumSlew is the best easy-to-use slew clipper. Params (default): Slew 0.0000.
- `GoldenSlew` — GoldenSlew is a multistage slew clipper based on the Golden Ratio. Params (default): Slew 0.0000.
- `Acceleration2` — Acceleration2 is Acceleration, better! Updated for high sample rate and more extreme effect. Params (default): Limit 0.3200, Dry/Wet 1.0000.
- `DeEss` — DeEss is the best de-essing solution there is. A go-to utility plugin. Params (default): Intense 0.0000, Max DS -24.0000, Freq 0.5000.
- `EverySlew` — EverySlew is a wide variety of unique sound processing tools. Params (default): Slew 0.0000, Depth 1.0000, Halo 0.1600, Inv/Wet 1.0000.
- `Slew3` — Slew3 is a new approach to slew clipping meant for a more analog-like darkening effect. Params (default): Clampng 0.0000.
- `Slew` — Slew is a slew clipper, which darkens treble in an unusual way. Params (default): Clamping 0.0000.
- `Air2` — Air2 is a different bright EQ with silk tone and high sample rate support. Params (default): Hiss 0.0000, Glitter 0.0000, Air 0.0000, Silk 0.0000, Dry/Wet 1.0000.
- `Air` — Air is a different bright EQ from any other. Requires 44.1K. Params (default): 22K tap 0.0000, 15K tap 0.0000, 11K tap 0.0000, filters Q 0.0000, Dry/Wet 1.0000.
- `PurestAir` — PurestAir is a brightness experiment. Params (default): Boost 0.0000, Limit 0.0000, Dry/Wet 1.0000.
- `Acceleration` — Acceleration is an acceleration limiter that tames edge, leaves brightness. Params (default): Limit 0.3200, Dry/Wet 1.0000.
- `Hypersonic` — Hypersonic is Ultrasonic, but steeper and higher cutoff. No parameters.
- `HypersonX` — HypersonX is like UltrasonX but with seven stages instead of five. Params (default): Q Reso D.
- `Ultrasonic` — Ultrasonic is a very clean, plain, high quality supersonic filter, for using inside digital mixes. No parameters.
- `UltrasonicLite` — UltrasonicLite is just one stage of supersonic filter, for using inside digital mixes. No parameters.
- `UltrasonicMed` — UltrasonicMed is two stages of supersonic filter, for using inside digital mixes. No parameters.
- `UltrasonX` — UltrasonX is a method for rolling your own Console-type systems with total control over your ultrasonic filtering. Params (default): Q Reso C.

### Bass

- `DubSub2` _(recommended)_ — DubSub2 is the essence of the Airwindows head bump. Params (default): HeadBmp 0.5000, HeadFrq 68.7500 hz, Dry/Wet 0.5000.
- `OrbitKick` _(recommended)_ — OrbitKick is a bass generator and kick drum reinforcer. Params (default): Drop 0.5000, Shape 0.5000, Start 0.5000, Finish 0.5000, Thresh 0.5000, Dry/Wet 1.0000.
- `Hermepass` — Hermepass is a mastering highpass to set by ear only. Params (default): Cutoff 0.5000, Slope 0.5000.
- `BassKit` — BassKit is centered bass reinforcement with subs fill. Clean and controllable. Params (default): Drive 0.5000, Voicing 0.5000, BassOut 0.0000, SubOut 0.0000.
- `DubCenter` — DubCenter is a version of DubSub where the bass reinforcement is purely mono. Params (default): TGrind 0.9000, Grd/Out 0.0000, XOver 0.7400, BsDrive 1.0000, BsVoice 0.9500, BassOut 0.0000, SbDrive 0.2000, SbVoice 0.2000, SubOut 0.0000, Dry/Wet 1.0000.
- `DubSub` — DubSub is a fully featured bass doctor. Params (default): TGrind 0.9000, Grd/Out 0.0000, XOver 0.7400, BsDrive 1.0000, BsVoice 0.9500, BassOut 0.0000, SbDrive 0.2000, SbVoice 0.2000, SubOut 0.0000, Dry/Wet 1.0000.
- `Floor` — Floor is fake bottom octave for fun and profit! Params (default): Floor 0.0000, Drive 0.0000, Dry/Wet 1.0000.
- `Infrasonic` — Infrasonic is a very steep subsonic filter, built like Ultrasonic. No parameters.
- `FathomFive` — FathomFive is a way of supplementing extreme bass that’s not just EQ. Params (default): Root Note 1.0000, SubOctave 0.0000, Frequency 0.5000, Dry/Wet 1.0000.

### Lo-Fi

- `DeRez5` _(recommended)_ — DeRez5 adds new forms of derezzing. Params (default): Derez 1.0000, Trim 1.0000, Square 0.0000, Bright 1.0000, Bassis 0.0000, Output 1.0000.
- `DeBez` _(recommended)_ — DeBez gives you retro sampley textures! Params (default): DeBez 0.5000, DeRez 0.5000, Inv/Wet 1.0000.
- `HipCrush` _(recommended)_ — HipCrush is the de-rez section for ConsoleH. Params (default): Hi Freq 0.5000 High, High 0.0000, HiCrush 0.5000, MidFreq 0.5000 Mid, Mid 0.0000, MdCrush 0.5000, Lo Freq 0.5000 Low, Low 0.0000, LoCrush 0.5000, Dry/Wet 1.0000 Wet.
- `Flutter2` _(recommended)_ — Flutter2 is the flutter from ToTape7, standalone. Params (default): Flutter 0.5000, Speed 0.5000, Dry/Wet 1.0000.
- `Pockey2` _(recommended)_ — Pockey2 is more efficient, more intense lo-fi hiphop in a plugin. Params (default): DeFreq 0.0000, DeRez 11.9200, Dry/Wet 1.0000.
- `CrunchyGrooveWear` _(recommended)_ — CrunchyGrooveWear is a version of GrooveWear for more edge and distortion. Params (default): Freq 0.0640, Apply 1.0000.
- `GrooveWear` _(basic, recommended)_ — GrooveWear is for scrubbing highs off mechanically like a stylus would. Params (default): Wear 0.0640, Dry/Wet 1.0000.
- `Vibrato` _(basic, recommended)_ — Vibrato lets you vibrato, chorus, flange, and make odd FM noises. Params (default): Speed 0.3000, Depth 0.0000, FMSpeed 0.4000, FMDepth 0.0000, Inv/Wet 1.0000.
- `DeRez4` — DeRez4 perfects retro digital tones. Params (default): DownRez 0.6180, Bright 0.6180, Bassis 0.3820, Output 1.0000.
- `Pockey` — Pockey is 12 bit (and under) lo-fi hiphop in a plugin. Params (default): DeFreq 0.0000, DeRez 0.0000, Dry/Wet 1.0000.
- `Flutter` — Flutter is the flutter from ToTape6, standalone. Params (default): Flutter 0.0000.
- `DeRez2` — DeRez2 brings more authentic retro-digital hardware tones. Params (default): Rate 1.0000, Rez 1.0000, Hard 1.0000, Dry/Wet 1.0000.
- `BitGlitter` — BitGlitter is an old-sampler style digital lo-fi plugin. Params (default): Input 0.0000, Glitter 0.0000, Output 0.0000, Dry/Wet 1.0000.
- `DeRez` — DeRez is an analog-style bit and sample rate crusher with continuous adjustments. Params (default): Rate 1.0000, Rez 1.0000.
- `TapeBias` — TapeBias is a new piece of ToTape allowing you to underbias or overbias. Params (default): Bias 0.5000.
- `ChromeOxide` — ChromeOxide is an alternate path to vibey old tape sonics. Params (default): Intense 0.5000, Bias 0.5000.
- `Cojones` — Cojones is a new kind of distorty. Params (default): Breathy 1.0000, Cojones 1.0000, Body 1.0000, Output 1.0000, Dry/Wet 1.0000.
- `Bite` — Bite is an unusual edge-maker. Params (default): Bite 0.0000.
- `DustBunny` — DustBunny is unpredictable distorty noise, like dust wedged under your turntable needle. Params (default): Bunny 0.0000.

### Effects

- `TakeCare` _(recommended)_ — TakeCare is a lush chorus ensemble universe. Params (default): Speed 0.1500, Rando 0.5000, Depth 0.5000, Regen 0.5000, Derez 0.5000, Buffer 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `PitchNasty` _(recommended)_ — PitchNasty is a primitive pitch shifter that also has primitive time-stretch artifacts. Params (default): Note -12, Bend -12.0000, Grind -18, Feedbck 0.0000, Output 1.0000, Dry/Wet 1.0000.
- `Trianglizer` _(recommended)_ — Trianglizer is a special waveshaper that can transform sines into triangles. Params (default): Tri/Fat 0.0000, Dry/Wet 1.0000.
- `TremoSquare` _(basic, recommended)_ — TremoSquare is a squarewave tremolo effect that only switches on zero crossings. Params (default): Freq 0.5000, Dry/Wet 1.0000.
- `Tremolo` _(basic, recommended)_ — Tremolo is fluctuating saturation curves for a tubey tremolo. Params (default): Speed 0.5000, Depth 1.0000.
- `GlitchShifter` _(recommended)_ — GlitchShifter is a really gnarly, raw-sounding pitch shifter with a dose of insanity! Params (default): Note 0 semi, Trim 0.0000, Tighten 0.5000, Feedbck 0.0000, Dry/Wet 0.5000.
- `Energy2` _(recommended)_ — Energy2 is electrifying fixed-frequency treble boosts for high sample rate. Params (default): Hiss 0.0000, Glitter 0.0000, Rat 0.0000, Fizz 0.0000, Scrape 0.0000, Chug 0.0000, Yowr 0.0000, Snarl 0.0000, I/Dr/Wt 1.0000.
- `Dubly2` — Dubly2 is a key part of seventies sonics! Params (default): EncAmnt 0.5000, EncFreq 0.5000, TapeDrv 0.0000, DecAmnt 0.5000, DecFreq 0.5000, Out Pad 0.0000, Dry/Wet 1.0000.
- `GalacticVibe` — GalacticVibe is the stereo vibrato from the original Galactic reverb. Params (default): Drift 0.0000, Dry/Wet 1.0000.
- `CloudCoat` — CloudCoat is an array of blur effects with a taste for evil. Params (default): Select 8, Sustain 0.5000, Dry/Wet 1.0000.
- `Disintegrate` — Disintegrate is Discontinuity on steroids. Params (default): Top dB 100.0000, BufSize 0.5000, Layers 0.2000, Filter 0.5000, Dry/Wet 1.0000.
- `Fracture2` — Fracture2 is a wavefolder that allows more extreme disruption. Params (default): Drive 0.1000, Fractre 0, Thresh 0.5000, Output 1.0000, Dry/Wet 1.0000.
- `Dubly` — Dubly is a retro vibe and atmosphere maker. Params (default): Dubly 0.5000, Dry/Wet 1.0000.
- `Pafnuty2` — Pafnuty2 is a Chebyshev filter, that adds harmonics, and fits in the VCV Rack port. Params (default): Second 0.0000, Third 0.0000, Fourth 0.0000, Fifth 0.0000, Sixth 0.0000, Seventh 0.0000, Eighth 0.0000, Ninth 0.0000, Tenth 0.0000, Inv/Wet 0.0000.
- `ShortBuss` — ShortBuss chases second harmonic, to add or remove it. Params (default): ShortBs 0.0000, Dry/Wet 1.0000.
- `GuitarConditioner` — GuitarConditioner is like a Tube Screamer voicing without the squishiness and indistinctness. No parameters.
- `Aura` — Aura is a new kind of resonant lowpass EQ. Params (default): Voicing 0.5000, Dry/Wet 1.0000.
- `Gringer` — Gringer is a full-wave rectifier plugin, like a Green Ringer guitar effect. No parameters.
- `Exciter` — Exciter is an Aural Exciter plugin that can be both subtle and extreme. Params (default): Freq 0.6000, Excite 0.0000.
- `Energy` — Energy is electrifying fixed-frequency treble boosts. Params (default): Hiss 0.0000, Glitter 0.0000, Rat 0.0000, Fizz 0.0000, Scrape 0.0000, Chug 0.0000, Yowr 0.0000, Snarl 0.0000, I/Dr/Wt 1.0000.
- `Facet` — Facet is a new kind of hard clip that adds two sharp corners into the wave. Params (default): Facet 1.0000.
- `Fracture` — Fracture is a frequency multiplier/waveshaper with a soft disruption to the sound. Params (default): Drive 1.0000, Fracture 2, Out Lvl 0.0000 dB, Dry/Wet 1.0000.
- `PowerSag2` — PowerSag2 is my improved circuit-starve plugin, now with inverse effect! Params (default): Range 0.3000, Inv/Wet 0.0000.
- `PowerSag` — PowerSag is for emulating power supply limitations in analog modeling. Params (default): Depth 0.0000, Speed 0.3000.
- `Preponderant` — Preponderant is Airwindows Anti-Soothe, a strange tone intensifier and balancer. Params (default): Narrow 0.2500, Medium 0.5000, Wide 0.7500, Resonan 0.5000, Dry/Wet 1.0000.
- `Nikola` — Nikola is an Audio Tesla Coil plugin! Note: audio tesla coils don't sound nice. Params (default): Voltage 0.0000, Dry/Wet 1.0000.

### Stereo

- `Srsly3` _(basic, recommended)_ — Srsly3 is Srsly2, with a Nonlin control to analogify the filters. Params (default): Center 0.5000, Space 0.5000, Level 1.0000, Q 0.5000, Nonlin 0.5000, Dry/Wet 1.0000.
- `Srsly` _(recommended)_ — Srsly is a psychoacoustic stereo processor. Params (default): Center 0.0000, Space 0.0000, Level 1.0000, Q 0.5000, Dry/Wet 1.0000.
- `Wider` _(recommended)_ — Wider is Airwindows stereo space shaping. Params (default): Width 0.0000, Center 0.0000, Dry/Wet 1.0000.
- `ToVinyl4` _(recommended)_ — ToVinyl4 is a vinyl-mastering simulator bringing several vinyl-type colors. Params (default): Mid HiP 22.0000 hz, SideHiP 44.0000 hz, H Limit 0.3200, Gv Wear 0.0640.
- `Srsly2` — Srsly2 is a revisit of Srsly, to make the stereo widening more extreme. Params (default): Center 0.5000, Space 0.5000, Level 1.0000, Q 0.5000, Dry/Wet 1.0000.
- `SweetWide` — SweetWide is a strange grungy stereo widener. Params (default): Soar 0.0000, Un/Wide 0.5000.
- `StereoFX` — StereoFX is an aggressive stereo widener. Params (default): Wide 0.0000, MonoBs 0.0000, CSquish 0.0000.
- `AutoPan` — AutoPan is for getting some organic stereo movement into your mix. Params (default): Rate 0.1000, Phase 0.5000, Wide 0.0000, Dry/Wet 1.0000.
- `LRFlipTimer` — LRFlipTimer is a utility that swaps Left with Right every few (1-10) minutes. Params (default): Minutes 1.0000.
- `MSFlipTimer` — MSFlipTimer is a utility that swaps stereo with mono every few (1-10) minutes. Params (default): Minutes 1.0000.
- `Sidepass` — Sidepass is a simple utility plugin, a highpass on the side channel. Mono-maker. Params (default): Cutoff 0.0000.
- `SideDull` — SideDull is like Sidepass, but a lowpass on the side channel. Treble-centerer. Params (default): Cutoff 0.0000.

### Amp Sims

- `ChimeyDeluxe` _(recommended)_ — ChimeyDeluxe is a very flexible compressed DI conditioner. Params (default): Hell 0.5000, Fizz 0.5000, Pick 0.5000, Satan 0.5000, Danger 0.5000, Crtical 0.5000, H Meat 0.5000, L Meat 0.5000, Swing 0.5000, Rarely 0.5000.
- `PointyGuitar` _(basic, recommended)_ — PointyGuitar is a supremely adaptable instrument amp. Params (default): Drive 0.6000, Presnce 0.5000, High 0.8000, Mid 0.6000, Low 0.4000, Sub 0.5000, HSpeakr 0.7000, LSpeakr 0.4000, Gate 0.0000, Output 0.7000.
- `ChimeyGuitar` _(basic, recommended)_ — ChimeyGuitar is a supremely compressible instrument amp. Params (default): Compres 0.5000, Presnce 0.8000, High 0.7000, Mid 0.5000, Low 0.7000, Sub 0.4000, HSpeakr 0.8000, LSpeakr 0.4000, Speed 0.3000, Output 1.0000.
- `ChimeyGuitar2` — ChimeyGuitar2 is a wilder ChimeyGuitar. Params (default): Compres 0.6000, Presnce 0.8000, High 0.7000, Mid 0.5000, Low 0.7000, Sub 0.4000, HSpeakr 0.8000, LSpeakr 0.4000, Speed 0.8000, Output 0.8000.
- `Cabs2` — Cabs2 uses really phasey filters to be a speaker cabinet simulator. Params (default): Lowpass 1.0000, Hipass 0.0000.
- `GrindAmp` — GrindAmp is a heavier amp sim for many purposes. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `FireAmp` — FireAmp is a bright, loud, tubey amp sim for leads and dirt guitar. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `LeadAmp` — LeadAmp is an amp sim with a clear, penetrating, vocal tone. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `CrickBass` — CrickBass is a flexible but aggressive bass tone for dual pickups. Params (default): Drive 0.5000, Tone 0.5000.
- `Wolfbot` — Wolfbot is an aggressive Kalman bandpass with evil in its heart. No parameters.
- `LilAmp` — LilAmp is a tiny amp sim, like a little bitty practice amp without that much gain. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `MidAmp` — MidAmp is a clean amp sim meant to work like a loud Twin or something of that nature. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `BigAmp` — BigAmp is a very flexible amplike thing with a taste for the bizarre. Params (default): Gain 0.5000, Tone 0.5000, Output 0.8000, Dry/Wet 1.0000.
- `Cabs` — Cabs is an Airwindows guitar speaker cabinet simulator. Params (default): CabType HPStack, Intense 0.6600, Rm Loud 0.3300, CabSize 0.6600, OffAxis 0.3300, Output 0.5000.
- `BassDrive` — BassDrive is an old secret weapon, like a kind of bass amp. Params (default): Presnce 0.5000, High 0.5000, Mid 0.5000, Low 0.5000, Drive 0.5000.
- `BassAmp` — BassAmp is an old plugin with new tweaks, that gives some bass guitar tones. Params (default): High 0.7500, Dry 0.0000, Dub 0.7500, Sub 0.0000.

### Consoles

- `Console9Buss` _(recommended)_ — Console9Buss is just the summing from ConsoleX. Params (default): Pan 0.5000, Fader 0.5000.
- `Console9Channel` _(recommended)_ — Console9Channel is just the summing from ConsoleX. Params (default): Pan 0.5000, Fader 0.5000.
- `ConsoleLABuss` _(recommended)_ — ConsoleLABuss is the Airwindows take on the Quad Eight console. Params (default): Master 1.0000.
- `ConsoleLAChannel` _(recommended)_ — ConsoleLAChannel is the Airwindows take on the Quad Eight console. Params (default): Treble 0.5000, Mid 0.5000, Bass 0.5000, Pan 0.5000, Fader 0.5000.
- `ConsoleMCBuss` _(recommended)_ — ConsoleMCBuss is the initial, bright take on the MCI console. Params (default): Master 1.0000.
- `ConsoleMCChannel` _(recommended)_ — ConsoleMCChannel is the initial, bright take on the MCI console. Params (default): Treble 0.5000, MidFreq 0.2500, MidPeak 0.0000, Bass 0.5000, Pan 0.5000, Fader 0.5000.
- `Console0Channel` _(recommended)_ — Console0Channel is a radical, minimalist Console system. Params (default): Vol 0.5000, Pan 0.5000.
- `Console0Buss` _(recommended)_ — Console0Buss is a radical, minimalist Console system. Params (default): Vol 0.5000, Pan 0.5000.
- `PurestConsoleBuss` _(recommended)_ — PurestConsoleBuss is the most free from coloration Console system. No parameters.
- `PurestConsoleChannel` _(recommended)_ — PurestConsoleChannel is the most free from coloration Console system. No parameters.
- `Console8LiteChannel` — Console8LiteChannel is simplified Console8, working with just a single mix buss. Params (default): Fader 0.5000.
- `Console8LiteBuss` — Console8LiteBuss is simplified Console8, working with just a single mix buss. Params (default): Fader 0.5000.
- `PurestConsole3Buss` — PurestConsole3Buss is an alternate algorithm for minimalist Console. No parameters.
- `PurestConsole3Channel` — PurestConsole3Channel is an alternate algorithm for minimalist Console. No parameters.
- `PurestConsole2Buss` — PurestConsole2Buss is the distributed filter version of PurestConsole. No parameters.
- `PurestConsole2Channel` — PurestConsole2Channel is the distributed filter version of PurestConsole. No parameters.
- `Console7Buss` — Console7Buss adds anti-alias filtering and special saturation curves. Params (default): Master 1.0000.
- `Console7Channel` — Console7Channel adds anti-alias filtering and special saturation curves. Params (default): Fader 0.7720.
- `Console7Cascade` — Console7Cascade is a drop-in replacement for Console7Channel that allows for MUCH higher gain. Params (default): Cascade 0.3900.
- `Console7Crunch` — Console7Crunch shows you Cascade but with its EQ weighted for more edge and grit. Params (default): Crunch 0.3900.
- `Console6Buss` — Console6Buss is a new more intense Console that peaks at lower dB. Params (default): Input 1.0000.
- `Console6Channel` — Console6Channel is a new more intense Console that peaks at lower dB. Params (default): Input 1.0000.
- `PDBuss` — PDBuss is Console5 and PurestDrive, sittin’ in a tree… (i.e. both at once, on channels and buss) Params (default): Gain 1.0000, Drive 1.0000.
- `PDChannel` — PDChannel is Console5 and PurestDrive, sittin’ in a tree… (i.e. both at once, on channels and buss) Params (default): Gain 1.0000, Drive 1.0000.
- `Console5Buss` — Console5Buss is a richer, warmer Console system. Params (default): Input 1.0000.
- `Console5Channel` — Console5Channel is a richer, warmer Console system. Params (default): Input 1.0000.
- `Console5DarkCh` — Console5DarkCh is a richer, warmer Console system. Params (default): Input 1.0000.
- `C5RawBuss` — C5RawBuss is the original Console5 algorithm, with optional very gentle DC suppression. Params (default): Center 0.0000.
- `C5RawChannel` — C5RawChannel is the original Console5 algorithm, with optional very gentle DC suppression. Params (default): Center 0.0000.
- `AtmosphereBuss` — AtmosphereBuss is Console5 processing with powerful new acoustic distance effects. Params (default): Input 1.0000.
- `AtmosphereChannel` — AtmosphereChannel is Console5 processing with powerful new acoustic distance effects. Params (default): Input 1.0000.
- `Console4Buss` — Console4Buss is the original Airwindows ITB mixing system, a two-plugin mix buss replacement for DAWs. Params (default): Trim 1.0000.
- `Console4Channel` — Console4Channel is the original Airwindows ITB mixing system, a two-plugin mix buss replacement for DAWs. Params (default): Trim 1.0000.
- `EveryConsole` — EveryConsole is mix-and-match Airwindows Console. Params (default): ConType Retro Ch, InTrim 1.0000, OutTrim 1.0000.

### Noise

- `DeNoise` _(recommended)_ — DeNoise can work as a multiband gate and as a wild effect. Params (default): High 0.0000, HMid 0.0000, Mid 0.0000, LMid 0.0000, Bass 0.0000, Sub 0.0000, Speed 0.2500, Dry/Wet 1.0000.
- `TapeDust` _(recommended)_ — TapeDust is just a special treble-erode noise, a ‘slew noise’ plugin. Params (default): Dust 0.0000, Dry/Wet 1.0000.
- `Noise` — Noise is the Airwindows deep noise oscillator, as a sound reinforcer. Params (default): HighCut 0.5000, LowCut 0.5000, LShape 0.5000, Decay 1.0000, Distnc 0.0000, Dry/Wet 1.0000.
- `Texturize` — Texturize is a hidden-noise plugin for adding sonic texture to things. Params (default): Bright 0.5000, Punchy 0.5000, Dry/Wet 0.5000.
- `TexturizeMS` — TexturizeMS is a hidden-noise plugin for adding mid-side sonic texture to things. Params (default): BrightM 0.5000, PunchyM 0.5000, DryWetM 0.5000, BrightS 0.5000, PunchyS 0.5000, DryWetS 0.5000.
- `VoiceOfTheStarship` — VoiceOfTheStarship is a deep noise tone source. Params (default): Filter 0.5000, Algrthm 0.0000.
- `DarkNoise` — DarkNoise is an alternative method for producing bassier noise directly. Sound design stuff. Params (default): Freq 0.5000, Dark 1.0000, Output 1.0000, Dry/Wet 1.0000.
- `ElectroHat` — ElectroHat is a hi-hat tone generator that uses the original sound as a control voltage. Params (default): HiHat Syn Hat, Trim 0.5000, Bright 1.0000, Output 0.1000, Dry/Wet 1.0000.

### Dithers

- `PaulWide` _(recommended)_ — PaulWide is a highpassed TPDF wide dither. (quieter, airier AND wider) Params (default): Quant HD 24, DeRez 0.0000.
- `PaulDither` _(recommended)_ — PaulDither is a highpassed TPDF dither. (quieter, airier) Params (default): Quant HD 24, DeRez 0.0000.
- `TPDFDither` _(basic, recommended)_ — TPDFDither is TPDF dither. With 16/24 bit output and a DeRez control. Params (default): Quant HD 24, DeRez 0.0000.
- `Dark` — Dark is a wordlength reducer that gives your music a blacker backdrop. Params (default): Quant HD 24, DeRez 0.0000.
- `TPDFWide` — TPDFWide is still TPDF dither but better and wider. With 16/24 bit output and a DeRez control. Params (default): Quant HD 24, DeRez 0.0000.
- `NotJustAnotherDither` — NotJustAnotherDither is a next-gen wordlength reducers for 24 bit with DeRez. Params (default): Quant HD 24, DeRez 0.0000.
- `NotJustAnotherCD` — NotJustAnotherCD is a next-gen wordlength reducers for 16 bit with DeRez. No parameters.
- `Beam` — Beam is a wordlength reducer that tries to heighten sonority. Params (default): Quant HD 24, Focus 0.5000, DeRez 0.0000.
- `TapeDither` — TapeDither is TPDF dither with noise like reel-to-reel tape. Params (default): Quant HD 24, DeRez 0.0000.
- `SpatializeDither` — SpatializeDither is a high-performance clarity and accuracy dither. Params (default): Quant HD 24, DeRez 0.0000.
- `VinylDither` — VinylDither is a high-performance dither that converts digital noise to ‘groove noise’. Params (default): Quant HD 24, DeRez 0.0000.
- `DoublePaul` — DoublePaul is like PaulDither but more so. No parameters.
- `Ditherbox` — Ditherbox is a switchable selection of dithers in 16 and 24 bit, plus monitoring tools. Params (default): Type NJAD 24.
- `BuildATPDF` — BuildATPDF is a dither-making toolkit. Params (default): First 0.0000, Second 0.0000, Third 0.0000, Fourth 0.0000, Fifth 0.0000, Sixth 0.0000, Seventh 0.0000, Eighth 0.0000, Ninth 0.0000, Tenth 0.0000.
- `NodeDither` — NodeDither is adjusta-TPDF-dither, like a dither flanger. Can do Paul and Tape settings. Params (default): Node 4 samples, Phase Out.
- `StudioTan` — StudioTan is all the 'non-dither' dithers, up to date and convenient. Params (default): Quantzr ST 24.
- `DitherMeTimbers` — DitherMeTimbers is dark deep 'dither' in a 24 bit version. No parameters.
- `DitherMeDiskers` — DitherMeDiskers is dark deep 'dither' in a 16 bit version. No parameters.
- `RawTimbers` — RawTimbers is just the quantization from Dither Me Timbers… and the opposite, as a brightener. Params (default): Quant HD 24, DeRez 0.0000.
- `RawGlitters` — RawGlitters is just the quantization from Dither Me Timbers… and the opposite, as a brightener. Params (default): Quant HD 24, DeRez 0.0000.
- `NaturalizeDither` — NaturalizeDither is deterministic dither that uses Benford Realness calculations for each sample. No parameters.
- `HighGlossDither` — HighGlossDither is a hybrid between shiny dither and truncation! Unnatural science experiment. No parameters.
- `DitherFloat` — DitherFloat is a demo of floating point truncation: can also dither to 32 bit float output. Params (default): Offset 0, Dither 1.0000.

### Utility

- `SoftClock3` _(recommended)_ — SoftClock3 is a groove-oriented time reference. Params (default): Tempo 80 bpm, Count 6 6, Tuple 3 th note, Triplet 0, Flavor 0.5000.
- `BitShiftPan` _(recommended)_ — BitShiftPan gives you a gain and a pan control that are ONLY done using bit shifts. Params (default): Vol 0.5000, Pan 0.5000.
- `BitShiftGain` _(basic)_ — BitShiftGain is the ‘One Weird Trick’ perfect boost/pad, but in 6db increments ONLY. Params (default): BitShift 0 bits.
- `PurestGain` _(basic, recommended)_ — PurestGain is a high-res noise shaped gain, with smoothed fader. Params (default): Gain 0.0000 dB, Slow Fade 1.0000.
- `PurestDualPan` _(basic, recommended)_ — PurestDualPan is an updated PurestGain but as a dual pan. Params (default): L Vol 0.5000, L Pan 0.0000, R Vol 0.5000, R Pan 1.0000.
- `PurestFade` _(basic, recommended)_ — PurestFade is just like PurestGain, but for LONG fades. Params (default): Gain 0.0000 dB, Fade 1.0000.
- `Monitoring3` _(basic, recommended)_ — Monitoring3 is Monitoring, but with Ten Nines into Dark as the wordlength reducer. Params (default): Monitor Out24.
- `DubPlate2` _(recommended)_ — DubPlate2 is like an ITB mastering house for electronic music. Params (default): Input 0.5000, Treble 0.5000, Bass 0.5000.
- `DubPlate` _(basic)_ — DubPlate is ruthless dubplate mastering for electronic music. No parameters.
- `CansAW` _(recommended)_ — CansAW is Chris's Airwindows room, in a plugin, for headphone mixers. Params (default): seatPos Bypass.
- `Cans` _(recommended)_ — Cans is a room simulator for headphone mixers. Params (default): Room StudioC, Diffuse 0.5000, Damping 0.5000, Crossfd 0.5000, Dry/Wet 0.5000.
- `Golem` _(basic, recommended)_ — Golem lets you blend a stereo track of two mics on an amp. Params (default): Balance 0.0000, Offset 0.0000, Phase In/Mic.
- `SoftClock2` — SoftClock2 is a groove-oriented time reference. Params (default): Tempo 80 bpm, Count 6 6, Tuple 3 th note, Swing 0.0000, Peak 0.0000, Valley 0.0000, Accents 0.5000, Boost 0.5000, Speed 0.5000.
- `SoftClock` — SoftClock is a groove-oriented time reference. Params (default): Tempo 80, Count 6, Tuple 3, Swing 0.0000, BigBeat 0.0000, Accents 0.5000, Boost 0.5000, Speed 0.5000.
- `BitDualPan` — BitDualPan is dual pan that's almost BitShiftGain. Params (default): L Vol 0.5000, L Pan 0.0000, R Vol 0.5000, R Pan 1.0000.
- `Monitoring2` — Monitoring2 is Monitoring, but with Dark as the wordlength reducer. Params (default): Monitor Out24.
- `Monitoring` — Monitoring is your one-stop shop for final 2-buss work! Params (default): Monitor Out24.
- `EveryTrim` — EveryTrim is Left/Right, Mid/Side, and Master in one plugin. Params (default): Left 0.0000 dB, Right 0.0000 dB, Mid 0.0000 dB, Side 0.0000 dB, Master 0.0000 dB.
- `HermeTrim` — HermeTrim is Very Fine Adjustments, otherwise just like EveryTrim. Params (default): Left 0.0000 dB, Right 0.0000 dB, Mid 0.0000 dB, Side 0.0000 dB, Master 0.0000 dB.
- `SlewOnly` — SlewOnly is a mix check plugin that shows you only the extreme highs. No parameters.
- `SubsOnly` — SubsOnly is a mix check plugin that shows you only the extreme lows. No parameters.
- `PeaksOnly` — PeaksOnly is a transformative mix check tool. No parameters.
- `DCVoltage` — DCVoltage is literally a DC control voltage, in a plugin. Params (default): Voltage 0.0000.
- `EdIsDim` — EdIsDim is mid/side conversion utility plugins. Params (default): ediS/diM 0.5000.
- `MidSide` — MidSide is the other side of the EdIsDim mid/side conversion utility plugin. Params (default): Mid/Side 0.5000.
- `uLawEncode` — uLawEncode is a Console-like encode/decode pair, but much more extreme. Params (default): Gain 1.0000, Dry/Wet 1.0000.
- `uLawDecode` — uLawDecode is a Console-like encode/decode pair, but much more extreme. Params (default): Gain 1.0000, Dry/Wet 1.0000.
- `Balanced` — Balanced lets you run an XLR input into a stereo input, sum it to mono, and use it. Params (default): BitShift 0 bits.
- `Flipity` — Flipity is a channel phase flipper/swapper utility. Params (default): Flipity Dry.
- `MoNoam` — MoNoam gives you variations on mono, mid, and side in different channels. Params (default): Monotor Bypass.
- `VoiceTrick` — VoiceTrick lets you record vocals while monitoring over speakers. Params (default): Tone 1.0000.
- `DeCrackle` — DeCrackle isolates clicks and vinyl crackles to remove them. Params (default): Filter 0.5000, Window 0.5000, Thresld 0.5000, Surface 0.5000, Dry/Wet 1.0000.

### Unclassified

- `CStrip` — CStrip is an Airwindows channel strip. Params (default): Replace 0.5000, Brightness 0.5000, Detune 0.5000, Bigness 1.0000, Dry/Wet 1.0000.
- `Pafnuty` — Pafnuty is a Chebyshev filter, that adds harmonics. Params (default): Replace 0.5000, Brightness 0.5000, Detune 0.5000, Bigness 1.0000, Dry/Wet 1.0000.

## Tips

- Always set "effect" first; without it the plugin runs Galactic (a 100 %-wet reverb), which is usually not what you want on a track.
- Parameter names change with the effect. After choosing, list them: `muser plugin params ~/.clap/Airwindows\ Consolidated.clap --params '{"effect": "ToTape9"}'`. Writing a parameter of another effect fails with a list of the real names.
- Switching effect resets all parameters to that effect's defaults; parameters written in the same params object are applied after the switch.
- `Input Level` / `Output Level` (dB) exist for every effect. Some effects also have their own `Input` / `Output` (0–1); write the exact name you mean.
- Use the CLAP build. The VST3 build also switches effects, but its parameter names stay those of Galactic (Replace, Brightness, …) — they then just mean parameter 1, 2, …. The LV2 build cannot switch effects from muser.
- Parameters are 0–1 numbers, not dB or Hz: 0.5 is often 'neutral' (EQ, tilt) and 0 often 'off' (Tube, Sweeten, Density2's Density). Read the description and audition: `muser audition --effect '{"type":"plugin","path":"...","params":{"effect":"Tube"}}' --param TUBE --values 0,0.3,0.6`.
- Many reverbs here (Galactic, VerbTiny, kCathedral5…) default to fully or half wet. On a track lower Dry/Wet (or Wetness) to 0.2–0.35; on a send bus keep it at 1.
- Airwindows effects are designed to be stacked subtly: e.g. Console9Channel on every track plus Console9Buss on the bus, or a little ToTape9 on the master. One instance = one effect, so chain several plugin entries.
- The installed plugin version decides which effects exist; the list here is what version 1.2026.298 contains. Unknown names are rejected with suggestions.
