# muser Skill：如何用 muser 编曲

> 这份文档是写给 **AI agent** 读的操作手册。它教你如何把用户的音乐意图
> 翻译成 muser 的工程 JSON，并通过 CLI/工具渲染成音频。

## 你的角色

用户用自然语言描述想要的音乐（"做一个 90bpm 的 lo-fi 节拍，钢琴+鼓"）。
你的工作是：
1. 把意图翻译成一个 **Project JSON**（格式见下）。
2. 调用工具渲染成 WAV（见 TOOLS.md）。
3. 听取用户反馈，**增量编辑** JSON，重新渲染。

你**不需要**理解底层音频 DSP。你只在 Project JSON 这一层工作。

## 核心心智模型

- 一个 **Project** 有一个 `tempo`（速度）和若干 **Track**（音轨）。
- 每个 Track 用一件 **乐器**（`instrument`，语义名），上面放若干 **Clip**（片段）。
- 每个 Clip 在时间线上有 `start` 和 `length`，里面是若干 **Note**（音符）。
- **时间单位是「拍 (beats)」**，不是秒。`1.0` = 一个四分音符。与 tempo 无关。

## 时间速查（4/4 拍）

| 音符 | beats |
|---|---|
| 全音符 | 4.0 |
| 二分音符 | 2.0 |
| 四分音符 | 1.0 |
| 八分音符 | 0.5 |
| 十六分音符 | 0.25 |
| 一小节 | 4.0 |

- Clip 的 `start` 从**工程开头**算（第 2 小节起 = `start: 4.0`）。
- Note 的 `start` 从**所在 clip 的开头**算。

## 音高速查 (MIDI pitch)

- `60` = 中央 C (C4)。每 +12 升一个八度，每 +1 升半音。
- `C4=60, D4=62, E4=64, F4=65, G4=67, A4=69, B4=71, C5=72`
- 贝斯常用低音区：`C2=36, C1=24`。

### 鼓组音高用 GM 鼓键（不是音阶音高！）

| 鼓件 | pitch |
|---|---|
| 底鼓 Kick | 36 |
| 军鼓 Snare | 38 |
| 闭镲 Closed Hat | 42 |
| 开镲 Open Hat | 46 |
| 踩镲 Pedal Hat | 44 |
| 拍手 Clap | 39 |
| 吊镲 Crash | 49 |
| 叮叮镲 Ride | 51 |
| 高音桶鼓 Hi Tom | 50 |
| 低音桶鼓 Low Tom | 45 |

## 可用乐器 (`instrument` 字段)

运行 `muser instruments` 获取权威列表。当前：

**鼓组**（音符用 GM 鼓键）：
`drums.808` `drums.909` `drums.606` `drums.cr78` `drums.acoustic`
`drums.trap` `drums.techno` `drums.house` `drums.dnb` `drums.lofi`

**旋律乐器**（音符用音阶音高）：
`synth.poly`（复音合成器，适合和弦/pad/lead）
`synth.fm`（FM 合成器，适合贝斯/电子音色）

## 效果器 (effects)

可以给**音轨**加 `effects`（轨级，在该轨增益/声像之前），也可以给**工程**加
`master_effects`（主总线，在主增益之前）。两者都是效果数组，按顺序串联。
每个效果用 `type` 区分，其余字段是参数，**全部有合理默认值**——写
`{ "type": "reverb" }` 就能得到一个像样的混响。

| type | 用途 | 关键参数（含默认值） |
|---|---|---|
| `reverb` | 混响/空间感 | `mix`(0.25) `decay_seconds`(1.6) `damping`(0.5) `size`(0.5) |
| `delay` | 延迟/回声 | `time_seconds`(0.3) `feedback`(0.35) `mix`(0.3) |
| `compressor` | 压缩动态/做胶水 | `threshold_db`(-18) `ratio`(4) `attack_ms`(10) `release_ms`(120) `makeup_db`(0) |
| `chorus` | 加厚/加宽音色 | `rate_hz`(0.8) `depth_ms`(3) `mix`(0.4) |
| `eq` | 均衡/滤波一个频段 | `shape`(peak) `frequency`(1000) `q`(0.707) `gain_db`(0) |
| `limiter` | 限峰防爆（放主总线末端） | `ceiling_db`(-0.3) `release_ms`(50) |
| `plugin` | 第三方插件 (CLAP/VST3/LV2) | `path`(必填) `id`(可选) `params`(可选, {参数id: 值}) |

> `eq` 的 `shape` 可取：`lowpass` `highpass` `bandpass` `notch` `allpass`
> `peak` `low_shelf` `high_shelf`。`gain_db` 只对 `peak`/`low_shelf`/`high_shelf` 有效。
> 例：`{ "type": "eq", "shape": "high_shelf", "frequency": 8000, "gain_db": 3 }` 提亮高频。

> `plugin` 效果器需要构建时启用 `host` 特性（`cargo build -p muser-cli --features host`），
> 否则渲染会报 `HostDisabled`。示例：
> `{ "type": "plugin", "path": "/path/to/MyReverb.vst3" }`

示例——给 keys 轨加混响，给主总线加压缩：

```json
{
  "tempo": 85.0,
  "master_effects": [
    { "type": "compressor", "threshold_db": -16, "ratio": 3, "makeup_db": 1.5 }
  ],
  "tracks": [
    {
      "name": "keys",
      "instrument": "synth.poly",
      "effects": [
        { "type": "reverb", "mix": 0.3, "decay_seconds": 2.2, "size": 0.7 }
      ],
      "clips": [ /* ... */ ]
    }
  ]
}
```

经验：
- **混响**放在旋律/pad 轨能加空间感；别给贝斯和底鼓加太多，会浑浊。
- **延迟**适合 lead/人声做空间回声；`feedback` 别超过 0.6 否则会堆积。
- **压缩器**放主总线（`master_effects`）做"胶水"，让整体更紧实；`ratio` 3-4、
  `makeup_db` 补回音量。

## 工作流示例

**用户**："做一个 90bpm 的 lo-fi 鼓点，4 小节，再加一条贝斯线。"

**你的步骤**：

1. 先用 `muser new` 看一眼标准格式（或直接按下面模板写）。

2. 写出 Project JSON：

```json
{
  "tempo": 90.0,
  "tracks": [
    {
      "name": "drums",
      "instrument": "drums.lofi",
      "clips": [
        {
          "start": 0.0,
          "length": 4.0,
          "notes": [
            { "pitch": 36, "start": 0.0, "length": 0.25 },
            { "pitch": 38, "start": 1.0, "length": 0.25 },
            { "pitch": 36, "start": 2.0, "length": 0.25 },
            { "pitch": 38, "start": 3.0, "length": 0.25 },
            { "pitch": 42, "start": 0.0, "length": 0.1 },
            { "pitch": 42, "start": 0.5, "length": 0.1 },
            { "pitch": 42, "start": 1.0, "length": 0.1 },
            { "pitch": 42, "start": 1.5, "length": 0.1 }
          ]
        }
      ]
    },
    {
      "name": "bass",
      "instrument": "synth.fm",
      "gain_db": -3.0,
      "clips": [
        {
          "start": 0.0,
          "length": 4.0,
          "notes": [
            { "pitch": 36, "start": 0.0, "length": 1.0 },
            { "pitch": 36, "start": 1.0, "length": 1.0 },
            { "pitch": 31, "start": 2.0, "length": 1.0 },
            { "pitch": 33, "start": 3.0, "length": 1.0 }
          ]
        }
      ]
    }
  ]
}
```

3. 保存为 `beat.muser.json`，然后 `muser validate beat.muser.json`，再
   `muser render beat.muser.json -o beat.wav`。

4. 告诉用户产出的 wav 路径、时长、峰值。根据反馈增量改 JSON。

## 常见套路（给你的创作提示）

- **4/4 鼓点骨架**：kick(36) 在 0.0、2.0；snare(38) 在 1.0、3.0；闭镲(42)走八分（0.0,0.5,1.0,...）。
- **重复多小节**：复制 clip 并把 `start` 往后移（第 2 小节 clip 的 `start: 4.0`），
  或直接在一个长 clip 里铺更多音符。
- **和弦**：在 `synth.poly` 上让多个音符 `start` 相同、`pitch` 不同（如 60/64/67 = C 大三和弦）。
- **贝斯线**：`synth.fm`，低音区（24-48），每拍一个根音。
- **编曲层次**：不同乐器放不同 track，用 `gain_db` 调平衡，用 `pan` 分左右。

## 校验与排错

- 渲染前**务必** `validate`。常见错误：`tempo <= 0`、clip `length <= 0`、
  未知乐器名（用 `muser instruments` 核对）。
- 鼓组用了音阶音高（如 60）会"没声音或不对"——鼓组必须用 GM 鼓键（36/38/42…）。
- 听起来太满/爆音 → 调低 `master_gain_db` 或各 track `gain_db`。
