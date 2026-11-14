//! 信号路由：总线（bus）、输出、发送（send）、侧链（sidechain）。
//!
//! 信号流：
//! ```text
//! 音轨：乐器 → effects → gain/pan ─┬─→ output（默认 master，或某条总线）
//!                                   └─→ sends：按 dB 送一份到其它总线（推子后）
//! 总线：所有送进来的信号求和 → effects → gain/pan ─┬─→ output（默认 master，或另一条总线）
//!                                                  └─→ sends
//! 主总线：所有送到 master 的信号求和 → master_effects → master_gain_db
//! ```
//! 压缩器的 `sidechain` 写某条音轨或总线的名字，用那条信号（推子后）来触发压缩。
//!
//! 这里只描述结构并检查引用与环路；渲染由 muser-engine 按 [`plan`] 给出的顺序进行。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Automation, EffectSlot, Project};

/// 输出目标里表示主总线的名字。
pub const MASTER: &str = "master";

/// 一条总线：汇集若干音轨/总线的信号，统一处理后再输出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bus {
    /// 总线名（唯一，不能叫 master）。音轨的 `output` / `sends` 和压缩器 `sidechain` 用它引用。
    pub name: String,
    /// 总线增益（dB），默认 0。
    #[serde(default)]
    pub gain_db: f64,
    /// 声像 -1~1，默认 0。总线是立体声信号，这里是平衡（居中不衰减）。
    #[serde(default)]
    pub pan: f64,
    #[serde(default)]
    pub muted: bool,
    /// 总线效果器链。
    #[serde(default)]
    pub effects: Vec<EffectSlot>,
    /// 输出到哪里：`"master"`（默认）或另一条总线的名字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// 发送：总线名 → 发送量（dB，推子后）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sends: BTreeMap<String, f64>,
    /// 总线自动化（目标 `effects[i]` / `gain_db` / `pan` / `sends.<总线>`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Automation>,
}

/// 发送量与增益的允许范围（dB）。
pub const LEVEL_RANGE: (f64, f64) = (-80.0, 24.0);

/// 图中的一个节点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    Track(usize),
    Bus(usize),
}

/// 一条连接：源节点的推子后输出送到目标（None = 主总线），带发送量（dB，None 表示直接输出、不衰减）。
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: Node,
    pub to: Option<usize>,
    /// 发送的总线名（直接输出时为 None）。
    pub send: Option<String>,
    pub level_db: f64,
}

/// 侧链：`node` 的第 `effect` 个效果器（压缩器）由 `key` 的推子后信号触发。
#[derive(Debug, Clone, PartialEq)]
pub struct Sidechain {
    pub node: Node,
    pub effect: usize,
    pub key: Node,
}

/// 直接输出：插件乐器的某一路输出（`instrument_params` 的 `@outputs` 里写了 `bus` 的项）不经过音轨的效果器与推子，
/// 直接送进总线（如多麦克风鼓组的底鼓、军鼓各进一条总线）。
#[derive(Debug, Clone, PartialEq)]
pub struct Tap {
    pub track: usize,
    /// 这条音轨的第几路直接输出（`@outputs` 里写了 `bus` 的项按顺序）。
    pub index: usize,
    /// 去哪条总线（None = 主总线）。
    pub to: Option<usize>,
}

/// 渲染计划。
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// 处理顺序：每个节点在它的所有输入（来自音轨/总线的信号、直接输出和侧链键）之后。
    pub order: Vec<Node>,
    pub edges: Vec<Edge>,
    pub sidechains: Vec<Sidechain>,
    pub taps: Vec<Tap>,
}

impl Plan {
    /// 某节点作为侧链键被哪些节点使用。
    pub fn key_users(&self, n: Node) -> usize {
        self.sidechains.iter().filter(|s| s.key == n).count()
    }
}

/// 按名字找节点：总线名优先匹配总线；否则匹配音轨名（须唯一）。
pub fn find_node(p: &Project, name: &str) -> Result<Node, String> {
    let name = name.trim();
    let bus = p.buses.iter().position(|b| b.name == name);
    let tracks: Vec<usize> = p.tracks.iter().enumerate().filter(|(_, t)| t.name == name).map(|(i, _)| i).collect();
    match (bus, tracks.len()) {
        (Some(_), n) if n > 0 => Err(format!("the name '{name}' is both a bus and a track; rename one to avoid confusion")),
        (Some(b), _) => Ok(Node::Bus(b)),
        (None, 1) => Ok(Node::Track(tracks[0])),
        (None, 0) => Err(format!("no track or bus named '{name}'")),
        (None, _) => Err(format!("{} tracks are named '{name}'; a referenced track name must be unique", tracks.len())),
    }
}

fn bus_index(p: &Project, name: &str, at: &str) -> Result<Option<usize>, String> {
    let name = name.trim();
    if name == MASTER {
        return Ok(None);
    }
    p.buses
        .iter()
        .position(|b| b.name == name)
        .map(Some)
        .ok_or_else(|| format!("{at}: no bus named '{name}' (available: master{})", p.buses.iter().map(|b| format!(", {}", b.name)).collect::<String>()))
}

/// 检查路由结构并给出渲染顺序。
pub fn plan(p: &Project) -> Result<Plan, String> {
    let mut seen = std::collections::HashSet::new();
    for (i, b) in p.buses.iter().enumerate() {
        let n = b.name.trim();
        if n.is_empty() || n == MASTER || n.contains('.') {
            return Err(format!("buses[{i}].name '{}' is invalid: it cannot be empty, be named master, or contain '.'", b.name));
        }
        if !seen.insert(n) {
            return Err(format!("duplicate bus name '{n}'"));
        }
        if !(-1.0..=1.0).contains(&b.pan) {
            return Err(format!("buses[{i}].pan = {} is out of range [-1, 1]", b.pan));
        }
        if !(LEVEL_RANGE.0..=LEVEL_RANGE.1).contains(&b.gain_db) {
            return Err(format!("buses[{i}].gain_db = {} is out of range [{}, {}]", b.gain_db, LEVEL_RANGE.0, LEVEL_RANGE.1));
        }
    }

    let mut edges = Vec::new();
    let mut add = |from: Node, output: &Option<String>, sends: &BTreeMap<String, f64>, at: &str| -> Result<(), String> {
        let to = match output {
            Some(o) => bus_index(p, o, &format!("{at}.output"))?,
            None => None,
        };
        edges.push(Edge { from, to, send: None, level_db: 0.0 });
        for (name, db) in sends {
            let to = bus_index(p, name, &format!("{at}.sends"))?.ok_or_else(|| format!("{at}.sends: cannot send to master (use output)"))?;
            if !(LEVEL_RANGE.0..=LEVEL_RANGE.1).contains(db) || !db.is_finite() {
                return Err(format!("{at}.sends.{name} = {db} is out of range [{}, {}] dB", LEVEL_RANGE.0, LEVEL_RANGE.1));
            }
            edges.push(Edge { from, to: Some(to), send: Some(name.clone()), level_db: *db });
        }
        Ok(())
    };
    for (i, t) in p.tracks.iter().enumerate() {
        add(Node::Track(i), &t.output, &t.sends, &format!("tracks[{i}]"))?;
    }
    for (i, b) in p.buses.iter().enumerate() {
        add(Node::Bus(i), &b.output, &b.sends, &format!("buses[{i}]"))?;
    }
    for (i, b) in p.buses.iter().enumerate() {
        if edges.iter().any(|e| e.from == Node::Bus(i) && e.to == Some(i)) {
            return Err(format!("buses[{i}] '{}' cannot output or send to itself", b.name));
        }
    }

    let mut sidechains = Vec::new();
    let mut scan = |node: Node, effects: &[EffectSlot], at: &str| -> Result<(), String> {
        for (ei, e) in effects.iter().enumerate() {
            if let Some(name) = e.effect.sidechain() {
                let key = find_node(p, name).map_err(|m| format!("{at}.effects[{ei}].sidechain: {m}"))?;
                if key == node {
                    return Err(format!("{at}.effects[{ei}].sidechain: cannot sidechain from itself"));
                }
                sidechains.push(Sidechain { node, effect: ei, key });
            }
        }
        Ok(())
    };
    for (i, t) in p.tracks.iter().enumerate() {
        scan(Node::Track(i), &t.effects, &format!("tracks[{i}]"))?;
    }
    for (i, b) in p.buses.iter().enumerate() {
        scan(Node::Bus(i), &b.effects, &format!("buses[{i}]"))?;
    }
    for (ei, e) in p.master_effects.iter().enumerate() {
        if let Some(name) = e.effect.sidechain() {
            find_node(p, name).map_err(|m| format!("master_effects[{ei}].sidechain: {m}"))?;
        }
    }

    let mut taps = Vec::new();
    for (i, t) in p.tracks.iter().enumerate() {
        let Some(list) = t.instrument_params.get("@outputs").and_then(|v| v.as_array()) else { continue };
        let mut index = 0;
        for (k, o) in list.iter().enumerate() {
            if let Some(bus) = o.get("bus").and_then(|b| b.as_str()) {
                if !t.instrument.trim().starts_with("plugin:") {
                    return Err(format!("tracks[{i}].instrument_params.@outputs is only for plugin instruments (plugin:)"));
                }
                let to = bus_index(p, bus, &format!("tracks[{i}].instrument_params.@outputs[{k}].bus"))?;
                taps.push(Tap { track: i, index, to });
                index += 1;
            }
        }
    }

    // 拓扑排序（Kahn）：依赖 = 送进来的信号 + 直接输出 + 侧链键。
    let nodes: Vec<Node> = (0..p.tracks.len()).map(Node::Track).chain((0..p.buses.len()).map(Node::Bus)).collect();
    let deps = |n: Node| -> Vec<Node> {
        let mut d: Vec<Node> = edges.iter().filter(|e| e.to.map(Node::Bus) == Some(n)).map(|e| e.from).collect();
        d.extend(taps.iter().filter(|t| t.to.map(Node::Bus) == Some(n)).map(|t| Node::Track(t.track)));
        d.extend(sidechains.iter().filter(|s| s.node == n).map(|s| s.key));
        d.sort();
        d.dedup();
        d
    };
    let mut order = Vec::new();
    let mut done = std::collections::HashSet::new();
    while order.len() < nodes.len() {
        let ready: Vec<Node> = nodes.iter().copied().filter(|n| !done.contains(n) && deps(*n).iter().all(|d| done.contains(d))).collect();
        if ready.is_empty() {
            let name = |n: &Node| match n {
                Node::Track(i) => format!("track '{}'", p.tracks[*i].name),
                Node::Bus(i) => format!("bus '{}'", p.buses[*i].name),
            };
            let stuck: Vec<String> = nodes.iter().filter(|n| !done.contains(*n)).map(name).collect();
            return Err(format!("routing has a cycle (output/send/sidechain dependencies): {}", stuck.join(", ")));
        }
        for n in ready {
            done.insert(n);
            order.push(n);
        }
    }
    Ok(Plan { order, edges, sidechains, taps })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 插件的直接输出：总线排在音轨之后；总线名要存在；只用于插件乐器。
    #[test]
    fn taps_order_and_checks() {
        let pr = |bus: &str, inst: &str| {
            project(&format!(
                r#"{{ "tempo": 120, "buses": [ {{ "name": "kick" }} ],
            "tracks": [ {{ "name": "kit", "instrument": "{inst}", "instrument_params": {{ "@outputs": [ {{ "channel": 1, "bus": "{bus}" }}, {{ "channel": 2 }} ] }} }} ] }}"#
            ))
        };
        let p = plan(&pr("kick", "plugin:/x.clap")).unwrap();
        assert_eq!(p.taps, vec![Tap { track: 0, index: 0, to: Some(0) }]);
        let at = |n: Node| p.order.iter().position(|x| *x == n).unwrap();
        assert!(at(Node::Track(0)) < at(Node::Bus(0)));
        assert!(plan(&pr("nope", "plugin:/x.clap")).unwrap_err().contains("no bus named 'nope'"));
        assert!(plan(&pr("kick", "drums.808")).unwrap_err().contains("only for plugin instruments"));
    }

    fn project(json: &str) -> Project {
        Project::from_json(json).unwrap()
    }

    #[test]
    fn plan_orders_tracks_before_buses_and_keys() {
        let p = project(
            r#"{ "tempo": 120,
            "buses": [ { "name": "drums", "output": "glue" }, { "name": "glue" }, { "name": "verb" } ],
            "tracks": [
              { "name": "kick", "instrument": "drums.808", "output": "drums" },
              { "name": "bass", "instrument": "bass.finger", "sends": { "verb": -12 },
                "effects": [ { "type": "compressor", "sidechain": "kick" } ] } ] }"#,
        );
        let plan = plan(&p).unwrap();
        let pos = |n| plan.order.iter().position(|x| *x == n).unwrap();
        assert!(pos(Node::Track(0)) < pos(Node::Bus(0)));
        assert!(pos(Node::Bus(0)) < pos(Node::Bus(1)));
        assert!(pos(Node::Track(0)) < pos(Node::Track(1)), "侧链键先于被压的轨");
        assert!(pos(Node::Track(1)) < pos(Node::Bus(2)));
        assert_eq!(plan.sidechains.len(), 1);
        assert_eq!(plan.edges.iter().filter(|e| e.send.is_some()).count(), 1);
    }

    #[test]
    fn plan_errors() {
        let err = |j: &str| plan(&project(j)).unwrap_err();
        assert!(err(r#"{ "tempo": 120, "buses": [ { "name": "a", "output": "b" }, { "name": "b", "output": "a" } ] }"#).contains("cycle"));
        assert!(err(r#"{ "tempo": 120, "tracks": [ { "instrument": "x", "output": "nope" } ] }"#).contains("nope"));
        assert!(err(r#"{ "tempo": 120, "buses": [ { "name": "master" } ] }"#).contains("master"));
        assert!(err(r#"{ "tempo": 120, "buses": [ { "name": "a" }, { "name": "a" } ] }"#).contains("duplicate"));
        assert!(err(r#"{ "tempo": 120, "tracks": [ { "instrument": "x", "sends": { "master": -6 } } ] }"#).contains("master"));
        assert!(err(r#"{ "tempo": 120, "tracks": [ { "name": "k", "instrument": "x" }, { "name": "k", "instrument": "x" },
            { "instrument": "x", "effects": [ { "type": "compressor", "sidechain": "k" } ] } ] }"#)
        .contains("unique"));
        // 侧链与输出形成环：t 输出到 a，a 的压缩器由 t 触发——合法（t 先于 a）。
        plan(&project(
            r#"{ "tempo": 120, "buses": [ { "name": "a", "effects": [ { "type": "compressor", "sidechain": "t" } ] } ],
            "tracks": [ { "name": "t", "instrument": "x", "output": "a" } ] }"#,
        ))
        .unwrap();
        // 真正的环：t 的压缩器由总线 a 触发，而 t 输出到 a。
        assert!(err(r#"{ "tempo": 120, "buses": [ { "name": "a" } ],
            "tracks": [ { "name": "t", "instrument": "x", "output": "a", "effects": [ { "type": "compressor", "sidechain": "a" } ] } ] }"#)
        .contains("cycle"));
    }
}
