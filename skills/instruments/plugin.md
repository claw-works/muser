<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->

# 乐器：外置 VSTi（第三方虚拟乐器）

把本机安装的 VST3 / CLAP / LV2 乐器插件当作音源。音轨写法：

```json
{ "name": "lead", "instrument": "plugin:/usr/lib/vst3/Surge XT.vst3", "clips": [ ... ] }
```

- 语法：`plugin:<绝对路径>`；文件里有多个插件时用 `plugin:<路径>#<插件id>`。
- 需要用 `cargo build -p muser-cli --features host` 构建的 muser，否则报 HostDisabled。
- 音符 `pitch` 是 MIDI 音高；插件自己决定音色与音域。
- 采样器类插件（如 sfizz）需要加载音色文件才有声音；目前还不能通过工程设置插件参数或音色，这是后续工作。
- 常见插件目录：/usr/lib/vst3、~/.vst3、/usr/lib/clap、~/.clap、/usr/lib/lv2。
- 加载插件会运行第三方代码，只使用可信的本机插件。
- `muser validate` 只检查 `plugin:` 语法，不加载插件；路径错误会在 render 时报 PluginLoad。
