# 音频音轨：直接放录音 / 采样文件

人声、吉他录音、现成的 loop、音效、整段伴奏——不用转成音符，直接放在音轨上，和其它音轨一起混音。

```json
{ "name": "vocal", "instrument": "audio", "gain_db": -2,
  "effects": [{ "type": "eq", "shape": "highpass", "frequency": 120 }, { "type": "compressor", "threshold_db": -20, "ratio": 3 }],
  "sends": { "reverb": -10 },
  "clips": [
    { "start": 8,  "length": 16, "audio": { "file": "vocal.wav", "offset": 1.5, "fade_out": 0.5 } },
    { "start": 32, "length": 8,  "audio": { "file": "vocal.wav", "offset": 30, "fade_in": 0.05, "gain_db": -3 } }
  ] }
```

- 音轨写 `"instrument": "audio"`；它的每个 clip 都写 `audio`，不写音符（notes / abc / grid / drummer / pedal）。
  音频片段不能放在乐器音轨上，乐器音轨的 clip 也不能混写音频。
- 之后和其它音轨一样：`effects`、`gain_db`、`pan`、`output`、`sends`、`sidechain`（可以作为触发源，也可以被别的轨压）、
  `gain_db` / `pan` / `sends` / 效果器的 `automation`、`mix-report`、`muser view` 的推子都照常用。音频音轨没有乐器参数。

## 字段

| 字段 | 单位 | 默认 | 含义 |
|---|---|---|---|
| `file` | 路径 | 必填 | 音频文件。相对路径相对**工程文件所在目录**；也可写绝对路径或 `~/` |
| `offset` | 秒 | 0 | 从文件的第几秒开始播（裁掉开头） |
| `gain_db` | dB | 0 | 片段增益（-60 ~ 24），在音轨推子之前 |
| `fade_in` | 秒 | 0.005 | 淡入 |
| `fade_out` | 秒 | 0.005 | 淡出，结束在片段结尾（clip 长度或文件结尾，先到的那个） |

- clip 的 `start` / `length` 仍然是**拍**：文件从 `start` 开始播，到 `length` 截止；文件比 clip 短就提前结束。
- 文件里的时间（`offset`、淡入淡出）是**秒**。按原速播放：**不随 tempo 拉伸、不变调**。换 tempo 后，音频的起点仍落在
  同一拍上，但它本身的长度（秒）不变——对齐节拍的 loop 素材要和工程同速。
- 格式：WAV（含 RF64）、AIFF、FLAC、Ogg Vorbis、Opus、MP3；macOS / Windows 上还有 AAC、ALAC（m4a，走系统解码器，Linux 不支持）。
  采样率自动转换，单声道放到两边。

## 先看文件再写 clip

```bash
muser audio vocal.wav --tempo 92      # vocal.wav: Wav，48.250 秒，44100 Hz，2 声道，92 bpm 下 73.983 拍
muser audio vocal.wav --json
```

`length` 写成"想放多长"的拍数；整段放完就写文件时长换算的拍数（`--tempo` 给出）。

## 经验

- 录音一般已经有自己的响度：先 `mix-report` 看它和其它轨的相对响度，再调 `gain_db`，不要凭感觉。
- 人声 / 录音轨常用：`eq` highpass 80–150 Hz 去低频轰鸣，轻压缩，送一点混响总线。
- 剪一段：同一个文件放多个 clip，用 `offset` 选从哪开始、`length` 选多长；拼接处给 `fade_in` / `fade_out` 几十毫秒，避免咔哒声。
- 只给某几句加效果：效果器挂在音轨上，不挂在 clip 上。整句处理（回声、镶边、电话音）就把这几句放到另一条 audio 轨，
  挂它自己的效果链；只在某一刻甩一下（句尾混响、回声）就在原轨用 `automation` 拉 `sends.<总线>` 或 `effects[i]` 的 `mix`。
  示例：[samples/audio/spoken-groove](../samples/audio/spoken-groove.muser.json)。
- 鼓 loop 文件可以作为侧链触发源：`"sidechain": "<音频轨名>"`。

## 常用的人声 / 录音效果（音频轨上的 effects）

只想让某几句带效果：把这几句放到单独的 audio 轨（见上一条）。示例：[samples/audio/voice-fx](../samples/audio/voice-fx.muser.json)。

| 效果 | effects（按顺序） | 要点 |
|---|---|---|
| 收音机 / 对讲机 | `eq` highpass 500 ×2、`eq` lowpass 3500 ×2、`eq` peak 1600 Hz +8 dB（q 1.2）、`compressor` -30 dB 8:1 快攻快放 + makeup | 两个滤波器叠起来更陡；只留中频（< 300 Hz 和 > 4 kHz 都约 1%）。电话更窄：300–3000 Hz |
| 水下 | `eq` lowpass 1000 Hz（q 1.4 再叠一个 q 0.7）、`chorus` 0.6 Hz、12 ms、mix 0.6、`reverb` mix 0.45、decay 2.5、damping 0.9，`gain_db` 提高 | 闷、慢速摇晃、暗的余响。低通别低于 ~800 Hz：能量全在 300 Hz 以下时，笔记本 / 手机喇叭几乎放不出来，听着像没声 |
| 隧道 / 大空间 | `eq` highpass 150、`delay` 0.09 s、feedback 0.45、mix 0.35、`reverb` mix 0.55、decay 4.5、damping 0.3、size 0.95 | 短回声叠长混响；混响要长、亮（damping 低） |
| 远处 / 隔墙 | `eq` lowpass 1200–2000、`eq` highpass 200、`reverb` mix 0.5 | 降 `gain_db` 6–10 dB 更像远 |
| 失真 / 扩音器 | 内置效果器没有失真：用 Airwindows 的 `Drive` / `Loud` / `Mackity`，或 [Plasma](plugins/plasma.md)，再接收音机的 EQ | |

### 更多音效：Airwindows Consolidated

装了 [Airwindows Consolidated](plugins/airwindows-consolidated.md)（免费、约 470 个效果，用 CLAP 版）时，音效的选择多得多。
一个 `plugin` 效果器条目是一个效果，`"effect"` 选名字，参数是 0–1；先 `muser plugin params` 看参数名、`muser audition` 试听：

```json
{ "type": "plugin", "path": "~/.clap/Airwindows Consolidated.clap", "params": { "effect": "Drive", "Drive": 0.6, "Dry/Wet": 1.0 } }
```

| 想要 | 效果 |
|---|---|
| 扩音器、对讲机的破音 | `Drive`、`Loud`、`Mackity`、`HardVacuum`，后面接收音机的 EQ |
| 老收音机 / 旧录音的颗粒 | `DeRez5`、`Pockey2`、`BitGlitter`、`GrooveWear`、`ToVinyl4` |
| 磁带摇晃、走带不稳 | `Flutter2`、`Vibrato`（水下的摇晃也可以用它代替 chorus） |
| 机器人 / 怪物声、变调 | `PitchNasty`、`GlitchShifter`（`Note` 半音）、`Gringer` |
| 信号断续、对讲机切换 | `TremoSquare`（方波开关，只在过零点切换） |
| 教堂、大厅、太空 | `kCathedral5`、`Galactic`（在音轨上把 Dry/Wet 降到 0.2–0.35） |
| 模糊、梦境 | `CloudCoat`、`TakeCare` |

没装插件时校验会报"找不到插件"：先 `muser plugin scan` 确认，或退回上面的内置配方。

- 滤波、压缩会改变响度：加完效果跑 `mix-report`，把各条调到差不多响再比较。
- 效果只在某一刻出现（句尾变远、突然进隧道）：在原轨用 `automation` 拉 `effects[i]` 的 `mix` 或 eq 的 `frequency`。

