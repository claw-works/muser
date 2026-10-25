<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：phaser

相位器：扫动的陷波，产生流动的"喷气式"质感。

**放在哪里**：适合 pad、电钢、吉他、合成器 lead；放在混响之前。

查看最新参数：`muser effects phaser`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `rate_hz` | 0.5 | 0 – 100 Hz | 扫动速度。0.1-0.5 缓慢；1-3 明显 |
| `feedback` | 0.3 | -0.95 – 0.95 | 谐振强度。绝对值越大越尖锐；负值音色不同 |
| `mix` | 0.5 | 0 – 1 | 干湿比。0.5 效果最明显 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "phaser", "rate_hz": 0.3, "feedback": 0.5, "mix": 0.5 }
```

## 场景预设

- 缓慢流动（pad、电钢）

  ```json
  {"type": "phaser", "rate_hz": 0.2, "feedback": 0.3, "mix": 0.5}
  ```

- 70 年代放克吉他/电钢

  ```json
  {"type": "phaser", "rate_hz": 0.6, "feedback": 0.5, "mix": 0.5}
  ```

- 强烈喷气式

  ```json
  {"type": "phaser", "rate_hz": 1.5, "feedback": 0.8, "mix": 0.6}
  ```

## 经验与常见错误

- mix 0.5 时效果最明显；更高或更低都会减弱扫频感。
- phaser 放在 reverb 之前。
