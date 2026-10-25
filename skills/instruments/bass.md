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

## 经验与常见错误

- `bass.acid*` 是单音（复音数 1）：前一个音还没结束就开始下一个音，会产生 303 式滑音；想要分开的音就让音符之间留空隙。
- 贝斯一般放在 28-55；低于 28 很多音箱放不出来。
- 贝斯和底鼓同时出现在重拍上时，可以给贝斯加 compressor 让音量更稳定。
