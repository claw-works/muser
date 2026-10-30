# muser Skill：如何用 muser 编曲

> 写给 **AI agent** 的操作手册：把用户的音乐意图翻译成 muser 工程 JSON，再渲染成音频。
> 这里是总入口；每个乐器类别、每个效果器的细节在 [`skills/`](skills/) 下的子 skill 里，按需查阅。

## 你的角色

用户用自然语言描述想要的音乐（"做一个 90bpm 的 lo-fi 节拍，钢琴+鼓"）。你的工作：

1. 选乐器、写音符、配效果器，产出一个 **Project JSON**。
2. 先 `validate`，再渲染成 WAV。
3. 根据反馈**增量修改** JSON，重新渲染。

你只在 Project JSON 这一层工作，不需要懂底层 DSP。

## 先听后写：和人一起创作的流程

音乐创作是感性的：**先拿草稿给人听，再写整首**。不要一上来就编配完整的曲子。

1. **出草稿**：用户描述想法后，先写 4–8 小节的草稿，最好一次给 2–3 个版本（A / B / C：同一段旋律配不同节奏、
   不同 loop、不同速度或音色）。旋律用 `abc`、鼓用 loop / `drummer`、律动用 `humanize`（见 [skills/notation.md](skills/notation.md)），
   一个版本几行 JSON，不要写脚本生成音符。
2. **一句话 + 一条命令**：每个版本说一句它是什么感觉，给出试听命令 `muser play sketch-a.muser.json`
   （用声卡放一遍就退出）。鼓型单独试 `muser loop play <名字>`。
3. **问具体的问题**，而不是"你觉得怎么样"：快一点还是慢一点？旋律更跳还是更平？鼓更满还是更空？更亮还是更暗？
   要不要换一种乐器？哪个版本最接近？
4. **按反馈改草稿**，再听；同一个版本"换一遍演奏"：`muser play sketch-a.muser.json --seed 2`。
5. **满意了再展开**：在定下来的草稿上编配整首——用编曲层写（`sections` + `form` + 各轨 `parts`，和弦写在段落上，
   贝斯、和声、琶音用 `chordal` 跟着和弦走，见 [skills/arrange.md](skills/arrange.md)），不要手算一个个 clip 的起点。
   写完跑 `muser analyze` 自查（能量曲线有没有起伏、副歌有没有起来、重复、掩蔽、冲突、"AI 味"检查），按建议改；最后 `mix-report` 平衡音量，`muser view` 交给人微调。

人可以先给动机：`muser loop edit --open` 打开编辑器，人在卷帘里画几小节或对着麦克风哼一段（自动转成音符），存到项目的 `loops/`，你读出来续写 / 扩写成几个版本
（存成新的 loop，编辑器里能直接试听、修改），见 [skills/notation.md](skills/notation.md) 的"人给动机，agent 续写"。

用户这样说时进入草稿流程（不要直接改整首）："先听听""来几个版本""随便哼一段""感觉不对""这段再活泼点""换个感觉"。

## 先查再写：可发现性

不要凭记忆写乐器名和效果器参数，先查：

| 想知道 | CLI | MCP tool |
|---|---|---|
| 有哪些乐器 | `muser instruments [--family F]` | `list_instruments` |
| 本机所有能用的音色（内置、SFZ、乐器插件）| `muser sounds list [--kind builtin\|sfz\|plugin] [--filter 词]` | — |
| 一个音色听起来怎样（亮度、起音、延音、音域、力度、立体声、音高）| `muser sounds describe <音色> [--params JSON]` | — |
| 某乐器的音高约定、音域、复音数、**可调参数与预设当前值** | `muser instruments <名称>` | `describe_instrument` |
| 有哪些效果器 | `muser effects` | `list_effects` |
| 某效果器的参数、范围、默认值、示例 | `muser effects <名称>` | `describe_effect` |
| 工程是否合法、错在哪 | `muser validate <file>` | `validate_project` |
| 本机有哪些外置插件 | `muser plugin scan` | `scan_plugins` |
| 插件的参数、选项文字 | `muser plugin params <插件> --filter <词>`、`muser plugin values <插件> <参数>` | `list_plugin_params`、`plugin_param_values` |
| 某个参数到底起什么作用 | `muser audition ... --param <名> --values ...` | `audition` |
| 各轨音量是否均衡 | `muser mix-report <file>` | `mix_report` |
| 编曲好不好：密度、音区、和弦、重复、冲突与建议 | `muser analyze <file> [--audio]` | `analyze` |
| 有哪些现成的鼓型 / 动机（loop），拿一个放进工程 | `muser loop list [--style S]`、`muser loop show <名字> [--abc]` | — |
| 让人画一段动机 / 鼓型，或对着麦克风哼一段（编辑器，存到项目） | `muser loop edit --open` | — |
| 把哼唱 / 口哨录音转成音符 | `muser loop hum <音频> --tempo T --name N` | — |
| 本机音色库里的 poly 音色 | `muser poly list`、`muser poly show <名字>` | — |
| 音频文件的格式、时长（换算成拍） | `muser audio <文件> [--tempo T]` | — |
| 把编曲交给 DAW / 分析工具 | `muser midi <工程> [-o x.mid]` | `export_midi` |
| 把乐谱交给读 ABC 的模型（如 YuE2） | `muser abc <工程> [--yue2 --vocal 旋律轨 --sections verse:8,chorus:8]` | `export_abc` |
| 让人快速听一下草稿 | `muser play <file> [--seed N]`、`muser loop play <名字>` | — |
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
- `bend`（弯音，吉他推弦等）`[[拍, 半音], ...]`，见下文"弯音：bend"。

## 乐器索引（共 115 个内置 + SFZ 音色库 + 外置 VSTi）

| 类别 | 子 skill | 代表 |
|---|---|---|
| 鼓组 | [drums.md](skills/instruments/drums.md) | `drums.808` `drums.909` `drums.acoustic` `drums.trap` `drums.lofi` … 10 套 |
| 合成器预设 | [poly.md](skills/instruments/poly.md) | 68 个：`poly.moog_bass` `poly.trance_supersaw` `poly.warm_analog_pad` `poly.juno_keys` …；完整音色 `patch` 与本机音色库（`muser poly list / show`） |
| 通用合成器 | [synth.md](skills/instruments/synth.md) | `synth.poly` `synth.fm` |
| 吉他 | [guitar.md](skills/instruments/guitar.md) | `guitar.nylon` `guitar.steel` |
| 贝斯 | [bass.md](skills/instruments/bass.md) | `bass.finger` `bass.slap` `bass.acid` |
| 风琴 | [organ.md](skills/instruments/organ.md) | `organ.jazz` `organ.gospel` `organ.full` `organ.ballad` |
| 打击旋律 | [mallet.md](skills/instruments/mallet.md) | `mallet.marimba` `mallet.vibraphone` `mallet.kalimba` … |
| 弦乐 | [strings.md](skills/instruments/strings.md) | `strings.violin` `strings.cello` `strings.ensemble` `*_pizz` … |
| 管乐 | [wind.md](skills/instruments/wind.md) | `wind.flute` `wind.sax` `wind.trumpet` … |
| 人声 | [choir.md](skills/instruments/choir.md) | `choir.aah` `choir.ooh` `choir.solo` |
| 音频文件（录音、采样、loop） | [audio.md](skills/audio.md) | `"instrument": "audio"`，clip 写 `"audio": { "file": "vocal.wav" }` |
| SFZ 音色库 | [sfz.md](skills/instruments/sfz.md) | `sfz:/path/to/Piano.sfz` 或相对 `MUSER_SFZ_PATH` 的路径（内置采样器，无需插件） |
| 外置 VSTi | [plugin.md](skills/instruments/plugin.md) | `plugin:/usr/lib/vst3/Surge XT.vst3` |

所有内置音源都由 manguebyte 实时合成（物理建模、模态合成、FM、减法合成等），不依赖系统 MIDI 音源。

## 选音色：先"听"，再记成用户自己的 skill

你听不到声音，但 muser 可以替你听：`muser sounds describe` 渲染一组探测音（音阶乐器在 C2–C6 各弹一个音、再轻弹一下；
鼓组敲底鼓、军鼓、镲片），把测量结果写成文字卡片，缓存在用户本机（同一音色、参数与文件不变时直接读缓存，`--refresh` 重听）。

```bash
muser sounds list --filter "piano"                 # 候选：内置、$MUSER_SFZ_PATH 里的 SFZ、乐器插件；♪ 是听过的
muser sounds describe poly.wurly                   # 很暗、起音柔和、慢衰减、松开即停；音域约 C2–C6 + 逐项数据
muser sounds describe "sfz:Salamander…/Salamander….sfz"
muser sounds describe "plugin:<路径>#<id>" --params '{"@state": "<状态文件>"}'   # 插件：按这份状态听
```

1. **按需求列 2–5 个候选**，各听一遍，对照卡片挑：
   | 想要 | 看卡片里的 |
   |---|---|
   | 抒情、温暖的和弦 / 铺底 | 亮度"偏暗、温暖"、起音"柔和"或"慢"、延音"持续"或"慢衰减" |
   | 穿透的主旋律 | 亮度"明亮"、起音"快"、音域覆盖旋律所在的音区 |
   | 打击感、节奏型伴奏 | 起音"瞬间"、"快衰减" |
   | 低音线 | 音域里 C2 发声、亮度不太亮 |
   | 能表现强弱的独奏 | 力度"表现大"、重弹明显更亮 |
   | 要合成单声道 / 和别的宽声部叠加 | 立体声不要"侧信号比中间还强" |
   卡片是测量换来的，不是真正的听感：选定后放进曲子里渲染，用 `muser analyze --audio` 看效果，最后请人听一下。
2. **插件的音色由它的状态决定**，别把插件当成只有一种声音。先看插件自带哪些音色：
   ```bash
   muser sounds presets "<插件>" --filter "pad evolving"   # 名字、音色库、类别、标签；[自动] [已采集] [需载入]
   muser sounds state "<插件>" "<音色名>"                    # [自动] / [已采集]：给出 @state 文件和工程里的写法
   ```
   - **[自动]**：VST3 标准预设（`.vstpreset`）、Surge 的 `.fxp` 等能直接换成插件状态，`sounds state` 就能用。
   - **[需载入]**：Kontakt 的乐器（`.nki`）只能在它自己的界面里载入。载入一次后状态存进用户本机的音色库
     （`~/.config/muser/sound-library/`），之后变成 [已采集]，和自动的一样直接用。载入方式见下面"用 computer use 载入"；
     没有 computer use 时请人在 `muser view` 的 🎛 → 插件 → 点这个音色，或 `muser sounds capture` 里载入。
   - 有了状态就能 `muser sounds describe "<插件>" --params '{"@state": "<文件>"}'` 听，再调插件公开的参数
     （`muser plugin params "<插件>" --params '{"@state": …}'`，如 Kontakt 乐器的旋钮）。

   **用 computer use 载入（[需载入] 的音色，macOS）**——只有你有 computer use（能截屏、点鼠标、拖拽）时才这样做：
   1. 后台运行 `muser sounds capture "<插件>" "<音色名或文件>"`（会一直等到窗口关闭）。它会打开插件窗口，标题是
      「载入：<音色>（<音色库>）— 载入后关闭窗口 · muser」，并在 Finder 里选中这个音色文件。
   2. 截屏，找到这两个窗口。把 Finder 里选中的文件**拖进插件窗口**（Kontakt：拖到右侧乐器机架的空白处）。
      拖不动时在插件自己的浏览器里找到它双击（Kontakt 左栏 Files / Libraries）。
   3. 等它载入完：截屏确认机架里出现这个乐器的名字和界面、载入进度条走完（大音色库要几秒到几十秒）。
   4. 关闭插件窗口（左上角红色按钮）。命令会打印"已采集 …"和一句听感；若是"没有声音"，说明没载入成功，重来一次。
   5. 一次只载入一个。遇到授权、激活、"找不到音色库"、更新提示之类的对话框不要自己点掉——停下来告诉用户。
      不要改插件的设置、不要删东西。
   批量采集时逐个做、每个都确认听感不是"没有声音"，做完把采集过的音色和听感记进用户自己的 skill（下一条）。
3. **听完把结论记成用户自己的 skill**，下次直接用，不用重听。写在你（agent）自己的 skill / 记忆目录里，例如 Claude Code 的
   `~/.claude/skills/my-sounds/SKILL.md`；**不要**写进 muser 的仓库或工程目录——里面是用户本机的音色库、插件与路径。已有就更新，不要另建一份：

   ```markdown
   ---
   name: my-sounds
   description: 这台机器上用 muser 听过的音色（内置、SFZ、插件状态）与适合的用途。选音色前先看这里。
   ---
   | 音色（instrument） | 参数 / 状态 | 听感（muser sounds describe） | 适合 | 备注 |
   |---|---|---|---|---|
   | `sfz:Salamander…/Salamander….sfz` | — | 偏暗、起音快、快衰减、宽立体声；C2–C6 | 古典钢琴、抒情伴奏 | 加载约 10 秒 |
   | `plugin:…/Kontakt 8.vst3#…` | `@state: ~/…/song.piano.muserstate`（`@label` Piano Uno） | 偏暗、起音瞬间、很宽 | 流行钢琴 | 人在 🎛 里选的 |
   ```
   只记描述与用途，不复制采样或插件状态的内容。描述通用音源（某个免费 SFZ 库、某个插件的出厂预设）的这类 skill，以后可以分享给别人。

## 连奏：legato

clip 写 `"legato": true`：每个音自动延长到与下一个**不同音高**的音重叠约 40 毫秒，乐器按连奏演奏——
SFZ 音色的连奏采样（`trigger=legato`，如 Sonatina 的 `* Legato.sfz`）在前一个音还按着时换用无起音的过渡采样，
单音合成器（`Polyphony 1` + `Legato On` 的插件、acid 贝斯）会滑过去。

- 一个乐句一个 clip：乐句内连，句与句之间断开（空隙超过 1/4 拍视为休止，不连；同音重复会重新发音）。
- 弦乐、木管、铜管唱旋律时用；和弦、伴奏、断奏不用。连奏音色的第一个音起音较慢，乐句开头可以略早一点。
- 不想全连时，不写 `legato`，而是让想连的两个音手动重叠（前一个音结束晚于后一个音开始）。
- MCP：`add_clip` 可带 `legato`。

## 弯音：bend（吉他推弦、滑进音、尾音下滑）

音符上的 `bend` 是 `[[拍, 半音], ...]` 折线：拍相对音符起点、递增，半音在 ±2 之内（弯音轮的标准范围）；
第一个点之前取第一个点的值，点与点之间直线过渡。每个带 `bend` 的音只弯自己，同时响着的其它音不受影响。

```json
{ "pitch": 74, "start": 1, "length": 1.5, "bend": [[0, 0], [0.3, 2]] }
```

- 推弦（全音）：`[[0, 0], [0.3, 2]]`；半音推：`[[0, 0], [0.15, 1]]`。推得越快越"抽"，0.2–0.4 拍是常见的手感。
- 预推再放：`[[0, 2], [0.6, 2], [1.1, 0]]`（先推好再拨，慢慢放回原音）。
- 推弦揉弦：推到位后在 1.7–2 之间来回，如 `[[0, 0], [0.3, 2], [0.6, 1.7], [0.9, 2], [1.2, 1.7]]`。
- 滑进音（人声、萨克斯的 scoop）：`[[0, -1], [0.15, 0]]`；尾音下滑（fall）：`[[0, 0], [0.7, 0], [1, -2]]`。
- 支持：内置乐器（吉他、合成器、管弦等）、SFZ 采样器（按区域的 `bend_up` / `bend_down`，默认 ±2 半音）、插件乐器（发弯音轮，范围由插件决定，多为 ±2）。鼓组忽略。
- MIDI 导出写成弯音轮事件（一个音轨一个通道，所以导出后会带上同时响着的其它音）。
- MCP：`add_notes` 的音符可带 `bend`。

## 延音踏板：pedal

clip 上的 `pedal` 是延音踏板（CC64）踩下的区间 `[[起, 止], ...]`（拍，相对 clip 起点，递增、不重叠）。踩着时松开的键继续发声，
抬起时一起释音。内置合成器、SFZ 采样器、插件乐器都支持，鼓组忽略。

```json
{ "start": 0, "length": 8, "pedal": [[0.1, 4.03], [4.1, 8.0]], "notes": [ ... ] }
```

- 钢琴写法：音符按谱上的实际时值写（手指松开），延音交给踏板；不要把每个音拉长来模拟踏板。
- **每换和弦换一次踏板**，用"切分踏板"：新和弦的第一个音弹下之后（约 0.1 拍）再踩下，前一个区间延续到这一刻稍后抬起，
  这样新音不会混进旧和弦、也不会断开。不换踏板会把不同和弦糊在一起。
- MCP：`add_clip` 可带 `pedal`，或用 `set_pedal` 设置。
- 完整写法、怎么踩与示例：[skills/pedal.md](skills/pedal.md)。

### 人性化演奏（humanize）

音轨或 clip 上写 `"humanize": { "timing_ms": 6, "drift": 0.6, "velocity": 0.08, "swing": 58 }`，时间、力度、时值像真人一样有偏移，
再加 swing、整体提前/拖后。同一种子每次渲染一样；换工程 `seed`（或 `muser render --seed N`）就换一遍。
- 字段、量化含义与各风格的起点：[skills/humanize.md](skills/humanize.md)。

## 写整首：段落、曲式、和弦（编曲层）

```json
"sections": { "verse": { "bars": 8, "chords": "Am7 Fmaj7 C G" }, "chorus": { "bars": 8, "chords": "F G | Em Am" } },
"form": ["verse", "chorus", "verse", { "section": "chorus", "transpose": 2 }],
"tracks": [{ "name": "bass", "instrument": "bass.finger", "parts": {
    "verse": { "chordal": "1 . . 1 . 5 8 .", "octave": 2 }, "chorus": { "chordal": "1 1 5 5 8 8 5 5", "octave": 2 } } }]
```

- `sections` 定义段落（小节数、和弦进行），`form` 按名字排顺序（可带 `transpose` / `mute` / `seed`），
  每条轨的 `parts` 写它在各段落里演奏什么（`"verse#2"` 只第 2 次、`"*"` 其余段落），`end` 替换段落最后几拍（过门）。
- `chordal` 按当前和弦取音：`1 3 5 7` 和弦音、`8` 八度、`c` 整个和弦（自动转位）、`B` 低音、`.` 休止、`-` 延长。改和弦只改 `chords`。
- `transform`：`transpose` `octave` `reverse` `invert` `stretch` `shift` `sequence`（模进）`fragment`（裁截）`every`+`then` `vary`（带种子的随机变奏）。
- 写完 `muser analyze` 看建议。完整写法：[skills/arrange.md](skills/arrange.md)。

## 直接放音频：audio 音轨

人声、录音、现成的 loop、音效直接放在音轨上：音轨写 `"instrument": "audio"`，clip 写 `audio` 而不是音符。

```json
{ "name": "vocal", "instrument": "audio", "clips": [{ "start": 8, "length": 16, "audio": { "file": "vocal.wav", "offset": 1.5, "fade_out": 0.5 } }] }
```

- `file` 相对工程文件所在目录；`offset`、淡入淡出单位是秒；clip 的 `start` / `length` 仍是拍。
- 按原速播放，不随 tempo 拉伸、不变调。先 `muser audio <文件> --tempo T` 看时长（拍）再写 `length`。
- 之后效果器、推子、总线、发送、侧链、`mix-report` 照常用。字段与格式：[skills/audio.md](skills/audio.md)。
- **给录音加音效**（收音机、电话、水下、隧道、远处、扩音器、机器人声…）：audio 轨上串效果器即可，配方见 [skills/audio.md](skills/audio.md)；
  只给某几句加就把它们放到单独的 audio 轨。装了 Airwindows 插件时还有失真、lo-fi、变调等几百种。

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
  画像的 `setup` 会自动应用（`"@setup": false` 关闭）。完整音色可以存成状态文件，用 `"@state": "<文件>"` 加载。
- muser 内置这些开源插件的画像（装了就能直接用，英文 skill）：

  | 插件 | 用途 | 要点 |
  |---|---|---|
  | [Surge XT](skills/plugins/surge-xt.md) | 音源 | 混合合成器，bass / pad / pluck / lead 预设 |
  | [Surge XT Effects](skills/plugins/surge-xt-effects.md) | 效果器 | `type` 选 28 种效果 |
  | [Vital](skills/plugins/vital.md) | 音源 | 波表合成器：`@wavetable` 选波表、`@modulations` 接调制；bass / sub / reese / supersaw / pad / pluck / wobble / pwm_lead / keys 预设 |
  | [Airwindows Consolidated](skills/plugins/airwindows-consolidated.md) | 效果器 | 约 470 个效果，`"effect": "<名称>"` 选择（混响、磁带、饱和、调音台、lo-fi…），参数随效果变化 |
  | [Plasma](skills/plugins/plasma.md) | 效果器 | 双级失真：warm / bass_grit / drum_crush / lofi / scream 预设 |

  插件路径可写 `~/`（如 `~/.clap/Airwindows Consolidated.clap`）。

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

## 给人听：muser view

`muser view` 起本地服务、打开播放页（时间线、播放、每条音轨与总线的推子 / 静音 / 独奏，改动由引擎重新渲染）。
由你判断何时给人听：

- **该给人听**：一首曲子完成一版（结构、配器、混音都有了）；混音或编曲有大改动；用户说"让我听听"；
  需要人来判断音量平衡、选 A/B 版本时。
- **不必**：每次小改参数、自己用 `muser audition` / `muser render` 自检时——先确认能渲染、没有明显问题再给人。

```bash
muser view song.muser.json --open      # 后台运行；页面 http://127.0.0.1:8737/
```

- 服务保持运行时，你改工程文件，页面会自动重新渲染刷新。用户在页面点「保存」会直接改工程的 `gain_db` / `muted`——
  **改工程前先重新读文件**，别覆盖用户刚保存的改动。用完结束该进程。
- 打开后告诉用户听什么（如"对比 Drop 1 与 Drop 2 的贝斯"），以及推子改动要点「保存」才写回工程。
- 用户复制给你的改动 JSON 形如 `{"changes": [{"node": "tracks[3]", "gain_db": -9}, {"node": "buses[1]", "muted": true}]}`：
  按 `node` 把对应音轨/总线的字段改成给出的值（MCP：`update_track` / `set_bus`）。
- 没有图形界面的环境（远程/容器）：照常启动，让用户用 SSH 端口转发（`ssh -L 8737:127.0.0.1:8737 …`）在本机浏览器打开。

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
- 平衡各轨音量：`muser mix-report <工程>` 给出每条轨的干声与推子后响度、相对最响轨的差，见 [skills/mixing.md](skills/mixing.md)。

## 效果器索引

效果器写在 `track.effects`（轨级，在该轨增益/声像之前）、总线的 `effects` 或 `master_effects`（主总线，在主增益之前），
按数组顺序串联。每个参数都可省略，`{ "type": "reverb" }` 就是一个可用的混响。
任何效果器都可加 `"bypass": true` 临时关掉（旁通：声音原样通过，参数与位置保留，自动化的 `effects[i]` 不变）；`muser view` 上点效果器标签切换。

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
- **音量**：乐器本体音量差别很大，`gain_db` 不要凭感觉定。写完一版跑 `muser mix-report`，按角色把各轨调到目标相对响度，
  主总线最后放 `limiter`。流程与各角色目标见 [skills/mixing.md](skills/mixing.md)。

## 排错

- 渲染前**务必** `validate`；它会给出出错的具体字段与可选值。
- 鼓组没声音或声音不对 → 用了音阶音高，或用了该套鼓不支持的键（见 drums.md）。
- 音色不对 → 先 `muser instruments <名称>` 看预设当前值，再用 `instrument_params` 小步调整。
- 渲染报"输出数值发散" → 某个乐器参数取了极端值（如很短的 release），往常规范围调。
- 拉奏弦乐/管乐短音听起来很弱 → 音符太短，弦乐短音改用 `*_pizz`。
- 混响尾音被截断 → 渲染时加大 `--tail`（默认 2 秒）。
- 整体发闷/发浑 → 非贝斯轨加 `eq` highpass，减少多条轨上的 reverb。
