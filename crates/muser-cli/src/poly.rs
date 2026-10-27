//! `muser poly`：poly 合成器的音色。
//!
//! - `edit`：本地网页音色编辑器（只监听 127.0.0.1）。从预设或本机音色库开始，改参数后自动渲染一段试听乐句，
//!   存进本机音色库（`~/.config/muser/poly/`）。
//! - `list` / `show` / `schema`：给 agent 用。`show <名字>` 输出写进工程音轨的完整 JSON。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use muser_engine::{poly_library, poly_patch, AudioBuffer, RenderSettings};
use serde_json::{json, Value};

use crate::web::{header, host_allowed, query, read_body, respond_json};

const PAGE: &str = include_str!("poly.html");

#[derive(Subcommand)]
pub enum PolyCommand {
    /// 打开音色编辑器（浏览器页面，只监听 127.0.0.1）。
    Edit {
        /// 端口。
        #[arg(long, default_value_t = 8739)]
        port: u16,
        /// 启动后用系统浏览器打开页面。
        #[arg(long)]
        open: bool,
        /// 试听的采样率。
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
    },
    /// 列出本机音色库里的音色（内置预设见 `muser instruments --family poly`）。
    List {
        #[arg(long)]
        json: bool,
    },
    /// 取出一个音色（音色库里的名字、poly.<预设> 或 synth.poly），输出写进工程音轨的 JSON（完整音色内嵌）。
    Show {
        name: String,
        /// 改为输出音色文件本身（含名字、说明、标签）。
        #[arg(long)]
        sound: bool,
    },
    /// 完整音色的结构：每个分区的字段、类型、范围、选项与说明。
    Schema,
}

pub fn run(command: PolyCommand) -> Result<()> {
    let fail = |e: String| anyhow!(e);
    match command {
        PolyCommand::Edit { port, open, sample_rate } => serve(port, open, sample_rate),
        PolyCommand::List { json } => {
            let sounds = poly_library::list();
            if json {
                let list: Vec<Value> = sounds.iter().map(|s| json!({ "name": s.name, "description": s.description, "tags": s.tags, "base": s.base })).collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            if sounds.is_empty() {
                println!("音色库是空的（{}）。用 `muser poly edit --open` 做一个。", poly_library::dir().display());
            }
            for s in &sounds {
                let tags = if s.tags.is_empty() { String::new() } else { format!("  [{}]", s.tags.join(", ")) };
                println!("{:<24} {}{tags}", s.name, s.description);
            }
            if !sounds.is_empty() {
                println!("\n共 {} 个（{}）。`muser poly show <名字>` 取出写进工程的 JSON。", sounds.len(), poly_library::dir().display());
            }
            Ok(())
        }
        PolyCommand::Show { name, sound } => {
            let s = poly_library::resolve(&name).map_err(fail)?;
            let out = if sound { serde_json::to_value(&s)? } else { s.track_snippet() };
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        PolyCommand::Schema => {
            println!("{}", serde_json::to_string_pretty(&poly_patch::schema())?);
            Ok(())
        }
    }
}

/// 试听乐句：(拍起点, 拍长, 相对根音的半音)。速度 120。
fn phrase(name: &str) -> Result<(Vec<(f64, f64, i32)>, bool), String> {
    let seq = |steps: &[i32], step: f64, len: f64| steps.iter().enumerate().map(|(i, &n)| (i as f64 * step, len, n)).collect::<Vec<_>>();
    Ok(match name {
        "note" => (vec![(0.0, 2.0, 0)], false),
        "chord" => ([0, 4, 7, 11].iter().map(|&n| (0.0, 2.0, n)).collect(), false),
        "bass" => (seq(&[-12, -12, 0, -12, -5, -12, -2, 0], 0.5, 0.45), false),
        "arp" => (seq(&[0, 4, 7, 12, 16, 12, 7, 4, 0, 4, 7, 12, 16, 12, 7, 4], 0.25, 0.22), false),
        "lead" => (seq(&[0, 2, 4, 7, 9, 7, 4, 2], 0.5, 0.5), true),
        _ => return Err(format!("未知试听乐句 '{name}'，可选: note, chord, bass, arp, lead")),
    })
}

/// 渲染试听：返回 (交错的 16-bit PCM, 峰值 dBFS)。
fn preview(patch: &Value, phrase_name: &str, root: i64, sample_rate: u32) -> Result<(Vec<u8>, f64), String> {
    let (notes, legato) = phrase(phrase_name)?;
    let notes: Vec<Value> = notes
        .iter()
        .filter_map(|&(start, length, n)| {
            let pitch = root + i64::from(n);
            (0..=127).contains(&pitch).then(|| json!({ "pitch": pitch, "start": start, "length": length }))
        })
        .collect();
    let length = notes.iter().map(|n| n["start"].as_f64().unwrap_or(0.0) + n["length"].as_f64().unwrap_or(0.0)).fold(1.0, f64::max);
    let project = json!({ "name": "poly-preview", "tempo": 120, "tracks": [{ "name": "preview", "instrument": "synth.poly",
        "instrument_params": { "patch": patch }, "clips": [{ "start": 0, "length": length, "legato": legato, "notes": notes }] }] });
    let project: muser_core::Project = serde_json::from_value(project).map_err(|e| e.to_string())?;
    let audio = muser_engine::render(&project, RenderSettings { sample_rate, tail_seconds: 1.5, ..RenderSettings::default() }).map_err(|e| format!("{e}"))?;
    Ok((pcm16(&audio), 20.0 * f64::from(audio.peak().max(1e-9)).log10()))
}

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

#[derive(serde::Deserialize)]
struct PreviewBody {
    patch: Value,
    #[serde(default = "default_phrase")]
    phrase: String,
    #[serde(default = "default_root")]
    root: i64,
}
fn default_phrase() -> String {
    "chord".into()
}
fn default_root() -> i64 {
    60
}

#[derive(serde::Deserialize)]
struct SaveBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    base: Option<String>,
    patch: Value,
    #[serde(default)]
    overwrite: bool,
}

fn handle(mut req: tiny_http::Request, port: u16, sample_rate: u32) {
    use tiny_http::Method;
    if !host_allowed(&req, port) {
        respond_json(req, 403, &json!({ "error": "只接受来自本机页面的请求" }));
        return;
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let err = |req: tiny_http::Request, e: String| respond_json(req, 400, &json!({ "error": e }));
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
        (Method::Get, "/api/schema") => respond_json(req, 200, &poly_patch::schema()),
        (Method::Get, "/api/presets") => {
            let list: Vec<Value> = muser_engine::instrument::catalog()
                .into_iter()
                .filter(|i| i.family.starts_with("poly/") || i.name == "synth.poly")
                .map(|i| json!({ "name": i.name, "family": i.family, "description": i.description }))
                .collect();
            respond_json(req, 200, &json!(list));
        }
        (Method::Get, "/api/library") => {
            let list: Vec<Value> = poly_library::list().iter().map(|s| json!({ "name": s.name, "description": s.description, "tags": s.tags, "base": s.base })).collect();
            respond_json(req, 200, &json!({ "dir": poly_library::dir().display().to_string(), "sounds": list }));
        }
        (Method::Get, "/api/sound") => match query(&url).get("name").map(|n| percent_decode(n)) {
            Some(name) => match poly_library::resolve(&name) {
                Ok(s) => respond_json(req, 200, &json!(s)),
                Err(e) => respond_json(req, 404, &json!({ "error": e })),
            },
            None => err(req, "需要 name".into()),
        },
        (Method::Post, "/api/preview") => match read_body::<PreviewBody>(&mut req) {
            Ok(b) => {
                let t0 = std::time::Instant::now();
                match preview(&b.patch, &b.phrase, b.root, sample_rate) {
                    Ok((pcm, peak)) => {
                        let r = tiny_http::Response::from_data(pcm)
                            .with_header(header("Content-Type", "application/octet-stream"))
                            .with_header(header("X-Sample-Rate", &sample_rate.to_string()))
                            .with_header(header("X-Peak-Db", &format!("{peak:.1}")))
                            .with_header(header("X-Render-Ms", &t0.elapsed().as_millis().to_string()))
                            .with_header(header("Cache-Control", "no-store"));
                        let _ = req.respond(r);
                    }
                    Err(e) => err(req, e),
                }
            }
            Err(e) => err(req, e),
        },
        (Method::Post, "/api/save") => match read_body::<SaveBody>(&mut req) {
            Ok(b) => {
                let sound = poly_library::Sound {
                    format: poly_library::FORMAT.into(),
                    version: poly_library::VERSION,
                    name: b.name.trim().to_string(),
                    description: b.description.trim().to_string(),
                    tags: b.tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
                    author: String::new(),
                    base: b.base,
                    patch: b.patch,
                };
                match poly_library::save(sound, b.overwrite) {
                    Ok(path) => respond_json(req, 200, &json!({ "ok": true, "path": path.display().to_string() })),
                    Err(e) if e.contains("已存在") => respond_json(req, 409, &json!({ "error": e })),
                    Err(e) => err(req, e),
                }
            }
            Err(e) => err(req, e),
        },
        (Method::Post, "/api/normalize") => match read_body::<Value>(&mut req) {
            // 把 overlay（可以只写部分字段）套到当前音色上，返回完整音色。
            Ok(b) => {
                let mut p = poly_patch::Params::default();
                let result = poly_patch::apply(&mut p, &b["patch"]).and_then(|_| poly_patch::apply(&mut p, &b["overlay"]));
                match result {
                    Ok(()) => respond_json(req, 200, &json!({ "patch": poly_patch::to_json(&p) })),
                    Err(e) => err(req, e),
                }
            }
            Err(e) => err(req, e),
        },
        (Method::Post, "/api/delete") => match read_body::<Value>(&mut req) {
            Ok(b) => match poly_library::delete(b["name"].as_str().unwrap_or("")) {
                Ok(()) => respond_json(req, 200, &json!({ "ok": true })),
                Err(e) => err(req, e),
            },
            Err(e) => err(req, e),
        },
        _ => respond_json(req, 404, &json!({ "error": "not found" })),
    }
}

/// 查询串里的 `%XX` 与 `+`。
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match (hex, b[i]) {
            (Some(v), _) => {
                out.push(v);
                i += 3;
                continue;
            }
            (None, b'+') => out.push(b' '),
            (None, c) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn serve(port: u16, open: bool, sample_rate: u32) -> Result<()> {
    // 只绑定本机回环地址。
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("无法监听 127.0.0.1:{port}: {e}"))?;
    let url = format!("http://127.0.0.1:{port}/");
    println!("muser poly 音色编辑器已启动：{url}  （只监听本机，无鉴权；Ctrl+C 退出）");
    println!("音色库：{}", poly_library::dir().display());
    if open {
        crate::web::open_browser(&url);
    }
    let server = Arc::new(server);
    for req in server.incoming_requests() {
        std::thread::spawn(move || handle(req, port, sample_rate));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_renders_every_phrase() {
        let patch = poly_library::resolve("poly.reese").unwrap().patch;
        for p in ["note", "chord", "bass", "arp", "lead"] {
            let (pcm, peak) = preview(&patch, p, 48, 48_000).unwrap();
            assert!(pcm.len() > 48_000 * 4, "{p}");
            assert!(peak > -60.0, "{p}: {peak}");
        }
        assert!(preview(&patch, "nope", 60, 48_000).unwrap_err().contains("可选"));
        assert!(preview(&json!({ "glide": 99 }), "note", 60, 48_000).unwrap_err().contains("patch.glide"));
    }

    #[test]
    fn decodes_query() {
        assert_eq!(percent_decode("poly.reese"), "poly.reese");
        assert_eq!(percent_decode("a%2Eb+c"), "a.b c");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
        assert_eq!(percent_decode("end%2"), "end%2");
    }
}
