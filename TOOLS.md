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
校验工程 JSON：tempo、clip、音符、声像、乐器名，以及每个效果器参数是否在允许范围内。
错误信息给出具体位置，如 `tracks[0].effects[0]: reverb.mix = 1.8 超出范围 [0, 1]`。
- **输出**：`OK: '<name>' 合法 — tempo <t>, <n> 轨, 时长 <d> 拍`，或非零退出码 + 错误。

### `muser instruments [name] [--family <F>] [--json]`
- 无参数：按类别列出全部 115 个内置乐器及说明。
- `--family`：只列某类（drums / synth / poly / guitar / bass / organ / mallet / strings / wind / choir）。
- `<name>`：显示该乐器的类别、音高约定（GM 鼓键 / 音阶音高）、推荐音域、复音数、说明，
  以及可调参数（`instrument_params` 的键、范围、单位、说明、该预设当前值、能否自动化）。
- `--json`：机器可读输出。

### `muser effects [name] [--json]`
- 无参数：列出全部效果器及一句话说明。
- `<name>`：显示该效果器的放置建议、每个参数的默认值/范围/单位/听感说明，以及 JSON 示例。
- `--json`：机器可读输出（即代码中的效果器描述表）。

### `muser plugin scan|info|params|values`（需 host 特性）
- `scan [--kind instrument|effect] [--refresh]`：扫描本机标准插件目录（每个插件在独立进程读取，有缓存），给出 `path#id`。
- `info <插件>`：名称、厂商、类型、是否接收音符、输入输出、参数总数与按名称分组统计、同文件其它插件。
- `params <插件> [--filter 关键词] [--params JSON] [--limit N] [--all]`：参数名、范围、当前值与文字、id；
  `--params` 先应用基础参数（参数含义随它变化时用）。
- `values <插件> <参数> [--params JSON]`：该参数可写的文字值（选项类即全部选项）。
- `profile list|show|draft|check|path`：插件画像（常用参数别名、预设、setup、经验的数据文件）。
  `show <插件|画像.json> [--markdown]`；`draft <插件> [--audition] [--save | -o 文件]`；`check <画像.json> [--plugin <插件>]`。
- `save-state <插件> [--params JSON] -o <文件>`：保存插件完整状态，工程里用 `"@state": "<文件>"` 加载。
- `<插件>` 是路径，可带 `#插件id`。都支持 `--json`。

### `muser audition`
试听一个参数的作用：`--instrument <乐器>`（或 `--effect '<效果器 JSON>'` + `--source <乐器>`），
`--params '<基础参数 JSON>'`，`--param <参数名> --values=a,b,c`（数字或插件可解析的文字），
`--pitch` `--hold` `--tail` `--out <目录>`（写出每版 wav）`--json`。
报告峰值、RMS、LUFS、频谱重心、低/中/高频占比、起音时间、余音长度。内置与外置都可用。
`--record` 把结果记进该插件的用户画像。

### `muser routing <project>`

打印信号流：处理顺序、每条音轨/总线的去向（output、发送量）、效果链与侧链关系。改完路由先看一眼。

### `muser view <project> [--port 8737] [--open]`

给人听的播放页：在 `127.0.0.1` 起本地服务，浏览器打开 `http://127.0.0.1:8737/`（`--open` 自动打开）。
时间线（波形、clip、音符）、播放/循环/跳转，每条音轨和总线的音量推子、静音、独奏。

- 页面按播放进度向服务分段取已经混好的音频，浏览器只占几秒钟的内存，曲子多长都行。
- 改音量、静音、独奏：交给引擎重新渲染（乐器输出有缓存，只重跑效果、总线、主总线；2 分钟的 15 轨曲子约 1.7 秒），
  新版本从当前播放位置无缝换上，结果是准确的。独奏只用于试听，不写进工程。
- 「保存到工程文件」只写回改过的 `gain_db` / `muted`；「撤销改动」回到文件里的值；「复制改动 JSON」交给 AI。
- 工程文件被别人（AI）改了：自动重新渲染、页面刷新（页面上未保存的改动丢弃）。
- 只监听本机、无鉴权（拒绝非本机 Host/Origin 的请求）。远程机器上用 SSH 端口转发访问。请用 release 构建。

改动 JSON 格式：`{"changes": [{"node": "tracks[3]", "gain_db": -9}, {"node": "buses[1]", "muted": true}]}`。
分享作品：分享工程 JSON（和插件状态文件）或渲染好的 WAV。

### `muser new [-o <path>]`
生成一个示例工程 JSON（AI 可用来参考标准格式）。缺省打印到 stdout。

---

## 二、MCP Server（已实现：`muser-mcp`）

crate `muser-mcp` 以 **stdio** 运行一个 MCP server，让 Claude/Cursor 这类 agent
**无需 shell**，直接以结构化 tool call 驱动引擎。它依赖 `muser-core` + `muser-engine`，
与 CLI 同源。

### 运行

```bash
muser-mcp          # 以 stdio 启动 MCP server（开发期: cargo run -p muser-mcp）
```

在 MCP 客户端（如 Claude Desktop）的配置里，把 `muser-mcp` 可执行文件作为一个
stdio server 注册即可。服务器初始化时会返回使用说明（instructions），告诉 agent
如何用 beats 时间、GM 鼓键、乐器语义名等。

### 已实现的 Tools（33 个）

| tool | 作用 |
|---|---|
| `create_project` | 新建空工程，返回 `project_id` |
| `get_project` | 取工程完整 JSON |
| `set_project` | 用完整 JSON 整体替换工程 |
| `validate_project` | 校验工程 |
| `load_project` | 从磁盘读工程 JSON 到会话，返回新 `project_id` |
| `save_project` | 把会话中的工程写到磁盘 JSON |
| `set_tempo` | 设置速度 |
| `add_track` | 新增音轨（可带 `instrument_params`、`output`、`sends`），返回 `track_index`；乐器名、参数与路由立即校验 |
| `remove_track` | 删除音轨 |
| `update_track` | 更新增益/声像/静音、更换乐器、`output`，合并式更新 `instrument_params` 与 `sends`（值为 null 删除该键）；立即校验 |
| `set_bus` | 新增或替换（同名）一条总线：effects / gain_db / pan / muted / output / sends / automation；立即校验名称、引用与环路 |
| `remove_bus` | 删除总线；仍被引用时报错并指出引用处 |
| `get_routing` | 查看信号流：处理顺序、去向与发送量、侧链关系 |
| `add_clip` | 新增片段，返回 `clip_index` |
| `add_notes` | 向片段加入一批音符（支持 `velocity`、`accent`） |
| `clear_clip` | 清空片段内所有音符 |
| `set_automation` | 设置某音轨（`track_index`）、总线（`bus`）或主总线的自动化（替换或追加），立即校验 |
| `set_seed` | 设置工程随机种子（决定 random 自动化） |
| `render` | 离线渲染为 WAV，返回时长/峰值 |
| `scan_plugins` | 扫描本机外置插件（需 host 特性） |
| `plugin_info` | 外置插件信息与参数分组 |
| `list_plugin_params` | 搜索外置插件参数（filter 关键词，可先应用基础参数） |
| `plugin_param_values` | 外置插件某参数可写的文字值 |
| `audition` | 试听：按参数取值逐个渲染测试音，返回响度/亮度/频段/起音/余音；`record` 记进画像 |
| `list_profiles` | 列出插件画像 |
| `get_profile` | 取某插件的画像（JSON 或 Markdown 使用说明） |
| `draft_profile` | 由插件自动生成画像草稿 |
| `save_profile` | 校验并保存画像到用户目录（可在插件上检查） |
| `save_plugin_state` | 保存插件完整状态（音色）文件 |
| `list_instruments` | 列出乐器（可按 `family` 过滤），返回名称/类别/音高约定/音域/复音数/说明 |
| `describe_instrument` | 查看某个乐器的详情与可调参数（含该预设当前值） |
| `list_effects` | 列出所有效果器及说明 |
| `describe_effect` | 查看某个效果器的参数（范围/默认/单位/听感）、放置建议与 JSON 示例 |

工程以**内存会话**（`project_id → Project`）管理。agent 的标准用法是**顺序**调用
（等每个工具返回后再发下一个），这与真实 MCP 客户端行为一致。

> 说明：服务器对每次工具调用持短时锁，正常的请求/响应式 agent 交互是安全的；
> 若客户端异常地并发发起多个编辑，可能交错。未来可改为按 `project_id` 加会话级锁。

### 设计原则（原始设计，供扩展参考）

- **工程以句柄 + 内存状态管理**，避免每次都在磁盘读写大 JSON；同时支持 `load`/`save` 落盘。
- 每个 tool 的输入/输出都是**结构化 JSON**，字段与 `muser-core` 模型一一对应。
- **渲染返回元信息**（时长、峰值、文件路径），音频本身写文件，返回路径而非二进制。
- 工具粒度兼顾"整体提交"与"增量编辑"两种 agent 工作风格。

### 可扩展的 Tools（尚未实现）

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
