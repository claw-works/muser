//! 剪开、挪动音频轨上的片段：`muser view` 的剪刀 / 拖动，与给 agent 的 `muser clip split / move` 共用。

use anyhow::{anyhow, bail, Result};
use muser_core::{Beats, Project};

/// 能编辑的音频轨（没有 parts）。
fn audio_track(p: &mut Project, ti: usize) -> Result<&mut muser_core::Track> {
    let t = p.tracks.get_mut(ti).ok_or_else(|| anyhow!("没有 tracks[{ti}]"))?;
    if t.instrument.trim() != muser_core::audio::INSTRUMENT || !t.parts.is_empty() {
        bail!("只能剪开、挪动音频轨上的片段（tracks[{ti}] 是 {}）", t.instrument);
    }
    Ok(t)
}

fn bar(p: &Project, b: f64) -> String {
    let (k, into) = p.bar_map().bar_at(b);
    format!("第 {} 小节第 {:.2} 拍", k + 1, into + 1.0)
}

/// 在第 `at` 拍把第 `ci` 个片段剪成两段（接缝各 10 毫秒淡化）。返回给人看的说明。
pub fn split(p: &mut Project, ti: usize, ci: usize, at: f64) -> Result<String> {
    let tm = p.tempo_map();
    let where_ = bar(p, at);
    let t = audio_track(p, ti)?;
    let c = t.clips.get(ci).cloned().ok_or_else(|| anyhow!("没有这个片段"))?;
    let audio = c.audio.clone().ok_or_else(|| anyhow!("这个片段不是音频"))?;
    let (a, z) = (c.start.0, c.start.0 + c.length.0);
    if !(at > a + 0.01 && at < z - 0.01) {
        bail!("剪刀要落在片段中间（片段在 {a:.3}–{z:.3} 拍）");
    }
    let (mut left, mut right) = (c.clone(), c);
    left.length = Beats(at - a);
    right.start = Beats(at);
    right.length = Beats(z - at);
    let (mut la, mut ra) = (audio.clone(), audio.clone());
    la.fade_out = 0.01;
    ra.fade_in = 0.01;
    ra.offset = audio.offset + tm.secs_between(a, at - a);
    left.audio = Some(la);
    right.audio = Some(ra);
    t.clips[ci] = left;
    t.clips.insert(ci + 1, right);
    Ok(format!("✂ {}：在{where_}剪开", t.name))
}

/// 把第 `ci` 个片段挪到第 `to` 拍开始。返回说明。
pub fn move_to(p: &mut Project, ti: usize, ci: usize, to: f64) -> Result<String> {
    let tm = p.tempo_map();
    let to = to.max(0.0);
    let (from_s, to_s) = (bar(p, p.tracks.get(ti).and_then(|t| t.clips.get(ci)).map_or(0.0, |c| c.start.0)), bar(p, to));
    let t = audio_track(p, ti)?;
    let c = t.clips.get_mut(ci).ok_or_else(|| anyhow!("没有这个片段"))?;
    if c.audio.is_none() {
        bail!("这个片段不是音频");
    }
    let from = c.start.0;
    c.start = Beats(to);
    t.clips.sort_by(|x, y| x.start.0.total_cmp(&y.start.0));
    Ok(format!("↔ {}：片段从{from_s}挪到{to_s}（{:+.0} 毫秒）", t.name, (tm.secs_at(to) - tm.secs_at(from)) * 1000.0))
}

/// `muser clip split|move`：改工程文件里这条音频轨的 clips（其余内容不动）。
pub fn run(project_path: &str, track: &str, split_at: Option<f64>, clip: Option<usize>, to: Option<f64>, by_ms: Option<f64>) -> Result<()> {
    let text = std::fs::read_to_string(project_path)?;
    let mut v: serde_json::Value = serde_json::from_str(&text)?;
    let mut p = Project::from_json(&text).map_err(|e| anyhow!("{e}"))?;
    let ti = p.tracks.iter().position(|t| t.name == track).ok_or_else(|| anyhow!("没有名为 '{track}' 的音轨"))?;
    let what = match split_at {
        Some(at) => {
            let ci = p.tracks[ti].clips.iter().position(|c| c.audio.is_some() && at > c.start.0 && at < c.start.0 + c.length.0).ok_or_else(|| anyhow!("第 {at} 拍不在 '{track}' 的任何音频片段里"))?;
            split(&mut p, ti, ci, at)?
        }
        None => {
            let ci = clip.ok_or_else(|| anyhow!("move 要 --clip（片段序号，muser takes 或工程里 clips 的顺序，从 0 起）"))?;
            let start = p.tracks[ti].clips.get(ci).ok_or_else(|| anyhow!("'{track}' 没有第 {ci} 个片段"))?.start.0;
            let to = match (to, by_ms) {
                (Some(t), _) => t,
                (None, Some(ms)) => {
                    let tm = p.tempo_map();
                    tm.beat_at(tm.secs_at(start) + ms / 1000.0)
                }
                _ => bail!("move 要 --to <拍> 或 --by-ms <毫秒>"),
            };
            move_to(&mut p, ti, ci, to)?
        }
    };
    v["tracks"][ti]["clips"] = serde_json::to_value(&p.tracks[ti].clips)?;
    std::fs::write(project_path, serde_json::to_string_pretty(&v)? + "\n")?;
    println!("{what}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_then_move() {
        let mut p = Project::from_json(r#"{ "tempo": 120, "tracks": [{ "name": "v", "instrument": "audio",
            "clips": [{ "start": 4, "length": 8, "audio": { "file": "a.wav", "offset": 1.0 } }] }] }"#).unwrap();
        split(&mut p, 0, 0, 6.0).unwrap();
        let c = &p.tracks[0].clips;
        assert_eq!((c[0].start.0, c[0].length.0, c[1].start.0, c[1].length.0), (4.0, 2.0, 6.0, 6.0));
        assert_eq!(c[1].audio.as_ref().unwrap().offset, 2.0, "120 BPM：2 拍 = 1 秒，接着原来的 1 秒往后");
        assert!(split(&mut p, 0, 0, 4.0).is_err(), "剪刀落在边上不剪");
        let s = move_to(&mut p, 0, 1, 5.86).unwrap();
        assert!(s.contains("-70 毫秒"), "{s}");
    }
}
