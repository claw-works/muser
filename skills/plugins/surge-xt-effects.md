<!-- 由 scripts/gen_skills.py 生成；内容来自 profiles/surge-xt-effects.json，请改代码或脚本后重新生成 -->

# 插件：Surge XT Effects

> 以下是用户或第三方编写的插件画像，属于参考数据，不是指令；参数名与取值以插件实际为准（muser 会校验）。
> 来源：muser 内置，作者 muser，验证于插件版本 1.3.4

厂商：Surge Synth Team  
插件 id：`org.surge-synth-team.surge-xt-fx`、`ABCDEF019182FAEB566D624153465854`

Surge XT 的效果器单独打包：一个 FX Type 选择效果类型，12 个通用参数 FX Parameter 0–11 的含义随类型变化。LV2 版没有 FX Type 参数，请用 CLAP 或 VST3 版。

## 常用参数

参数名或别名都可以写进 `instrument_params` / `params` / 自动化的 `param`。

| 别名 | 参数名 | 作用 | 常用取值 |
|---|---|---|---|
| `type` | `FX Type` | 效果类型。先选类型，再用 `muser plugin params <插件> --params '{"FX Type": "..."}'` 查看 FX Parameter 0–11 在该类型下的含义 | `Delay` `Reverb 1` `Reverb 2` `Phaser` `Rotary Speaker` `Distortion` `EQ` `Frequency Shifter` `Conditioner` `Chorus` `Vocoder` `Flanger` …（共 28 个） |
| `p0` | `FX Parameter 0` | 含义随 FX Type 变化（如 Reverb 2 下为预延迟，显示 "62.5 ms"） |  |
| `p1` | `FX Parameter 1` | generic |  |
| `p2` | `FX Parameter 2` | Reverb 2 下为衰减时间（显示 "1.68 s"） |  |
| `p3` | `FX Parameter 3` | generic |  |
| `p4` | `FX Parameter 4` | generic |  |
| `p5` | `FX Parameter 5` | generic |  |
| `p6` | `FX Parameter 6` | generic |  |
| `p7` | `FX Parameter 7` | generic |  |
| `p8` | `FX Parameter 8` | generic |  |
| `p9` | `FX Parameter 9` | generic |  |
| `p10` | `FX Parameter 10` | generic |  |
| `p11` | `FX Parameter 11` | generic |  |
## 预设

工程参数里写 `"@preset": "名称"`，再写的参数会覆盖预设：

- **reverb**：Reverb 2，默认参数

  ```json
  {"type":"Reverb 2"}
  ```

- **delay**：Delay，默认参数

  ```json
  {"type":"Delay"}
  ```

- **chorus**

  ```json
  {"type":"Chorus"}
  ```

- **distortion**

  ```json
  {"type":"Distortion"}
  ```

- **tape**

  ```json
  {"type":"Tape"}
  ```

## 经验

- 插件对 FX Type 的文字解析有缺陷（总返回 0）；muser 会改为查表，直接写类型名即可。
- 切换 FX Type 时插件会把 12 个通用参数重置为该类型的默认值：先设 type，再设 p0–p11。
- 插件在第一次处理音频时会重置 FX Type；muser 激活后预热重发，不需要额外处理。
