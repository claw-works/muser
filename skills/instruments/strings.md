<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：弦乐

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family strings`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `strings.violin` | 55–100 | 8 | 小提琴（拉奏）：旋律主奏 |
| `strings.viola` | 48–88 | 8 | 中提琴（拉奏）：比小提琴更暗更厚 |
| `strings.cello` | 36–76 | 8 | 大提琴（拉奏）：温暖深沉 |
| `strings.contrabass` | 28–60 | 8 | 低音提琴（拉奏）：弦乐低音声部 |
| `strings.ensemble` | 36–96 | 8 | 弦乐组：多把琴的合奏，适合和弦铺底与抒情段落 |
| `strings.violin_pizz` | 55–100 | 8 | 小提琴拨奏：短促跳跃 |
| `strings.cello_pizz` | 36–76 | 8 | 大提琴拨奏 |
| `strings.contrabass_pizz` | 28–60 | 8 | 低音提琴拨奏：爵士 walking bass |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 自动化 | 说明 |
|---|---|---|---|
| `attack` | 0.005 – 2 s | 下个音 | 运弓起音时间。0.02 有力；0.3+ 柔和渐入 |
| `release` | 0.15 – 10 s | 下个音 | 松开后的衰减时间。下限 0.15：更短时上游弓弦模型会数值发散 |
| `vibrato_rate` | 0 – 12 Hz | 下个音 | 揉弦速度。5-6 常规 |
| `vibrato_depth` | 0 – 100 cent | 下个音 | 揉弦深度（上下各多少音分）。0 不揉；10-20 常规；40+ 夸张 |
| `vibrato_delay` | 0 – 4 s | 下个音 | 音开始后多久才开始揉弦 |
| `tone` | 500 – 20000 Hz | ✓ | 最终低通：明暗 |
| `pressure` | 0 – 1 | ✓ | 弓压：小 = 轻柔带气声；大 = 浓密带颗粒感 |
| `players` | 1 – 4（整数） | — | 每个音由几把琴演奏：1 独奏；3-4 组合 |

自动化列：✓ = 正在发声的音也立即跟随；下个音 = 只影响之后开始的音；— = 不能自动化。见 [automation.md](../automation.md)。

各乐器的预设取值：

| instrument | `attack` | `release` | `vibrato_rate` | `vibrato_depth` | `vibrato_delay` | `tone` | `pressure` | `players` |
|---|---|---|---|---|---|---|---|---|
| `strings.violin` | 0.06 | 0.25 | 5.8 | 18 | 0.25 | 7500 | 0.5 | 1 |
| `strings.viola` | 0.06 | 0.25 | 5.5 | 18 | 0.25 | 6500 | 0.5 | 1 |
| `strings.cello` | 0.08 | 0.3 | 5.2 | 20 | 0.3 | 5500 | 0.5 | 1 |
| `strings.contrabass` | 0.1 | 0.35 | 4.8 | 16 | 0.35 | 4000 | 0.5 | 1 |
| `strings.ensemble` | 0.14 | 0.25 | 5.5 | 14 | 0.2 | 6000 | 0.4 | 4 |
| `strings.violin_pizz` | 0.06 | 0.15 | 5.8 | 18 | 0.25 | 7500 | 0.5 | 1 |
| `strings.cello_pizz` | 0.08 | 0.2 | 5.2 | 20 | 0.3 | 5500 | 0.5 | 1 |
| `strings.contrabass_pizz` | 0.1 | 0.2 | 4.8 | 16 | 0.35 | 4000 | 0.5 | 1 |

### 调音示例

- 慢起音、深揉弦的抒情大提琴

  ```json
  { "instrument": "strings.cello", "instrument_params": {"attack": 0.4, "vibrato_depth": 25, "vibrato_delay": 0.3, "release": 1.0} }
  ```

- 小编制弦乐组（2 人）

  ```json
  { "instrument": "strings.ensemble", "instrument_params": {"players": 2} }
  ```


## 经验与常见错误

- 拉奏乐器有起音过程，`length` 太短（< 0.25 拍）听起来会弱；短音请用 `*_pizz` 拨奏版本。
- 多声部弦乐：小提琴(旋律) + 中提琴(内声部) + 大提琴(低音) 分三条轨，比单条 `strings.ensemble` 更有层次。
- 配一点 `reverb`（mix 0.25-0.35、decay 2s 左右）会更像在音乐厅里。
