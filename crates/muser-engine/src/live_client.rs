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
    let text = std::fs::read_to_string(port_file()).map_err(|_| "no live session in progress: run muser live start <template> first".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    v["port"].as_u64().map(|p| p as u16).ok_or_else(|| format!("{} is corrupted", port_file().display()))
}

/// 发一个请求，返回 JSON；服务报错时返回它的错误信息。`wait` 最多 30 秒，读超时留足余量。
pub fn request(port: Option<u16>, method: &str, path: &str, body: Option<&str>) -> Result<Value, String> {
    use std::io::{Read, Write};
    let port = find_port(port)?;
    let mut s =
        std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|_| format!("cannot connect to muser live (127.0.0.1:{port}): is it still running?"))?;
    s.set_read_timeout(Some(Duration::from_secs(45))).map_err(|e| e.to_string())?;
    let body = body.unwrap_or("");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let (head, rest) = out.split_once("\r\n\r\n").ok_or("incomplete response from muser live")?;
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    // 分块传输时，JSON 在块长度行之后：取第一个 { 到最后一个 }。
    let text = rest.find('{').map_or(rest, |i| &rest[i..]);
    let text = text.rfind('}').map_or(text, |j| &text[..=j]);
    let v: Value = serde_json::from_str(text).map_err(|_| format!("the muser live response is not JSON: {rest}"))?;
    if code >= 400 {
        return Err(v["error"].as_str().unwrap_or("muser live reported an error").to_string());
    }
    Ok(v)
}

/// 在后台启动 `muser live start …`，与调用它的进程完全脱离（unix 上新开一个会话，Windows 上脱离控制台）：
/// 调用者（agent 的一条 shell 命令、muser-mcp）结束时音乐不跟着停。等到服务能回应（最多 10 秒）就返回它的状态；
/// 输出写进临时目录的 `muser-live.log`。已经有一场在进行时报错。
pub fn spawn_detached(muser: &std::path::Path, args: &[String]) -> Result<Value, String> {
    if request(None, "GET", "/api/status", None).is_ok() {
        return Err("a live session is already running: run muser live stop first, or just send to keep writing".into());
    }
    let log = std::env::temp_dir().join("muser-live.log");
    let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(muser);
    cmd.args(["live", "start"]).args(args).stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid 只改变子进程自己的会话，在 fork 之后、exec 之前调用是异步信号安全的。
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let _ = std::fs::remove_file(port_file());
    let mut child = cmd.spawn().map_err(|e| format!("failed to start muser live: {e}"))?;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(v) = request(None, "GET", "/api/status", None) {
            return Ok(v);
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
    }
    Err(format!("muser live did not start: {}", std::fs::read_to_string(&log).unwrap_or_default().trim()))
}
