//! `muser self-update`：查 GitHub 上的最新版本，需要时运行官方安装脚本（scripts/install.sh / install.ps1）。
//! 下载、校验、切换版本都在脚本里（同一套代码也给一行安装用）；这里只决定版本和装到哪。
//! 网络用系统自带的 curl（macOS、Linux、Windows 10 及以上都有），不给 muser 加 HTTP 依赖。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const REPO: &str = "claw-works/muser";

pub fn run(check: bool, version: Option<String>) -> Result<()> {
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let latest = match version {
        Some(v) => normalize(&v),
        None => latest_tag()?,
    };
    if check {
        println!("current {current}, latest {latest}{}", if latest == current { "(up to date)" } else { ": run `muser self-update` to update" });
        return Ok(());
    }
    if latest == current {
        println!("already up to date: {current}");
        return Ok(());
    }
    let home = install_home();
    match &home {
        Some(h) => println!("updating {current} -> {latest} (installed at {})...", h.display()),
        None => println!(
            "updating {current} -> {latest}. This muser was not installed by the install script ({}),\n\
             so the new version goes to the default location; this copy is left untouched, and whichever comes first on PATH is the one that runs.",
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    }
    let status = installer(&latest, home.as_deref()).status().context("cannot run the install script")?;
    if !status.success() {
        bail!("install script failed ({status})");
    }
    Ok(())
}

fn normalize(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

/// `releases/latest` 会跳转到 `…/releases/tag/<版本>`：不调 API，不受频率限制。
fn latest_tag() -> Result<String> {
    let out = Command::new("curl")
        .args(["-fsSLo", NULL_DEVICE, "-w", "%{url_effective}", &format!("https://github.com/{REPO}/releases/latest")])
        .output()
        .context("curl is required to check for a new version")?;
    if !out.status.success() {
        bail!("cannot reach GitHub: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    tag_from_url(&String::from_utf8_lossy(&out.stdout)).context("could not find the latest version")
}

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn tag_from_url(url: &str) -> Option<String> {
    let tag = url.trim().rsplit_once("/tag/")?.1;
    (tag.starts_with('v') && !tag.contains('/')).then(|| tag.to_string())
}

/// 用安装脚本装的 muser 在 `<目录>/<版本>/muser`，旁边有 `<目录>/current`：更新到同一个目录。
fn install_home() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let home = exe.parent()?.parent()?;
    home.join("current").exists().then(|| home.to_path_buf())
}

/// 脚本取自要装的那个版本的标签，与发行包对应；早于 v0.4.0 的标签里还没有脚本，退回 main 上的。
#[cfg(not(windows))]
fn installer(tag: &str, home: Option<&Path>) -> Command {
    let url = |r: &str| format!("https://raw.githubusercontent.com/{REPO}/{r}/scripts/install.sh");
    let mut c = Command::new("sh");
    c.args([
        "-c",
        "s=$(curl -fsSL \"$0\" || curl -fsSL \"$1\") || { echo 'failed to download the install script' >&2; exit 1; }; printf '%s\\n' \"$s\" | sh",
        &url(tag),
        &url("main"),
    ])
    .env("MUSER_VERSION", tag);
    if let Some(h) = home {
        c.env("MUSER_HOME", h);
    }
    c
}

#[cfg(windows)]
fn installer(tag: &str, home: Option<&Path>) -> Command {
    let url = |r: &str| format!("https://raw.githubusercontent.com/{REPO}/{r}/scripts/install.ps1");
    let script = format!("$s = try {{ irm '{}' }} catch {{ irm '{}' }}; iex $s", url(tag), url("main"));
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]).env("MUSER_VERSION", tag);
    if let Some(h) = home {
        c.env("MUSER_HOME", h);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_from_the_latest_redirect() {
        assert_eq!(tag_from_url("https://github.com/claw-works/muser/releases/tag/v0.4.0\n").as_deref(), Some("v0.4.0"));
        assert_eq!(tag_from_url("https://github.com/claw-works/muser/releases"), None);
        assert_eq!(normalize("0.4.0"), "v0.4.0");
        assert_eq!(normalize("v0.4.0"), "v0.4.0");
    }
}
