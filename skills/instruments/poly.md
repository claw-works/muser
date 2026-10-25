<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：poly 合成器预设

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family poly`。

- 这些是 manguebyte 自带的 68 个合成器预设，按用途分 7 类。写法：`"instrument": "poly.<名称>"`。
- 按用途选：贝斯选 bass，主旋律选 lead，铺底选 pad，短促的琶音/拨奏选 pluck，和弦伴奏选 keys，铜管齐奏选 brass，转场音效选 fx。
- pad 类起音和释音都较慢，音符要长（2 拍以上），并提前一点开始。
- fx 类（riser / downlifter 等）通常放一个长音（4-8 拍）在段落转换前。

## 可调参数（instrument_params，所有 poly.* 通用）

在预设音色上覆盖，只写想改的项。各预设的当前值用 `muser instruments poly.<名称>` 查看。
以下预设没有启用滤波器，`cutoff` / `resonance` / `filter_env` 对它们无效（校验会拒绝）：`poly.deep_sub`、`poly.808_glide`、`poly.chiptune`、`poly.soft_sine_pad`、`poly.bell_pluck`、`poly.marimba`、`poly.glass_pluck`、`poly.kalimba`、`poly.fm_electric_piano`、`poly.drawbar_organ`、`poly.vibraphone`。

| 参数 | 范围 | 说明 |
|---|---|---|
| `attack` | 0 – 20 s | 音量起音时间。0.001-0.01 干脆；0.1-0.5 柔和；>1 慢慢浮现（pad） |
| `decay` | 0 – 20 s | 音量从峰值落到 sustain 的时间 |
| `sustain` | 0 – 1 | 按住时保持的音量比例。0 = 只有起音（pluck）；1 = 一直满音量（pad/风琴） |
| `release` | 0 – 20 s | 松开后余音长度。0.05-0.2 利落；0.5-2 有尾巴；>3 很长的铺底 |
| `cutoff` | 20 – 20000 Hz | 滤波截止频率：越低越闷越暗，越高越亮。贝斯 200-1500；lead 1500-6000 |
| `resonance` | 0 – 1 | 滤波谐振：截止频率处的突起。0.2-0.4 有个性；0.7+ 啸叫感（acid） |
| `filter_env` | -8 – 8 oct | 滤波包络把截止频率推开多少个八度。正值 = 起音时变亮再回落（'哇'声）；0 = 不动 |
| `filter_attack` | 0 – 20 s | 滤波包络起音时间 |
| `filter_decay` | 0 – 20 s | 滤波包络衰减时间：越短'哇'声越短促 |
| `filter_sustain` | 0 – 1 | 滤波包络保持比例 |
| `filter_release` | 0 – 20 s | 滤波包络释放时间 |
| `glide` | 0 – 5 s | 滑音时间；只在单音/连奏型预设上起作用（重叠的音符之间滑过去） |

### 调音示例

- 把 moog bass 变得更亮更有'哇'声

  ```json
  { "instrument": "poly.moog_bass", "instrument_params": {"cutoff": 900, "resonance": 0.5, "filter_env": 5} }
  ```

- pad 更慢地浮现、更长的尾巴

  ```json
  { "instrument": "poly.warm_analog_pad", "instrument_params": {"attack": 1.5, "release": 4.0} }
  ```

- lead 加滑音

  ```json
  { "instrument": "poly.mono_saw_lead", "instrument_params": {"glide": 0.12} }
  ```


## bass（15 个，推荐音域 24–60）

| instrument | 说明 |
|---|---|
| `poly.moog_bass` | Moog Bass：Two saws and a square an octave down through a ladder that snaps shut: round and punchy. |
| `poly.reese` | Reese：Two pairs of saws detuned against each other with a sine sub: the slow, phasing drum-and-bass growl. |
| `poly.wobble_bass` | Wobble Bass：Saw and square through a driven MS-20 filter that an LFO opens in eighths of the tempo: dubstep, ready to wobble. |
| `poly.deep_sub` | Deep Sub：A sine with a touch of triangle an octave up and a quick pitch punch: weight you feel more than hear. |
| `poly.fm_growl` | FM Growl：A sine frequency-modulated by another at its own pitch, the index falling after the attack and wobbling in sixteenths. |
| `poly.house_organ_bass` | House Organ Bass：A drawbar organ with its fundamental reinforced, a percussive decay and a key click: the bass of 90s house. |
| `poly.acid_line` | Acid Line：A saw through a resonant ladder its envelope opens and closes on every note; accents open it further, overlapping notes slide. |
| `poly.garage_square` | Garage Square：A square and a square sub through a gently enveloped ladder: the hollow bass of UK garage. |
| `poly.wavetable_growl` | Wavetable Growl：Folded and digital wavetables swept in eighths through a driven MS-20 filter: a modern, metallic growl. |
| `poly.synthpop_bass` | Synthpop Bass：Two slowly pulse-width-modulated squares through a ladder with a short envelope: the 80s sequencer bass. |
| `poly.funk_bass` | Funk Bass：Saw and pulse through an SEM-like filter set between low-pass and band-pass, with a quick, velocity-hungry envelope. |
| `poly.808_glide` | 808 Glide：A saturated sine with a falling pitch at the start and a long decay that slides between overlapping notes: the trap 808. |
| `poly.rolling_bass` | Rolling Bass：Three detuned saws and one an octave down, a short filter blip on every note: the off-beat trance bass. |
| `poly.rubber_bass` | Rubber Bass：A triangle and a sine sub through a resonant 12 dB ladder, gliding: round and bouncy. |
| `poly.talking_bass` | Talking Bass：Detuned saws and a square through a driven vowel filter that an LFO moves from vowel to vowel every beat: a bass that says "yoi". |

## lead（11 个，推荐音域 48–96）

| instrument | 说明 |
|---|---|
| `poly.trance_supersaw` | Trance Supersaw：Seven detuned saws spread wide and seven more an octave up: the classic trance lead; the wheel adds vibrato. |
| `poly.square_lead` | Square Lead：A square with a softer one an octave up, slowly pulse-width-modulated, with a vibrato that comes in late; it slurs. |
| `poly.sync_lead` | Sync Lead：A saw hard-synced to another, its pitch swept down by an envelope on every note: the tearing sync lead. |
| `poly.chiptune` | Chiptune：A bare 25% pulse with a blip of pitch at the start and a delayed vibrato: the sound of the game consoles. |
| `poly.hoover` | Hoover：Wide detuned saws and pulses an octave down that bend up into every note and slur: the rave hoover. |
| `poly.mono_saw_lead` | Mono Saw Lead：Three saws, one an octave down, through a ladder that pressure opens: the fat monophonic lead that glides. |
| `poly.whistle_lead` | Whistle Lead：A sine with a faint triangle an octave up, a slow attack and a late vibrato: soft, flute-like, gliding. |
| `poly.digital_lead` | Digital Lead：A stepped digital wavetable slowly scanned, with a spectral one a fifth up: glassy and modern. |
| `poly.talking_lead` | Talking Lead：Two saws through the vowel filter, which an LFO moves in eighths: a lead that talks. |
| `poly.screamer` | Screamer：Saw and pulse through an MS-20 high-pass and a driven, resonant MS-20 low-pass in a row: aggressive and loud. |
| `poly.future_lead` | Future Lead：Detuned saws and squares spread wide with a hint of pitch at the attack: the bright future-bass lead. |

## pad（12 个，推荐音域 36–84）

| instrument | 说明 |
|---|---|
| `poly.warm_analog_pad` | Warm Analog Pad：Two sets of detuned saws through a slowly opening ladder that breathes: the warm polysynth pad. |
| `poly.string_machine` | String Machine：Detuned saws and an octave above, their detune shimmering slowly, through a gentle 12 dB filter: the string ensemble. |
| `poly.glass_pad` | Glass Pad：The glass wavetable, slowly scanned and spread wide, with a sine an octave up: crystalline and airy. |
| `poly.dark_drone` | Dark Drone：Low detuned saws and a fifth, a sub and brown noise through a dark, driven ladder that moves very slowly. |
| `poly.choir_pad` | Choir Pad：The choir wavetable drifting between oo and ah, spread like an ensemble, with a faint vibrato: voices. |
| `poly.formant_pad` | Formant Pad：Saws through the vowel filter, which an envelope and a slow LFO carry from vowel to vowel: a choir-like formant pad. |
| `poly.pwm_pad` | PWM Pad：Two sets of pulses whose widths two LFOs move against each other: the lush, chorused pad of the 80s. |
| `poly.shimmer_pad` | Shimmer Pad：Saws, a harmonic wavetable an octave up and a sine a twelfth up, high-passed: bright and floating. |
| `poly.evolving_spectra` | Evolving Spectra：Spectral and resonant wavetables scanned by two slow LFOs through an SEM-like filter that opens over seconds. |
| `poly.soft_sine_pad` | Soft Sine Pad：Detuned sines and triangles an octave up, with a slow vibrato: pure and gentle. |
| `poly.comb_wash` | Comb Wash：Saws and pink noise through a comb tuned to the note, its damping drifting: a resonant, ambient wash. |
| `poly.sweep_pad` | Sweep Pad：Detuned saws and an octave down through a resonant ladder that a very slow LFO sweeps open and shut. |

## pluck（9 个，推荐音域 48–96）

| instrument | 说明 |
|---|---|
| `poly.house_pluck` | House Pluck：Detuned saws and a pulse with a filter that snaps shut: the short, bright pluck of house music. |
| `poly.bell_pluck` | Bell Pluck：A sine frequency-modulated at three and a half times its pitch, the index dying with the note: a bright bell. |
| `poly.marimba` | Marimba：A sine struck by a quick burst of FM at four times its pitch, dying fast: a wooden mallet. |
| `poly.plucked_string` | Plucked String：A burst of noise into a comb tuned to the note, damped: a plucked string, by Karplus and Strong's idea. |
| `poly.future_pluck` | Future Pluck：Seven wide saws and an octave above with a closing filter, a pitch flick and some saturation: the future-bass pluck. |
| `poly.glass_pluck` | Glass Pluck：The glass wavetable with a sine two octaves up, decaying: a delicate, chiming pluck. |
| `poly.trance_pluck` | Trance Pluck：Five detuned saws through a resonant ladder with a short envelope: the arpeggiated trance pluck. |
| `poly.pizzicato` | Pizzicato：A narrow pulse and a saw an octave down through an SEM-like band-pass that clicks open: plucked strings, synthetic. |
| `poly.kalimba` | Kalimba：A sine with a quick FM ping at six times its pitch and a quiet octave: a thumb piano. |

## keys（7 个，推荐音域 36–96）

| instrument | 说明 |
|---|---|
| `poly.polysynth_piano` | Polysynth Piano：Saw and triangle through a ladder that velocity opens, dying away like a struck string: the synth piano. |
| `poly.clavinet` | Clavinet：A narrow pulse and a saw through a key-tracked ladder band-pass with a snappy envelope: funky and percussive. |
| `poly.fm_electric_piano` | FM Electric Piano：A sine modulated at its own pitch that barks harder the harder it is struck, a faint tine and a tremolo: the FM electric piano. |
| `poly.drawbar_organ` | Drawbar Organ：A gospel registration of the organ wavetable with a key click and a light vibrato. |
| `poly.juno_keys` | Juno Keys：A saw and a pulse an octave down, slightly spread, through a ladder that settles after the attack: bright chord keys. |
| `poly.wurly` | Wurly：A saturated triangle with an octave that velocity brings in, and a tremolo: a reedy electric piano. |
| `poly.vibraphone` | Vibraphone：A sine and its fourth partial ringing a long time, with the motor's tremolo: metal bars. |

## brass（6 个，推荐音域 40–84）

| instrument | 说明 |
|---|---|
| `poly.synth_brass` | Synth Brass：Two pairs of saws through a ladder whose envelope swells after the attack, with a little scoop of pitch: polysynth brass. |
| `poly.brass_stab` | Brass Stab：Wide saws and a square with a filter that bites and closes: the short brass stab of 90s house. |
| `poly.jump_brass` | Jump Brass：A saw and a narrow pulse through an SEM-like low-pass with a swelling envelope: the bright, brassy stab of the 80s. |
| `poly.rave_chord_stab` | Rave Chord Stab：A minor chord of saws on every key with a snapping filter: the chord stab of old-school rave. |
| `poly.soft_horns` | Soft Horns：A saw and a pulse through a 12 dB ladder that opens slowly, with a late vibrato: mellow horns. |
| `poly.fanfare` | Fanfare：Bright saws and an octave above through an opening ladder: a full, triumphant brass section. |

## fx（8 个，推荐音域 36–84）

| instrument | 说明 |
|---|---|
| `poly.riser` | Riser：Wide saws and noise whose pitch climbs an octave and filter opens over five seconds: the build-up before the drop. |
| `poly.downlifter` | Downlifter：Wide saws and pink noise falling an octave and closing over four seconds: the release after the peak. |
| `poly.noise_sweep` | Noise Sweep：White noise through a resonant band-pass that an LFO sweeps up and down: a whoosh. |
| `poly.laser_zap` | Laser Zap：A saw and a sine whose pitch falls three octaves in a flash: a laser. |
| `poly.random_bleeps` | Random Bleeps：A pulse whose pitch and filter a sample-and-hold sets at random eight times a second: a computer thinking. |
| `poly.wind` | Wind：Pink noise through two band-passes side by side, each drifting on its own slow LFO: wind. |
| `poly.impact_boom` | Impact Boom：A sine an octave down falling two octaves, with a burst of noise and saturation: a cinematic hit. |
| `poly.siren` | Siren：A square whose pitch an LFO swings a fifth up and down: an alarm. |
