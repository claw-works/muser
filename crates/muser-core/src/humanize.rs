//! 人性化演奏：给音符的时间、力度、时值加上像真人一样的偏移，再加律动（swing、整体提前/拖后）。
//!
//! 写在音轨（整轨默认）或 clip（覆盖音轨的同名字段）的 `humanize` 上，只写要用的项：
//!
//! | 字段 | 含义 | 常见取值 |
//! |---|---|---|
//! | `timing_ms` | 时间偏移的标准差（毫秒，正态分布，限在 ±3 倍内） | 3–15 |
//! | `drift` | 偏移的连贯程度：0 每个音各自随机；越接近 1 越像真人一样缓慢漂移（忽快忽慢） | 0–0.9 |
//! | `velocity` | 力度偏移的标准差（力度是 0–1） | 0.03–0.12 |
//! | `length` | 时值变化的标准差（比例） | 0–0.15 |
//! | `swing` | 摇摆（%）：50 是直的，66.7 是三连音感觉；反拍的音往后推 | 50–66 |
//! | `swing_unit` | 摇摆的单位（拍）：0.5 = 八分音符，0.25 = 十六分音符 | 0.5 / 0.25 |
//! | `push_ms` | 整体提前（负）或拖后（正），如军鼓 lay back +10 | −20–20 |
//! | `seed` | 随机种子；不写时由工程 `seed`、音轨与 clip 位置推导 | — |
//!
//! 同一个种子每次渲染结果完全相同（喜欢的一遍可以留住）；换种子就是换一遍演奏。
//! 同一时刻按下的音（和弦）用同一个时间偏移，力度各自变化。

use serde::{Deserialize, Serialize};

use crate::automation::SplitMix64;
use crate::Note;

/// 人性化设置。每项都可省略：clip 上写的覆盖音轨上的，都没写的按不变处理。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Humanize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swing: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swing_unit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// (字段, 最小, 最大)。
const RANGES: [(&str, f64, f64); 7] = [
    ("timing_ms", 0.0, 50.0),
    ("drift", 0.0, 0.99),
    ("velocity", 0.0, 0.5),
    ("length", 0.0, 0.5),
    ("swing", 50.0, 75.0),
    ("swing_unit", 0.125, 1.0),
    ("push_ms", -50.0, 50.0),
];

impl Humanize {
    fn values(&self) -> [Option<f64>; 7] {
        [self.timing_ms, self.drift, self.velocity, self.length, self.swing, self.swing_unit, self.push_ms]
    }

    /// 检查取值范围。`at` 是出错位置（如 `tracks[0].clips[1].humanize`）。
    pub fn validate(&self, at: &str) -> Result<(), String> {
        for ((name, min, max), v) in RANGES.iter().zip(self.values()) {
            if let Some(v) = v {
                if !(*min..=*max).contains(&v) {
                    return Err(format!("{at}.{name} = {v} 超出范围 [{min}, {max}]"));
                }
            }
        }
        Ok(())
    }

    /// `over`（clip）覆盖 `base`（音轨）的同名字段。都没有时为 `None`。
    pub fn merged(base: Option<&Humanize>, over: Option<&Humanize>) -> Option<Humanize> {
        match (base, over) {
            (None, None) => None,
            (Some(b), None) => Some(b.clone()),
            (None, Some(o)) => Some(o.clone()),
            (Some(b), Some(o)) => Some(Humanize {
                timing_ms: o.timing_ms.or(b.timing_ms),
                drift: o.drift.or(b.drift),
                velocity: o.velocity.or(b.velocity),
                length: o.length.or(b.length),
                swing: o.swing.or(b.swing),
                swing_unit: o.swing_unit.or(b.swing_unit),
                push_ms: o.push_ms.or(b.push_ms),
                seed: o.seed.or(b.seed),
            }),
        }
    }
}

/// 一个音实际怎么弹。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Performed {
    /// 起点（拍，相对 clip 起点；已加上 swing）。
    pub start: f64,
    /// 另加的时间偏移（秒，随机偏移 + push_ms）。
    pub offset_s: f64,
    /// 时值（拍）。
    pub length: f64,
    pub velocity: f32,
}

/// 标准正态分布（Box–Muller）。
fn gaussian(rng: &mut SplitMix64) -> f64 {
    let u = rng.unit().max(1e-12);
    let v = rng.unit();
    (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
}

/// 按 `h` 演奏一个 clip 的音符（`lengths` 是每个音要用的时值，如连奏延长后的）。`clip_start` 是 clip 在
/// 工程里的起点（拍）：swing 按工程的拍子网格算。结果与 `notes` 一一对应。
pub fn perform(h: &Humanize, notes: &[Note], lengths: &[f64], clip_start: f64, seed: u64) -> Vec<Performed> {
    let sigma = h.timing_ms.unwrap_or(0.0) / 1000.0;
    let rho = h.drift.unwrap_or(0.0);
    let vel_sd = h.velocity.unwrap_or(0.0);
    let len_sd = h.length.unwrap_or(0.0);
    let push = h.push_ms.unwrap_or(0.0) / 1000.0;
    let unit = h.swing_unit.unwrap_or(0.5);
    let swing_shift = (h.swing.unwrap_or(50.0) - 50.0) / 50.0 * unit;

    // 时间偏移按时间顺序走一个 AR(1) 过程：drift 越大，相邻的音偏移越相近（缓慢漂移）。
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].start.0.total_cmp(&notes[b].start.0).then(a.cmp(&b)));
    let mut timing_rng = SplitMix64(seed ^ 0x7469_6D69_6E67);
    let mut feel_rng = SplitMix64(seed ^ 0x6665_656C);
    let mut offsets = vec![0.0; notes.len()];
    let (mut state, mut last_start): (f64, Option<f64>) = (0.0, None);
    for &i in &order {
        let s = notes[i].start.0;
        // 同时按下的音（和弦）共用一个偏移。
        if last_start.is_none_or(|l| (s - l).abs() > 1e-6) {
            let z = gaussian(&mut timing_rng);
            state = if last_start.is_none() { z } else { rho * state + (1.0 - rho * rho).sqrt() * z };
            last_start = Some(s);
        }
        offsets[i] = (sigma * state).clamp(-3.0 * sigma, 3.0 * sigma) + push;
    }

    notes
        .iter()
        .zip(lengths)
        .zip(offsets)
        .map(|((n, &len), offset_s)| {
            // swing：落在反拍（单位网格的奇数格）上的音往后推。
            let pos = (clip_start + n.start.0) / unit;
            let on_offbeat = (pos - pos.round()).abs() < 0.05 && (pos.round() as i64).rem_euclid(2) == 1;
            let start = n.start.0 + if on_offbeat { swing_shift } else { 0.0 };
            let zv = gaussian(&mut feel_rng).clamp(-3.0, 3.0);
            let zl = gaussian(&mut feel_rng).clamp(-3.0, 3.0);
            Performed {
                start,
                offset_s,
                length: len * (1.0 + len_sd * zl).max(0.2),
                velocity: (f64::from(n.velocity) + vel_sd * zv).clamp(0.01, 1.0) as f32,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Beats;

    fn notes(n: usize, step: f64) -> Vec<Note> {
        (0..n).map(|i| Note { pitch: 60, start: Beats(i as f64 * step), length: Beats(step * 0.9), velocity: 0.7, accent: false }).collect()
    }

    fn lengths(n: &[Note]) -> Vec<f64> {
        n.iter().map(|n| n.length.0).collect()
    }

    #[test]
    fn same_seed_same_take_other_seed_other_take() {
        let h = Humanize { timing_ms: Some(10.0), velocity: Some(0.1), ..Default::default() };
        let n = notes(16, 0.25);
        let a = perform(&h, &n, &lengths(&n), 0.0, 1);
        assert_eq!(a, perform(&h, &n, &lengths(&n), 0.0, 1));
        assert_ne!(a, perform(&h, &n, &lengths(&n), 0.0, 2));
        // 偏移在 ±3σ 内，力度在 0..1。
        assert!(a.iter().all(|p| p.offset_s.abs() <= 0.030 + 1e-12 && (0.01..=1.0).contains(&p.velocity)));
    }

    #[test]
    fn nothing_set_changes_nothing() {
        let n = notes(8, 0.5);
        for (p, n) in perform(&Humanize::default(), &n, &lengths(&n), 0.0, 9).iter().zip(&n) {
            assert_eq!((p.start, p.offset_s, p.length, p.velocity), (n.start.0, 0.0, n.length.0, n.velocity));
        }
    }

    #[test]
    fn swing_pushes_offbeats_only() {
        let h = Humanize { swing: Some(66.0), ..Default::default() };
        let n = notes(4, 0.5);
        let p = perform(&h, &n, &lengths(&n), 0.0, 0);
        let starts: Vec<f64> = p.iter().map(|p| (p.start * 1000.0).round() / 1000.0).collect();
        assert_eq!(starts, [0.0, 0.66, 1.0, 1.66]);
        // clip 从反拍开始：网格按工程算。
        let p = perform(&h, &n, &lengths(&n), 0.5, 0);
        assert!((p[0].start - 0.16).abs() < 1e-9 && p[1].start == 0.5);
    }

    #[test]
    fn chord_notes_share_timing_and_drift_is_smooth() {
        let h = Humanize { timing_ms: Some(10.0), ..Default::default() };
        let mut n = notes(2, 1.0);
        n.push(Note { pitch: 64, ..n[0].clone() });
        let p = perform(&h, &n, &lengths(&n), 0.0, 3);
        assert_eq!(p[0].offset_s, p[2].offset_s, "和弦同时");
        // drift 大时相邻偏移的差比 drift 为 0 时小（缓慢漂移）。
        let n = notes(400, 0.25);
        let jump = |rho: f64| {
            let p = perform(&Humanize { timing_ms: Some(10.0), drift: Some(rho), ..Default::default() }, &n, &lengths(&n), 0.0, 5);
            p.windows(2).map(|w| (w[1].offset_s - w[0].offset_s).abs()).sum::<f64>()
        };
        assert!(jump(0.9) < jump(0.0) * 0.5);
    }

    #[test]
    fn merge_and_validate() {
        let track = Humanize { timing_ms: Some(8.0), swing: Some(55.0), ..Default::default() };
        let clip = Humanize { swing: Some(62.0), ..Default::default() };
        let m = Humanize::merged(Some(&track), Some(&clip)).unwrap();
        assert_eq!((m.timing_ms, m.swing), (Some(8.0), Some(62.0)));
        assert!(Humanize { swing: Some(80.0), ..Default::default() }.validate("x").unwrap_err().contains("x.swing"));
        assert!(serde_json::from_str::<Humanize>(r#"{"timming_ms": 5}"#).is_err(), "拼错的字段要报错");
    }
}
