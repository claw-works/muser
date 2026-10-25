//! muser CLI —— AI/用户驱动引擎的命令行接口。
//!
//! 这是「AI 接口层」的 CLI 形态。每个子命令都是一个明确的、可被 AI 通过 shell
//! 调用的工具。将来 MCP server 会把这些能力再包一层给 agent 直接调用。
//!
//! 子命令：
//! - `render <project.json> [-o out.wav]`  读工程 JSON，离线渲染出 WAV
//! - `validate <project.json>`             校验工程 JSON 是否合法
//! - `instruments`                          列出所有可用乐器名
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
    /// 列出所有可用的乐器名。
    Instruments,
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
        Command::Instruments => cmd_instruments(),
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
    project.validate().with_context(|| "工程校验未通过")?;
    println!(
        "OK: '{}' 合法 — tempo {}, {} 轨, 时长 {:.2} 拍",
        project.name,
        project.tempo,
        project.tracks.len(),
        project.duration_beats().0
    );
    Ok(())
}

fn cmd_instruments() -> Result<()> {
    println!("可用乐器：");
    for name in muser_engine::available_instruments() {
        println!("  {name}");
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
