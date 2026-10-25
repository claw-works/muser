# muser 示范工程

用 muser 工程 JSON 直接编写、离线渲染的示范，按用途分组：

| 目录 | 内容 |
|---|---|
| [`songs/`](#songs) | 四种风格的完整 loop |
| [`instruments/`](#instruments) | 内置音源巡礼、SFZ 音色库 |
| [`params/`](#params) | 调音色（`instrument_params`）A/B 对比 |
| [`effects/`](#effects) | 内置效果器 A/B 对比 |
| [`automation/`](#automation) | 自动化：参数随时间变化 |
| [`routing/`](#routing) | 混音路由：总线、发送、侧链（A/B 对比） |
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
| `dnb-liquid-skyline` | Liquid Drum & Bass（约 2 分钟完整曲） | 174 | F 小调 Fm7–Dbmaj7–Abmaj7–Eb7；Intro（pad 滤波渐开、高通鼓）→ Build（电钢、sub 第 25 小节进、军鼓滚奏）→ Drop 1（reese 滤波 LFO + 侧链泵感）→ Breakdown（弦乐、主旋律 hook、ride 预示）→ Drop 2（新 reese 节奏、16 分镲、ride、hook 回归）→ Outro；riser / downlifter / impact 过渡，军鼓段尾甩混响 |
| `bach-prelude-c` | 巴赫《C 大调前奏曲》BWV 846（第 1–19 小节，约 80 秒） | 66 | 原谱（公有领域）+ 自己的演奏处理：Salamander 三角钢琴（SFZ，需本机音色库）；按谱时值（右手十六分音符不连奏）+ 真延音踏板（每小节切分换踏板，`pedal` 字段）；低音与高声部顶音略突出；力度随和声张力起伏（第 12 小节减七、第 18 小节属七最强），乐句末呼吸、结尾渐慢渐弱，落在 C 大调长和弦；时值带细微人性化（固定种子） |

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

## routing

总线、发送与侧链（见 [skills/routing.md](../skills/routing.md)）。成对的工程只差路由，可直接 A/B。

| 工程 | 做法 | 听感 |
|---|---|---|
| `shared-reverb` / `shared-reverb-dry` | 电钢、弦乐、合唱各按 -10 / -6 / -3 dB 送进同一条 `hall` 混响总线（总线先 highpass 300 Hz） | 三件乐器在同一个空间里，结尾有约 2 秒的厅堂尾音；dry 版贴耳、干 |
| `drum-bus` / `drum-bus-raw` | 原声鼓三件 `output` 到 `drums` 总线：compressor 4:1 + 高架 + limiter；军鼓再送一点到短房间混响 | 鼓更紧、更响、更"粘"成一体，军鼓带房间感 |
| `sidechain-pump` / `sidechain-pump-off` | pad、贝斯和混响总线都挂 `sidechain: "kick"` 的压缩器（-30 dB、6:1、release 180 ms）；拍手/镲走 `drums` 总线 | 底鼓一响 pad 与贝斯就让开，随四拍"呼吸"（pad+贝斯拍头比拍中低约 13 dB；关闭时约 1.5 dB） |
| `dub-throws` | 军鼓平时几乎不送，每两小节末用 `sends.echo` 的 step 自动化"甩"进延迟；`echo` 总线左右摆动并送进 `space` 混响总线；和弦也送 echo/space；鼓走 `rhythm` 总线压缩 | dub 式回声在乐句末飘出去、在左右之间游走、融进大混响 |

## plugins

需要 `cargo build -p muser-cli --features host` 构建的 muser，以及本机装好对应插件。插件路径写系统目录或 `~/`
（主目录下的 `~/.clap`、`~/.vst3`），装在别处时改路径即可。
插件参数按名称设置，值写插件显示的文字（流程见 [skills/plugins.md](../skills/plugins.md)）。

| 工程 | 内容 |
|---|---|
| `vsti-instrument` | Surge XT（CLAP）当音源，`instrument_params` 设滤波类型、cutoff、谐振、释放 |
| `vst-effect` | poly 和弦经过 Surge XT Effects，`params` 选 `"FX Type": "Reverb 2"` |
| `surge-sweep` | Surge XT 贝斯线：插件 cutoff 自动化 120 Hz → 5000 Hz（文字取值）、声像 LFO，后接 Surge 延迟 |
| `surge-presets` | 三轨都用 Surge XT 内置画像的预设：`"@preset": "bass"`（并用别名 `cutoff` 覆盖）、`pad`（接 Surge XT Effects 的 `reverb` 预设）、`pluck`（别名自动化） |
| `surge-state` | `"@state"` 加载状态文件 `surge-lead.muserstate`；该文件由 `surge-lead.state.json` 描述，渲染脚本现场生成（不入库） |
| `vital-synthwave` | 四个声部全用 Vital 内置画像预设：`bass`（滤波包络）、`pad`（LFO 扫波表帧）、`pluck`（琶音，cutoff 自动化渐亮）、`pwm_lead`（第 5 小节进）；底鼓侧链压 bass、pad 和混响总线 |
| `airwindows-mix` | 整套混音用 Airwindows Consolidated：鼓件 `Console9Channel` → 鼓总线 `Console9Buss` + `BussColors4`，军鼓送 `kRockstar2` 房间；电钢 `Tube` + `TremoSquare`，贝斯 `Density2`，主总线 `ToTape9` |
| `plasma-grit` | Plasma：鼓总线并行 `drum_crush`；贝斯 `bass_grit` 前 4 小节 mix 为 0、第 5 小节起由 30 % 渐增到 60 %（后半段亮度约 3.4 倍）；supersaw 主音 `scream` 40 % |

Surge XT 的振荡器默认自由相位；内置画像的 `setup` 会自动打开 `A Osc N Retrigger`，使结果可复现。
未启用 `host` 特性时报 `HostDisabled`；路径不存在报 `PluginLoad`。

---

工程格式与编曲约定见仓库根目录的 [SKILLS.md](../SKILLS.md)。
