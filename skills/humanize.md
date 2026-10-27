# 人性化演奏（humanize）

机器式的"每个音都在格子上、力度一样"听起来僵。`humanize` 给音符的时间、力度、时值加上像真人一样的偏移，
再加律动（swing、整体提前/拖后）。写在**音轨**上是整轨默认，写在 **clip** 上覆盖音轨的同名字段；只写要用的项。

```json
{ "name": "hats", "instrument": "drums.909",
  "humanize": { "timing_ms": 6, "drift": 0.6, "velocity": 0.08, "swing": 58, "swing_unit": 0.25 },
  "clips": [ … ] }
```

| 字段 | 含义 | 常见取值 |
|---|---|---|
| `timing_ms` | 时间偏移的标准差（毫秒，正态分布，限在 ±3 倍内） | 鼓 3–8；键盘、吉他 5–12；松弛的 lo-fi 10–20 |
| `drift` | 偏移的连贯程度：0 每个音各自随机；越接近 1 越像真人缓慢漂移（忽快忽慢） | 0.5–0.8 最像人；0 像抖动 |
| `velocity` | 力度偏移的标准差（力度 0–1） | 0.03–0.12 |
| `length` | 时值变化的标准差（比例） | 0–0.15 |
| `swing` | 摇摆（%）：50 直；66.7 三连音感觉；反拍上的音往后推 | house / hip-hop 54–62；shuffle 66 |
| `swing_unit` | 摇摆的单位（拍）：0.5 八分音符（默认），0.25 十六分音符 | hats、16 分节奏用 0.25 |
| `push_ms` | 整轨提前（负）或拖后（正） | 军鼓 lay back +5~+15；急促的贝斯 −5 |
| `seed` | 随机种子。不写时由工程 `seed`、音轨与 clip 位置推导 | — |

- **同一个种子每次渲染完全一样**：听到喜欢的一遍可以留住（把种子写进 `humanize.seed` 固定下来）。
- **换一遍**：改工程的 `seed`（MCP `set_seed`），或试听时 `muser play song.muser.json --seed 7`（`render` 也有 `--seed`）临时换，不改文件。
- 同一时刻按下的音（和弦）共用一个时间偏移，力度各自变化；swing 按工程的拍子网格算（clip 从哪开始都对齐）。
- 量化的 swing 是"律动"，不是随机：整首统一的 swing（同一个 `swing` / `swing_unit`）才会让各轨咬在一起；
  随机偏移（`timing_ms`）各轨可以不同，鼓最小、和声乐器略大。

## 起点

| 想要 | humanize |
|---|---|
| 鼓：不那么机械但仍然紧 | `{ "timing_ms": 4, "drift": 0.6, "velocity": 0.06 }` |
| hip-hop / lo-fi 的松弛鼓 | `{ "timing_ms": 10, "drift": 0.7, "velocity": 0.1, "swing": 60, "swing_unit": 0.25 }`，军鼓轨再加 `"push_ms": 12` |
| house 的 hats | `{ "timing_ms": 3, "velocity": 0.08, "swing": 56, "swing_unit": 0.25 }` |
| 弹出来的钢琴、吉他 | `{ "timing_ms": 8, "drift": 0.5, "velocity": 0.08, "length": 0.08 }` |
| 铺底 pad、长音 | 一般不用；最多 `{ "timing_ms": 15 }` 让和弦里的音别完全同时 |

精确的电子乐（trance 琶音、techno 的 16 分 hats）通常不需要随机偏移，可以只用 `swing` 或不用。
