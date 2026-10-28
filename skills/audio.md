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
