//! 插件自己往 stderr 打的日志（如 Kontakt 8 每个实例都打的
//! `[2026-11-03 16:01:31.723] [error] PresetSlotManager::selectSlot: slot not found …`）会刷满 muser 的终端，
//! 看起来像 muser 出错了。插件在进程内、在自己的线程里随时写 fd 2，只能在 fd 层面拦：
//! 第一次加载插件时把 fd 1、fd 2 各接到一根管道上，**以日志时间戳开头的行**（spdlog 等日志库的默认格式
//! `[YYYY-MM-DD …`）写进临时目录的 `muser-plugin.log`，其余（muser 自己的输出、进度条）原样转回终端。
//! 进程退出时（atexit）把 fd 还原并等管道里剩下的内容转完，不丢 muser 最后的报错。
//! `MUSER_PLUGIN_LOG=show` 关掉过滤。

use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::sync::mpsc;
use std::sync::{Mutex, Once};
use std::time::Duration;

static ONCE: Once = Once::new();
/// 每个被接管的 fd：原来指向的文件（dup 出来的 fd）和"转发线程已转完"的信号。
static STATE: Mutex<Vec<(i32, i32, mpsc::Receiver<()>)>> = Mutex::new(Vec::new());

/// 开始过滤（只做一次；设了 `MUSER_PLUGIN_LOG=show` 时什么也不做）。
/// stdout 和 stderr 都接管：Kontakt 8 的日志写在 stdout 上。提示一律写到原来的 stderr，不混进 stdout（agent 读的 JSON）。
pub fn install() {
    ONCE.call_once(|| {
        if std::env::var_os("MUSER_PLUGIN_LOG").is_some_and(|v| v == "show") {
            return;
        }
        // SAFETY: 只操作本进程的文件描述符。
        let note = unsafe { libc::dup(2) };
        if note < 0 {
            return;
        }
        // SAFETY: 同上。
        unsafe { libc::fcntl(note, libc::F_SETFD, libc::FD_CLOEXEC) };
        // SAFETY: note 是刚 dup 出来、只归这里所有的 fd。
        let note = std::sync::Arc::new(Mutex::new(unsafe { std::fs::File::from_raw_fd(note) }));
        let mut taken = 0;
        for fd in [1, 2] {
            if let Some(entry) = take_over(fd, note.clone()) {
                STATE.lock().unwrap_or_else(|e| e.into_inner()).push(entry);
                taken += 1;
            }
        }
        if taken > 0 {
            // SAFETY: restore 是不带参数、不会 unwind 的 extern "C" 函数。
            unsafe { libc::atexit(restore) };
        }
    });
}

/// 把 `fd` 接到一根管道上，起一个转发线程。失败时 fd 保持原样、返回 None。
fn take_over(fd: i32, note: std::sync::Arc<Mutex<std::fs::File>>) -> Option<(i32, i32, mpsc::Receiver<()>)> {
    // SAFETY: 只操作本进程的文件描述符；失败时把已打开的关掉。
    unsafe {
        let orig = libc::dup(fd);
        if orig < 0 {
            return None;
        }
        let mut fds = [0i32; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            libc::close(orig);
            return None;
        }
        let (r, w) = (fds[0], fds[1]);
        libc::fcntl(orig, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(r, libc::F_SETFD, libc::FD_CLOEXEC);
        let out = libc::dup(orig);
        if out < 0 || libc::dup2(w, fd) < 0 {
            for x in [orig, r, w, out] {
                if x >= 0 {
                    libc::close(x);
                }
            }
            return None;
        }
        libc::close(w);
        let (done_tx, done_rx) = mpsc::channel();
        let reader = std::fs::File::from_raw_fd(r);
        let out = std::fs::File::from_raw_fd(out);
        let spawned = std::thread::Builder::new().name("muser-plugin-log".into()).spawn(move || {
            pump(reader, out, &note);
            let _ = done_tx.send(());
        });
        if spawned.is_err() {
            libc::dup2(orig, fd);
            libc::close(orig);
            return None;
        }
        Some((fd, orig, done_rx))
    }
}

extern "C" fn restore() {
    let taken = STATE.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
    for (fd, orig, _) in &taken {
        // SAFETY: 把 fd 换回原来的文件；管道的写端随之关闭，转发线程读到结尾后退出。
        unsafe { libc::dup2(*orig, *fd) };
    }
    // 插件若自己复制过 fd，管道不会结束：每根最多等半秒。
    for (_, _, done) in &taken {
        let _ = done.recv_timeout(Duration::from_millis(500));
    }
}

/// 把管道里的内容按行（`\n` 或 `\r` 结尾）分开：日志行进文件，其余转回终端。
/// 不以 `[` 开头的半行（如 `\r渲染中 42%`）立即转发，进度条不会被拖住。
fn pump(mut reader: std::fs::File, mut out: std::fs::File, note: &Mutex<std::fs::File>) {
    let path = std::env::temp_dir().join("muser-plugin.log");
    let mut log: Option<std::fs::File> = None;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        let mut start = 0;
        while let Some(i) = buf[start..].iter().position(|&b| b == b'\n' || b == b'\r') {
            let end = start + i + 1;
            route(&buf[start..end], &mut out, &mut log, &path, note);
            start = end;
        }
        buf.drain(..start);
        if !buf.is_empty() && buf[0] != b'[' {
            let _ = out.write_all(&buf);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        route(&buf, &mut out, &mut log, &path, note);
    }
}

fn route(line: &[u8], out: &mut std::fs::File, log: &mut Option<std::fs::File>, path: &std::path::Path, note: &Mutex<std::fs::File>) {
    if !is_log_line(line) {
        let _ = out.write_all(line);
        return;
    }
    if log.is_none() {
        *log = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        static NOTED: Once = Once::new();
        NOTED.call_once(|| {
            if let Ok(mut n) = note.lock() {
                let _ = writeln!(n, "(the plugin's own log output was redirected to {}; set MUSER_PLUGIN_LOG=show to display it directly)", path.display());
            }
        });
    }
    match log {
        Some(f) => {
            let _ = f.write_all(line);
        }
        None => {
            let _ = out.write_all(line);
        }
    }
}

/// `[2026-11-03 16:01:31.723] …`：方括号里以日期开头（spdlog 等日志库的默认格式）。
fn is_log_line(line: &[u8]) -> bool {
    let d = |i: usize| line.get(i).is_some_and(u8::is_ascii_digit);
    line.first() == Some(&b'[') && (1..5).all(d) && line.get(5) == Some(&b'-') && d(6) && d(7) && line.get(8) == Some(&b'-')
}

#[cfg(test)]
mod tests {
    use super::is_log_line;

    #[test]
    fn spots_timestamped_plugin_logs_only() {
        assert!(is_log_line(b"[2026-11-03 16:01:31.723] [error] PresetSlotManager::selectSlot: slot not found\n"));
        assert!(!is_log_line(b"[view] \xe7\x89\x88\xe6\x9c\xac 1\n"));
        assert!(!is_log_line("\r渲染中  42%".as_bytes()));
        assert!(!is_log_line(b"[2026] short\n"));
        assert!(!is_log_line(b"Error: something\n"));
    }
}
