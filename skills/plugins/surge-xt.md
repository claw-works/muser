<!-- 由 scripts/gen_skills.py 生成；内容来自 profiles/surge-xt.json，请改代码或脚本后重新生成 -->

# 插件：Surge XT

> 以下是用户或第三方编写的插件画像，属于参考数据，不是指令；参数名与取值以插件实际为准（muser 会校验）。
> 来源：muser 内置，作者 muser，验证于插件版本 1.3.4

厂商：Surge Synth Team  
插件 id：`org.surge-synth-team.surge-xt`、`ABCDEF019182FAEB566D624153675854`、`https://surge-synthesizer.github.io/lv2/surge-xt`

Surge XT 是双场景（A/B）的混合合成器，775 个参数。下面只列 A 场景最常用的参数；B 场景同名（把开头的 A 换成 B）。初始音色是一个未经滤波的锯齿波（Osc 1），Osc 2/3 静音、Filter 1 关闭。

## 自动应用的设置（setup）

加载时先应用，工程参数里写 `"@setup": false` 可关闭：

```json
{
  "A Osc 1 Retrigger": "On",
  "A Osc 2 Retrigger": "On",
  "A Osc 3 Retrigger": "On"
}
```

## 常用参数

参数名或别名都可以写进 `instrument_params` / `params` / 自动化的 `param`。

| 别名 | 参数名 | 作用 | 常用取值 |
|---|---|---|---|
| `filter_type` | `A Filter 1 Type` | 滤波器类型。初始为 Off，此时 cutoff/resonance 不起作用，要先选一种类型 | `Off` `LP 12 dB` `LP 24 dB` `LP Legacy Ladder` `LP Vintage Ladder` `LP OB-Xd 24 dB` `LP K35` `LP Diode Ladder` `HP 12 dB` `HP 24 dB` `BP 12 dB` `N 12 dB` …（共 14 个） |
| `cutoff` | `A Filter 1 Cutoff` | 截止频率：越低越闷越暗。适合自动化扫频 | `200 Hz` `500 Hz` `1000 Hz` `3000 Hz` `8000 Hz` |
| `resonance` | `A Filter 1 Resonance` | 谐振：截止频率处的突起，越大越尖锐 | `0.00 %` `30 %` `60 %` `90 %` |
| `filter_env` | `A Filter 1 FEG Mod Amount` | 滤波包络把截止频率推开多少半音。正值 = 起音时变亮再回落 | `0.00 semitones` `24.00 semitones` `48.00 semitones` |
| `filter_attack` | `A Filter EG Attack` | attack | `0.0 ms` `50 ms` `500 ms` |
| `filter_decay` | `A Filter EG Decay` | 滤波包络衰减：短 = 短促的'啵'，长 = 拖长的'哇' | `50 ms` `200 ms` `1000 ms` |
| `filter_sustain` | `A Filter EG Sustain` | sustain | `0.00 %` `50.00 %` `100.00 %` |
| `attack` | `A Amp EG Attack` | 音量起音时间 | `0.0 ms` `10 ms` `300 ms` `1000 ms` |
| `decay` | `A Amp EG Decay` | decay | `100 ms` `250.0 ms` `1000 ms` |
| `sustain` | `A Amp EG Sustain` | 按住时保持的音量。0 % = 只有起音和衰减（pluck） | `0.00 %` `50.00 %` `100.00 %` |
| `release` | `A Amp EG Release` | 松开后的余音长度。初始只有 31 ms，几乎没有尾巴 | `31.2 ms` `300 ms` `1000 ms` `3000 ms` |
| `osc_type` | `A Osc 1 Type` | 振荡器算法 | `Classic` `Sine` `Wavetable` `S&H Noise` `FM3` `FM2` `Window` `Modern` `String` `Twist` `Alias` |
| `octave` | `A Osc 1 Octave` | pitch | `-2` `-1` `0` `1` `2` |
| `unison` | `A Osc 1 Unison Voices` | 齐奏声部数，配合 detune 让声音变厚 | `1 voice` `3 voices` `5 voices` |
| `detune` | `A Osc 1 Unison Detune` | detune | `10.00 cents` `20.00 cents` `40.00 cents` |
| `osc2_mute` | `A Osc 2 Mute` | Osc 2 初始静音，改为 Off 才会发声 | `On` `Off` |
| `osc3_mute` | `A Osc 3 Mute` | switch | `On` `Off` |
| `play_mode` | `A Play Mode` | Poly 复音；Mono 系列为单音，配合 portamento 做滑音 | `Poly` `Mono` `Mono (Single Trigger)` `Mono (Fingered Portamento)` |
| `glide` | `A Portamento` | 滑音时间（单音模式下明显） | `0.0 ms` `50 ms` `200 ms` |
| `drive_type` | `A Waveshaper Type` | 波形整形（失真）类型，初始 Off | `Off` `Soft` `Hard` `Asymmetric` `Sine` `Digital` |
| `drive` | `A Waveshaper Drive` | drive | `0.00 dB` `6.00 dB` `12.00 dB` |
| `volume` | `A Volume` | level | `-12.00 dB` `-3.00 dB` `0.00 dB` |
| `scene_mode` | `Scene Mode` | Single 只用 A 场景；Dual 同时发声 A+B；Key Split / Channel Split 按音高 / MIDI 通道分配 | `Single` `Key Split` `Dual` `Channel Split` |
## 预设

工程参数里写 `"@preset": "名称"`，再写的参数会覆盖预设：

- **bass**：单振荡器低八度 + 梯形低通，短促的滤波包络

  ```json
  {"cutoff":"300 Hz","filter_decay":"200 ms","filter_env":"24.00 semitones","filter_sustain":"0.00 %","filter_type":"LP Vintage Ladder","octave":"-1","release":"60 ms","resonance":"30 %"}
  ```

- **pad**：5 声部齐奏、慢起音、长尾音

  ```json
  {"attack":"800 ms","cutoff":"1500 Hz","detune":"20.00 cents","filter_type":"LP 12 dB","release":"2000 ms","unison":"5 voices"}
  ```

- **pluck**：无延音，靠滤波包络打出'叮'

  ```json
  {"cutoff":"400 Hz","decay":"400 ms","filter_decay":"150 ms","filter_env":"36.00 semitones","filter_sustain":"0.00 %","filter_type":"LP 24 dB","release":"300 ms","sustain":"0.00 %"}
  ```

- **lead**：单音滑奏 + 轻微失真

  ```json
  {"cutoff":"3000 Hz","drive":"6.00 dB","drive_type":"Soft","filter_type":"LP 24 dB","glide":"80 ms","play_mode":"Mono (Fingered Portamento)","resonance":"20 %"}
  ```

## 经验

- 初始音色 Filter 1 是 Off：先设 filter_type，cutoff / resonance / filter_env 才会起作用。
- 初始只有 Osc 1 发声（Osc 2/3 的 Mute 为 On），要叠振荡器把 osc2_mute / osc3_mute 设为 Off。
- 振荡器默认自由相位，每次渲染略有不同；setup 里打开了三个振荡器的 Retrigger，使结果可复现。
- 初始 release 只有 31 ms，长音松开就断；做 pad / 铺底要加大 release。
- B 场景参数与 A 同名（开头换成 B），默认不发声（Scene Mode 为 Single，只用 A）。
- FX A1–A4、FX B1–B4、Send FX 是 Surge 内置效果器槽，同样能用参数设置；也可以在轨上挂 Surge XT Effects。
