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
- 10 种鼓组（808/909/606/CR-78/acoustic/trap/techno/house/dnb/lofi）+ 复音合成器 + FM 合成器
- 效果器链（reverb / delay / compressor），挂在轨级或主总线
- 加载第三方 CLAP/VST3/LV2 插件作为效果器（`host` 特性）
- 离线渲染为 WAV
- 两种 AI 接口：**CLI**（shell 调用）和 **MCP server**（agent 直接工具调用）

## 快速上手

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

### 3. 用 CLI 做第一首

```bash
# 生成一个示例工程 JSON
cargo run -p muser-cli -- new -o song.muser.json

# 校验
cargo run -p muser-cli -- validate song.muser.json

# 渲染为 WAV
cargo run -p muser-cli -- render song.muser.json -o song.wav

# 查看可用乐器
cargo run -p muser-cli -- instruments
```

发布构建后也可直接用二进制 `target/release/muser`。

### 4. 以 MCP server 运行（给 agent 用）

```bash
cargo run -p muser-mcp          # stdio MCP server，暴露 10 个 tools
```

在 MCP 客户端（如 Claude Desktop）中把该可执行文件注册为一个 stdio server，
agent 即可通过 `create_project / add_track / add_clip / add_notes / render` 等工具编曲。

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
├─ samples/           示范工程（lo-fi/techno/house/dnb）
├─ ARCHITECTURE.md    架构、分层、工程 JSON schema、路线图
├─ SKILLS.md          给 AI 的编曲操作手册
└─ TOOLS.md           CLI + MCP 工具清单
```

## 深入阅读

- [ARCHITECTURE.md](ARCHITECTURE.md) — 三层架构、数据流、工程文件格式、vendored 策略、路线图
- [SKILLS.md](SKILLS.md) — AI 如何表达音乐意图（时间/音高/乐器/效果速查、工作流、排错）
- [TOOLS.md](TOOLS.md) — CLI 命令与 MCP tools 清单
- [samples/](samples/) — 四种风格的示范工程

## 许可

本仓库自身代码遵循 MIT / Apache-2.0 双许可。vendored 的 manguebyte 同为 MIT / Apache-2.0。
