# muser 架构

muser 是一个**面向 AI 的无头（headless）编曲引擎**。它没有 GUI，只提供
结构化数据格式 + CLI + （规划中的）MCP server，让 AI agent 能够以「表达音乐意图」
的方式驱动一个确定性的音频引擎，产出可听的音频。

设计信条：**引擎负责确定性的"音频真相"，AI 负责"意图翻译"，接口层是两者的契约。**

---

## 三层模型

```
┌─────────────────────────────────────────────────────────────┐
│  3. 接口层 (muser-cli + muser-mcp)                            │
│     - CLI 子命令：render / validate / instruments / new       │
│     - MCP server (muser-mcp)：把能力作为 tools 暴露给 agent    │
│     - skill 文档：告诉 AI 如何表达音乐意图 (SKILLS.md)         │
├─────────────────────────────────────────────────────────────┤
│  2. 引擎适配层 (muser-engine)                                 │
│     - instrument：语义乐器名 → manguebyte 乐器                 │
│     - render：Project → 样本精确调度 → 离线渲染 → WAV          │
├─────────────────────────────────────────────────────────────┤
│  1. 领域模型层 (muser-core)                                   │
│     - Project / Track / Clip / Note / Beats / TimeSignature   │
│     - 全 serde：JSON 就是 AI 读写工程的格式                    │
│     - 时间以「拍 (beats)」表示，与 tempo/采样率解耦            │
├─────────────────────────────────────────────────────────────┤
│  0. 音频引擎 (vendored: vendor/manguebyte)                    │
│     - DSP、乐器/鼓组、音频图、插件宿主、音频文件格式           │
│     - 上游独立 workspace，MIT/Apache，通过 path 依赖引入       │
└─────────────────────────────────────────────────────────────┘
```

AI 只需理解**第 1 层的 JSON 格式**和**第 3 层的命令**；第 0、2 层对 AI 透明。

---

## crate 划分

| crate | 层 | 职责 | 关键依赖 |
|---|---|---|---|
| `muser-core` | 1 | 领域模型 + 序列化。无音频依赖，编译极快。 | serde |
| `muser-engine` | 2 | 把 core 模型编译成 manguebyte 乐器并离线渲染。 | muser-core, manguebyte |
| `muser-cli` | 3 | CLI 可执行文件 `muser`。 | muser-core, muser-engine, clap |
| `muser-mcp` | 3 | MCP server（stdio），把能力暴露为 agent tools。 | muser-core, muser-engine, rmcp |
| `vendor/manguebyte` | 0 | 上游音频引擎（vendored）。 | — |

### 为什么 vendored 而非 git submodule / crates.io 依赖

- **可控**：源码在仓库内，上游若断更可无缝接管维护（许可为 MIT/Apache，允许 fork）。
- **隔离**：`vendor/manguebyte` 是它自己的 Cargo workspace；muser 顶层 workspace 用
  `exclude = ["vendor"]` 排除它，再以 `path` 依赖指向 `vendor/manguebyte/crates/manguebyte`。
  （Cargo 不允许 workspace 嵌套，这是唯一正确的接法。）
- **纯离线**：依赖时 `default-features = false`，不拉 `devices`（ALSA/CoreAudio 等），
  只用离线渲染所需的 dsp/formats/graph。将来要做实时预览再开 `devices` 特性。

### 同步上游

```bash
# 在 vendor/manguebyte 内拉取上游更新
cd vendor/manguebyte && git pull
# 若上游 API 变化，只需调整 muser-engine 这一层（唯一与 manguebyte 耦合的 crate）
```

与引擎耦合的代码**全部集中在 `muser-engine`**。core 和 cli 不碰 manguebyte，
因此上游变动的影响面被限制在一个 crate 内。

---

## 数据流

```
自然语言意图
    │  (AI 翻译)
    ▼
Project JSON  ──Project::from_json──►  muser_core::Project
                                           │  Project::validate()
                                           ▼
                                    muser_engine::render()
                                           │
                 ┌─────────────────────────┼─────────────────────────┐
                 ▼                          ▼                          ▼
          每条 Track                 instrument::resolve        逐 clip/note
       instrument 语义名  ──►   manguebyte 乐器(Synth<V>)   ──► 换算成样本精确事件
                                                                 (note_on/note_off)
                                           │
                                           ▼
                              分块处理：到点触发事件 → synth.process()
                                           │
                                           ▼
                          按 track 增益/等功率声像 混入 master 缓冲
                                           │
                                           ▼
                              主增益 → wav::write_file (16bit stereo)
                                           │
                                           ▼
                                        out.wav
```

### 时间模型

core 层用**拍 (beats)** 作为唯一时间单位，`1 beat = 一个四分音符`。
引擎层负责换算：

```
seconds = beats × (60 / tempo)
samples = seconds × sample_rate
```

这样 AI 推理时无需关心采样率，改 tempo 不必改音符时间。

### 乐器解析

音轨用**语义名**引用乐器（如 `"drums.808"`、`"synth.poly"`、`"synth.fm"`），
由 `muser_engine::instrument::resolve()` 映射到具体的 manguebyte 乐器。
不同乐器的 Voice 是不同 Rust 类型，用枚举 `TrackInstrument` 做类型擦除，
对外暴露统一的 `prepare / note_on / note_off / process`。鼓组按惯例走 MIDI 通道 9。

---

## 工程文件格式 (`*.muser.json`)

工程文件是 muser 的核心契约——**AI 产出/编辑的就是这个 JSON**。

### 结构

```jsonc
{
  "schema_version": 1,          // 格式版本，用于未来迁移（可省略，默认 1）
  "name": "example",            // 工程名（可省略）
  "tempo": 90.0,                // 必填，每分钟四分音符数，> 0
  "time_signature": {           // 拍号（可省略，默认 4/4）
    "numerator": 4,
    "denominator": 4
  },
  "master_gain_db": 0.0,        // 主输出增益 dB（可省略，默认 0）
  "master_effects": [           // 主总线效果链（可省略，默认空；在主增益前施加）
    { "type": "compressor", "threshold_db": -16, "ratio": 3 }
  ],
  "tracks": [                   // 音轨数组
    {
      "name": "drums",          // 轨名（可省略）
      "instrument": "drums.lofi", // 必填，语义乐器名（见 `muser instruments`）
      "gain_db": 0.0,           // 轨增益 dB（可省略，默认 0）
      "pan": 0.0,               // 声像 -1(左)~1(右)（可省略，默认 0）
      "muted": false,           // 是否静音（可省略，默认 false）
      "effects": [              // 轨级效果链（可省略，默认空；在增益/声像前施加）
        { "type": "reverb", "mix": 0.3 }
      ],
      "clips": [                // 片段数组
        {
          "start": 0.0,         // 必填，片段起点（beats，从工程开头算）
          "length": 4.0,        // 必填，片段长度（beats），> 0
          "notes": [            // 音符数组
            {
              "pitch": 36,      // 必填，MIDI 音高 0-127（60 = 中央C）
              "start": 0.0,     // 必填，音符起点（beats，相对 clip 起点）
              "length": 0.25,   // 必填，音符时值（beats），> 0
              "velocity": 0.8   // 力度 0.0-1.0（可省略，默认 0.8）
            }
          ]
        }
      ]
    }
  ]
}
```

### 字段约束（`Project::validate` 会检查）

- `tempo` 必须 > 0
- 每个 clip 的 `start` ≥ 0，`length` > 0
- 超出 clip `length` 的音符会在渲染时被裁剪到 clip 边界
- 鼓组音轨的 `pitch` 用 GM 鼓键编号（36=底鼓, 38=军鼓, 42=闭镲, …）

### 时间直觉速查（4/4 拍）

| 想要 | beats 值 |
|---|---|
| 一个全音符 | 4.0 |
| 一个四分音符 | 1.0 |
| 一个八分音符 | 0.5 |
| 一个十六分音符 | 0.25 |
| 一小节 | 4.0（= `time_signature.beats_per_bar()`） |

---

## 当前状态 (MVP)

已验证跑通的端到端闭环：

```bash
muser new -o demo.muser.json     # 生成示例工程
muser validate demo.muser.json   # 校验
muser render demo.muser.json -o demo.wav   # 离线渲染
# → 产出有效的 16-bit / 48kHz / stereo WAV
```

已实现：
- 多轨、多 clip、多音符的离线渲染
- 10 种鼓组 + 复音合成器 + FM 合成器
- 轨增益、等功率声像、主增益、结尾余响
- 效果器链：reverb / delay / compressor / chorus / eq(filter) / limiter，可挂到轨级或主总线
- 插件宿主（`host` 特性）：加载第三方 CLAP/VST3/LV2 作为**效果器**
  （`{ "type": "plugin", "path": "..." }`），由 manguebyte-host 激活为 Processor。
  已用真实插件验证：加载本机 `Surge XT Effects.vst3`，输出与无插件对照显著不同。
- 第三方插件作为**乐器**（`host` 特性）：音轨 `instrument` 用 `plugin:路径[#id]`，
  音符经 `process_with`+`ProcessContext` 喂给插件。已用真实 VSTi 验证：
  `Surge XT.vst3` 有音符时 RMS 895、无音符时 RMS 0（完全静音），证明响应音符发声。
- MCP server（`muser-mcp`）：15 个 tools（工程生命周期 + 增量编辑 + 磁盘读写 + 渲染），stdio 运行

尚未实现（路线图）：
- CI：GitHub Actions 工作流（拉取 submodule、构建、跑 `cargo test --workspace`、
  可选 `--features host` 构建、`cargo fmt --check` / `clippy`）
- 插件乐器/效果器的**参数调节**（包络、滤波等插件内部参数）
- 更多内置效果器类型（phaser / flanger / waveshaper 等，manguebyte 均自带）
- 实时音频预览（需开启 manguebyte 的 `devices` 特性 + libasound2-dev）
- 音频图编排（manguebyte-graph：运行时热重连的信号图，供更复杂的路由）
```
