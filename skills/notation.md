# 简写音符：ABC 旋律、grid 节奏、loop 库、鼓手

不要一个个写 `notes` 数组，也不要写脚本去生成音符：clip 上用下面的简写，渲染时展开（与 `notes` 合在一起，
时间都相对 clip 起点）。工程文件因此很短，人也读得懂，以后直接改这几行字。`clip.length` 照常要写，
grid、loop、drummer 会在这个长度里循环。

## ABC：旋律与和声（`"abc"`）

```json
{ "start": 0, "length": 8, "abc": "L:1/8\nK:Am\nA2 cB e2 dc | B4 z4 |" }
```

- `C` = 中央 C（60），`c` 高八度，`'` 再升、`,` 降八度；`^` 升、`_` 降、`=` 还原；`K:` 调号自动生效（`Am` `Dmix` `Edor` `F#m` `Bb` …），
  临时记号在本小节内持续。
- `L:` 单位时值（默认 1/8）：`A2` 两倍、`A/2` 一半、`A3/2`；休止 `z`，整小节休止 `Z`（`Z4`）。
- 和弦 `[CEG]2`，连音线 `A2-A`，附点 `A>B`，三连音 `(3ABc`，反复 `|: … :|`，一二房 `[1 … :|[2 …`。
- 力度 `!p!` `!mf!` `!f!`（之后都用这个力度），重音 `!>!`，断奏 `.A`。和弦标记 `"Am"`、装饰音 `{g}` 忽略。
- 一个 clip 一个声部；和声的另一个声部（贝斯、和弦）写进别的音轨的 clip。
- 写错会报出位置，如 `tracks[0].clips[0].abc: 第 2 行第 5 列：和弦 [ 没有闭合`。

## grid：步进节奏（`"grid"`）

```json
{ "start": 0, "length": 16, "grid": { "kick": "x...x...x...x...", "snare": "....x.......x...", "hat": "x.x.x.x.x.x.x.x." } }
```

- 一个字符一步，`step` 拍一步（默认 0.25 = 十六分）。每行按自己的长度在 clip 里循环。
- 行名：鼓件（`kick` `snare` `clap` `rim` `hat` `open_hat` `pedal_hat` `crash` `ride` `tom_high` `tom_mid` `tom_low` `floor_tom`
  `shaker` `cowbell` …）、MIDI 数字（`"36"`）或音名（`"c2"` `"eb3"`，C4 = 60，用于贝斯线等有音高的乐器）。
- `x` 敲（0.8）、`X` 重音（1.0）、`o` 轻 / 鬼音（0.45）、`1`–`9` 力度 0.1–0.9；`=` 或 `~` 延长上一个音一步；`.` `-` 休止；空格、`|` 忽略。

## loop 库：拿来就用的节奏型

```bash
muser loop list                 # 全部（内置 + 本机 loop 库），--style dnb 筛
muser loop play dnb_two_step    # 用声卡试听（--kit drums.909 换鼓组，--tempo 170）
muser loop show dnb_two_step --start 16 --repeat 8   # 输出 clip JSON，直接放进鼓轨的 clips
muser loop save song.muser.json --track 0 --clip 2 --name my_break --style dnb   # 把工程里的 clip 存成 loop
```

- `show` 输出的 clip 内嵌 `grid` 和这个节奏型的律动 `humanize`，并标注 `"from": "loop:<名字>"`。工程因此自足，
  内置 loop 以后改了也不影响旧工程。鼓件按名字写，换任意 `drums.*` 鼓组都能播（`kit` 只是建议）。
- 用户存的 loop 在 `~/.config/muser/loops/`；和内置同名时用户的优先。

## drummer：会变化、会打过门的鼓手（`"drummer"`）

```json
{ "start": 0, "length": 32, "drummer": { "loop": "rock_basic", "complexity": 0.6, "intensity": 0.6 } }
```

| 字段 | 含义 |
|---|---|
| `loop` | 底子（`muser loop list` 里的名字） |
| `complexity` | 0–1：0 照着 loop 打；越高军鼓鬼音、底鼓切分、踩镲加花越多，过门越长 |
| `intensity` | 0–1：打多重（0.5 不变） |
| `fill_every` | 缺省自动：clip 最后一小节一定打，中间的 4 小节乐句末尾随机打（约三分之一），长 1 或 2 拍不等——不会机械地每 4 小节一次；写 N 则每 N 小节一个，0 不打 |
| `crash` | 过门后下一小节第一拍加镲，默认 true |
| `seed` | 不写时由工程 seed 推导；换 seed 换一种打法 |

- 每小节都不完全一样；没写 `humanize` 时用底子 loop 的律动。
- 需要逐个音精确控制时用 `grid`；要"像个鼓手一样打一段"时用 `drummer`。

## 跟着和弦走：chordal

旋律用 `abc`；贝斯、和弦、琶音这些"跟着和弦走"的声部，在编曲层里用 `chordal`（`"1 . 5 . 8 . 5 3"`、`"c - - -"`），
和弦写在段落的 `chords` 上，改和弦时一起变。见 [arrange.md](arrange.md)。

## 混用

同一个 clip 里 `notes`、`abc`、`grid`、`drummer` 可以同时写，音符合在一起：比如 `drummer` 打底，`grid` 再加一行 `shaker`。
`muser view` 的时间线显示展开后的音符。

## 反过来：导出 ABC / MIDI

- `muser abc <工程>`：整首写成标准多声部 ABC（每条乐器音轨一个声部），给能读 ABC 的模型做分析、续写、改编。
- `muser abc <工程> --yue2 --vocal <旋律轨> --sections verse:8,chorus:8`：YuE2 的原生谱面（Vocal / Ins + 和弦记号），
  直接作为它的 `--abc-file`（`--cot full`；加 `--no-chords` 给 `--cot melody`）。
- `muser midi <工程>`：标准 MIDI 文件，交给 DAW。
- 导出写谱面上的音（humanize 之前），鼓组与 audio 音轨不导出；和弦记号是估计的，交出去之前看一眼。详见 [TOOLS.md](../TOOLS.md)。
