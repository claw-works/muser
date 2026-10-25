<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 自动化：让参数随时间变化

自动化让乐器参数、效果器参数、增益、声像在演奏中变化：滤波扫频、淡入淡出、声像摆动、
带种子的随机游走。时间单位是**拍**，从工程开头算起，与音符一致；取值用参数自己的单位（Hz、秒、dB…）。

## 写在哪里

- 音轨的 `automation`：目标 `instrument`（乐器参数）、`effects[i]`（该轨第 i 个效果器，从 0 起）、`gain_db`、`pan`。
- 工程的 `automation`：目标 `master_gain_db`、`master_effects[i]`。

```json
{
  "tempo": 128,
  "seed": 7,
  "master_effects": [{ "type": "reverb", "mix": 0.05 }],
  "automation": [ {"target": "master_effects[0]", "param": "mix", "shape": "points", "points": [[0, 0.05], [16, 0.4]], "curve": "smooth"} ],
  "tracks": [{
    "instrument": "bass.acid",
    "automation": [
      { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp" }
    ],
    "clips": [ ... ]
  }]
}
```

`param` 与 `instrument_params` / 效果器参数同名；鼓组写 `鼓件.参数`（如 `kick.tune`）；目标是 `gain_db` / `pan` /
`master_gain_db` 时省略。哪些参数能自动化、变化何时生效，看 `muser instruments <名称>` 里的标注，
或各乐器子 skill 参数表的"自动化"列。

## 三种形状

### points：时间点曲线

```json
{ "shape": "points", "points": [[0, 200], [16, 3000], [32, 800]], "curve": "exp" }
```

- `points`：`[拍, 值]`，时间严格递增。第一个点之前取第一个值，最后一个点之后保持最后的值。
- `curve`：`linear`（默认，直线）、`exp`（按比例变化，频率/时间类参数听感更均匀；要求相邻点同号且非零）、
  `smooth`（两端平缓的 S 形）、`step`（保持前一个值直到下一个点，做"切换"）。
- 值可以写参数允许的名字，如合唱元音 `"a"` `"o"`。

### lfo：周期摆动

```json
{ "shape": "lfo", "wave": "sine", "period_beats": 4, "min": -0.5, "max": 0.5, "phase": 0 }
```

- `wave`：`sine`（默认）/ `triangle` / `saw`（从 min 升到 max 再跳回）/ `square`（min 与 max 间切换）。
- `period_beats`：一个周期多少拍。4 = 每小节一次（4/4），1 = 每拍，0.5 = 每八分音符。按拍计，所以跟着速度走。
- `phase`：起始相位 0-1，`sine` 在 0 处从 min 开始。

### random：带种子的随机

```json
{ "shape": "random", "style": "walk", "step_beats": 0.5, "min": 200, "max": 2500, "max_step": 0.2 }
```

- `style`：`jump`（每步跳到新随机值）/ `glide`（默认，平滑滑向新值）/ `walk`（随机游走：每步在当前值附近移动，
  最多 `max_step` × 范围）。
- `step_beats`：多少拍换一次目标值。
- `seed`（可省略）：省略时由工程的 `seed` 与这条自动化的位置推导。**同一种子每次渲染完全相同；换工程 `seed`
  就换一种随机变化**，适合"多渲染几版挑一个"。

## 生效区间

`start` / `end`（拍，可省略）限定自动化生效的时间；区间外参数回到静态值（`instrument_params` 或效果器里写的值）。
`lfo` / `random` 的相位与随机序列从 `start` 开始计。

## 行为说明

- 乐器与效果器参数每 64 帧（48 kHz 下约 1.3 ms）更新一次；增益与声像逐帧更新，淡入淡出没有台阶。
- "下个音生效"的参数（如弓弦的揉弦、鼓件的调音）只影响自动化之后开始的音符，已经在响的音不变。
- 同一目标多条自动化：效果器/乐器参数按顺序写入，后写的覆盖先写的；`gain_db` / `pan` 取最后一条在生效区间内的值。
- 取值超出参数范围、参数名错误、参数不能自动化（开关、选项、整数），`muser validate` 都会指出具体位置。
- 外置插件参数也能自动化：`param` 写插件参数名，取值可写插件显示的文字（如 `[[0, "200 Hz"], [16, "4000 Hz"]]`）或数字（插件自动化刻度）。
  插件在下一个处理块的第一帧收到新值（块长 ≤ 64 帧）。`sfz:` 乐器的参数自动化尚未支持。

## 示例

- 滤波扫频：8 小节内从闷到亮（exp 曲线让频率变化听起来均匀）

  ```json
  {"instrument": "bass.acid", "automation": [{"target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp"}]}
  ```

- 淡入：前 4 拍从 -60 dB 升到 0 dB

  ```json
  {"instrument": "poly.warm_analog_pad", "automation": [{"target": "gain_db", "shape": "points", "points": [[0, -60], [4, 0]], "curve": "smooth"}]}
  ```

- 声像自动摆动：每 2 小节左右来回一次

  ```json
  {"instrument": "poly.juno_keys", "automation": [{"target": "pan", "shape": "lfo", "wave": "sine", "period_beats": 8, "min": -0.7, "max": 0.7}]}
  ```

- acid 随机游走：每半拍滤波随机小幅移动，每次渲染一样，换 seed 换一种

  ```json
  {"instrument": "bass.acid", "automation": [{"target": "instrument", "param": "cutoff", "shape": "random", "style": "walk", "step_beats": 0.5, "min": 200, "max": 2500, "max_step": 0.25}]}
  ```

- 合唱元音变化：a → o → u

  ```json
  {"instrument": "choir.aah", "automation": [{"target": "instrument", "param": "vowel", "shape": "points", "points": [[0, "a"], [8, "o"], [16, "u"]], "curve": "smooth"}]}
  ```

- 效果器参数：第 9 拍起回声越来越多（只在 8-16 拍生效）

  ```json
  {"instrument": "synth.poly", "effects": [{"type": "delay", "time_seconds": 0.25, "feedback": 0.2, "mix": 0.3}], "automation": [{"target": "effects[0]", "param": "feedback", "shape": "points", "points": [[8, 0.2], [16, 0.75]], "start": 8, "end": 16}]}
  ```

- 鼓件：每小节底鼓随机调音（下一次击打生效）

  ```json
  {"instrument": "drums.808", "automation": [{"target": "instrument", "param": "kick.tune", "shape": "random", "style": "jump", "step_beats": 4, "min": -3, "max": 3}]}
  ```

- 门限式开关：方波 LFO 让 pad 每拍一开一关（trance gate）

  ```json
  {"instrument": "poly.warm_analog_pad", "automation": [{"target": "gain_db", "shape": "lfo", "wave": "square", "period_beats": 0.5, "min": -40, "max": 0}]}
  ```

## 常见用法

- 段落推进：8-16 小节 `points` 把 `cutoff` 从低扫到高，配合主总线 `reverb.mix` 慢慢加大，营造 build-up。
- 让 loop 不重复：给 `cutoff` / `pan` / 鼓件 `tune` 加慢速 `random`（`walk` 或 `glide`），每一遍都略有不同。
- 换种子挑版本：同一工程改 `seed` 渲染几遍，挑最喜欢的一版；记下 seed 就能复现。
- 律动感：`square` LFO 作用在 `gain_db` 上做 trance gate；`sine` LFO 作用在 `pan` 上做自动声像。
