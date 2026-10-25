<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 路由：总线、发送、侧链

默认每条音轨直接进主总线（master）。需要"几条轨一起处理"或"多条轨共用一个效果"时用路由：

- **总线（bus）**：工程的 `buses` 列表。总线把送进来的信号相加，再过自己的 `effects`、`gain_db`、`pan`，然后去它的 `output`。
- **output**：音轨或总线的去向，写 `"master"`（默认，可省略）或某条总线的名字。整份信号都去那里。
- **sends**：额外送一份到别的总线，`{"总线名": dB}`。推子后发送：跟随本轨的 gain_db/pan 与其自动化。
  0 dB 等量，-6 dB 约一半，常用 -18 ~ -3。sends 不能指向 master。
- **sidechain**：compressor 的 `sidechain` 写音轨或总线名，由那个信号（推子后）触发压缩，被压的是本轨/本总线。
  作为触发信号的音轨即使 `muted` 也照样触发（静音只是不输出）。

```json
{
  "buses": [
    { "name": "reverb", "effects": [{ "type": "reverb", "mix": 1.0 }] },
    { "name": "drums", "effects": [{ "type": "compressor" }], "output": "master" }
  ],
  "tracks": [
    { "name": "kick", "instrument": "drums.909", "output": "drums" },
    { "name": "pad", "instrument": "poly.warm_analog_pad", "sends": { "reverb": -6 } }
  ]
}
```

## 规则

- 总线名唯一、非空，不能叫 `master`、不能含 `.`。音轨与总线不要同名；被引用的音轨名必须唯一。
- 不能成环：A → B → A、以及"被自己压缩后的信号触发自己"都会报错，错误信息指出位置。
- 总线字段：`name`、`effects`、`gain_db`（-80..24）、`pan`（-1..1，平衡：居中不衰减）、`muted`、`output`、`sends`、`automation`。
- 总线自动化目标：`effects[i]`、`gain_db`、`pan`、`sends.<总线>`；音轨也可自动化 `sends.<总线>`（见 [automation.md](automation.md)）。
- 有延迟的效果器（limiter 的前瞻、部分插件）会被自动补偿，并联的各路信号保持对齐。
- `muser routing <工程>`（MCP：`get_routing`）打印处理顺序、去向、发送量与侧链，改完路由先看一眼。

## 混音经验

- 发送到效果总线时，总线里的 reverb/delay 用 `mix: 1`（只出湿声），湿干比用发送量调。
- 混响总线前放 `eq` highpass 300 Hz 左右，低频不进混响，混音不浑。
- 同一个空间：所有需要混响的轨送进同一条混响总线，比每轨一个混响更统一、更省算力。
- 鼓组总线压缩 attack 10-30 ms，保留瞬态；需要更'贴'再在总线末尾加 limiter。
- 侧链泵感：pad、贝斯、混响总线都由底鼓触发；release 约一拍的 1/3 到 1/2。
- 贝斯和底鼓打架时，贝斯挂一个由 kick 触发的轻压缩（ratio 2-3、threshold 较高）就能让出底鼓的瞬间。
- dub/过渡：平时发送很低，用 `sends.<总线>` 的 step 曲线在乐句末尾"甩"一下。

## 示例

### 共用混响：几条音轨各送一份到同一个混响总线

各轨用 `sends` 按 dB 决定湿声多少；总线里的 reverb 用 `mix: 1`（全湿），干声仍走各轨自己的 output。比每轨各挂一个混响更省、空间感更统一。

```json
{
  "tempo": 84,
  "buses": [
    {"name": "hall", "effects": [{"type": "eq", "shape": "highpass", "frequency": 300}, {"type": "reverb", "mix": 1.0, "decay_seconds": 3.5, "size": 0.85}]}
  ],
  "tracks": [
    {"name": "piano", "instrument": "poly.fm_electric_piano", "sends": {"hall": -10}, "clips": []},
    {"name": "strings", "instrument": "strings.ensemble", "sends": {"hall": -6}, "clips": []},
    {"name": "choir", "instrument": "choir.ooh", "sends": {"hall": -3}, "clips": []}
  ]
}
```

`muser routing` 输出：

```
处理顺序与去向：
  piano（poly.fm_electric_piano） ⇒ master，[hall] 发送 -10.0 dB
  strings（strings.ensemble） ⇒ master，[hall] 发送 -6.0 dB
  choir（choir.ooh） ⇒ master，[hall] 发送 -3.0 dB
  [hall] → eq → reverb ⇒ master
  master → 主增益 +0.0 dB
```

### 鼓组总线：所有鼓件 output 到一条总线，一起压缩

总线压缩让鼓件'粘'在一起；军鼓再送一点到短房间混响。

```json
{
  "tempo": 96,
  "buses": [
    {"name": "drums", "effects": [{"type": "compressor", "threshold_db": -24, "ratio": 4, "attack_ms": 15, "release_ms": 120, "makeup_db": 5}, {"type": "limiter", "ceiling_db": -1}]},
    {"name": "room", "effects": [{"type": "reverb", "mix": 1.0, "decay_seconds": 0.8, "size": 0.35}]}
  ],
  "tracks": [
    {"name": "kick", "instrument": "drums.acoustic", "output": "drums", "clips": []},
    {"name": "snare", "instrument": "drums.acoustic", "output": "drums", "sends": {"room": -10}, "clips": []},
    {"name": "hats", "instrument": "drums.acoustic", "output": "drums", "gain_db": -8, "clips": []}
  ]
}
```

`muser routing` 输出：

```
处理顺序与去向：
  kick（drums.acoustic） ⇒ [drums]
  snare（drums.acoustic） ⇒ [drums]，[room] 发送 -10.0 dB
  hats（drums.acoustic） ⇒ [drums]
  [drums] → compressor → limiter ⇒ master
  [room] → reverb ⇒ master
  master → 主增益 +0.0 dB
```

### 侧链泵感：底鼓压 pad 和贝斯

被压的轨挂 compressor 并写 `sidechain: "kick"`；混响总线也压，尾音一起呼吸。

```json
{
  "tempo": 124,
  "buses": [
    {"name": "verb", "effects": [{"type": "reverb", "mix": 1.0, "decay_seconds": 2.5}, {"type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick"}]}
  ],
  "tracks": [
    {"name": "kick", "instrument": "drums.909", "clips": [{"start": 0, "length": 4, "notes": [{"pitch": 36, "start": 0, "length": 0.2}, {"pitch": 36, "start": 1, "length": 0.2}, {"pitch": 36, "start": 2, "length": 0.2}, {"pitch": 36, "start": 3, "length": 0.2}]}]},
    {"name": "pad", "instrument": "poly.warm_analog_pad", "sends": {"verb": -8}, "effects": [{"type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick"}], "clips": []},
    {"name": "bass", "instrument": "poly.moog_bass", "effects": [{"type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick"}], "clips": []}
  ]
}
```

`muser routing` 输出：

```
处理顺序与去向：
  kick（drums.909） ⇒ master
  pad（poly.warm_analog_pad） → compressor⟵kick ⇒ master，[verb] 发送 -8.0 dB
  bass（poly.moog_bass） → compressor⟵kick ⇒ master
  [verb] → reverb → compressor⟵kick ⇒ master
  master → 主增益 +0.0 dB
侧链：
  kick 触发 pad 的 effects[0]
  kick 触发 bass 的 effects[0]
  kick 触发 [verb] 的 effects[1]
```

### dub 甩音：发送量自动化 + 总线套总线

军鼓平时几乎不送（-40 dB），在第 4 拍末把发送推到 -2 dB 一下，回声接着飘；echo 总线再送进 space 混响总线，并让回声左右摆动。

```json
{
  "tempo": 75,
  "buses": [
    {"name": "echo", "effects": [{"type": "delay", "time_seconds": 0.6, "feedback": 0.6, "mix": 1.0}], "sends": {"space": -6}, "automation": [{"target": "pan", "shape": "lfo", "period_beats": 8, "min": -0.7, "max": 0.7}]},
    {"name": "space", "effects": [{"type": "reverb", "mix": 1.0, "decay_seconds": 4.0}]}
  ],
  "tracks": [
    {"name": "snare", "instrument": "drums.lofi", "sends": {"echo": -40}, "automation": [{"target": "sends.echo", "shape": "points", "curve": "step", "points": [[0, -40], [6.9, -40], [7.0, -2], [8.0, -2], [8.1, -40]]}], "clips": [{"start": 0, "length": 8, "notes": [{"pitch": 38, "start": 1, "length": 0.2}, {"pitch": 38, "start": 3, "length": 0.2}, {"pitch": 38, "start": 5, "length": 0.2}, {"pitch": 38, "start": 7, "length": 0.2}]}]}
  ]
}
```

`muser routing` 输出：

```
处理顺序与去向：
  snare（drums.lofi） ⇒ master，[echo] 发送 -40.0 dB
  [echo] → delay ⇒ master，[space] 发送 -6.0 dB
  [space] → reverb ⇒ master
  master → 主增益 +0.0 dB
```

完整可听的示范在 `samples/routing/`（共用混响、鼓组总线、侧链泵感、dub 甩音，均有 A/B 对比）。
