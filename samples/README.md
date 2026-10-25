# muser 示范作品

这些是用 muser 工程 JSON 直接编写、离线渲染出的示范曲，展示引擎当前的能力与不同风格的编曲手法。每首都是 8 拍（约一个 loop），三条音轨（鼓 + 贝斯 + 和声）。

## 作品列表

| 工程文件 | 风格 | 速度 | 鼓组 | 编曲要点 |
|---|---|---|---|---|
| `lofi-dusk` | Lo-fi Hip-hop | 85 | `drums.lofi` | 慵懒摇摆鼓、FM 贝斯、poly 七和弦 pad（Am7→FM7→E…），力度有起伏 |
| `techno-pulse` | Techno | 128 | `drums.techno` | four-on-the-floor 底鼓、off-beat 开镲、十六分闭镲、断奏 FM bassline、poly stab |
| `house-groove` | House | 124 | `drums.house` | 四踩底鼓 + 二四拍手、律动贝斯、poly 和弦（C→Dm 走向） |
| `dnb-roller` | Drum & Bass | 174 | `drums.dnb` | 断奏（breakbeat）鼓型、超低 sub-bass、长音 poly pad 铺底 |

## 效果器 A/B 对比组

同一段内容（lo-fi 鼓 + poly 和弦），**只有效果器不同**，用来直接听出各效果器的作用：

| 工程文件 | 效果 | 听感 |
|---|---|---|
| `fx-demo-dry` | 无（对照组） | 和弦干净短促，收得利落 |
| `fx-demo-reverb` | keys: `reverb` (mix 0.45, decay 2.8s, size 0.8) | 和弦带明显空间感与尾音，更"湿"更远 |
| `fx-demo-delay` | keys: `delay` (0.45s, feedback 0.45, mix 0.4) | 和弦后跟有节奏的回声，逐渐衰减 |
| `fx-demo-chorus` | keys: `chorus` (rate 0.9Hz, depth 4ms, mix 0.5) | 和弦变厚、变宽，有轻微起伏 |
| `fx-demo-eq` | keys: `eq` 高架+6dB@4k + 低切200Hz | 和弦更亮更清爽，低频被切 |
| `fx-demo-limiter` | master: +6dB 增益 + `limiter` (ceiling -0.3dB) | 整体更响，峰值被收在 0dB 内不爆 |
| `fx-demo-phaser` | keys: `phaser` (rate 0.5Hz, feedback 0.4, mix 0.6) | 和弦上有流动的"喷气式"扫频感 |
| `fx-demo-flanger` | keys: `flanger` (rate 0.2Hz, feedback 0.7, mix 0.6) | 和弦带金属感的扫动、梳状着色 |

依次渲染这些工程并对比试听，即可体验 muser 的内置效果器链
（reverb / delay / chorus / eq / limiter / phaser / flanger）。

## 外置 VST 效果器示例

`plugin-fx-example.muser.json` 演示用 `plugin` 效果器加载**本机的第三方 VST3/CLAP**
（示例里用的是 `Surge XT Effects.vst3`）。它的 `path` 是本机绝对路径，**换台机器需改成
你自己的插件路径**。

需要用 `host` 特性构建才能加载：

```bash
muser render samples/plugin-fx-example.muser.json -o out.wav   # 需用 --features host 构建的 muser
```

未启用 `host` 特性时渲染会报 `HostDisabled`；路径不存在会报 `PluginLoad`。

## 外置 VSTi（虚拟乐器）作为音源

`vsti-example.muser.json` 演示把第三方 **VSTi/CLAP 乐器**当音源：音轨的
`instrument` 用 `plugin:路径` 前缀（示例用 `Surge XT.vst3` 合成器）。音符通过
事件喂给插件，由插件合成发声。

```bash
muser render samples/vsti-example.muser.json -o out.wav   # 需用 --features host 构建的 muser
```

- 语法：`"instrument": "plugin:/path/to/Instrument.vst3"`，可带 `#插件id`
  指定文件内的具体插件：`"plugin:/path.vst3#id"`。
- 同样需要 `host` 特性；`path` 是本机绝对路径，换机需改。
- 采样器类插件（如 sfizz）需自带/加载音色文件才会发声。

## 如何重新渲染

```bash
# 校验
muser validate samples/lofi-dusk.muser.json
# 渲染（产出同名 .wav）
muser render samples/lofi-dusk.muser.json -o samples/lofi-dusk.wav
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
