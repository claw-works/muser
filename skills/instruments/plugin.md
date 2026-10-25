<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：外置 VSTi（第三方虚拟乐器）

把本机安装的 VST3 / CLAP / LV2 乐器插件当作音源：

```json
{
  "name": "lead",
  "instrument": "plugin:/usr/lib/clap/Surge XT.clap",
  "instrument_params": { "A Filter 1 Type": "LP 24 dB", "A Filter 1 Cutoff": "800 Hz", "A Osc 1 Retrigger": "On" },
  "automation": [
    { "target": "instrument", "param": "A Filter 1 Cutoff", "shape": "points", "points": [[0, "200 Hz"], [16, "4000 Hz"]] }
  ],
  "clips": [ ... ]
}
```

- 语法：`plugin:<绝对路径>`；文件里有多个插件时用 `plugin:<路径>#<插件id>`（`muser plugin scan` 直接给出这个写法）。
- `instrument_params` 用插件参数**名称**做键（大小写不敏感，也可写数字 id）；值写插件显示的文字（`"800 Hz"`、`"LP 24 dB"`）、
  数字（插件自动化刻度：CLAP/LV2 为声明范围，VST3 为 0–1）或 true/false。写错名称时会提示相近的参数名。
- 参数可以自动化，写法与内置乐器相同，`param` 写插件参数名。
- 音符 `pitch` 是 MIDI 音高；插件自己决定音色与音域。`accent` 对插件无效。
- 需要用 `cargo build -p muser-cli --features host` 构建的 muser，否则报 HostDisabled。
- `muser validate` 会加载插件检查参数名与取值；渲染时插件以工程的采样率激活。
- 加载插件会运行第三方代码，只使用可信的本机插件。

陌生插件怎么摸清楚：见 [plugins.md](../plugins.md)。
