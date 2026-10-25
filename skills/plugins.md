<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 外置插件：从陌生到会用

muser 不需要事先"适配"某个插件：任何 VST3 / CLAP / LV2 插件都能用同一套工具读出它的信息、
搜索参数、试听参数的作用。下面是遇到一个陌生插件时的标准流程（括号里是 MCP tool 名）。
所有命令需要 `--features host` 构建的 muser。

## 1. 找到插件（scan_plugins）

```bash
muser plugin scan                  # 本机标准目录里的全部插件
muser plugin scan --kind instrument
```

每个插件在独立进程里读取，崩溃或卡死的插件只会出现在失败列表，不影响 muser；结果有缓存，
`--refresh` 重新扫描。输出给出可以直接写进工程的 `path#id`。同一个插件常同时有 VST3 / CLAP / LV2
三种格式：优先用 CLAP，其次 VST3（LV2 版本暴露的参数可能不同或更少）。

## 2. 看结构（plugin_info）

```bash
muser plugin info "/usr/lib/clap/Surge XT.clap"
```

看类型（instrument / effect）、是否接收音符、输入输出、参数总数，以及**参数按名称首词的分组**
（如 Surge XT：`A(273) B(273) FX(210) …`），对大插件的结构一目了然。

## 3. 搜参数（list_plugin_params）

```bash
muser plugin params "/usr/lib/clap/Surge XT.clap" --filter "a filter 1"
#  A Filter 1 Type     0 – 1  当前 0.005 "Off"        id 1537728286
#  A Filter 1 Cutoff   0 – 1  当前 0.485 "523.25 Hz"  id 4092842705
```

- 大插件有数百个参数，**一定用 `--filter` 关键词搜索**（空格分隔，全部命中）：cutoff、resonance、attack、
  release、mix、drive、volume、type、wave……
- 引号里是插件自己显示的文字：设置参数时照这个格式写最可靠（`"2000 Hz"`、`"40 %"`、`"10.0 ms"`）。

## 4. 看选项（plugin_param_values）

```bash
muser plugin values "/usr/lib/clap/Surge XT.clap" "A Filter 1 Type"
#  "Off" "LP 12 dB" "LP 24 dB" "HP 12 dB" … （34 种）
```

选项类参数（滤波类型、波形、效果类型、开关）的全部可选文字。连续参数则给出一组刻度。

## 5. 试听（audition）

不知道一个参数调大调小会怎样，就试：

```bash
muser audition --instrument "plugin:/usr/lib/clap/Surge XT.clap" \
  --params '{"A Filter 1 Type": "LP 24 dB"}' \
  --param "A Filter 1 Cutoff" --values="200 Hz,1000 Hz,8000 Hz" --pitch 48
# A Filter 1 Cutoff   峰值dB  RMS dB  LUFS  重心Hz  低%  中%  高%  起音ms  余音s
# 200 Hz              -21.8   -26.2  -25.1    160  96.0  4.0  0.0    13   0.02
# 1000 Hz             -17.7   -28.5  -27.0    553  48.3 51.7  0.0     9   0.02
# 8000 Hz             -17.1   -29.6  -27.7   2937  58.7 37.5  3.8    11   0.02
```

每个取值渲染一段测试音（效果器用 `--effect '<效果器 JSON>'`，由 `--source` 乐器弹和弦送进去），报告：

| 指标 | 含义 |
|---|---|
| `lufs` / `rms_db` / `peak_db` | 响度、平均电平、峰值 |
| `centroid_hz` | 频谱重心：越高越亮 |
| `low/mid/high_pct` | 低 <250 Hz / 中 250–4000 Hz / 高 >4000 Hz 的能量占比 |
| `attack_ms` | 起音到峰值 90% 的时间 |
| `release_s` | 音符结束后衰减 40 dB 的时间（`+` 表示被渲染长度截断，加大 `--tail`） |

`--out <目录>` 会把每一版写成 wav，方便人听。内置乐器与效果器同样可以试听。

## 6. 写进工程

```json
{ "instrument": "plugin:/usr/lib/clap/Surge XT.clap",
  "instrument_params": { "A Filter 1 Type": "LP 24 dB", "A Filter 1 Cutoff": "800 Hz" } }
{ "type": "plugin", "path": "/usr/lib/clap/Surge XT Effects.clap", "params": { "FX Type": "Reverb 2" } }
```

参数名大小写不敏感；名字有重复时改用数字 id。`muser validate` 会加载插件检查每个参数名与取值，
写错时提示相近的名字。插件参数也可以自动化（见 [automation.md](automation.md)）。

## 常见情况

- **参数改了没效果**：常见原因是它依赖别的开关。例如 Surge XT 的初始音色里 Filter 1 是 `Off`，
  要先设 `"A Filter 1 Type"` 再调 cutoff。用 audition 对比可以很快发现"所有取值结果都一样"。
- **参数名是通用的**：有的插件参数叫 `FX Parameter 0..11`，含义随别的参数变化（如 Surge XT Effects 的
  `FX Type`）。先用 `--params` 设好那个参数再列：`muser plugin params <path> --params '{"FX Type": "Reverb 2"}'`，
  文字会变成 `"62.5 ms"`、`"1.68 s"` 这样有含义的值。
- **每次渲染结果略有不同**：不少合成器的振荡器自由相位或带随机漂移。需要可复现时找相关开关
  （Surge XT：`"A Osc 1 Retrigger": "On"`，2、3 号振荡器同理）。
- **插件改回了参数**：有的插件在第一次处理时重置内部状态。muser 激活插件后会先用静音预热，若插件没有
  采纳设定的参数就重发，仍不采纳时在 stderr 打印警告，说明是哪些参数。
- **文字解析不准**：muser 会把插件解析出的数值换回文字核对，对不上时沿范围查找显示为这段文字的取值；
  纯文字（非数字）找不到匹配会报错并提示用 `muser plugin values` 查看可写的文字。

## 把结论记下来

摸清一个插件后，把常用参数、可用取值、试听结论记在你自己的笔记或 skill 里（下次直接用）。
muser 之后会提供插件画像（profile）文件，按插件 id + 版本保存这些结论并可分享。
