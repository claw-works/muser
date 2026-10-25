//! `muser view`：给人听、调音量的页面，由本地服务与引擎联动。
//!
//! - 服务只监听 127.0.0.1。页面不下载整首音频：按播放进度向服务分段取已经混好的立体声（16-bit PCM），
//!   内存只占几秒钟，曲子多长都行。
//! - 页面上改音量、静音、独奏，服务用引擎重新渲染（乐器输出有缓存，只重跑效果、总线与主总线），
//!   新版本就绪后从当前播放位置无缝换上。独奏只用于试听，不写进工程。
//! - 「保存」只把改过的 `gain_db` / `muted` 写回工程文件；工程文件被别人（AI）改了会自动重新渲染、页面刷新。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow, bail};
use muser_core::Project;
use muser_core::routing::{self, Node};
use muser_engine::{AudioBuffer, METER_WINDOW, RenderCache, RenderSettings, render_metered};
use serde_json::{Value, json};

const PAGE: &str = include_str!("view.html");
/// 页面上的波形条数。
const PEAKS: usize = 2000;

/// 立体声 f32 → 交错的 16-bit PCM（小端）。
fn pcm16(audio: &AudioBuffer<f32>) -> Vec<u8> {
    let (l, r) = (audio.channel(0), audio.channel(1));
    let mut out = Vec::with_capacity(l.len() * 4);
    for f in 0..l.len() {
        for s in [l[f], r[f]] {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    out
}

fn peaks(audio: &AudioBuffer<f32>) -> Vec<f32> {
    let n = audio.num_frames().max(1);
    let per = n.div_ceil(PEAKS).max(1);
    (0..PEAKS)
        .map(|i| {
            let (a, b) = ((i * per).min(n), ((i + 1) * per).min(n));
            (a..b).step_by(4).map(|f| audio.channel(0)[f].abs().max(audio.channel(1)[f].abs())).fold(0f32, f32::max)
        })
        .map(|p| (p * 1000.0).round() / 1000.0)
        .collect()
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

fn meter_of(audio: &AudioBuffer<f32>) -> Vec<[f32; 2]> {
    let n = audio.num_frames();
    (0..n.div_ceil(METER_WINDOW))
        .map(|w| {
            let r = w * METER_WINDOW..((w + 1) * METER_WINDOW).min(n);
            let pk = |ch: usize| audio.channel(ch)[r.clone()].iter().fold(0f32, |m, s| m.max(s.abs()));
            [pk(0), pk(1)]
        })
        .collect()
}

/// 效果器在信号流图里的显示：类型、简短标签、侧链键。
fn chain(list: &[muser_core::Effect]) -> Vec<Value> {
    use muser_core::Effect;
    list.iter()
        .map(|e| {
            let (label, sidechain) = match e {
                Effect::Plugin { path, params, .. } => {
                    let stem = Path::new(path).file_stem().map_or_else(|| path.clone(), |s| s.to_string_lossy().to_string());
                    let pick = ["effect", "@preset", "type", "FX Type"].iter().find_map(|k| params.get(*k).and_then(Value::as_str));
                    (pick.map_or(stem.clone(), |p| format!("{stem}: {p}")), None)
                }
                Effect::Compressor { sidechain, .. } => ("compressor".to_string(), sidechain.clone()),
                Effect::Eq { shape, .. } => (format!("eq {shape}"), None),
                e => (e.type_name().to_string(), None),
            };
            json!({ "type": e.type_name(), "label": label, "sidechain": sidechain })
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
    let mut push = |node: String, g0: f64, g1: f64, m0: bool, m1: bool| {
        let c = Change { node, gain_db: (g0 != g1).then_some(g1), muted: (m0 != m1).then_some(m1) };
        if c.gain_db.is_some() || c.muted.is_some() {
            out.push(c);
        }
    };
    for (i, (a, b)) in file.tracks.iter().zip(&working.tracks).enumerate() {
        push(format!("tracks[{i}]"), a.gain_db, b.gain_db, a.muted, b.muted);
    }
    for (i, (a, b)) in file.buses.iter().zip(&working.buses).enumerate() {
        push(format!("buses[{i}]"), a.gain_db, b.gain_db, a.muted, b.muted);
    }
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
    let fx = |list: &[muser_core::Effect]| list.iter().map(|e| e.type_name()).collect::<Vec<_>>();
    let mut nodes = Vec::new();
    for (i, t) in working.tracks.iter().enumerate() {
        let id = format!("tracks[{i}]");
        let clips: Vec<Value> = t
            .clips
            .iter()
            .map(|c| {
                let notes: Vec<Value> = c.notes.iter().map(|n| json!([n.start.0, n.length.0, n.pitch])).collect();
                json!({ "start": c.start.0, "length": c.length.0, "notes": notes })
            })
            .collect();
        nodes.push(json!({
            "id": id, "kind": "track", "name": if t.name.is_empty() { id.clone() } else { t.name.clone() },
            "instrument": t.instrument, "output": t.output.as_deref().unwrap_or("master"), "sends": t.sends,
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
        "nodes": nodes,
    })
}

/// 一个渲染好的版本。
struct Version {
    pcm: Arc<Vec<u8>>,
    frames: usize,
    peaks: Vec<f32>,
    summary: Value,
    seconds_to_render: f64,
    /// 节点 id（或 "master"）→ 编码后的电平。
    meters: BTreeMap<String, String>,
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
}

struct Shared {
    path: PathBuf,
    settings: RenderSettings,
    state: Mutex<State>,
    wake: Condvar,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Shared {
    fn request_render(&self, st: &mut State) {
        st.requested = true;
        self.wake.notify_all();
    }

    /// 从磁盘读工程（启动时、文件变化时）。页面上未保存的改动丢弃。
    fn reload(&self) {
        let m = mtime(&self.path);
        let loaded = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("读取失败: {e}"))
            .and_then(|t| Project::from_json(&t).map_err(|e| format!("解析失败: {e}")));
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

    /// 渲染线程：合并请求，总是渲染最新的工作副本。
    fn render_loop(&self) {
        let mut cache = RenderCache::new();
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
            let result = render_metered(&project, self.settings, &mut cache);
            let secs = t0.elapsed().as_secs_f64();
            let mut st = self.state.lock().unwrap();
            st.rendering = st.requested;
            match result {
                Ok((audio, node_meters)) => {
                    let mut meters: BTreeMap<String, String> =
                        node_meters.iter().map(|m| (node_id(m.node), encode_meter(&m.peaks))).collect();
                    meters.insert("master".into(), encode_meter(&meter_of(&audio)));
                    let v = Arc::new(Version {
                        meters,
                        pcm: Arc::new(pcm16(&audio)),
                        frames: audio.num_frames(),
                        peaks: peaks(&audio),
                        summary,
                        seconds_to_render: secs,
                    });
                    if let Some(cur) = st.current.take() {
                        st.previous = Some((st.version, cur));
                    }
                    st.version += 1;
                    st.current = Some(v);
                    st.error = None;
                    eprintln!("[view] 版本 {}：渲染 {secs:.1}s（乐器缓存 {} 条，{:.0} MB）", st.version, cache.len(), cache.bytes() as f64 / 1e6);
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    eprintln!("[view] 渲染失败：{msg}");
                    st.error = Some(msg);
                }
            }
        }
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

    fn reset(&self) {
        let mut st = self.state.lock().unwrap();
        st.working = st.file.clone();
        st.solo.clear();
        self.request_render(&mut st);
    }

    fn status(&self) -> Value {
        let st = self.state.lock().unwrap();
        json!({ "version": st.version, "rendering": st.rendering || st.requested, "error": st.error })
    }

    fn state_json(&self) -> Value {
        let st = self.state.lock().unwrap();
        let changes = match (&st.file, &st.working) {
            (Some(f), Some(w)) => diff(f, w),
            _ => Vec::new(),
        };
        let mut out = match &st.current {
            Some(v) => {
                let mut s = v.summary.clone();
                s["frames"] = json!(v.frames);
                s["peaks"] = json!(v.peaks);
                s["render_seconds"] = json!((v.seconds_to_render * 10.0).round() / 10.0);
                s["meters"] = json!(v.meters);
                s["meter_window"] = json!(METER_WINDOW);
                s
            }
            None => json!({}),
        };
        out["ready"] = json!(st.current.is_some());
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
        out
    }

    /// 某版本的一段 PCM。版本不在了返回 None（页面会重新取状态）。
    fn pcm(&self, version: u64, from: usize, frames: usize) -> Option<(Vec<u8>, usize)> {
        let st = self.state.lock().unwrap();
        let v = match (&st.current, &st.previous) {
            (Some(c), _) if st.version == version => c.clone(),
            (_, Some((pv, p))) if *pv == version => p.clone(),
            _ => return None,
        };
        drop(st);
        let from = from.min(v.frames);
        let to = (from + frames.min(48_000 * 10)).min(v.frames);
        Some((v.pcm[from * 4..to * 4].to_vec(), v.frames))
    }
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("合法的 header")
}

fn respond_json(req: tiny_http::Request, status: u16, v: &Value) {
    let r = tiny_http::Response::from_string(v.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    let _ = req.respond(r);
}

/// 只接受发往本机地址的请求（防 DNS rebinding：别的网站把域名解析到 127.0.0.1 来调用本服务）。
fn host_allowed(req: &tiny_http::Request, port: u16) -> bool {
    let ok = |v: &str| v == format!("127.0.0.1:{port}") || v == format!("localhost:{port}");
    let get = |name: &str| req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string());
    let host_ok = get("Host").is_some_and(|h| ok(&h));
    let origin_ok = get("Origin").is_none_or(|o| o.strip_prefix("http://").is_some_and(ok));
    host_ok && origin_ok
}

fn query(url: &str) -> BTreeMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
        .unwrap_or_default()
}

fn read_body<T: serde::de::DeserializeOwned>(req: &mut tiny_http::Request) -> Result<T, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(&mut req.as_reader(), &mut body).map_err(|_| "读取请求失败".to_string())?;
    if body.trim().is_empty() {
        body = "{}".into();
    }
    serde_json::from_str(&body).map_err(|e| format!("请求格式错误: {e}"))
}

fn handle(shared: &Shared, mut req: tiny_http::Request, port: u16) {
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
        (Method::Post, "/api/mix") => match read_body::<MixBody>(&mut req).and_then(|b| shared.mix(b)) {
            Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
            Err(e) => respond_json(req, 400, &json!({ "error": e })),
        },
        (Method::Post, "/api/save") => match shared.save() {
            Ok(n) => respond_json(req, 200, &json!({ "ok": true, "saved": n })),
            Err(e) => respond_json(req, 400, &json!({ "error": e })),
        },
        (Method::Post, "/api/reset") => {
            shared.reset();
            respond_json(req, 200, &json!({ "ok": true }));
        }
        _ => respond_json(req, 404, &json!({ "error": "not found" })),
    }
}

/// 启动联动服务，阻塞运行直到进程被中断。`open` 时用系统浏览器打开页面。
pub fn serve(path: &str, settings: RenderSettings, port: u16, open: bool) -> Result<()> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        bail!("找不到工程文件: {}", path.display());
    }
    let shared = Arc::new(Shared {
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
    println!("工程文件变化会自动重新渲染；页面上的改动点「保存」写回工程文件。");
    if open {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        if let Err(e) = std::process::Command::new(opener).arg(&url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            eprintln!("[view] 无法打开浏览器（{opener}: {e}），请手动打开 {url}");
        }
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
            Change { node: "tracks[0]".into(), gain_db: Some(-4.5), muted: None },
            Change { node: "buses[0]".into(), gain_db: None, muted: Some(true) },
        ];
        apply_to_project(&mut p, &ch).unwrap();
        assert_eq!((p.tracks[0].gain_db, p.buses[0].muted), (-4.5, true));
        assert_eq!(diff(&project(), &p), ch);
        let mut v: Value = serde_json::to_value(project()).unwrap();
        apply_to_json(&mut v, &ch, &p).unwrap();
        assert_eq!(v["tracks"][0]["gain_db"], json!(-4.5));
        assert_eq!(v["buses"][0]["muted"], json!(true));
        let bad = vec![Change { node: "tracks[9]".into(), gain_db: Some(0.0), muted: None }];
        assert!(apply_to_project(&mut p, &bad).is_err());
        let bad = vec![Change { node: "tracks[0]".into(), gain_db: Some(99.0), muted: None }];
        assert!(apply_to_project(&mut p, &bad).unwrap_err().contains("超出范围"));
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
        let ch = vec![Change { node: "tracks[0]".into(), gain_db: Some(-6.0), muted: None }];
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
        apply_to_project(&mut louder, &[Change { node: "tracks[0]".into(), gain_db: Some(0.0), muted: None }]).unwrap();
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
        let b = pcm16(&a);
        assert_eq!(b.len(), 12);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), 32767);
        assert_eq!(i16::from_le_bytes([b[4], b[5]]), -32767);
        assert_eq!(peaks(&a).len(), PEAKS);
    }
}
