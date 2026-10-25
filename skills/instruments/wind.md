<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：管乐

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family wind`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `wind.flute` | 60–96 | 4 | 长笛：轻盈透亮 |
| `wind.clarinet` | 50–91 | 4 | 单簧管：圆润木质 |
| `wind.sax` | 49–81 | 4 | 萨克斯：爵士/流行的主奏 |
| `wind.trumpet` | 54–82 | 4 | 小号：明亮有穿透力 |
| `wind.trombone` | 40–72 | 4 | 长号：浑厚的铜管中低音 |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 说明 |
|---|---|---|
| `attack` | 0.002 – 1 s | 气息建立的时间：短 = 吐音清楚；长 = 柔和进入 |
| `release` | 0.005 – 2 s | 松开后的收尾时间 |
| `vibrato_rate` | 0 – 12 Hz | 颤音速度。4-6 常规 |
| `vibrato_depth` | 0 – 100 cent | 颤音深度（音分）。0 直音；10-25 常规 |
| `vibrato_delay` | 0 – 4 s | 音开始后多久才开始颤音 |
| `tone` | 500 – 20000 Hz | 最终低通：明暗 |
| `breath_noise` | 0 – 1 | 气声：越大越多气流噪声 |
| `brassiness` | 0 – 1 | 铜管的'炸'（小号/长号在大声时变亮） |
| `portamento` | `true` / `false` | 连音（重叠的音符）之间滑音，像长号的滑管 |

各乐器的预设取值：

| instrument | `attack` | `release` | `vibrato_rate` | `vibrato_depth` | `vibrato_delay` | `tone` | `breath_noise` | `brassiness` | `portamento` |
|---|---|---|---|---|---|---|---|---|---|
| `wind.flute` | 0.05 | 0.08 | 5 | 8 | 0.25 | 8000 | 0.35 | 0 | false |
| `wind.clarinet` | 0.03 | 0.06 | 5 | 0 | 0.4 | 8000 | 0.15 | 0 | false |
| `wind.sax` | 0.03 | 0.06 | 5.2 | 15 | 0.3 | 8000 | 0.2 | 0 | false |
| `wind.trumpet` | 0.025 | 0.06 | 5.5 | 6 | 0.4 | 8000 | 0.08 | 0.6 | false |
| `wind.trombone` | 0.04 | 0.06 | 5.5 | 4 | 0.4 | 6000 | 0.08 | 0.45 | false |

### 调音示例

- 气声多、颤音少的长笛

  ```json
  { "instrument": "wind.flute", "instrument_params": {"breath_noise": 0.6, "vibrato_depth": 5} }
  ```

- 滑音长号

  ```json
  { "instrument": "wind.trombone", "instrument_params": {"portamento": true} }
  ```


## 经验与常见错误

- 管乐是单旋律乐器，同一时刻尽量只放一个音；复音数只有 4。
- 音域外的音能发声，但会失真或不像该乐器，参考下表的推荐音域。
