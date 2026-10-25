<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：chorus

合唱：叠加轻微失谐的副本，让音色变厚变宽。

**放在哪里**：适合 pad、电钢、吉他、和弦；放在混响之前。贝斯慎用（低频相位抵消）。

查看最新参数：`muser effects chorus`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `rate_hz` | 0.8 | 0 – 100 Hz | 调制速度。0.2-1 缓慢自然；2-5 明显颤动 |
| `depth_ms` | 3 | 0 – 50 ms | 调制深度。1-3 细腻；5-10 明显失谐 |
| `mix` | 0.4 | 0 – 1 | 干湿比。0.3-0.5 常规 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "chorus", "rate_hz": 0.6, "depth_ms": 4, "mix": 0.5 }
```

## 场景预设

- 轻微加厚（电钢、pad）

  ```json
  {"type": "chorus", "rate_hz": 0.5, "depth_ms": 2.0, "mix": 0.3}
  ```

- 80 年代宽阔合成器

  ```json
  {"type": "chorus", "rate_hz": 0.8, "depth_ms": 5.0, "mix": 0.5}
  ```

- 明显颤动（特殊效果）

  ```json
  {"type": "chorus", "rate_hz": 3.0, "depth_ms": 6.0, "mix": 0.5}
  ```

## 经验与常见错误

- 贝斯加 chorus 会让低频相位抵消、发虚；如需要，mix ≤ 0.2。
- chorus 放在 reverb 之前。
