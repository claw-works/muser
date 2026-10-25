<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：风琴

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family organ`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `organ.jazz` | 36–96 | 16 | 拉杆风琴（Hammond 风格）爵士音色：柔和带打击泛音 |
| `organ.gospel` | 36–96 | 16 | 福音风琴：饱满明亮，适合和弦铺底 |
| `organ.full` | 36–96 | 16 | 全拉杆风琴：最饱满响亮，摇滚风琴 |
| `organ.ballad` | 36–96 | 16 | 抒情风琴：柔和圆润，适合慢歌铺底 |

## 可调参数（instrument_params）

在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。

| 参数 | 范围 | 自动化 | 说明 |
|---|---|---|---|
| `drawbars` | 9 位数字串，每位 0-8 | — | 九根拉杆（16' 5⅓' 8' 4' 2⅔' 2' 1⅗' 1⅓' 1'），每位 0-8。"888000000" 经典爵士；"838000000" 柔和；"888888888" 全开 |
| `percussion` | `"off"` / `"second"` / `"third"` | — | 打击泛音：按键瞬间加一声短促的二次/三次谐波，爵士风琴的'叮' |
| `click` | 0 – 1 | ✓ | 按键咔哒声。0 干净；0.5 复古 |

自动化列：✓ = 正在发声的音也立即跟随；下个音 = 只影响之后开始的音；— = 不能自动化。见 [automation.md](../automation.md)。

各乐器的预设取值：

| instrument | `drawbars` | `percussion` | `click` |
|---|---|---|---|
| `organ.jazz` | "888000000" | "third" | 0.5 |
| `organ.gospel` | "888800000" | "second" | 0.6 |
| `organ.full` | "888888888" | "off" | 0.7 |
| `organ.ballad` | "838000000" | "off" | 0.15 |

### 调音示例

- 柔和的抒情拉杆 + 打击泛音

  ```json
  { "instrument": "organ.jazz", "instrument_params": {"drawbars": "838000000", "percussion": "second", "click": 0.2} }
  ```


## 经验与常见错误

- 拉杆风琴适合长音铺底：和弦音符 `length` 放长（2-4 拍）。
- 配合 `chorus` 或较慢的 `phaser` 可以模拟旋转扬声器的晃动感。
