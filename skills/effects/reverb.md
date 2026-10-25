<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：reverb

混响：模拟房间/大厅的空间感与尾音。

**放在哪里**：放在旋律、pad、人声类轨的链尾；贝斯、底鼓少用或不用，否则低频发浑。

查看最新参数：`muser effects reverb`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `mix` | 0.25 | 0 – 1 | 干湿比。0.1-0.2 轻微空间；0.25-0.35 常规；>0.5 很湿、声音变远 |
| `decay_seconds` | 1.6 | 0.05 – 120 s | 尾音衰减时间。0.3-0.8 小房间；1.2-2.5 大厅；>4 氛围/环境 |
| `damping` | 0.5 | 0 – 1 | 高频阻尼。越大尾音越暗越温暖；越小越亮越金属 |
| `size` | 0.5 | 0 – 1 | 空间大小。0.2 小房间；0.5 中等；0.8+ 大厅/教堂 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "reverb", "mix": 0.3, "decay_seconds": 2.2, "size": 0.7 }
```

## 场景预设

- 小房间（鼓、原声乐器显得真实）

  ```json
  {"type": "reverb", "mix": 0.15, "decay_seconds": 0.6, "damping": 0.5, "size": 0.25}
  ```

- 常规大厅（pad、弦乐、人声）

  ```json
  {"type": "reverb", "mix": 0.3, "decay_seconds": 2.0, "damping": 0.5, "size": 0.6}
  ```

- 氛围长尾（ambient、lo-fi 键盘）

  ```json
  {"type": "reverb", "mix": 0.45, "decay_seconds": 5.0, "damping": 0.6, "size": 0.9}
  ```

- 明亮金属感（钟琴、合成器 pluck）

  ```json
  {"type": "reverb", "mix": 0.25, "decay_seconds": 1.8, "damping": 0.15, "size": 0.6}
  ```

## 经验与常见错误

- 多条轨都要空间感时，每条轨用小 mix（0.1-0.2），比单条轨大 mix 更自然。
- 贝斯、底鼓尽量不加；非要加时 mix ≤ 0.1，并在前面串一个 `eq` highpass 150Hz。
- 混响放在链尾（delay 之后）；放在 compressor 之前会让尾音被压缩而变得不自然。
- 尾音会延续到工程结束之后，渲染时 `--tail` 要大于 decay_seconds，否则会被截断。
