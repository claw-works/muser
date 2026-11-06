//! 录人声的"遍"（take）：`muser view` 里录的每一遍都立刻存成工程目录下 `recordings/<轨名>-take-NN.wav`，
//! 并记进 `recordings/takes.json`（录在第几拍、多长、什么时候录的）。工程 JSON 只在点"保存"时才写进选定的那几遍。
//! `muser takes` 给 agent 看每一遍（长度、响度、实际开始唱 / 唱完的位置、工程用了没有），帮用户挑、剪、对齐。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use muser_core::Project;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一遍录音。`file` 相对工程文件所在的目录（就是音频片段里写的路径）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub file: String,
    pub track: String,
    /// 录在第几拍开始（片段的起点）。
    pub start: f64,
    pub seconds: f64,
    /// 录制时间（Unix 秒）。
    pub recorded: u64,
}

/// 一遍录音最大多少字节（48 kHz、16 位、单声道约 40 分钟）。
const MAX_BYTES: usize = 256 << 20;

fn dir(project: &Path) -> PathBuf {
    project.parent().unwrap_or(Path::new(".")).join("recordings")
}

/// 清单里的每一遍（没有清单时为空）。
pub fn list(project: &Path) -> Vec<Take> {
    std::fs::read_to_string(dir(project).join("takes.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_list(project: &Path, takes: &[Take]) -> Result<()> {
    std::fs::write(dir(project).join("takes.json"), serde_json::to_string_pretty(takes)? + "\n")?;
    Ok(())
}

/// 存下一遍：`wav` 是浏览器传来的 WAV 文件内容。文件名由这里生成（轨名只留字母数字），只写进 `recordings/`。
pub fn store(project: &Path, track: &str, start: f64, wav: &[u8]) -> Result<Take> {
    if wav.len() > MAX_BYTES {
        bail!("recording too large ({} MB, max {} MB)", wav.len() >> 20, MAX_BYTES >> 20);
    }
    if !(start.is_finite() && start >= 0.0) {
        bail!("invalid start: {start}");
    }
    let parsed = muser_engine::wav::read(wav).map_err(|e| anyhow!("not a readable WAV: {e:?}"))?;
    let seconds = parsed.buffer.num_frames() as f64 / f64::from(parsed.sample_rate.max(1));
    if seconds < 0.05 {
        bail!("recording too short");
    }
    let stem: String = track.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
    let stem = if stem.is_empty() { "vocal".to_string() } else { stem };
    let d = dir(project);
    std::fs::create_dir_all(&d).with_context(|| format!("cannot create {}", d.display()))?;
    let mut takes = list(project);
    let n = (1..).find(|k| !d.join(format!("{stem}-take-{k:02}.wav")).exists()).unwrap_or(1);
    let name = format!("{stem}-take-{n:02}.wav");
    std::fs::write(d.join(&name), wav)?;
    let take = Take {
        file: format!("recordings/{name}"),
        track: track.to_string(),
        start,
        seconds,
        recorded: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
    };
    takes.push(take.clone());
    write_list(project, &takes)?;
    Ok(take)
}

/// 删掉一遍（文件与清单项）。`in_use` 为真时不删。
pub fn delete(project: &Path, file: &str, in_use: bool) -> Result<()> {
    let mut takes = list(project);
    let i = takes.iter().position(|t| t.file == file).ok_or_else(|| anyhow!("{file} is not in the manifest"))?;
    if in_use {
        bail!("{file} is in use by the project: switch to another take or delete that clip first");
    }
    let path = dir(project).join(Path::new(file).file_name().ok_or_else(|| anyhow!("invalid file name"))?);
    let _ = std::fs::remove_file(path);
    takes.remove(i);
    write_list(project, &takes)
}

/// 工程里有没有片段在用这个文件。
pub fn referenced(p: &Project, file: &str) -> bool {
    p.tracks.iter().flat_map(|t| &t.clips).any(|c| c.audio.as_ref().is_some_and(|a| a.file == file))
}

/// 把一遍贴到工作副本：名为 `take.track` 的音频轨（没有就新建），和这一遍重叠的旧片段换掉。
pub fn place(p: &mut Project, take: &Take) -> Result<()> {
    let tm = p.tempo_map();
    let beats = tm.beat_at(tm.secs_at(take.start) + take.seconds) - take.start;
    let i = match p.tracks.iter().position(|t| t.name == take.track) {
        Some(i) => i,
        None => {
            p.tracks.push(serde_json::from_value(json!({ "name": take.track, "instrument": "audio", "clips": [] }))?);
            p.tracks.len() - 1
        }
    };
    let t = &mut p.tracks[i];
    if t.instrument.trim() != muser_core::audio::INSTRUMENT {
        bail!("track '{}' is not an audio track ({}); record to another track name", t.name, t.instrument);
    }
    let (a, z) = (take.start, take.start + beats);
    t.clips.retain(|c| c.start.0 + c.length.0 <= a + 1e-9 || c.start.0 >= z - 1e-9);
    t.clips.push(serde_json::from_value(clip_json(take, beats))?);
    t.clips.sort_by(|x, y| x.start.0.total_cmp(&y.start.0));
    Ok(())
}

fn clip_json(take: &Take, beats: f64) -> Value {
    json!({ "start": take.start, "length": (beats * 1000.0).round() / 1000.0, "audio": { "file": take.file } })
}

/// 保存时把一遍写进工程 JSON（与 [`place`] 一样：找轨或新建、换掉重叠的片段）。
pub fn place_json(v: &mut Value, take: &Take, tm: &muser_core::tempo::TempoMap) -> Result<()> {
    let beats = tm.beat_at(tm.secs_at(take.start) + take.seconds) - take.start;
    let tracks = v.get_mut("tracks").and_then(Value::as_array_mut).ok_or_else(|| anyhow!("the project file has no tracks"))?;
    let i = match tracks.iter().position(|t| t.get("name").and_then(Value::as_str) == Some(take.track.as_str())) {
        Some(i) => i,
        None => {
            tracks.push(json!({ "name": take.track, "instrument": "audio", "clips": [] }));
            tracks.len() - 1
        }
    };
    let t = tracks[i].as_object_mut().ok_or_else(|| anyhow!("malformed track"))?;
    let clips = t.entry("clips").or_insert_with(|| json!([])).as_array_mut().ok_or_else(|| anyhow!("clips is not an array"))?;
    let (a, z) = (take.start, take.start + beats);
    clips.retain(|c| {
        let s = c.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let l = c.get("length").and_then(Value::as_f64).unwrap_or(0.0);
        s + l <= a + 1e-9 || s >= z - 1e-9
    });
    clips.push(clip_json(take, beats));
    clips.sort_by(|x, y| x["start"].as_f64().unwrap_or(0.0).total_cmp(&y["start"].as_f64().unwrap_or(0.0)));
    Ok(())
}

/// `muser takes`：列出每一遍，`--timing` 逐句量节奏，或 `--clean` 删掉工程没用到的。
pub fn run(project_path: &str, clean: bool, timing: bool) -> Result<()> {
    let path = Path::new(project_path);
    let project = Project::from_json(&std::fs::read_to_string(path)?).map_err(|e| anyhow!("{e}"))?.at_file(path);
    if timing {
        return report_timing(&project);
    }
    let takes = list(path);
    if takes.is_empty() {
        println!("{project_path}: no recordings yet (record with the microphone button on a vocal track in muser view)");
        return Ok(());
    }
    if clean {
        let unused: Vec<&Take> = takes.iter().filter(|t| !referenced(&project, &t.file)).collect();
        for t in &unused {
            delete(path, &t.file, false)?;
            println!("deleted {}", t.file);
        }
        println!("deleted {} unused take(s), kept {}", unused.len(), takes.len() - unused.len());
        return Ok(());
    }
    let map = project.bar_map();
    println!("{} take(s) (recordings/takes.json):", takes.len());
    for t in &takes {
        let (bar, beat) = map.bar_at(t.start);
        let file = dir(path).join(Path::new(&t.file).file_name().unwrap_or_default());
        let (voice, rms) = voice_span(&file).unwrap_or((None, f64::NEG_INFINITY));
        let span = voice.map_or("(almost silent)".to_string(), |(a, z)| format!("voiced {a:.2}-{z:.2} s"));
        println!(
            "  {}  track {}  from bar {} beat {}  {:.2} s  avg {:.0} dB  {}{}",
            t.file,
            t.track,
            bar + 1,
            (beat * 100.0).round() / 100.0 + 1.0,
            t.seconds,
            rms,
            span,
            if referenced(&project, &t.file) { "  (used by the project)" } else { "" }
        );
    }
    println!("To trim leading/trailing silence: edit the clip's audio.offset (seconds) and start / length (beats); use fade_in / fade_out for the seams.");
    Ok(())
}

/// `--timing`：工程里的人声（没静音的音频轨）逐句和引导旋律（带 `lyric` 的轨）比节奏。
/// 每句取录音的音高曲线（逐 10 毫秒的基频），在 −300 到 +400 毫秒之间找和引导旋律最吻合的时间差（比相对音高，八度不同也行）。
fn report_timing(p: &Project) -> Result<()> {
    let rows = line_lags(p)?;
    let mut lags = Vec::new();
    println!("Per-line timing (positive = later than the guide melody, negative = earlier; fit is the average difference in semitones, lower is more reliable):");
    for (k, (text, bar, best)) in rows.iter().enumerate() {
        match best {
            Some((lag, err)) => {
                let mark = if *err > 1.5 { "  (poor fit, unreliable)" } else if *lag > 0.04 { "  <- late" } else if *lag < -0.04 { "  <- early" } else { "" };
                println!("  {:>2}. bar {:>2} {text}: {:+4.0} ms ({err:.2}){mark}", k + 1, bar + 1, lag * 1000.0);
                if *err <= 1.5 {
                    lags.push(*lag);
                }
            }
            None => println!("  {:>2}. bar {:>2} {text}: no recording for this line (or no pitch detected)", k + 1, bar + 1),
        }
    }
    if !lags.is_empty() {
        lags.sort_by(f64::total_cmp);
        let med = lags[lags.len() / 2];
        let same = lags.iter().filter(|l| (*l - med).abs() < 0.04).count() * 4 >= lags.len() * 3;
        println!(
            "Overall median {:+.0} ms. {}",
            med * 1000.0,
            if same && med.abs() > 0.03 {
                format!("Most lines are off by about this much: likely recording latency; enter {:.0} in the recording panel's Latency (ms) field and record again.", med * 1000.0)
            } else {
                "Lines vary early and late, which is normal in singing; to shift one line use muser clip split / move (or split and nudge with the arrows in view).".into()
            }
        );
    }
    Ok(())
}

/// 每一句：歌词、第几小节（从 0 起）、（时间差秒, 吻合度）。
fn line_lags(p: &Project) -> Result<Vec<(String, usize, Option<(f64, f64)>)>> {
    let guide = muser_engine::lyric::guide_notes(p);
    let lines = muser_engine::lyric::karaoke(p);
    if guide.is_empty() || lines.is_empty() {
        bail!("no guide melody: need a track whose notes carry lyric (or an ABC w: lyrics line) to compare against");
    }
    let tm = p.tempo_map();
    let base = p.base_dir.clone().unwrap_or_default();
    // 人声：没静音的音频轨上的片段，(起点秒, 终点秒, 文件里从第几秒, 文件)。
    let clips: Vec<(f64, f64, f64, String)> = p
        .tracks
        .iter()
        .filter(|t| t.instrument.trim() == muser_core::audio::INSTRUMENT && !t.muted)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| c.audio.as_ref().map(|a| (tm.secs_at(c.start.0), tm.secs_at(c.start.0 + c.length.0), a.offset, a.file.clone())))
        .collect();
    if clips.is_empty() {
        bail!("the project has no (unmuted) audio track to check");
    }
    let mut files: std::collections::HashMap<String, Option<(Vec<f32>, f64)>> = Default::default();
    let map = p.bar_map();
    let mut rows = Vec::new();
    for line in &lines {
        let text: String = line.tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(if line.tokens.iter().all(|t| t.text.chars().count() == 1) { "" } else { " " });
        let (a, z) = (line.start - 0.3, line.end + 0.3);
        // 这一句的人声音高（歌曲时间 → 片段 → 文件）。
        let mut times = Vec::new();
        let mut midi = Vec::new();
        let mut t = a.max(0.0);
        while t < z {
            let pitch = clips.iter().filter(|c| t >= c.0 && t < c.1).find_map(|c| {
                let entry = files.entry(c.3.clone()).or_insert_with(|| load_mono(&base.join(&c.3)));
                let (x, sr) = entry.as_ref()?;
                yin(x, *sr, t - c.0 + c.2 - 0.02) // 40 毫秒的窗以 t 为中心
            });
            times.push(t);
            midi.push(pitch);
            t += 0.01;
        }
        let midi = smooth(&midi);
        let notes: Vec<&(f64, f64, u8)> = guide.iter().filter(|n| n.0 >= line.start - 1e-6 && n.0 < line.end).collect();
        let mut best: Option<(f64, f64)> = None;
        for step in -60..=80 {
            let lag = f64::from(step) * 0.005;
            let mut d: Vec<f64> = Vec::new();
            for (tt, m) in times.iter().zip(&midi) {
                let (Some(m), Some(n)) = (m, notes.iter().find(|n| tt - lag >= n.0 && tt - lag < n.1)) else { continue };
                d.push(m - f64::from(n.2));
            }
            if d.len() < 30 {
                continue;
            }
            let mut sorted = d.clone();
            sorted.sort_by(f64::total_cmp);
            let off = sorted[sorted.len() / 2].round();
            // 音高检测偶尔差一个八度：按八度折回（差 12 个半音算没差）。
            let err = d.iter().map(|x| ((x - off + 6.0).rem_euclid(12.0) - 6.0).abs().min(3.0)).sum::<f64>() / d.len() as f64;
            if best.is_none_or(|b| err < b.1) {
                best = Some((lag, err));
            }
        }
        rows.push((text, map.bar_at(tm.beat_at(line.start)).0, best));
    }
    Ok(rows)
}

/// 读一个音频文件，混成单声道，降到约 16 kHz（只为测音高）。
fn load_mono(path: &Path) -> Option<(Vec<f32>, f64)> {
    let w = muser_engine::wav::read_file(path).ok()?;
    let (n, ch) = (w.buffer.num_frames(), w.buffer.num_channels().max(1));
    let step = (w.sample_rate / 16_000).max(1) as usize;
    let x: Vec<f32> = (0..n / step)
        .map(|i| (0..step).map(|k| (0..ch).map(|c| w.buffer.channel(c)[i * step + k]).sum::<f32>() / ch as f32).sum::<f32>() / step as f32)
        .collect();
    Some((x, f64::from(w.sample_rate) / step as f64))
}

/// YIN 基频（文件第 `at` 秒处 40 毫秒的窗），转成 MIDI 音高；太轻或没有音高时为 None。
fn yin(x: &[f32], sr: f64, at: f64) -> Option<f64> {
    let win = (sr * 0.04) as usize;
    let i = (at * sr) as isize;
    if i < 0 || i as usize + win >= x.len() {
        return None;
    }
    let fr = &x[i as usize..i as usize + win];
    let mean = fr.iter().sum::<f32>() / win as f32;
    if (fr.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / win as f32).sqrt() < 0.004 {
        return None;
    }
    let (min_lag, max_lag) = ((sr / 900.0) as usize, ((sr / 70.0) as usize).min(win - 1));
    let mut cum = 0.0f32;
    for lag in 1..max_lag {
        let d: f32 = (0..win - lag).map(|k| (fr[k] - fr[k + lag]).powi(2)).sum();
        cum += d;
        if lag >= min_lag && d * lag as f32 / cum.max(1e-9) < 0.15 {
            return Some(69.0 + 12.0 * (sr / lag as f64 / 440.0).log2());
        }
    }
    None
}

/// 5 点中值（至少 3 个有音高才算）。
fn smooth(m: &[Option<f64>]) -> Vec<Option<f64>> {
    (0..m.len())
        .map(|i| {
            let mut v: Vec<f64> = m[i.saturating_sub(2)..(i + 3).min(m.len())].iter().flatten().copied().collect();
            (v.len() >= 3).then(|| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            })
        })
        .collect()
}

/// 实际唱的部分（秒，按 −45 dB 判断有没有声音）与整遍的平均电平（dB）。
fn voice_span(file: &Path) -> Option<(Option<(f64, f64)>, f64)> {
    let w = muser_engine::wav::read_file(file).ok()?;
    let x = w.buffer.channel(0);
    let sr = f64::from(w.sample_rate);
    let hop = (sr * 0.02) as usize;
    let mut on = Vec::new();
    let mut total = 0.0f64;
    for (k, c) in x.chunks(hop.max(1)).enumerate() {
        let ms = c.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / c.len() as f64;
        total += ms * c.len() as f64;
        if 10.0 * ms.max(1e-12).log10() > -45.0 {
            on.push(k);
        }
    }
    let rms = 10.0 * (total / x.len().max(1) as f64).max(1e-12).log10();
    let span = on.first().zip(on.last()).map(|(a, z)| (*a as f64 * hop as f64 / sr, (*z + 1) as f64 * hop as f64 / sr));
    Some((span, rms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(seconds: f64) -> Vec<u8> {
        let n = (48_000.0 * seconds) as usize;
        let mut b = muser_engine::AudioBuffer::<f32>::new(1, n);
        for (i, x) in b.channel_mut(0).iter_mut().enumerate() {
            *x = if i > n / 4 { (i as f32 * 0.05).sin() * 0.3 } else { 0.0 };
        }
        let mut out = Vec::new();
        muser_engine::wav::write(&mut out, &b, 48_000, muser_engine::WavFormat::Int16).unwrap();
        out
    }

    #[test]
    fn timing_finds_a_known_delay() {
        // 引导旋律两句，每句两个音；"录音"是同样音高的正弦，整体晚 80 毫秒。
        let d = std::env::temp_dir().join(format!("muser-timing-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let notes = [(0.0, 1.0, 57u8), (1.0, 2.0, 62), (4.0, 5.0, 64), (5.0, 6.0, 60)];
        let sr = 48_000usize;
        let mut b = muser_engine::AudioBuffer::<f32>::new(1, sr * 7);
        let mut phase = 0.0f64;
        for (i, x) in b.channel_mut(0).iter_mut().enumerate() {
            let t = i as f64 / sr as f64 - 0.08;
            if let Some(n) = notes.iter().find(|n| t >= n.0 && t < n.1) {
                phase += 440.0 * 2f64.powf((f64::from(n.2) - 69.0) / 12.0) / sr as f64;
                *x = (phase * std::f64::consts::TAU).sin() as f32 * 0.3;
            }
        }
        let mut wav = Vec::new();
        muser_engine::wav::write(&mut wav, &b, 48_000, muser_engine::WavFormat::Int16).unwrap();
        std::fs::write(d.join("v.wav"), wav).unwrap();
        let p = Project::from_json(r#"{ "tempo": 60, "lyrics": "啊哦\n咿呜", "tracks": [
            { "name": "guide", "instrument": "choir.solo", "muted": true, "clips": [{ "start": 0, "length": 8, "abc": "L:1/4\nK:C\nA d z2 | e c z2 |\nw: a o | yi wu" }] },
            { "name": "vocal", "instrument": "audio", "clips": [{ "start": 0, "length": 7, "audio": { "file": "v.wav" } }] } ] }"#)
            .unwrap()
            .at_file(d.join("song.muser.json"));
        let rows = line_lags(&p).unwrap();
        assert_eq!(rows.len(), 2);
        for (text, _, best) in &rows {
            let (lag, err) = best.unwrap();
            assert!((lag - 0.08).abs() <= 0.02 && err < 0.5, "{text}: {lag} {err}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn store_place_and_save() {
        let d = std::env::temp_dir().join(format!("muser-takes-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("song.muser.json");
        std::fs::write(&path, r#"{ "tempo": 120, "tracks": [{ "name": "keys", "instrument": "poly.juno_keys", "clips": [] }] }"#).unwrap();
        let a = store(&path, "vocal", 4.0, &wav(2.0)).unwrap();
        let b = store(&path, "vo/../cal", 4.0, &wav(1.0)).unwrap();
        assert_eq!(a.file, "recordings/vocal-take-01.wav");
        assert_eq!(b.file, "recordings/vocal-take-02.wav", "轨名里的 / 和 .. 去掉，文件只会在 recordings/ 里");
        assert_eq!(list(&path).len(), 2);
        // 贴到工作副本：新建音频轨；第二遍和第一遍重叠，换掉它。
        let mut p = Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        place(&mut p, &a).unwrap();
        place(&mut p, &Take { track: "vocal".into(), ..b.clone() }).unwrap();
        let v = &p.tracks[1];
        assert_eq!((v.name.as_str(), v.instrument.as_str(), v.clips.len()), ("vocal", "audio", 1));
        assert_eq!(v.clips[0].audio.as_ref().unwrap().file, "recordings/vocal-take-02.wav");
        assert_eq!(v.clips[0].length.0, 2.0, "1 秒 @ 120 BPM = 2 拍");
        // 写进 JSON 一样。
        let mut j: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        place_json(&mut j, &a, &muser_core::tempo::TempoMap::constant(120.0)).unwrap();
        assert_eq!(j["tracks"][1]["clips"][0]["audio"]["file"], json!("recordings/vocal-take-01.wav"));
        assert!(delete(&path, &a.file, true).is_err(), "a take in use is not deleted");
        delete(&path, &a.file, false).unwrap();
        assert_eq!(list(&path).len(), 1);
        assert!(store(&path, "vocal", 0.0, b"not a wav").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
