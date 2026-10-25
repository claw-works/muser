//! muser CLI —— AI/用户驱动引擎的命令行接口。
//!
//! 这是「AI 接口层」的 CLI 形态。每个子命令都是一个明确的、可被 AI 通过 shell
//! 调用的工具。将来 MCP server 会把这些能力再包一层给 agent 直接调用。
//!
//! 子命令：
//! - `render <project.json> [-o out.wav]`  读工程 JSON，离线渲染出 WAV
//! - `validate <project.json>`             校验工程 JSON 是否合法
//! - `instruments [name] [--family F] [--json]`  列出乐器 / 查看某乐器详情
//! - `effects [name] [--json]`              列出效果器 / 查看某效果器参数
//! - `new [-o project.json]`                生成一个示例工程 JSON（供 AI 参考格式）

use std::fs;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use muser_core::{Clip, Note, Project, Track};
use muser_engine::{render_to_wav, RenderSettings};

#[derive(Parser)]
#[command(name = "muser", version, about = "面向 AI 的无头编曲引擎 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 读工程 JSON，离线渲染为 WAV。
    Render {
        /// 工程文件路径（.json）。
        project: String,
        /// 输出 WAV 路径。
        #[arg(short, long, default_value = "out.wav")]
        output: String,
        /// 采样率。
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// 结尾余响秒数。
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
    },
    /// 校验工程 JSON 是否合法。
    Validate {
        /// 工程文件路径。
        project: String,
    },
    /// 列出可用乐器；给出名称则显示该乐器详情。
    Instruments {
        /// 乐器名（如 guitar.nylon / poly.moog_bass）；缺省列出全部。
        name: Option<String>,
        /// 只列某个类别（drums / synth / guitar / bass / organ / mallet / strings / wind / choir / poly）。
        #[arg(long)]
        family: Option<String>,
        /// 以 JSON 输出（便于 AI 解析）。
        #[arg(long)]
        json: bool,
    },
    /// 列出可用效果器；给出名称则显示该效果器的参数、范围、默认值与示例。
    Effects {
        /// 效果器名（如 reverb）；缺省列出全部。
        name: Option<String>,
        /// 以 JSON 输出（便于 AI 解析）。
        #[arg(long)]
        json: bool,
    },
    /// 外置插件（VST3 / CLAP / LV2）：扫描本机、查看信息、列参数。需要 host 特性。
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// 试听：把乐器或效果器的某个参数依次设成几个值，各渲染一段测试音，报告响度、亮度、起音、余音等特征。
    Audition {
        /// 乐器名（如 poly.moog_bass、sfz:/x.sfz、plugin:/x.clap#id）；与 --effect 二选一。
        #[arg(long)]
        instrument: Option<String>,
        /// 效果器 JSON（如 '{"type":"reverb"}' 或 '{"type":"plugin","path":"..."}'）；与 --instrument 二选一。
        #[arg(long)]
        effect: Option<String>,
        /// 试效果器时用来发声的乐器。
        #[arg(long, default_value = "synth.poly")]
        source: String,
        /// 乐器的基础参数（JSON 对象）。
        #[arg(long)]
        params: Option<String>,
        /// 要试的参数名（内置参数名、效果器字段名或插件参数名）。省略则只渲染原样一版。
        #[arg(long)]
        param: Option<String>,
        /// 取值，逗号分隔；数字按数字处理，其余作为文字交给插件解析（如 "200 Hz,2000 Hz"）。
        #[arg(long, value_delimiter = ',')]
        values: Vec<String>,
        /// 测试音高（MIDI；鼓组用 GM 鼓键）。
        #[arg(long, default_value_t = 60)]
        pitch: u8,
        /// 力度 0-1。
        #[arg(long, default_value_t = 0.8)]
        velocity: f32,
        /// 音符时长（秒）。
        #[arg(long, default_value_t = 1.0)]
        hold: f64,
        /// 音符结束后再录多久（秒）。
        #[arg(long, default_value_t = 1.5)]
        tail: f64,
        /// 把每一版写成 wav 的目录。
        #[arg(long)]
        out: Option<String>,
        /// 以 JSON 输出。
        #[arg(long)]
        json: bool,
    },
    /// 生成一个示例工程 JSON（供 AI 参考格式）。
    New {
        /// 输出路径；缺省打印到 stdout。
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginCommand {
    /// 扫描本机标准插件目录（每个插件在独立进程中读取，结果有缓存）。
    Scan {
        /// 只列某类：instrument / effect。
        #[arg(long)]
        kind: Option<String>,
        /// 忽略缓存重新扫描。
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// 插件信息：名称、厂商、类型、输入输出、参数总数与分组。
    Info {
        /// 插件路径，可带 #插件id。
        plugin: String,
        #[arg(long)]
        json: bool,
    },
    /// 列出插件参数（名称、范围、默认、当前值与文字）。
    Params {
        /// 插件路径，可带 #插件id。
        plugin: String,
        /// 关键词过滤（空格分隔，全部命中；大小写不敏感），如 "filter cutoff"。
        #[arg(long)]
        filter: Option<String>,
        /// 包括插件标记为隐藏的参数。
        #[arg(long)]
        all: bool,
        /// 先应用这些参数（JSON 对象），如 '{"FX Type": "Reverb 2"}'，再列出（参数含义随它变化时有用）。
        #[arg(long)]
        params: Option<String>,
        /// 最多显示多少个（默认 50；0 = 不限）。
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// 列出某个参数可写的文字值（选项类参数即全部选项，如滤波类型、波形）。
    Values {
        /// 插件路径，可带 #插件id。
        plugin: String,
        /// 参数名或 id。
        param: String,
        /// 沿范围取样的点数。
        #[arg(long, default_value_t = 400)]
        samples: usize,
        /// 先应用这些参数（JSON 对象）。
        #[arg(long)]
        params: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    // 本进程被插件扫描器以子进程方式启动时，读完插件即退出。
    muser_engine::plugin::serve_scanner_if_asked();
    let cli = Cli::parse();
    match cli.command {
        Command::Render {
            project,
            output,
            sample_rate,
            tail,
        } => cmd_render(&project, &output, sample_rate, tail),
        Command::Validate { project } => cmd_validate(&project),
        Command::Instruments { name, family, json } => {
            cmd_instruments(name.as_deref(), family.as_deref(), json)
        }
        Command::Effects { name, json } => cmd_effects(name.as_deref(), json),
        Command::New { output } => cmd_new(output.as_deref()),
        Command::Plugin { command } => cmd_plugin(command),
        Command::Audition { instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, json } => {
            cmd_audition(instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, json)
        }
    }
}

fn load_project(path: &str) -> Result<Project> {
    let text = fs::read_to_string(path).with_context(|| format!("读取工程文件失败: {path}"))?;
    let project = Project::from_json(&text).with_context(|| format!("解析工程 JSON 失败: {path}"))?;
    Ok(project)
}

fn cmd_render(project_path: &str, output: &str, sample_rate: u32, tail: f64) -> Result<()> {
    let project = load_project(project_path)?;
    let settings = RenderSettings {
        sample_rate,
        tail_seconds: tail,
        ..RenderSettings::default()
    };
    let (seconds, peak_db) = render_to_wav(&project, output, settings)
        .with_context(|| format!("渲染失败: {project_path}"))?;
    println!(
        "已渲染 '{}' → {output}: {:.2}s, 峰值 {:.1} dBFS",
        project.name, seconds, peak_db
    );
    Ok(())
}

fn cmd_validate(project_path: &str) -> Result<()> {
    let project = load_project(project_path)?;
    muser_engine::validate(&project).with_context(|| "工程校验未通过")?;
    println!(
        "OK: '{}' 合法 — tempo {}, {} 轨, 时长 {:.2} 拍",
        project.name,
        project.tempo,
        project.tracks.len(),
        project.duration_beats().0
    );
    Ok(())
}

fn note_label(n: muser_engine::NoteConvention) -> &'static str {
    match n {
        muser_engine::NoteConvention::GmDrums => "GM 鼓键",
        muser_engine::NoteConvention::Pitched => "音阶音高",
    }
}

fn cmd_instruments(name: Option<&str>, family: Option<&str>, json: bool) -> Result<()> {
    if let Some(name) = name {
        let info = muser_engine::describe_instrument(name)
            .with_context(|| format!("未知乐器 '{name}'，用 `muser instruments` 查看全部"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&info)?);
        } else {
            println!("{}", info.name);
            println!("  类别:   {}", info.family);
            println!("  音高:   {}", note_label(info.notes));
            println!("  音域:   {} - {}", info.range.0, info.range.1);
            println!("  复音数: {}", info.polyphony);
            println!("  说明:   {}", info.description);
            if !info.params.is_empty() {
                println!("  可调参数（instrument_params）:");
                for p in &info.params {
                    let spec = serde_json::to_value(p.spec)?;
                    let range = match spec["type"].as_str() {
                        Some("number") => {
                            let unit = spec["unit"].as_str().unwrap_or("");
                            let mut r = format!("{} – {}{}", spec["min"], spec["max"], if unit.is_empty() { String::new() } else { format!(" {unit}") });
                            if spec["integer"] == true { r.push_str("（整数）"); }
                            if let Some(n) = spec["names"].as_array() {
                                r.push_str(&format!("，或 {}", n.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("/")));
                            }
                            r
                        }
                        Some("choice") => spec["choices"].as_array().map(|c| c.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" / ")).unwrap_or_default(),
                        Some("bool") => "true / false".into(),
                        Some("drawbars") => "9 位数字串，每位 0-8".into(),
                        _ => String::new(),
                    };
                    let current = if p.current.is_null() { String::new() } else { format!("  当前 {}", p.current) };
                    let auto = match p.automation {
                        muser_engine::Automatable::Live => "  [可自动化]",
                        muser_engine::Automatable::NextNote => "  [可自动化·下个音生效]",
                        muser_engine::Automatable::No => "",
                    };
                    println!("    {:<16} {range}{current}{auto}", p.spec.name);
                    println!("    {:<16} {}", "", p.spec.description);
                }
            }
            if !info.params_note.is_empty() {
                println!("  说明:   {}", info.params_note);
            }
        }
        return Ok(());
    }
    let list: Vec<_> = muser_engine::instrument_catalog()
        .into_iter()
        .filter(|i| match family {
            Some(f) => i.family == f || i.family.starts_with(&format!("{f}/")),
            None => true,
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let mut current = String::new();
    for info in &list {
        let group = info.family.split('/').next().unwrap_or("").to_string();
        if group != current {
            println!("\n[{group}]");
            current = group;
        }
        println!("  {:<28} {}", info.name, info.description);
    }
    println!(
        "\n共 {} 个。另可用 sfz:/path/to.sfz 加载 SFZ 音色库，plugin:/path/to.vst3 加载第三方 VSTi（需 host 特性）。\n`muser instruments <名称>` 查看详情与可调参数。",
        list.len()
    );
    Ok(())
}

fn cmd_effects(name: Option<&str>, json: bool) -> Result<()> {
    if let Some(name) = name {
        let spec = muser_core::effect_spec(name)
            .with_context(|| format!("未知效果器 '{name}'，用 `muser effects` 查看全部"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(spec)?);
            return Ok(());
        }
        println!("{} — {}", spec.name, spec.summary);
        println!("位置: {}", spec.placement);
        if !spec.params.is_empty() || !spec.choices.is_empty() {
            println!("参数:");
        }
        for c in spec.choices {
            println!("  {:<14} 默认 {:<10} 可选 {}", c.name, c.default, c.choices.join(" / "));
            println!("  {:<14} {}", "", c.description);
        }
        for p in spec.params {
            let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
            println!(
                "  {:<14} 默认 {:<10} 范围 [{}, {}]{}",
                p.name, p.default, p.min, p.max, unit
            );
            println!("  {:<14} {}", "", p.description);
        }
        println!("示例: {}", spec.example);
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(muser_core::EFFECT_SPECS)?);
        return Ok(());
    }
    println!("可用效果器（用于 track.effects 或 master_effects）：");
    for spec in muser_core::EFFECT_SPECS {
        println!("  {:<12} {}", spec.name, spec.summary);
    }
    println!("\n`muser effects <名称>` 查看参数、范围与示例。");
    Ok(())
}

fn parse_map(s: Option<&str>) -> Result<serde_json::Map<String, serde_json::Value>> {
    match s {
        Some(s) => serde_json::from_str(s).context("--params 应为 JSON 对象"),
        None => Ok(serde_json::Map::new()),
    }
}

fn cmd_plugin(command: PluginCommand) -> Result<()> {
    use muser_engine::plugin;
    let fail = |e: String| anyhow::anyhow!(e);
    match command {
        PluginCommand::Scan { kind, refresh, json } => {
            let (mut found, failures) = plugin::scan(refresh).map_err(fail)?;
            if let Some(k) = &kind {
                found.retain(|p| &p.kind == k);
            }
            if json {
                let failures: Vec<_> = failures.iter().map(|(p, e)| serde_json::json!({ "path": p, "error": e })).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "plugins": found, "failures": failures }))?);
                return Ok(());
            }
            for p in &found {
                println!("[{:<10}] {:<5} {} — {}", p.kind, p.format, p.name, p.vendor);
                println!("             {}#{}", p.path, p.id);
            }
            for (p, e) in &failures {
                println!("[失败] {p}: {e}");
            }
            println!("\n共 {} 个插件{}。乐器写 \"instrument\": \"plugin:<路径>#<id>\"；效果器写 {{\"type\": \"plugin\", \"path\": ..., \"id\": ...}}。",
                found.len(), if failures.is_empty() { String::new() } else { format!("，{} 个读取失败", failures.len()) });
        }
        PluginCommand::Info { plugin: spec, json } => {
            let info = plugin::info(&spec).map_err(fail)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
                return Ok(());
            }
            println!("{} — {} {}", info.name, info.vendor, info.version);
            println!("  类型:     {}（{}）  分类: {}", info.kind, info.format, info.categories.join(", "));
            println!("  id:       {}", info.id);
            println!("  接收音符: {}", if info.takes_notes { "是" } else { "否" });
            println!("  音频输入: {}", if info.audio_inputs.is_empty() { "无".into() } else { info.audio_inputs.join("; ") });
            println!("  音频输出: {}", info.audio_outputs.join("; "));
            if info.latency_samples > 0 {
                println!("  延迟:     {} 帧", info.latency_samples);
            }
            println!("  参数:     {} 个（可见 {}）", info.params_total, info.params_visible);
            let mut groups = info.param_groups.clone();
            groups.sort_by(|a, b| b.1.cmp(&a.1));
            let shown: Vec<String> = groups.iter().take(20).map(|(g, n)| format!("{g}({n})")).collect();
            println!("  参数分组: {}", shown.join(" "));
            if !info.siblings.is_empty() {
                println!("  同文件其它插件:");
                for (id, name) in &info.siblings {
                    println!("    {name}  →  {}#{id}", info.path);
                }
            }
            println!("\n`muser plugin params '{spec}' --filter <关键词>` 查看参数。");
        }
        PluginCommand::Values { plugin: spec, param, samples, params, json } => {
            let base = parse_map(params.as_deref())?;
            let (p, values) = plugin::values(&spec, &param, samples, &base).map_err(fail)?;
            if json {
                let list: Vec<_> = values.iter().map(|(v, t)| serde_json::json!({ "value": v, "text": t })).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "param": p, "values": list }))?);
                return Ok(());
            }
            println!("{}（id {}，当前 \"{}\"）可写的值：", p.name, p.id, p.current_text);
            let many = values.len() > 40;
            for (i, (v, t)) in values.iter().enumerate() {
                if many && i >= 20 && i + 10 < values.len() {
                    if i == 20 {
                        println!("  ……（共 {} 个，连续参数，中间省略）", values.len());
                    }
                    continue;
                }
                println!("  {:<28} = {v:.6}", format!("\"{t}\""));
            }
            println!("\n在 params / instrument_params 里写文字即可，如 {{ \"{}\": \"{}\" }}。", p.name, values.last().map_or("", |(_, t)| t.as_str()));
        }
        PluginCommand::Params { plugin: spec, filter, all, params, limit, json } => {
            let base = parse_map(params.as_deref())?;
            let list = plugin::params(&spec, filter.as_deref(), all, &base).map_err(fail)?;
            let total = list.len();
            let shown: Vec<_> = if limit == 0 { list } else { list.into_iter().take(limit).collect() };
            if json {
                println!("{}", serde_json::to_string_pretty(&shown)?);
                return Ok(());
            }
            for p in &shown {
                let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
                let steps = p.steps.map_or(String::new(), |s| format!(" {s} 档"));
                let flags = format!("{}{}", if p.automatable { "" } else { " 不可自动化" }, if p.read_only { " 只读" } else { "" });
                println!("  {:<40} {} – {}{unit}{steps}  当前 {} \"{}\"  id {}{flags}",
                    p.name, p.min, p.max, p.current, p.current_text, p.id);
            }
            if total > shown.len() {
                println!("  …… 还有 {} 个，用 --filter 缩小范围或 --limit 0 显示全部", total - shown.len());
            }
            println!("\n共 {total} 个。设置时用参数名做键，值可写插件显示的文字（如 \"{}\"）或数字。",
                shown.first().map_or("…".into(), |p| p.current_text.clone()));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_audition(
    instrument: Option<String>,
    effect: Option<String>,
    source: String,
    params: Option<String>,
    param: Option<String>,
    values: Vec<String>,
    pitch: u8,
    velocity: f32,
    hold: f64,
    tail: f64,
    out: Option<String>,
    json: bool,
) -> Result<()> {
    use muser_engine::audition::{audition, parse_value, AuditionSettings, Subject};
    let subject = match (instrument, effect) {
        (Some(name), None) => {
            let params = match params {
                Some(p) => serde_json::from_str(&p).context("--params 应为 JSON 对象")?,
                None => serde_json::Map::new(),
            };
            Subject::Instrument { name, params }
        }
        (None, Some(e)) => Subject::Effect { effect: serde_json::from_str(&e).context("--effect 应为效果器 JSON")?, source },
        _ => anyhow::bail!("请给出 --instrument 或 --effect 之一"),
    };
    let values: Vec<serde_json::Value> = values.iter().filter(|v| !v.trim().is_empty()).map(|v| parse_value(v)).collect();
    let settings = AuditionSettings { pitch, velocity, hold_seconds: hold, tail_seconds: tail, out_dir: out.map(Into::into) };
    let takes = audition(&subject, param.as_deref(), &values, &settings)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&takes)?);
        return Ok(());
    }
    println!("{:<14} {:>7} {:>7} {:>7} {:>9} {:>6} {:>6} {:>6} {:>8} {:>9}",
        param.as_deref().unwrap_or("取值"), "峰值dB", "RMS dB", "LUFS", "重心Hz", "低%", "中%", "高%", "起音ms", "余音s");
    for t in &takes {
        let a = &t.analysis;
        let v = match &t.value { serde_json::Value::Null => "原样".to_string(), serde_json::Value::String(s) => s.clone(), v => v.to_string() };
        println!("{:<14} {:>7.1} {:>7.1} {:>7.1} {:>9.0} {:>6.1} {:>6.1} {:>6.1} {:>8.0} {:>8.2}{}",
            v, a.peak_db, a.rms_db, a.lufs, a.centroid_hz, a.low_pct, a.mid_pct, a.high_pct, a.attack_ms, a.release_s,
            if a.release_truncated { "+" } else { " " });
        if let Some(w) = &t.wav {
            println!("{:<14} → {w}", "");
        }
    }
    println!("\n重心越高越亮；余音带 + 表示被渲染长度截断（用 --tail 加长）。");
    Ok(())
}

fn cmd_new(output: Option<&str>) -> Result<()> {
    // 一个带鼓+贝斯线的示例工程，展示完整格式。
    let mut p = Project::new("example", 90.0);

    let mut drums = Track::new("drums", "drums.lofi");
    let mut dclip = Clip::new(0.0, 4.0);
    // 4/4 lo-fi：kick 在 1、3；snare 在 2、4；闭镲走八分
    dclip.notes.push(Note::new(36, 0.0, 0.25));
    dclip.notes.push(Note::new(38, 1.0, 0.25));
    dclip.notes.push(Note::new(36, 2.0, 0.25));
    dclip.notes.push(Note::new(38, 3.0, 0.25));
    for i in 0..8 {
        dclip.notes.push(Note::new(42, i as f64 * 0.5, 0.1));
    }
    drums.clips.push(dclip);
    p.tracks.push(drums);

    let mut bass = Track::new("bass", "synth.fm");
    bass.gain_db = -3.0;
    let mut bclip = Clip::new(0.0, 4.0);
    // 简单根音走向：C2 - C2 - G1 - A1
    bclip.notes.push(Note::new(36, 0.0, 1.0));
    bclip.notes.push(Note::new(36, 1.0, 1.0));
    bclip.notes.push(Note::new(31, 2.0, 1.0));
    bclip.notes.push(Note::new(33, 3.0, 1.0));
    bass.clips.push(bclip);
    p.tracks.push(bass);

    let json = p.to_json()?;
    match output {
        Some(path) => {
            fs::write(path, &json).with_context(|| format!("写入失败: {path}"))?;
            println!("已生成示例工程: {path}");
        }
        None => println!("{json}"),
    }
    Ok(())
}
