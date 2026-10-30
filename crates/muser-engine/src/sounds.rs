//! 音色目录与"听"音色：本机所有能用的音色（内置乐器、`$MUSER_SFZ_PATH` 里的 SFZ、扫描到的乐器插件），
//! 以及给一个音色渲染探测音、测出听感特征、写成文字卡片（缓存在本机）。
//!
//! 卡片是测量换来的描述（亮度、起音、延音、音域、力度、立体声、音高），帮 agent 粗筛与比较，不是真正的听感。
//! 探测音在一次渲染里依次排好：音阶乐器从低到高 5 个音区各一个音、再一个轻力度的音；鼓组 6 个常用鼓键与一下轻军鼓。
//! 加载很慢的音源（大采样库、Kontakt）只加载一次。

use std::path::PathBuf;

use manguebyte::analysis::{PitchDetector, PitchMethod, PitchSettings};
use manguebyte::prelude::{AudioBuffer, ProcessSpec};
use muser_core::{Beats, Clip, Note, Project, Track};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audition::{analyze, Analysis};
use crate::{render, RenderSettings};

/// 探测方法有变化时加一，旧缓存作废。
const PROBE_VERSION: u32 = 2;

/// 目录里的一个音色。
#[derive(Debug, Clone, Serialize)]
pub struct Sound {
    /// 写进音轨 `instrument` 的值。
    pub spec: String,
    pub name: String,
    /// builtin / sfz / plugin。
    pub kind: String,
    /// 类别（内置乐器的 family）、音色库目录（SFZ）或厂商（插件）。
    pub group: String,
    pub description: String,
    /// 本机已有卡片时的一句话描述。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
}

/// 本机所有能用的音色。插件需要 `host` 特性（读扫描缓存，第一次会扫描）。
pub fn list() -> Vec<Sound> {
    let mut out: Vec<Sound> = crate::instrument::catalog()
        .into_iter()
        .map(|i| Sound { spec: i.name.clone(), name: i.name, kind: "builtin".into(), group: i.family, description: i.description, card: None })
        .collect();
    out.extend(crate::sfz::library().into_iter().map(|(rel, path)| Sound {
        spec: format!("sfz:{rel}"),
        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| rel.clone()),
        kind: "sfz".into(),
        group: rel.split('/').next().filter(|g| *g != rel).unwrap_or("").to_string(),
        description: String::new(),
        card: None,
    }));
    if let Ok((plugins, _)) = crate::plugin::scan(false) {
        out.extend(plugins.into_iter().filter(|p| p.kind == "instrument").map(|p| Sound {
            spec: format!("plugin:{}#{}", p.path, p.id),
            name: p.name,
            kind: "plugin".into(),
            group: p.vendor,
            description: format!("{} 乐器插件（音色由插件自己的状态决定：用 @state，或请人在 muser view 的 🎛 里选）", p.format.to_uppercase()),
            card: None,
        }));
    }
    for s in &mut out {
        s.card = cached(&s.spec, &serde_json::Map::new()).map(|c| c.summary);
    }
    out
}

/// 一个探测音的测量结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub pitch: u8,
    pub velocity: f32,
    /// 有没有声音（峰值高于 -60 dBFS）。
    pub audible: bool,
    #[serde(flatten)]
    pub analysis: Option<AnalysisJson>,
    /// 按住到松开前一刻的电平（相对峰值，dB）：接近 0 是持续发声（风琴、弦乐、铺底），很低是会自己衰减（钢琴、拨弦、打击）。
    pub hold_db: Option<f64>,
    /// 立体声宽度：侧信号相对中间信号的能量（%）。
    pub width_pct: Option<f64>,
    /// 测到的音高（MIDI，带小数）；没有明确音高时为空（打击、噪声）。
    pub detected_pitch: Option<f64>,
}

/// [`Analysis`] 的可反序列化副本（缓存用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisJson {
    pub peak_db: f64,
    pub lufs: f64,
    pub centroid_hz: f64,
    pub low_pct: f64,
    pub mid_pct: f64,
    pub high_pct: f64,
    pub attack_ms: f64,
    pub release_s: f64,
}

impl From<Analysis> for AnalysisJson {
    fn from(a: Analysis) -> Self {
        Self { peak_db: a.peak_db, lufs: a.lufs, centroid_hz: a.centroid_hz, low_pct: a.low_pct, mid_pct: a.mid_pct, high_pct: a.high_pct, attack_ms: a.attack_ms, release_s: a.release_s }
    }
}

/// 一个音色的声音卡片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub spec: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, Value>,
    /// 按鼓组探测（鼓键）还是按音阶乐器探测。
    pub drums: bool,
    pub probes: Vec<Probe>,
    /// 一句话描述。
    pub summary: String,
    /// 逐项描述。
    pub traits: Vec<String>,
    pub probe_version: u32,
}

fn cache_file(spec: &str, params: &serde_json::Map<String, Value>) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // 音色文件变了（换了 .sfz、重存了 @state）卡片作废：计入修改时间。
    let file = spec.strip_prefix("sfz:").map(crate::sfz::resolve).or_else(|| params.get("@state").and_then(Value::as_str).map(|p| PathBuf::from(&*crate::plugin::expand_home(p))));
    let mtime = file.and_then(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).map(|t| format!("{t:?}"));
    (spec, Value::Object(params.clone()).to_string(), mtime, PROBE_VERSION).hash(&mut h);
    crate::plugin::cache_dir().join("sounds").join(format!("{:016x}.json", h.finish()))
}

/// 本机缓存里的卡片。
pub fn cached(spec: &str, params: &serde_json::Map<String, Value>) -> Option<Card> {
    let text = std::fs::read_to_string(cache_file(spec, params)).ok()?;
    serde_json::from_str::<Card>(&text).ok().filter(|c| c.probe_version == PROBE_VERSION)
}

/// 鼓组的探测鼓键：底鼓、军鼓、闭镲、开镲、强音镲、叮叮镲。
const DRUM_KEYS: [u8; 6] = [36, 38, 42, 46, 49, 51];
const PITCHES: [u8; 5] = [36, 48, 60, 72, 84];

/// 听一个音色：渲染探测音并测量，写进本机缓存。`drums` 为空时自动判断（内置鼓组、名字里有 drum / kit / perc）。
pub fn describe(spec: &str, params: &serde_json::Map<String, Value>, drums: Option<bool>, refresh: bool) -> Result<Card, String> {
    if !refresh {
        if let Some(c) = cached(spec, params) {
            return Ok(c);
        }
    }
    let drums = drums.unwrap_or_else(|| {
        let builtin = crate::instrument::catalog().into_iter().find(|i| i.name == spec);
        match builtin {
            Some(i) => matches!(i.notes, crate::instrument::NoteConvention::GmDrums),
            None => {
                let l = spec.to_lowercase();
                ["drum", "kit", "perc"].iter().any(|w| l.contains(w))
            }
        }
    });
    // 每个探测音占一格：音阶乐器 4 秒（按住 1 秒、余音 3 秒），鼓 3 秒（敲 0.25 秒）。tempo 60：1 拍 = 1 秒。
    let (slot, hold) = if drums { (3.0, 0.25) } else { (4.0, 1.0) };
    let mut probes: Vec<(u8, f32)> = if drums { DRUM_KEYS.iter().map(|&k| (k, 0.8)).collect() } else { PITCHES.iter().map(|&p| (p, 0.8)).collect() };
    probes.push(if drums { (38, 0.3) } else { (60, 0.3) });
    let notes: Vec<Note> = probes
        .iter()
        .enumerate()
        .map(|(i, &(pitch, velocity))| Note { velocity, ..Note::new(pitch, Beats(i as f64 * slot), Beats(hold)) })
        .collect();
    let mut track = Track::new("probe", spec);
    track.instrument_params = params.clone();
    let mut clip = Clip::new(0.0, probes.len() as f64 * slot);
    clip.notes = notes;
    track.clips.push(clip);
    let mut project = Project::new("probe", 60.0);
    project.tracks.push(track);
    crate::validate(&project).map_err(|e| e.to_string())?;
    let settings = RenderSettings { tail_seconds: 0.0, ..RenderSettings::default() };
    let sr = f64::from(settings.sample_rate);
    let audio = render(&project, settings).map_err(|e| e.to_string())?;

    let seg_frames = (slot * sr) as usize;
    let hold_frames = (hold * sr) as usize;
    let mut measured = Vec::new();
    for (i, &(pitch, velocity)) in probes.iter().enumerate() {
        let a = (i * seg_frames).min(audio.num_frames());
        let z = (a + seg_frames).min(audio.num_frames());
        let mut seg = AudioBuffer::<f32>::new(2, (z - a).max(1));
        for ch in 0..2 {
            seg.channel_mut(ch)[..z - a].copy_from_slice(&audio.channel(ch)[a..z]);
        }
        let peak = seg.peak();
        if peak < 0.001 {
            measured.push(Probe { pitch, velocity, audible: false, analysis: None, hold_db: None, width_pct: None, detected_pitch: None });
            continue;
        }
        let mut an = analyze(&seg, sr, (0, hold_frames.min(seg.num_frames())));
        if drums {
            // 鼓从击打就开始衰减：余音按峰值之后衰减 40 dB 的时间算，而不是从松键算。
            let env: Vec<f32> = (0..seg.num_frames()).map(|k| seg.channel(0)[k].abs().max(seg.channel(1)[k].abs())).collect();
            let at = env.iter().enumerate().fold((0, 0f32), |m, (k, &e)| if e > m.1 { (k, e) } else { m }).0;
            let last = env.iter().rposition(|&e| e >= peak * 0.01).unwrap_or(at);
            an.release_s = ((last.saturating_sub(at)) as f64 / sr * 100.0).round() / 100.0;
        }
        let (l, r) = (seg.channel(0), seg.channel(1));
        let (mut mid, mut side) = (0f64, 0f64);
        for k in 0..seg.num_frames() {
            mid += f64::from(l[k] + r[k]).powi(2);
            side += f64::from(l[k] - r[k]).powi(2);
        }
        // 松开前 50 ms 的电平相对整段峰值。
        let w = (0.05 * sr) as usize;
        let end = hold_frames.min(seg.num_frames());
        let tail = (end.saturating_sub(w)..end).map(|k| l[k].abs().max(r[k].abs())).fold(0f32, f32::max);
        let hold_db = 20.0 * (f64::from(tail.max(1e-9)) / f64::from(peak)).log10();
        measured.push(Probe {
            pitch,
            velocity,
            audible: true,
            analysis: Some(an.into()),
            hold_db: Some((hold_db * 10.0).round() / 10.0),
            width_pct: Some((100.0 * side / mid.max(1e-12) * 10.0).round() / 10.0),
            detected_pitch: if drums { None } else { detect_pitch(&seg, sr, hold_frames) },
        });
    }
    let (summary, traits) = words(&measured, drums);
    let card = Card { spec: spec.to_string(), params: params.clone(), drums, probes: measured, summary, traits, probe_version: PROBE_VERSION };
    let path = cache_file(spec, params);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&card).unwrap_or_default());
    Ok(card)
}

/// 前 `hold` 帧里可信的音高的中位数（MIDI）。
fn detect_pitch(seg: &AudioBuffer<f32>, sr: f64, hold: usize) -> Option<f64> {
    let n = hold.min(seg.num_frames());
    let mono: Vec<f32> = (0..n).map(|k| 0.5 * (seg.channel(0)[k] + seg.channel(1)[k])).collect();
    let hop = (sr * 0.01) as usize;
    let mut det = PitchDetector::new(PitchSettings::new(PitchMethod::Mpm { cutoff: 0.93 }).with_range(30.0, 2500.0).with_hop(hop));
    det.prepare(&ProcessSpec { sample_rate: sr, max_block_size: hop, num_channels: 1 });
    let mut found: Vec<f64> = Vec::new();
    for chunk in mono.chunks(hop) {
        det.measure_samples(chunk);
        let p = det.latest();
        if !p.is_none() && p.confidence >= 0.85 {
            found.push(69.0 + 12.0 * (p.frequency / 440.0).log2());
        }
    }
    // 至少一半的时间测得到音高才算有明确音高。
    if found.len() * 2 < mono.len() / hop {
        return None;
    }
    found.sort_by(f64::total_cmp);
    Some((found[found.len() / 2] * 10.0).round() / 10.0)
}

fn drum_name(k: u8) -> &'static str {
    match k {
        36 => "底鼓 36",
        38 => "军鼓 38",
        42 => "闭镲 42",
        46 => "开镲 46",
        49 => "强音镲 49",
        51 => "叮叮镲 51",
        _ => "鼓键",
    }
}

/// 把测量换成文字。
fn words(probes: &[Probe], drums: bool) -> (String, Vec<String>) {
    let mut traits = Vec::new();
    let loud: Vec<&Probe> = probes.iter().filter(|p| p.audible && p.velocity > 0.5).collect();
    if loud.is_empty() {
        return ("没有声音：探测音都没有发声（音色没载入，或音域 / 鼓键不对）".into(), vec!["所有探测音峰值都低于 -60 dBFS".into()]);
    }
    if drums {
        for p in probes.iter().filter(|p| p.velocity > 0.5) {
            match &p.analysis {
                Some(a) => traits.push(format!("{}：衰减 40 dB 用 {:.2} s，重心 {:.0} Hz，响度 {:.1} LUFS", drum_name(p.pitch), a.release_s, a.centroid_hz, a.lufs)),
                None => traits.push(format!("{}：不发声", drum_name(p.pitch))),
            }
        }
        let silent = probes.iter().filter(|p| p.velocity > 0.5 && !p.audible).count();
        let soft = probes.iter().find(|p| p.velocity < 0.5);
        let snare = probes.iter().find(|p| p.pitch == 38 && p.velocity > 0.5);
        if let (Some(s), Some(n)) = (soft.and_then(|p| p.analysis.as_ref()), snare.and_then(|p| p.analysis.as_ref())) {
            traits.push(format!("力度：轻敲比重敲低 {:.1} dB", n.lufs - s.lufs));
        }
        let kick = probes.iter().find(|p| p.pitch == 36).and_then(|p| p.analysis.as_ref());
        let mut summary = String::from("鼓组");
        if let Some(k) = kick {
            summary += &format!("，底鼓{}", if k.release_s > 0.6 { "长、带低频尾巴（808 类）" } else if k.centroid_hz < 150.0 { "沉、短" } else { "紧、短" });
        }
        if silent > 0 {
            summary += &format!("，{silent} 个常用鼓键不发声");
        }
        return (summary, traits);
    }
    // 音阶乐器。
    let range: Vec<u8> = probes.iter().filter(|p| p.audible && p.velocity > 0.5).map(|p| p.pitch).collect();
    let name = |p: u8| ["C1", "C2", "C3", "C4", "C5", "C6", "C7"].get(usize::from(p / 12).saturating_sub(2)).copied().unwrap_or("?");
    traits.push(format!("可用音域：探测的 C2–C6 里 {} 发声", range.iter().map(|&p| name(p)).collect::<Vec<_>>().join(" ")));
    let mid = probes.iter().find(|p| p.pitch == 60 && p.velocity > 0.5 && p.audible).or(loud.get(loud.len() / 2).copied());
    let mut summary = Vec::new();
    if let Some(m) = mid {
        let a = m.analysis.as_ref().expect("有声的探测音都有分析");
        // 亮度看重心相对基音（C4 ≈ 262 Hz）的倍数，不同音区可比。
        let ratio = a.centroid_hz / 261.6;
        let bright = match ratio {
            r if r < 2.5 => "很暗、很圆",
            r if r < 5.0 => "偏暗、温暖",
            r if r < 10.0 => "明暗适中",
            r if r < 18.0 => "明亮",
            _ => "很亮、偏尖",
        };
        traits.push(format!("亮度：C4 的频谱重心 {:.0} Hz（基音的 {:.1} 倍）→ {bright}", a.centroid_hz, ratio));
        let attack = match a.attack_ms {
            t if t < 15.0 => "起音瞬间（打击、拨奏感）",
            t if t < 60.0 => "起音快",
            t if t < 250.0 => "起音柔和",
            _ => "起音慢（渐入、铺底型）",
        };
        traits.push(format!("起音：{:.0} ms → {attack}", a.attack_ms));
        let hold = m.hold_db.unwrap_or(-99.0);
        let body = if hold > -6.0 {
            "按住就一直响（持续型：风琴、弦乐、管乐、铺底）".to_string()
        } else if hold > -20.0 {
            format!("按住时慢慢变弱（按住 1 秒后低 {:.0} dB）", -hold)
        } else {
            format!("按下后很快衰减（按住 1 秒后已低 {:.0} dB：拨奏、打击、钢琴类）", -hold)
        };
        traits.push(format!("延音：{body}"));
        let release = if a.release_s < 0.15 { "松开即停" } else if a.release_s < 1.0 { "松开后短余音" } else { "松开后长余音" };
        traits.push(format!("余音：松开后 {:.2} s 衰减 40 dB → {release}", a.release_s));
        let width = m.width_pct.unwrap_or(0.0);
        let stereo = if width < 3.0 {
            "单声道 / 居中"
        } else if width < 25.0 {
            "略有立体声"
        } else if width <= 100.0 {
            "很宽的立体声"
        } else {
            "侧信号比中间还强：非常宽，合成单声道时会明显变薄"
        };
        traits.push(format!("立体声：侧 / 中能量 {width:.0}% → {stereo}"));
        match m.detected_pitch {
            Some(d) if (d - 60.0).abs() > 6.0 => traits.push(format!("音高：弹 C4 测到 {d:.1}（差 {:+.0} 个半音：音色的音高映射和标注不一致）", d - 60.0)),
            Some(d) => traits.push(format!("音高：弹 C4 测到 {d:.1}，音准正常")),
            None => traits.push("音高：测不到明确音高（噪声、打击类，或泛音很杂）".into()),
        }
        summary.push(bright.split('、').next().unwrap_or(bright).to_string());
        summary.push(attack.split('（').next().unwrap_or(attack).to_string());
        summary.push(if hold > -6.0 { "持续发声".into() } else if hold > -20.0 { "慢衰减".to_string() } else { "快衰减".to_string() });
        summary.push(release.to_string());
        if width >= 25.0 {
            summary.push("宽立体声".into());
        }
    }
    if let (Some(soft), Some(m)) = (probes.iter().find(|p| p.velocity < 0.5).and_then(|p| p.analysis.as_ref()), mid.and_then(|p| p.analysis.as_ref())) {
        let db = m.lufs - soft.lufs;
        let tone = m.centroid_hz / soft.centroid_hz.max(1.0);
        let resp = if db < 2.0 { "力度几乎不影响音量" } else if db < 10.0 { "力度影响适中" } else { "力度表现大" };
        traits.push(format!(
            "力度：轻弹（0.3）比重弹（0.8）低 {db:.1} dB，重弹的重心是轻弹的 {tone:.2} 倍 → {resp}{}",
            if tone > 1.25 { "，重弹明显更亮" } else { "" }
        ));
    }
    let lo = range.first().map(|&p| name(p)).unwrap_or("?");
    let hi = range.last().map(|&p| name(p)).unwrap_or("?");
    (format!("{}；音域约 {lo}–{hi}", summary.join("、")), traits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_a_pad_a_pluck_and_a_kit() {
        let none = serde_json::Map::new();
        let organ = describe("organ.jazz", &none, None, true).unwrap();
        assert!(organ.summary.contains("持续发声"), "{}", organ.summary);
        assert!(!organ.drums);
        let pluck = describe("guitar.nylon", &none, None, true).unwrap();
        assert!(pluck.summary.contains("衰减"), "{}", pluck.summary);
        let c4 = pluck.probes.iter().find(|p| p.pitch == 60).unwrap();
        assert!((c4.detected_pitch.unwrap() - 60.0).abs() < 0.5, "{:?}", c4.detected_pitch);
        let kit = describe("drums.808", &none, None, true).unwrap();
        assert!(kit.drums && kit.summary.starts_with("鼓组"), "{}", kit.summary);
        // 第二次从缓存读。
        assert_eq!(cached("drums.808", &none).unwrap().summary, kit.summary);
    }
}
