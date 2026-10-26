# muser

**面向 AI 的无头（headless）编曲引擎。**

muser 没有 GUI。它把一个确定性的音频引擎包装成 **结构化数据格式 + CLI + MCP server**，
让 AI agent 以"表达音乐意图"的方式创作：AI 产出/编辑一份工程 JSON，引擎把它离线渲染成音频。

设计信条：**引擎负责确定性的"音频真相"，AI 负责"意图翻译"，接口层是两者的契约。**

底层音频由 [manguebyte](https://github.com/alessonqueirozdev-hub/manguebyte)（一个 Rust 音频框架）
提供，以 git submodule 的形式 vendored 引入。

---

## 能做什么

- 多轨编曲：鼓组 + 合成器 + 贝斯，多 clip、多音符
- 115 个内置音源：10 套鼓组、68 个合成器预设，以及吉他、贝斯、风琴、马林巴等打击旋律乐器、弦乐、管乐、合唱（全部由 manguebyte 实时合成，不依赖系统 MIDI 音源），每个都可用 `instrument_params` 调音色
- 内置 SFZ 采样器：`sfz:/path/to.sfz` 直接加载 SFZ 音色库
- 效果器链（reverb / delay / compressor / chorus / eq / limiter / phaser / flanger），挂在轨级、总线或主总线
- 混音路由：总线编组、发送到共用混响/延迟、总线套总线、压缩器侧链（底鼓压贝斯的泵感），自动延迟补偿
- 自动化：乐器参数、效果器参数、增益、声像、发送量随时间变化（时间点曲线 / LFO / 带种子的随机游走）
- 加载第三方 CLAP/VST3/LV2 插件：作为效果器(VST)或作为音源(VSTi)，扫描本机插件、按参数名 + 文字设置参数、参数自动化（`host` 特性）
- 试听：`muser audition` 按参数取值逐个渲染，报告响度、亮度、起音、余音，帮 AI 摸清陌生参数
- 插件画像：把插件使用经验（别名、预设、试听结论）存成可分享的数据文件；插件音色可存为状态文件
- 内置开源插件的画像与英文 skill：Surge XT / Surge XT Effects、Vital（波表合成器，可设波表与调制矩阵）、
  Airwindows Consolidated（约 470 个效果，按名称选择）、Plasma（失真）——装好插件即可按名称、别名、预设使用
- 离线渲染为 WAV（内置乐器多线程并行）；`muser view` 本地播放页：流式播放，调音量/静音/独奏由引擎准确重渲染（有缓存，约 1–2 秒），可保存回工程
- 两种 AI 接口：**CLI**（shell 调用）和 **MCP server**（agent 直接工具调用）

## 快速上手

### 0. 直接下载（可选）

[Releases](https://github.com/claw-works/muser/releases) 提供 Linux x86_64、macOS（Apple Silicon / Intel）、Windows x86_64 的预编译版本
（`muser` 与 `muser-mcp`，已启用插件宿主），解压即用。想从源码构建，继续往下。

### 1. 克隆（含 submodule）

muser 依赖 vendored 的 manguebyte 子模块，务必递归克隆：

```bash
git clone --recurse-submodules git@github.com:claw-works/muser.git
cd muser
```

若已普通克隆，补拉子模块：

```bash
git submodule update --init --recursive
```

### 2. 构建

需要 Rust 1.85+（edition 2024）。

```bash
cargo build --release
```

> 中国大陆网络访问 crates.io 较慢时，建议配置镜像（如 rsproxy.cn）加速。

构建产物在 `target/release/muser`（CLI）和 `target/release/muser-mcp`（MCP server）。
下文命令里的 `muser` 即指该二进制——把 `target/release` 加入 `PATH`，或用全路径调用。

### 3. 用 CLI 做第一首

```bash
# 生成一个示例工程 JSON
muser new -o song.muser.json

# 校验
muser validate song.muser.json

# 渲染为 WAV
muser render song.muser.json -o song.wav

# 查看可用乐器与效果器（加名称看详情，加 --json 给 AI 解析）
muser instruments
muser instruments guitar.nylon
muser effects
muser effects reverb

# 查看信号流（总线、发送、侧链）
muser routing song.muser.json

# 给人听：本地播放页（时间线、音量/静音/独奏，改动由引擎重新渲染，可保存回工程；AI 改了工程页面自动刷新）
muser view song.muser.json --open      # http://127.0.0.1:8737/
```

> 开发期也可用 `cargo run -p muser-cli -- <子命令>` 免安装直接跑。

### 4. 以 MCP server 运行（给 agent 用）

```bash
muser-mcp          # stdio MCP server，暴露 34 个 tools
```

在 MCP 客户端（如 Claude Desktop）中把该可执行文件注册为一个 stdio server，
agent 即可通过 `create_project / add_track / add_clip / add_notes / render` 等工具编曲。

## 外部音色与插件（可选）

内置的 115 个音源不需要下载任何东西（`samples/songs/dnb-liquid-skyline` 等大部分示范只用内置音源）。
下面这些免费 / 开源的音色库和插件是可选的：`skills/` 与部分 `samples/` 用到它们，装好后按名称、别名、预设就能用。

### SFZ 采样音色库

| 音色库 | 内容 | 许可 | 下载 | 用到它的示范 |
|---|---|---|---|---|
| Salamander Grand Piano（freepats） | 三角钢琴，多力度层、琴弦共鸣 | CC-BY 3.0 | [freepats.zenvoid.org](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html)（SFZ+FLAC 版） | `songs/bach-prelude-c` |
| PianoFB（freepats） | 立式钢琴 | GPL-3.0 + 例外 | [freepats.zenvoid.org](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html) | `instruments/sfz-piano` |
| Sonatina Symphonic Orchestra 4.0 | 管弦乐：各声部的 Legato / Marcato / Sustain / Staccato / Pizzicato 等奏法 | CC Sampling Plus 1.0 | [github.com/peastman/sso](https://github.com/peastman/sso)（Releases 里的 v4.0 zip） | `songs/ode-to-joy-orchestra` |
| Black And Blue Basses（Karoryfer） | 电贝斯，多种奏法（键位切换）与控制器开关 | CC0 | [github.com/sfzinstruments/karoryfer.black-and-blue-basses](https://github.com/sfzinstruments/karoryfer.black-and-blue-basses) | `instruments/sfz-bass-black-and-blue` |
| VSCO 2 Community Edition | 管弦乐（较小，起音偏慢） | CC0 | [versilian-studios.com](https://versilian-studios.com/vsco-community/) | — |

更多免费 SFZ：[sfzinstruments.github.io](https://sfzinstruments.github.io/)。工程里写 `"instrument": "sfz:<.sfz 文件路径>"`；
示范工程里的路径是作者本机的 `/data/Music/SFZ-samples/...`，解压到别处时改成你的路径即可。
SFZ 的控制器与奏法用 `instrument_params` 的 `cc` / `keyswitch` 设置（见 [skills/instruments/sfz.md](skills/instruments/sfz.md)）。

### 插件（需要 `host` 特性；预编译版已启用）

| 插件 | 类型 | 许可 | 下载 | muser 内置画像 / skill |
|---|---|---|---|---|
| Surge XT（含 Surge XT Effects） | 混合合成器 + 效果器合集 | GPL-3.0 | [surge-synthesizer.github.io](https://surge-synthesizer.github.io/) | [surge-xt](skills/plugins/surge-xt.md)、[surge-xt-effects](skills/plugins/surge-xt-effects.md) |
| Vital | 波表合成器 | 免费（源码 GPL-3.0） | [vital.audio](https://vital.audio/)（免费版需注册） | [vital](skills/plugins/vital.md) |
| Airwindows Consolidated | 约 470 个 Airwindows 效果合一（请用 CLAP 版） | MIT | [airwindows.com/consolidated](https://www.airwindows.com/consolidated/) 或 [github.com/baconpaul/airwin2rack](https://github.com/baconpaul/airwin2rack) | [airwindows-consolidated](skills/plugins/airwindows-consolidated.md) |
| Plasma（Dimethoxy） | 双级失真 | AGPL-3.0 | [github.com/Dimethoxy/Plasma](https://github.com/Dimethoxy/Plasma) | [plasma](skills/plugins/plasma.md) |

装好后运行 `muser plugin scan` 确认 muser 能找到它们（CLAP / VST3 / LV2 的系统与用户目录都会扫描），
插件示范在 `samples/plugins/`，用法见 [skills/plugins.md](skills/plugins.md)。

## 工程文件格式（给 AI 的契约）

时间单位是**拍 (beats)**：`1.0` = 四分音符，与 tempo 解耦。最小示例：

```json
{
  "tempo": 90.0,
  "tracks": [
    {
      "instrument": "drums.lofi",
      "clips": [
        { "start": 0.0, "length": 4.0, "notes": [
          { "pitch": 36, "start": 0.0, "length": 0.25 },
          { "pitch": 38, "start": 1.0, "length": 0.25 }
        ]}
      ]
    }
  ]
}
```

鼓组音高用 GM 鼓键（36=底鼓, 38=军鼓, 42=闭镲）；旋律乐器用音阶音高（60=中央C）。

## 可选特性

```bash
# 启用插件宿主（加载第三方 CLAP/VST3/LV2 效果器）
cargo build -p muser-cli --features host
```

## 项目结构

```
muser/
├─ crates/
│  ├─ muser-core/     领域模型：Project/Track/Clip/Note/Effect（全 serde）
│  ├─ muser-engine/   引擎适配：编译成 manguebyte 乐器并离线渲染
│  ├─ muser-cli/      CLI 可执行文件 `muser`
│  └─ muser-mcp/      MCP server（stdio）
├─ vendor/manguebyte/ 上游音频引擎（git submodule）
├─ samples/           示范工程（songs/instruments/params/effects/automation/routing/plugins 分组）
├─ ARCHITECTURE.md    架构、分层、工程 JSON schema、路线图
├─ SKILLS.md          给 AI 的编曲操作手册（总入口）
├─ skills/            子 skill：每个乐器类别、每个效果器、自动化、路由、内置插件画像
├─ profiles/          muser 内置的插件画像（JSON）
├─ scripts/           gen_skills.py 生成 skills/；render_samples.sh 批量渲染示范
└─ TOOLS.md           CLI + MCP 工具清单
```

## 深入阅读

- [ARCHITECTURE.md](ARCHITECTURE.md) — 三层架构、数据流、工程文件格式、vendored 策略、路线图
- [SKILLS.md](SKILLS.md) — AI 如何表达音乐意图（总入口：工作流、速查、乐器与效果器索引）
- [skills/](skills/) — 每个乐器类别、每个效果器的参数、场景预设与常见错误；自动化、路由、延音踏板
- [TOOLS.md](TOOLS.md) — CLI 命令与 MCP tools 清单
- [samples/](samples/) — 分组示范工程（风格曲、音源、调音、效果器、自动化、路由、插件）

## 许可

本仓库自身代码遵循 MIT / Apache-2.0 双许可。vendored 的 manguebyte 同为 MIT / Apache-2.0。
