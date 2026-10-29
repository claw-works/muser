//! 生成式鼓手（[`Drummer`]）：以 loop 库里的节奏型为底，每小节按复杂度加变化，每隔几小节打过门。
//!
//! - 变化（复杂度越高越多）：军鼓鬼音、底鼓切分、踩镲加花（加 16 分、偶尔开镲）。
//! - 过门：缺省自动——clip 最后一小节一定打；中间每个 4 小节乐句的末尾按种子约三分之一的概率打，长度 1 或 2 拍不等
//!   （真实编曲的过门落在乐句末尾，但不会每 4 小节一次、一样长）。写 `fill_every: N` 则每 N 小节一组、组末一定打，
//!   长 1 拍（复杂度 > 0.6 时 2 拍）。过门是军鼓滚奏 / 通鼓下行 / 军鼓与通鼓交替（按种子选），力度渐强；过门后的下一小节第一拍加镲。
//! - 力度：整体乘 `0.6 + 0.8 × intensity`（0.5 时不变）。
//! - 每小节用自己的种子（工程 seed、音轨、clip、小节号推导），所以小节之间不完全一样；同一种子结果相同。

use muser_core::automation::SplitMix64;
use muser_core::grid::Drummer;
use muser_core::{Beats, Note};

use crate::loop_library;

const KICK: u8 = 36;
const SNARE: u8 = 38;
const HAT: u8 = 42;
const OPEN_HAT: u8 = 46;
const CRASH: u8 = 49;
/// 通鼓从高到低。
const TOMS: [u8; 6] = [50, 48, 47, 45, 43, 41];

fn chance(rng: &mut SplitMix64, p: f64) -> bool {
    rng.unit() < p
}

/// 在 `length` 拍里展开（相对 clip 起点，按 4/4）。`dir` 是工程文件所在目录（找项目里的 loop）。
pub fn expand(d: &Drummer, length: f64, seed: u64, dir: Option<&std::path::Path>) -> Result<Vec<Note>, String> {
    let base = loop_library::load_for(&d.base, dir).map_err(|e| format!("drummer.loop: {e}"))?;
    let cycle = base.grid.expand(base.beats())?;
    let step = base.grid.step.unwrap_or(0.25).min(0.25);
    let bars = (length / 4.0 - 1e-9).ceil().max(0.0) as u32;
    let has = |pitch: u8| cycle.iter().any(|n| n.pitch == pitch);
    // 军鼓用 loop 里实际在用的那个（snare / rim / clap）。
    let snare = [SNARE, 37, 39, 40].into_iter().find(|&p| has(p)).unwrap_or(SNARE);
    let scale = 0.6 + 0.8 * d.intensity;
    let c = d.complexity;
    let mut out: Vec<Note> = Vec::new();
    let note = |pitch: u8, start: f64, velocity: f64| Note { pitch, start: Beats(start), length: Beats(step), velocity: velocity as f32, accent: false, bend: Vec::new() };

    // 哪些小节打过门、打几拍。
    let fills: Vec<Option<f64>> = (0..bars)
        .map(|bar| {
            let mut r = SplitMix64(seed ^ u64::from(bar).wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ 0x4649_4c4c);
            match d.fill_every {
                Some(n) => (n > 0 && (bar + 1) % n == 0).then_some(if c > 0.6 { 2.0 } else { 1.0 }),
                None => {
                    let last = bar + 1 == bars && bars >= 2;
                    let phrase_end = (bar + 1) % 4 == 0 && r.unit() < 0.35;
                    (last || phrase_end).then(|| if r.unit() < 0.3 + 0.4 * c { 2.0 } else { 1.0 })
                }
            }
        })
        .collect();
    for bar in 0..bars {
        let mut rng = SplitMix64(seed ^ u64::from(bar).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x6472_756D);
        let bar_start = f64::from(bar) * 4.0;
        let in_loop = f64::from(bar % base.bars) * 4.0;
        let fill_bar = fills[bar as usize].is_some();
        let fill_len = fills[bar as usize].unwrap_or(1.0);
        let fill_from = 4.0 - fill_len;
        let mut bar_notes: Vec<Note> = cycle
            .iter()
            .filter(|n| n.start.0 >= in_loop - 1e-9 && n.start.0 < in_loop + 4.0 - 1e-9)
            .map(|n| Note { start: Beats(n.start.0 - in_loop), ..n.clone() })
            .filter(|n| !(fill_bar && n.start.0 >= fill_from - 1e-9 && n.pitch != KICK))
            .collect();
        let taken = |notes: &[Note], pitch: u8, at: f64| notes.iter().any(|n| n.pitch == pitch && (n.start.0 - at).abs() < 1e-6);
        let any_at = |notes: &[Note], at: f64| notes.iter().any(|n| (n.start.0 - at).abs() < 1e-6 && n.pitch != HAT && n.pitch != OPEN_HAT);
        let grid_steps = (4.0 / step).round() as usize;
        let mut opened = false;
        for k in 0..grid_steps {
            let at = k as f64 * step;
            if fill_bar && at >= fill_from - 1e-9 {
                break;
            }
            let on_beat = (at.fract()).abs() < 1e-6;
            // 军鼓鬼音：反拍上的空位。
            if !on_beat && !any_at(&bar_notes, at) && chance(&mut rng, c * 0.16) {
                bar_notes.push(note(snare, at, 0.28));
            }
            // 底鼓切分：16 分反拍的空位，不和军鼓撞。
            if !on_beat && !taken(&bar_notes, snare, at) && !taken(&bar_notes, KICK, at) && chance(&mut rng, c * 0.1) {
                bar_notes.push(note(KICK, at, 0.65));
            }
            if has(HAT) {
                // 踩镲加花：在两个踩镲之间补一个轻的。
                if !taken(&bar_notes, HAT, at) && !taken(&bar_notes, OPEN_HAT, at) && taken(&bar_notes, HAT, at - step) && chance(&mut rng, c * 0.2) {
                    bar_notes.push(note(HAT, at, 0.4));
                }
                // 偶尔把一个反拍踩镲换成开镲（每小节最多一次）。
                if !opened && !on_beat && taken(&bar_notes, HAT, at) && chance(&mut rng, c * 0.12) {
                    bar_notes.retain(|n| !(n.pitch == HAT && (n.start.0 - at).abs() < 1e-6));
                    bar_notes.push(note(OPEN_HAT, at, 0.7));
                    opened = true;
                }
            }
        }
        if fill_bar {
            let steps = (fill_len / 0.25).round() as usize;
            let kind = rng.next_u64() % 3;
            let sparse = c < 0.3;
            for k in 0..steps {
                if sparse && k % 2 == 1 {
                    continue;
                }
                let at = fill_from + k as f64 * 0.25;
                let v = 0.55 + 0.45 * (k as f64 + 1.0) / steps as f64;
                let pitch = match kind {
                    0 => snare,
                    1 => TOMS[(k * TOMS.len()) / steps],
                    _ if k % 2 == 0 => snare,
                    _ => TOMS[(k * TOMS.len()) / steps],
                };
                bar_notes.push(note(pitch, at, v));
            }
            if !taken(&bar_notes, KICK, fill_from) {
                bar_notes.push(note(KICK, fill_from, 0.8));
            }
        }
        if d.crash && bar > 0 && fills[bar as usize - 1].is_some() {
            bar_notes.push(note(CRASH, 0.0, 0.9));
        }
        for mut n in bar_notes {
            n.start = Beats(n.start.0 + bar_start);
            if n.start.0 >= length - 1e-9 {
                continue;
            }
            n.velocity = (f64::from(n.velocity) * scale).clamp(0.05, 1.0) as f32;
            out.push(n);
        }
    }
    out.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drummer(c: f64) -> Drummer {
        serde_json::from_value(serde_json::json!({ "loop": "rock_basic", "complexity": c, "fill_every": 4 })).unwrap()
    }

    /// 自动过门：最后一小节一定有；不是每个 4 小节都有；换种子位置不同。
    #[test]
    fn auto_fills_vary() {
        let d: Drummer = serde_json::from_value(serde_json::json!({ "loop": "rock_basic", "complexity": 0.5 })).unwrap();
        let fill_bars = |seed: u64| -> Vec<u32> {
            let n = expand(&d, 32.0 * 4.0, seed, None).unwrap();
            (0..32).filter(|&b| n.iter().any(|x| TOMS.contains(&x.pitch) && x.start.0 >= f64::from(b) * 4.0 && x.start.0 < f64::from(b + 1) * 4.0)
                || n.iter().filter(|x| x.pitch == SNARE && x.start.0 >= f64::from(b) * 4.0 + 3.0 && x.start.0 < f64::from(b + 1) * 4.0).count() >= 3).collect()
        };
        let (a, b) = (fill_bars(1), fill_bars(2));
        assert!(a.contains(&31) && b.contains(&31), "最后一小节有过门：{a:?}");
        assert!(a.len() < 8, "不是每 4 小节都打：{a:?}");
        assert_ne!(a, b);
    }

    #[test]
    fn plain_at_zero_complexity_except_fills() {
        let base = loop_library::load("rock_basic").unwrap();
        let one = base.grid.expand(4.0).unwrap();
        let notes = expand(&drummer(0.0), 12.0, 1, None).unwrap();
        // 前三小节照着 loop 打（没有过门）。
        assert_eq!(notes.len(), one.len() * 3);
    }

    #[test]
    fn fills_crashes_and_variation() {
        let notes = expand(&drummer(0.8), 32.0, 7, None).unwrap();
        let toms_or_snare_in_bar4 = notes.iter().filter(|n| n.start.0 >= 14.0 && n.start.0 < 16.0 && (n.pitch == SNARE || TOMS.contains(&n.pitch))).count();
        assert!(toms_or_snare_in_bar4 >= 6, "第 4 小节末有过门：{toms_or_snare_in_bar4}");
        assert!(notes.iter().any(|n| n.pitch == CRASH && n.start.0 == 16.0), "过门后第一拍有镲");
        // 各小节不完全一样。
        let bar = |b: f64| notes.iter().filter(|n| n.start.0 >= b * 4.0 && n.start.0 < b * 4.0 + 4.0).map(|n| (n.pitch, ((n.start.0 - b * 4.0) * 4.0).round() as i64)).collect::<Vec<_>>();
        assert!(bar(0.0) != bar(1.0) || bar(1.0) != bar(2.0));
        // 同种子相同，换种子不同。
        assert_eq!(notes, expand(&drummer(0.8), 32.0, 7, None).unwrap());
        assert_ne!(notes, expand(&drummer(0.8), 32.0, 8, None).unwrap());
    }

    #[test]
    fn intensity_scales_velocity_and_unknown_loop_errors() {
        let soft: Drummer = serde_json::from_value(serde_json::json!({ "loop": "rock_basic", "intensity": 0.0, "complexity": 0, "fill_every": 0 })).unwrap();
        let loud = Drummer { intensity: 1.0, ..soft.clone() };
        let (a, b) = (expand(&soft, 4.0, 1, None).unwrap(), expand(&loud, 4.0, 1, None).unwrap());
        assert!(a.iter().zip(&b).all(|(x, y)| x.velocity < y.velocity || y.velocity == 1.0));
        let bad = Drummer { base: "nope".into(), ..soft };
        assert!(expand(&bad, 4.0, 1, None).unwrap_err().contains("drummer.loop"));
    }
}
