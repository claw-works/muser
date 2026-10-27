//! 自动化的校验与编译。
//!
//! core 的 [`muser_core::Automation`] 只描述"什么参数、什么形状"；这里按乐器参数表与
//! 效果器描述表检查参数名与取值范围，再编译成 [`Lane`]：渲染时按拍求值、只在值变化时
//! 写入。乐器与效果器每 [`BLOCK`] 帧更新一次，增益与声像逐帧更新。

use muser_core::automation::{derive_seed, Automation, Shape, Target};
use muser_core::{CompiledCurve, Effect, Project, Track};
use serde_json::Value;

use crate::instrument;
use crate::params::{self, Automatable};

/// 乐器与效果器参数的更新间隔（帧）。48 kHz 下约 1.3 ms。
pub const BLOCK: usize = 64;

/// 增益（dB）可自动化的范围。
const GAIN_RANGE: (f64, f64) = (-80.0, 24.0);

/// 一条编译好的自动化。
#[derive(Clone)]
pub struct Lane {
    /// 目标参数名（`gain_db` / `pan` 等标量目标时为空）。
    pub param: String,
    curve: CompiledCurve,
    /// 生效时间窗之外的静态值。
    base: f64,
    last: Option<f64>,
}

impl Lane {
    /// 这一拍的取值（窗外为静态值）。
    pub fn current(&self, beat: f64) -> f64 {
        self.curve.value_at(beat).unwrap_or(self.base)
    }

    /// 取值相对上次写入有变化时返回新值，否则 `None`。
    pub fn poll(&mut self, beat: f64) -> Option<f64> {
        let v = self.current(beat);
        let changed = self.last.is_none_or(|l| (l - v).abs() > 1e-9 * (1.0 + v.abs()));
        if changed {
            self.last = Some(v);
            Some(v)
        } else {
            None
        }
    }
}

/// 同一标量目标有多条自动化时，后写的覆盖先写的（在其生效窗内）。
pub fn scalar(lanes: &[Lane], beat: f64, base: f64) -> f64 {
    lanes
        .iter()
        .rev()
        .find_map(|l| l.curve.value_at(beat))
        .unwrap_or(base)
}

/// 一条音轨（或总线）的全部自动化。总线没有 `instrument`。
#[derive(Default)]
pub struct TrackLanes {
    pub instrument: Vec<Lane>,
    /// 下标对应 `effects`。
    pub effects: Vec<Vec<Lane>>,
    pub gain: Vec<Lane>,
    pub pan: Vec<Lane>,
    /// 发送量自动化：总线名 → lanes。
    pub sends: std::collections::BTreeMap<String, Vec<Lane>>,
}

/// 主总线的全部自动化。
#[derive(Default)]
pub struct MasterLanes {
    /// 下标对应 `project.master_effects`。
    pub effects: Vec<Vec<Lane>>,
    pub gain: Vec<Lane>,
}

/// 一个参数的取值规则。
struct Rule {
    min: f64,
    max: f64,
    unit: String,
    /// 可用名字代替数字（如元音），第 i 个名字 = i。
    names: &'static [&'static str],
    base: f64,
    /// 插件参数：字符串取值交给插件解析（如 "2000 Hz"）。
    text: Option<Box<dyn Fn(&str) -> Option<f64>>>,
    /// 写入时使用的参数名（插件为数字 id）；None 表示用自动化里写的名字。
    key: Option<String>,
}

impl Rule {
    fn number(&self, v: &Value) -> Result<f64, String> {
        let x = match v {
            Value::String(s) if self.text.is_some() => (self.text.as_ref().unwrap())(s)
                .ok_or_else(|| format!("插件无法解析 '{s}'，可写 {}–{} 的数字", self.min, self.max))?,
            Value::String(s) => self
                .names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(s.trim()))
                .map(|i| i as f64)
                .ok_or_else(|| {
                    if self.names.is_empty() {
                        format!("应为数字，实际 \"{s}\"")
                    } else {
                        format!("应为数字或 {}，实际 \"{s}\"", self.names.join("/"))
                    }
                })?,
            _ => v.as_f64().ok_or_else(|| format!("应为数字，实际 {v}"))?,
        };
        if !x.is_finite() || x < self.min || x > self.max {
            let unit = if self.unit.is_empty() { String::new() } else { format!(" {}", self.unit) };
            let unit = unit.as_str();
            return Err(format!("{x} 超出范围 [{}, {}]{unit}", self.min, self.max));
        }
        Ok(x)
    }
}

fn instrument_rule(track: &Track, param: &str) -> Result<(Rule, Automatable), String> {
    let (spec, how, base) = instrument::automation_param(&track.instrument, &track.instrument_params, param)?;
    let params::ParamKind::Number { min, max, unit, names, .. } = spec.kind else {
        return Err(format!("参数 '{param}' 不能自动化"));
    };
    Ok((Rule { min, max, unit: unit.into(), names, base, text: None, key: None }, how))
}

/// 插件参数的取值规则：打开插件（已应用基础参数），按名称或 id 找参数。
#[cfg(feature = "host")]
fn plugin_rule(path: &str, id: Option<&str>, base_params: &serde_json::Map<String, Value>, param: &str) -> Result<Rule, String> {
    let (opened, pid) = crate::plugin::automation_target(path, id, base_params, param)?;
    let p = opened.params.iter().find(|p| p.id == pid).expect("刚解析出的参数").clone();
    let base = opened.plugin.param_value(pid).unwrap_or(p.default);
    let (min, max, unit) = (p.min, p.max, p.unit.clone());
    Ok(Rule {
        min,
        max,
        unit,
        names: &[],
        base,
        text: Some(Box::new(move |s: &str| opened.text_value(&p, s))),
        key: Some(pid.0.to_string()),
    })
}

#[cfg(not(feature = "host"))]
fn plugin_rule(_path: &str, _id: Option<&str>, _base: &serde_json::Map<String, Value>, _param: &str) -> Result<Rule, String> {
    Err("插件参数自动化不可用：这个 muser 构建没有启用插件宿主（`host` 特性）；发行版与默认构建都带，别用 --no-default-features 去掉它".into())
}

fn effect_rule(effect: &Effect, param: &str) -> Result<Rule, String> {
    if let Effect::Plugin { path, id, params } = effect {
        return plugin_rule(path, id.as_deref(), params, param);
    }
    let spec = muser_core::effect_spec(effect.type_name()).expect("每个效果器都有描述");
    let p = spec.params.iter().find(|p| p.name == param).ok_or_else(|| {
        format!(
            "{} 没有参数 '{param}'，可自动化: {}",
            spec.name,
            spec.params.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
        )
    })?;
    let base = effect
        .numeric_params()
        .into_iter()
        .find(|(n, _)| *n == param)
        .map(|(_, v)| v)
        .unwrap_or(p.default);
    Ok(Rule { min: p.min, max: p.max, unit: p.unit.into(), names: &[], base, text: None, key: None })
}

fn scalar_rule(min: f64, max: f64, unit: &'static str, base: f64) -> Rule {
    Rule { min, max, unit: unit.into(), names: &[], base, text: None, key: None }
}

/// 检查取值并编译。
fn compile_lane(a: &Automation, rule: &Rule, seed: u64, total_beats: f64) -> Result<Lane, String> {
    let values = match &a.shape {
        Shape::Points { points, .. } => points
            .iter()
            .enumerate()
            .map(|(i, (_, v))| rule.number(v).map_err(|e| format!("points[{i}]: {e}")))
            .collect::<Result<Vec<_>, _>>()?,
        Shape::Lfo { min, max, .. } | Shape::Random { min, max, .. } => {
            rule.number(&Value::from(*min)).map_err(|e| format!("min: {e}"))?;
            rule.number(&Value::from(*max)).map_err(|e| format!("max: {e}"))?;
            Vec::new()
        }
    };
    if let Shape::Points { curve: muser_core::Curve::Exp, .. } = &a.shape {
        if values.windows(2).any(|w| w[0] * w[1] <= 0.0) {
            return Err("curve \"exp\" 要求相邻点同号且非零；含 0 或跨越 0 时请用 linear / smooth".into());
        }
    }
    Ok(Lane {
        param: rule.key.clone().unwrap_or_else(|| a.param.clone()),
        curve: a.compile(&values, seed, total_beats),
        base: rule.base,
        last: None,
    })
}

/// 编译一条音轨的自动化。`ti` 用于推导默认种子与错误位置。
pub fn compile_track(project: &Project, ti: usize, total_beats: f64) -> Result<TrackLanes, String> {
    let track = &project.tracks[ti];
    let mut out = TrackLanes { effects: (0..track.effects.len()).map(|_| Vec::new()).collect(), ..Default::default() };
    for (ai, a) in track.automation.iter().enumerate() {
        let at = |e: String| format!("tracks[{ti}].automation[{ai}]: {e}");
        let seed = derive_seed(project.seed, Some(ti), ai);
        let target = Target::parse(&a.target).ok_or_else(|| at(format!("target '{}' 无效", a.target)))?;
        match target {
            Target::Instrument => {
                let rule = match track.instrument.trim().strip_prefix("plugin:") {
                    Some(spec) => {
                        let (path, id) = crate::plugin::split_spec(spec);
                        plugin_rule(path, id, &track.instrument_params, &a.param).map_err(at)?
                    }
                    None => instrument_rule(track, &a.param).map_err(at)?.0,
                };
                out.instrument.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Effect(i) => {
                let fx = track.effects.get(i).ok_or_else(|| at(format!("没有 effects[{i}]")))?;
                let rule = effect_rule(fx, &a.param).map_err(at)?;
                out.effects[i].push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Gain => {
                let rule = scalar_rule(GAIN_RANGE.0, GAIN_RANGE.1, "dB", track.gain_db);
                out.gain.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Pan => {
                let rule = scalar_rule(-1.0, 1.0, "", track.pan);
                out.pan.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Send(bus) => {
                let base = *track.sends.get(&bus).ok_or_else(|| at(format!("没有发送到 '{bus}'")))?;
                let rule = scalar_rule(GAIN_RANGE.0, GAIN_RANGE.1, "dB", base);
                out.sends.entry(bus).or_default().push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::MasterGain | Target::MasterEffect(_) => {
                return Err(at("主总线目标请写在工程级 automation".into()));
            }
        }
    }
    Ok(out)
}

/// 编译一条总线的自动化（目标 effects[i] / gain_db / pan / sends.<总线>）。
pub fn compile_bus(project: &Project, bi: usize, total_beats: f64) -> Result<TrackLanes, String> {
    let bus = &project.buses[bi];
    let mut out = TrackLanes { effects: (0..bus.effects.len()).map(|_| Vec::new()).collect(), ..Default::default() };
    for (ai, a) in bus.automation.iter().enumerate() {
        let at = |e: String| format!("buses[{bi}].automation[{ai}]: {e}");
        // 种子空间与音轨分开：总线用 tracks.len() + bi。
        let seed = derive_seed(project.seed, Some(project.tracks.len() + bi), ai);
        match Target::parse(&a.target).ok_or_else(|| at(format!("target '{}' 无效", a.target)))? {
            Target::Effect(i) => {
                let fx = bus.effects.get(i).ok_or_else(|| at(format!("没有 effects[{i}]")))?;
                let rule = effect_rule(fx, &a.param).map_err(at)?;
                out.effects[i].push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Gain => {
                let rule = scalar_rule(GAIN_RANGE.0, GAIN_RANGE.1, "dB", bus.gain_db);
                out.gain.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Pan => {
                let rule = scalar_rule(-1.0, 1.0, "", bus.pan);
                out.pan.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::Send(name) => {
                let base = *bus.sends.get(&name).ok_or_else(|| at(format!("没有发送到 '{name}'")))?;
                let rule = scalar_rule(GAIN_RANGE.0, GAIN_RANGE.1, "dB", base);
                out.sends.entry(name).or_default().push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            _ => return Err(at("总线上可自动化的目标: effects[i] / gain_db / pan / sends.<总线>".into())),
        }
    }
    Ok(out)
}

/// 编译主总线自动化。
pub fn compile_master(project: &Project, total_beats: f64) -> Result<MasterLanes, String> {
    let mut out = MasterLanes {
        effects: (0..project.master_effects.len()).map(|_| Vec::new()).collect(),
        ..Default::default()
    };
    for (ai, a) in project.automation.iter().enumerate() {
        let at = |e: String| format!("automation[{ai}]: {e}");
        let seed = derive_seed(project.seed, None, ai);
        match Target::parse(&a.target).ok_or_else(|| at(format!("target '{}' 无效", a.target)))? {
            Target::MasterGain => {
                let rule = scalar_rule(GAIN_RANGE.0, GAIN_RANGE.1, "dB", project.master_gain_db);
                out.gain.push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            Target::MasterEffect(i) => {
                let fx = project.master_effects.get(i).ok_or_else(|| at(format!("没有 master_effects[{i}]")))?;
                let rule = effect_rule(fx, &a.param).map_err(at)?;
                out.effects[i].push(compile_lane(a, &rule, seed, total_beats).map_err(at)?);
            }
            _ => return Err(at("音轨目标请写在该音轨的 automation".into())),
        }
    }
    Ok(out)
}

/// 工程的总长度（拍），含结尾余响，用于随机自动化的预计算。
pub fn total_beats(project: &Project, tail_seconds: f64) -> f64 {
    project.duration_beats().0 + tail_seconds * project.tempo / 60.0
}

/// 只校验，不渲染。
pub fn check(project: &Project) -> Result<(), String> {
    let beats = total_beats(project, 0.0);
    for ti in 0..project.tracks.len() {
        if !project.tracks[ti].automation.is_empty() {
            compile_track(project, ti, beats)?;
        }
    }
    for bi in 0..project.buses.len() {
        compile_bus(project, bi, beats)?;
    }
    compile_master(project, beats).map(|_| ())
}
