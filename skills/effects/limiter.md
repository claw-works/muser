<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：limiter

限幅器：保证输出峰值不超过上限，防止削顶爆音。

**放在哪里**：只放在主总线（master_effects）的最后一个。

查看最新参数：`muser effects limiter`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `ceiling_db` | -0.3 | -24 – 0 dB | 输出上限。-0.3 到 -1 常用 |
| `release_ms` | 50 | 1 – 2000 ms | 释放时间。短更响但可能失真；长更平滑 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "limiter", "ceiling_db": -0.5 }
```

## 场景预设

- 常规母带（安全）

  ```json
  {"type": "limiter", "ceiling_db": -1.0, "release_ms": 80}
  ```

- 更响（流行/电子）

  ```json
  {"type": "limiter", "ceiling_db": -0.3, "release_ms": 50}
  ```

## 经验与常见错误

- 只放在 master_effects 的最后一个。
- limiter 只防止爆音，不会让作品自动变好听；输入推得太猛会挤压动态、失真。
- 要更响时，先提 master_gain_db 或前面 compressor 的 makeup_db，再由 limiter 收住峰值。
