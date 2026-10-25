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

## 经验与常见错误

- 音色本身会自然衰减，`length` 太长没有影响；适合琶音和短旋律。
- 颤音琴、管钟余音很长，快速连奏会叠在一起；需要干净时降低速度或减少音符。
