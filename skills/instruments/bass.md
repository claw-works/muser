<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：贝斯

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family bass`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `bass.finger` | 28–60 | 4 | 指弹电贝斯：圆润饱满，pop / funk / R&B 的标准贝斯 |
| `bass.slap` | 28–60 | 4 | 击勾弦贝斯：有冲击力的打击感，funk / disco |
| `bass.acid` | 24–60 | 1 | TB-303 风格 acid 贝斯（单音，重叠音符会滑音），acid house / techno |
| `bass.acid_squelch` | 24–60 | 1 | acid 贝斯，更高谐振与包络，'吱吱'声更强烈 |
| `bass.acid_square` | 24–60 | 1 | acid 贝斯，方波版本，更空心 |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 说明 |
|---|---|---|
| `decay` | 0.05 – 60 s | 按住时弦振动持续多久（中央 C 衰减 60dB 的时间）。短 = 闷音；长 = 余音绵长 |
| `release` | 0.01 – 10 s | 松开（止音）后的衰减时间 |
| `tone` | 200 – 20000 Hz | 最终低通：琴体/拾音的明暗。2000 暗；5000 常规；10000+ 很亮 |
| `pluck_brightness` | 100 – 20000 Hz | 拨弦的亮度：指甲/拨片亮（5000+），指腹暗（1000-2000） |
| `pluck_position` | 0.02 – 0.5 | 拨弦位置（离琴桥的比例）。0.05 靠近琴桥，薄而亮；0.5 在弦中间，圆而空 |
| `buzz` | 0 – 1 | 弦打品丝的嗡声。0 干净；0.3+ 有击弦感（slap） |

各乐器的预设取值：

| instrument | `decay` | `release` | `tone` | `pluck_brightness` | `pluck_position` | `buzz` |
|---|---|---|---|---|---|---|
| `bass.finger` | 6 | 0.12 | 2600 | 1600 | 0.2 | 0 |
| `bass.slap` | 5 | 0.1 | 6000 | 9000 | 0.28 | 0.45 |

### 调音示例

- 更闷更短的指弹贝斯

  ```json
  { "instrument": "bass.finger", "instrument_params": {"tone": 1200, "decay": 1.0} }
  ```

- 尖叫 acid（配合 accent 音符）

  ```json
  { "instrument": "bass.acid", "instrument_params": {"cutoff": 250, "resonance": 0.9, "env_mod": 0.8, "decay": 0.3, "accent": 1.0} }
  ```


## 经验与常见错误

- `bass.acid*` 是单音（复音数 1）：前一个音还没结束就开始下一个音，会产生 303 式滑音；想要分开的音就让音符之间留空隙。
- 贝斯一般放在 28-55；低于 28 很多音箱放不出来。
- 贝斯和底鼓同时出现在重拍上时，可以给贝斯加 compressor 让音量更稳定。
