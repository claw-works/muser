# muser 工具清单 (CLI + MCP 设计)

muser 的接口层以两种形态暴露给 AI：

1. **CLI**（已实现）：AI 通过 shell 调用 `muser <子命令>`。
2. **MCP server**（规划中）：把同一批能力包成 agent 可直接调用的 tools。

两者共享同一组语义，保证行为一致。

---

## 一、CLI（已实现）

可执行文件名：`muser`（来自 `muser-cli` crate）。

### `muser render <project> [options]`
读工程 JSON，离线渲染为 WAV。
- `<project>`：工程文件路径（`.json`）
- `-o, --output <path>`：输出 WAV 路径（默认 `out.wav`）
- `--sample-rate <hz>`：采样率（默认 48000）
- `--tail <seconds>`：结尾余响秒数（默认 2.0）
- **输出**：`已渲染 '<name>' → <out>: <时长>s, 峰值 <dBFS>`

```bash
muser render beat.muser.json -o beat.wav
```

### `muser validate <project>`
校验工程 JSON 是否合法（tempo/clip/乐器名等）。
- **输出**：`OK: '<name>' 合法 — tempo <t>, <n> 轨, 时长 <d> 拍`，或非零退出码 + 错误。

### `muser instruments`
列出所有可用乐器语义名。

### `muser new [-o <path>]`
生成一个示例工程 JSON（AI 可用来参考标准格式）。缺省打印到 stdout。

---

## 二、MCP Server 设计（规划）

目标：让 Claude/Cursor 这类 agent **无需 shell**，直接以结构化 tool call 驱动引擎。
建议新建 crate `muser-mcp`（依赖 `muser-core` + `muser-engine`，复用同一套逻辑）。

### 设计原则

- **工程以句柄 + 内存状态管理**，避免每次都在磁盘读写大 JSON；同时支持 `load`/`save` 落盘。
- 每个 tool 的输入/输出都是**结构化 JSON**，字段与 `muser-core` 模型一一对应。
- **渲染返回元信息**（时长、峰值、文件路径），音频本身写文件，返回路径而非二进制。
- 工具粒度兼顾"整体提交"与"增量编辑"两种 agent 工作风格。

### Tools 清单

#### 工程生命周期

| tool | 输入 | 输出 | 说明 |
|---|---|---|---|
| `create_project` | `{ name?, tempo, time_signature? }` | `{ project_id }` | 新建空工程，返回句柄 |
| `load_project` | `{ path }` | `{ project_id, project }` | 从磁盘 JSON 载入 |
| `save_project` | `{ project_id, path }` | `{ path }` | 落盘为 JSON |
| `get_project` | `{ project_id }` | `{ project }` | 取当前完整工程 JSON |
| `set_project` | `{ project_id, project }` | `{ ok }` | 整体替换（适合"一次性提交完整构思"） |
| `validate_project` | `{ project_id }` | `{ ok, errors? }` | 校验 |

#### 增量编辑（适合"对话式微调"）

| tool | 输入 | 输出 |
|---|---|---|
| `add_track` | `{ project_id, name?, instrument, gain_db?, pan? }` | `{ track_index }` |
| `remove_track` | `{ project_id, track_index }` | `{ ok }` |
| `update_track` | `{ project_id, track_index, gain_db?, pan?, muted? }` | `{ ok }` |
| `add_clip` | `{ project_id, track_index, start, length }` | `{ clip_index }` |
| `add_notes` | `{ project_id, track_index, clip_index, notes: [...] }` | `{ added }` |
| `clear_clip` | `{ project_id, track_index, clip_index }` | `{ ok }` |
| `set_tempo` | `{ project_id, tempo }` | `{ ok }` |

#### 渲染与发现

| tool | 输入 | 输出 |
|---|---|---|
| `render` | `{ project_id, output_path, sample_rate?, tail?}` | `{ path, seconds, peak_dbfs }` |
| `list_instruments` | `{}` | `{ instruments: [...] }` |
| `describe_instrument` | `{ name }` | `{ kind, note_convention, hint }` |

### 实现路径（最小改动复用现有代码）

```
muser-mcp (新 crate)
  ├─ 维护 HashMap<ProjectId, muser_core::Project>  // 内存会话状态
  ├─ 每个 tool = 对 Project 的一次结构化变更 / muser_engine::render 调用
  └─ 用任一 Rust MCP SDK 暴露为 stdio / SSE server
```

因为 `muser-core` 的模型已全 serde，几乎所有 tool 的 I/O 可直接用模型类型派生，
`render` 直接转调 `muser_engine::render_to_wav`——与 CLI 完全同源，零行为分叉。

### 与 CLI 的关系

CLI 和 MCP 是同一引擎的两张脸：
- CLI 适合**脚本化/批处理**和"AI 通过 shell 工作"的场景。
- MCP 适合**对话式 agent** 直接工具调用、维持多轮会话中的工程状态。

两者都只是 `muser-core` + `muser-engine` 的薄封装。
