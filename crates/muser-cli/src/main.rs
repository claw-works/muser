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

mod view;

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
        /// 插件显示为纯数字的文字用双引号括起来（如 '"200.0","800.0"'）；负数用 --values=-12,0 的写法。
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
        /// 把试听结果记进该插件的用户画像（只对外置插件有效）。
        #[arg(long)]
        record: bool,
        /// 以 JSON 输出。
        #[arg(long)]
        json: bool,
    },
    /// 给人听的播放页：起本地服务（只监听 127.0.0.1），浏览器里播放、看时间线，调音量/静音/独奏，
    /// 改动由引擎重新渲染；「保存」写回工程。工程文件被改了会自动重新渲染、页面刷新。
    View {
        /// 工程文件路径。
        project: String,
        /// 端口。
        #[arg(long, default_value_t = 8737)]
        port: u16,
        /// 启动后用系统浏览器打开页面。
        #[arg(long)]
        open: bool,
        /// 采样率。
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// 结尾余响秒数。
        #[arg(long, default_value_t = 2.0)]
        tail: f64,
    },

    /// 打印工程的信号流：每条音轨/总线送到哪里、发送量、侧链、处理顺序。
    Routing {
        /// 工程文件路径。
        project: String,
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
    /// 插件画像：常用参数别名、预设、经验的数据文件（按插件 id 匹配，可分享）。
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// 保存插件当前状态（音色）到文件：先应用 --params（含画像 setup / @preset），工程里用 "@state": "<文件>" 加载。
    SaveState {
        /// 插件路径，可带 #插件id。
        plugin: String,
        /// 要应用的参数（JSON 对象）。
        #[arg(long)]
        params: Option<String>,
        /// 输出文件（建议扩展名 .muserstate）。
        #[arg(short, long)]
        output: String,
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

#[derive(Subcommand)]
enum ProfileCommand {
    /// 列出所有可用画像（用户目录、MUSER_PROFILE_PATH、内置）。
    List {
        #[arg(long)]
        json: bool,
    },
    /// 显示某插件的画像（插件路径，或画像文件 .json）。
    Show {
        target: String,
        /// 以 Markdown（skill 文档）输出。
        #[arg(long)]
        markdown: bool,
        #[arg(long)]
        json: bool,
    },
    /// 由插件自动生成画像草稿。
    Draft {
        plugin: String,
        /// 对数值类常用参数做试听，把观察结果写进草稿（较慢）。
        #[arg(long)]
        audition: bool,
        /// 保存到用户画像目录（已有同名文件会被覆盖）。
        #[arg(long)]
        save: bool,
        /// 写到指定文件；都不写则打印。
        #[arg(short, long)]
        output: Option<String>,
    },
    /// 校验画像文件；给出插件时同时检查参数名与预设能否在插件上应用。
    Check {
        file: String,
        #[arg(long)]
        plugin: Option<String>,
    },
    /// 显示画像查找路径。
    Path,
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
        Command::Routing { project } => cmd_routing(&project),
        Command::View { project, port, open, sample_rate, tail } => {
            let settings = RenderSettings { sample_rate, tail_seconds: tail, ..RenderSettings::default() };
            view::serve(&project, settings, port, open)
        }
        Command::Plugin { command } => cmd_plugin(command),
        Command::Audition { instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, record, json } => {
            cmd_audition(instrument, effect, source, params, param, values, pitch, velocity, hold, tail, out, record, json)
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

fn cmd_profile(command: ProfileCommand) -> Result<()> {
    use muser_engine::{plugin, profile};
    let fail = |e: String| anyhow::anyhow!(e);
    let load_target = |target: &str| -> Result<profile::Loaded> {
        if target.ends_with(".json") && std::path::Path::new(target).is_file() {
            return Ok(profile::Loaded {
                origin: profile::Origin::File,
                path: Some(target.into()),
                profile: profile::load_file(std::path::Path::new(target)).map_err(fail)?,
            });
        }
        let (id, name, _) = plugin::plugin_identity(target).map_err(fail)?;
        profile::find(&id)
            .map(|l| (*l).clone())
            .ok_or_else(|| anyhow::anyhow!("{name}（{id}）还没有画像；用 `muser plugin profile draft '{target}' --save` 生成一份"))
    };
    match command {
        ProfileCommand::Path => {
            if let Ok(v) = std::env::var("MUSER_PROFILE_PATH") {
                println!("MUSER_PROFILE_PATH: {v}");
            }
            println!("用户画像目录: {}", profile::user_dir().display());
            println!("另有 muser 内置画像（优先级最低）。");
        }
        ProfileCommand::List { json } => {
            let all = profile::all();
            if json {
                let list: Vec<_> = all.iter().map(|l| serde_json::json!({
                    "name": l.profile.plugin.name, "ids": l.profile.plugin.ids, "origin": l.origin, "path": l.path,
                    "tested_version": l.profile.source.tested_version, "params": l.profile.params.len(), "presets": l.profile.presets.len()
                })).collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            for l in &all {
                println!("{:<24} {:>3} 参数 {:>2} 预设  {:?}  {}", l.profile.plugin.name, l.profile.params.len(), l.profile.presets.len(), l.origin, l.path.as_deref().unwrap_or(""));
            }
        }
        ProfileCommand::Show { target, markdown, json } => {
            let l = load_target(&target)?;
            if json {
                println!("{}", l.profile.to_json());
            } else if markdown {
                print!("{}", profile::render_markdown(&l));
            } else {
                let p = &l.profile;
                println!("{} — {}（{:?}，{}）", p.plugin.name, p.plugin.vendor, l.origin, l.path.as_deref().unwrap_or(""));
                if !p.setup.is_empty() {
                    println!("  setup: {}", serde_json::Value::Object(p.setup.clone()));
                }
                for q in &p.params {
                    println!("  {:<14} {}", q.alias.as_deref().unwrap_or("-"), q.name);
                }
                if !p.presets.is_empty() {
                    println!("  预设: {}", p.presets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", "));
                }
                println!("\n`--markdown` 输出 skill 文档。");
            }
        }
        ProfileCommand::Draft { plugin: spec, audition, save, output } => {
            let p = plugin::draft_profile(&spec, audition).map_err(fail)?;
            if save {
                let path = profile::save_user(&p).map_err(fail)?;
                println!("已保存草稿 → {}（{} 个参数）", path.display(), p.params.len());
            } else if let Some(out) = output {
                std::fs::write(&out, p.to_json()).with_context(|| format!("写入 {out} 失败"))?;
                println!("已写出草稿 → {out}（{} 个参数）", p.params.len());
            } else {
                print!("{}", p.to_json());
            }
        }
        ProfileCommand::Check { file, plugin: spec } => {
            let p = profile::load_file(std::path::Path::new(&file)).map_err(fail)?;
            println!("结构 OK：{}，{} 参数，{} 预设", p.plugin.name, p.params.len(), p.presets.len());
            if let Some(spec) = spec {
                let (id, _, version) = plugin::plugin_identity(&spec).map_err(fail)?;
                if !p.matches(&id) {
                    anyhow::bail!("画像的 plugin.ids 不包含该插件的 id '{id}'");
                }
                let mut problems = Vec::new();
                let have: std::collections::HashSet<String> =
                    plugin::params(&spec, None, true, &Default::default()).map_err(fail)?.into_iter().map(|x| x.name).collect();
                for q in &p.params {
                    if !have.contains(&q.name) {
                        problems.push(format!("参数 '{}' 在插件里不存在", q.name));
                    }
                }
                let base = |m: serde_json::Map<String, serde_json::Value>| plugin::check_params_in(&spec, &p, &m);
                if let Err(e) = base(p.setup.clone()) {
                    problems.push(format!("setup: {e}"));
                }
                for pr in &p.presets {
                    if let Err(e) = base(pr.params.clone()) {
                        problems.push(format!("预设 '{}': {e}", pr.name));
                    }
                }
                if !p.source.tested_version.is_empty() && p.source.tested_version != version {
                    println!("提示：画像验证于 {}，已安装 {version}", p.source.tested_version);
                }
                if problems.is_empty() {
                    println!("插件检查 OK：参数名、setup 与 {} 个预设都能应用", p.presets.len());
                } else {
                    for e in &problems {
                        println!("  ✗ {e}");
                    }
                    anyhow::bail!("{} 个问题", problems.len());
                }
            }
        }
    }
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
            match &info.profile {
                Some(pr) => {
                    println!("  画像:     {:?} {}，{} 个别名，预设: {}", pr.origin, pr.path.as_deref().unwrap_or(""), pr.aliases,
                        if pr.presets.is_empty() { "无".into() } else { pr.presets.join(", ") });
                    if !pr.setup.is_empty() {
                        println!("            自动应用 setup: {}（写 \"@setup\": false 关闭）", serde_json::Value::Object(pr.setup.clone()));
                    }
                    println!("            `muser plugin profile show '{spec}' --markdown` 查看使用说明");
                }
                None => println!("  画像:     无（`muser plugin profile draft '{spec}' --save` 生成草稿）"),
            }
            if !info.siblings.is_empty() {
                println!("  同文件其它插件:");
                for (id, name) in &info.siblings {
                    println!("    {name}  →  {}#{id}", info.path);
                }
            }
            println!("\n`muser plugin params '{spec}' --filter <关键词>` 查看参数。");
        }
        PluginCommand::SaveState { plugin: spec, params, output } => {
            let base = parse_map(params.as_deref())?;
            let h = plugin::save_state(&spec, &base, std::path::Path::new(&output)).map_err(fail)?;
            println!("已保存 {} {} 的状态 → {output}", h.plugin_name, h.version);
            println!("工程里写 \"instrument_params\": {{ \"@state\": \"{output}\" }}（效果器写在 params 里）。");
        }
        PluginCommand::Profile { command } => cmd_profile(command)?,
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
                let alias = p.alias.as_ref().map_or(String::new(), |a| format!("  别名 {a}"));
                println!("  {:<40} {} – {}{unit}{steps}  当前 {} \"{}\"  id {}{flags}{alias}",
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
    record: bool,
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
    if record {
        let spec = match &subject {
            Subject::Instrument { name, .. } => name.trim().strip_prefix("plugin:").map(String::from),
            Subject::Effect { effect: muser_core::Effect::Plugin { path, id, .. }, .. } => {
                Some(id.as_ref().map_or(path.clone(), |i| format!("{path}#{i}")))
            }
            _ => None,
        };
        match (spec, &param) {
            (Some(spec), Some(param)) => {
                let path = muser_engine::plugin::record_observations(&spec, param, &takes).map_err(|e| anyhow::anyhow!(e))?;
                eprintln!("已记入画像: {}", path.display());
            }
            _ => anyhow::bail!("--record 需要外置插件（plugin:… 乐器或 plugin 效果器）并给出 --param"),
        }
    }
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

fn cmd_routing(path: &str) -> Result<()> {
    use muser_core::routing::{self, Node};
    let p = load_project(path)?;
    let plan = routing::plan(&p).map_err(|e| anyhow::anyhow!(e))?;
    let name = |n: Node| match n {
        Node::Track(i) => {
            let t = &p.tracks[i];
            format!("{}{}", if t.name.is_empty() { format!("tracks[{i}]") } else { t.name.clone() }, if t.muted { "（静音）" } else { "" })
        }
        Node::Bus(i) => format!("[{}]{}", p.buses[i].name, if p.buses[i].muted { "（静音）" } else { "" }),
    };
    let fx = |list: &[muser_core::Effect]| {
        if list.is_empty() {
            String::new()
        } else {
            format!(" → {}", list.iter().map(|e| match e {
                muser_core::Effect::Compressor { sidechain: Some(k), .. } => format!("compressor⟵{k}"),
                e => e.type_name().to_string(),
            }).collect::<Vec<_>>().join(" → "))
        }
    };
    println!("处理顺序与去向：");
    for n in &plan.order {
        let (src, effects) = match *n {
            Node::Track(i) => (format!("{}（{}）", name(*n), p.tracks[i].instrument), &p.tracks[i].effects),
            Node::Bus(i) => (name(*n), &p.buses[i].effects),
        };
        let outs: Vec<String> = plan
            .edges
            .iter()
            .filter(|e| e.from == *n)
            .map(|e| {
                let to = e.to.map_or("master".to_string(), |b| format!("[{}]", p.buses[b].name));
                match &e.send {
                    Some(_) => format!("{to} 发送 {:+.1} dB", e.level_db),
                    None => to,
                }
            })
            .collect();
        println!("  {src}{} ⇒ {}", fx(effects), outs.join("，"));
    }
    println!("  master{} → 主增益 {:+.1} dB", fx(&p.master_effects), p.master_gain_db);
    if !plan.sidechains.is_empty() {
        println!("侧链：");
        for s in &plan.sidechains {
            println!("  {} 触发 {} 的 effects[{}]", name(s.key), name(s.node), s.effect);
        }
    }
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
