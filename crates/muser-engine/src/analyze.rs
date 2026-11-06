//! 听感报告（`muser analyze`）：按段落给出密度、音区、和声节奏、重复度、冲突，可选渲染后的响度与频段占比，
//! 最后用一句句话给出建议。给听不到声音的 agent 一副"耳朵"，用来自我检查编曲。
//!
//! - 段落：有 `form` 时按曲式的每一遍；没有时每 8 小节一段。
//! - 密度：每小节多少个起音（所有音轨，含鼓）。
//! - 重复：这一段里和前面某一小节完全相同（所有音轨合起来）的小节占多少。
//! - 冲突：拍点上的长音（≥ 1 拍）是避免音（比某个和弦音高半音：♭9、大三和弦上的 4、♭6）；或两条音轨同时长音成小二度 / 小九度。
//!   和弦用写好的 `chords`；没写时只查音轨之间的冲突，和弦按音高估计（标 `~`）。
//! - 能量：每段 0–10，按密度、在场声部数、平均力度（`audio` 时加上响度）相对全曲最满的一段。
//! - AI 味检查（`tells`）：真实编曲里几乎总有、而机器默认写法常缺的特征——段落全是 8 小节的倍数、能量只升不降、
//!   编制从头到尾不变、全曲一个和声循环、过门刻板地落在 4 的倍数小节、旋律 / 和声轨全部对齐网格、副歌每遍一模一样、
//!   各段动态与明暗一样（`audio`）。命中的会写进建议，并说明怎么改。
//! - `audio`：渲染一遍，给每段的响度（LUFS）、频谱重心与低 / 中 / 高频能量占比（能量占比在混音里总是低频为主，明暗看重心）。

use std::collections::BTreeMap;

use muser_core::Project;
use serde::Serialize;

use crate::abc_export::{Tone, estimate_key, guess_chord};
use crate::render::{EngineError, RenderSettings};

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub tempo: f64,
    pub bars: usize,
    /// 估计的调（或 clip 里 ABC 写的）。
    pub key: String,
    pub sections: Vec<SectionReport>,
    /// AI 味检查（只列适用的）。
    pub tells: Vec<Tell>,
    /// 建议，一句一条。
    pub findings: Vec<String>,
}

/// 一条 AI 味检查。
#[derive(Debug, Clone, Serialize)]
pub struct Tell {
    pub id: u8,
    pub name: String,
    /// 命中（像机器写的）。
    pub hit: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectionReport {
    pub name: String,
    pub occurrence: usize,
    /// 从第几小节开始（从 1 起）。
    pub bar: usize,
    pub bars: usize,
    /// 每小节起音数（全部音轨）。
    pub density: f64,
    /// 能量 0–10（相对全曲最满的一段）。
    pub energy: u8,
    pub tracks: Vec<TrackStat>,
    /// 和弦序列（每小节一个或几个）；估计的前面带 `~`。
    pub chords: Vec<String>,
    /// 每小节换几次和弦。
    pub chord_changes_per_bar: f64,
    /// 与前面某小节完全相同的小节占比（0–1，不算空小节）。
    pub repeated: f64,
    pub clashes: Vec<Clash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioStat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackStat {
    pub name: String,
    pub onsets_per_bar: f64,
    /// 发声时间占这一段的比例（0–1）。
    pub sounding: f64,
    /// 音区（最低–最高，音名）；鼓组为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(skip)]
    median: Option<u8>,
    #[serde(skip)]
    drums: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Clash {
    /// 第几小节（全曲，从 1 起）、第几拍（从 1 起）。
    pub bar: usize,
    pub beat: f64,
    pub what: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioStat {
    /// 积分响度（有门限：比整段平均低 10 LU 以上的部分不算）。
    pub lufs: f64,
    /// 平均电平（dB，不设门限）：大部分很安静、只有几下很响的段落，它比 lufs 低得多。
    pub rms_db: f64,
    pub centroid_hz: f64,
    pub low_pct: f64,
    pub mid_pct: f64,
    pub high_pct: f64,
}

/// MIDI 音高 → 音名（C4 = 60）。
fn name(p: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
    format!("{}{}", N[usize::from(p % 12)], i32::from(p) / 12 - 1)
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// 生成报告。`audio` 时渲染一遍（用 `settings`）。
pub fn analyze(project: &Project, audio: bool, settings: RenderSettings) -> Result<Report, EngineError> {
    crate::validate_score(project)?;
    let invalid = |m: String| EngineError::Core(muser_core::CoreError::Invalid(m));
    let mut spans = crate::arrange::timeline(project).map_err(invalid)?;
    // 整首变调：写在段落上的和弦也跟着移，和移过调的音对得上。
    for s in &mut spans {
        for c in &mut s.chords {
            c.chord = c.chord.transposed(project.transpose);
        }
    }
    let arranged = crate::arrange::arranged(project).map_err(invalid)?;
    let p = &*arranged;
    let tm = p.tempo_map();
    let map = p.bar_map();
    // 每条音轨的音（绝对拍）。audio 音轨没有音符，不计。
    let mut tracks: Vec<(String, bool, Vec<Tone>)> = Vec::new();
    let mut vels: Vec<Vec<f32>> = Vec::new();
    let mut unhumanized: Vec<String> = Vec::new();
    for (ti, t) in p.tracks.iter().enumerate() {
        if t.instrument.trim() == muser_core::audio::INSTRUMENT {
            continue;
        }
        let mut tones = Vec::new();
        let mut v = Vec::new();
        for (ci, clip) in t.clips.iter().enumerate() {
            for n in crate::clip_notes(p, ti, ci).map_err(invalid)?.iter() {
                let (a, b) = (n.start.0, (n.start.0 + n.length.0).min(clip.length.0));
                if b > a {
                    tones.push(Tone { start: clip.start.0 + a, end: clip.start.0 + b, pitch: n.pitch });
                    v.push(n.velocity);
                }
            }
        }
        let name = if t.name.is_empty() { format!("tracks[{ti}]") } else { t.name.clone() };
        let drums = t.instrument.trim().starts_with("drums.");
        if !drums && !tones.is_empty() && t.humanize.is_none() && t.clips.iter().all(|c| c.humanize.is_none()) {
            unhumanized.push(name.clone());
        }
        tracks.push((name, drums, tones));
        vels.push(v);
    }
    let total_beats = p.duration_beats().0;
    let bars = map.bars_until(total_beats);
    let pitched: Vec<Tone> = tracks.iter().filter(|t| !t.1).flat_map(|t| t.2.iter().copied()).collect();
    let key = estimate_key(&pitched);

    // 段落：曲式，或每 8 小节。
    let windows: Vec<(String, usize, f64, f64, Vec<muser_core::chord::Placed>)> = if spans.is_empty() {
        (0..bars.div_ceil(8))
            .map(|i| {
                let (a, b) = (i * 8, ((i + 1) * 8).min(bars));
                (format!("bars {}–{}", a + 1, b), 1, map.bar_start(a), map.bar_start(b) - map.bar_start(a), Vec::new())
            })
            .collect()
    } else {
        spans.iter().map(|s| (s.name.clone(), s.occurrence, s.start, s.length, s.chords.clone())).collect()
    };

    // 每小节的指纹（所有音轨的起音位置与音高），算重复度。
    let fingerprint = |b: usize| -> Vec<(usize, i64, u8)> {
        let (a, z) = (map.bar_start(b), map.bar_start(b + 1));
        let mut f: Vec<(usize, i64, u8)> = tracks
            .iter()
            .enumerate()
            .flat_map(|(i, t)| t.2.iter().filter(move |n| n.start >= a - 1e-9 && n.start < z - 1e-9).map(move |n| (i, ((n.start - a) * 48.0).round() as i64, n.pitch)))
            .collect();
        f.sort_unstable();
        f
    };
    let prints: Vec<Vec<(usize, i64, u8)>> = (0..bars).map(fingerprint).collect();
    let repeated_bar: Vec<bool> = (0..bars).map(|b| !prints[b].is_empty() && prints[..b].contains(&prints[b])).collect();

    let rendered = if audio { Some(crate::render(project, settings)?) } else { None };
    let mut energy_raw: Vec<(f64, f64, f64)> = Vec::new();
    let sr = f64::from(settings.sample_rate);
    let mut sections = Vec::new();
    for (name_s, occurrence, start, length, chords) in &windows {
        let (a, z) = (*start, start + length);
        let first_bar = map.bar_at(a + 1e-9).0;
        let nbars = map.bars_until(z).saturating_sub(first_bar).max(1);
        let inside = |n: &&Tone| n.start >= a - 1e-9 && n.start < z - 1e-9;
        let mut stats = Vec::new();
        let mut onsets = 0usize;
        for (tname, drums, tones) in &tracks {
            let ts: Vec<&Tone> = tones.iter().filter(inside).collect();
            onsets += ts.len();
            if ts.is_empty() {
                continue;
            }
            // 发声时间：音符区间的并集。
            let mut iv: Vec<(f64, f64)> = tones.iter().filter(|n| n.end > a && n.start < z).map(|n| (n.start.max(a), n.end.min(z))).collect();
            iv.sort_by(|x, y| x.0.total_cmp(&y.0));
            let (mut sounding, mut end) = (0.0, f64::MIN);
            for (x, y) in iv {
                sounding += (y - x.max(end)).max(0.0);
                end = end.max(y);
            }
            let mut ps: Vec<u8> = ts.iter().map(|n| n.pitch).collect();
            ps.sort_unstable();
            stats.push(TrackStat {
                name: tname.clone(),
                onsets_per_bar: round2(ts.len() as f64 / nbars as f64),
                sounding: round2(sounding / length),
                range: (!drums).then(|| format!("{}–{}", name(ps[0]), name(*ps.last().unwrap()))),
                median: (!drums).then(|| ps[ps.len() / 2]),
                drums: *drums,
            });
        }
        // 和弦：写好的，或按每小节估计。
        let (chord_names, changes): (Vec<String>, usize) = if !chords.is_empty() {
            let names: Vec<String> = chords.iter().map(|c| c.chord.text.clone()).collect();
            let changes = names.windows(2).filter(|w| w[0] != w[1]).count();
            (names, changes)
        } else {
            let weighted: Vec<(Tone, f64)> = pitched.iter().map(|t| (*t, 1.0)).collect();
            let mut names = Vec::new();
            for b in 0..nbars {
                let (x, y) = (map.bar_start(first_bar + b), map.bar_start(first_bar + b + 1));
                if let Some((n, _)) = guess_chord(&weighted, x, y, key) {
                    names.push(format!("~{n}"));
                }
            }
            let changes = names.windows(2).filter(|w| w[0] != w[1]).count();
            (names, changes)
        };
        // 冲突。
        let mut clashes: Vec<Clash> = Vec::new();
        let at = |t: f64| { let (b, into) = map.bar_at(t); (b + 1, round2(into + 1.0)) };
        let held: Vec<(&str, Tone)> = tracks
            .iter()
            .filter(|t| !t.1)
            .flat_map(|t| t.2.iter().filter(inside).filter(|n| n.end - n.start >= 1.0 - 1e-9 && (n.start % 1.0).abs() < 1e-6).map(move |n| (t.0.as_str(), *n)))
            .collect();
        for (tn, n) in &held {
            if let Some(c) = muser_core::chord::at(chords, n.start) {
                let pcs = c.pitch_classes();
                let pc = n.pitch % 12;
                // 避免音：比某个和弦音高半音（♭9、大三和弦上的 4、♭6）。低半音的（大七、九音）是常用的色彩音，不算。
                if !pcs.contains(&pc) && pcs.contains(&((pc + 11) % 12)) {
                    let (bar, beat) = at(n.start);
                    let below = name((n.pitch + 11) % 12 + 60).trim_end_matches('4').to_string();
                    clashes.push(Clash { bar, beat, what: format!("{tn}: {} is an avoid note over {} (a semitone above the chord tone {below})", name(n.pitch), c.text) });
                }
            }
        }
        for (i, (ta, x)) in held.iter().enumerate() {
            for (tb, y) in held.iter().skip(i + 1) {
                if ta == tb || (x.start - y.start).abs() > 1e-6 {
                    continue;
                }
                let (lo, hi) = if x.pitch < y.pitch { (x.pitch, y.pitch) } else { (y.pitch, x.pitch) };
                if hi - lo == 1 || hi - lo == 13 {
                    let (bar, beat) = at(x.start);
                    let kind = if hi - lo == 1 { "minor 2nd" } else { "minor 9th" };
                    clashes.push(Clash { bar, beat, what: format!("{ta} {} and {tb} {} form a {kind}", name(x.pitch), name(y.pitch)) });
                }
            }
        }
        clashes.sort_by(|p, q| (p.bar, p.beat).partial_cmp(&(q.bar, q.beat)).unwrap());
        clashes.dedup_by(|p, q| p.what == q.what);
        clashes.truncate(8);
        let nonempty = (first_bar..(first_bar + nbars).min(bars)).filter(|&b| !prints[b].is_empty()).count();
        let rep = (first_bar..(first_bar + nbars).min(bars)).filter(|&b| repeated_bar[b]).count();
        let audio_stat = rendered.as_ref().map(|buf| {
            let secs = |beats: f64| tm.secs_at(beats);
            let (fa, fz) = ((secs(a) * sr) as usize, ((secs(z) * sr) as usize).min(buf.num_frames()));
            let mut part = crate::AudioBuffer::<f32>::new(2, fz.saturating_sub(fa).max(1));
            for c in 0..2 {
                let n = part.num_frames().min(buf.num_frames().saturating_sub(fa));
                part.channel_mut(c)[..n].copy_from_slice(&buf.channel(c)[fa..fa + n]);
            }
            let an = crate::audition::analyze(&part, sr, (0, part.num_frames()));
            let ms = (0..2).map(|c| part.channel(c).iter().map(|x| f64::from(*x).powi(2)).sum::<f64>()).sum::<f64>() / (2 * part.num_frames()) as f64;
            let rms_db = (10.0 * ms.max(1e-12).log10() * 10.0).round() / 10.0;
            AudioStat { lufs: (an.lufs * 10.0).round() / 10.0, rms_db, centroid_hz: an.centroid_hz.round(), low_pct: an.low_pct.round(), mid_pct: an.mid_pct.round(), high_pct: an.high_pct.round() }
        });
        let vel: Vec<f32> = tracks.iter().zip(&vels).flat_map(|(t, v)| t.2.iter().zip(v).filter(|(n, _)| inside(n)).map(|(_, &x)| x)).collect();
        energy_raw.push((onsets as f64 / nbars as f64, stats.len() as f64, if vel.is_empty() { 0.0 } else { f64::from(vel.iter().sum::<f32>()) / vel.len() as f64 }));
        sections.push(SectionReport {
            name: name_s.clone(),
            occurrence: *occurrence,
            bar: first_bar + 1,
            bars: nbars,
            density: round2(onsets as f64 / nbars as f64),
            energy: 0,
            tracks: stats,
            chord_changes_per_bar: round2(changes as f64 / nbars as f64),
            chords: chord_names,
            repeated: if nonempty == 0 { 0.0 } else { round2(rep as f64 / nonempty as f64) },
            clashes,
            audio: audio_stat,
        });
    }
    // 能量：密度、在场声部数、平均力度各自相对全曲最大值，加权；有渲染时再和响度平均。
    let max = |f: fn(&(f64, f64, f64)) -> f64| energy_raw.iter().map(f).fold(1e-9, f64::max);
    let (md, ma, mv) = (max(|e| e.0), max(|e| e.1), max(|e| e.2));
    let lufs: Vec<Option<f64>> = sections.iter().map(|s| s.audio.as_ref().map(|a| a.lufs).filter(|l| l.is_finite())).collect();
    let (lo, hi) = lufs.iter().flatten().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    for (i, s) in sections.iter_mut().enumerate() {
        let (d, a, v) = energy_raw[i];
        let mut e = 0.45 * d / md + 0.35 * a / ma + 0.2 * v / mv;
        if let Some(l) = lufs[i] {
            e = 0.5 * e + 0.5 * if hi - lo > 1e-6 { (l - lo) / (hi - lo) } else { 1.0 };
        }
        s.energy = (e * 10.0).round().clamp(0.0, 10.0) as u8;
    }
    let tells = tells(project, &sections, &tracks, &prints, &unhumanized, key, &map);
    let mut findings = findings(&sections, &repeated_bar, &prints);
    findings.splice(0..0, silent_sections(project, &sections, &map));
    findings.retain(|f| !f.starts_with("Section loudness differs by only") && f != "No obvious problems found");
    findings.extend(tells.iter().filter(|t| t.hit).map(|t| format!("AI tell #{} {}: {}", t.id, t.name, t.detail)));
    if findings.is_empty() {
        findings.push("No obvious problems found".into());
    }
    Ok(Report { tempo: p.tempo, bars, key: key.name().to_string(), sections, tells, findings })
}

/// "第 i 段撤掉这几条轨"的具体写法：有 form 时给出改 form[i] 的 JSON，没有 form 时说去掉哪几小节的音符。
fn mute_hint(project: &Project, sections: &[SectionReport], i: usize, mute: &[&str]) -> String {
    let s = &sections[i];
    let span = format!("bars {}–{}", s.bar, s.bar + s.bars - 1);
    if project.form.is_empty() {
        return format!("{} ({span}): drop the notes of {}", s.name, mute.join(", "));
    }
    let list = mute.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ");
    format!("{} ({span}): mute {}: write form[{i}] as {{\"section\": \"{}\", \"mute\": [{list}]}} (keep any existing transpose / seed)", s.name, mute.join(", "), s.name)
}

/// 编制不变时：开头一段只留最稀的一件（铺底 / 和弦），其余先不进。
fn sparse_hint(project: &Project, sections: &[SectionReport]) -> String {
    let Some(first) = sections.first() else { return String::new() };
    let keep = first.tracks.iter().filter(|t| !t.drums).min_by(|a, b| a.onsets_per_bar.total_cmp(&b.onsets_per_bar)).or(first.tracks.first());
    let Some(keep) = keep else { return String::new() };
    let mute: Vec<&str> = first.tracks.iter().filter(|t| t.name != keep.name).map(|t| t.name.as_str()).collect();
    if mute.is_empty() {
        return String::new();
    }
    let mut out = format!("e.g. {}, keeping only {}", mute_hint(project, sections, 0, &mute), keep.name);
    if sections.len() >= 3 {
        out.push('；');
        out.push_str(&drop_hint(project, sections));
    }
    out
}

/// 能量只升时：在最后一个高潮段的前一段撤掉鼓（没有鼓就撤掉最密的一件），让高潮"重新进来"。
fn drop_hint(project: &Project, sections: &[SectionReport]) -> String {
    let peak = sections.iter().rposition(|s| s.energy == sections.iter().map(|x| x.energy).max().unwrap_or(0)).unwrap_or(0);
    let i = if peak >= 2 { peak - 1 } else { sections.len().saturating_sub(2).max(1).min(sections.len() - 1) };
    let s = &sections[i];
    let mut mute: Vec<&str> = s.tracks.iter().filter(|t| t.drums).map(|t| t.name.as_str()).collect();
    if mute.is_empty() {
        if let Some(t) = s.tracks.iter().max_by(|a, b| a.onsets_per_bar.total_cmp(&b.onsets_per_bar)) {
            mute.push(&t.name);
        }
    }
    if mute.is_empty() || mute.len() == s.tracks.len() {
        return String::new();
    }
    format!("e.g. {}, then bring everything back in the next section", mute_hint(project, sections, i, &mute))
}

/// AI 味检查：真实编曲里几乎总有、机器默认写法常缺的特征。只给适用的（段落太少、没有鼓等时不查）。
#[allow(clippy::too_many_arguments)]
fn tells(
    project: &Project,
    sections: &[SectionReport],
    tracks: &[(String, bool, Vec<Tone>)],
    prints: &[Vec<(usize, i64, u8)>],
    unhumanized: &[String],
    key: crate::abc_export::Key,
    map: &muser_core::BarMap,
) -> Vec<Tell> {
    let mut out = Vec::new();
    let mut tell = |id: u8, name: &str, hit: bool, detail: String| out.push(Tell { id, name: name.into(), hit, detail });
    let n = sections.len();
    let has_form = !project.form.is_empty();
    if has_form && n >= 3 {
        let all8 = sections.iter().all(|s| s.bars % 8 == 0);
        tell(1, "Every section is a multiple of 8 bars", all8, if all8 {
            "Real songs often have sections of 2 / 4 / 6 / 10 / 12 bars: shorten or lengthen the intro, pre-chorus or outro, or add a 2-bar transition".into()
        } else { "Some sections are not a multiple of 8 bars".into() });
    }
    if n >= 3 {
        let e: Vec<u8> = sections.iter().map(|s| s.energy).collect();
        let rising = e.windows(2).all(|w| w[1] >= w[0]);
        tell(2, "Energy only rises, never drops", rising, if rising {
            format!("Energy {:?} never drops (all 62 of 62 real arrangements measured have drops): pull a few instruments out of the interlude, bridge or before the last chorus. {}", e, drop_hint(project, sections))
        } else { format!("Energy {:?} rises and falls", e) });
        let present: Vec<Vec<&str>> = sections.iter().map(|s| s.tracks.iter().map(|t| t.name.as_str()).collect()).collect();
        let used: std::collections::BTreeSet<&str> = present.iter().flatten().copied().collect();
        let constant = used.len() >= 2 && present.iter().all(|p| p.len() == used.len());
        tell(3, "Instrumentation never changes", constant, if constant {
            format!("Every track plays in every section: let instruments come and go - fewer in the intro, enter at the chorus, drop the drums or bass in some section. {}", sparse_hint(project, sections))
        } else { "Instruments come and go".into() });
        let progs: Vec<String> = sections.iter().filter(|s| !s.chords.is_empty()).map(|s| {
            let mut c: Vec<&str> = s.chords.iter().map(|x| x.trim_start_matches('~')).collect();
            c.dedup();
            c.join(" ")
        }).collect();
        let one_loop = progs.len() >= 3 && progs.windows(2).all(|w| w[0] == w[1]);
        let scale = key.scale_pcs();
        let outside = sections.iter().flat_map(|s| s.chords.iter()).filter_map(|c| muser_core::chord::Chord::parse(c.trim_start_matches('~')).ok())
            .any(|c| c.pitch_classes().iter().any(|pc| !scale.contains(pc)));
        let modulates = project.form.iter().any(|e| e.transpose() != 0);
        if !progs.is_empty() {
            let hit = one_loop || (!outside && !modulates && n >= 4);
            tell(4, "One harmonic loop for the whole song / never leaves the key", hit, if one_loop {
                format!("Every section uses the same progression ({}): give the pre / bridge / chorus different chords", progs[0])
            } else if hit {
                "Only diatonic chords and no key change: use a non-diatonic chord in the bridge or pre (borrowed chords such as iv, bVII, or secondary dominants such as V/vi), or transpose the last chorus".into()
            } else { "The harmony varies".into() });
        }
    }
    // 过门：鼓轨里通鼓 ≥ 3 下、或最后两拍军鼓 ≥ 4 下的小节。
    let drum_tones: Vec<&Tone> = tracks.iter().filter(|t| t.1).flat_map(|t| t.2.iter()).collect();
    if !drum_tones.is_empty() {
        let fill_bars: Vec<usize> = (0..prints.len())
            .filter(|&b| {
                let (a, z) = (map.bar_start(b), map.bar_start(b + 1));
                let toms = drum_tones.iter().filter(|t| t.start >= a && t.start < z && [41, 43, 45, 47, 48, 50].contains(&t.pitch)).count();
                let rolls = drum_tones.iter().filter(|t| t.start >= z - 2.0 && t.start < z && [37, 38, 40].contains(&t.pitch)).count();
                toms >= 3 || rolls >= 4
            })
            .collect();
        if fill_bars.len() >= 3 {
            let rigid = fill_bars.iter().all(|b| (b + 1) % 4 == 0) && fill_bars.len() * 4 >= prints.len() * 3 / 4;
            let shown: Vec<String> = fill_bars.iter().take(12).map(|b| (b + 1).to_string()).collect();
            tell(5, "Fills fall rigidly on multiples of 4 bars", rigid, if rigid {
                format!("Fills at bar {}, every 4 bars: leave fill_every out of the drummer (it then fills automatically at section ends and at random phrase ends), or write fills at section ends with end", shown.join(", "))
            } else { format!("Fills at bar {}", shown.join(", ")) });
        }
    }
    let pitched = tracks.iter().filter(|t| !t.1 && !t.2.is_empty()).count();
    if pitched > 0 {
        let hit = unhumanized.len() == pitched;
        tell(8, "Melody / harmony all on the grid", hit, if hit {
            format!("{} have no humanize: add humanize to the melody, bass and harmony (timing_ms, drift, or push / lay-back); the drum kit can stay on the grid", unhumanized.join(", "))
        } else if unhumanized.is_empty() { "Melody / harmony tracks all have humanize".into() } else { format!("Tracks without humanize: {}", unhumanized.join(", ")) });
    }
    // 副歌每遍一模一样：各遍的小节指纹完全相同。
    let is_chorus = |n: &str| { let n = n.to_lowercase(); ["chorus", "drop", "hook", "副歌"].iter().any(|k| n.contains(k)) };
    let choruses: Vec<&SectionReport> = sections.iter().filter(|s| is_chorus(&s.name)).collect();
    if choruses.len() >= 2 {
        let body = |s: &SectionReport| -> Vec<Vec<(usize, i64, u8)>> { (s.bar - 1..s.bar - 1 + s.bars).filter_map(|b| prints.get(b).cloned()).collect() };
        let same = choruses.windows(2).all(|w| body(w[0]) == body(w[1]));
        tell(11, "Every chorus is identical", same, if same {
            "The notes of every chorus are identical: for later choruses use chorus#2 to write it differently, add parts, vary, or transpose the last one".into()
        } else { "The choruses vary".into() });
    }
    let audio: Vec<&AudioStat> = sections.iter().filter_map(|s| s.audio.as_ref()).filter(|a| a.lufs.is_finite()).collect();
    if audio.len() >= 3 {
        let (lo, hi) = audio.iter().fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x.lufs), b.max(x.lufs)));
        let mean = audio.iter().map(|a| a.centroid_hz).sum::<f64>() / audio.len() as f64;
        let sd = (audio.iter().map(|a| (a.centroid_hz - mean).powi(2)).sum::<f64>() / audio.len() as f64).sqrt();
        let flat = hi - lo < 1.5 || sd < 0.04 * mean;
        tell(7, "Dynamics flattened, brightness never changes", flat, if flat {
            format!("Section loudness differs by only {:.1} LU and spectral centroid varies by {:.0}%: make the intro / interlude sparser and darker and the chorus fuller and brighter (drop instruments, change register, eq / filter automation)", hi - lo, 100.0 * sd / mean.max(1.0))
        } else { format!("Section loudness differs by {:.1} LU", hi - lo) });
    }
    out
}

/// 几乎没有声音的段落（平均电平低于 −45 dB，或整段静音），并说出从工程里看得出的原因。
/// 来自 dreamer 四首的教训：主输出音量曲线和音轨的淡入淡出叠在一起，一整段就没声了，agent 听不到。
fn silent_sections(p: &Project, sections: &[SectionReport], map: &muser_core::BarMap) -> Vec<String> {
    const QUIET_DB: f64 = -45.0;
    let mut out = Vec::new();
    for s in sections {
        let Some(a) = &s.audio else { continue };
        if a.lufs.is_finite() && a.rms_db >= QUIET_DB {
            continue;
        }
        let start = map.bar_start(s.bar - 1);
        let len = map.bar_start(s.bar - 1 + s.bars) - start;
        // 段内 8 个点的平均：自动化有值就用它，没有就是静态值。
        let level = |autos: &[muser_core::Automation], target: &str, base: f64| {
            let curve = autos.iter().find(|x| x.target == target).map(|x| {
                let values: Vec<f64> = match &x.shape {
                    muser_core::automation::Shape::Points { points, .. } => points.iter().map(|(_, v)| v.as_f64().unwrap_or(0.0)).collect(),
                    _ => Vec::new(),
                };
                x.compile(&values, 0, start + len)
            });
            (0..8).map(|k| curve.as_ref().and_then(|c| c.value_at(start + (k as f64 + 0.5) / 8.0 * len)).unwrap_or(base)).sum::<f64>() / 8.0
        };
        let playing: Vec<&TrackStat> = s.tracks.iter().filter(|t| t.onsets_per_bar > 0.0 || t.sounding > 0.0).collect();
        let mut why = Vec::new();
        if playing.is_empty() {
            why.push("this section has no notes (muted in form, or parts do not cover it)".to_string());
        }
        let master = level(&p.automation, "master_gain_db", p.master_gain_db);
        if master < -24.0 {
            why.push(format!("master level averages {master:.0} dB here"));
        }
        let low: Vec<String> = p
            .tracks
            .iter()
            .filter(|t| playing.iter().any(|x| x.name == t.name))
            .filter_map(|t| {
                let g = level(&t.automation, "gain_db", t.gain_db);
                (g < -30.0).then(|| format!("{} {g:.0} dB", t.name))
            })
            .collect();
        if !low.is_empty() {
            let more = if low.len() > 5 { format!(" and {} in all", low.len()) } else { String::new() };
            why.push(format!("the tracks that are playing are very quiet ({}{more})", low[..low.len().min(5)].join(", ")));
        }
        if master < -24.0 && !low.is_empty() {
            why.push("the master curve and the track fades overlap".into());
        }
        let why = if why.is_empty() { "cause unclear: check the level automation, fades and effects (gate, filter) on the master and the tracks".to_string() } else { why.join("; ") };
        let level = if a.rms_db.is_finite() { format!("averages {:.0} dB", a.rms_db) } else { "completely silent".into() };
        out.push(format!(
            "{} (bars {}–{}) is almost silent ({level}): {why}. Ignore this if the gap is intentional",
            s.name,
            s.bar,
            s.bar + s.bars - 1
        ));
    }
    out
}

/// 把数字翻成建议。
fn findings(sections: &[SectionReport], repeated_bar: &[bool], prints: &[Vec<(usize, i64, u8)>]) -> Vec<String> {
    let mut out = Vec::new();
    let avg = |pred: &dyn Fn(&str) -> bool, f: &dyn Fn(&SectionReport) -> Option<f64>| {
        let v: Vec<f64> = sections.iter().filter(|s| pred(&s.name.to_lowercase())).filter_map(f).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    let is_chorus = |n: &str| ["chorus", "drop", "hook", "副歌"].iter().any(|k| n.contains(k));
    let is_verse = |n: &str| ["verse", "主歌"].iter().any(|k| n.contains(k));
    if let (Some(c), Some(v)) = (avg(&is_chorus, &|s| Some(s.density)), avg(&is_verse, &|s| Some(s.density))) {
        let louder = match (avg(&is_chorus, &|s| s.audio.as_ref().map(|a| a.lufs)), avg(&is_verse, &|s| s.audio.as_ref().map(|a| a.lufs))) {
            (Some(cl), Some(vl)) => Some(cl - vl),
            _ => None,
        };
        if c <= v * 1.05 && louder.is_none_or(|d| d < 1.0) {
            out.push(format!(
                "The chorus density ({c:.1} notes/bar) is not higher than the verse ({v:.1}){}: the chorus does not lift; add parts, raise the register, fill out the drums or use a denser pattern",
                louder.map_or(String::new(), |d| format!(", loudness differs by only {d:+.1} LU"))
            ));
        }
    }
    let nonempty = prints.iter().filter(|p| !p.is_empty()).count();
    let rep = repeated_bar.iter().filter(|&&r| r).count();
    if nonempty >= 8 && rep as f64 > 0.75 * nonempty as f64 {
        out.push(format!(
            "{}% of bars are exactly the same as an earlier bar: add variation with transform (vary, every), end fills or a different part",
            (100.0 * rep as f64 / nonempty as f64).round()
        ));
    }
    let distinct: std::collections::BTreeSet<&str> = sections.iter().flat_map(|s| s.chords.iter().map(|c| c.trim_start_matches('~'))).collect();
    if distinct.len() == 1 && nonempty >= 8 {
        out.push(format!("The whole song has only one chord ({}): unless this is a deliberate drone / one-chord groove, write a chord progression in sections", distinct.iter().next().unwrap()));
    }
    // 音区重叠：同一段里两条旋律 / 和声音轨长时间发声、中位音高相差不到 5 个半音。同一条轨和几条轨都重叠时合成一句。
    let mut masked: BTreeMap<String, BTreeMap<String, (u8, u8, Vec<String>)>> = BTreeMap::new();
    for s in sections {
        let ts: Vec<&TrackStat> = s.tracks.iter().filter(|t| !t.drums && t.sounding >= 0.5).collect();
        for (i, x) in ts.iter().enumerate() {
            for y in ts.iter().skip(i + 1) {
                if let (Some(a), Some(b)) = (x.median, y.median) {
                    if a.abs_diff(b) < 5 {
                        let e = masked.entry(x.name.clone()).or_default().entry(y.name.clone()).or_insert((a, b, Vec::new()));
                        if !e.2.contains(&s.name) {
                            e.2.push(s.name.clone());
                        }
                    }
                }
            }
        }
    }
    for (a, with) in masked {
        let ma = with.values().next().map_or(0, |w| w.0);
        let others: Vec<String> = with.iter().map(|(b, (_, mb, _))| format!("{b} (median {})", name(*mb))).collect();
        let mut secs: Vec<String> = with.values().flat_map(|w| w.2.clone()).collect();
        secs.dedup();
        let mut seen = std::collections::HashSet::new();
        secs.retain(|x| seen.insert(x.clone()));
        out.push(format!(
            "{a} (median {}) and {} sound together for long stretches in {} in the same register (median pitches less than 5 semitones apart) and may mask each other: separate them by an octave, change the inversion, or rest one",
            name(ma), others.join("、"), secs.join("、")
        ));
    }
    let clashes: usize = sections.iter().map(|s| s.clashes.len()).sum();
    if clashes > 0 {
        out.push(format!("{clashes} long-note clashes on strong beats (see clashes in each section): check whether this is the tension you want; otherwise use chord tones or change the chord"));
    }
    let lufs: Vec<f64> = sections.iter().filter_map(|s| s.audio.as_ref().map(|a| a.lufs)).filter(|l| l.is_finite()).collect();
    if lufs.len() >= 3 {
        let (lo, hi) = lufs.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        if hi - lo < 1.5 {
            out.push(format!("Section loudness differs by only {:.1} LU: no dynamic contrast; make the intro / interlude sparser and the chorus fuller", hi - lo));
        }
    }
    // 明暗看频谱重心：完整的混音一般在 1500–3000 Hz（低频的能量占比在混音里总是很高，不作判断）。
    for (sec, a) in sections.iter().filter_map(|s| s.audio.as_ref().map(|a| (s, a))).filter(|(_, a)| a.lufs.is_finite()) {
        if a.centroid_hz < 1000.0 {
            out.push(format!("{} has a spectral centroid of only {} Hz: dark and muffled; add hi-hat / shaker, brighten the harmony track with high_shelf, or add highpass to non-bass tracks", sec.name, a.centroid_hz));
        } else if a.centroid_hz > 4500.0 {
            out.push(format!("{} has a spectral centroid of {} Hz: bright and possibly harsh; lower the cymbals / high tracks, or use lowpass", sec.name, a.centroid_hz));
        }
    }
    if out.is_empty() {
        out.push("No obvious problems found".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_with_their_own_meter() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120,
            "sections": { "a": { "bars": 2, "chords": "C" },
                          "odd": { "bars": 3, "chords": "F G A", "time_signature": { "numerator": 7, "denominator": 8 } } },
            "form": ["a", "odd", "a"],
            "tracks": [{ "name": "pad", "instrument": "poly.juno_keys", "parts": { "*": { "chordal": "c", "step": 1, "octave": 4 } } }] }))
        .unwrap();
        let spans = crate::arrange::timeline(&p).unwrap();
        assert_eq!(spans.iter().map(|s| (s.start, s.length)).collect::<Vec<_>>(), vec![(0.0, 8.0), (8.0, 10.5), (18.5, 8.0)]);
        // 7/8 段落的和弦每个 3.5 拍。
        assert_eq!(spans[1].chords.iter().map(|c| c.start).collect::<Vec<_>>(), vec![8.0, 11.5, 15.0]);
        let r = analyze(&p, false, RenderSettings::default()).unwrap();
        assert_eq!(r.sections.iter().map(|s| (s.bar, s.bars)).collect::<Vec<_>>(), vec![(1, 2), (3, 3), (6, 2)]);
        let midi = crate::midi::export(&p).unwrap().bytes;
        let ts_events = midi.windows(3).filter(|w| w == &[0xFF, 0x58, 0x04]).count();
        assert_eq!(ts_events, 3, "4/4、7/8、回到 4/4");
    }

    #[test]
    fn warns_about_a_section_the_master_curve_silences() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120,
            "sections": { "a": { "bars": 2, "chords": "C" }, "gap": { "bars": 2, "chords": "C" } },
            "form": ["a", "gap"],
            "automation": [{ "target": "master_gain_db", "shape": "points", "curve": "step", "points": [[0, 0.0], [8, -80.0]] }],
            "tracks": [{ "name": "pad", "instrument": "poly.juno_keys", "parts": { "*": { "chordal": "c - - -", "step": 1, "octave": 4 } } }] }))
        .unwrap();
        let r = analyze(&p, true, RenderSettings::default()).unwrap();
        let hit: Vec<&String> = r.findings.iter().filter(|f| f.contains("almost silent")).collect();
        assert_eq!(hit.len(), 1, "{:?}", r.findings);
        assert!(hit[0].starts_with("gap (bars 3–4)") && hit[0].contains("master level"), "{hit:?}");
    }

    #[test]
    fn sections_density_clashes_and_findings() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120,
            "sections": { "verse": { "bars": 4, "chords": "C" }, "chorus": { "bars": 4, "chords": "C" } },
            "form": ["verse", "chorus"],
            "tracks": [
                { "name": "lead", "instrument": "wind.flute", "parts": { "*": { "notes": [{ "pitch": 65, "start": 0, "length": 2 }], "length": 4 } } },
                { "name": "pad", "instrument": "poly.juno_keys", "parts": { "*": { "chordal": "c - - -", "step": 1, "octave": 4 } } },
                { "name": "drums", "instrument": "drums.808", "parts": { "*": { "grid": { "kick": "x...x..." } } } }
            ] }))
        .unwrap();
        let r = analyze(&p, false, RenderSettings::default()).unwrap();
        assert_eq!(r.sections.len(), 2);
        let v = &r.sections[0];
        assert_eq!((v.name.as_str(), v.bar, v.bars), ("verse", 1, 4));
        // 每小节：lead 1 + pad 3（三和弦）+ 鼓 4（x...x... 是 2 拍，一小节循环两次）= 8。
        assert_eq!(v.density, 8.0);
        // F 在 C 和弦上：与 E 差半音。
        assert!(v.clashes.iter().any(|c| c.what.contains("F4") && c.what.contains("avoid note")), "{:?}", v.clashes);
        assert!(r.findings.iter().any(|f| f.contains("does not lift")), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.contains("exactly the same")), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.contains("only one chord")), "{:?}", r.findings);
    }

    /// 三段、8 小节、能量只升、编制不变、一个进行、没有 humanize：这些 AI 味都命中；改掉之后不再命中。
    #[test]
    fn ai_tells() {
        let mk = |verse_extra: &str, humanize: bool| -> Project {
            let mut v = serde_json::json!({ "tempo": 120,
                "sections": { "a": { "bars": 8, "chords": "C G" }, "b": { "bars": 8, "chords": "C G" }, "c": { "bars": 8, "chords": "C G" } },
                "form": ["a", "b", "c"],
                "tracks": [
                    { "name": "keys", "instrument": "poly.juno_keys", "parts": {
                        "a": { "chordal": "c - - -", "step": 1 }, "b": { "chordal": "c . c .", "step": 1 }, "c": { "chordal": "c c c c", "step": 0.5 } } },
                    { "name": "bass", "instrument": "bass.finger", "parts": { "*": { "chordal": "1 . 5 .", "octave": 2 } } }
                ] });
            if verse_extra == "drop" {
                v["form"] = serde_json::json!(["a", "c", { "section": "b", "mute": ["bass"] }]);
                v["sections"]["b"]["bars"] = serde_json::json!(6);
                v["sections"]["b"]["chords"] = serde_json::json!("Am Fm");
            }
            if humanize {
                v["tracks"][0]["humanize"] = serde_json::json!({ "timing_ms": 6 });
            }
            serde_json::from_value(v).unwrap()
        };
        let hits = |p: &Project| -> Vec<u8> { analyze(p, false, RenderSettings::default()).unwrap().tells.iter().filter(|t| t.hit).map(|t| t.id).collect() };
        assert_eq!(hits(&mk("", false)), [1, 2, 3, 4, 8]);
        let r = analyze(&mk("", false), false, RenderSettings::default()).unwrap();
        let t3 = &r.tells.iter().find(|t| t.id == 3).unwrap().detail;
        assert!(t3.contains("a (bars 1–8): mute bass") && t3.contains(r#"form[0] as {"section": "a", "mute": ["bass"]}"#), "{t3}");
        // 一段 6 小节、能量回落、贝斯退出、借用和弦 Fm、keys 有 humanize。
        assert_eq!(hits(&mk("drop", true)), Vec::<u8>::new());
    }
}
