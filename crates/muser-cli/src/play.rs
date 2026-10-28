//! `muser play`：用本机声卡把工程从头放一遍，放完退出（给人快速试听草稿；不开网页）。
//! `muser loop`：loop 库（列出、取出写进工程的 clip、存、试听）。

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use muser_core::Project;
use muser_engine::{loop_library, RenderSettings};

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
    /// 列出 loop：项目（工程旁的 loops/）、全局（~/.config/muser/loops）、内置。
    List {
        /// 只列这个风格（house、dnb、hiphop …）。
        #[arg(long)]
        style: Option<String>,
        /// 工程文件（找它旁边的 loops/；缺省当前目录）。
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 输出写进工程音轨的 clip JSON（grid / notes + humanize 内嵌，`from` 标注来源）。
    Show {
        name: String,
        /// clip 起点（拍）。
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        /// 重复几遍（clip 长度 = 遍数 × loop 长度）。
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// 改为输出 loop 文件本身（含风格、速度、建议乐器、调、和弦）。
        #[arg(long, conflicts_with = "abc")]
        file: bool,
        /// 改为输出 ABC 记谱（旋律 loop）。
        #[arg(long)]
        abc: bool,
        #[arg(long)]
        project: Option<String>,
    },
    /// 用本机声卡试听一个 loop。
    Play {
        name: String,
        /// 用这个乐器 / 鼓组（默认 loop 建议的）。
        #[arg(long, alias = "kit")]
        instrument: Option<String>,
        /// 速度（默认 loop 速度范围的中间）。
        #[arg(long)]
        tempo: Option<f64>,
        /// 放几遍。
        #[arg(long, default_value_t = 4)]
        repeat: u32,
        #[arg(long)]
        project: Option<String>,
    },
    /// 把工程里一个 clip 存成 loop：鼓存 grid，旋律存音符。缺省存到工程旁的 loops/。
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
        /// 鼓的量化网格（拍），只在鼓的 clip 是普通音符时用。
        #[arg(long, default_value_t = 0.25)]
        step: f64,
        /// 存到全局（~/.config/muser/loops），而不是项目。
        #[arg(long)]
        global: bool,
        /// 同名已存在时覆盖。
        #[arg(long)]
        overwrite: bool,
    },
    /// 哼唱转谱：把一段单旋律录音（哼、唱、口哨、单音乐器）转成音符，存成旋律 loop（不用 AI）。
    Hum {
        /// 音频文件（WAV、AIFF、FLAC、Ogg、Opus、MP3 …）。
        audio: String,
        /// loop 名（缺省用音频文件名）。
        #[arg(long)]
        name: Option<String>,
        /// 速度（BPM）：按它把秒换成拍。
        #[arg(long, default_value_t = 100.0)]
        tempo: f64,
        /// 对齐网格：1/16、1/8、1/8t、0.25（拍），off 为不对齐。
        #[arg(long, default_value = "1/16")]
        grid: String,
        /// 保留录音开头到第一个音的空白（跟着节拍器录时用），否则第一个音移到第 0 拍。
        #[arg(long)]
        from_start: bool,
        /// 建议的乐器（缺省 poly.juno_keys）。
        #[arg(long)]
        instrument: Option<String>,
        /// 调（如 Am）；缺省由音高估计。
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        project: Option<String>,
        /// 存到全局，而不是项目。
        #[arg(long)]
        global: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// 把项目里的 loop 复制到全局（别的项目也能用）。
    Share {
        name: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        overwrite: bool,
    },
    /// 打开 loop / 动机编辑器（本地网页：旋律用卷帘，鼓用步进网格），存到项目或全局。
    Edit {
        /// 打开这个 loop（缺省新建）。
        name: Option<String>,
        /// 工程文件（loop 存在它旁边的 loops/；缺省当前目录）。
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 8739)]
        port: u16,
        /// 用系统浏览器打开。
        #[arg(long)]
        open: bool,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
    },
}

/// `--project` 的工程文件所在目录。
fn base_of(project: &Option<String>) -> Option<std::path::PathBuf> {
    project.as_ref().map(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf).unwrap_or_default())
}

pub fn run_loop(command: LoopCommand) -> Result<()> {
    let fail = |e: String| anyhow!(e);
    match command {
        LoopCommand::List { style, project, json } => {
            let base = base_of(&project);
            let mut all = loop_library::list_for(base.as_deref());
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
                let mark = match e.source { "project" => " +", "user" => " *", _ => "" };
                let kind = if l.is_drums() { "鼓" } else { "旋律" };
                println!("{:<18} {:<4} {:<10} {:>12}  {} 小节  {}{mark}", l.name, kind, l.style, tempo, l.bars, l.description);
            }
            println!(
                "\n共 {} 个（+ 为项目 {}，* 为全局 {}）。`muser loop show <名字>` 取出写进工程的 clip；`muser loop play <名字>` 试听；`muser loop edit` 编辑。",
                all.len(),
                loop_library::project_dir(base.as_deref()).display(),
                loop_library::dir().display()
            );
            Ok(())
        }
        LoopCommand::Show { name, start, repeat, file, abc, project } => {
            let l = loop_library::load_for(&name, base_of(&project).as_deref()).map_err(fail)?;
            if abc {
                let p = l.project(None, None, repeat).map_err(fail)?;
                let mut opts = muser_engine::abc_export::AbcOptions::new(muser_engine::abc_export::Dialect::Standard);
                opts.key = l.key.clone();
                let out = muser_engine::abc_export::export(&p, &opts)?;
                print!("{}", out.text);
                return Ok(());
            }
            let out = if file { serde_json::to_value(&l)? } else { l.clip(start, repeat) };
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        LoopCommand::Play { name, instrument, tempo, repeat, project } => {
            let l = loop_library::load_for(&name, base_of(&project).as_deref()).map_err(fail)?;
            let p = l.project(instrument.as_deref(), tempo, repeat).map_err(fail)?;
            eprintln!("{}：{}，{} BPM，{repeat} 遍", l.name, p.tracks[0].instrument, p.tempo);
            play(p, 0.0)
        }
        LoopCommand::Save { project, track, clip, name, style, description, step, global, overwrite } => {
            let p = crate::load_project(&project)?;
            let t = p.tracks.get(track).ok_or_else(|| anyhow!("没有 tracks[{track}]"))?;
            let c = t.clips.get(clip).ok_or_else(|| anyhow!("tracks[{track}] 没有 clips[{clip}]"))?;
            let drums = t.instrument.trim().starts_with("drums.");
            let notes = muser_engine::clip_notes(&p, track, clip).map_err(fail)?;
            let grid = match (drums, &c.grid, c.notes.is_empty() && c.abc.is_none() && c.drummer.is_none()) {
                (false, _, _) => muser_core::grid::Grid::default(),
                (true, Some(g), true) => g.clone(),
                (true, _, _) => loop_library::grid_from_notes(&notes, c.length.0, step).map_err(fail)?,
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
                kit: drums.then(|| t.instrument.clone()),
                instrument: (!drums).then(|| t.instrument.clone()),
                key: None,
                chords: None,
                grid,
                notes: if drums { Vec::new() } else { notes.into_iter().filter(|n| n.start.0 < f64::from(bars) * 4.0).collect() },
                humanize: muser_core::humanize::Humanize::merged(t.humanize.as_ref(), c.humanize.as_ref()),
            };
            let path = if global { loop_library::save(&l, overwrite) } else { loop_library::save_project(&l, p.base_dir.as_deref(), overwrite) }.map_err(fail)?;
            println!("已存 loop '{}' → {}", l.name, path.display());
            Ok(())
        }
        LoopCommand::Hum { audio, name, tempo, grid, from_start, instrument, key, project, global, overwrite } => {
            let step = muser_engine::hum::parse_grid(&grid).map_err(fail)?;
            let path = std::path::Path::new(&audio);
            let mut r = muser_engine::hum::transcribe_file(path, muser_engine::hum::HumOptions { tempo, step, from_start }).map_err(fail)?;
            if let Some(k) = key {
                r.key = k;
            }
            let name = name.unwrap_or_else(|| {
                let stem = path.file_stem().map_or("hum".into(), |s| s.to_string_lossy().to_lowercase());
                stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
            });
            let l = loop_library::Loop::melody(&name, r.bars, tempo, Some(instrument.unwrap_or_else(|| "poly.juno_keys".into())), Some(r.key.clone()), r.notes);
            let base = base_of(&project);
            let saved = if global { loop_library::save(&l, overwrite) } else { loop_library::save_project(&l, base.as_deref(), overwrite) }.map_err(fail)?;
            println!("已转出 {} 个音，{} 小节，调 {}（音准整体偏 {:+} 音分，已校正）→ {}", l.notes.len(), l.bars, r.key, r.tuning_cents, saved.display());
            let p = l.project(None, None, 1).map_err(fail)?;
            let mut opts = muser_engine::abc_export::AbcOptions::new(muser_engine::abc_export::Dialect::Standard);
            opts.key = Some(r.key);
            if let Ok(abc) = muser_engine::abc_export::export(&p, &opts) {
                print!("{}", abc.text.lines().skip_while(|l| !l.starts_with("K:")).collect::<Vec<_>>().join("\n"));
                println!();
            }
            println!("`muser loop edit {}` 在卷帘里修改；`muser loop play {}` 试听。", l.name, l.name);
            Ok(())
        }
        LoopCommand::Share { name, project, overwrite } => {
            let path = loop_library::share(&name, base_of(&project).as_deref(), overwrite).map_err(fail)?;
            println!("已复制到全局 → {}", path.display());
            Ok(())
        }
        LoopCommand::Edit { name, project, port, open, sample_rate } => crate::loop_edit::serve(name, base_of(&project), port, open, sample_rate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
