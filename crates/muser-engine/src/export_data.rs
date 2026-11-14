//! `muser export-data`：给可视化用的一份 JSON——每轨的音符（秒）、段落、自动化曲线，可选每帧的音量包络。
//!
//! 《电子羊》四个视频都是手写脚本解析 MIDI、手抄自动化曲线、自己算每帧电平做出来的；有了这个，agent 给音乐配画面
//! 只需要读一个文件。音符与渲染一致（与 MIDI 导出同一份调度：humanize、连奏、abc / grid / drummer 展开都在内）。

use muser_core::automation::Shape;
use muser_core::{Automation, Project};
use serde_json::{json, Value};

use crate::render::{schedule_ticks, Act, EngineError, RenderSettings};

const TPQ: u16 = 960;

/// `fps` 给了就渲染一遍，加上每帧的音量包络。
pub fn export(project: &Project, fps: Option<u32>, settings: RenderSettings) -> Result<Value, EngineError> {
    crate::validate_score(project)?;
    let invalid = |e: String| EngineError::Core(muser_core::CoreError::Invalid(e));
    let sections = crate::arrange::timeline(project).map_err(invalid)?;
    let arranged = crate::arrange::arranged(project).map_err(invalid)?;
    let p = &*arranged;
    let spb = 60.0 / p.tempo;
    let tm = p.tempo_map();
    let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
    let beats = crate::automate::total_beats(p, 0.0);

    let mut tracks = Vec::new();
    for (ti, t) in p.tracks.iter().enumerate() {
        let audio = t.instrument.trim() == muser_core::audio::INSTRUMENT;
        let mut notes = Vec::new();
        if !audio {
            let mut on: std::collections::HashMap<(u8, Option<u8>), Vec<(usize, f32)>> = Default::default();
            for (tick, act) in schedule_ticks(p, ti, TPQ) {
                match act {
                    Act::On { key, velocity, ch, .. } => on.entry((key, ch)).or_default().push((tick, velocity)),
                    Act::Off { key, ch } => {
                        if let Some((start, vel)) = on.get_mut(&(key, ch)).and_then(|v| (!v.is_empty()).then(|| v.remove(0))) {
                            let sec = |k: usize| tm.secs_at(k as f64 / f64::from(TPQ));
                            notes.push(json!([r3(sec(start)), key, (vel * 127.0).round() as u8, r3(sec(tick) - sec(start))]));
                        }
                    }
                    _ => {}
                }
            }
            notes.sort_by(|a, b| a[0].as_f64().partial_cmp(&b[0].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
        }
        tracks.push(json!({
            "name": t.name, "instrument": t.instrument, "drums": muser_core::is_drum_kit(&t.instrument),
            "muted": t.muted, "gain_db": t.gain_db, "notes": notes,
            "automation": curves(&t.automation, beats, &tm),
        }));
    }

    let mut out = json!({
        "name": p.name, "tempo": p.tempo, "seconds_per_beat": spb,
        "time_signature": [p.time_signature.numerator, p.time_signature.denominator],
        "duration": r3(tm.secs_at(beats)),
        "sections": sections.iter().map(|s| {
            let ts = p.sections.get(&s.name).map_or(p.time_signature, |x| p.section_meter(x));
            json!({
                "name": s.name, "occurrence": s.occurrence,
                "start": r3(tm.secs_at(s.start)), "end": r3(tm.secs_at(s.start + s.length)),
                "bars": (s.length / ts.beats_per_bar()).round(), "time_signature": [ts.numerator, ts.denominator],
            })
        }).collect::<Vec<_>>(),
        "tracks": tracks,
        "master": { "gain_db": p.master_gain_db, "automation": curves(&p.automation, beats, &tm) },
        "notes_format": "[start_seconds, midi_pitch, velocity_0_127, duration_seconds]",
    });
    // 给画面卡节奏：每一小节、每一拍在第几秒（速度、拍号会变时照样准）。
    let map = p.bar_map();
    let n_bars = map.bars_until(beats);
    out["bars"] = json!((0..=n_bars).map(|b| r3(tm.secs_at(map.bar_start(b)))).collect::<Vec<_>>());
    out["beats"] = json!((0..=beats.floor() as usize).map(|b| r3(tm.secs_at(b as f64))).collect::<Vec<_>>());
    // 歌词：每一句、每个字唱在第几秒（和 muser view 的卡拉 OK 一样，按引导旋律）。没有带歌词的音轨时为空。
    out["lyrics"] = json!(crate::lyric::karaoke(project));
    if let Some(fps) = fps.filter(|&f| f > 0) {
        let audio = crate::render::render(project, settings)?;
        out["envelope"] = envelope(&audio, settings.sample_rate, fps);
        out["duration"] = json!(r3(audio.num_frames() as f64 / f64::from(settings.sample_rate)));
    }
    Ok(out)
}

/// 每条自动化在 1/4 拍的格点上取值：`[[秒, 值], …]`。值不是数字（如元音名）的点曲线不导出。
fn curves(autos: &[Automation], beats: f64, tm: &muser_core::tempo::TempoMap) -> Vec<Value> {
    autos
        .iter()
        .filter_map(|a| {
            let values: Vec<f64> = match &a.shape {
                Shape::Points { points, .. } => points.iter().map(|(_, v)| v.as_f64()).collect::<Option<_>>()?,
                _ => Vec::new(),
            };
            let curve = a.compile(&values, 0, beats);
            let n = ((beats / 0.25).ceil() as usize).min(200_000);
            let pts: Vec<Value> = (0..=n)
                .filter_map(|k| {
                    let b = k as f64 * 0.25;
                    curve.value_at(b).map(|v| json!([(tm.secs_at(b) * 1000.0).round() / 1000.0, (v * 1000.0).round() / 1000.0]))
                })
                .collect();
            let mut o = json!({ "target": a.target, "points": pts });
            if !a.param.is_empty() {
                o["param"] = json!(a.param);
            }
            Some(o)
        })
        .collect()
}

/// 每帧的整体电平与 150 Hz 以下的电平（dBFS，RMS，两个声道平均；静音为 −120）。
fn envelope(audio: &manguebyte::prelude::AudioBuffer<f32>, sr: u32, fps: u32) -> Value {
    let hop = (sr / fps).max(1) as usize;
    let n = audio.num_frames();
    let chans = audio.num_channels().max(1);
    let a = 1.0 - (-2.0 * std::f64::consts::PI * 150.0 / f64::from(sr)).exp();
    let mut lp = vec![0.0f64; chans];
    let (mut level, mut low) = (Vec::new(), Vec::new());
    let db = |ms: f64| if ms > 1e-12 { ((10.0 * ms.log10()) * 10.0).round() / 10.0 } else { -120.0 };
    let mut start = 0;
    while start < n {
        let end = (start + hop).min(n);
        let (mut s, mut l) = (0.0f64, 0.0f64);
        for (ch, y) in lp.iter_mut().enumerate() {
            for &v in &audio.channel(ch)[start..end] {
                let v = f64::from(v);
                *y += a * (v - *y);
                s += v * v;
                l += *y * *y;
            }
        }
        let count = ((end - start) * chans) as f64;
        level.push(db(s / count));
        low.push(db(l / count));
        start = end;
    }
    json!({ "fps": fps, "level_db": level, "low_db": low })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_sections_curves_and_envelope() {
        let p: Project = serde_json::from_value(json!({ "name": "viz", "tempo": 120,
            "sections": { "a": { "bars": 1 }, "b": { "bars": 1 } }, "form": ["a", "b"],
            "automation": [{ "target": "master_gain_db", "shape": "points", "points": [[0, 0.0], [8, -12.0]] }],
            "tracks": [{ "name": "k", "instrument": "drums.909", "parts": { "*": { "grid": { "kick": "x..." } } } }] }))
        .unwrap();
        let d = export(&p, Some(10), RenderSettings::default()).unwrap();
        assert_eq!(d["sections"][1]["start"], json!(2.0));
        let notes = d["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 8, "每小节 4 下，两小节");
        assert_eq!(notes[1][0], json!(0.5));
        assert_eq!(notes[0][1], json!(36));
        let m = d["master"]["automation"][0]["points"].as_array().unwrap();
        assert_eq!(m.last().unwrap()[1], json!(-12.0));
        assert_eq!(m[8], json!([1.0, -3.0]), "1 秒 = 2 拍，0→−12 dB 的四分之一");
        let env = d["envelope"]["level_db"].as_array().unwrap();
        assert!(env.len() >= 40 && env[0].as_f64().unwrap() > -60.0);
    }
}
