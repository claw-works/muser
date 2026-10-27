//! `muser play`：用本机声卡把工程从头放一遍，放完退出（给人快速试听草稿；不开网页）。
//! `muser loop`：loop 库（列出、取出写进工程的 clip、存、试听）。

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use muser_core::Project;
use muser_engine::{loop_library, RenderSettings};
use serde_json::json;

/// 放一遍工程（本机声卡）。`from` 秒起。
pub fn play(project: Project, from: f64) -> Result<()> {
    muser_engine::validate(&project)?;
    #[cfg(feature = "device")]
    {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;
        let cache = Arc::new(Mutex::new(muser_engine::RenderCache::new()));
        let player = muser_engine::playback::Player::start(RenderSettings { tail_seconds: 1.5, ..RenderSettings::default() }, cache).map_err(|e| anyhow!(e))?;
        player.load(project);
        if from > 0.0 {
            player.seek(from);
        }
        player.play();
        let mut shown = -1i64;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let s = player.status();
            if let Some(e) = s.error {
                bail!("播放失败: {e}");
            }
            if (s.position as i64) != shown && s.duration > 0.0 {
                shown = s.position as i64;
                eprint!("\r播放中 {:5.1}s / {:.1}s（{}）  Ctrl+C 停止", s.position, s.duration, s.device);
            }
            if s.duration > 0.0 && !s.playing && !s.seeking && s.position >= s.duration - 0.01 {
                eprintln!();
                return Ok(());
            }
        }
    }
    #[cfg(not(feature = "device"))]
    {
        let _ = from;
        bail!("这个 muser 构建没有启用声卡播放（device 特性）；用 `muser render` 渲染成 WAV 再听")
    }
}

#[derive(Subcommand)]
pub enum LoopCommand {
    /// 列出 loop（内置 + 本机 loop 库）。
    List {
        /// 只列这个风格（house、dnb、hiphop …）。
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 输出写进工程音轨的 clip JSON（grid + humanize 内嵌，`from` 标注来源）。
    Show {
        name: String,
        /// clip 起点（拍）。
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        /// 重复几遍（clip 长度 = 遍数 × loop 长度）。
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// 改为输出 loop 文件本身（含风格、速度范围、建议鼓组）。
        #[arg(long)]
        file: bool,
    },
    /// 用本机声卡试听一个 loop。
    Play {
        name: String,
        /// 用这个鼓组（默认 loop 建议的）。
        #[arg(long)]
        kit: Option<String>,
        /// 速度（默认 loop 速度范围的中间）。
        #[arg(long)]
        tempo: Option<f64>,
        /// 放几遍。
        #[arg(long, default_value_t = 4)]
        repeat: u32,
    },
    /// 把工程里一个 clip 存成 loop（有 grid 时存 grid；否则把音符量化成网格）。
    Save {
        /// 工程文件。
        project: String,
        /// 第几条音轨（从 0 起）。
        #[arg(long)]
        track: usize,
        /// 第几个 clip（从 0 起）。
        #[arg(long, default_value_t = 0)]
        clip: usize,
        /// loop 名（小写字母、数字、_、-）。
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "")]
        style: String,
        #[arg(long, default_value = "")]
        description: String,
        /// 量化网格（拍），只在 clip 是普通音符时用。
        #[arg(long, default_value_t = 0.25)]
        step: f64,
        /// 同名已存在时覆盖。
        #[arg(long)]
        overwrite: bool,
    },
}

pub fn run_loop(command: LoopCommand) -> Result<()> {
    let fail = |e: String| anyhow!(e);
    match command {
        LoopCommand::List { style, json } => {
            let mut all = loop_library::list();
            if let Some(s) = &style {
                all.retain(|e| e.item.style.eq_ignore_ascii_case(s) || e.item.tags.iter().any(|t| t.eq_ignore_ascii_case(s)));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
                return Ok(());
            }
            for e in &all {
                let l = &e.item;
                let tempo = l.tempo.map_or(String::new(), |[a, b]| format!("{a:.0}–{b:.0} BPM"));
                let mark = if e.source == "user" { " *" } else { "" };
                println!("{:<18} {:<10} {:>12}  {} 小节  {}{mark}", l.name, l.style, tempo, l.bars, l.description);
            }
            println!(
                "\n共 {} 个（* 为本机 loop 库 {}）。`muser loop show <名字>` 取出写进工程的 clip；`muser loop play <名字>` 试听。",
                all.len(),
                loop_library::dir().display()
            );
            Ok(())
        }
        LoopCommand::Show { name, start, repeat, file } => {
            let l = loop_library::load(&name).map_err(fail)?;
            let out = if file { serde_json::to_value(&l)? } else { l.clip(start, repeat) };
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        LoopCommand::Play { name, kit, tempo, repeat } => {
            let l = loop_library::load(&name).map_err(fail)?;
            let kit = kit.or(l.kit.clone()).unwrap_or_else(|| "drums.acoustic".into());
            let tempo = tempo.or(l.tempo.map(|[a, b]| ((a + b) / 2.0).round())).unwrap_or(120.0);
            eprintln!("{}：{kit}，{tempo} BPM，{repeat} 遍", l.name);
            let project: Project = serde_json::from_value(json!({ "name": format!("loop:{}", l.name), "tempo": tempo,
                "tracks": [{ "name": l.name, "instrument": kit, "clips": [l.clip(0.0, repeat)] }] }))?;
            play(project, 0.0)
        }
        LoopCommand::Save { project, track, clip, name, style, description, step, overwrite } => {
            let p = crate::load_project(&project)?;
            let t = p.tracks.get(track).ok_or_else(|| anyhow!("没有 tracks[{track}]"))?;
            let c = t.clips.get(clip).ok_or_else(|| anyhow!("tracks[{track}] 没有 clips[{clip}]"))?;
            let grid = match (&c.grid, c.notes.is_empty() && c.abc.is_none()) {
                (Some(g), true) => g.clone(),
                _ => loop_library::grid_from_notes(&c.all_notes().map_err(fail)?, c.length.0, step).map_err(fail)?,
            };
            let bars = (c.length.0 / 4.0).ceil().max(1.0) as u32;
            let l = loop_library::Loop {
                format: loop_library::FORMAT.into(),
                version: loop_library::VERSION,
                name,
                style,
                description,
                tags: Vec::new(),
                tempo: Some([p.tempo, p.tempo]),
                bars,
                meter: "4/4".into(),
                kit: t.instrument.starts_with("drums.").then(|| t.instrument.clone()),
                grid,
                humanize: muser_core::humanize::Humanize::merged(t.humanize.as_ref(), c.humanize.as_ref()),
            };
            let path = loop_library::save(&l, overwrite).map_err(fail)?;
            println!("已存 loop '{}' → {}", l.name, path.display());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_produces_a_valid_clip() {
        let l = loop_library::load("dnb_two_step").unwrap();
        let clip = l.clip(8.0, 2);
        assert_eq!((clip["start"].as_f64(), clip["length"].as_f64(), clip["from"].as_str()), (Some(8.0), Some(8.0), Some("loop:dnb_two_step")));
        let p: Project = serde_json::from_value(json!({ "tempo": 174, "tracks": [{ "instrument": "drums.dnb", "clips": [clip] }] })).unwrap();
        muser_engine::validate(&p).unwrap();
        assert_eq!(p.tracks[0].clips[0].all_notes().unwrap().len(), 2 * l.grid.expand(l.beats()).unwrap().len());
    }
}
