//! `muser view`：给人听、调音量的页面，由本地服务与引擎联动。
//!
//! - 服务只监听 127.0.0.1。页面不下载整首音频：按播放进度向服务分段取已经混好的立体声（16-bit PCM），
//!   页面内存只占几秒钟，曲子多长都行。
//! - 边渲染边播放：引擎流式渲染，开头一小段好了就能播（乐器边演奏边出声，同时录进缓存），
//!   波形与电平随渲染进度补齐。
//! - 页面上改音量、静音、独奏，服务用引擎重新渲染（乐器输出有缓存，只重跑效果、总线与主总线），
//!   新版本渲染过当前播放位置后从这里无缝换上。独奏只用于试听，不写进工程。
//! - 「保存」只把改过的 `gain_db` / `muted` 写回工程文件；工程文件被别人（AI）改了会自动重新渲染、页面刷新。
//! - `--device`：声音从本机声卡出（`muser_engine::playback::Player`），页面只做遥控；改推子、静音、独奏
//!   在播放中即时生效，不等重新渲染。浏览器模式的渲染照常进行（波形、电平，并填充跳转用的乐器缓存）。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow, bail};
use muser_core::Project;
use muser_core::routing::{self, Node};
use muser_engine::{AudioBuffer, METER_WINDOW, RenderCache, RenderSettings, Renderer};
use serde_json::{Value, json};

use crate::web::{header, host_allowed, query, read_body, respond_json};

const PAGE: &str = include_str!("view.html");
/// 页面上的波形条数。
const PEAKS: usize = 2000;

/// 立体声 f32 的前 `n` 帧 → 交错的 16-bit PCM（小端），追加到 `out`。
fn pcm16(audio: &AudioBuffer<f32>, n: usize, out: &mut Vec<u8>) {
    let (l, r) = (audio.channel(0), audio.channel(1));
    out.reserve(n * 4);
    for f in 0..n {
        for s in [l[f], r[f]] {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
}

/// 交错的 16-bit 立体声 PCM → WAV 文件。
fn wav16(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    let len = pcm.len() as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// 下载用的 Content-Disposition：ASCII 文件名兜底，`filename*` 给出 UTF-8 原名。
fn attachment(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    let encoded: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// 波形：整首分成 [`PEAKS`] 段，每段每 4 帧取一个样本的峰值。`audio` 的前 `n` 帧是整首的第 `at` 帧起。
fn add_peaks(peaks: &mut [f32], total: usize, audio: &AudioBuffer<f32>, at: usize, n: usize) {
    let per = total.max(1).div_ceil(PEAKS).max(1);
    for i in 0..n {
        let f = at + i;
        if (f % per) % 4 == 0 {
            let p = &mut peaks[(f / per).min(PEAKS - 1)];
            *p = p.max(audio.channel(0)[i].abs().max(audio.channel(1)[i].abs()));
        }
    }
}

/// 电平：每 [`METER_WINDOW`] 帧一组 `[左峰值, 右峰值]`。`audio` 的前 `n` 帧是整首的第 `at` 帧起。
fn add_meter(meter: &mut Vec<[f32; 2]>, audio: &AudioBuffer<f32>, at: usize, n: usize) {
    for i in 0..n {
        let w = (at + i) / METER_WINDOW;
        if meter.len() <= w {
            meter.push([0.0; 2]);
        }
        for (c, m) in meter[w].iter_mut().enumerate() {
            *m = m.max(audio.channel(c)[i].abs());
        }
    }
}

/// 标准 base64。
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 电平：线性峰值 → 0..=255（-60 dBFS..=0 dBFS 线性映射到 dB 刻度），交错 L/R，base64。
fn encode_meter(peaks: &[[f32; 2]]) -> String {
    let q = |p: f32| {
        let db = 20.0 * f64::from(p.max(1e-9)).log10();
        (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 255.0).round() as u8
    };
    base64(&peaks.iter().flat_map(|[l, r]| [q(*l), q(*r)]).collect::<Vec<u8>>())
}

/// 效果器在信号流图里的显示：类型、简短标签、侧链键。
fn chain(list: &[muser_core::EffectSlot]) -> Vec<Value> {
    use muser_core::Effect;
    list.iter()
        .map(|slot| {
            let e = &slot.effect;
            let (label, sidechain) = match e {
                Effect::Plugin { path, id, params, .. } => {
                    let stem = Path::new(path).file_stem().map_or_else(|| path.clone(), |s| s.to_string_lossy().to_string());
                    // LV2 的 id 是 URI（同一个 bundle 含很多插件），取末段区分；VST3 的十六进制 id 不显示。
                    let uri_tail = id.as_deref().filter(|i| i.contains('/')).and_then(|i| i.rsplit(['/', '#']).next());
                    let pick = ["effect", "@preset", "type", "FX Type"].iter().find_map(|k| params.get(*k).and_then(Value::as_str)).or(uri_tail);
                    (pick.map_or(stem.clone(), |p| format!("{stem}: {p}")), None)
                }
                Effect::Compressor { sidechain, .. } => ("compressor".to_string(), sidechain.clone()),
                Effect::Eq { shape, .. } => (format!("eq {shape}"), None),
                e => (e.type_name().to_string(), None),
            };
            json!({ "type": e.type_name(), "label": label, "sidechain": sidechain, "bypass": slot.bypass })
        })
        .collect()
}

/// 页面提交的一项改动。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
struct Change {
    node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gain_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    muted: Option<bool>,
    /// 效果链每一格是否旁通（整条链，按顺序）。`node` 还可以是 `master`（主总线的效果链）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bypass: Option<Vec<bool>>,
}

fn effects_mut<'a>(p: &'a mut Project, id: &str) -> Result<&'a mut Vec<muser_core::EffectSlot>, String> {
    if id == "master" {
        return Ok(&mut p.master_effects);
    }
    match parse_node(id).ok_or_else(|| format!("无效的 node '{id}'"))? {
        ("tracks", i) => p.tracks.get_mut(i).map(|t| &mut t.effects),
        (_, i) => p.buses.get_mut(i).map(|b| &mut b.effects),
    }
    .ok_or_else(|| format!("没有 {id}"))
}

#[derive(serde::Deserialize)]
struct MixBody {
    #[serde(default)]
    changes: Vec<Change>,
    /// 给出时整体替换独奏集合（节点 id）。
    #[serde(default)]
    solo: Option<Vec<String>>,
}

/// 解析 `tracks[i]` / `buses[i]`。
fn parse_node(id: &str) -> Option<(&str, usize)> {
    let (kind, rest) = id.split_once('[')?;
    let i = rest.strip_suffix(']')?.parse().ok()?;
    matches!(kind, "tracks" | "buses").then_some((kind, i))
}

fn node_id(n: Node) -> String {
    match n {
        Node::Track(i) => format!("tracks[{i}]"),
        Node::Bus(i) => format!("buses[{i}]"),
    }
}

type Fields<'a> = (&'a mut f64, &'a mut bool, &'a mut Vec<muser_core::Automation>);

fn fields<'a>(p: &'a mut Project, id: &str) -> Result<Fields<'a>, String> {
    let (kind, i) = parse_node(id).ok_or_else(|| format!("无效的 node '{id}'"))?;
    match kind {
        "tracks" => p.tracks.get_mut(i).map(|t| (&mut t.gain_db, &mut t.muted, &mut t.automation)),
        _ => p.buses.get_mut(i).map(|b| (&mut b.gain_db, &mut b.muted, &mut b.automation)),
    }
    .ok_or_else(|| format!("{id} 不存在"))
}

/// 音量自动化的取值是绝对 dB，会盖过推子。推子改了 `delta` dB 时，把 `gain_db` 自动化整体平移同样的量
/// （DAW 里的 trim），这样有音量自动化的轨推子照样有效。
fn shift_gain_automation(autos: &mut [muser_core::Automation], delta: f64) {
    use muser_core::automation::Shape;
    let clamp = |v: f64| (v + delta).clamp(-80.0, 24.0);
    for a in autos.iter_mut().filter(|a| a.target.trim() == "gain_db") {
        match &mut a.shape {
            Shape::Points { points, .. } => {
                for (_, v) in points.iter_mut() {
                    if let Some(x) = v.as_f64() {
                        *v = json!(clamp(x));
                    }
                }
            }
            Shape::Lfo { min, max, .. } | Shape::Random { min, max, .. } => {
                *min = clamp(*min);
                *max = clamp(*max);
            }
        }
    }
}

fn has_gain_automation(autos: &[muser_core::Automation]) -> bool {
    autos.iter().any(|a| a.target.trim() == "gain_db")
}

fn apply_to_project(p: &mut Project, changes: &[Change]) -> Result<(), String> {
    for c in changes {
        if let Some(b) = &c.bypass {
            let fx = effects_mut(p, &c.node)?;
            if b.len() != fx.len() {
                return Err(format!("{}.bypass 应有 {} 项（效果链长度）", c.node, fx.len()));
            }
            for (slot, on) in fx.iter_mut().zip(b) {
                slot.bypass = *on;
            }
        }
        if c.node == "master" {
            continue;
        }
        let (gain, muted, autos) = fields(p, &c.node)?;
        if let Some(g) = c.gain_db {
            if !(-80.0..=24.0).contains(&g) {
                return Err(format!("{}.gain_db = {g} 超出范围 [-80, 24]", c.node));
            }
            shift_gain_automation(autos, g - *gain);
            *gain = g;
        }
        if let Some(m) = c.muted {
            *muted = m;
        }
    }
    Ok(())
}

/// 把改动写进原始 JSON（只动改了的字段，保留文件里其它写法）。
fn apply_to_json(v: &mut Value, changes: &[Change], working: &Project) -> Result<()> {
    let mut working = working.clone();
    for c in changes {
        if let Some(b) = &c.bypass {
            let list = match c.node.as_str() {
                "master" => v.get_mut("master_effects"),
                id => {
                    let (kind, i) = parse_node(id).ok_or_else(|| anyhow!("无效的 node '{id}'"))?;
                    v.get_mut(kind).and_then(|a| a.get_mut(i)).and_then(|o| o.get_mut("effects"))
                }
            }
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("工程文件里没有 {} 的效果链", c.node))?;
            for (fx, on) in list.iter_mut().zip(b) {
                if let Some(o) = fx.as_object_mut() {
                    if *on {
                        o.insert("bypass".into(), json!(true));
                    } else {
                        o.remove("bypass");
                    }
                }
            }
        }
        if c.node == "master" {
            continue;
        }
        let autos = fields(&mut working, &c.node).map_err(|e| anyhow!(e))?.2.clone();
        let (kind, i) = parse_node(&c.node).ok_or_else(|| anyhow!("无效的 node '{}'", c.node))?;
        let obj = v
            .get_mut(kind)
            .and_then(|a| a.get_mut(i))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("工程文件里没有 {}", c.node))?;
        if let Some(g) = c.gain_db {
            obj.insert("gain_db".into(), json!(g));
            if has_gain_automation(&autos) {
                obj.insert("automation".into(), serde_json::to_value(&autos)?);
            }
        }
        if let Some(m) = c.muted {
            obj.insert("muted".into(), json!(m));
        }
    }
    Ok(())
}

/// 工作副本相对文件的改动（只列出确实不同的字段）。
fn diff(file: &Project, working: &Project) -> Vec<Change> {
    let mut out = Vec::new();
    let flags = |fx: &[muser_core::EffectSlot]| fx.iter().map(|e| e.bypass).collect::<Vec<_>>();
    let mut push = |node: String, g0: f64, g1: f64, m0: bool, m1: bool, b0: Vec<bool>, b1: Vec<bool>| {
        let c = Change { node, gain_db: (g0 != g1).then_some(g1), muted: (m0 != m1).then_some(m1), bypass: (b0 != b1).then_some(b1) };
        if c.gain_db.is_some() || c.muted.is_some() || c.bypass.is_some() {
            out.push(c);
        }
    };
    for (i, (a, b)) in file.tracks.iter().zip(&working.tracks).enumerate() {
        push(format!("tracks[{i}]"), a.gain_db, b.gain_db, a.muted, b.muted, flags(&a.effects), flags(&b.effects));
    }
    for (i, (a, b)) in file.buses.iter().zip(&working.buses).enumerate() {
        push(format!("buses[{i}]"), a.gain_db, b.gain_db, a.muted, b.muted, flags(&a.effects), flags(&b.effects));
    }
    push("master".into(), 0.0, 0.0, false, false, flags(&file.master_effects), flags(&working.master_effects));
    out
}

/// 独奏：只留下独奏的音轨、以及经过（直接或间接送进）独奏总线的音轨。总线本身不静音。
fn with_solo(p: &Project, solo: &BTreeSet<String>) -> Project {
    let mut q = p.clone();
    if solo.is_empty() {
        return q;
    }
    let Ok(plan) = routing::plan(p) else { return q };
    let mut down: HashMap<Node, Vec<Node>> = HashMap::new();
    for e in &plan.edges {
        if let Some(b) = e.to {
            down.entry(e.from).or_default().push(Node::Bus(b));
        }
    }
    let reaches_solo = |start: Node| {
        let mut stack = vec![start];
        let mut seen = BTreeSet::new();
        while let Some(n) = stack.pop() {
            let id = node_id(n);
            if solo.contains(&id) {
                return true;
            }
            if seen.insert(id) {
                stack.extend(down.get(&n).into_iter().flatten().copied());
            }
        }
        false
    };
    for i in 0..q.tracks.len() {
        if !reaches_solo(Node::Track(i)) {
            q.tracks[i].muted = true;
        }
    }
    q
}

/// 页面需要的工程摘要（不含音频）。
fn summary(working: &Project, solo: &BTreeSet<String>) -> Value {
    let sections: Vec<Value> = muser_engine::arrange::timeline(working)
        .unwrap_or_default()
        .iter()
        .map(|s| json!({ "name": s.name, "occurrence": s.occurrence, "start": s.start, "length": s.length }))
        .collect();
    let arranged = muser_engine::arrange::arranged(working);
    let working: &Project = match &arranged {
        Ok(a) => a,
        Err(_) => working,
    };
    let fx = |list: &[muser_core::EffectSlot]| list.iter().map(|e| e.type_name()).collect::<Vec<_>>();
    let mut nodes = Vec::new();
    for (i, t) in working.tracks.iter().enumerate() {
        let id = format!("tracks[{i}]");
        let clips: Vec<Value> = t
            .clips
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let notes: Vec<Value> = muser_engine::clip_notes(working, i, ci).unwrap_or_default().iter().map(|n| json!([n.start.0, n.length.0, n.pitch])).collect();
                let mut v = json!({ "start": c.start.0, "length": c.length.0, "notes": notes });
                if let Some(a) = &c.audio {
                    let name = std::path::Path::new(&a.file).file_name().map_or(a.file.clone(), |n| n.to_string_lossy().into_owned());
                    v["audio"] = json!(name);
                    if let Some((wave, beats)) = muser_engine::audio::clip_wave(working, c, 400) {
                        v["wave"] = json!(wave.iter().map(|x| (x * 100.0).round() / 100.0).collect::<Vec<_>>());
                        v["wave_beats"] = json!(beats);
                    }
                }
                v
            })
            .collect();
        nodes.push(json!({
            "id": id, "kind": "track", "name": if t.name.is_empty() { id.clone() } else { t.name.clone() },
            "instrument": t.instrument, "label": t.instrument_params.get("@label"), "output": t.output.as_deref().unwrap_or("master"), "sends": t.sends,
            "gain_db": t.gain_db, "muted": t.muted, "solo": solo.contains(&id), "effects": fx(&t.effects), "clips": clips,
            "gain_auto": has_gain_automation(&t.automation), "chain": chain(&t.effects),
        }));
    }
    for (i, b) in working.buses.iter().enumerate() {
        let id = format!("buses[{i}]");
        nodes.push(json!({
            "id": id, "kind": "bus", "name": b.name, "output": b.output.as_deref().unwrap_or("master"), "sends": b.sends,
            "gain_db": b.gain_db, "muted": b.muted, "solo": solo.contains(&id), "effects": fx(&b.effects), "clips": [],
            "gain_auto": has_gain_automation(&b.automation), "chain": chain(&b.effects),
        }));
    }
    json!({
        "name": working.name, "tempo": working.tempo,
        "beats_per_bar": working.time_signature.numerator,
        "time_signature": format!("{}/{}", working.time_signature.numerator, working.time_signature.denominator),
        "duration_beats": working.duration_beats().0,
        "master_gain_db": working.master_gain_db, "master_effects": fx(&working.master_effects),
        "master_chain": chain(&working.master_effects),
        "nodes": nodes, "sections": sections,
    })
}

/// 一个版本：边渲染边往里写，读的一方按已渲染的部分取。
struct Version {
    /// 整首的帧数。
    frames: usize,
    summary: Value,
    progress: Mutex<Progress>,
    grown: Condvar,
}

#[derive(Default)]
struct Progress {
    pcm: Vec<u8>,
    /// 已渲染的帧数。
    rendered: usize,
    peaks: Vec<f32>,
    /// 节点 id（或 "master"）→ 电平。
    meters: BTreeMap<String, Vec<[f32; 2]>>,
    /// 渲染到结尾（或失败、被新请求取代）后不再增长。
    done: bool,
    seconds_to_render: f64,
}

struct State {
    /// 文件里的工程（最近一次成功读取）。
    file: Option<Project>,
    /// 工作副本 = 文件 + 页面改动。
    working: Option<Project>,
    solo: BTreeSet<String>,
    /// 有新的渲染请求（渲染线程处理后清除）。
    requested: bool,
    rendering: bool,
    version: u64,
    current: Option<Arc<Version>>,
    /// 上一版，供正在拉取的旧分段读完。
    previous: Option<(u64, Arc<Version>)>,
    error: Option<String>,
    file_mtime: Option<SystemTime>,
    /// 页面最近取音频的位置（帧），新版本渲染过这里才换上。
    fetching_at: usize,
    /// 声卡播放器当前载入的工程（去掉推子、静音后的 JSON；变了要重新载入）。
    #[cfg(feature = "device")]
    player_shape: Option<String>,
    /// 播放中结构变了的工程：等这一版渲染完、乐器输出进了缓存再交给声卡播放器（声卡线程不现场载入乐器，换音色时不卡）。
    player_pending: Option<Project>,
}

struct Shared {
    path: PathBuf,
    settings: RenderSettings,
    state: Mutex<State>,
    wake: Condvar,
    /// 乐器输出缓存：渲染线程填充，声卡播放跳转时复用。
    cache: Arc<Mutex<RenderCache>>,
    #[cfg(feature = "device")]
    player: Option<muser_engine::playback::Player>,
    /// 正在进行的音色编辑（插件界面子进程），同一时间一个。
    edit: Mutex<Option<EditSession>>,
}

/// 一次音色编辑：`muser plugin edit --serve` 子进程，插件状态每次自动存盘都同步进工程并重新渲染。
struct EditSession {
    track: usize,
    /// 插件引用（`路径#id`）。
    spec: String,
    label: String,
    stdin: std::process::ChildStdin,
    open: bool,
    synced: Option<SystemTime>,
    /// 已经把 `@state` 写进工程文件。
    wrote: bool,
    out: PathBuf,
    /// 打开前的音轨 JSON 与状态文件内容：放弃时恢复。
    backup: (Value, Option<Vec<u8>>),
    /// 采集：载入的是插件的这个内置音色，状态存在本机音色库里，结束时记进音色库索引。
    capture: Option<(muser_engine::presets::PluginRef, String)>,
    /// 采集没成功等，给页面看。
    error: Option<String>,
    /// 提示（如"载入的是另一个音色，已按它存进音色库"）。
    notice: Option<String>,
}

/// 换了乐器：旧乐器的参数与针对乐器参数的自动化（`target: instrument`）都作废；音量、声像、效果器的自动化保留。
fn switch_instrument(t: &mut serde_json::Map<String, Value>, instrument: &str) {
    t.insert("instrument".into(), json!(instrument));
    t.remove("instrument_params");
    if let Some(a) = t.get_mut("automation").and_then(Value::as_array_mut) {
        a.retain(|x| x.get("target").and_then(Value::as_str).map(str::trim) != Some("instrument"));
        if a.is_empty() {
            t.remove("automation");
        }
    }
}

/// 两个插件引用是不是同一个插件（`#id` 可省略：省略时文件里只有一个插件）。
fn same_plugin(a: &str, b: &str) -> bool {
    let (pa, ia) = muser_engine::plugin::split_spec(a);
    let (pb, ib) = muser_engine::plugin::split_spec(b);
    pa == pb && (ia.is_none() || ib.is_none() || ia == ib)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Shared {
    fn request_render(&self, st: &mut State) {
        st.requested = true;
        self.wake.notify_all();
        #[cfg(feature = "device")]
        if let (Some(player), Some(w)) = (&self.player, &st.working) {
            let p = with_solo(w, &st.solo);
            // 只改了推子、静音时即时生效；别的变了（文件被改）重新载入。
            let mut shape = p.clone();
            for t in &mut shape.tracks {
                (t.gain_db, t.muted) = (0.0, false);
                t.automation.retain(|a| a.target.trim() != "gain_db");
            }
            for b in &mut shape.buses {
                (b.gain_db, b.muted) = (0.0, false);
                b.automation.retain(|a| a.target.trim() != "gain_db");
            }
            shape.master_gain_db = 0.0;
            let shape = serde_json::to_string(&shape).unwrap_or_default();
            if st.player_shape.as_deref() == Some(shape.as_str()) {
                match st.player_pending.as_mut() {
                    Some(pending) => *pending = p,
                    None => player.set_mix(p),
                }
            } else if player.status().playing && st.player_shape.is_some() {
                // 正在播放时结构变了（换音色、改音符…）：旧的接着放，这一版渲染完、乐器输出进了缓存再在当前位置换上，
                // 免得声卡线程现场载入乐器而卡顿。
                st.player_shape = Some(shape);
                st.player_pending = Some(p);
            } else {
                // 没在播放（或第一次载入）：立刻交给声卡，边渲染边播。
                st.player_shape = Some(shape);
                st.player_pending = None;
                player.load(p);
            }
        }
    }

    /// 从磁盘读工程（启动时、文件变化时）。页面上未保存的改动丢弃。
    fn reload(&self) {
        let m = mtime(&self.path);
        let loaded = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("读取失败: {e}"))
            .and_then(|t| Project::from_json(&t).map(|p| p.at_file(&self.path)).map_err(|e| format!("解析失败: {e}")));
        let mut st = self.state.lock().unwrap();
        st.file_mtime = m;
        match loaded {
            Ok(p) => {
                let had_changes = match (&st.file, &st.working) {
                    (Some(f), Some(w)) => !diff(f, w).is_empty(),
                    _ => false,
                };
                if had_changes {
                    eprintln!("[view] 工程文件已变化，丢弃页面上未保存的改动");
                }
                // 结构变了（音轨/总线增删）时，独奏可能指向别的节点：清空。
                let same_shape = st.file.as_ref().is_some_and(|f| f.tracks.len() == p.tracks.len() && f.buses.len() == p.buses.len());
                if !same_shape {
                    st.solo.clear();
                }
                st.working = Some(p.clone());
                st.file = Some(p);
                self.request_render(&mut st);
            }
            Err(e) => {
                eprintln!("[view] {e}");
                st.error = Some(e);
            }
        }
    }

    /// 渲染线程：总是渲染最新的工作副本。渲染中来了新请求就放下当前这版，从头渲染新的。
    fn render_loop(&self) {
        loop {
            let project = {
                let mut st = self.state.lock().unwrap();
                while !st.requested {
                    st = self.wake.wait(st).unwrap();
                }
                st.requested = false;
                st.rendering = true;
                st.working.as_ref().map(|w| (with_solo(w, &st.solo), summary(w, &st.solo)))
            };
            let Some((project, summary)) = project else {
                self.state.lock().unwrap().rendering = false;
                continue;
            };
            let t0 = std::time::Instant::now();
            let renderer = Renderer::new(&project, self.settings, Some(&self.cache.lock().unwrap()), false).map(|r| r.with_meters().recording());
            let mut r = match renderer {
                Ok(r) => r,
                Err(e) => {
                    self.fail(format!("{e:#}"));
                    continue;
                }
            };
            let total = r.total_frames();
            let v = Arc::new(Version {
                frames: total,
                summary,
                progress: Mutex::new(Progress { peaks: vec![0.0; PEAKS], ..Default::default() }),
                grown: Condvar::new(),
            });
            // 还没有可播的版本时立即换上，否则等渲染过页面的播放位置。
            let mut published = false;
            if self.state.lock().unwrap().current.is_none() {
                self.publish(&v);
                published = true;
            }
            let mut block = AudioBuffer::<f32>::new(2, 8192);
            let outcome = loop {
                let n = match r.process(&mut block) {
                    Ok(0) => break Ok(true),
                    Ok(n) => n,
                    Err(e) => break Err(format!("{e:#}")),
                };
                {
                    let mut pr = v.progress.lock().unwrap();
                    let at = pr.rendered;
                    pcm16(&block, n, &mut pr.pcm);
                    add_peaks(&mut pr.peaks, total, &block, at, n);
                    add_meter(pr.meters.entry("master".into()).or_default(), &block, at, n);
                    pr.rendered = at + n;
                    // 节点电平每秒更新一次（复制有代价）。
                    if pr.rendered / 48_000 != at / 48_000 {
                        for m in r.meters() {
                            pr.meters.insert(node_id(m.node), m.peaks);
                        }
                    }
                    pr.seconds_to_render = t0.elapsed().as_secs_f64();
                }
                v.grown.notify_all();
                let st = self.state.lock().unwrap();
                let past_playhead = v.progress.lock().unwrap().rendered >= st.fetching_at + self.settings.sample_rate as usize;
                let superseded = st.requested;
                drop(st);
                if !published && past_playhead {
                    self.publish(&v);
                    published = true;
                }
                if superseded {
                    break Ok(false);
                }
            };
            {
                let mut pr = v.progress.lock().unwrap();
                if matches!(outcome, Ok(true)) {
                    for m in r.meters() {
                        pr.meters.insert(node_id(m.node), m.peaks);
                    }
                }
                pr.done = true;
                pr.seconds_to_render = t0.elapsed().as_secs_f64();
            }
            v.grown.notify_all();
            match outcome {
                Ok(true) => {
                    if !published {
                        self.publish(&v);
                    }
                    let mut cache = self.cache.lock().unwrap();
                    r.store_instruments(&mut cache);
                    let mut st = self.state.lock().unwrap();
                    self.hand_to_player(&mut st);
                    st.rendering = st.requested;
                    st.error = None;
                    eprintln!(
                        "[view] 版本 {}：渲染 {:.1}s（乐器缓存 {} 条，{:.0} MB）",
                        st.version,
                        t0.elapsed().as_secs_f64(),
                        cache.len(),
                        cache.bytes() as f64 / 1e6
                    );
                }
                Ok(false) => {}
                Err(e) => self.fail(e),
            }
        }
    }

    /// 换上新版本（上一版留着，让正在取的分段读完）。
    fn publish(&self, v: &Arc<Version>) {
        let mut st = self.state.lock().unwrap();
        if let Some(cur) = st.current.take() {
            st.previous = Some((st.version, cur));
        }
        st.version += 1;
        st.current = Some(Arc::clone(v));
    }

    /// 把等着的工程交给声卡播放器（在当前位置换上）。
    fn hand_to_player(&self, st: &mut State) {
        #[cfg(feature = "device")]
        if let (Some(player), Some(p)) = (&self.player, st.player_pending.take()) {
            player.load(p);
        }
        let _ = st;
    }

    fn fail(&self, msg: String) {
        eprintln!("[view] 渲染失败：{msg}");
        let mut st = self.state.lock().unwrap();
        // 渲染失败也交过去：声卡那边报出同样的错误，或用能渲染的部分。
        self.hand_to_player(&mut st);
        st.rendering = st.requested;
        st.error = Some(msg);
    }

    fn mix(&self, body: MixBody) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let working = st.working.as_mut().ok_or("工程尚未成功加载")?;
        let mut next = working.clone();
        apply_to_project(&mut next, &body.changes)?;
        *working = next;
        if let Some(solo) = body.solo {
            for id in &solo {
                parse_node(id).ok_or_else(|| format!("无效的 node '{id}'"))?;
            }
            st.solo = solo.into_iter().collect();
        }
        self.request_render(&mut st);
        Ok(())
    }

    fn save(&self) -> Result<usize, String> {
        let mut st = self.state.lock().unwrap();
        let (Some(file), Some(working)) = (&st.file, &st.working) else { return Err("工程尚未成功加载".into()) };
        let changes = diff(file, working);
        if changes.is_empty() {
            return Ok(0);
        }
        let write = || -> Result<()> {
            let text = std::fs::read_to_string(&self.path)?;
            let mut v: Value = serde_json::from_str(&text)?;
            apply_to_json(&mut v, &changes, working)?;
            std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
            Ok(())
        };
        write().map_err(|e| format!("保存失败: {e:#}"))?;
        st.file = st.working.clone();
        // 自己写的文件不触发重新加载。
        st.file_mtime = mtime(&self.path);
        eprintln!("[view] 已保存 {} 项改动到 {}", changes.len(), self.path.display());
        Ok(changes.len())
    }

    /// 可选的音色：内置乐器与 `$MUSER_SFZ_PATH` 里的 SFZ（直接换）、扫描到的 VST3 / CLAP 乐器插件（在插件窗口里选）。
    fn sound_choices() -> Vec<Value> {
        let mut out: Vec<Value> = muser_engine::instrument::catalog()
            .into_iter()
            .map(|i| json!({ "spec": i.name, "name": i.name, "vendor": i.family, "description": i.description, "format": "builtin" }))
            .collect();
        out.extend(muser_engine::sfz::library()
            .into_iter()
            .map(|(rel, path)| {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| rel.clone());
                let group = rel.split('/').next().filter(|g| *g != rel).unwrap_or("").to_string();
                json!({ "spec": format!("sfz:{rel}"), "name": name, "vendor": group, "format": "sfz" })
            }));
        if cfg!(all(target_os = "macos", feature = "host")) {
            if let Ok((list, _)) = muser_engine::plugin::scan(false) {
                out.extend(
                    list.into_iter()
                        .filter(|p| p.kind == "instrument" && (p.format == "vst3" || p.format == "clap"))
                        .map(|p| json!({ "spec": format!("plugin:{}#{}", p.path, p.id), "name": p.name, "vendor": p.vendor, "format": p.format })),
                );
            }
        }
        out
    }

    /// 直接换成一个不需要插件窗口的音色（内置乐器名或 `sfz:…`）：写进工程文件（文件监视会重新渲染）。
    fn sound_use(&self, ti: usize, instrument: &str, label: &str) -> Result<(), String> {
        if self.edit.lock().unwrap().as_ref().is_some_and(|e| e.open && e.track == ti) {
            return Err("这条轨的插件窗口还开着，先完成或放弃它".into());
        }
        let write = || -> Result<()> {
            let text = std::fs::read_to_string(&self.path)?;
            let mut v: Value = serde_json::from_str(&text)?;
            let t = v.get_mut("tracks").and_then(|t| t.get_mut(ti)).and_then(Value::as_object_mut).ok_or_else(|| anyhow!("工程文件里没有第 {ti} 条音轨"))?;
            if t.get("instrument").and_then(Value::as_str) != Some(instrument) {
                switch_instrument(t, instrument);
            }
            let label = label.trim();
            if !label.is_empty() {
                let p = t.entry("instrument_params").or_insert_with(|| json!({}));
                if let Some(p) = p.as_object_mut() {
                    p.insert("@label".into(), json!(label));
                }
            }
            std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
            Ok(())
        };
        write().map_err(|e| format!("写回工程失败: {e:#}"))
    }

    /// 打开第 `ti` 条音轨的音色编辑：`spec` 是要用的乐器插件（和现在的不同就换乐器，从插件的初始音色开始）。
    /// `fresh`：从插件的初始状态开始（载入一个新音色），否则从这条轨现在的音色开始调。
    fn edit_open(self: &Arc<Self>, ti: usize, spec: &str, label: &str, fresh: bool) -> Result<(), String> {
        self.edit_open_with(ti, spec, label, None, fresh)
    }

    /// 插件的一个内置音色：能直接换成状态的（自动 / 已采集）写进工程；要在插件窗口里载入的开一个采集窗口
    /// （在 Finder 里选中音色文件，人或 agent 把它载入后点完成），状态存进本机音色库，下次直接用。
    fn preset_use(self: &Arc<Self>, ti: usize, spec: &str, file: &str) -> Result<&'static str, String> {
        use muser_engine::presets;
        let p = presets::plugin_ref(spec)?;
        let x = presets::find(&p, file)?;
        if x.load != presets::Load::Manual {
            let state = presets::state_for(&p, &x)?;
            let write = || -> Result<()> {
                let text = std::fs::read_to_string(&self.path)?;
                let mut v: Value = serde_json::from_str(&text)?;
                let t = v.get_mut("tracks").and_then(|t| t.get_mut(ti)).and_then(Value::as_object_mut).ok_or_else(|| anyhow!("工程文件里没有第 {ti} 条音轨"))?;
                let same = t.get("instrument").and_then(Value::as_str).and_then(|i| i.strip_prefix("plugin:")).is_some_and(|c| same_plugin(c, &p.spec()[7..]));
                if !same {
                    switch_instrument(t, &p.spec());
                }
                t.insert("instrument_params".into(), json!({ "@state": state.display().to_string(), "@label": x.name }));
                std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
                Ok(())
            };
            write().map_err(|e| format!("写回工程失败: {e:#}"))?;
            return Ok("used");
        }
        // 这条轨的采集窗口已经开着：换成载入这个音色，窗口不用关。
        {
            use std::io::Write;
            let mut edit = self.edit.lock().unwrap();
            if let Some(e) = edit.as_mut().filter(|e| e.open && e.track == ti && e.capture.is_some() && same_plugin(&e.spec, &p.spec()[7..])) {
                let title = format!("{} — 载入「{}」后点 muser 的完成", p.name, x.name);
                e.label = x.name.clone();
                e.capture = Some((p, x.file));
                let line = json!({ "cmd": "title", "title": title });
                writeln!(e.stdin, "{line}").and_then(|_| e.stdin.flush()).map_err(|err| format!("插件窗口已退出: {err}"))?;
                return Ok("retarget");
            }
        }
        let label = x.name.clone();
        self.edit_open_with(ti, &p.spec(), &label, Some((p, x.file)), true)?;
        Ok("capture")
    }

    fn edit_open_with(self: &Arc<Self>, ti: usize, spec: &str, label: &str, capture: Option<(muser_engine::presets::PluginRef, String)>, fresh: bool) -> Result<(), String> {
        let spec = spec.strip_prefix("plugin:").unwrap_or(spec);
        if !cfg!(target_os = "macos") {
            return Err("插件界面目前只支持 macOS".into());
        }
        let mut edit = self.edit.lock().unwrap();
        if edit.as_ref().is_some_and(|e| e.open) {
            return Err("已经有一个插件窗口开着，先完成或放弃它".into());
        }
        let (name, current, mut params) = {
            let st = self.state.lock().unwrap();
            let file = st.file.as_ref().ok_or("工程尚未成功加载")?;
            let t = file.tracks.get(ti).ok_or_else(|| format!("没有第 {ti} 条音轨"))?;
            if t.instrument == "audio" {
                return Err("音频音轨没有乐器".into());
            }
            (if t.name.is_empty() { format!("track{ti}") } else { t.name.clone() }, t.instrument.clone(), t.instrument_params.clone())
        };
        let keep = current.strip_prefix("plugin:").is_some_and(|c| same_plugin(c, spec));
        if !keep || capture.is_some() || fresh {
            // 换插件、采集、载入新音色：从插件的初始状态开始。
            params.clear();
        }
        params.remove("@label");
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        let dir = self.path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let dir = std::fs::canonicalize(dir).map_err(|e| format!("{e}"))?;
        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("project").trim_end_matches(".muser");
        let out = match &capture {
            Some((p, file)) => muser_engine::presets::library_state_path(p, file),
            None => dir.join(format!("{stem}.{safe}.muserstate")),
        };
        if let Some(d) = out.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| format!("读取工程失败: {e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("解析工程失败: {e}"))?;
        let backup = (v["tracks"][ti].clone(), std::fs::read(&out).ok());
        if !keep || capture.is_some() || fresh {
            // 换插件 / 重新采集 / 新音色：旧的状态文件不能沿用（放弃时从 backup 恢复）。
            let _ = std::fs::remove_file(&out);
        }
        let plugin_name = muser_engine::plugin::split_spec(spec).0.rsplit('/').next().unwrap_or(spec).trim_end_matches(".vst3").trim_end_matches(".clap").to_string();
        let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| format!("{e}"))?)
            .args(["plugin", "edit", spec, "--serve", "--params", &Value::Object(params).to_string(), "--title", &match &capture {
                Some(_) => format!("{plugin_name} — 载入「{label}」后点 muser 的完成 · {name}"),
                None => format!("{plugin_name} — {name} · muser"),
            }, "-o"])
            .arg(&out)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动插件界面失败: {e}"))?;
        let stdin = child.stdin.take().ok_or("子进程没有 stdin")?;
        let stdout = child.stdout.take().ok_or("子进程没有 stdout")?;
        let label = if label.trim().is_empty() { plugin_name } else { label.trim().to_string() };
        eprintln!("[view] 打开 {spec} 的界面（音轨 {ti}），状态同步到 {}", out.display());
        *edit = Some(EditSession { track: ti, spec: spec.to_string(), label, stdin, open: true, synced: None, wrote: false, out, backup, capture, error: None, notice: None });
        drop(edit);
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.contains("\"saved\"") {
                    me.edit_synced();
                }
            }
            let _ = child.wait();
            me.edit_finish();
            eprintln!("[view] 插件窗口已关闭");
        });
        Ok(())
    }

    /// 编辑子进程存了一次盘：第一次把乐器、`@state`、`@label` 写进工程文件（文件监视会重新渲染），之后只重新渲染。
    fn edit_synced(&self) {
        let mut edit = self.edit.lock().unwrap();
        let Some(e) = edit.as_mut() else { return };
        e.synced = Some(SystemTime::now());
        if !e.wrote {
            if let Err(err) = self.edit_write(e) {
                eprintln!("[view] 写回工程失败: {err}");
                return;
            }
            e.wrote = true;
        } else {
            drop(edit);
            let mut st = self.state.lock().unwrap();
            self.request_render(&mut st);
        }
    }

    fn edit_write(&self, e: &EditSession) -> Result<()> {
        let text = std::fs::read_to_string(&self.path)?;
        let mut v: Value = serde_json::from_str(&text)?;
        let t = v.get_mut("tracks").and_then(|t| t.get_mut(e.track)).and_then(Value::as_object_mut).ok_or_else(|| anyhow!("工程文件里没有第 {} 条音轨", e.track))?;
        let keep = t.get("instrument").and_then(Value::as_str).and_then(|i| i.strip_prefix("plugin:")).is_some_and(|c| same_plugin(c, &e.spec));
        if !keep {
            switch_instrument(t, &format!("plugin:{}", e.spec));
        }
        let p = t.entry("instrument_params").or_insert_with(|| json!({}));
        let p = p.as_object_mut().ok_or_else(|| anyhow!("instrument_params 不是对象"))?;
        p.insert("@state".into(), json!(e.out.display().to_string()));
        p.insert("@label".into(), json!(e.label));
        std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
        Ok(())
    }

    /// 发命令给编辑子进程：`play`（循环试听这条音轨的音符）/ `stop` / `done`（`label` 可改名）/ `cancel`（恢复打开前的样子）。
    fn edit_cmd(&self, cmd: &str, label: Option<&str>) -> Result<(), String> {
        use std::io::Write;
        let mut edit = self.edit.lock().unwrap();
        let e = edit.as_mut().filter(|e| e.open).ok_or("没有开着的插件窗口")?;
        let line = match cmd {
            "play" => {
                let st = self.state.lock().unwrap();
                let p = st.working.as_ref().ok_or("工程尚未成功加载")?;
                let p = muser_engine::arrange::arranged(p).map_err(|e| e.to_string())?;
                let mut notes = Vec::new();
                for (ci, c) in p.tracks.get(e.track).map(|t| t.clips.as_slice()).unwrap_or_default().iter().enumerate() {
                    for n in muser_engine::clip_notes(&p, e.track, ci).unwrap_or_default() {
                        notes.push((n.pitch, c.start.0 + n.start.0, n.length.0, n.velocity));
                    }
                }
                if notes.is_empty() {
                    return Err("这条音轨没有音符可试听".into());
                }
                // 从第一个音所在的小节起，最多 8 小节一圈。
                let bar = f64::from(p.time_signature.numerator) * 4.0 / f64::from(p.time_signature.denominator);
                let from = (notes.iter().map(|n| n.1).fold(f64::INFINITY, f64::min) / bar).floor() * bar;
                let end = notes.iter().map(|n| n.1 + n.2).fold(0.0, f64::max);
                let beats = ((end - from) / bar).ceil().clamp(1.0, 8.0) * bar;
                let notes: Vec<_> = notes.into_iter().filter(|n| n.1 >= from && n.1 < from + beats).map(|n| (n.0, n.1 - from, n.2, n.3)).collect();
                json!({ "cmd": "play", "notes": notes, "beats": beats, "tempo": p.tempo })
            }
            "stop" | "done" | "cancel" => json!({ "cmd": cmd }),
            _ => return Err(format!("未知命令 '{cmd}'")),
        };
        if let Some(l) = label.map(str::trim).filter(|l| !l.is_empty()) {
            if l != e.label {
                e.label = l.to_string();
                if e.wrote {
                    self.edit_write(e).map_err(|err| format!("写回工程失败: {err:#}"))?;
                }
            }
        }
        if cmd == "cancel" {
            self.edit_restore(e)?;
            e.wrote = false;
        }
        writeln!(e.stdin, "{line}").and_then(|_| e.stdin.flush()).map_err(|err| format!("插件窗口已退出: {err}"))
    }

    /// 编辑窗口关了之后：采集的先听一遍（没声音就恢复原样）、认出实际载入的是哪个音色、记进音色库；
    /// 自己调的存一份进音色库的"我的音色"。听与认要几秒，不拿着锁做。
    fn edit_finish(&self) {
        use muser_engine::presets;
        let job = self.edit.lock().unwrap().as_ref().filter(|e| e.wrote && e.out.is_file()).map(|e| (e.spec.clone(), e.capture.clone(), e.out.clone(), e.label.clone()));
        let outcome: Option<Result<(String, PathBuf, Option<String>), String>> = job.map(|(spec, capture, out, label)| {
            let p = presets::plugin_ref(&spec)?;
            let Some((_, target)) = capture else {
                // 自己调的音色：存进"我的音色"，工程里仍用自己的那份。
                presets::save_mine(&p, &label, &out)?;
                eprintln!("[view] 已存进我的音色：{label}");
                return Ok((label, out, None));
            };
            let mut params = serde_json::Map::new();
            params.insert("@state".into(), json!(out.display().to_string()));
            let silent = muser_engine::sounds::describe(&p.spec(), &params, None, true).map_or(true, |c| c.summary.starts_with("没有声音"));
            if silent {
                return Err(format!("「{label}」没有载入成功（存下的状态没有声音），已恢复成打开前的样子：再试一次，等它在插件窗口里载入完再点完成"));
            }
            // 实际载入的是哪个：吻合度最高的那个（目标本身吻合就用目标）。
            let all: Vec<presets::Preset> = presets::presets(&p).into_iter().filter(|x| !x.file.ends_with(".muserstate")).collect();
            let ranked = presets::identify(&out, &all);
            let target_score = ranked.iter().find(|(_, x)| x.file == target).map_or(0.0, |r| r.0);
            let (file, name, notice) = match ranked.first() {
                Some((score, best)) if *score >= 0.3 && target_score < 0.3 => {
                    (best.file.clone(), best.name.clone(), Some(format!("窗口里载入的是「{}」，不是「{label}」：已按「{}」存进音色库", best.name, best.name)))
                }
                _ => (target.clone(), label.clone(), None),
            };
            let fin = presets::library_state_path(&p, &file);
            if fin != out {
                std::fs::rename(&out, &fin).or_else(|_| std::fs::copy(&out, &fin).map(|_| ())).map_err(|e| format!("存进音色库失败: {e}"))?;
            }
            presets::record_capture(&p, &file, &fin)?;
            eprintln!("[view] 已采集 {name} → {}", fin.display());
            Ok((name, fin, notice))
        });
        let mut edit = self.edit.lock().unwrap();
        let Some(e) = edit.as_mut() else { return };
        e.open = false;
        match outcome {
            Some(Ok((name, fin, notice))) => {
                if fin != e.out || name != e.label {
                    (e.out, e.label) = (fin, name);
                    if let Err(err) = self.edit_write(e) {
                        eprintln!("[view] 写回工程失败: {err}");
                    }
                }
                e.notice = notice;
            }
            Some(Err(msg)) => {
                if let Err(r) = self.edit_restore(e) {
                    eprintln!("[view] 恢复失败: {r}");
                }
                e.wrote = false;
                eprintln!("[view] {msg}");
                e.error = Some(msg);
            }
            None => {}
        }
    }

    /// 恢复打开编辑之前的音轨与状态文件。
    fn edit_restore(&self, e: &EditSession) -> Result<(), String> {
        if e.wrote {
            let restore = || -> Result<()> {
                let text = std::fs::read_to_string(&self.path)?;
                let mut v: Value = serde_json::from_str(&text)?;
                v["tracks"][e.track] = e.backup.0.clone();
                std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
                Ok(())
            };
            restore().map_err(|err| format!("恢复工程失败: {err:#}"))?;
        }
        match &e.backup.1 {
            Some(b) => std::fs::write(&e.out, b).map_err(|err| format!("恢复状态文件失败: {err}"))?,
            None if e.wrote => {
                let _ = std::fs::remove_file(&e.out);
            }
            None => {}
        }
        Ok(())
    }

    fn edit_status(&self) -> Value {
        let edit = self.edit.lock().unwrap();
        match edit.as_ref() {
            Some(e) => json!({ "open": e.open, "track": e.track, "spec": e.spec, "label": e.label, "error": e.error, "notice": e.notice,
                "synced": e.synced.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()) }),
            None => json!({ "open": false }),
        }
    }

    fn reset(&self) {
        let mut st = self.state.lock().unwrap();
        st.working = st.file.clone();
        st.solo.clear();
        self.request_render(&mut st);
    }

    fn status(&self) -> Value {
        let st = self.state.lock().unwrap();
        let progress = st.current.as_ref().map_or(0.0, |v| {
            let pr = v.progress.lock().unwrap();
            if pr.done { 1.0 } else { pr.rendered as f64 / v.frames.max(1) as f64 }
        });
        json!({ "version": st.version, "rendering": st.rendering || st.requested, "progress": progress, "error": st.error, "device": self.device_status() })
    }

    fn state_json(&self) -> Value {
        let st = self.state.lock().unwrap();
        let changes = match (&st.file, &st.working) {
            (Some(f), Some(w)) => diff(f, w),
            _ => Vec::new(),
        };
        let mut out = match &st.current {
            Some(v) => {
                let pr = v.progress.lock().unwrap();
                let mut s = v.summary.clone();
                s["frames"] = json!(v.frames);
                s["rendered"] = json!(pr.rendered);
                s["peaks"] = json!(pr.peaks.iter().map(|p| (p * 1000.0).round() / 1000.0).collect::<Vec<_>>());
                s["render_seconds"] = json!((pr.seconds_to_render * 10.0).round() / 10.0);
                s["meters"] = json!(pr.meters.iter().map(|(k, m)| (k.clone(), encode_meter(m))).collect::<BTreeMap<_, _>>());
                s["meter_window"] = json!(METER_WINDOW);
                s
            }
            None => json!({}),
        };
        out["ready"] = json!(st.current.is_some());
        out["device_mode"] = json!(!self.device_status().is_null());
        out["sample_rate"] = json!(self.settings.sample_rate);
        out["version"] = json!(st.version);
        out["rendering"] = json!(st.rendering || st.requested);
        out["error"] = json!(st.error);
        if let Some(f) = &st.file {
            let mut fv = serde_json::Map::new();
            for (i, t) in f.tracks.iter().enumerate() {
                fv.insert(format!("tracks[{i}]"), json!(t.gain_db));
            }
            for (i, b) in f.buses.iter().enumerate() {
                fv.insert(format!("buses[{i}]"), json!(b.gain_db));
            }
            out["file_values"] = Value::Object(fv);
        }
        out["changes"] = json!(changes);
        out["solo"] = json!(st.solo);
        out["project_path"] = json!(self.path.display().to_string());
        out["sound_edit"] = json!(cfg!(all(target_os = "macos", feature = "host")));
        out
    }

    fn device_status(&self) -> Value {
        #[cfg(feature = "device")]
        if let Some(p) = &self.player {
            return json!(p.status());
        }
        Value::Null
    }

    /// 声卡播放的走带：`play` / `pause` / `seek`（`seconds`）/ `monitor`（`db`）。
    fn transport(&self, body: &Value) -> Result<(), String> {
        #[cfg(feature = "device")]
        if let Some(p) = &self.player {
            let num = |k: &str| body.get(k).and_then(Value::as_f64).ok_or_else(|| format!("需要 {k}"));
            match body.get("action").and_then(Value::as_str) {
                Some("play") => p.play(),
                Some("pause") => p.pause(),
                Some("seek") => p.seek(num("seconds")?),
                Some("monitor") => p.set_monitor_db(num("db")?),
                _ => return Err("action 应为 play / pause / seek / monitor".into()),
            }
            return Ok(());
        }
        let _ = body;
        Err("没有用 --device 启动，声音在浏览器里播放".into())
    }

    /// 导出文件名的主干：工程文件名去掉 `.muser.json` / `.json`。
    fn export_stem(&self) -> String {
        let name = self.path.file_name().map_or("muser".into(), |n| n.to_string_lossy().into_owned());
        let stem = name.strip_suffix(".json").unwrap_or(&name);
        stem.strip_suffix(".muser").unwrap_or(stem).to_string()
    }

    /// 当前混音（含页面上的推子、静音、独奏）导出成 MIDI。
    fn export_midi(&self) -> Result<Vec<u8>, String> {
        let working = self.state.lock().unwrap().working.clone().ok_or("工程还没有读入")?;
        muser_engine::midi::export(&working).map(|m| m.bytes).map_err(|e| e.to_string())
    }

    /// 当前工程导出成 ABC（标准写法或 YuE2 写法，选项都用缺省）。
    fn export_abc(&self, yue2: bool) -> Result<Vec<u8>, String> {
        use muser_engine::abc_export::{AbcOptions, Dialect};
        let working = self.state.lock().unwrap().working.clone().ok_or("工程还没有读入")?;
        let opts = AbcOptions::new(if yue2 { Dialect::Yue2 } else { Dialect::Standard });
        muser_engine::abc_export::export(&working, &opts).map(|o| o.text.into_bytes()).map_err(|e| e.to_string())
    }

    /// 当前版本（正在听的混音）整首的 16-bit WAV。还在渲染时等它渲染完。
    fn export_wav(&self) -> Result<Vec<u8>, String> {
        let v = self.state.lock().unwrap().current.clone().ok_or("还没有渲染好的版本")?;
        let (pr, _) = v.grown.wait_timeout_while(v.progress.lock().unwrap(), Duration::from_secs(600), |pr| !pr.done).unwrap();
        if pr.rendered < v.frames {
            return Err("这一版没有渲染完（失败了，或被新的改动取代），稍后再导出".into());
        }
        Ok(wav16(&pr.pcm[..v.frames * 4], self.settings.sample_rate))
    }

    /// 某版本的一段 PCM。还没渲染到的部分最多等 2 秒，返回已有的（可能为空）。
    /// 版本不在了返回 None（页面会重新取状态）。
    fn pcm(&self, version: u64, from: usize, frames: usize) -> Option<(Vec<u8>, usize)> {
        let mut st = self.state.lock().unwrap();
        let v = match (&st.current, &st.previous) {
            (Some(c), _) if st.version == version => c.clone(),
            (_, Some((pv, p))) if *pv == version => p.clone(),
            _ => return None,
        };
        st.fetching_at = from;
        drop(st);
        let from = from.min(v.frames);
        let (pr, _) = v
            .grown
            .wait_timeout_while(v.progress.lock().unwrap(), Duration::from_secs(2), |pr| pr.rendered <= from && !pr.done)
            .unwrap();
        let to = (from + frames.min(48_000 * 10)).min(pr.rendered).max(from);
        Some((pr.pcm[from * 4..to * 4].to_vec(), v.frames))
    }
}

fn handle(shared: &Arc<Shared>, mut req: tiny_http::Request, port: u16) {
    use tiny_http::Method;
    if !host_allowed(&req, port) {
        respond_json(req, 403, &json!({ "error": "只接受来自本机页面的请求" }));
        return;
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    match (req.method(), path.as_str()) {
        (Method::Get, "/") => {
            let r = tiny_http::Response::from_string(PAGE)
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store"));
            let _ = req.respond(r);
        }
        (Method::Get, "/favicon.ico") => {
            let _ = req.respond(tiny_http::Response::empty(204));
        }
        (Method::Get, "/api/state") => respond_json(req, 200, &shared.state_json()),
        (Method::Get, "/api/status") => respond_json(req, 200, &shared.status()),
        (Method::Get, "/api/pcm") => {
            let q = query(&url);
            let num = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok());
            let (Some(v), Some(from), Some(frames)) = (num("v"), num("from"), num("frames")) else {
                return respond_json(req, 400, &json!({ "error": "需要 v / from / frames" }));
            };
            match shared.pcm(v, from as usize, frames as usize) {
                Some((bytes, total)) => {
                    let r = tiny_http::Response::from_data(bytes)
                        .with_header(header("Content-Type", "application/octet-stream"))
                        .with_header(header("X-Total-Frames", &total.to_string()))
                        .with_header(header("Cache-Control", "no-store"));
                    let _ = req.respond(r);
                }
                None => respond_json(req, 410, &json!({ "error": "版本已过期" })),
            }
        }
        (Method::Get, "/api/export.mid" | "/api/export.wav" | "/api/export.abc" | "/api/export.yue2.abc") => {
            let (result, suffix, mime) = match path.as_str() {
                "/api/export.wav" => (shared.export_wav(), "wav", "audio/wav"),
                "/api/export.mid" => (shared.export_midi(), "mid", "audio/midi"),
                "/api/export.abc" => (shared.export_abc(false), "abc", "text/vnd.abc; charset=utf-8"),
                _ => (shared.export_abc(true), "yue2.abc", "text/vnd.abc; charset=utf-8"),
            };
            match result {
                Ok(bytes) => {
                    let name = format!("{}.{suffix}", shared.export_stem());
                    let r = tiny_http::Response::from_data(bytes)
                        .with_header(header("Content-Type", mime))
                        .with_header(header("Content-Disposition", &attachment(&name)))
                        .with_header(header("Cache-Control", "no-store"));
                    let _ = req.respond(r);
                }
                Err(e) => respond_json(req, 409, &json!({ "error": e })),
            }
        }
        (Method::Post, "/api/mix") => match read_body::<MixBody>(&mut req).and_then(|b| shared.mix(b)) {
            Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
            Err(e) => respond_json(req, 400, &json!({ "error": e })),
        },
        (Method::Post, "/api/save") => match shared.save() {
            Ok(n) => respond_json(req, 200, &json!({ "ok": true, "saved": n })),
            Err(e) => respond_json(req, 400, &json!({ "error": e })),
        },
        (Method::Post, "/api/transport") => match read_body::<Value>(&mut req).and_then(|b| shared.transport(&b)) {
            Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
            Err(e) => respond_json(req, 400, &json!({ "error": e })),
        },
        (Method::Get, "/api/sound/choices") => respond_json(req, 200, &json!(Shared::sound_choices())),
        (Method::Get, "/api/sound/status") => respond_json(req, 200, &shared.edit_status()),
        (Method::Post, "/api/sound/open") => {
            let r = read_body::<Value>(&mut req).and_then(|b| {
                let ti = b["track"].as_u64().ok_or("需要 track")? as usize;
                let (spec, label) = (b["spec"].as_str().ok_or("需要 spec")?, b["label"].as_str().unwrap_or(""));
                if spec.starts_with("plugin:") {
                    shared.edit_open(ti, spec, label, b["fresh"].as_bool().unwrap_or(false))
                } else {
                    shared.sound_use(ti, spec, label)
                }
            });
            match r {
                Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
                Err(e) => respond_json(req, 400, &json!({ "error": e })),
            }
        }
        (Method::Get, "/api/sound/presets") => {
            let spec = query(&url).get("spec").map(|s| crate::poly::percent_decode(s)).unwrap_or_default();
            match muser_engine::presets::plugin_ref(&spec) {
                // 抽屉里只列能直接用的（我的音色、已采集、能自动转换的预设）；要在插件里载入的去插件自己的浏览器里选。
                Ok(p) => respond_json(req, 200, &json!(muser_engine::presets::presets(&p).into_iter().filter(|x| x.load != muser_engine::presets::Load::Manual).collect::<Vec<_>>())),
                Err(e) => respond_json(req, 400, &json!({ "error": e })),
            }
        }
        (Method::Post, "/api/sound/preset") => {
            let r = read_body::<Value>(&mut req).and_then(|b| {
                let ti = b["track"].as_u64().ok_or("需要 track")? as usize;
                shared.preset_use(ti, b["spec"].as_str().ok_or("需要 spec")?, b["file"].as_str().ok_or("需要 file")?)
            });
            match r {
                Ok(how) => respond_json(req, 200, &json!({ "ok": true, "how": how })),
                Err(e) => respond_json(req, 400, &json!({ "error": e })),
            }
        }
        (Method::Post, "/api/sound/cmd") => {
            let r = read_body::<Value>(&mut req).and_then(|b| shared.edit_cmd(b["cmd"].as_str().unwrap_or(""), b["label"].as_str()));
            match r {
                Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
                Err(e) => respond_json(req, 400, &json!({ "error": e })),
            }
        }
        (Method::Post, "/api/reset") => {
            shared.reset();
            respond_json(req, 200, &json!({ "ok": true }));
        }
        _ => respond_json(req, 404, &json!({ "error": "not found" })),
    }
}

/// 启动联动服务，阻塞运行直到进程被中断。`open` 时用系统浏览器打开页面。
pub fn serve(path: &str, settings: RenderSettings, port: u16, open: bool, device: bool) -> Result<()> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        bail!("找不到工程文件: {}", path.display());
    }
    let cache = Arc::new(Mutex::new(RenderCache::new()));
    #[cfg(feature = "device")]
    let player = match device {
        true => Some(muser_engine::playback::Player::start(settings, Arc::clone(&cache)).map_err(|e| anyhow!(e))?),
        false => None,
    };
    #[cfg(not(feature = "device"))]
    if device {
        bail!("这个 muser 构建时没有启用 device 特性，不能用声卡播放");
    }
    let shared = Arc::new(Shared {
        edit: Mutex::new(None),
        cache,
        #[cfg(feature = "device")]
        player,
        path,
        settings,
        state: Mutex::new(State {
            file: None,
            working: None,
            solo: BTreeSet::new(),
            requested: false,
            rendering: false,
            version: 0,
            current: None,
            previous: None,
            error: None,
            file_mtime: None,
            fetching_at: 0,
            #[cfg(feature = "device")]
            player_shape: None,
            player_pending: None,
        }),
        wake: Condvar::new(),
    });
    // 只绑定本机回环地址。
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("无法监听 127.0.0.1:{port}: {e}"))?;
    let url = format!("http://127.0.0.1:{port}/");
    {
        let s = Arc::clone(&shared);
        std::thread::spawn(move || s.render_loop());
    }
    shared.reload();
    println!("muser view 已启动：{url}  （只监听本机，无鉴权；Ctrl+C 退出）");
    #[cfg(feature = "device")]
    if let Some(p) = &shared.player {
        let s = p.status();
        println!("声卡播放：{}（{} Hz）。页面上的播放、推子都作用于本机声卡。", s.device, s.sample_rate);
    }
    println!("工程文件变化会自动重新渲染；页面上的改动点「保存」写回工程文件。");
    if open {
        crate::web::open_browser(&url);
    }
    {
        let watcher = Arc::clone(&shared);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let now = mtime(&watcher.path);
                let changed = now.is_some() && now != watcher.state.lock().unwrap().file_mtime;
                if changed {
                    eprintln!("[view] 工程文件已变化，重新渲染");
                    watcher.reload();
                }
            }
        });
    }
    for req in server.incoming_requests() {
        let s = Arc::clone(&shared);
        std::thread::spawn(move || handle(&s, req, port));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project::from_json(
            r#"{ "tempo": 120, "buses": [{ "name": "v" }, { "name": "w", "output": "v" }],
                 "tracks": [{ "name": "a", "instrument": "synth.poly", "output": "w", "clips": [] },
                            { "name": "b", "instrument": "synth.poly", "clips": [] },
                            { "name": "c", "instrument": "synth.poly", "sends": { "v": -6 }, "clips": [] }] }"#,
        )
        .unwrap()
    }

    #[test]
    fn apply_changes_to_project_and_json() {
        let mut p = project();
        let ch = vec![
            Change { node: "tracks[0]".into(), gain_db: Some(-4.5), muted: None, bypass: None },
            Change { node: "buses[0]".into(), gain_db: None, muted: Some(true), bypass: None },
        ];
        apply_to_project(&mut p, &ch).unwrap();
        assert_eq!((p.tracks[0].gain_db, p.buses[0].muted), (-4.5, true));
        assert_eq!(diff(&project(), &p), ch);
        let mut v: Value = serde_json::to_value(project()).unwrap();
        apply_to_json(&mut v, &ch, &p).unwrap();
        assert_eq!(v["tracks"][0]["gain_db"], json!(-4.5));
        assert_eq!(v["buses"][0]["muted"], json!(true));
        let bad = vec![Change { node: "tracks[9]".into(), gain_db: Some(0.0), muted: None, bypass: None }];
        assert!(apply_to_project(&mut p, &bad).is_err());
        let bad = vec![Change { node: "tracks[0]".into(), gain_db: Some(99.0), muted: None, bypass: None }];
        assert!(apply_to_project(&mut p, &bad).unwrap_err().contains("超出范围"));
    }

    #[test]
    fn switching_instrument_drops_its_automation() {
        let mut t = json!({ "instrument": "poly.supersaw", "instrument_params": { "cutoff": 900 },
            "automation": [{ "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 160], [8, 9000]] },
                           { "target": "gain_db", "shape": "points", "points": [[0, -30], [8, 0]] }] });
        switch_instrument(t.as_object_mut().unwrap(), "plugin:/k.vst3#X");
        assert_eq!(t["instrument"], "plugin:/k.vst3#X");
        assert!(t.get("instrument_params").is_none());
        assert_eq!(t["automation"].as_array().unwrap().len(), 1);
        assert_eq!(t["automation"][0]["target"], "gain_db");
    }

    #[test]
    fn bypass_changes_round_trip() {
        let file = Project::from_json(
            r#"{ "tempo": 120, "master_effects": [{ "type": "limiter" }],
                 "tracks": [{ "name": "a", "instrument": "synth.poly", "clips": [], "effects": [{ "type": "reverb" }, { "type": "eq", "bypass": true }] }] }"#,
        )
        .unwrap();
        let mut p = file.clone();
        let ch = vec![
            Change { node: "tracks[0]".into(), gain_db: None, muted: None, bypass: Some(vec![true, false]) },
            Change { node: "master".into(), gain_db: None, muted: None, bypass: Some(vec![true]) },
        ];
        apply_to_project(&mut p, &ch).unwrap();
        assert!(p.tracks[0].effects[0].bypass && !p.tracks[0].effects[1].bypass && p.master_effects[0].bypass);
        assert_eq!(diff(&file, &p), ch);
        let mut v: Value = serde_json::to_value(&file).unwrap();
        apply_to_json(&mut v, &ch, &p).unwrap();
        assert_eq!(v["tracks"][0]["effects"][0]["bypass"], json!(true));
        assert!(v["tracks"][0]["effects"][1].get("bypass").is_none(), "关掉旁通时删掉字段");
        assert_eq!(v["master_effects"][0]["bypass"], json!(true));
        let wrong = vec![Change { node: "tracks[0]".into(), gain_db: None, muted: None, bypass: Some(vec![true]) }];
        assert!(apply_to_project(&mut p, &wrong).unwrap_err().contains("2 项"));
    }

    #[test]
    fn fader_trims_gain_automation() {
        let mut p = Project::from_json(
            r#"{ "tempo": 120, "tracks": [{ "name": "a", "instrument": "synth.poly", "gain_db": -10, "clips": [],
                 "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, -30], [8, -10]] },
                                { "target": "pan", "shape": "lfo", "period_beats": 4, "min": -0.5, "max": 0.5 }] }] }"#,
        )
        .unwrap();
        let file = p.clone();
        let ch = vec![Change { node: "tracks[0]".into(), gain_db: Some(-6.0), muted: None, bypass: None }];
        apply_to_project(&mut p, &ch).unwrap();
        let pts = match &p.tracks[0].automation[0].shape {
            muser_core::automation::Shape::Points { points, .. } => points.iter().map(|(_, v)| v.as_f64().unwrap()).collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        assert_eq!(pts, [-26.0, -6.0], "音量自动化整体平移 +4 dB");
        assert_eq!(p.tracks[0].automation[1], file.tracks[0].automation[1], "声像自动化不变");
        let mut v: Value = serde_json::to_value(&file).unwrap();
        apply_to_json(&mut v, &diff(&file, &p), &p).unwrap();
        assert_eq!(v["tracks"][0]["automation"][0]["points"][0][1], json!(-26.0));
        // 引擎确实听得到：渲染能量随推子变化。
        let mut q = file.clone();
        q.tracks[0].clips = serde_json::from_value(json!([{ "start": 0, "length": 8, "notes": [{ "pitch": 60, "start": 0, "length": 8 }] }])).unwrap();
        let mut louder = q.clone();
        apply_to_project(&mut louder, &[Change { node: "tracks[0]".into(), gain_db: Some(0.0), muted: None, bypass: None }]).unwrap();
        let e = |p: &Project| muser_engine::render(p, RenderSettings { tail_seconds: 0.1, ..Default::default() }).unwrap().channel(0).iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        assert!(e(&louder) > e(&q) * 5.0);
    }

    #[test]
    fn solo_follows_routing() {
        let p = project();
        let muted = |solo: &[&str]| {
            let q = with_solo(&p, &solo.iter().map(|s| s.to_string()).collect());
            q.tracks.iter().map(|t| t.muted).collect::<Vec<_>>()
        };
        assert_eq!(muted(&[]), [false, false, false]);
        assert_eq!(muted(&["tracks[1]"]), [true, false, true]);
        // 独奏总线 v：a 经 w 进 v，c 发送到 v。
        assert_eq!(muted(&["buses[0]"]), [false, true, false]);
        assert_eq!(muted(&["buses[1]", "tracks[1]"]), [false, false, true]);
    }

    #[test]
    fn pcm_and_peaks() {
        let mut a = AudioBuffer::<f32>::new(2, 3);
        a.channel_mut(0).copy_from_slice(&[1.0, -1.0, 0.5]);
        let mut b = Vec::new();
        pcm16(&a, 3, &mut b);
        assert_eq!(b.len(), 12);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), 32767);
        assert_eq!(i16::from_le_bytes([b[4], b[5]]), -32767);
        // 分两次加与一次加相同。
        let total = 3 * PEAKS * 4;
        let mut long = AudioBuffer::<f32>::new(2, total);
        for (i, s) in long.channel_mut(0).iter_mut().enumerate() {
            *s = (i as f32 * 0.37).sin();
        }
        let mut once = vec![0.0; PEAKS];
        add_peaks(&mut once, total, &long, 0, total);
        let mut twice = vec![0.0; PEAKS];
        let half = total / 2 + 3;
        let mut tail = AudioBuffer::<f32>::new(2, total - half);
        for c in 0..2 {
            tail.channel_mut(c).copy_from_slice(&long.channel(c)[half..]);
        }
        add_peaks(&mut twice, total, &long, 0, half);
        add_peaks(&mut twice, total, &tail, half, total - half);
        assert_eq!(once, twice);
        let (mut m1, mut m2) = (Vec::new(), Vec::new());
        add_meter(&mut m1, &long, 0, total);
        add_meter(&mut m2, &long, 0, half);
        add_meter(&mut m2, &tail, half, total - half);
        assert_eq!(m1, m2);
        assert_eq!(m1.len(), total.div_ceil(METER_WINDOW));
    }
}
