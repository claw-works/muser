<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：吉他

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family guitar`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `guitar.nylon` | 40–84 | 8 | 尼龙弦古典吉他：温暖柔和，适合分解和弦、拉丁、民谣 |
| `guitar.steel` | 40–84 | 8 | 钢弦民谣吉他：明亮清脆，适合扫弦与分解和弦 |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 自动化 | 说明 |
|---|---|---|---|
| `decay` | 0.05 – 60 s | 下个音 | 按住时弦振动持续多久（中央 C 衰减 60dB 的时间）。短 = 闷音；长 = 余音绵长 |
| `release` | 0.01 – 10 s | 下个音 | 松开（止音）后的衰减时间 |
| `tone` | 200 – 20000 Hz | ✓ | 最终低通：琴体/拾音的明暗。2000 暗；5000 常规；10000+ 很亮 |
| `pluck_brightness` | 100 – 20000 Hz | 下个音 | 拨弦的亮度：指甲/拨片亮（5000+），指腹暗（1000-2000） |
| `pluck_position` | 0.02 – 0.5 | 下个音 | 拨弦位置（离琴桥的比例）。0.05 靠近琴桥，薄而亮；0.5 在弦中间，圆而空 |
| `buzz` | 0 – 1 | 下个音 | 弦打品丝的嗡声。0 干净；0.3+ 有击弦感（slap） |

自动化列：✓ = 正在发声的音也立即跟随；下个音 = 只影响之后开始的音；— = 不能自动化。见 [automation.md](../automation.md)。

各乐器的预设取值：

| instrument | `decay` | `release` | `tone` | `pluck_brightness` | `pluck_position` | `buzz` |
|---|---|---|---|---|---|---|
| `guitar.nylon` | 3 | 0.18 | 6500 | 2800 | 0.16 | 0 |
| `guitar.steel` | 4.5 | 0.25 | 11000 | 6500 | 0.12 | 0 |

### 调音示例

- 更暗、更闷的指弹（靠近琴颈拨弦）

  ```json
  { "instrument": "guitar.nylon", "instrument_params": {"tone": 2500, "pluck_brightness": 1500, "pluck_position": 0.3} }
  ```

- 闷音（palm mute 感）

  ```json
  { "instrument": "guitar.steel", "instrument_params": {"decay": 0.3, "release": 0.05} }
  ```


## 经验与常见错误

- 物理建模拨弦：音符一开始就是拨弦，`length` 决定何时止音。分解和弦时让音符重叠更自然。
- 扫弦：同一和弦的音符起点依次错开 0.01-0.03 拍，从低音到高音。
