# muser 架构

muser 是一个**面向 AI 的无头（headless）编曲引擎**。它没有 GUI，只提供
结构化数据格式 + CLI + MCP server，让 AI agent 能够以「表达音乐意图」
的方式驱动一个确定性的音频引擎，产出可听的音频。

设计信条：**引擎负责确定性的"音频真相"，AI 负责"意图翻译"，接口层是两者的契约。**

---

## 三层模型

```
┌─────────────────────────────────────────────────────────────┐
│  3. 接口层 (muser-cli + muser-mcp)                            │
│     - CLI 子命令：render / validate / instruments / effects /     │
│       plugin / audition / view / routing / new               │
│     - MCP server (muser-mcp)：把能力作为 tools 暴露给 agent    │
│     - skill 文档：告诉 AI 如何表达音乐意图 (SKILLS.md)         │
├─────────────────────────────────────────────────────────────┤
│  2. 引擎适配层 (muser-engine)                                 │
│     - instrument：语义乐器名 → manguebyte 乐器                 │
│     - render：Project → 流式 Renderer → WAV / 播放页 / 声卡    │
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
| `muser-engine` | 2 | 把 core 模型编译成 manguebyte 乐器，流式渲染（离线 WAV、边渲染边播、本机声卡）。 | muser-core, manguebyte |
| `muser-cli` | 3 | CLI 可执行文件 `muser`。 | muser-core, muser-engine, clap |
| `muser-mcp` | 3 | MCP server（stdio），把能力暴露为 agent tools。 | muser-core, muser-engine, rmcp |
| `vendor/manguebyte` | 0 | 上游音频引擎（vendored）。 | — |

### 为什么 vendored（git submodule）而非 crates.io 依赖

- **可控**：以 git submodule 固定在仓库内的某个提交，上游若断更可切到自己的 fork 接管维护（许可为 MIT/Apache，允许 fork）。
- **隔离**：`vendor/manguebyte` 是它自己的 Cargo workspace；muser 顶层 workspace 用
  `exclude = ["vendor"]` 排除它，再以 `path` 依赖指向 `vendor/manguebyte/crates/manguebyte`。
  （Cargo 不允许 workspace 嵌套，这是唯一正确的接法。）
- **按需开设备**：依赖时 `default-features = false`，只用渲染所需的部分；本机声卡播放（`muser view --device`）
  通过 muser-engine 的 `devices` 特性打开 manguebyte 的 `devices`（CoreAudio / WASAPI / ALSA，Linux 需 libasound2-dev）。

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
                                 muser_engine::Renderer（流式）
                                           │
                 ┌─────────────────────────┼─────────────────────────┐
                 ▼                          ▼                          ▼
          每条 Track                 instrument::resolve        逐 clip/note
       instrument 语义名  ──►   manguebyte 乐器(Synth<V>)   ──► 换算成样本精确事件
                                                                 (note_on/note_off)
                                           │  内置乐器各在一个演奏线程里领先一段先算
                                           ▼
             每次 process() 往前推进一段：按路由计划依次处理每个节点
             （乐器 → 效果链 → 推子 → 送进总线 / 发送 / 侧链键 → 主总线效果链 → 主增益）
                                           │
             ┌─────────────────────────────┼─────────────────────────────┐
             ▼                             ▼                             ▼
   render()：从头推进到结尾       muser view：边渲染边播放       muser view --device：
   → wav::write_file              （分段送给浏览器）             无锁队列 → 本机声卡
```

离线渲染就是从头到尾调用流式渲染器，两者逐样本一致（测试 `streaming_matches_offline` 用任意块长对比）。
效果链按从工程开头算起、固定步长的块处理，流式推进的每一段都对齐到这些块的公倍数，所以分块方式与一次处理整首相同；
有延迟的效果器（limiter 前瞻、插件）让节点的输出取效果链输出往后 latency 帧的样本，上游相应多算这么多。
声卡播放时推子、静音、独奏在播放线程里从下一段起生效（`Renderer::set_mix`），听到的延迟约等于队列长度（几十毫秒）；
跳转像 DAW 一样直接从目标位置开始（`Renderer::new_at`）：那时还按着的音与踏板补发（chase），效果器从空状态开始，
音轨乐器已在缓存里时用缓存的输出；SFZ 音色在进程里只加载一次（`sfz::load_shared`），重建不用重新读采样。

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
  "seed": 0,                    // 随机种子（可省略）：决定没写 seed 的 random 自动化
  "automation": [               // 主总线自动化（可省略）：目标 master_gain_db / master_effects[i]
    { "target": "master_effects[0]", "param": "ratio", "shape": "points", "points": [[0, 2], [16, 4]] }
  ],
  "buses": [                    // 总线（可省略）：汇集送来的信号 → effects → gain_db/pan → output
    {
      "name": "reverb",         // 必填，唯一，不能叫 master、不能含 "."
      "effects": [{ "type": "reverb", "mix": 1.0 }],
      "gain_db": 0.0, "pan": 0.0, "muted": false,
      "output": "master",       // 去向（可省略，默认 master）：master 或另一条总线
      "sends": {},              // 发送到其它总线（dB）
      "automation": []          // 目标 effects[i] / gain_db / pan / sends.<总线>
    }
  ],
  "tracks": [                   // 音轨数组
    {
      "name": "drums",          // 轨名（可省略）
      "instrument": "drums.lofi", // 必填，乐器名（见 `muser instruments`），或 sfz:/path.sfz、plugin:/path.vst3
      "instrument_params": {    // 调音色（可省略）：在预设上覆盖，键与范围见 `muser instruments <名称>`
        "kick": { "tune": -2 }  //   鼓组按鼓件；旋律乐器直接写参数，如 { "cutoff": 800 }
      },
      "gain_db": 0.0,           // 轨增益 dB（可省略，默认 0）
      "pan": 0.0,               // 声像 -1(左)~1(右)（可省略，默认 0）
      "muted": false,           // 是否静音（可省略，默认 false）
      "effects": [              // 轨级效果链（可省略，默认空；在增益/声像前施加）
        { "type": "reverb", "mix": 0.3 }
      ],
      "output": "master",       // 去向（可省略，默认 master）：master 或总线名
      "sends": { "reverb": -6 }, // 推子后发送到总线（可省略）：总线名 → dB
      "automation": [           // 轨自动化（可省略）：目标 instrument / effects[i] / gain_db / pan / sends.<总线>
        { "target": "pan", "shape": "lfo", "period_beats": 8, "min": -0.5, "max": 0.5 }
      ],
      "clips": [                // 片段数组
        {
          "start": 0.0,         // 必填，片段起点（beats，从工程开头算）
          "length": 4.0,        // 必填，片段长度（beats），> 0
          "legato": false,      // 连奏（可省略）：每个音接到下一个不同音高的音，重叠约 40 ms
          "pedal": [[0.1, 4.03]], // 延音踏板（CC64）踩下区间（可省略）：[起, 止] 拍，相对 clip 起点，递增不重叠
          "notes": [            // 音符数组
            {
              "pitch": 36,      // 必填，MIDI 音高 0-127（60 = 中央C）
              "start": 0.0,     // 必填，音符起点（beats，相对 clip 起点）
              "length": 0.25,   // 必填，音符时值（beats），> 0
              "velocity": 0.8,  // 力度 0.0-1.0（可省略，默认 0.8）
              "accent": false   // 重音（可省略）：鼓组、acid 贝斯更响更亮
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
- 路由（`muser_core::routing::plan`）：总线名合法且唯一；output / sends / compressor 的 `sidechain` 引用存在；
  不自指、不成环（侧链键也算依赖）。通过后按拓扑序得到处理顺序

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
- 115 个内置音源：10 套鼓组、68 个 poly 合成器预设、吉他/贝斯/风琴/打击旋律/弦乐/管乐/合唱
- 乐器与效果器描述表（名称、参数、范围、默认值、说明）只在代码里写一份：
  core `EFFECT_SPECS`、engine `instrument::catalog()` 与 `params.rs`；CLI、MCP、`validate` 与 `skills/` 文档都由它派生
- 内置乐器参数 `instrument_params`：每类乐器开放常用参数（统一命名、真实单位），在预设上覆盖；
  鼓组按鼓件设置；渲染时检测数值发散并报错
- `sfz:` 内置 SFZ 采样器（manguebyte-sampler）：直接加载 .sfz 音色库，无需外部插件
- 外置插件通用层（`host` 特性）：扫描（独立进程、缓存）、信息、参数搜索、选项文字；插件参数按**名称**设置，
  值可写插件显示的文字（往返校验 + 查表回退），用于 `plugin:` 乐器的 `instrument_params`、`plugin` 效果器的 `params`
  与自动化；激活后静音预热，插件未采纳参数时重发
- 插件画像（profile）：纯数据 JSON，按插件 id 匹配（多格式 id），含别名、常用取值、试听观察、预设、setup、经验、
  来源与验证版本；用户目录 > `MUSER_PROFILE_PATH` > 内置；结构校验与文字清洗（为社区分享做准备）；
  自动草稿与试听回写；渲染为 skill 文档。工程参数里的保留键：`@state` / `@setup` / `@preset`
- 插件状态（state）：保存/加载插件完整音色，文件头记录插件 id 与版本，装错插件会被拒绝
- 画像扩展：`selector`（"类型/程序"只在状态里的插件：改写 JUCE XML 状态的根属性切换并刷新参数表，
  如 Airwindows Consolidated 的 `effect`）；`state_format: "vital"`（解析 Vital 状态里的 JSON 预设，
  `@wavetable` 生成波表、`@modulations` 写调制矩阵）；`source.lang: "en"` 生成英文 skill
- 内置画像（`profiles/`）：Surge XT、Surge XT Effects、Vital、Airwindows Consolidated（效果清单来自 Airwindows 注册表，
  与安装版本取交集）、Plasma
- 插件走带：渲染时把工程速度、拍号交给插件（按块推进），tempo 同步的 LFO / 延迟正确；插件动态库在进程内常驻复用
  （有的插件卸载时后台线程仍在运行会崩溃）；路径支持 `~/`
- 试听（audition）：按参数取值逐个渲染测试音，报告响度、频谱重心、频段占比、起音、余音——给 AI 的"耳朵"
- 自动化：points / lfo / random（带种子）三种形状，目标覆盖内置乐器参数、效果器参数、
  轨增益与声像、主增益与主总线效果器；乐器/效果器每 64 帧更新，增益与声像逐帧更新
- 轨增益、等功率声像、主增益、结尾余响
- 效果器链：reverb / delay / compressor / chorus / eq(filter) / limiter / phaser / flanger，可挂到轨级、总线或主总线
- 路由：总线（`buses`）、音轨/总线的 `output` 与推子后 `sends`（可自动化）、总线套总线；compressor `sidechain`
  由任意音轨/总线的推子后信号触发（静音的键轨照样触发）；按拓扑序离线处理，缓冲用完即释放；
  有延迟的效果器（limiter 前瞻、插件上报的延迟）在各自链路上补偿，保证并联信号对齐。
  AI 只写这些简单字段，不接触节点/连线图
- 插件宿主（`host` 特性）：加载第三方 CLAP/VST3/LV2 作为**效果器**
  （`{ "type": "plugin", "path": "..." }`），由 manguebyte-host 激活为 Processor。
  已用真实插件验证：加载本机 `Surge XT Effects.vst3`，输出与无插件对照显著不同。
- 第三方插件作为**乐器**（`host` 特性）：音轨 `instrument` 用 `plugin:路径[#id]`，
  音符经 `process_with`+`ProcessContext` 喂给插件。已用真实 VSTi 验证：
  `Surge XT.vst3` 有音符时 RMS 895、无音符时 RMS 0（完全静音），证明响应音符发声。
- 播放页 `muser view`：本地服务（tiny_http，只绑 127.0.0.1，校验 Host/Origin）。后台渲染线程合并请求、复用
  `RenderCache`（各音轨乐器输出按"影响声音的内容"做键缓存，裁掉首尾静音），改音量/静音/独奏只重跑效果链、总线与主总线；
  页面按播放进度分段拉取 16-bit PCM，用 Web Audio 排期播放，新版本从当前位置无缝换上；保存只写回改过的字段；轮询文件变化
- 流式渲染：离线 WAV、边渲染边播、本机声卡共用 `Renderer`，结果逐样本一致；内置乐器各在一个演奏线程里并行演奏
  （插件乐器与效果器在渲染线程），渲染期间关闭非规格化浮点（`NoDenormals`），近乎静音的尾巴不再拖慢滤波与混响
- 插件实例延迟约一分钟释放（同一线程内），避免插件后台线程（如 JUCE 联网检查更新）访问已释放对象
- CI / 发布：GitHub Actions 在 Linux、macOS、Windows 上构建与测试（默认与插件宿主特性）；推送 `v*` 标签时构建
  Linux x86_64、macOS arm64 / x86_64、Windows x86_64 的 `muser` + `muser-mcp`（含插件宿主），附文档与 SHA256，发布到 GitHub Release
- MCP server（`muser-mcp`）：34 个 tools（工程生命周期 + 增量编辑 + 路由 + 自动化 + 磁盘读写 + 渲染 + 乐器/效果器查询 + 外置插件 + 试听 + 插件画像与状态），stdio 运行

尚未实现（路线图）：
- 更多内置效果器类型（waveshaper / 多段 EQ 等，manguebyte 均自带）
```
