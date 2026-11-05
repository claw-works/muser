//! 本机 `muser live` 的客户端：命令行（`muser live send|status|…`）和 muser-mcp 的 live 工具都用它。
//! 不加 HTTP 依赖：直接在 127.0.0.1 上写 HTTP/1.1。

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

/// 正在跑的 `muser live` 写下端口的地方（只在本机用）。
pub fn port_file() -> PathBuf {
    std::env::temp_dir().join("muser-live.json")
}

/// 正在跑的 `muser live` 的端口（`port` 给了就用它）。
pub fn find_port(port: Option<u16>) -> Result<u16, String> {
    if let Some(p) = port {
        return Ok(p);
    }
    let text = std::fs::read_to_string(port_file()).map_err(|_| "没有正在进行的现场演奏：先 muser live start <模板>".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    v["port"].as_u64().map(|p| p as u16).ok_or_else(|| format!("{} 写坏了", port_file().display()))
}

/// 发一个请求，返回 JSON；服务报错时返回它的错误信息。`wait` 最多 30 秒，读超时留足余量。
pub fn request(port: Option<u16>, method: &str, path: &str, body: Option<&str>) -> Result<Value, String> {
    use std::io::{Read, Write};
    let port = find_port(port)?;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|_| format!("连不上 muser live（127.0.0.1:{port}）：它还在跑吗？"))?;
    s.set_read_timeout(Some(Duration::from_secs(45))).map_err(|e| e.to_string())?;
    let body = body.unwrap_or("");
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let (head, rest) = out.split_once("\r\n\r\n").ok_or("muser live 的回应不完整")?;
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    // 分块传输时，JSON 在块长度行之后：取第一个 { 到最后一个 }。
    let text = rest.find('{').map_or(rest, |i| &rest[i..]);
    let text = text.rfind('}').map_or(text, |j| &text[..=j]);
    let v: Value = serde_json::from_str(text).map_err(|_| format!("muser live 的回应不是 JSON：{rest}"))?;
    if code >= 400 {
        return Err(v["error"].as_str().unwrap_or("muser live 报错").to_string());
    }
    Ok(v)
}
