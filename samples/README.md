# muser 示范作品

这些是用 muser 工程 JSON 直接编写、离线渲染出的示范曲，展示引擎当前的能力与不同风格的编曲手法。每首都是 8 拍（约一个 loop），三条音轨（鼓 + 贝斯 + 和声）。

## 作品列表

| 工程文件 | 风格 | 速度 | 鼓组 | 编曲要点 |
|---|---|---|---|---|
| `lofi-dusk` | Lo-fi Hip-hop | 85 | `drums.lofi` | 慵懒摇摆鼓、FM 贝斯、poly 七和弦 pad（Am7→FM7→E…），力度有起伏 |
| `techno-pulse` | Techno | 128 | `drums.techno` | four-on-the-floor 底鼓、off-beat 开镲、十六分闭镲、断奏 FM bassline、poly stab |
| `house-groove` | House | 124 | `drums.house` | 四踩底鼓 + 二四拍手、律动贝斯、poly 和弦（C→Dm 走向） |
| `dnb-roller` | Drum & Bass | 174 | `drums.dnb` | 断奏（breakbeat）鼓型、超低 sub-bass、长音 poly pad 铺底 |

## 如何重新渲染

```bash
# 校验
cargo run -p muser-cli -- validate samples/lofi-dusk.muser.json
# 渲染（产出同名 .wav）
cargo run -p muser-cli -- render samples/lofi-dusk.muser.json -o samples/lofi-dusk.wav
```

或用已编译的二进制：

```bash
target/debug/muser render samples/techno-pulse.muser.json -o samples/techno-pulse.wav
```

## 编曲约定速查

- 时间单位是**拍 (beats)**，`1.0` = 四分音符，`0.5` = 八分，`0.25` = 十六分。
- 鼓组音高用 **GM 鼓键**：36=底鼓, 38=军鼓, 39=拍手, 42=闭镲, 46=开镲。
- 旋律乐器用**音阶音高**：60=中央C，低八度减 12。
- `velocity`（力度，0-1）用来做强弱起伏，让节奏更有人味。
- `gain_db` / `pan` 调音轨平衡与声像；`master_gain_db` 控总输出，留余量防爆音。

完整格式见仓库根目录的 `SKILLS.md` 与 `ARCHITECTURE.md`。
