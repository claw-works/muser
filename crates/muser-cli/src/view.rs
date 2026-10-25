//! `muser view`：给人看的界面。
//!
//! - 静态模式：渲染成品与分轨，把音频与工程摘要嵌进一个单文件 HTML，浏览器打开即可播放、调分轨音量。
//! - 联动模式（`--serve`）：在 127.0.0.1 起本地 HTTP 服务。页面上的改动（音量、静音）交给引擎重新渲染，
//!   可保存回工程文件；工程文件被别人（AI）改了，服务自动重新渲染，页面自动刷新。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use muser_core::Project;
use muser_core::routing::Node;
use muser_engine::{RenderSettings, Rendered, render_with_stems};
use serde_json::{Value, json};

const TEMPLATE: &str = include_str!("view.html");
const PLACEHOLDER: &str = "/*MUSER_DATA*/null";

/// 立体声 f32 → 16-bit PCM WAV 字节。
pub fn wav_bytes(audio: &muser_engine::AudioBuffer<f32>, sample_rate: u32) -> Vec<u8> {
    let frames = audio.num_frames();
    let data_len = (frames * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // 声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let (l, r) = (audio.channel(0), audio.channel(1));
    for f in 0..frames {
        for s in [l[f], r[f]] {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// 标准 base64（带填充）。
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

fn node_id(n: Node) -> String {
    match n {
        Node::Track(i) => format!("tracks[{i}]"),
        Node::Bus(i) => format!("buses[{i}]"),
    }
}

/// 页面需要的工程摘要。`urls` 给出成品与各分轨的音频地址。
fn payload(project: &Project, rendered: &Rendered, settings: RenderSettings, mix_url: String, stem_urls: Vec<String>) -> Value {
    let stem_of = |id: &str| rendered.stems.iter().position(|s| node_id(s.node) == id);
    let fx = |list: &[muser_core::Effect]| list.iter().map(|e| e.type_name()).collect::<Vec<_>>();
    let mut nodes = Vec::new();
    for (i, t) in project.tracks.iter().enumerate() {
        let id = format!("tracks[{i}]");
        let clips: Vec<Value> = t
            .clips
            .iter()
            .map(|c| {
                let notes: Vec<Value> = c.notes.iter().map(|n| json!([n.start.0, n.length.0, n.pitch, n.velocity])).collect();
                json!({ "start": c.start.0, "length": c.length.0, "notes": notes })
            })
            .collect();
        nodes.push(json!({
            "id": id, "kind": "track",
            "name": if t.name.is_empty() { id.clone() } else { t.name.clone() },
            "instrument": t.instrument, "output": t.output.as_deref().unwrap_or("master"), "sends": t.sends,
            "gain_db": t.gain_db, "muted": t.muted, "effects": fx(&t.effects), "stem": stem_of(&id), "clips": clips,
        }));
    }
    for (i, b) in project.buses.iter().enumerate() {
        let id = format!("buses[{i}]");
        nodes.push(json!({
            "id": id, "kind": "bus", "name": b.name, "output": b.output.as_deref().unwrap_or("master"), "sends": b.sends,
            "gain_db": b.gain_db, "muted": b.muted, "effects": fx(&b.effects), "stem": stem_of(&id), "clips": [],
        }));
    }
    let stems: Vec<Value> = rendered
        .stems
        .iter()
        .zip(stem_urls)
        .map(|(s, url)| json!({ "node": node_id(s.node), "label": s.label, "url": url }))
        .collect();
    json!({
        "name": project.name,
        "tempo": project.tempo,
        "beats_per_bar": project.time_signature.numerator,
        "time_signature": format!("{}/{}", project.time_signature.numerator, project.time_signature.denominator),
        "duration_beats": project.duration_beats().0,
        "seconds": rendered.mix.num_frames() as f64 / f64::from(settings.sample_rate),
        "master_gain_db": project.master_gain_db,
        "master_effects": fx(&project.master_effects),
        "nodes": nodes, "stems": stems, "mix": mix_url,
        "version": 0, "error": Value::Null, "dirty": false, "project_path": Value::Null,
    })
}

fn page(data: Option<&Value>) -> String {
    let embedded = match data {
        // `</` 转义，防止音轨名等内容提前结束 <script>。
        Some(v) => v.to_string().replace("</", "<\\/"),
        None => "null".to_string(),
    };
    TEMPLATE.replacen(PLACEHOLDER, &embedded, 1)
}

/// 静态模式：生成单文件 HTML。返回 (文件字节数, 分轨数)。
pub fn write_static(project: &Project, settings: RenderSettings, out: &str) -> Result<(usize, usize)> {
    let r = render_with_stems(project, settings).context("渲染失败")?;
    let url = |a: &muser_engine::AudioBuffer<f32>| format!("data:audio/wav;base64,{}", base64(&wav_bytes(a, settings.sample_rate)));
    let stem_urls = r.stems.iter().map(|s| url(&s.audio)).collect();
    let data = payload(project, &r, settings, url(&r.mix), stem_urls);
    let html = page(Some(&data));
    std::fs::write(out, &html).with_context(|| format!("写出失败: {out}"))?;
    Ok((html.len(), r.stems.len()))
}

// ---------------------------------------------------------------- 联动模式

/// 页面提交的一项改动。
#[derive(Debug, Clone, serde::Deserialize)]
struct Change {
    node: String,
    #[serde(default)]
    gain_db: Option<f64>,
    #[serde(default)]
    muted: Option<bool>,
}

#[derive(serde::Deserialize)]
struct ApplyBody {
    changes: Vec<Change>,
    #[serde(default)]
    save: bool,
}

/// 解析 `tracks[i]` / `buses[i]`。
fn parse_node(id: &str) -> Option<(&str, usize)> {
    let (kind, rest) = id.split_once('[')?;
    let i = rest.strip_suffix(']')?.parse().ok()?;
    matches!(kind, "tracks" | "buses").then_some((kind, i))
}

fn apply_to_project(p: &mut Project, changes: &[Change]) -> Result<(), String> {
    for c in changes {
        let (kind, i) = parse_node(&c.node).ok_or_else(|| format!("无效的 node '{}'", c.node))?;
        let (gain, muted) = match kind {
            "tracks" => p.tracks.get_mut(i).map(|t| (&mut t.gain_db, &mut t.muted)),
            _ => p.buses.get_mut(i).map(|b| (&mut b.gain_db, &mut b.muted)),
        }
        .ok_or_else(|| format!("{} 不存在", c.node))?;
        if let Some(g) = c.gain_db {
            if !(-80.0..=24.0).contains(&g) {
                return Err(format!("{}.gain_db = {g} 超出范围 [-80, 24]", c.node));
            }
            *gain = g;
        }
        if let Some(m) = c.muted {
            *muted = m;
        }
    }
    Ok(())
}

/// 把改动写进原始 JSON（只动改了的字段，保留文件里其它写法）。
fn apply_to_json(v: &mut Value, changes: &[Change]) -> Result<()> {
    for c in changes {
        let (kind, i) = parse_node(&c.node).ok_or_else(|| anyhow!("无效的 node '{}'", c.node))?;
        let obj = v
            .get_mut(kind)
            .and_then(|a| a.get_mut(i))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("工程文件里没有 {}", c.node))?;
        if let Some(g) = c.gain_db {
            obj.insert("gain_db".into(), json!(g));
        }
        if let Some(m) = c.muted {
            obj.insert("muted".into(), json!(m));
        }
    }
    Ok(())
}

struct Inner {
    version: u64,
    project: Option<Project>,
    payload: Value,
    audio: HashMap<String, Arc<Vec<u8>>>,
    error: Option<String>,
    rendering: bool,
    /// 已应用到内存、尚未写回文件的改动。
    pending: Vec<Change>,
    file_mtime: Option<SystemTime>,
}

struct Shared {
    path: PathBuf,
    settings: RenderSettings,
    inner: Mutex<Inner>,
    render_lock: Mutex<()>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Shared {
    /// 渲染 `project` 并换上新版本。失败时保留上一版音频，记录错误。
    fn rerender(&self, project: Project) -> Result<(), String> {
        let _g = self.render_lock.lock().unwrap();
        self.inner.lock().unwrap().rendering = true;
        let t0 = std::time::Instant::now();
        let result = render_with_stems(&project, self.settings);
        let mut inner = self.inner.lock().unwrap();
        inner.rendering = false;
        match result {
            Ok(r) => {
                let v = inner.version + 1;
                let mut audio = HashMap::new();
                audio.insert(format!("/audio/{v}/mix.wav"), Arc::new(wav_bytes(&r.mix, self.settings.sample_rate)));
                let mut urls = Vec::new();
                for (i, s) in r.stems.iter().enumerate() {
                    let u = format!("/audio/{v}/stem-{i}.wav");
                    audio.insert(u.clone(), Arc::new(wav_bytes(&s.audio, self.settings.sample_rate)));
                    urls.push(u);
                }
                inner.payload = payload(&project, &r, self.settings, format!("/audio/{v}/mix.wav"), urls);
                inner.audio = audio;
                inner.version = v;
                inner.project = Some(project);
                inner.error = None;
                eprintln!("[view] 版本 {v}：渲染完成（{:.1}s）", t0.elapsed().as_secs_f64());
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e:#}");
                eprintln!("[view] 渲染失败：{msg}");
                inner.error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// 从磁盘读工程并渲染（启动时、文件变化时）。
    fn reload(&self) {
        let m = mtime(&self.path);
        let loaded = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("读取失败: {e}"))
            .and_then(|t| Project::from_json(&t).map_err(|e| format!("解析失败: {e}")));
        {
            let mut inner = self.inner.lock().unwrap();
            inner.file_mtime = m;
            if !inner.pending.is_empty() {
                eprintln!("[view] 工程文件已变化，丢弃未保存的页面改动");
                inner.pending.clear();
            }
        }
        match loaded {
            Ok(p) => {
                let _ = self.rerender(p);
            }
            Err(e) => {
                eprintln!("[view] {e}");
                self.inner.lock().unwrap().error = Some(e);
            }
        }
    }

    fn apply(&self, body: ApplyBody) -> Result<u64, String> {
        let mut project = self.inner.lock().unwrap().project.clone().ok_or("工程尚未成功加载")?;
        if !body.changes.is_empty() {
            apply_to_project(&mut project, &body.changes)?;
            self.rerender(project)?;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.pending.extend(body.changes);
        if body.save {
            let pending = std::mem::take(&mut inner.pending);
            let write = || -> Result<()> {
                let text = std::fs::read_to_string(&self.path)?;
                let mut v: Value = serde_json::from_str(&text)?;
                apply_to_json(&mut v, &pending)?;
                std::fs::write(&self.path, serde_json::to_string_pretty(&v)? + "\n")?;
                Ok(())
            };
            if let Err(e) = write() {
                inner.pending = pending;
                return Err(format!("保存失败: {e:#}"));
            }
            // 自己写的文件不触发重新加载。
            inner.file_mtime = mtime(&self.path);
            eprintln!("[view] 已保存到 {}", self.path.display());
        }
        Ok(inner.version)
    }

    fn state(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let mut p = inner.payload.clone();
        p["version"] = json!(inner.version);
        p["error"] = json!(inner.error);
        p["dirty"] = json!(!inner.pending.is_empty());
        p["rendering"] = json!(inner.rendering);
        p["project_path"] = json!(self.path.display().to_string());
        p
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

fn handle(shared: &Shared, mut req: tiny_http::Request, port: u16) {
    use tiny_http::Method;
    if !host_allowed(&req, port) {
        respond_json(req, 403, &json!({ "error": "只接受来自本机页面的请求" }));
        return;
    }
    let url = req.url().split('?').next().unwrap_or("").to_string();
    match (req.method(), url.as_str()) {
        (Method::Get, "/") => {
            let r = tiny_http::Response::from_string(page(None))
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store"));
            let _ = req.respond(r);
        }
        (Method::Get, "/favicon.ico") => {
            let _ = req.respond(tiny_http::Response::empty(204));
        }
        (Method::Get, "/api/state") => respond_json(req, 200, &shared.state()),
        (Method::Get, "/api/version") => {
            let inner = shared.inner.lock().unwrap();
            let v = json!({ "version": inner.version, "rendering": inner.rendering, "error": inner.error });
            drop(inner);
            respond_json(req, 200, &v);
        }
        (Method::Post, "/api/apply") => {
            let mut body = String::new();
            if std::io::Read::read_to_string(&mut req.as_reader(), &mut body).is_err() {
                respond_json(req, 400, &json!({ "error": "读取请求失败" }));
                return;
            }
            let parsed: ApplyBody = match serde_json::from_str(&body) {
                Ok(b) => b,
                Err(e) => return respond_json(req, 400, &json!({ "error": format!("请求格式错误: {e}") })),
            };
            match shared.apply(parsed) {
                Ok(v) => respond_json(req, 200, &json!({ "ok": true, "version": v })),
                Err(e) => respond_json(req, 400, &json!({ "error": e })),
            }
        }
        (Method::Get, u) if u.starts_with("/audio/") => {
            let data = shared.inner.lock().unwrap().audio.get(u).cloned();
            match data {
                Some(bytes) => {
                    let r = tiny_http::Response::from_data(bytes.as_slice().to_vec())
                        .with_header(header("Content-Type", "audio/wav"))
                        .with_header(header("Cache-Control", "no-store"));
                    let _ = req.respond(r);
                }
                None => respond_json(req, 404, &json!({ "error": "音频已过期" })),
            }
        }
        _ => respond_json(req, 404, &json!({ "error": "not found" })),
    }
}

/// 联动模式：阻塞运行直到进程被中断。
pub fn serve(path: &str, settings: RenderSettings, port: u16) -> Result<()> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        bail!("找不到工程文件: {}", path.display());
    }
    let shared = Arc::new(Shared {
        path,
        settings,
        inner: Mutex::new(Inner {
            version: 0,
            project: None,
            payload: Value::Null,
            audio: HashMap::new(),
            error: None,
            rendering: false,
            pending: Vec::new(),
            file_mtime: None,
        }),
        render_lock: Mutex::new(()),
    });
    // 只绑定本机回环地址。
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("无法监听 127.0.0.1:{port}: {e}"))?;
    eprintln!("[view] 渲染 {} …", shared.path.display());
    shared.reload();
    println!("muser view 已启动：http://127.0.0.1:{port}/  （只监听本机，无鉴权；Ctrl+C 退出）");
    println!("工程文件变化会自动重新渲染；页面上的改动可「应用」或「保存到工程文件」。");

    let watcher = Arc::clone(&shared);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = mtime(&watcher.path);
            let changed = now.is_some() && now != watcher.inner.lock().unwrap().file_mtime;
            if changed {
                eprintln!("[view] 工程文件已变化，重新渲染");
                watcher.reload();
            }
        }
    });
    for req in server.incoming_requests() {
        let s = Arc::clone(&shared);
        std::thread::spawn(move || handle(&s, req, port));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn apply_changes_to_project_and_json() {
        let text = r#"{ "tempo": 120, "buses": [{ "name": "v" }], "tracks": [{ "name": "a", "instrument": "synth.poly", "clips": [] }] }"#;
        let mut p = Project::from_json(text).unwrap();
        let ch = vec![
            Change { node: "tracks[0]".into(), gain_db: Some(-4.5), muted: None },
            Change { node: "buses[0]".into(), gain_db: None, muted: Some(true) },
        ];
        apply_to_project(&mut p, &ch).unwrap();
        assert_eq!((p.tracks[0].gain_db, p.buses[0].muted), (-4.5, true));
        let mut v: Value = serde_json::from_str(text).unwrap();
        apply_to_json(&mut v, &ch).unwrap();
        assert_eq!(v["tracks"][0]["gain_db"], json!(-4.5));
        assert_eq!(v["buses"][0]["muted"], json!(true));
        assert!(v["tracks"][0].get("muted").is_none(), "没改的字段不写入");
        let bad = vec![Change { node: "tracks[9]".into(), gain_db: Some(0.0), muted: None }];
        assert!(apply_to_project(&mut p, &bad).is_err());
        let bad = vec![Change { node: "tracks[0]".into(), gain_db: Some(99.0), muted: None }];
        assert!(apply_to_project(&mut p, &bad).unwrap_err().contains("超出范围"));
    }

    #[test]
    fn page_escapes_script_end() {
        let html = page(Some(&json!({ "name": "</script><b>x" })));
        assert!(!html.contains("</script><b>"));
    }
}
