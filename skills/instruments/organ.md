<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：风琴

音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family organ`。

| instrument | 推荐音域 | 复音数 | 说明 |
|---|---|---|---|
| `organ.jazz` | 36–96 | 16 | 拉杆风琴（Hammond 风格）爵士音色：柔和带打击泛音 |
| `organ.gospel` | 36–96 | 16 | 福音风琴：饱满明亮，适合和弦铺底 |
| `organ.full` | 36–96 | 16 | 全拉杆风琴：最饱满响亮，摇滚风琴 |
| `organ.ballad` | 36–96 | 16 | 抒情风琴：柔和圆润，适合慢歌铺底 |

## 经验与常见错误

- 拉杆风琴适合长音铺底：和弦音符 `length` 放长（2-4 拍）。
- 配合 `chorus` 或较慢的 `phaser` 可以模拟旋转扬声器的晃动感。
