//! `muser view` / `muser poly edit` 共用的本地网页服务小工具。
//!
//! 服务只绑定 127.0.0.1；[`host_allowed`] 再拒绝 Host / Origin 不是本机的请求（防 DNS rebinding）。

use std::collections::BTreeMap;

use serde_json::Value;

pub(crate) fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("合法的 header")
}

pub(crate) fn respond_json(req: tiny_http::Request, status: u16, v: &Value) {
    let r = tiny_http::Response::from_string(v.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    let _ = req.respond(r);
}

/// 只接受发往本机地址的请求（防 DNS rebinding：别的网站把域名解析到 127.0.0.1 来调用本服务）。
pub(crate) fn host_allowed(req: &tiny_http::Request, port: u16) -> bool {
    let ok = |v: &str| v == format!("127.0.0.1:{port}") || v == format!("localhost:{port}");
    let get = |name: &str| req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string());
    let host_ok = get("Host").is_some_and(|h| ok(&h));
    let origin_ok = get("Origin").is_none_or(|o| o.strip_prefix("http://").is_some_and(ok));
    host_ok && origin_ok
}

pub(crate) fn query(url: &str) -> BTreeMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
        .unwrap_or_default()
}

pub(crate) fn read_body<T: serde::de::DeserializeOwned>(req: &mut tiny_http::Request) -> Result<T, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(&mut req.as_reader(), &mut body).map_err(|_| "读取请求失败".to_string())?;
    if body.trim().is_empty() {
        body = "{}".into();
    }
    serde_json::from_str(&body).map_err(|e| format!("请求格式错误: {e}"))
}

/// 用系统浏览器打开 `url`；打不开时提示手动打开。
pub(crate) fn open_browser(url: &str) {
    let (opener, mut cmd) = if cfg!(target_os = "macos") {
        ("open", std::process::Command::new("open"))
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        ("start", c)
    } else {
        ("xdg-open", std::process::Command::new("xdg-open"))
    };
    if let Err(e) = cmd.arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        eprintln!("[muser] 无法打开浏览器（{opener}: {e}），请手动打开 {url}");
    }
}
