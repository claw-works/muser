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
- `--seed <n>`：临时换工程的随机种子（不改文件）：`humanize` 与 random 自动化换一遍
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

### `muser plugin scan|info|params|values`
- `scan [--kind instrument|effect] [--refresh]`：扫描本机标准插件目录（每个插件在独立进程读取，有缓存），给出 `path#id`。
- `info <插件>`：名称、厂商、类型、是否接收音符、输入输出、参数总数与按名称分组统计、同文件其它插件。
- `params <插件> [--filter 关键词] [--params JSON] [--limit N] [--all]`：参数名、范围、当前值与文字、id；
  `--params` 先应用基础参数（参数含义随它变化时用）。
- `values <插件> <参数> [--params JSON]`：该参数可写的文字值（选项类即全部选项）。
- `profile list|show|draft|check|path`：插件画像（常用参数别名、预设、setup、经验的数据文件）。
  `show <插件|画像.json> [--markdown]`；`draft <插件> [--audition] [--save | -o 文件]`；`check <画像.json> [--plugin <插件>]`。
- `save-state <插件> [--params JSON] -o <文件>`：保存插件完整状态，工程里用 `"@state": "<文件>"` 加载。
- `edit <插件> [--params JSON] -o <文件>`（macOS）：打开插件自己的界面让人选音色，关窗口时保存状态；`muser view` 的 🎛 面板用的就是它。
- `<插件>` 是路径，可带 `#插件id`。都支持 `--json`。

### `muser sounds list|describe`

- `list [--kind builtin|sfz|plugin] [--filter 词] [--json]`：本机能用的音色（内置乐器、`$MUSER_SFZ_PATH` 里的 SFZ、扫描到的乐器插件），听过的附一句卡片描述。
- `describe <音色> [--params JSON] [--drums] [--refresh] [--json]`："听"一个音色：渲染探测音，测亮度、起音、延音、余音、音域、力度、立体声、音高，
  写成文字卡片，缓存在本机（`~/Library/Caches/muser/sounds`、`~/.cache/muser/sounds`）。插件用 `--params '{"@state": …}'` 指定音色。

### `muser audition`
试听一个参数的作用：`--instrument <乐器>`（或 `--effect '<效果器 JSON>'` + `--source <乐器>`），
`--params '<基础参数 JSON>'`，`--param <参数名> --values=a,b,c`（数字或插件可解析的文字），
`--pitch` `--hold` `--tail` `--out <目录>`（写出每版 wav）`--json`。
报告峰值、RMS、LUFS、频谱重心、低/中/高频占比、起音时间、余音长度。内置与外置都可用。
`--record` 把结果记进该插件的用户画像。

### `muser audio <file> [--tempo T] [--json]`

音频文件的格式、时长、采样率、声道；给了 `--tempo` 时换算成拍（写 audio 音轨 clip 的 `length` 用）。见 [skills/audio.md](skills/audio.md)。

### `muser routing <project>`

打印信号流：处理顺序、每条音轨/总线的去向（output、发送量）、效果链与侧链关系。改完路由先看一眼。

### `muser mix-report <project> [--json]`

混音报告：渲染一遍整首，列出每条音轨/总线/主总线的响度——输入（效果链前，音轨即乐器干声）与输出（推子后）的
积分 LUFS、短时最大 LUFS、峰值；音轨到主总线的估计响度（加上输出路径上各总线 gain_db）与相对最响音轨的差；
以及没声音、过响、主总线过载、削波等提示。用来平衡各轨 `gain_db`，流程见 [skills/mixing.md](skills/mixing.md)。

### `muser view <project> [--port 8737] [--open]`

给人听的播放页：在 `127.0.0.1` 起本地服务，浏览器打开 `http://127.0.0.1:8737/`（`--open` 自动打开）。
时间线（波形、clip、音符）、播放/循环/跳转（空格键随时播放/暂停），每条音轨和总线的音量推子、静音、独奏、电平表、
信号链（效果器方块 → 去向、发送量），以及整体信号流图（输出、发送、侧链连线）。

- 页面按播放进度向服务分段取已经混好的音频，浏览器只占几秒钟的内存，曲子多长都行。
- 改音量、静音、独奏：交给引擎重新渲染（乐器输出有缓存，只重跑效果、总线、主总线；2 分钟的 15 轨曲子约 1.7 秒），
  新版本从当前播放位置无缝换上，结果是准确的。独奏只用于试听，不写进工程。
- 有音量自动化（`gain_db`）的轨：自动化是绝对 dB，推子改动会把它整体平移同样的量（trim），保存时一并写回。
- 「保存到工程文件」只写回改过的 `gain_db` / `muted`；「撤销改动」回到文件里的值；「复制改动 JSON」交给 AI。
- 工程文件被别人（AI）改了：自动重新渲染、页面刷新（页面上未保存的改动丢弃）。
- 只监听本机、无鉴权（拒绝非本机 Host/Origin 的请求）。远程机器上用 SSH 端口转发访问。请用 release 构建。

改动 JSON 格式：`{"changes": [{"node": "tracks[3]", "gain_db": -9}, {"node": "buses[1]", "muted": true}]}`。
分享作品：分享工程 JSON（和插件状态文件）或渲染好的 WAV。

### `muser poly edit|list|show|schema`

poly 合成器的完整音色（振荡器、sub / 噪声、两个滤波器、四个包络、三个 LFO、16 槽调制矩阵）。
- `edit [--port 8739] [--open]`：本地网页音色编辑器（只监听 127.0.0.1）。从预设或本机音色库开始，改参数后自动渲染一段
  试听乐句（和弦、单音、贝斯线、琶音、连奏旋律，可选根音），存进本机音色库 `~/.config/muser/poly/<名字>.json`。
- `list [--json]`：本机音色库里的音色。
- `show <名字> [--sound]`：输出写进工程音轨的 JSON（`"instrument": "synth.poly"` + `instrument_params.patch` 完整音色）；
  名字可以是音色库里的、`poly.<预设>` 或 `synth.poly`。`--sound` 输出音色文件本身。
- `schema`：完整音色的结构（每个字段的类型、范围、选项、说明），`instrument_params.patch` 按它写。

### `muser play <project> [--seed N] [--from 秒]`

用本机声卡把工程从头放一遍，放完退出：给人快速试听草稿（不开网页）。`--seed` 临时换一遍演奏（humanize / drummer /
random 自动化），不改文件。需要 device 特性（发行版都带；headless 版没有，用 `render` 再听）。

### `muser loop edit|list|show|play|save|share`

片段库：鼓的节奏型（`grid`）与旋律 / 动机 / 和声音型（`notes`）。三处，同名时近的优先：
项目（工程旁的 `loops/`，没有工程时当前目录的 `loops/`）> 全局（`~/.config/muser/loops/`）> 内置（17 个常见鼓型）。
各命令的 `--project <工程文件>` 指定项目目录。
- `edit [名字] [--project P] [--port 8739] [--open]`：loop / 动机编辑器（本地网页，只听本机）。旋律用卷帘（点、拖、改长度、
  力度、移调、撤销、对齐网格或自由时值），鼓用步进网格；用 muser 的乐器渲染、循环试听；「● 哼唱录音」用麦克风哼一段直接转成音符；
  存到项目或全局，或把项目里的分享到全局。
  agent 改了 loop 文件，页面自动重新读入（有未保存的改动时提示）。
- `list [--style S] [--json]`：列出（标出 项目 `+` / 全局 `*`），按风格或标签筛。
- `show <名字> [--start 拍] [--repeat N] [--file | --abc]`：输出写进工程的 clip（`grid` / `notes` + `humanize` 内嵌，`from` 标注来源）；
  `--file` 输出 loop 文件（含建议乐器、调、和弦）；`--abc` 输出 ABC 记谱。
- `play <名字> [--instrument X] [--tempo BPM] [--repeat N]`：声卡试听。
- `save <project> --track I [--clip J] --name N [--style S] [--description D] [--step 0.25] [--global] [--overwrite]`：
  把工程里的 clip 存成 loop（鼓：grid，普通音符量化成网格；旋律：音符），缺省存到工程旁的 `loops/`。
- `share <名字> [--overwrite]`：把项目里的 loop 复制到全局，别的项目也能用。
- `hum <音频> [--name N] [--tempo 100] [--grid 1/16|1/8|1/8t|0.25|off] [--from-start] [--key Am] [--instrument X] [--global]`：
  哼唱转谱（不用 AI）。单旋律录音（哼、唱、口哨、单音乐器）→ 音符，存成旋律 loop，并打印 ABC。McLeod 法测音高（每 10 ms），
  中值滤波、整体音准校正，按停顿 / 音高跳变 / 同音高上的响度起伏切音，减掉整体的早晚后对齐网格，估计调。
  `--from-start` 保留录音开头到第一个音的空白（跟着节拍器录时用）。编辑器里的「● 哼唱录音」是同一套（浏览器录麦克风，可带一小节预备拍与节拍器）。
- loop 文件格式（`format: "muser.loop"`, `version: 2`）：`name` `bars` `tempo` `style` `description` `tags` `kit` / `instrument`
  `key` `chords` `grid` / `notes` `humanize`。drummer 的 `loop` 也能引用项目或全局里的鼓 loop。
clip 上的简写（`abc` / `grid` / `drummer`）与 `humanize` 见 [skills/notation.md](skills/notation.md)、[skills/humanize.md](skills/humanize.md)。

### `muser midi <project> [-o out.mid] [--seed N]`

导出标准 MIDI 文件（format 1，480 tick/四分音符）：第一条轨放速度与拍号，之后每条乐器音轨一条 MIDI 轨。
音符与渲染完全一致：humanize 后的时间与力度、连奏延长、clip 裁剪、abc / grid / drummer 展开；延音踏板写成 CC64，
`gain_db` / `pan` 写成 CC7 / CC10。鼓组在通道 10，其它音轨按乐器名猜一个 GM 音色。静音音轨照常导出（轨名标 "(muted)"），
audio 音轨不导出。缺省输出为工程同名的 `.mid`。`muser view` 页面的「导出」也能下载 MIDI 与整首混音 WAV。

### `muser analyze <project> [--audio] [--json]`

听感报告，给听不到声音的 agent 自查编曲。按段落（有 `form` 时按曲式的每一遍，否则每 8 小节）列出：
密度（每小节起音数）、各轨每小节起音数 / 发声占比 / 音区、和弦与和声节奏（写好的 `chords`，没写时估计、带 `~`）、
重复度（与前面某小节完全相同的比例）、冲突（拍点长音是避免音；两轨同时长音成小二度 / 小九度）。
`--audio` 渲染一遍，加每段的响度、频谱重心与低 / 中 / 高频能量占比。最后是建议：副歌没起来、重复太多、只有一个和弦、
音区掩蔽、冲突、各段响度太平、偏暗 / 偏亮。最上面是能量曲线（每段 0–10）；还有一组"AI 味"检查（`tells`）：
段落全是 8 小节的倍数、能量只升不降、编制不变、一个和声循环 / 不离调、过门每 4 小节一次、全部对齐网格、副歌每遍一样、
（`--audio`）动态与明暗不变。见 [skills/arrange.md](skills/arrange.md)。

### `muser abc <project> [-o out.abc] [--yue2] [options]`

导出 ABC 记谱（缺省打印到 stdout），写谱面上的音（humanize 之前、clip 裁剪之后；abc / grid / drummer 展开的都算），
鼓组与 audio 音轨不导出。起点与时值量化到 1/16（需要时 1/32、1/64）音符网格，三连音等就近取整（会提示）。

- 标准写法（缺省）：ABC 2.1 多声部，每条乐器音轨一个 `V:`（低音区用低音谱号），复音写成 `[CEG]`、跨小节用连音线。
  `--track <名>`（可重复）只导出这几条；`--chords` 加和弦记号。
- `--yue2`：[YuE2](https://github.com/multimodal-art-projection/YuE) 的原生谱面——`Vocal` / `Ins` 两个单音声部，
  和弦记号写在 `Vocal`（只用它支持的 15 种和弦），时值只用 1 2 3 4 6 8 12 16 24 32 48 个单位、其它用连音线拼，
  每组 1–4 小节一个 `V: Vocal` 块接一个 `V: Ins` 块。给 `--cot full` 用；加 `--no-chords` 给 `--cot melody`（翻唱/改编）。
  - `--vocal a,b,c`：唱旋律的音轨，旋律在几件乐器间传递时写几条（合并后每个时刻取最高的音）；缺省按名字
    （vocal / lead / melody…），否则取偏高音区里发声最久的。`--ins <名>`：器乐旋律声部（缺省全休止）。
  - `--sections intro:4,verse:8,chorus:8`：段落写成 `% verse` 等注释，与歌词里的 `[Verse]` 标签对应。
  - 网格至少用 `L:1/32`（YuE2 原生导出常用 1/32；第三方实测 `L:1/16` 的谱会生成到一半停）。
  - `--vocal-octave 1`：Vocal 整体升八度，写到小写音名区（第三方实测这样咬字更清楚，官方没有说明）。
  - 导出结果用 YuE2 的 `abc_tools.py inspect` 校验通过。
- `--key G` / `--key F#m`：指定调号；缺省用 clip 里 ABC 写的 `K:`（都一样时），否则由整首的音高估计。
- 和弦记号是按每小节（必要时半小节）的伴奏音估计的：伴奏清楚时可靠，只有一条低音线时只是猜测，交给模型前可以人工改。

`muser view` 页面的「导出」也能下载 ABC 与 ABC（YuE2）（选项都用缺省）。

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

### 已实现的 Tools（38 个）

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
| `add_clip` | 新增片段（可带延音踏板 `pedal`），返回 `clip_index` |
| `set_pedal` | 设置片段的延音踏板区间（CC64，踩下期间松键继续发声） |
| `add_notes` | 向片段加入一批音符（支持 `velocity`、`accent`） |
| `clear_clip` | 清空片段内所有音符 |
| `set_automation` | 设置某音轨（`track_index`）、总线（`bus`）或主总线的自动化（替换或追加），立即校验 |
| `set_seed` | 设置工程随机种子（决定 random 自动化） |
| `render` | 离线渲染为 WAV，返回时长/峰值 |
| `export_midi` | 导出标准 MIDI 文件（音符与渲染一致，audio 音轨不导出） |
| `analyze` | 听感报告：按段落的密度、音区、和弦、重复度、冲突与建议（`audio` 加响度与频谱重心） |
| `export_abc` | 导出 ABC 记谱：`standard`（ABC 2.1 多声部）或 `yue2`（YuE2 原生谱面） |
| `mix_report` | 混音报告：各轨/总线/主总线的干声与推子后响度、相对差与提示（乐器输出按工程缓存） |
| `scan_plugins` | 扫描本机外置插件 |
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
