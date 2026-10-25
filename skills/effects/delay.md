<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：delay

延迟/回声：把信号延后若干时间重复播放。

**放在哪里**：适合 lead、和弦、人声；放在混响之前。time 按节拍算更合拍：60/tempo 秒 = 一拍。

查看最新参数：`muser effects delay`（MCP：`describe_effect`）

## 参数

| 参数 | 默认 | 范围 | 说明 |
|---|---|---|---|
| `time_seconds` | 0.3 | 0 – 10 s | 回声间隔。按节拍取值：四分音符=60/tempo，八分=30/tempo，附点八分=45/tempo |
| `feedback` | 0.35 | 0 – 0.99 | 回声重复次数。0.2 一两次；0.4-0.5 多次；>0.7 会长时间堆积 |
| `mix` | 0.3 | 0 – 1 | 干湿比。0.2-0.35 常规；>0.5 回声比原声还响 |

所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。

## 最小示例

```json
{ "type": "delay", "time_seconds": 0.35, "feedback": 0.4, "mix": 0.3 }
```

## 场景预设

- 八分音符回声 @120bpm（time = 30/tempo）

  ```json
  {"type": "delay", "time_seconds": 0.25, "feedback": 0.35, "mix": 0.3}
  ```

- 附点八分 @120bpm（time = 45/tempo，流行/摇滚 lead 经典）

  ```json
  {"type": "delay", "time_seconds": 0.375, "feedback": 0.4, "mix": 0.3}
  ```

- slapback 短回声（复古人声/吉他）

  ```json
  {"type": "delay", "time_seconds": 0.11, "feedback": 0.0, "mix": 0.25}
  ```

- dub 长回声（反复多次）

  ```json
  {"type": "delay", "time_seconds": 0.5, "feedback": 0.65, "mix": 0.4}
  ```

## 经验与常见错误

- time_seconds 按 tempo 计算才合拍：四分 = 60/tempo，八分 = 30/tempo，附点八分 = 45/tempo，十六分 = 15/tempo。
- feedback > 0.7 时回声会堆积成持续轰鸣，只适合 dub 或过渡段。
- 密集的快速音符加长回声会变糊；回声适合留有空隙的乐句。
