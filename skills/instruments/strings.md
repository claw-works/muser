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

## 经验与常见错误

- 拉奏乐器有起音过程，`length` 太短（< 0.25 拍）听起来会弱；短音请用 `*_pizz` 拨奏版本。
- 多声部弦乐：小提琴(旋律) + 中提琴(内声部) + 大提琴(低音) 分三条轨，比单条 `strings.ensemble` 更有层次。
- 配一点 `reverb`（mix 0.25-0.35、decay 2s 左右）会更像在音乐厅里。
