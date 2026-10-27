//! 混音报告：渲染一遍整首，给出每条音轨、总线与主总线的响度，帮 AI 平衡各轨音量。
//!
//! 每个节点两个测量点：效果链之前（音轨是乐器干声：音源本身有多响）与推子后（送进混音的信号）。
//! 音轨之间比"到主总线的估计响度"：推子后响度加上直接输出路径上各总线的 gain_db
//! （不计总线效果器与发送）。经过带压缩、EQ 的总线时这个估计可能差 2–5 LU（总线压缩器对整组起作用，
//! 单条轨过总线后的响度本来就分不开），这时同一总线内比各音轨的 `output`，组与组之间比各总线的 `output`，
//! 这两个都是实测值。主总线看送进来的信号峰值：超过 0 dBFS 说明靠主效果链硬压。

use muser_core::Project;
use muser_core::routing::Node;
use serde::Serialize;

use crate::render::{render_loudness, Level, RenderCache, RenderSettings};
use crate::EngineError;

/// 一个节点的报告。
#[derive(Debug, Clone, Serialize)]
pub struct NodeReport {
    /// `tracks[i]` / `buses[i]` / `master`。
    pub node: String,
    pub name: String,
    pub gain_db: f64,
    pub muted: bool,
    /// 效果链之前。
    pub input: Level,
    /// 推子后。
    pub output: Level,
    /// 音轨：到主总线的估计响度（LUFS）= 推子后响度 + 直接输出路径上各总线的 gain_db。
    /// 不计总线效果器；路径上有静音的总线时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_master_lufs: Option<f64>,
    /// 音轨：`at_master_lufs` 相对最响音轨的差（LU，0 为最响）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_lu: Option<f64>,
}

/// 整首的混音报告。
#[derive(Debug, Clone, Serialize)]
pub struct MixReport {
    pub tracks: Vec<NodeReport>,
    pub buses: Vec<NodeReport>,
    pub master: NodeReport,
    /// 需要注意的地方。
    pub warnings: Vec<String>,
}

/// 从音轨的输出沿直接输出路径到主总线，经过的各总线 gain_db 之和；路径上有静音的总线时为 `None`（到不了主总线）。
fn path_gain<'a>(project: &'a Project, mut output: Option<&'a str>) -> Option<f64> {
    let mut g = 0.0;
    // 路由已校验无环；上限防御。
    for _ in 0..=project.buses.len() {
        let Some(name) = output.map(str::trim).filter(|n| !n.is_empty() && *n != "master") else { break };
        let Some(b) = project.buses.iter().find(|b| b.name == name) else { break };
        if b.muted {
            return None;
        }
        g += b.gain_db;
        output = b.output.as_deref();
    }
    Some(g)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// 渲染整首并生成混音报告。复用并更新 `cache` 里的乐器输出（只改推子再跑很快）。
pub fn mix_report(project: &Project, settings: RenderSettings, cache: &mut RenderCache) -> Result<MixReport, EngineError> {
    let nodes = render_loudness(project, settings, cache)?;
    let (mut tracks, mut buses, mut master) = (Vec::new(), Vec::new(), None);
    for n in nodes {
        let (node, name, gain_db) = match n.node {
            Some(Node::Track(i)) => (format!("tracks[{i}]"), project.tracks[i].name.clone(), project.tracks[i].gain_db),
            Some(Node::Bus(i)) => (format!("buses[{i}]"), project.buses[i].name.clone(), project.buses[i].gain_db),
            None => ("master".into(), "master".into(), project.master_gain_db),
        };
        let at_master_lufs = match n.node {
            Some(Node::Track(i)) => n.output.lufs.zip(path_gain(project, project.tracks[i].output.as_deref())).map(|(l, g)| round1(l + g)),
            _ => None,
        };
        let r = NodeReport { node, name, gain_db, muted: n.muted, input: n.input, output: n.output, at_master_lufs, rel_lu: None };
        match n.node {
            Some(Node::Track(_)) => tracks.push(r),
            Some(Node::Bus(_)) => buses.push(r),
            None => master = Some(r),
        }
    }
    let master = master.expect("主总线总在");
    // 报告按工程里的顺序列出（渲染按计划顺序）。
    let index = |n: &NodeReport| n.node.trim_end_matches(']').rsplit('[').next().and_then(|i| i.parse::<usize>().ok()).unwrap_or(0);
    tracks.sort_by_key(index);
    buses.sort_by_key(index);

    let mut warnings = Vec::new();
    let loudest = tracks.iter().filter_map(|t| t.at_master_lufs).fold(f64::NEG_INFINITY, f64::max);
    for t in &mut tracks {
        if let Some(l) = t.at_master_lufs {
            t.rel_lu = Some(round1(l - loudest));
        } else if !t.muted && t.output.lufs.is_none() {
            warnings.push(format!("{} \"{}\" 没有声音（检查音符、乐器或自动化）", t.node, t.name));
        }
    }
    for n in tracks.iter().chain(&buses) {
        if let Some(p) = n.output.peak_db.filter(|p| *p > 0.0) {
            warnings.push(format!("{} \"{}\" 推子后峰值 +{p:.1} dBFS，降低 gain_db", n.node, n.name));
        }
    }
    if let Some(p) = master.input.peak_db.filter(|p| *p > 0.0) {
        warnings.push(format!(
            "送进主总线的信号峰值 {p:+.1} dBFS，全靠主效果链硬压：把各轨（或各总线）的 gain_db 一起降约 {:.0} dB，主总线留 3 dB 以上余量",
            (p + 3.0).ceil()
        ));
    }
    if let Some(p) = master.output.peak_db.filter(|p| *p > -0.1) {
        warnings.push(format!("主输出峰值 {p:+.1} dBFS，会削波；主总线最后放 limiter（ceiling_db -1）或降 master_gain_db"));
    }
    Ok(MixReport { tracks, buses, master, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(pad_gain: f64) -> Project {
        let notes: Vec<_> = (0..8).map(|i| serde_json::json!({ "pitch": 48 + (i % 4) * 3, "start": i as f64 * 0.5, "length": 0.4 })).collect();
        serde_json::from_value(serde_json::json!({
            "tempo": 120,
            "buses": [{ "name": "grp" }],
            "tracks": [
                { "name": "bass", "instrument": "bass.acid", "clips": [{ "start": 0, "length": 4, "notes": notes }] },
                { "name": "pad", "instrument": "synth.poly", "output": "grp", "gain_db": pad_gain,
                  "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 60, "start": 0, "length": 4 }] }] },
                { "name": "off", "instrument": "synth.poly", "muted": true, "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 64, "start": 0, "length": 4 }] }] }
            ],
            "master_effects": [{ "type": "limiter", "ceiling_db": -1 }]
        }))
        .unwrap()
    }

    fn settings() -> RenderSettings {
        RenderSettings { tail_seconds: 0.5, ..RenderSettings::default() }
    }

    #[test]
    fn gain_moves_output_not_input() {
        let mut cache = RenderCache::new();
        let a = mix_report(&project(0.0), settings(), &mut cache).unwrap();
        let b = mix_report(&project(-12.0), settings(), &mut cache).unwrap();
        let (pa, pb) = (&a.tracks[1], &b.tracks[1]);
        assert_eq!(pa.name, "pad");
        // 干声不随推子变，推子后跟着降约 12 dB。
        assert_eq!(pa.input.lufs, pb.input.lufs);
        let d = pa.output.lufs.unwrap() - pb.output.lufs.unwrap();
        assert!((d - 12.0).abs() < 0.3, "推子 -12 dB，推子后响度差 {d}");
        assert!(pa.input.lufs.is_some() && pa.output.peak_db.is_some());
        // 最响音轨为 0，其余为负。
        assert!(a.tracks.iter().filter_map(|t| t.rel_lu).any(|r| r == 0.0));
        assert!(a.tracks.iter().filter_map(|t| t.rel_lu).all(|r| r <= 0.0));
    }

    #[test]
    fn bus_gain_counts_toward_master() {
        let mut p = project(0.0);
        p.buses[0].gain_db = -6.0;
        let r = mix_report(&p, settings(), &mut RenderCache::new()).unwrap();
        let pad = &r.tracks[1];
        assert_eq!(pad.at_master_lufs, Some(round1(pad.output.lufs.unwrap() - 6.0)));
        let bass = &r.tracks[0];
        assert_eq!(bass.at_master_lufs, bass.output.lufs);
    }

    /// 输出总线被静音的轨到不了主总线：不参与相对比较，也不当成"没有声音"。
    #[test]
    fn muted_bus_drops_out_of_comparison() {
        let mut p = project(0.0);
        p.buses[0].muted = true;
        let r = mix_report(&p, settings(), &mut RenderCache::new()).unwrap();
        let pad = &r.tracks[1];
        assert!(pad.output.lufs.is_some() && pad.at_master_lufs.is_none() && pad.rel_lu.is_none());
        assert_eq!(r.tracks[0].rel_lu, Some(0.0), "参照是还能到主总线的最响轨");
        assert!(!r.warnings.iter().any(|w| w.contains("pad")), "{:?}", r.warnings);
    }

    #[test]
    fn muted_track_and_master() {
        let r = mix_report(&project(0.0), settings(), &mut RenderCache::new()).unwrap();
        let off = &r.tracks[2];
        assert!(off.muted && off.output.lufs.is_none() && off.rel_lu.is_none());
        assert_eq!(r.buses[0].name, "grp");
        assert!(r.buses[0].input.lufs.is_some());
        assert!(r.master.output.peak_db.unwrap() <= -0.9, "limiter ceiling -1");
        assert!(r.master.input.peak_db.is_some());
    }
}
