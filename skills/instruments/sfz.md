<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：SFZ 采样音色库

用 muser 内置的 SFZ 采样器（manguebyte-sampler）播放 `.sfz` 音色库，不需要任何外部插件。

```json
{ "name": "piano", "instrument": "sfz:/path/to/Piano.sfz", "clips": [ ... ] }
```

- 语法：`sfz:<.sfz 文件路径>`；采样文件按 .sfz 里的相对路径从其所在目录读取（支持 WAV、FLAC 等）。
- 音符 `pitch` 用 MIDI 音高；音域、力度层、循环、释放采样都由 .sfz 文件定义。最多 32 个同时发声。
- `muser validate` 会检查 .sfz 文件是否存在；采样缺失或格式错误会在渲染时报出。
- 部分 SFZ opcode（如 `<curve>`、`<effect>`）不支持，渲染时会在 stderr 提示数量，音色可能与原设计略有差异。
- 音色由 .sfz 文件决定，暂不支持 `instrument_params`；需要调整时可在轨上加效果器（如 `eq`、`reverb`）。
- 免费音色库：freepats（钢琴、吉他等）、Versilian VSCO（管弦乐）、sfzinstruments.github.io。
- 路径是本机路径，工程换机器后需改成新位置。
