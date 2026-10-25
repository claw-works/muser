//! 自动化：让参数随时间变化。
//!
//! 一条自动化 = 目标（乐器 / 某个效果器 / 增益 / 声像）+ 参数名 + 形状。形状有三种：
//! - `points`：在若干时间点给出取值，中间按曲线插值；
//! - `lfo`：周期性摆动（正弦 / 三角 / 锯齿 / 方波）；
//! - `random`：带种子的随机变化（跳变 / 滑动 / 随机游走）。同一种子每次渲染结果相同，
//!   换种子就换一种变化。
//!
//! 时间单位是拍，从工程开头算起，与音符一致。取值用参数自己的单位（Hz、秒、dB…）；
//! 参数的范围由 muser-engine 校验。

use serde::{Deserialize, Serialize};

use crate::Beats;

/// 一条自动化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    /// 作用对象。音轨上：`"instrument"`、`"effects[0]"`（该轨第 1 个效果器）、`"gain_db"`、`"pan"`、
    /// `"sends.<总线名>"`（发送量 dB）；总线上同音轨（没有 instrument）；
    /// 工程上：`"master_gain_db"`、`"master_effects[0]"`。
    pub target: String,
    /// 参数名，与 `instrument_params` / 效果器参数同名；鼓组用 `"kick.tune"` 这种 `鼓件.参数`。
    /// 目标是 `gain_db` / `pan` / `master_gain_db` 时省略。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub param: String,
    /// 形状。
    #[serde(flatten)]
    pub shape: Shape,
    /// 生效起点（拍，可省略）。只对 `lfo` / `random` 有意义；之前参数保持静态值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Beats>,
    /// 生效终点（拍，可省略）。之后参数回到静态值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Beats>,
}

/// 自动化的形状。JSON 中以 `shape` 字段区分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// 时间点曲线：`points` 为 `[拍, 值]` 列表，按时间递增。第一个点之前取第一个值，
    /// 最后一个点之后保持最后的值。值可以是数字，或参数允许的名字（如元音 `"o"`）。
    Points {
        points: Vec<(f64, serde_json::Value)>,
        /// 点之间的插值：`linear` 直线；`exp` 按比例（频率、时间类参数听感更均匀，要求两端同号且非零）；
        /// `smooth` 两端平缓的 S 形；`step` 保持前一个值直到下一个点。
        #[serde(default)]
        curve: Curve,
    },
    /// 周期摆动，在 `min` 与 `max` 之间。
    Lfo {
        #[serde(default)]
        wave: LfoWave,
        /// 一个周期多少拍。4 = 每小节（4/4）一次；0.5 = 每八分音符一次。
        period_beats: f64,
        min: f64,
        max: f64,
        /// 起始相位 0-1（0 = 周期开头）。
        #[serde(default)]
        phase: f64,
    },
    /// 带种子的随机变化，在 `min` 与 `max` 之间。
    Random {
        #[serde(default)]
        style: RandomStyle,
        /// 多少拍换一次目标值。
        step_beats: f64,
        min: f64,
        max: f64,
        /// `walk` 每步最多移动整个范围的多少（0-1），默认 0.2。
        #[serde(default = "default_max_step")]
        max_step: f64,
        /// 随机种子（可省略）。省略时由工程 `seed` 与这条自动化的位置推导。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },
}

fn default_max_step() -> f64 {
    0.2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    #[default]
    Linear,
    Exp,
    Smooth,
    Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoWave {
    #[default]
    Sine,
    Triangle,
    /// 从 min 升到 max 再跳回。
    Saw,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RandomStyle {
    /// 每步跳到一个新的随机值。
    Jump,
    /// 每步平滑地滑向新的随机值。
    #[default]
    Glide,
    /// 随机游走：每步在当前值附近小幅移动（受 `max_step` 限制），平滑过渡。
    Walk,
}

/// 自动化的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Instrument,
    Effect(usize),
    Gain,
    Pan,
    /// 发送到某条总线的量（dB）。
    Send(String),
    MasterGain,
    MasterEffect(usize),
}

impl Target {
    pub fn parse(s: &str) -> Option<Target> {
        let s = s.trim();
        let index = |prefix: &str| -> Option<usize> {
            s.strip_prefix(prefix)?.strip_prefix('[')?.strip_suffix(']')?.trim().parse().ok()
        };
        Some(match s {
            "instrument" => Target::Instrument,
            "gain_db" => Target::Gain,
            "pan" => Target::Pan,
            "master_gain_db" => Target::MasterGain,
            _ if s.starts_with("sends.") && s.len() > 6 => Target::Send(s[6..].trim().to_string()),
            _ => {
                if let Some(i) = index("master_effects") {
                    Target::MasterEffect(i)
                } else {
                    Target::Effect(index("effects")?)
                }
            }
        })
    }

    /// 是否属于工程（主总线）级。
    pub fn is_master(&self) -> bool {
        matches!(self, Target::MasterGain | Target::MasterEffect(_))
    }

    /// 目标本身就是一个值（不需要 param）。
    pub fn is_scalar(&self) -> bool {
        matches!(self, Target::Gain | Target::Pan | Target::MasterGain | Target::Send(_))
    }
}

impl Automation {
    /// 结构检查（不涉及参数范围，范围由 engine 按参数表检查）。
    pub fn check_shape(&self) -> Result<(), String> {
        match &self.shape {
            Shape::Points { points, curve } => {
                if points.is_empty() {
                    return Err("points 至少需要一个点".into());
                }
                for w in points.windows(2) {
                    if !(w[1].0 > w[0].0) {
                        return Err(format!("points 的时间必须严格递增：{} 之后是 {}", w[0].0, w[1].0));
                    }
                }
                if points.iter().any(|(t, _)| !t.is_finite() || *t < 0.0) {
                    return Err("points 的时间必须 ≥ 0".into());
                }
                let _ = curve;
            }
            Shape::Lfo { period_beats, min, max, phase, .. } => {
                if !(*period_beats > 0.0) {
                    return Err("lfo.period_beats 必须 > 0".into());
                }
                if !(min.is_finite() && max.is_finite()) {
                    return Err("lfo.min / max 必须是有限数".into());
                }
                if !(0.0..=1.0).contains(phase) {
                    return Err("lfo.phase 应在 0-1".into());
                }
            }
            Shape::Random { step_beats, min, max, max_step, .. } => {
                if !(*step_beats > 0.0) {
                    return Err("random.step_beats 必须 > 0".into());
                }
                if !(min.is_finite() && max.is_finite()) {
                    return Err("random.min / max 必须是有限数".into());
                }
                if !(*max_step > 0.0 && *max_step <= 1.0) {
                    return Err("random.max_step 应在 (0, 1]".into());
                }
            }
        }
        if let (Some(s), Some(e)) = (self.start, self.end) {
            if !(e.0 > s.0) {
                return Err("end 必须大于 start".into());
            }
        }
        Ok(())
    }

    /// 编译成可求值的曲线。`values` 已由调用方转成数值（points 的每个值）。
    /// `default_seed` 在自动化没写 `seed` 时使用；`total_beats` 用于随机游走预计算。
    pub fn compile(&self, values: &[f64], default_seed: u64, total_beats: f64) -> CompiledCurve {
        let kind = match &self.shape {
            Shape::Points { points, curve } => CurveKind::Points {
                times: points.iter().map(|(t, _)| *t).collect(),
                values: values.to_vec(),
                curve: *curve,
            },
            Shape::Lfo { wave, period_beats, min, max, phase } => CurveKind::Lfo {
                wave: *wave,
                period: *period_beats,
                min: *min,
                max: *max,
                phase: *phase,
            },
            Shape::Random { style, step_beats, min, max, max_step, seed } => {
                let seed = seed.unwrap_or(default_seed);
                let steps = ((total_beats.max(0.0) / step_beats).ceil() as usize + 2).min(1_000_000);
                let mut rng = SplitMix64(seed ^ 0x9E37_79B9_7F4A_7C15);
                let mut targets = Vec::with_capacity(steps);
                let mut x = rng.unit();
                for _ in 0..steps {
                    match style {
                        RandomStyle::Walk => {
                            x = (x + (rng.unit() * 2.0 - 1.0) * max_step).clamp(0.0, 1.0);
                            targets.push(x);
                        }
                        _ => targets.push(rng.unit()),
                    }
                }
                CurveKind::Random { style: *style, step: *step_beats, min: *min, max: *max, targets }
            }
        };
        CompiledCurve {
            kind,
            start: self.start.map(|b| b.0),
            end: self.end.map(|b| b.0),
        }
    }
}

/// 简单、可复现的 64 位随机数生成器（SplitMix64）。
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// [0, 1) 均匀分布。
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 由工程种子和自动化的位置推导出这条自动化的默认种子。
pub fn derive_seed(project_seed: u64, track: Option<usize>, index: usize) -> u64 {
    let mut r = SplitMix64(project_seed);
    let a = r.next_u64();
    let t = track.map_or(u64::MAX, |t| t as u64);
    SplitMix64(a ^ t.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ (index as u64).wrapping_mul(0xA076_1D64_78BD_642F)).next_u64()
}

#[derive(Debug, Clone)]
enum CurveKind {
    Points { times: Vec<f64>, values: Vec<f64>, curve: Curve },
    Lfo { wave: LfoWave, period: f64, min: f64, max: f64, phase: f64 },
    Random { style: RandomStyle, step: f64, min: f64, max: f64, targets: Vec<f64> },
}

/// 编译好的曲线：给定拍，得到参数值；在生效时间窗之外返回 `None`。
#[derive(Debug, Clone)]
pub struct CompiledCurve {
    kind: CurveKind,
    start: Option<f64>,
    end: Option<f64>,
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

impl CompiledCurve {
    pub fn value_at(&self, beat: f64) -> Option<f64> {
        if self.start.is_some_and(|s| beat < s) || self.end.is_some_and(|e| beat >= e) {
            return None;
        }
        let local = beat - self.start.unwrap_or(0.0);
        Some(match &self.kind {
            CurveKind::Points { times, values, curve } => {
                let i = times.partition_point(|t| *t <= beat);
                if i == 0 {
                    values[0]
                } else if i == times.len() {
                    values[values.len() - 1]
                } else {
                    let (t0, t1, v0, v1) = (times[i - 1], times[i], values[i - 1], values[i]);
                    let x = (beat - t0) / (t1 - t0);
                    match curve {
                        Curve::Linear => v0 + (v1 - v0) * x,
                        Curve::Smooth => v0 + (v1 - v0) * smoothstep(x),
                        Curve::Step => v0,
                        Curve::Exp => {
                            if v0 * v1 > 0.0 {
                                v0 * (v1 / v0).powf(x)
                            } else {
                                v0 + (v1 - v0) * x
                            }
                        }
                    }
                }
            }
            CurveKind::Lfo { wave, period, min, max, phase } => {
                let p = (local / period + phase).rem_euclid(1.0);
                let u = match wave {
                    LfoWave::Sine => 0.5 - 0.5 * (std::f64::consts::TAU * p).cos(),
                    LfoWave::Triangle => 1.0 - (2.0 * p - 1.0).abs(),
                    LfoWave::Saw => p,
                    LfoWave::Square => {
                        if p < 0.5 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                };
                min + (max - min) * u
            }
            CurveKind::Random { style, step, min, max, targets } => {
                let pos = (local / step).max(0.0);
                let k = (pos.floor() as usize).min(targets.len() - 2);
                let x = (pos - k as f64).clamp(0.0, 1.0);
                let u = match style {
                    RandomStyle::Jump => targets[k],
                    RandomStyle::Glide | RandomStyle::Walk => {
                        targets[k] + (targets[k + 1] - targets[k]) * smoothstep(x)
                    }
                };
                min + (max - min) * u
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Automation {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn points_linear_exp_step_and_hold() {
        let a = parse(json!({ "target": "instrument", "param": "cutoff", "shape": "points",
            "points": [[0, 100], [4, 1600]], "curve": "exp" }));
        a.check_shape().unwrap();
        let c = a.compile(&[100.0, 1600.0], 0, 8.0);
        assert!((c.value_at(2.0).unwrap() - 400.0).abs() < 1e-6, "exp 中点应为几何平均");
        assert_eq!(c.value_at(-1.0), Some(100.0));
        assert_eq!(c.value_at(10.0), Some(1600.0));
        let lin = parse(json!({ "target": "pan", "shape": "points", "points": [[0, -1], [2, 1]] }));
        assert_eq!(lin.compile(&[-1.0, 1.0], 0, 4.0).value_at(1.0), Some(0.0));
        let step = parse(json!({ "target": "pan", "shape": "points", "points": [[0, -1], [2, 1]], "curve": "step" }));
        assert_eq!(step.compile(&[-1.0, 1.0], 0, 4.0).value_at(1.9), Some(-1.0));
    }

    #[test]
    fn lfo_waves_and_window() {
        let a = parse(json!({ "target": "pan", "shape": "lfo", "wave": "triangle", "period_beats": 4,
            "min": -1, "max": 1, "start": 4, "end": 12 }));
        let c = a.compile(&[], 0, 16.0);
        assert_eq!(c.value_at(2.0), None);
        assert_eq!(c.value_at(4.0), Some(-1.0));
        assert_eq!(c.value_at(6.0), Some(1.0));
        assert_eq!(c.value_at(12.0), None);
        let s = parse(json!({ "target": "pan", "shape": "lfo", "period_beats": 2, "min": 0, "max": 10 }));
        assert!((s.compile(&[], 0, 4.0).value_at(1.0).unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn random_is_reproducible_and_bounded() {
        let a = parse(json!({ "target": "instrument", "param": "cutoff", "shape": "random",
            "style": "walk", "step_beats": 0.5, "min": 200, "max": 4000 }));
        a.check_shape().unwrap();
        let c1 = a.compile(&[], 42, 32.0);
        let c2 = a.compile(&[], 42, 32.0);
        let c3 = a.compile(&[], 43, 32.0);
        let mut differs = false;
        for i in 0..640 {
            let b = i as f64 * 0.05;
            let (v1, v2, v3) = (c1.value_at(b).unwrap(), c2.value_at(b).unwrap(), c3.value_at(b).unwrap());
            assert_eq!(v1, v2, "同种子应完全相同");
            assert!((200.0..=4000.0).contains(&v1));
            differs |= v1 != v3;
        }
        assert!(differs, "不同种子应不同");
        assert_ne!(derive_seed(0, Some(0), 0), derive_seed(0, Some(0), 1));
        assert_ne!(derive_seed(0, Some(0), 0), derive_seed(1, Some(0), 0));
    }

    #[test]
    fn shape_errors() {
        let bad = parse(json!({ "target": "pan", "shape": "points", "points": [[2, 0], [1, 1]] }));
        assert!(bad.check_shape().unwrap_err().contains("递增"));
        let bad = parse(json!({ "target": "pan", "shape": "lfo", "period_beats": 0, "min": 0, "max": 1 }));
        assert!(bad.check_shape().is_err());
    }

    #[test]
    fn targets() {
        assert_eq!(Target::parse("effects[2]"), Some(Target::Effect(2)));
        assert_eq!(Target::parse("master_effects[0]"), Some(Target::MasterEffect(0)));
        assert_eq!(Target::parse("instrument"), Some(Target::Instrument));
        assert_eq!(Target::parse("effects"), None);
        assert_eq!(Target::parse("sends.reverb"), Some(Target::Send("reverb".into())));
        assert_eq!(Target::parse("sends."), None);
    }
}
