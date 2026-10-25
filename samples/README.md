# muser 示范工程

用 muser 工程 JSON 直接编写、离线渲染的示范，按用途分组：

| 目录 | 内容 |
|---|---|
| [`songs/`](#songs) | 四种风格的完整 loop |
| [`instruments/`](#instruments) | 内置音源巡礼、SFZ 音色库 |
| [`params/`](#params) | 调音色（`instrument_params`）A/B 对比 |
| [`effects/`](#effects) | 内置效果器 A/B 对比 |
| [`automation/`](#automation) | 自动化：参数随时间变化 |
| [`plugins/`](#plugins) | 外置 VST 效果器与 VSTi 音源（需 `host` 特性 + 本机插件） |

wav 不入库，用脚本一次渲染（生成在工程旁边，同名 `.wav`）：

```bash
scripts/render_samples.sh            # 全部
scripts/render_samples.sh effects    # 只渲染一个分组
```

依赖本机文件的工程（`plugins/`、`instruments/sfz-piano`）缺文件时会单独报错，不影响其它工程。

## songs

四种风格，各 8 拍，鼓 + 贝斯 + 和声三轨。

| 工程 | 风格 | 速度 | 要点 |
|---|---|---|---|
| `lofi-dusk` | Lo-fi Hip-hop | 85 | 慵懒摇摆鼓、FM 贝斯、poly 七和弦 pad，keys 轨混响 + 主总线压缩 |
| `techno-pulse` | Techno | 128 | 四踩底鼓、off-beat 开镲、十六分闭镲、断奏 bassline、和弦 stab |
| `house-groove` | House | 124 | 四踩底鼓 + 二四拍手、律动贝斯、C→Dm 和弦 |
| `dnb-roller` | Drum & Bass | 174 | breakbeat 鼓型、超低 sub-bass、长音 pad 铺底 |

## instruments

- `tour`：每 2 秒换一种内置音源演奏同一乐句，共 20 种：
  guitar.nylon → guitar.steel → bass.finger → bass.acid → organ.jazz → mallet.marimba →
  mallet.vibraphone → mallet.kalimba → strings.cello → strings.ensemble → strings.violin_pizz →
  wind.flute → wind.sax → wind.trumpet → choir.aah → poly.moog_bass → poly.trance_supersaw →
  poly.warm_analog_pad → poly.juno_keys → poly.bell_pluck。完整列表用 `muser instruments` 查看（115 个）。
- `sfz-piano`：内置 SFZ 采样器播放 freepats 的 PianoFB 钢琴。路径是本机路径
  `/data/Music/SFZ-samples/freepats/...`，换机器需改成你自己的 `.sfz` 文件。

## params

同一段内容，一个用乐器预设原样，一个用 `instrument_params` 调过。

| 对比 | 调了什么 | 听感 |
|---|---|---|
| `acid-preset` / `acid-tuned` | bass.acid：cutoff 220、resonance 0.92、env_mod 0.85、drive_db 8，部分音符 `accent` | 更"吱"、更脏，重音明显跳出来 |
| `cello-preset` / `cello-tuned` | strings.cello：attack 0.6、vibrato_depth 45、release 1.5、tone 2500、pressure 0.85 | 起音更慢、揉弦更深、更暗更有颗粒感 |

## effects

同一段内容（lo-fi 鼓 + poly 和弦），只有效果器不同；`dry` 是对照组。

| 工程 | 效果 | 听感 |
|---|---|---|
| `dry` | 无 | 和弦干净短促 |
| `reverb` | keys：mix 0.45、decay 2.8s、size 0.8 | 明显空间感与尾音 |
| `delay` | keys：0.45s、feedback 0.45、mix 0.4 | 有节奏的回声，逐渐衰减 |
| `chorus` | keys：rate 0.9Hz、depth 4ms、mix 0.5 | 变厚变宽、轻微起伏 |
| `eq` | keys：高架 +6dB@4k + 低切 200Hz | 更亮更清爽，低频被切 |
| `limiter` | master：+6dB 增益 + limiter（ceiling -0.3dB） | 更响，峰值收在 0dB 内 |
| `phaser` | keys：rate 0.5Hz、feedback 0.4、mix 0.6 | 流动的"喷气式"扫频 |
| `flanger` | keys：rate 0.2Hz、feedback 0.7、mix 0.6 | 金属感的扫动 |

## automation

参数随时间变化（格式见 [skills/automation.md](../skills/automation.md)）。

| 工程 | 自动化了什么 | 听感 |
|---|---|---|
| `acid-sweep` | 303 cutoff 8 小节 200→3000 Hz（exp），后半段 resonance 加大，delay mix 渐增 | 经典 acid 扫频，越来越亮、越来越"吱" |
| `random-walk-seed1` / `random-walk-seed2` | 同一工程只换 `seed`：cutoff 随机游走、decay 随机滑动、声像随机漂移 | 两版走向不同，每版重复渲染结果一样 |
| `pad-motion` | 前 2 小节淡入；声像正弦摆动；cutoff 三角 LFO；第 5 小节起方波 gate | 慢慢浮现、左右流动，后半段变成 trance gate |
| `choir-vowels` | 合唱元音 a→e→o→u→a，亮度先升后降 | 和弦在元音间平滑变化 |
| `build-up` | 主总线混响 mix 0.05→0.45；stab 的 lowpass 400→16000 Hz；军鼓调音逐步升高 | 典型的电子乐 build-up：越来越亮、越来越湿、越来越紧 |

## plugins

需要 `cargo build -p muser-cli --features host` 构建的 muser，工程里的插件路径是本机绝对路径，换机器需修改。

- `vst-effect`：poly 和弦经过 `Surge XT Effects.vst3`（外置 VST 效果器，`{ "type": "plugin", "path": ... }`）。
- `vsti-instrument`：`Surge XT.vst3` 合成器作为音源（`"instrument": "plugin:/path.vst3"`）。

未启用 `host` 特性时报 `HostDisabled`；路径不存在报 `PluginLoad`。

---

工程格式与编曲约定见仓库根目录的 [SKILLS.md](../SKILLS.md)。
