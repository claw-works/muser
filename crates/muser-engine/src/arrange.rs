//! 展开编曲层（[`muser_core::arrange`]）：曲式里每个段落的每一遍、每条有 `parts` 的音轨，生成一个普通 clip。
//! 渲染、导出、view 都只看展开后的工程。
//!
//! - 普通内容（notes / abc / grid / drummer）按 `length` 循环铺满段落；没写 `length` 就是整段。
//! - `chordal` 按当前和弦取音：`1 3 5 7` 是和弦实际的根音、三音、五音、七音（没有七音时 `7` 取八度），
//!   `c` 整个和弦，按和上一个和弦最近的转位排（声部进行），`B` 低音（斜线和弦的低音）。
//! - `transform` 每一遍循环施加；`every` 按遍数，`vary` 带种子（同一种子结果相同）。
//! - `end` 替换每一遍段落的最后几拍；`form` 里的 `transpose` 移调（和弦一起移，鼓组不动），`mute` 让音轨这一遍休息。

use std::borrow::Cow;

use muser_core::arrange::{self, Chordal, Part, Step, Transform};
use muser_core::automation::{SplitMix64, derive_seed};
use muser_core::chord::{self, Chord, Placed};
use muser_core::{Beats, Clip, Note, Project};

/// 展开后的一个段落：名字、第几次（从 1 起）、起点与长度（拍）、和弦（起点从工程开头算，已移调）。
#[derive(Debug, Clone)]
pub struct SectionSpan {
    pub name: String,
    pub occurrence: usize,
    pub start: f64,
    pub length: f64,
    pub chords: Vec<Placed>,
}

/// 曲式的时间线。没有 `form` 时为空。
pub fn timeline(project: &Project) -> Result<Vec<SectionSpan>, String> {
    let bar = project.bar_beats();
    let mut out = Vec::new();
    let mut count = std::collections::HashMap::<&str, usize>::new();
    let mut t = 0.0;
    for (fi, e) in project.form.iter().enumerate() {
        let name = e.section();
        let sec = project.sections.get(name).ok_or_else(|| format!("form[{fi}]: 没有段落 '{name}'"))?;
        let k = count.entry(name).or_default();
        *k += 1;
        let length = f64::from(sec.bars) * bar;
        let chords = match &sec.chords {
            Some(c) => chord::progression(c, sec.bars, bar)
                .map_err(|e| format!("sections.{name}.chords: {e}"))?
                .into_iter()
                .map(|p| Placed { start: p.start + t, length: p.length, chord: p.chord.transposed(e.transpose()) })
                .collect(),
            None => Vec::new(),
        };
        out.push(SectionSpan { name: name.to_string(), occurrence: *k, start: t, length, chords });
        t += length;
    }
    Ok(out)
}

/// 把编曲层展开成普通 clip。没有 `form`（或已经展开过）时原样返回。
pub fn arranged(project: &Project) -> Result<Cow<'_, Project>, String> {
    if project.form.is_empty() || project.tracks.iter().all(|t| t.parts.is_empty()) {
        return Ok(Cow::Borrowed(project));
    }
    let spans = timeline(project)?;
    let mut out = project.clone();
    for (fi, (entry, span)) in project.form.iter().zip(&spans).enumerate() {
        let occ_seed = entry.seed().unwrap_or_else(|| derive_seed(project.seed, None, 0x464f_0000 + fi));
        // 和弦的时间相对段落开头；先按原调取音，最后整体移调。
        let chords: Vec<Placed> = span
            .chords
            .iter()
            .map(|p| Placed { start: p.start - span.start, length: p.length, chord: p.chord.transposed(-entry.transpose()) })
            .collect();
        for (ti, track) in project.tracks.iter().enumerate() {
            if track.parts.is_empty() || entry.muted(&track.name) {
                continue;
            }
            let Some(part) = arrange::pick(&track.parts, &span.name, span.occurrence) else { continue };
            let at = format!("tracks[{ti}].parts.{}", span.name);
            let ctx = Ctx {
                project,
                drums: track.instrument.trim().starts_with("drums."),
                seed: SplitMix64(occ_seed ^ (ti as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).next_u64(),
            };
            let mut notes = ctx.render(part, span.length, &chords, &at)?;
            if let Some(end) = &part.end {
                let b = end.beats.unwrap_or(0.0).min(span.length);
                let cut = span.length - b;
                notes.retain(|n| n.start.0 < cut - 1e-9);
                for n in &mut notes {
                    n.length = Beats(n.length.0.min(cut - n.start.0));
                }
                let tail: Vec<Placed> = chords.iter().map(|p| Placed { start: p.start - cut, ..p.clone() }).collect();
                let end_ctx = Ctx { seed: ctx.seed ^ 0x0045_4e44, ..ctx };
                for mut n in end_ctx.render(end, b, &tail, &format!("{at}.end"))? {
                    n.start = Beats(n.start.0 + cut);
                    notes.push(n);
                }
            }
            if !ctx.drums && entry.transpose() != 0 {
                notes = notes.into_iter().filter_map(|n| shift_pitch(n, entry.transpose())).collect();
            }
            let loop_len = part.length.unwrap_or(span.length);
            let mut pedal = Vec::new();
            let mut t = 0.0;
            while !part.pedal.is_empty() && t < span.length - 1e-9 {
                pedal.extend(part.pedal.iter().map(|&(a, b)| (a + t, (b + t).min(span.length))).filter(|(a, b)| b > a));
                t += loop_len;
            }
            let mut clip = Clip::new(span.start, span.length);
            clip.notes = notes;
            clip.humanize = part.humanize.clone();
            clip.legato = part.legato;
            clip.pedal = pedal;
            clip.from = Some(format!("form:{}#{}", span.name, span.occurrence));
            out.tracks[ti].clips.push(clip);
        }
    }
    // 保留 form / sections（总长、段落名、和弦给导出与分析用）；parts 清空，再展开一次不会重复生成。
    for t in &mut out.tracks {
        t.parts.clear();
    }
    Ok(Cow::Owned(out))
}

fn shift_pitch(mut n: Note, semitones: i32) -> Option<Note> {
    let p = i32::from(n.pitch) + semitones;
    (0..=127).contains(&p).then(|| {
        n.pitch = p as u8;
        n
    })
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    project: &'a Project,
    drums: bool,
    seed: u64,
}

impl Ctx<'_> {
    /// 一个 part 铺满 `len` 拍的音符（相对段落开头）。`chords` 的时间也相对段落开头。
    fn render(&self, part: &Part, len: f64, chords: &[Placed], at: &str) -> Result<Vec<Note>, String> {
        let steps = part.chordal.as_deref().map(Chordal::parse).transpose().map_err(|e| format!("{at}.{e}"))?;
        if steps.is_some() && chords.is_empty() {
            return Err(format!("{at}: chordal 要按和弦取音，这个段落没有写 chords"));
        }
        let step = part.step.unwrap_or(0.5);
        let loop_len = part.length.unwrap_or_else(|| steps.as_ref().map_or(len, |s| s.len() as f64 * step));
        let mut voicing: Vec<i32> = Vec::new();
        let mut out = Vec::new();
        let (mut t, mut k) = (0.0, 0usize);
        while t < len - 1e-9 && k < 100_000 {
            let mut notes = match &steps {
                Some(s) => chordal_notes(s, step, loop_len, t, chords, part, &mut voicing),
                None => self.content(part, loop_len, k, at)?,
            };
            // 本遍循环里落在和弦上的位置（vary 取和弦音用）。
            let rep_chords: Vec<Placed> = chords.iter().map(|p| Placed { start: p.start - t, ..p.clone() }).collect();
            let mut l = loop_len;
            apply(&part.transform, &mut notes, &mut l, k, &rep_chords, self.drums, self.seed ^ (k as u64).wrapping_mul(0xA076_1D64_78BD_642F));
            for mut n in notes {
                n.start = Beats(n.start.0 + t);
                if n.start.0 < len - 1e-9 {
                    n.length = Beats(n.length.0.min(len - n.start.0));
                    out.push(n);
                }
            }
            t += l.max(1e-3);
            k += 1;
        }
        Ok(out)
    }

    /// 普通内容一遍循环（`len` 拍）的音符：notes + abc + grid + drummer。
    fn content(&self, part: &Part, len: f64, k: usize, at: &str) -> Result<Vec<Note>, String> {
        let mut notes = part.notes.clone();
        if let Some(a) = &part.abc {
            notes.extend(muser_core::abc::parse(a).map_err(|e| format!("{at}.abc: {e}"))?);
        }
        if let Some(g) = &part.grid {
            notes.extend(g.expand(len).map_err(|e| format!("{at}.grid: {e}"))?);
        }
        if let Some(d) = &part.drummer {
            let seed = d.seed.unwrap_or_else(|| SplitMix64(self.seed ^ 0x4452 ^ k as u64).next_u64());
            notes.extend(crate::drummer::expand(d, len, seed, self.project.base_dir.as_deref()).map_err(|e| format!("{at}.{e}"))?);
        }
        let _ = self.project;
        notes.retain(|n| n.start.0 < len - 1e-9);
        for n in &mut notes {
            n.length = Beats(n.length.0.min(len - n.start.0));
        }
        Ok(notes)
    }
}

/// 大调音阶上第几级的半音数（显式升降号 `b7`、`#11` 以它为准）。
fn scale_interval(deg: u8) -> i32 {
    match deg {
        1 => 0, 2 => 2, 3 => 4, 4 => 5, 5 => 7, 6 => 9, 7 => 11, 8 => 12, 9 => 14, 11 => 17, _ => 21,
    }
}

/// 和弦的第 `deg` 个音相对根音的半音数：按和弦实际的音（小三和弦的 3 是小三度，减和弦的 5 是减五度）。
fn chord_interval(c: &Chord, deg: u8, acc: i8) -> i32 {
    if acc != 0 {
        return scale_interval(deg) + i32::from(acc);
    }
    let has = |lo: u8, hi: u8| c.intervals.iter().copied().find(|&i| (lo..=hi).contains(&i)).map(i32::from);
    match deg {
        1 => 0,
        3 => i32::from(c.third()),
        5 => i32::from(c.fifth()),
        7 => c.seventh().map_or(12, i32::from),
        8 => 12,
        9 => has(13, 15).unwrap_or(14),
        11 => has(17, 18).unwrap_or(17),
        13 => has(20, 21).unwrap_or(21),
        6 => 9,
        d => scale_interval(d),
    }
}

/// chordal 音型一遍循环的音符（相对这一遍开头）。`rep_start` 是这一遍在段落里的起点（查和弦用）。
fn chordal_notes(steps: &[Step], step: f64, loop_len: f64, rep_start: f64, chords: &[Placed], part: &Part, voicing: &mut Vec<i32>) -> Vec<Note> {
    let octave = part.octave.unwrap_or(3);
    let base = 12 * (octave + 1); // 这一组的 C
    let vel = part.velocity.unwrap_or(0.8);
    let mut out: Vec<Note> = Vec::new();
    let mut sounding: Vec<usize> = Vec::new();
    let cells = (loop_len / step - 1e-9).ceil() as usize;
    for i in 0..cells {
        let t = i as f64 * step;
        let Some(c) = chord::at(chords, rep_start + t) else { continue };
        let root = base + i32::from(c.root);
        let note = |pitch: i32, accent: bool| {
            (0..=127).contains(&pitch).then_some(Note { pitch: pitch.clamp(0, 127) as u8, start: Beats(t), length: Beats(step), velocity: if accent { 1.0 } else { vel }, accent, bend: Vec::new() })
        };
        let new: Vec<Note> = match steps[i % steps.len()] {
            Step::Rest => {
                sounding.clear();
                continue;
            }
            Step::Hold => {
                for &j in &sounding {
                    out[j].length = Beats(out[j].length.0 + step);
                }
                continue;
            }
            Step::Degree { deg, acc, oct, accent } => note(root + chord_interval(c, deg, acc) + 12 * i32::from(oct), accent).into_iter().collect(),
            Step::Bass { oct, accent } => note(base + i32::from(c.bass_pc()) + 12 * i32::from(oct), accent).into_iter().collect(),
            Step::Chord { oct, accent } => {
                *voicing = voice(c, base + 7 + 12 * i32::from(oct), voicing);
                voicing.iter().filter_map(|&p| note(p, accent)).collect()
            }
        };
        sounding = (out.len()..out.len() + new.len()).collect();
        out.extend(new);
    }
    out
}

/// 和弦的一个排列（密集排列的某个转位），离 `center` 近、和上一个排列 `prev` 衔接最近。
fn voice(c: &Chord, center: i32, prev: &[i32]) -> Vec<i32> {
    let mut pcs: Vec<i32> = Vec::new();
    for &i in &c.intervals {
        let pc = (i32::from(c.root) + i32::from(i)) % 12;
        if !pcs.contains(&pc) {
            pcs.push(pc);
        }
    }
    let mut best: Option<(f64, Vec<i32>)> = None;
    for r in 0..pcs.len() {
        for shift in [-12, 0, 12] {
            let mut v: Vec<i32> = Vec::new();
            for k in 0..pcs.len() {
                let pc = pcs[(r + k) % pcs.len()];
                let mut p = match v.last() {
                    None => center - 6 + (pc - (center - 6)).rem_euclid(12) + shift,
                    Some(&last) => last + 1 + (pc - (last + 1)).rem_euclid(12),
                };
                if p < 0 {
                    p += 12;
                }
                v.push(p);
            }
            let mean = v.iter().sum::<i32>() as f64 / v.len() as f64;
            // 排列里相邻两音成小二度（大七和弦的七音紧贴根音下面）很刺耳，尽量不用。
            let rough = v.windows(2).filter(|w| w[1] - w[0] == 1).count() as f64 * 6.0;
            let near = |a: &[i32], b: &[i32]| a.iter().map(|x| b.iter().map(|y| (x - y).abs()).min().unwrap_or(0) as f64).sum::<f64>();
            let cost = rough + if prev.is_empty() { (mean - f64::from(center)).abs() } else { near(&v, prev) + near(prev, &v) + 0.15 * (mean - f64::from(center)).abs() };
            if best.as_ref().is_none_or(|(b, _)| cost < *b - 1e-9) {
                best = Some((cost, v));
            }
        }
    }
    best.map(|(_, v)| v).unwrap_or_default()
}

/// 模进用的音阶（音级，升序）：这一段和弦里的音加上动机里的音；不够 5 个音时用第一个和弦根音（或 C）的大调。
fn sequence_scale(chords: &[Placed], notes: &[Note]) -> Vec<i32> {
    let mut pcs: Vec<i32> = chords.iter().flat_map(|p| p.chord.pitch_classes()).map(i32::from).chain(notes.iter().map(|n| i32::from(n.pitch % 12))).collect();
    pcs.sort_unstable();
    pcs.dedup();
    if pcs.len() < 5 {
        let root = chords.first().map_or(0, |p| i32::from(p.chord.root));
        pcs = [0, 2, 4, 5, 7, 9, 11].iter().map(|i| (root + i) % 12).collect();
        pcs.sort_unstable();
    }
    pcs
}

/// 在音阶上移 `steps` 级。不在音阶上的音先就近落到音阶上，移完再补回原来的偏移。
fn scale_step(pitch: i32, steps: i32, scale: &[i32]) -> i32 {
    if steps == 0 {
        return pitch;
    }
    // 音阶上的音（跨八度展开），找离 pitch 最近、不高于它的那个的位置。
    let at = |i: i32| scale[i.rem_euclid(scale.len() as i32) as usize] + 12 * i.div_euclid(scale.len() as i32);
    let mut i = (pitch.div_euclid(12)) * scale.len() as i32;
    while at(i + 1) <= pitch {
        i += 1;
    }
    while at(i) > pitch {
        i -= 1;
    }
    let off = pitch - at(i);
    at(i + steps) + off
}

/// 按顺序施加变换。`len` 是这一遍循环的长度（stretch 会改）；`k` 是第几遍（从 0 起）。
pub(crate) fn apply(ts: &[Transform], notes: &mut Vec<Note>, len: &mut f64, k: usize, chords: &[Placed], drums: bool, seed: u64) {
    for (ti, t) in ts.iter().enumerate() {
        let pitched = |n: &mut Vec<Note>, f: &dyn Fn(i32) -> i32| {
            if !drums {
                *n = std::mem::take(n).into_iter().filter_map(|x| {
                    let d = f(i32::from(x.pitch)) - i32::from(x.pitch);
                    shift_pitch(x, d)
                }).collect();
            }
        };
        if let Some(s) = t.transpose {
            pitched(notes, &|p| p + s);
        } else if let Some(o) = t.octave {
            pitched(notes, &|p| p + 12 * o);
        } else if t.reverse {
            for n in notes.iter_mut() {
                n.start = Beats((*len - n.start.0 - n.length.0).max(0.0));
            }
        } else if let Some(axis) = t.invert {
            pitched(notes, &|p| 2 * i32::from(axis) - p);
        } else if let Some(f) = t.stretch {
            for n in notes.iter_mut() {
                n.start = Beats(n.start.0 * f);
                n.length = Beats(n.length.0 * f);
            }
            *len *= f;
        } else if let Some(d) = t.shift {
            for n in notes.iter_mut() {
                n.start = Beats((n.start.0 + d).rem_euclid(*len));
            }
        } else if let Some(f) = t.fragment {
            notes.retain(|n| n.start.0 < f - 1e-9);
            for n in notes.iter_mut() {
                n.length = Beats(n.length.0.min(f - n.start.0));
            }
            *len = f;
        } else if !t.sequence.is_empty() {
            if !drums {
                let steps = t.sequence[k % t.sequence.len()];
                let scale = sequence_scale(chords, notes);
                for n in notes.iter_mut() {
                    n.pitch = scale_step(i32::from(n.pitch), steps, &scale).clamp(0, 127) as u8;
                }
            }
        } else if let Some(every) = t.every {
            if (k + 1) % every as usize == 0 {
                apply(&t.then, notes, len, k, chords, drums, seed ^ 0x4556_0000 ^ ti as u64);
            }
        } else if let Some(p) = t.vary {
            let mut rng = SplitMix64(t.seed.map_or(seed ^ 0x5641_5259 ^ ti as u64, |s| s ^ k as u64));
            let pattern: Vec<i32> = notes.iter().map(|n| i32::from(n.pitch)).collect();
            let mut keep = Vec::with_capacity(notes.len());
            for mut n in std::mem::take(notes) {
                let r = rng.unit();
                if r >= p {
                    keep.push(n);
                    continue;
                }
                if r < p / 3.0 {
                    continue; // 省掉
                }
                if drums {
                    n.velocity = (n.velocity * (0.55 + 0.5 * rng.unit() as f32)).clamp(0.05, 1.0);
                } else {
                    // 换成相邻的和弦音（没有和弦时，音型里出现过的音高）。
                    let up = rng.unit() < 0.5;
                    let here = i32::from(n.pitch);
                    let pcs: Vec<i32> = chord::at(chords, n.start.0).map_or_else(|| pattern.iter().map(|p| p.rem_euclid(12)).collect(), |c| c.pitch_classes().iter().map(|&x| i32::from(x)).collect());
                    let cand = (1..=12).map(|d| if up { here + d } else { here - d }).find(|q| pcs.contains(&q.rem_euclid(12)));
                    if let Some(q) = cand.filter(|q| (0..=127).contains(q)) {
                        n.pitch = q as u8;
                    }
                }
                keep.push(n);
            }
            *notes = keep;
        }
    }
    notes.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(v: serde_json::Value) -> Project {
        let p: Project = serde_json::from_value(v).unwrap();
        p.validate().unwrap();
        p
    }

    fn notes(p: &Project, track: usize) -> Vec<(f64, f64, u8)> {
        let a = arranged(p).unwrap();
        let mut v: Vec<(f64, f64, u8)> = a.tracks[track]
            .clips
            .iter()
            .flat_map(|c| c.notes.iter().map(move |n| (c.start.0 + n.start.0, n.length.0, n.pitch)))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        v
    }

    #[test]
    fn form_parts_and_occurrences() {
        let p = project(serde_json::json!({ "tempo": 120,
            "sections": { "a": { "bars": 1 }, "b": { "bars": 2 } },
            "form": ["a", "b", "a", { "section": "b", "mute": ["lead"] }],
            "tracks": [
                { "name": "lead", "instrument": "wind.flute", "parts": {
                    "a": { "notes": [{ "pitch": 60, "start": 0, "length": 1 }], "length": 2 },
                    "a#2": { "notes": [{ "pitch": 72, "start": 0, "length": 4 }] },
                    "*": { "abc": "L:1/4\nK:C\nG" } } }
            ] }));
        let n = notes(&p, 0);
        // a：每 2 拍一个 C（4 拍里两次）；b：整段 abc 一个 G；第二次 a 用 a#2；第二次 b 被 mute。
        assert_eq!(n, [(0.0, 1.0, 60), (2.0, 1.0, 60), (4.0, 1.0, 67), (12.0, 4.0, 72)]);
        let a = arranged(&p).unwrap();
        assert_eq!((a.duration_beats().0, a.tracks[0].clips.len(), a.tracks[0].clips[1].from.as_deref()), (24.0, 3, Some("form:b#1")));
    }

    #[test]
    fn chordal_follows_chords_and_voice_leads() {
        let p = project(serde_json::json!({ "tempo": 120,
            "sections": { "v": { "bars": 2, "chords": "Am C/E" } }, "form": ["v", { "section": "v", "transpose": 2 }],
            "tracks": [
                { "name": "bass", "instrument": "bass.finger", "parts": { "v": { "chordal": "1 5 B 8", "step": 1, "octave": 2 } } },
                { "name": "pad", "instrument": "poly.juno_keys", "parts": { "v": { "chordal": "c - - -", "step": 1, "octave": 4 } } }
            ] }));
        let b: Vec<u8> = notes(&p, 0).iter().map(|n| n.2).collect();
        // octave 2：根音在 C2–B2。Am：A2 E3 A2 A3；C/E：C2 G2 E2（斜线低音）C3；第二遍移高 2 个半音。
        assert_eq!(&b[..8], [45, 52, 45, 57, 36, 43, 40, 48]);
        assert_eq!(b[8], 47);
        let pad = notes(&p, 1);
        assert_eq!(pad.iter().filter(|n| n.0 == 0.0).count(), 3);
        assert!(pad.iter().filter(|n| n.0 == 0.0).all(|n| n.1 == 4.0), "c - - - 延长成一整小节");
        // 声部进行：Am → C/E 至少保留两个共同音（C、E）。
        let at = |t: f64| pad.iter().filter(|n| n.0 == t).map(|n| n.2).collect::<Vec<_>>();
        assert!(at(0.0).iter().filter(|p| at(4.0).contains(p)).count() >= 2, "{:?} → {:?}", at(0.0), at(4.0));
    }

    #[test]
    fn transforms_end_and_every() {
        let p = project(serde_json::json!({ "tempo": 120,
            "sections": { "s": { "bars": 2 } }, "form": ["s"],
            "tracks": [{ "name": "m", "instrument": "wind.flute", "parts": { "s": {
                "notes": [{ "pitch": 60, "start": 0, "length": 1 }, { "pitch": 64, "start": 1, "length": 1 }], "length": 2,
                "transform": [{ "every": 2, "then": [{ "reverse": true }, { "transpose": 12 }] }],
                "end": { "beats": 1, "notes": [{ "pitch": 50, "start": 0, "length": 1 }] } } } }] }));
        let n = notes(&p, 0);
        // 第 1、3 遍原样，第 2、4 遍逆行 + 高八度；最后 1 拍换成 end。
        assert_eq!(n, [(0.0, 1.0, 60), (1.0, 1.0, 64), (2.0, 1.0, 76), (3.0, 1.0, 72), (4.0, 1.0, 60), (5.0, 1.0, 64), (6.0, 1.0, 76), (7.0, 1.0, 50)]);
    }

    #[test]
    fn vary_is_seeded_and_drums_dont_transpose() {
        let mk = |seed: u64| project(serde_json::json!({ "tempo": 120, "seed": seed,
            "sections": { "s": { "bars": 4, "chords": "C" } }, "form": [{ "section": "s", "transpose": 5 }],
            "tracks": [
                { "name": "arp", "instrument": "poly.bell_pluck", "parts": { "s": { "chordal": "1 3 5 8", "step": 0.25, "transform": [{ "vary": 0.5 }] } } },
                { "name": "drums", "instrument": "drums.808", "parts": { "s": { "grid": { "kick": "x..." }, "transform": [{ "transpose": 7 }] } } }
            ] }));
        assert_eq!(notes(&mk(1), 0), notes(&mk(1), 0));
        assert_ne!(notes(&mk(1), 0), notes(&mk(2), 0));
        assert!(notes(&mk(1), 1).iter().all(|n| n.2 == 36));
        // 换成的都是和弦音（F 大三：移调 5 之后）。
        assert!(notes(&mk(1), 0).iter().all(|n| [5, 9, 0].contains(&(n.2 % 12))));
    }

    #[test]
    fn sequence_and_fragment() {
        let p = project(serde_json::json!({ "tempo": 120,
            "sections": { "s": { "bars": 2, "chords": "C" } }, "form": ["s"],
            "tracks": [{ "name": "m", "instrument": "wind.flute", "parts": { "s": {
                "notes": [{ "pitch": 67, "start": 0, "length": 1 }, { "pitch": 64, "start": 1, "length": 1 }, { "pitch": 60, "start": 2, "length": 2 }],
                "transform": [{ "fragment": 2 }, { "sequence": [0, -1, -2, -3] }] } } }] }));
        let n: Vec<u8> = notes(&p, 0).iter().map(|n| n.2).collect();
        // 只留前 2 拍（G E），C 大调上逐级下行：G E → F D → E C → D B。
        assert_eq!(n, [67, 64, 65, 62, 64, 60, 62, 59]);
        assert_eq!(scale_step(61, 1, &[0, 2, 4, 5, 7, 9, 11]), 63, "调外音保留偏移");
    }

    #[test]
    fn errors() {
        let p: Project = serde_json::from_value(serde_json::json!({ "tempo": 120, "sections": { "s": { "bars": 1 } }, "form": ["s"],
            "tracks": [{ "name": "b", "instrument": "bass.finger", "parts": { "s": { "chordal": "1" } } }] })).unwrap();
        assert!(arranged(&p).unwrap_err().contains("没有写 chords"));
        let bad: Project = serde_json::from_value(serde_json::json!({ "tempo": 120, "sections": { "s": { "bars": 1 } }, "form": ["x"], "tracks": [] })).unwrap();
        assert!(bad.validate().unwrap_err().to_string().contains("没有段落 'x'"));
    }
}
