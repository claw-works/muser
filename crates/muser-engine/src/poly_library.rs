//! 本机 poly 音色库：每个音色一个 JSON 文件，放在 `~/.config/muser/poly/<名字>.json`
//! （见 [`crate::profile::config_dir`]）。
//!
//! 文件里总是**完整**音色（保存时按 [`poly_patch`] 规范化），所以复制进工程就能在任何机器上渲染。
//! 工程里用法：`"instrument": "synth.poly", "instrument_params": { "sound": "<名字>", "patch": { … } }`，
//! `sound` 只作来源标注，渲染只看 `patch`。

use std::path::{Path, PathBuf};

use manguebyte::dsp::instruments::poly::{self, PolySynthParams};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::poly_patch;

/// 文件格式标识与版本。
pub const FORMAT: &str = "muser.poly";
pub const VERSION: u32 = 1;

/// 音色库里的一个音色。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    pub format: String,
    pub version: u32,
    /// 名字：小写字母、数字、`_`、`-`，同时是文件名。
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub author: String,
    /// 从哪个音色改来的（如 `poly.reese`），只作记录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// 完整音色（[`poly_patch::to_json`] 的格式）。
    pub patch: Value,
}

impl Sound {
    /// 这个音色的合成器参数。
    pub fn params(&self) -> Result<PolySynthParams, String> {
        let mut p = PolySynthParams::default();
        poly_patch::apply(&mut p, &self.patch)?;
        Ok(p)
    }

    /// 写进工程音轨的写法（完整音色内嵌，`sound` 标注来源）。
    pub fn track_snippet(&self) -> Value {
        json!({ "instrument": "synth.poly", "instrument_params": { "sound": self.name, "patch": self.patch } })
    }
}

/// 名字是否可用：1–64 个小写字母、数字、`_`、`-`，以字母或数字开头（也保证不会写到目录外）。
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=64).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn check_name(name: &str) -> Result<(), String> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(format!("音色名 '{name}' 不可用：只能用小写字母、数字、_、-（以字母或数字开头，最多 64 个字符）"))
    }
}

/// 音色库目录。
pub fn dir() -> PathBuf {
    crate::profile::config_dir().join("poly")
}

pub fn list() -> Vec<Sound> {
    list_in(&dir())
}

pub fn load(name: &str) -> Result<Sound, String> {
    load_in(&dir(), name)
}

/// 保存（规范化成完整音色）。`overwrite` 为假时同名已存在则报错。返回文件路径。
pub fn save(sound: Sound, overwrite: bool) -> Result<PathBuf, String> {
    save_in(&dir(), sound, overwrite)
}

pub fn delete(name: &str) -> Result<(), String> {
    delete_in(&dir(), name)
}

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

fn read(path: &Path) -> Result<Sound, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取音色 '{}' 失败: {e}", path.display()))?;
    let s: Sound = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if s.format != FORMAT {
        return Err(format!("{}: 不是 poly 音色文件（format 应为 \"{FORMAT}\"）", path.display()));
    }
    s.params().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(s)
}

pub(crate) fn list_in(dir: &Path) -> Vec<Sound> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Sound> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| read(&p).map_err(|e| eprintln!("[muser] 跳过音色文件：{e}")).ok())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(crate) fn load_in(dir: &Path, name: &str) -> Result<Sound, String> {
    check_name(name)?;
    let path = file(dir, name);
    if !path.is_file() {
        return Err(format!("音色库里没有 '{name}'（{}）", dir.display()));
    }
    read(&path)
}

pub(crate) fn save_in(dir: &Path, mut sound: Sound, overwrite: bool) -> Result<PathBuf, String> {
    check_name(&sound.name)?;
    sound.patch = poly_patch::to_json(&sound.params()?);
    sound.format = FORMAT.into();
    sound.version = VERSION;
    let path = file(dir, &sound.name);
    if path.exists() && !overwrite {
        return Err(format!("音色 '{}' 已存在", sound.name));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&sound).map_err(|e| e.to_string())? + "\n";
    // 先写临时文件再改名：写到一半出错不会毁掉已有音色。
    let tmp = dir.join(format!(".{}.json.tmp", sound.name));
    std::fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(path)
}

pub(crate) fn delete_in(dir: &Path, name: &str) -> Result<(), String> {
    check_name(name)?;
    std::fs::remove_file(file(dir, name)).map_err(|e| format!("删除音色 '{name}' 失败: {e}"))
}

/// 按名字取音色：`poly.<预设>`、`synth.poly`（初始音色）或音色库里的名字。
pub fn resolve(name: &str) -> Result<Sound, String> {
    let name = name.trim();
    let preset = |p: &poly::Preset, id: String| Sound {
        format: FORMAT.into(),
        version: VERSION,
        name: id.clone(),
        description: p.description.into(),
        tags: vec![p.category.name().into()],
        author: String::new(),
        base: Some(id),
        patch: poly_patch::to_json(&p.params),
    };
    if name == "synth.poly" {
        return Ok(Sound {
            format: FORMAT.into(),
            version: VERSION,
            name: name.into(),
            description: "初始音色".into(),
            tags: Vec::new(),
            author: String::new(),
            base: Some(name.into()),
            patch: poly_patch::to_json(&PolySynthParams::default()),
        });
    }
    if let Some(slug) = name.strip_prefix("poly.") {
        if let Some(p) = poly::PRESETS.iter().find(|p| crate::instrument::slug(p.name) == slug) {
            return Ok(preset(p, name.into()));
        }
        return Err(format!("没有 poly 预设 '{name}'（muser instruments --family poly 查看全部）"));
    }
    load(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_list_delete() {
        let dir = std::env::temp_dir().join(format!("muser-polylib-{}", std::process::id()));
        let mut s = resolve("poly.reese").unwrap();
        s.name = "wide_reese".into();
        s.patch = json!({ "oscillators": [{ "unison": 8 }] });
        let path = save_in(&dir, s.clone(), false).unwrap();
        let got = load_in(&dir, "wide_reese").unwrap();
        assert_eq!(got.params().unwrap().oscillators[0].unison, 8);
        assert!(got.patch["filters"].is_array(), "保存时规范化成完整音色");
        assert!(save_in(&dir, s.clone(), false).unwrap_err().contains("已存在"));
        assert!(save_in(&dir, s, true).is_ok());
        assert_eq!(list_in(&dir).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["wide_reese"]);
        // 坏文件跳过，不影响列表。
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        assert_eq!(list_in(&dir).len(), 1);
        delete_in(&dir, "wide_reese").unwrap();
        assert!(load_in(&dir, "wide_reese").is_err());
        assert!(path.parent().is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names_cannot_escape_the_directory() {
        for bad in ["", "../x", "a/b", "A", ".hidden", "-x", "x y", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(valid_name("my-reese_2"));
    }

    #[test]
    fn resolve_presets_and_init() {
        assert_eq!(resolve("synth.poly").unwrap().params().unwrap(), PolySynthParams::default());
        assert!(resolve("poly.reese").unwrap().params().is_ok());
        assert!(resolve("poly.nope").is_err());
    }
}
