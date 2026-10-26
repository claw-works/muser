# muser

[English](README.md) | 简体中文

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
- 内置 SFZ 采样器：`sfz:/path/to.sfz`（或相对 `MUSER_SFZ_PATH` 音色库目录的路径）直接加载 SFZ 音色库
- 效果器链（reverb / delay / compressor / chorus / eq / limiter / phaser / flanger），挂在轨级、总线或主总线
- 混音路由：总线编组、发送到共用混响/延迟、总线套总线、压缩器侧链（底鼓压贝斯的泵感），自动延迟补偿
- 自动化：乐器参数、效果器参数、增益、声像、发送量随时间变化（时间点曲线 / LFO / 带种子的随机游走）
- 加载第三方 CLAP/VST3/LV2 插件：作为效果器(VST)或作为音源(VSTi)，扫描本机插件、按参数名 + 文字设置参数、参数自动化（`host` 特性）
- 试听：`muser audition` 按参数取值逐个渲染，报告响度、亮度、起音、余音，帮 AI 摸清陌生参数
- 插件画像：把插件使用经验（别名、预设、试听结论）存成可分享的数据文件；插件音色可存为状态文件
- 内置开源插件的画像与英文 skill：Surge XT / Surge XT Effects、Vital（波表合成器，可设波表与调制矩阵）、
  Airwindows Consolidated（约 470 个效果，按名称选择）、Plasma（失真）——装好插件即可按名称、别名、预设使用
- 流式渲染引擎：逐块走完整张路由图，离线渲染为 WAV 与边渲染边播放共用同一份代码、结果逐样本一致（各音轨乐器并行演奏）
- `muser view` 播放页：边渲染边播（开头一小段好了就能听），调音量/静音/独奏后从当前位置无缝换上新版本，可保存回工程；
  加 `--device` 直接用本机声卡出声，调音量/静音/独奏即时生效（远程经 SSH 端口转发时用默认的浏览器播放）
- 两种 AI 接口：**CLI**（shell 调用）和 **MCP server**（agent 直接工具调用）

## 快速上手

### 下载即用（不需要装 Rust）

1. 从 [Releases](https://github.com/claw-works/muser/releases) 下载对应系统的压缩包：

   | 系统 | 文件 |
   |---|---|
   | macOS，Apple 芯片（M1–M4） | `muser-<版本>-aarch64-apple-darwin.tar.gz` |
   | macOS，Intel | `muser-<版本>-x86_64-apple-darwin.tar.gz` |
   | Windows | `muser-<版本>-x86_64-pc-windows-msvc.zip` |
   | Linux 桌面 | `muser-<版本>-x86_64-unknown-linux-gnu.tar.gz`（需要 `libasound2`，桌面系统都有） |
   | Linux 服务器 / Docker | `muser-<版本>-x86_64-unknown-linux-gnu-headless.tar.gz`（不带声卡播放） |

2. 解压，在解压出来的目录里打开终端。目录里有 `muser`（CLI）、`muser-mcp`（MCP server）、
   给 AI agent 的文档（`SKILLS.md`、`skills/`、`TOOLS.md`）和示范工程（`samples/`）。

   macOS / Linux：

   ```bash
   tar xzf muser-<版本>-aarch64-apple-darwin.tar.gz
   cd muser-<版本>-aarch64-apple-darwin
   # 仅 macOS：程序没有签名，先去掉下载隔离标记
   xattr -dr com.apple.quarantine .
   ```

   Windows（PowerShell）：

   ```powershell
   Expand-Archive muser-<版本>-x86_64-pc-windows-msvc.zip -DestinationPath .
   cd muser-<版本>-x86_64-pc-windows-msvc
   ```

   如果 Windows SmartScreen 提示"未知发布者"，点"更多信息"→"仍要运行"。

3. 试听一首示范曲。播放页会在浏览器里打开，声音从这台电脑的声卡出：

   ```bash
   ./muser view samples/songs/dnb-liquid-skyline.muser.json --open --device
   ```

   Windows 上把 `./muser` 换成 `.\muser.exe`。在终端里按 Ctrl+C 退出。
   去掉 `--device` 则在浏览器里播放（比如 muser 跑在远程机器上时）。

4. 渲染成 WAV 文件：

   ```bash
   ./muser render samples/songs/dnb-liquid-skyline.muser.json -o dnb.wav
   ```

5. 让 AI agent 来作曲：在 agent（Claude Code、Codex 等）里打开解压出来的目录，让它先读 `SKILLS.md`，
   再描述你想要的音乐。agent 会写出工程 JSON 并用 `./muser` 渲染；
   用 `./muser view <工程文件> --open --device` 试听结果。
   支持 MCP 的 agent 也可以改用 `muser-mcp`（见[下文](#以-mcp-server-运行给-agent-用)）。

想在任意目录直接运行 `muser`，把解压出来的目录加入 `PATH`。

### 从源码构建（开发者）

#### 1. 克隆（含 submodule）

muser 依赖 vendored 的 manguebyte 子模块，务必递归克隆：

```bash
git clone --recurse-submodules git@github.com:claw-works/muser.git
cd muser
```

若已普通克隆，补拉子模块：

```bash
git submodule update --init --recursive
```

#### 2. 构建

需要 Rust 1.85+（edition 2024）。

```bash
cargo build --release
```

Linux 上声卡播放（`muser view --device`，默认启用）需要 ALSA 开发包：`sudo apt install libasound2-dev`；
不需要时可用 `cargo build --release --no-default-features` 跳过。

构建产物在 `target/release/muser`（CLI）和 `target/release/muser-mcp`（MCP server）。
下文命令里的 `muser` 即指该二进制——把 `target/release` 加入 `PATH`，或用全路径调用。

### 用 CLI 编曲

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
# 在本机听：声音直接从这台机器的声卡出，推子/静音/独奏即时生效，页面只做遥控
muser view song.muser.json --open --device
```

> 开发期也可用 `cargo run -p muser-cli -- <子命令>` 免安装直接跑。

### 以 MCP server 运行（给 agent 用）

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
示范工程用的是相对音色库的路径（如 `sfz:SalamanderGrandPiano-SFZ+FLAC-V3+20200602/...`）：
把音色库解压到同一个目录，设 `export MUSER_SFZ_PATH=~/Music/SFZ`（多个目录用 `:` 分隔，Windows 用 `;`）即可，
muser 会在这些目录及其两层以内的子目录里查找；也可以直接写绝对路径。
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
│  ├─ muser-engine/   引擎适配：编译成 manguebyte 乐器，流式渲染（WAV / 播放页 / 本机声卡）
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
