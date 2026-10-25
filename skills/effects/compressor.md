<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：compressor

压缩器：压低超过阈值的峰值，让动态更平稳、更紧实。

**放在哪里**：单轨上放在链首（EQ 之后）控制动态；主总线上做"胶水"，在 limiter 之前。写 "sidechain": "<音轨或总线名>" 时改由那条信号触发：底鼓压贝斯/pad 的泵感（常用 threshold -30、ratio 6-10、attack 1-5、release 100-200）。

查看最新参数：`muser effects compressor`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `threshold_db` | -18 | -60 – 0 dB | 阈值。越低压得越多。轻压 -12；常规 -18；重压 -30 |
| `ratio` | 4 | 1 – 100 :1 | 压缩比。2 温和；4 常规；8+ 接近限幅 |
| `attack_ms` | 10 | 0.01 – 1000 ms | 启动时间。短(1-5)压住瞬态；长(20-50)保留鼓的冲击 |
| `release_ms` | 120 | 1 – 5000 ms | 释放时间。短(50)跟得紧有泵感；长(200-500)更自然 |
| `makeup_db` | 0 | -24 – 36 dB | 补偿增益，弥补压缩损失的音量。一般 0-6 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "compressor", "threshold_db": -16, "ratio": 3, "makeup_db": 2 }
```

## 场景预设

- 主总线胶水（整体更紧实）

  ```json
  {"type": "compressor", "threshold_db": -14, "ratio": 2, "attack_ms": 30, "release_ms": 200, "makeup_db": 2}
  ```

- 鼓组冲击（保留瞬态）

  ```json
  {"type": "compressor", "threshold_db": -18, "ratio": 4, "attack_ms": 25, "release_ms": 100, "makeup_db": 3}
  ```

- 贝斯平稳（音量起伏变小）

  ```json
  {"type": "compressor", "threshold_db": -20, "ratio": 4, "attack_ms": 5, "release_ms": 150, "makeup_db": 3}
  ```

- 重压（明显的泵感）

  ```json
  {"type": "compressor", "threshold_db": -30, "ratio": 8, "attack_ms": 1, "release_ms": 60, "makeup_db": 8}
  ```

- 侧链泵感（底鼓一响就压下 pad/贝斯，EDM 呼吸感）

  ```json
  {"type": "compressor", "threshold_db": -30, "ratio": 6, "attack_ms": 1, "release_ms": 180, "sidechain": "kick"}
  ```

## 经验与常见错误

- 压缩后整体音量会变小，用 makeup_db 补回；补完后峰值可能变高，主总线再用 limiter 收住。
- attack 太短会削掉鼓的'啪'声，鼓组建议 ≥ 20ms。
- 主总线常见顺序：eq → compressor → limiter。
- `sidechain` 写音轨或总线名：由那个信号（推子后）触发压缩，被压的是本轨。见 [routing.md](../routing.md)。
- 侧链泵感的深浅主要看 threshold 和 ratio，节奏感看 release：release ≈ 一拍的 1/3 到 1/2 时最'呼吸'。
- 键信号的音轨即使 muted 也照样触发侧链：常用一条静音的'幽灵底鼓'只做触发。
