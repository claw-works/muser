# muser Skill：如何用 muser 编曲

> 写给 **AI agent** 的操作手册：把用户的音乐意图翻译成 muser 工程 JSON，再渲染成音频。
> 这里是总入口；每个乐器类别、每个效果器的细节在 [`skills/`](skills/) 下的子 skill 里，按需查阅。

## 你的角色

用户用自然语言描述想要的音乐（"做一个 90bpm 的 lo-fi 节拍，钢琴+鼓"）。你的工作：

1. 选乐器、写音符、配效果器，产出一个 **Project JSON**。
2. 先 `validate`，再渲染成 WAV。
3. 根据反馈**增量修改** JSON，重新渲染。

你只在 Project JSON 这一层工作，不需要懂底层 DSP。

## 先查再写：可发现性

不要凭记忆写乐器名和效果器参数，先查：

| 想知道 | CLI | MCP tool |
|---|---|---|
| 有哪些乐器 | `muser instruments [--family F]` | `list_instruments` |
| 某乐器的音高约定、音域、复音数、**可调参数与预设当前值** | `muser instruments <名称>` | `describe_instrument` |
| 有哪些效果器 | `muser effects` | `list_effects` |
| 某效果器的参数、范围、默认值、示例 | `muser effects <名称>` | `describe_effect` |
| 工程是否合法、错在哪 | `muser validate <file>` | `validate_project` |
| 本机有哪些外置插件 | `muser plugin scan` | `scan_plugins` |
| 插件的参数、选项文字 | `muser plugin params <插件> --filter <词>`、`muser plugin values <插件> <参数>` | `list_plugin_params`、`plugin_param_values` |
| 某个参数到底起什么作用 | `muser audition ... --param <名> --values ...` | `audition` |
| 插件有没有现成的使用经验（别名、预设） | `muser plugin profile show <插件> --markdown` | `get_profile` / `list_profiles` |

加 `--json` 得到机器可读输出。`validate` 会指出越界参数和未知乐器的具体位置，例如
`tracks[0].effects[0]: reverb.mix = 1.8 超出范围 [0, 1]`。

## 核心心智模型

- **Project**：一个 `tempo`（速度）+ 若干 **Track**（音轨）+ 可选的 `master_effects`。
- **Track**：一件 `instrument`（乐器）+ 可选的 `instrument_params`（调音色）+ 可选的 `effects` + 可选的 `automation`（随时间变化）+ 若干 **Clip**（片段）。
- **Clip**：在时间线上有 `start` 和 `length`，里面是若干 **Note**（音符）。
- **时间单位是拍 (beats)**，`1.0` = 一个四分音符，与 tempo 无关。
- Clip 的 `start` 从**工程开头**算；Note 的 `start` 从**所在 clip 的开头**算。

## 时间速查（4/4 拍）

| 音符 | 全音符 | 二分 | 四分 | 八分 | 十六分 | 一小节 |
|---|---|---|---|---|---|---|
| beats | 4.0 | 2.0 | 1.0 | 0.5 | 0.25 | 4.0 |

## 音高速查

- 旋律乐器用 **MIDI 音高**：`60` = 中央 C (C4)，+12 升八度，+1 升半音。
  `C4=60 D4=62 E4=64 F4=65 G4=67 A4=69 B4=71 C5=72`；贝斯常用 `C2=36`、`E1=28`。
- 鼓组（`drums.*`）用 **GM 鼓键**：36=底鼓, 38=军鼓, 39=拍手, 42=闭镲, 46=开镲, 49=吊镲, 51=叮叮镲。
  完整键位与每套鼓支持的键见 [skills/instruments/drums.md](skills/instruments/drums.md)。
- `velocity`（力度）0-1，默认 0.8。
- `accent`（重音）true/false，默认 false。鼓组与 `bass.acid*` 的重音更响更亮（acid 的 303 味道主要靠它）；其它乐器忽略。

## 乐器索引（共 115 个内置 + SFZ 音色库 + 外置 VSTi）

| 类别 | 子 skill | 代表 |
|---|---|---|
| 鼓组 | [drums.md](skills/instruments/drums.md) | `drums.808` `drums.909` `drums.acoustic` `drums.trap` `drums.lofi` … 10 套 |
| 合成器预设 | [poly.md](skills/instruments/poly.md) | 68 个：`poly.moog_bass` `poly.trance_supersaw` `poly.warm_analog_pad` `poly.juno_keys` … |
| 通用合成器 | [synth.md](skills/instruments/synth.md) | `synth.poly` `synth.fm` |
| 吉他 | [guitar.md](skills/instruments/guitar.md) | `guitar.nylon` `guitar.steel` |
| 贝斯 | [bass.md](skills/instruments/bass.md) | `bass.finger` `bass.slap` `bass.acid` |
| 风琴 | [organ.md](skills/instruments/organ.md) | `organ.jazz` `organ.gospel` `organ.full` `organ.ballad` |
| 打击旋律 | [mallet.md](skills/instruments/mallet.md) | `mallet.marimba` `mallet.vibraphone` `mallet.kalimba` … |
| 弦乐 | [strings.md](skills/instruments/strings.md) | `strings.violin` `strings.cello` `strings.ensemble` `*_pizz` … |
| 管乐 | [wind.md](skills/instruments/wind.md) | `wind.flute` `wind.sax` `wind.trumpet` … |
| 人声 | [choir.md](skills/instruments/choir.md) | `choir.aah` `choir.ooh` `choir.solo` |
| SFZ 音色库 | [sfz.md](skills/instruments/sfz.md) | `sfz:/path/to/Piano.sfz`（内置采样器，无需插件） |
| 外置 VSTi | [plugin.md](skills/instruments/plugin.md) | `plugin:/usr/lib/vst3/Surge XT.vst3` |

所有内置音源都由 manguebyte 实时合成（物理建模、模态合成、FM、减法合成等），不依赖系统 MIDI 音源。

## 调音色：instrument_params

每个内置乐器都开放一组常用参数，在所选乐器/预设的音色**之上覆盖**，只写想改的项：

```json
{ "instrument": "poly.moog_bass", "instrument_params": { "cutoff": 900, "resonance": 0.5, "release": 0.15 } }
{ "instrument": "strings.cello",  "instrument_params": { "attack": 0.4, "vibrato_depth": 25 } }
{ "instrument": "organ.jazz",     "instrument_params": { "drawbars": "838000000", "percussion": "second" } }
{ "instrument": "drums.808",      "instrument_params": { "kick": { "tune": -3, "decay": 1.8 }, "snare": { "snap": 0.8 } } }
```

- 参数名在各乐器间统一：`attack` / `decay` / `sustain` / `release`（秒）、`tone`（Hz，最终明暗）、
  `cutoff` / `resonance`（滤波）、`vibrato_rate`（Hz）/ `vibrato_depth`（音分）。用真实单位，不是 0-1。
- 每个乐器有哪些参数、范围、**该预设当前值**：`muser instruments <名称>`（MCP `describe_instrument`）。
  先看当前值再改，比凭空写数更稳。
- 鼓组按鼓件设置，键用 GM 鼓键数字或别名（kick / snare / clap / closed_hat / open_hat / crash / ride …）。
- 写错参数名、越界、类型不对，`validate` 都会指出，例如
  `instrument_params 无效: cutoff: 99999 超出范围 [30, 8000] Hz`。
- 各类参数表与调音示例见对应的乐器子 skill。
- 外置插件（`plugin:` 乐器、`plugin` 效果器）同样用 `instrument_params` / `params`，键是插件参数名，
  值写插件显示的文字（如 `"2000 Hz"`、`"LP 24 dB"`）。遇到陌生插件按 [skills/plugins.md](skills/plugins.md) 的流程：
  scan → info → 搜参数 → 看选项 → 试听 → 写进工程 → 存成画像。
- 有画像的插件（`muser plugin profile list`）可以用短别名（`"cutoff"`）和预设（`"@preset": "pad"`），
  画像的 `setup` 会自动应用（`"@setup": false` 关闭）。内置画像：[Surge XT](skills/plugins/surge-xt.md)、
  [Surge XT Effects](skills/plugins/surge-xt-effects.md)。完整音色可以存成状态文件，用 `"@state": "<文件>"` 加载。

## 让声音动起来：automation

参数可以随时间变化：滤波扫频、淡入淡出、声像摆动、带种子的随机游走。写在音轨或工程的 `automation` 里：

```json
"automation": [
  { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp" },
  { "target": "pan", "shape": "lfo", "wave": "sine", "period_beats": 8, "min": -0.6, "max": 0.6 },
  { "target": "effects[0]", "param": "mix", "shape": "random", "style": "walk", "step_beats": 1, "min": 0.1, "max": 0.5 }
]
```

- `target`：音轨上 `instrument` / `effects[i]` / `gain_db` / `pan` / `sends.<总线>`；总线上 `effects[i]` / `gain_db` / `pan` / `sends.<总线>`；
  工程上 `master_gain_db` / `master_effects[i]`。
- `shape`：`points`（时间点曲线）、`lfo`（周期摆动）、`random`（随机）。时间单位都是拍。
- `random` 由工程 `seed` 决定：同一 seed 每次渲染一样，换 seed 换一种变化——可以多渲染几版挑一个。
- 哪些参数能自动化：`muser instruments <名称>` 里标了 `[可自动化]` / `[可自动化·下个音生效]`；插件参数用插件参数名，取值可写文字。
- 完整说明与示例：[skills/automation.md](skills/automation.md)。

## 混音路由：总线、发送、侧链

默认每条轨直接进主总线。要让几条轨"一起处理"或"共用一个效果"，用工程的 `buses`：

```json
"buses": [
  { "name": "reverb", "effects": [{ "type": "eq", "shape": "highpass", "frequency": 300 }, { "type": "reverb", "mix": 1.0, "decay_seconds": 3 }] },
  { "name": "drums",  "effects": [{ "type": "compressor", "threshold_db": -24, "ratio": 4, "attack_ms": 15 }] }
],
"tracks": [
  { "name": "kick", "instrument": "drums.909", "output": "drums" },
  { "name": "pad",  "instrument": "poly.warm_analog_pad", "sends": { "reverb": -6 },
    "effects": [{ "type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick" }] }
]
```

- `output`：整份信号去哪（`"master"` 默认，或总线名）。鼓组编组、分组处理用它。
- `sends`：额外送一份到效果总线，`{总线: dB}`，推子后发送。共用混响/延迟用它，总线里效果器 `mix: 1`。
- `sidechain`：compressor 由另一条轨/总线的信号触发——底鼓压 pad/贝斯的泵感。触发轨 muted 也照样触发。
- 总线可以有自己的 `effects` / `gain_db` / `pan` / `output` / `sends` / `automation`，总线可以送进总线，不能成环。
- `muser routing <工程>` 打印信号流，改完先看一眼。完整说明与混音经验：[skills/routing.md](skills/routing.md)。

## 效果器索引

效果器写在 `track.effects`（轨级，在该轨增益/声像之前）、总线的 `effects` 或 `master_effects`（主总线，在主增益之前），
按数组顺序串联。每个参数都可省略，`{ "type": "reverb" }` 就是一个可用的混响。

| type | 作用 | 子 skill |
|---|---|---|
| `eq` | 均衡/滤波一个频段 | [eq.md](skills/effects/eq.md) |
| `compressor` | 压缩动态 | [compressor.md](skills/effects/compressor.md) |
| `chorus` | 加厚、加宽 | [chorus.md](skills/effects/chorus.md) |
| `phaser` | 扫频流动感 | [phaser.md](skills/effects/phaser.md) |
| `flanger` | 金属感扫动 | [flanger.md](skills/effects/flanger.md) |
| `delay` | 回声 | [delay.md](skills/effects/delay.md) |
| `reverb` | 空间感 | [reverb.md](skills/effects/reverb.md) |
| `limiter` | 限峰防爆（主总线最后） | [limiter.md](skills/effects/limiter.md) |
| `plugin` | 第三方 VST3/CLAP/LV2 | [plugin.md](skills/effects/plugin.md) |

### 推荐串联顺序

- 单条轨：`eq → compressor → chorus/phaser/flanger → delay → reverb`（用不到的跳过）。
- 主总线：`eq → compressor → limiter`，limiter 永远在最后。
- 贝斯、底鼓一般不加 reverb / chorus；其它轨用 `eq` highpass 150Hz 左右给它们让出低频。

## 工作流示例

**用户**："做一个 90bpm 的 lo-fi 节拍，4 拍循环，鼓 + 贝斯 + 电钢和弦。"

1. 查乐器：`muser instruments --family drums`、`muser instruments --family poly`，
   选 `drums.lofi`、`bass.finger`、`poly.wurly`。
2. 查效果器：`muser effects reverb`，给电钢加一点混响。
3. 写工程：

```json
{
  "tempo": 90.0,
  "master_effects": [
    { "type": "compressor", "threshold_db": -14, "ratio": 2, "makeup_db": 2 },
    { "type": "limiter", "ceiling_db": -1.0 }
  ],
  "tracks": [
    {
      "name": "drums",
      "instrument": "drums.lofi",
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 36, "start": 0.0, "length": 0.25, "velocity": 0.9 },
        { "pitch": 38, "start": 1.0, "length": 0.25 },
        { "pitch": 36, "start": 2.5, "length": 0.25, "velocity": 0.7 },
        { "pitch": 38, "start": 3.0, "length": 0.25 },
        { "pitch": 42, "start": 0.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 0.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 1.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 1.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 2.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 2.5, "length": 0.1, "velocity": 0.35 },
        { "pitch": 42, "start": 3.0, "length": 0.1, "velocity": 0.5 },
        { "pitch": 42, "start": 3.5, "length": 0.1, "velocity": 0.35 }
      ]}]
    },
    {
      "name": "bass",
      "instrument": "bass.finger",
      "gain_db": -2.0,
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 33, "start": 0.0, "length": 1.5 },
        { "pitch": 29, "start": 2.0, "length": 1.5 }
      ]}]
    },
    {
      "name": "keys",
      "instrument": "poly.wurly",
      "gain_db": -6.0,
      "effects": [
        { "type": "eq", "shape": "highpass", "frequency": 150 },
        { "type": "reverb", "mix": 0.3, "decay_seconds": 2.0, "size": 0.6 }
      ],
      "clips": [{ "start": 0.0, "length": 4.0, "notes": [
        { "pitch": 57, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 60, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 64, "start": 0.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 53, "start": 2.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 57, "start": 2.0, "length": 2.0, "velocity": 0.6 },
        { "pitch": 60, "start": 2.0, "length": 2.0, "velocity": 0.6 }
      ]}]
    }
  ]
}
```

4. `muser validate beat.muser.json` → `muser render beat.muser.json -o beat.wav`。
5. 把 wav 路径、时长、峰值告诉用户，按反馈增量修改。

## 编曲常见套路

- **重复多小节**：复制 clip 并把 `start` 后移 4 拍（第 2 小节 `start: 4.0`），或在一个长 clip 里铺满。
- **和弦**：多个音符 `start` 相同、`pitch` 不同。C 大三 = 60/64/67，A 小三 = 57/60/64。
- **层次**：不同角色放不同 track（鼓 / 贝斯 / 和弦 / 旋律），用 `gain_db` 平衡，用 `pan`（-1 左 ~ 1 右）分开。
- **音量**：几条轨叠加后容易过载；各轨 `gain_db` 设 -3 到 -8，主总线最后放 `limiter`。

## 排错

- 渲染前**务必** `validate`；它会给出出错的具体字段与可选值。
- 鼓组没声音或声音不对 → 用了音阶音高，或用了该套鼓不支持的键（见 drums.md）。
- 音色不对 → 先 `muser instruments <名称>` 看预设当前值，再用 `instrument_params` 小步调整。
- 渲染报"输出数值发散" → 某个乐器参数取了极端值（如很短的 release），往常规范围调。
- 拉奏弦乐/管乐短音听起来很弱 → 音符太短，弦乐短音改用 `*_pizz`。
- 混响尾音被截断 → 渲染时加大 `--tail`（默认 2 秒）。
- 整体发闷/发浑 → 非贝斯轨加 `eq` highpass，减少多条轨上的 reverb。
