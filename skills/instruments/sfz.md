<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：SFZ 采样音色库

用 muser 内置的 SFZ 采样器（manguebyte-sampler）播放 `.sfz` 音色库，不需要任何外部插件。

```json
{ "name": "piano", "instrument": "sfz:/path/to/Piano.sfz", "clips": [ ... ] }
```

- 语法：`sfz:<.sfz 文件路径>`；采样文件按 .sfz 里的相对路径从其所在目录读取（支持 WAV、FLAC 等）。
- 路径可以是绝对路径，也可以是相对音色库的路径（如 `sfz:SalamanderGrandPiano-SFZ+FLAC-V3+20200602/SalamanderGrandPiano-V3+20200602.sfz`）：
  相对路径在环境变量 `MUSER_SFZ_PATH`（音色库根目录，`:` 分隔，Windows 用 `;`）里查找，根目录下两层以内的子目录也会找，
  这样同一份工程在不同机器上都能用。分享的工程优先用相对路径。
- 音符 `pitch` 用 MIDI 音高；音域、力度层、循环、释放采样都由 .sfz 文件定义。最多 32 个同时发声。
- `muser validate` 会检查 .sfz 文件是否存在；采样缺失或格式错误会在渲染时报出。
- 支持的表现力 opcode：滤波器（`fil_type` `cutoff` `resonance` `fil_keytrack` `fil_veltrack`）与滤波包络（`fileg_*`）；
  按控制器**实时**调制的 `<opcode>_onccN`（音量、声像、音高、截止频率、共振、两个包络的各级），控制器的值来自工程里的
  `instrument_params.cc`，没写时用 .sfz 的 `set_ccN`。`amplitude_onccN` 按 SFZ 的惯例是乘法，在加载时按控制器值折成固定振幅。
  力度交叉淡化（`xfin_lovel` 等）与力度曲线 `amp_velcurve_N` 按力度切段近似（每 8 级一段）；`*noise` 白噪声、
  头部后同一行的 `#include` 也能读。
- 不支持的（LFO `lfoN_*` `amplfo_*` 等、音高包络 `pitcheg_*`、`offset_random`、`<curve>`、`<effect>`、区域 EQ `eqN_*` 等）
  渲染时在 stderr 列出名字与次数，音色可能与原设计略有差异（如没有颤音）。
- `instrument_params` 支持两项（加载时生效，不能自动化）：
  - `"cc": { "105": 127 }`：设控制器值。很多音色库用控制器做开关（单音模式、选弦、齐奏音量、滤波、颤音深度…），
    muser 按这些值选择区域（`loccN`/`hiccN`）并折算 `volume_onccN` / `amplitude_onccN` / `pan_onccN`；
    没给的用 .sfz 里 `set_ccN` 的默认值。
  - `"keyswitch": "Staccato"`：选键位切换（`sw_last`）的奏法，可写 `sw_label` 名称、音高数字或音名；默认 `sw_default`。
  - `muser instruments "sfz:/path/to.sfz"` 会列出该音色声明的控制器（名称与默认值）和可选奏法。
- 触发方式按 .sfz 设计工作：`trigger=first` / `legato`（连奏采样，前一个键还按着时触发；clip 写 `"legato": true` 自动连起来，
  见 SKILLS.md 的连奏一节）、`release`（松键/抬踏板时的余音、琴弦共鸣）。
- 其它需要调整的地方可在轨上加效果器（如 `eq`、`reverb`）。
- 揉弦、颤音深度常由 CC1（mod wheel）控制：`instrument_params: { "cc": { "1": 64 } }` 打开一半，0 关掉。
- 免费音色库：freepats（钢琴、吉他等）、Versilian VSCO（管弦乐）、sfzinstruments.github.io。
- 路径是本机路径，工程换机器后需改成新位置。
- 钢琴类音色配合延音踏板：clip 的 `pedal` 字段，见 [pedal.md](../pedal.md)。
