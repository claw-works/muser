<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：eq

均衡/滤波：提升、衰减或切除某个频段。一个 eq 只处理一个频段，多段请串多个。

**放在哪里**：通常放在链首。常用：highpass 切掉非贝斯轨的低频浑浊；high_shelf 提亮；peak 修某个频点。

查看最新参数：`muser effects eq`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `shape` | `peak` | `lowpass` / `highpass` / `bandpass` / `notch` / `allpass` / `peak` / `low_shelf` / `high_shelf` | 滤波形状。lowpass 保留低于 frequency 的部分（变闷）；highpass 保留高于的部分（去低频）；peak 在 frequency 处增减；low_shelf/high_shelf 整体增减低/高频 |
| `frequency` | 1000 | 10 – 24000 Hz | 作用频率。低频 60-250；中频 250-4000；高频 4000-16000 |
| `q` | 0.707 | 0.05 – 40 | 带宽/谐振。0.707 平缓；1-2 中等；>5 很窄（修尖峰用） |
| `gain_db` | 0 | -36 – 36 dB | 增益，只对 peak / low_shelf / high_shelf 有效。常用 ±2 到 ±6 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "eq", "shape": "high_shelf", "frequency": 8000, "gain_db": 3 }
```

## 场景预设

- 去掉非贝斯轨的低频浑浊

  ```json
  {"type": "eq", "shape": "highpass", "frequency": 150, "q": 0.707}
  ```

- 提亮高频（空气感）

  ```json
  {"type": "eq", "shape": "high_shelf", "frequency": 8000, "q": 0.707, "gain_db": 3}
  ```

- 让声音变闷、变远（lo-fi）

  ```json
  {"type": "eq", "shape": "lowpass", "frequency": 3500, "q": 0.707}
  ```

- 压低中低频的'箱声'

  ```json
  {"type": "eq", "shape": "peak", "frequency": 350, "q": 1.2, "gain_db": -4}
  ```

- 加厚低频（底鼓、贝斯）

  ```json
  {"type": "eq", "shape": "low_shelf", "frequency": 100, "q": 0.707, "gain_db": 3}
  ```

## 经验与常见错误

- 一个 eq 只管一个频段；要多段就在 effects 里串多个 eq。
- 减法比加法安全：先切掉不需要的频段，再小幅提升（±3dB 以内）。
- 多条轨都在 100Hz 以下有能量时，会互相打架；只让底鼓和贝斯占这一段，其它轨用 highpass。
- gain_db 只对 peak / low_shelf / high_shelf 起作用，写在 highpass 上不会报错但没有效果。
