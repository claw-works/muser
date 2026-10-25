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
    /// 生成一个示例工程 JSON（供 AI 参考格式）。
    New {
        /// 输出路径；缺省打印到 stdout。
        #[arg(short, long)]
        output: Option<String>,
    },
}

fn main() -> Result<()> {
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
