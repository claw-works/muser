<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 效果器：plugin

第三方插件效果器（VST3/CLAP/LV2）。需要用 host 特性构建。

**放在哪里**：与内置效果器一样串在链上。path 是本机插件的绝对路径；params 按插件参数名设置，值可写插件界面上的文字（如 "2000 Hz"）。用 `muser plugin params <path>` 查看参数。

查看最新参数：`muser effects plugin`（MCP：`describe_effect`）

## 字段

| 字段 | 说明 |
|---|---|
| `path` | 必填，插件文件或包的绝对路径（.vst3 / .clap / .lv2） |
| `id` | 可选，插件 id |
| `params` | 可选，`{ 参数id: 值 }` |

## 最小示例

```json
{ "type": "plugin", "path": "/usr/lib/clap/Surge XT Effects.clap", "params": { "FX Type": "Reverb 2" } }
```

## 经验与常见错误

- 需要用 `cargo build -p muser-cli --features host` 构建的 muser；未启用时渲染报 HostDisabled。
- path 是本机绝对路径，常见目录：/usr/lib/vst3、~/.vst3、/usr/lib/clap、~/.clap、/usr/lib/lv2。
- 一个插件文件里有多个插件时，用 `id` 指定；不写则用第一个。
- `params` 用插件参数**名称**做键（大小写不敏感，也可写数字 id），值写插件显示的文字（如 "2000 Hz"、"Reverb 2"）、数字（插件自动化刻度）或 true/false。
- 查参数：`muser plugin params <path> --filter <关键词>`；选项类参数的可选文字：`muser plugin values <path> <参数名>`；参数作用不明时用 `muser audition --effect ... --param ...` 试听。完整流程见 [plugins.md](../plugins.md)。
- 插件参数可以自动化：`{ "target": "effects[0]", "param": "<插件参数名>", ... }`，取值同样可写文字。
- 加载插件会运行第三方代码，只使用可信的本机插件。
