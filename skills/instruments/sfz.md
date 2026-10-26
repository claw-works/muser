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
- 部分 SFZ opcode（如 `<curve>`、`<effect>`）不支持，渲染时会在 stderr 提示数量，音色可能与原设计略有差异。
- `instrument_params` 支持两项（加载时生效，不能自动化）：
  - `"cc": { "105": 127 }`：设控制器值。很多音色库用控制器做开关（单音模式、选弦、齐奏音量、滤波、颤音深度…），
    muser 按这些值选择区域（`loccN`/`hiccN`）并折算 `volume_onccN` / `amplitude_onccN` / `pan_onccN`；
    没给的用 .sfz 里 `set_ccN` 的默认值。
  - `"keyswitch": "Staccato"`：选键位切换（`sw_last`）的奏法，可写 `sw_label` 名称、音高数字或音名；默认 `sw_default`。
  - `muser instruments "sfz:/path/to.sfz"` 会列出该音色声明的控制器（名称与默认值）和可选奏法。
- 触发方式按 .sfz 设计工作：`trigger=first` / `legato`（连奏采样，前一个键还按着时触发；clip 写 `"legato": true` 自动连起来，
  见 SKILLS.md 的连奏一节）、`release`（松键/抬踏板时的余音、琴弦共鸣）。
- 其它需要调整的地方可在轨上加效果器（如 `eq`、`reverb`）。
- 采样里的 LFO、滤波器、速度曲线等 opcode 暂不支持（加载时提示数量），控制器默认值让它们不起作用时影响不大。
- 免费音色库：freepats（钢琴、吉他等）、Versilian VSCO（管弦乐）、sfzinstruments.github.io。
- 路径是本机路径，工程换机器后需改成新位置。
- 钢琴类音色配合延音踏板：clip 的 `pedal` 字段，见 [pedal.md](../pedal.md)。
