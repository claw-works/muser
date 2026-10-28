//! `muser loop edit`：loop / 动机编辑器（本地网页）。旋律片段用卷帘，鼓用步进网格；用 muser 的乐器渲染试听；
//! 存到项目（工程旁的 `loops/`）或全局（`~/.config/muser/loops`）。agent 改了 loop 文件，页面会提示并重新读入。
//! 只监听 127.0.0.1，与 `muser poly edit` 相同。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use muser_engine::{loop_library, RenderSettings};
use serde_json::{json, Value};

use crate::poly::{pcm16, percent_decode};
use crate::web::{header, host_allowed, query, read_body, respond_json};

const PAGE: &str = include_str!("loop_edit.html");

struct Ctx {
    base: Option<PathBuf>,
    initial: Option<String>,
    port: u16,
    sample_rate: u32,
}

/// loop 文件的修改时间（秒，内置的没有）。
fn mtime(path: &std::path::Path) -> Option<f64> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64())
}

#[derive(serde::Deserialize)]
struct PreviewBody {
    #[serde(rename = "loop")]
    item: loop_library::Loop,
    #[serde(default)]
    instrument: Option<String>,
    #[serde(default)]
    tempo: Option<f64>,
    #[serde(default = "one")]
    repeat: u32,
}
fn one() -> u32 {
    1
}

#[derive(serde::Deserialize)]
struct SaveBody {
    #[serde(rename = "loop")]
    item: loop_library::Loop,
    /// "project" / "global"。
    #[serde(default)]
    scope: String,
    #[serde(default)]
    overwrite: bool,
}

/// 渲染试听：交错的 16-bit PCM。`repeat` 遍，不留余响（循环播放时首尾接得上）。
fn preview(b: &PreviewBody, sample_rate: u32) -> Result<Vec<u8>, String> {
    b.item.check()?;
    let p = b.item.project(b.instrument.as_deref().filter(|s| !s.is_empty()), b.tempo, b.repeat.clamp(1, 16))?;
    let audio = muser_engine::render(&p, RenderSettings { sample_rate, tail_seconds: 0.0, ..RenderSettings::default() }).map_err(|e| format!("{e}"))?;
    Ok(pcm16(&audio))
}

fn handle(mut req: tiny_http::Request, cx: &Ctx) {
    use tiny_http::Method;
    if !host_allowed(&req, cx.port) {
        respond_json(req, 403, &json!({ "error": "只接受来自本机页面的请求" }));
        return;
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let base = cx.base.as_deref();
    let err = |req: tiny_http::Request, e: String| respond_json(req, 400, &json!({ "error": e }));
    let name = || query(&url).get("name").map(|n| percent_decode(n)).unwrap_or_default();
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
        (Method::Get, "/api/list") => {
            let loops: Vec<Value> = loop_library::list_for(base)
                .iter()
                .map(|e| json!({ "name": e.item.name, "source": e.source, "drums": e.item.is_drums(), "bars": e.item.bars, "style": e.item.style, "description": e.item.description }))
                .collect();
            respond_json(req, 200, &json!({
                "project_dir": loop_library::project_dir(base).display().to_string(),
                "global_dir": loop_library::dir().display().to_string(),
                "initial": cx.initial, "sample_rate": cx.sample_rate, "loops": loops,
            }));
        }
        (Method::Get, "/api/loop") => {
            let n = name();
            match loop_library::load_for(&n, base) {
                Ok(l) => {
                    let (path, source) = loop_library::locate(&n, base).map_or((None, "builtin"), |(p, s)| (Some(p), s));
                    respond_json(req, 200, &json!({ "loop": l, "source": source, "path": path.as_ref().map(|p| p.display().to_string()), "mtime": path.as_deref().and_then(mtime) }));
                }
                Err(e) => respond_json(req, 404, &json!({ "error": e })),
            }
        }
        (Method::Get, "/api/mtime") => {
            let n = name();
            let m = loop_library::locate(&n, base).and_then(|(p, _)| mtime(&p));
            respond_json(req, 200, &json!({ "mtime": m }));
        }
        (Method::Get, "/api/instruments") => {
            let list: Vec<Value> = muser_engine::instrument::catalog().into_iter().map(|i| json!({ "name": i.name, "family": i.family })).collect();
            respond_json(req, 200, &json!(list));
        }
        (Method::Post, "/api/preview") => match read_body::<PreviewBody>(&mut req).and_then(|b| preview(&b, cx.sample_rate)) {
            Ok(pcm) => {
                let r = tiny_http::Response::from_data(pcm)
                    .with_header(header("Content-Type", "application/octet-stream"))
                    .with_header(header("X-Sample-Rate", &cx.sample_rate.to_string()))
                    .with_header(header("Cache-Control", "no-store"));
                let _ = req.respond(r);
            }
            Err(e) => err(req, e),
        },
        (Method::Post, "/api/save") => match read_body::<SaveBody>(&mut req) {
            Ok(mut b) => {
                b.item.format = loop_library::FORMAT.into();
                b.item.version = loop_library::VERSION;
                let r = match b.scope.as_str() {
                    "global" => loop_library::save(&b.item, b.overwrite),
                    _ => loop_library::save_project(&b.item, base, b.overwrite),
                };
                match r {
                    Ok(path) => respond_json(req, 200, &json!({ "ok": true, "path": path.display().to_string(), "mtime": mtime(&path) })),
                    Err(e) if e.contains("已存在") => respond_json(req, 409, &json!({ "error": e })),
                    Err(e) => err(req, e),
                }
            }
            Err(e) => err(req, e),
        },
        (Method::Post, "/api/share") => match read_body::<Value>(&mut req) {
            Ok(b) => match loop_library::share(b["name"].as_str().unwrap_or(""), base, b["overwrite"].as_bool().unwrap_or(false)) {
                Ok(path) => respond_json(req, 200, &json!({ "ok": true, "path": path.display().to_string() })),
                Err(e) if e.contains("已存在") => respond_json(req, 409, &json!({ "error": e })),
                Err(e) => err(req, e),
            },
            Err(e) => err(req, e),
        },
        _ => respond_json(req, 404, &json!({ "error": "not found" })),
    }
}

pub fn serve(initial: Option<String>, base: Option<PathBuf>, port: u16, open: bool, sample_rate: u32) -> Result<()> {
    // 只绑定本机回环地址。
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("无法监听 127.0.0.1:{port}: {e}"))?;
    let url = format!("http://127.0.0.1:{port}/");
    println!("muser loop 编辑器已启动：{url}  （只监听本机，无鉴权；Ctrl+C 退出）");
    println!("项目 loop：{}   全局 loop：{}", loop_library::project_dir(base.as_deref()).display(), loop_library::dir().display());
    if open {
        crate::web::open_browser(&url);
    }
    let cx = Arc::new(Ctx { base, initial, port, sample_rate });
    for req in server.incoming_requests() {
        let cx = Arc::clone(&cx);
        std::thread::spawn(move || handle(req, &cx));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_melody_and_drums() {
        let melody: loop_library::Loop = serde_json::from_value(json!({ "format": "muser.loop", "version": 2, "name": "m", "bars": 1,
            "notes": [{ "pitch": 60, "start": 0, "length": 1 }, { "pitch": 64, "start": 1, "length": 1 }] })).unwrap();
        let pcm = preview(&PreviewBody { item: melody, instrument: Some("wind.flute".into()), tempo: Some(120.0), repeat: 2 }, 48_000).unwrap();
        // 2 遍 × 4 拍 @ 120 = 4 秒（总帧数含 1 帧余量）。
        assert!((pcm.len() / 4).abs_diff(4 * 48_000) <= 2, "{}", pcm.len() / 4);
        let drums = loop_library::load("rock_basic").unwrap();
        assert!(!preview(&PreviewBody { item: drums, instrument: None, tempo: None, repeat: 1 }, 48_000).unwrap().is_empty());
    }
}
