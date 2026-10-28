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
    pub lufs: f64,
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
    let spans = crate::arrange::timeline(project).map_err(invalid)?;
    let arranged = crate::arrange::arranged(project).map_err(invalid)?;
    let p = &*arranged;
    let bar_beats = p.bar_beats();
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
    let bars = ((total_beats / bar_beats) - 1e-9).ceil().max(1.0) as usize;
    let pitched: Vec<Tone> = tracks.iter().filter(|t| !t.1).flat_map(|t| t.2.iter().copied()).collect();
    let key = estimate_key(&pitched);

    // 段落：曲式，或每 8 小节。
    let windows: Vec<(String, usize, f64, f64, Vec<muser_core::chord::Placed>)> = if spans.is_empty() {
        (0..bars.div_ceil(8))
            .map(|i| {
                let (a, b) = (i * 8, ((i + 1) * 8).min(bars));
                (format!("bars {}–{}", a + 1, b), 1, a as f64 * bar_beats, (b - a) as f64 * bar_beats, Vec::new())
            })
            .collect()
    } else {
        spans.iter().map(|s| (s.name.clone(), s.occurrence, s.start, s.length, s.chords.clone())).collect()
    };

    // 每小节的指纹（所有音轨的起音位置与音高），算重复度。
    let fingerprint = |b: usize| -> Vec<(usize, i64, u8)> {
        let (a, z) = (b as f64 * bar_beats, (b + 1) as f64 * bar_beats);
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
        let nbars = (length / bar_beats).round().max(1.0) as usize;
        let first_bar = (a / bar_beats).round() as usize;
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
                let (x, y) = (a + b as f64 * bar_beats, a + (b + 1) as f64 * bar_beats);
                if let Some((n, _)) = guess_chord(&weighted, x, y, key) {
                    names.push(format!("~{n}"));
                }
            }
            let changes = names.windows(2).filter(|w| w[0] != w[1]).count();
            (names, changes)
        };
        // 冲突。
        let mut clashes: Vec<Clash> = Vec::new();
        let at = |t: f64| ((t / bar_beats).floor() as usize + 1, round2((t % bar_beats) + 1.0));
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
                    clashes.push(Clash { bar, beat, what: format!("{tn} 的 {} 在 {} 上是避免音（比和弦音 {below} 高半音）", name(n.pitch), c.text) });
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
                    let kind = if hi - lo == 1 { "小二度" } else { "小九度" };
                    clashes.push(Clash { bar, beat, what: format!("{ta} 的 {} 与 {tb} 的 {} 成{kind}", name(x.pitch), name(y.pitch)) });
                }
            }
        }
        clashes.sort_by(|p, q| (p.bar, p.beat).partial_cmp(&(q.bar, q.beat)).unwrap());
        clashes.dedup_by(|p, q| p.what == q.what);
        clashes.truncate(8);
        let nonempty = (first_bar..(first_bar + nbars).min(bars)).filter(|&b| !prints[b].is_empty()).count();
        let rep = (first_bar..(first_bar + nbars).min(bars)).filter(|&b| repeated_bar[b]).count();
        let audio_stat = rendered.as_ref().map(|buf| {
            let secs = |beats: f64| beats * 60.0 / p.tempo;
            let (fa, fz) = ((secs(a) * sr) as usize, ((secs(z) * sr) as usize).min(buf.num_frames()));
            let mut part = crate::AudioBuffer::<f32>::new(2, fz.saturating_sub(fa).max(1));
            for c in 0..2 {
                let n = part.num_frames().min(buf.num_frames().saturating_sub(fa));
                part.channel_mut(c)[..n].copy_from_slice(&buf.channel(c)[fa..fa + n]);
            }
            let an = crate::audition::analyze(&part, sr, (0, part.num_frames()));
            AudioStat { lufs: (an.lufs * 10.0).round() / 10.0, centroid_hz: an.centroid_hz.round(), low_pct: an.low_pct.round(), mid_pct: an.mid_pct.round(), high_pct: an.high_pct.round() }
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
    let tells = tells(project, &sections, &tracks, &prints, &unhumanized, key, bar_beats);
    let mut findings = findings(&sections, &repeated_bar, &prints);
    findings.retain(|f| !f.starts_with("各段响度只相差") && f != "没有发现明显的问题");
    findings.extend(tells.iter().filter(|t| t.hit).map(|t| format!("AI 味 #{} {}：{}", t.id, t.name, t.detail)));
    if findings.is_empty() {
        findings.push("没有发现明显的问题".into());
    }
    Ok(Report { tempo: p.tempo, bars, key: key.name().to_string(), sections, tells, findings })
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
    bar_beats: f64,
) -> Vec<Tell> {
    let mut out = Vec::new();
    let mut tell = |id: u8, name: &str, hit: bool, detail: String| out.push(Tell { id, name: name.into(), hit, detail });
    let n = sections.len();
    let has_form = !project.form.is_empty();
    if has_form && n >= 3 {
        let all8 = sections.iter().all(|s| s.bars % 8 == 0);
        tell(1, "段落都是 8 小节的倍数", all8, if all8 {
            "真实的歌常有 2 / 4 / 6 / 10 / 12 小节的段落：把前奏、预副歌或尾声改短 / 加长，或加一段 2 小节的过渡".into()
        } else { "有不是 8 小节倍数的段落".into() });
    }
    if n >= 3 {
        let e: Vec<u8> = sections.iter().map(|s| s.energy).collect();
        let rising = e.windows(2).all(|w| w[1] >= w[0]);
        tell(2, "能量只升不降", rising, if rising {
            format!("能量 {:?} 从不下降（实测的真实编曲 62/62 都有下降）：让间奏、bridge 或最后一遍副歌前撤掉几件乐器（form 的 mute，或这一段不写 part）", e)
        } else { format!("能量 {:?} 有起有落", e) });
        let present: Vec<Vec<&str>> = sections.iter().map(|s| s.tracks.iter().map(|t| t.name.as_str()).collect()).collect();
        let used: std::collections::BTreeSet<&str> = present.iter().flatten().copied().collect();
        let constant = used.len() >= 2 && present.iter().all(|p| p.len() == used.len());
        tell(3, "编制从头到尾不变", constant, if constant {
            "每条轨每段都在：让乐器有进有出——前奏少几件、副歌再进来、某段撤掉鼓或贝斯（form 的 mute，或不写那一段的 part）".into()
        } else { "乐器有进有出".into() });
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
            tell(4, "全曲一个和声循环 / 不离调", hit, if one_loop {
                format!("每段都是同一个进行（{}）：给 pre / bridge / 副歌写不同的 chords", progs[0])
            } else if hit {
                "全曲只用调内和弦、也没有转调：在 bridge 或 pre 里用一个离调和弦（借用和弦如 iv、bVII，副属和弦如 V/vi），或最后一遍副歌 transpose 转调".into()
            } else { "和声有变化".into() });
        }
    }
    // 过门：鼓轨里通鼓 ≥ 3 下、或最后两拍军鼓 ≥ 4 下的小节。
    let drum_tones: Vec<&Tone> = tracks.iter().filter(|t| t.1).flat_map(|t| t.2.iter()).collect();
    if !drum_tones.is_empty() {
        let fill_bars: Vec<usize> = (0..prints.len())
            .filter(|&b| {
                let (a, z) = (b as f64 * bar_beats, (b + 1) as f64 * bar_beats);
                let toms = drum_tones.iter().filter(|t| t.start >= a && t.start < z && [41, 43, 45, 47, 48, 50].contains(&t.pitch)).count();
                let rolls = drum_tones.iter().filter(|t| t.start >= z - 2.0 && t.start < z && [37, 38, 40].contains(&t.pitch)).count();
                toms >= 3 || rolls >= 4
            })
            .collect();
        if fill_bars.len() >= 3 {
            let rigid = fill_bars.iter().all(|b| (b + 1) % 4 == 0) && fill_bars.len() * 4 >= prints.len() * 3 / 4;
            let shown: Vec<String> = fill_bars.iter().take(12).map(|b| (b + 1).to_string()).collect();
            tell(5, "过门刻板地落在 4 的倍数小节", rigid, if rigid {
                format!("过门在第 {} 小节，每 4 小节一次：drummer 不写 fill_every（自动在段尾与随机的乐句末尾打），或用 end 在段落结尾写过门", shown.join("、"))
            } else { format!("过门在第 {} 小节", shown.join("、")) });
        }
    }
    let pitched = tracks.iter().filter(|t| !t.1 && !t.2.is_empty()).count();
    if pitched > 0 {
        let hit = unhumanized.len() == pitched;
        tell(8, "旋律 / 和声全部对齐网格", hit, if hit {
            format!("{} 都没有 humanize：给旋律、贝斯、和声加 humanize（timing_ms、drift，或 push / lay-back），鼓组可以保持对齐", unhumanized.join("、"))
        } else if unhumanized.is_empty() { "旋律 / 和声轨都有 humanize".into() } else { format!("没有 humanize 的轨：{}", unhumanized.join("、")) });
    }
    // 副歌每遍一模一样：各遍的小节指纹完全相同。
    let is_chorus = |n: &str| { let n = n.to_lowercase(); ["chorus", "drop", "hook", "副歌"].iter().any(|k| n.contains(k)) };
    let choruses: Vec<&SectionReport> = sections.iter().filter(|s| is_chorus(&s.name)).collect();
    if choruses.len() >= 2 {
        let body = |s: &SectionReport| -> Vec<Vec<(usize, i64, u8)>> { (s.bar - 1..s.bar - 1 + s.bars).filter_map(|b| prints.get(b).cloned()).collect() };
        let same = choruses.windows(2).all(|w| body(w[0]) == body(w[1]));
        tell(11, "副歌每遍一模一样", same, if same {
            "副歌各遍的音一模一样：后几遍用 chorus#2 换写法、加声部、vary，或最后一遍 transpose".into()
        } else { "副歌各遍有变化".into() });
    }
    let audio: Vec<&AudioStat> = sections.iter().filter_map(|s| s.audio.as_ref()).filter(|a| a.lufs.is_finite()).collect();
    if audio.len() >= 3 {
        let (lo, hi) = audio.iter().fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x.lufs), b.max(x.lufs)));
        let mean = audio.iter().map(|a| a.centroid_hz).sum::<f64>() / audio.len() as f64;
        let sd = (audio.iter().map(|a| (a.centroid_hz - mean).powi(2)).sum::<f64>() / audio.len() as f64).sqrt();
        let flat = hi - lo < 1.5 || sd < 0.04 * mean;
        tell(7, "动态压死、明暗不变", flat, if flat {
            format!("各段响度只差 {:.1} LU、频谱重心变化 {:.0}%：让前奏 / 间奏更空更暗，副歌更满更亮（撤乐器、换音区、eq / 滤波自动化）", hi - lo, 100.0 * sd / mean.max(1.0))
        } else { format!("各段响度相差 {:.1} LU", hi - lo) });
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
                "副歌的密度（{c:.1} 音/小节）不高于主歌（{v:.1}）{}：副歌没有起来，可以加声部、提高音区、加满鼓或换更密的音型",
                louder.map_or(String::new(), |d| format!("、响度只差 {d:+.1} LU"))
            ));
        }
    }
    let nonempty = prints.iter().filter(|p| !p.is_empty()).count();
    let rep = repeated_bar.iter().filter(|&&r| r).count();
    if nonempty >= 8 && rep as f64 > 0.75 * nonempty as f64 {
        out.push(format!(
            "{}% 的小节和前面某小节完全一样：用 transform（vary、every）、end 过门或换一种 part 加变化",
            (100.0 * rep as f64 / nonempty as f64).round()
        ));
    }
    let distinct: std::collections::BTreeSet<&str> = sections.iter().flat_map(|s| s.chords.iter().map(|c| c.trim_start_matches('~'))).collect();
    if distinct.len() == 1 && nonempty >= 8 {
        out.push(format!("全曲只有一个和弦（{}）：如果不是刻意的 drone / 单和弦律动，可以在 sections 里写和弦进行", distinct.iter().next().unwrap()));
    }
    // 音区重叠：同一段里两条旋律 / 和声音轨长时间发声、中位音高相差不到 5 个半音。
    let mut masked: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for s in sections {
        let ts: Vec<&TrackStat> = s.tracks.iter().filter(|t| !t.drums && t.sounding >= 0.5).collect();
        for (i, x) in ts.iter().enumerate() {
            for y in ts.iter().skip(i + 1) {
                if let (Some(a), Some(b)) = (x.median, y.median) {
                    if a.abs_diff(b) < 5 {
                        masked.entry((x.name.clone(), y.name.clone())).or_default().push(s.name.clone());
                    }
                }
            }
        }
    }
    for ((a, b), secs) in masked {
        let mut secs = secs;
        secs.dedup();
        out.push(format!("{a} 与 {b} 在 {} 里同一音区长时间同时发声，可能互相掩蔽：拉开八度、换转位，或让一个休息", secs.join("、")));
    }
    let clashes: usize = sections.iter().map(|s| s.clashes.len()).sum();
    if clashes > 0 {
        out.push(format!("有 {clashes} 处强拍长音冲突（见各段 clashes）：确认是不是想要的张力，否则换成和弦音或改和弦"));
    }
    let lufs: Vec<f64> = sections.iter().filter_map(|s| s.audio.as_ref().map(|a| a.lufs)).filter(|l| l.is_finite()).collect();
    if lufs.len() >= 3 {
        let (lo, hi) = lufs.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        if hi - lo < 1.5 {
            out.push(format!("各段响度只相差 {:.1} LU：缺少动态起伏，可以让前奏 / 间奏更空，副歌更满", hi - lo));
        }
    }
    // 明暗看频谱重心：完整的混音一般在 1500–3000 Hz（低频的能量占比在混音里总是很高，不作判断）。
    for (sec, a) in sections.iter().filter_map(|s| s.audio.as_ref().map(|a| (s, a))).filter(|(_, a)| a.lufs.is_finite()) {
        if a.centroid_hz < 1000.0 {
            out.push(format!("{} 的频谱重心只有 {} Hz：偏暗、偏闷，可以加踩镲 / shaker、给和声轨 high_shelf 提亮，或给非贝斯轨加 highpass", sec.name, a.centroid_hz));
        } else if a.centroid_hz > 4500.0 {
            out.push(format!("{} 的频谱重心到了 {} Hz：偏亮，可能刺耳，可以降低镲片 / 高音轨，或 lowpass", sec.name, a.centroid_hz));
        }
    }
    if out.is_empty() {
        out.push("没有发现明显的问题".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(v.clashes.iter().any(|c| c.what.contains("F4") && c.what.contains("避免音")), "{:?}", v.clashes);
        assert!(r.findings.iter().any(|f| f.contains("副歌没有起来")), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.contains("完全一样")), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.contains("只有一个和弦")), "{:?}", r.findings);
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
        // 一段 6 小节、能量回落、贝斯退出、借用和弦 Fm、keys 有 humanize。
        assert_eq!(hits(&mk("drop", true)), Vec::<u8>::new());
    }
}
