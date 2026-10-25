<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：打击旋律乐器

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family mallet`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `mallet.marimba` | 45–96 | 12 | 马林巴：温暖木质，适合旋律与琶音 |
| `mallet.xylophone` | 65–108 | 12 | 木琴：明亮短促 |
| `mallet.vibraphone` | 53–89 | 12 | 颤音琴：金属、长余音，爵士与 lo-fi 常用 |
| `mallet.glockenspiel` | 79–108 | 12 | 钟琴：高而清亮的铃声 |
| `mallet.celesta` | 60–108 | 12 | 钢片琴：柔和的铃音，梦幻感 |
| `mallet.kalimba` | 60–88 | 12 | 卡林巴（拇指琴）：清脆温柔 |
| `mallet.steel_pan` | 60–84 | 12 | 钢鼓：加勒比风情的金属音色 |
| `mallet.tubular_bells` | 60–77 | 8 | 管钟：庄严的长余音钟声 |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 说明 |
|---|---|---|
| `decay` | 0.05 – 60 s | 敲击后自然衰减的时间（中央 C） |
| `release` | 0.01 – 60 s | 松开后的止音时间（颤音琴/钢片琴/管钟有制音器） |
| `softness` | 1 – 8 | 槌头软硬：1 硬而亮；4-8 软而圆 |
| `strike_position` | 0 – 1 | 敲击位置。0.5 在中心（最饱满）；靠边泛音更多 |
| `motor_rate` | 0 – 16 Hz | 颤音琴电机转速（颤音速度），0 = 关闭 |
| `motor_depth` | 0 – 1 | 颤音琴电机颤音深度 |

各乐器的预设取值：

| instrument | `decay` | `release` | `softness` | `strike_position` | `motor_rate` | `motor_depth` |
|---|---|---|---|---|---|---|
| `mallet.marimba` | 1.6 | 60 | 2.6 | 0.42 | 0 | 0 |
| `mallet.xylophone` | 1.1 | 60 | 2 | 0.4 | 0 | 0 |
| `mallet.vibraphone` | 7 | 0.12 | 3 | 0.4 | 5.5 | 0.75 |
| `mallet.glockenspiel` | 9 | 60 | 2 | 0.3 | 0 | 0 |
| `mallet.celesta` | 3.5 | 0.25 | 2.4 | 0.3 | 0 | 0 |
| `mallet.kalimba` | 2.6 | 60 | 2.2 | 0.9 | 0 | 0 |
| `mallet.steel_pan` | 2.2 | 60 | 2 | 0.5 | 0 | 0 |
| `mallet.tubular_bells` | 9 | 0.5 | 2.2 | 0 | 0 | 0 |

### 调音示例

- 软槌、长余音的马林巴

  ```json
  { "instrument": "mallet.marimba", "instrument_params": {"softness": 5, "decay": 3.0} }
  ```

- 颤音琴打开电机颤音

  ```json
  { "instrument": "mallet.vibraphone", "instrument_params": {"motor_rate": 5, "motor_depth": 0.6} }
  ```


## 经验与常见错误

- 音色本身会自然衰减，`length` 太长没有影响；适合琶音和短旋律。
- 颤音琴、管钟余音很长，快速连奏会叠在一起；需要干净时降低速度或减少音符。
