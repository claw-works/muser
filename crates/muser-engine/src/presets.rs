//! 乐器插件的内置音色（预设 / 乐器），与用户本机的音色库。
//!
//! 来源：
//! - **VST3 标准预设**（`.vstpreset`，任何 VST3 插件；放在系统的 VST3 预设目录 `<厂商>/<插件>/` 下）：直接换成插件状态。
//! - **插件自己的预设格式**：Surge XT 的 `.fxp`（直接换成状态）；Serum 2 的 `.SerumPreset`（出厂和第三方音色包都是，直接换成状态，
//!   见 [`crate::xfer`]；放在 Serum 的预设目录或 `MUSER_SERUM_PATH` 列出的文件夹里）；Kontakt 的 `.nki`（NI 的 Native Access 记录的音色库目录，
//!   读文件开头的明文元数据：名字、音色库、类别、风格）。
//! - **采集**：载不了的（Kontakt 的乐器只能在它自己的界面里载入）在插件窗口里载入一次（人，或用 computer use 的 agent），
//!   状态存进本机音色库（`~/.config/muser/sound-library/`），之后和自动的一样直接用。
//!
//! 只读、只记本机的文件路径；不复制、不改写任何采样或音色文件。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::plugin::{read_state_file, write_state_file, StateHeader};

/// 一个乐器插件（来自插件扫描）。
#[derive(Debug, Clone)]
pub struct PluginRef {
    pub path: String,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// vst3 / clap。
    pub format: String,
    /// instrument / effect（插件扫描里的类型）。
    pub kind: String,
}

impl PluginRef {
    /// 工程里写的 `instrument`。
    pub fn spec(&self) -> String {
        format!("plugin:{}#{}", self.path, self.id)
    }

    /// 用这个音色（状态文件）时工程里怎么写：乐器是音轨的 `instrument` + `instrument_params`，
    /// 效果器是 `effects` 里的一项（`path` 和 `id` 分开写：一个文件里可能既有乐器又有效果器）。
    pub fn usage(&self, state: &Path, label: &str) -> serde_json::Value {
        let state = state.display().to_string();
        if self.kind == "effect" {
            serde_json::json!({ "effect": { "type": "plugin", "path": self.path, "id": self.id, "params": { "@state": state, "@label": label } } })
        } else {
            serde_json::json!({ "instrument": self.spec(), "instrument_params": { "@state": state, "@label": label } })
        }
    }
}

/// 按 `plugin:<路径>#<id>`（或不带 `plugin:`、不带 id）在扫描结果里找乐器插件。
pub fn plugin_ref(spec: &str) -> Result<PluginRef, String> {
    let (path, id) = crate::plugin::split_spec(spec.strip_prefix("plugin:").unwrap_or(spec));
    let path = crate::plugin::expand_home(path).into_owned();
    let (list, _) = crate::plugin::scan(false)?;
    list.into_iter()
        .find(|p| p.path == path && id.is_none_or(|i| i == p.id))
        .map(|p| PluginRef { path: p.path, id: p.id, name: p.name, vendor: p.vendor, version: p.version, format: p.format, kind: p.kind })
        .ok_or_else(|| format!("plugin '{spec}' is not in the scan results (see the plugins on this machine with `muser plugin scan`)"))
}

/// 按名字或文件找一个预设（名字不分大小写；重名时要写文件路径）。
pub fn find(p: &PluginRef, key: &str) -> Result<Preset, String> {
    let all = presets(p);
    if let Some(x) = all.iter().find(|x| x.file == key) {
        return Ok(x.clone());
    }
    let hits: Vec<&Preset> = all.iter().filter(|x| x.name.eq_ignore_ascii_case(key)).collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] if p.name.to_lowercase().starts_with("serum 2") && key.to_lowercase().ends_with(".fxp") => {
            Err(format!("'{key}' is a Serum 1 preset, which muser does not read: open it in Serum 2 and save it as a .SerumPreset, then use that"))
        }
        [] => Err(format!("{} has no sound named '{key}' (list them with `muser sounds presets '{}'`)", p.name, p.spec())),
        many => Err(format!("{} sounds are named '{key}'; give the file path instead: {}", many.len(), many.iter().map(|x| x.file.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// 怎么用上这个音色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Load {
    /// 能直接换成插件状态。
    Auto,
    /// 采集过（本机音色库里有它的状态）。
    Captured,
    /// 要在插件窗口里载入一次（`muser sounds capture`）。
    Manual,
}

/// 插件的一个内置音色。
#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub name: String,
    /// 类别（文件夹或音色元数据里的类别）。
    pub category: String,
    /// 音色库 / 作者。
    pub library: String,
    /// 风格、用途标签。
    pub tags: Vec<String>,
    /// 预设 / 乐器文件。
    pub file: String,
    pub load: Load,
}

/// 本机音色库：采集来的、转换来的插件状态。
pub fn library_dir() -> PathBuf {
    crate::profile::config_dir().join("sound-library")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    /// 插件 id → 预设文件 → 采集的状态文件。
    #[serde(default)]
    captured: BTreeMap<String, BTreeMap<String, String>>,
    /// 插件 id → 名字 → 用户自己调出来存下的状态（"My sounds"）。
    #[serde(default)]
    mine: BTreeMap<String, BTreeMap<String, String>>,
}

fn write_index(idx: &Index) -> Result<(), String> {
    std::fs::create_dir_all(library_dir()).map_err(|e| e.to_string())?;
    std::fs::write(index_path(), serde_json::to_string_pretty(idx).unwrap_or_default()).map_err(|e| e.to_string())
}

/// 把用户自己调出来的音色存进音色库的"My sounds"（同名覆盖）。返回音色库里的文件。
pub fn save_mine(p: &PluginRef, name: &str, state: &Path) -> Result<PathBuf, String> {
    let out = library_dir().join(slug(&p.name)).join("mine").join(format!("{}.muserstate", slug(name)));
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    // 已经在这个位置（就是音色库里的那份）时不用复制：复制到自己会把文件截断。
    let same = std::fs::canonicalize(state).ok().zip(std::fs::canonicalize(&out).ok()).is_some_and(|(a, b)| a == b);
    if !same {
        std::fs::copy(state, &out).map_err(|e| format!("failed to save into the sound library: {e}"))?;
    }
    let mut idx = read_index();
    idx.mine.entry(p.id.clone()).or_default().insert(name.to_string(), out.display().to_string());
    write_index(&idx)?;
    Ok(out)
}

/// 状态里载入的是哪个音色文件：取每个候选文件（跳过开头的元数据）的若干 64 字节片段，看有多少原样出现在状态里。
/// Kontakt 的状态带着乐器块的原样字节，同一个音色吻合度约 0.8，别的音色约 0。按吻合度从高到低。
pub fn identify(state: &Path, candidates: &[Preset]) -> Vec<(f64, Preset)> {
    let Ok(st) = std::fs::read(state) else { return Vec::new() };
    let pieces: std::collections::HashSet<&[u8]> = st.windows(64).collect();
    let mut out: Vec<(f64, Preset)> = candidates
        .iter()
        .filter_map(|x| {
            let n = std::fs::read(&x.file).ok()?;
            let windows: Vec<&[u8]> = (2600..n.len().saturating_sub(64)).step_by(512).map(|o| &n[o..o + 64]).collect();
            if windows.is_empty() {
                return None;
            }
            let hit = windows.iter().filter(|w| pieces.contains(**w)).count();
            Some((hit as f64 / windows.len() as f64, x.clone()))
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out
}

fn index_path() -> PathBuf {
    library_dir().join("index.json")
}

fn read_index() -> Index {
    std::fs::read_to_string(index_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn captured_state(p: &PluginRef, file: &str) -> Option<PathBuf> {
    read_index().captured.get(&p.id).and_then(|m| m.get(file)).map(PathBuf::from).filter(|s| s.is_file())
}

fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// 某个预设在音色库里的状态文件位置。
pub fn library_state_path(p: &PluginRef, preset_file: &str) -> PathBuf {
    let stem = Path::new(preset_file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // 同名的预设（不同文件夹）用路径的哈希区分。
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    preset_file.hash(&mut h);
    library_dir().join(slug(&p.name)).join(format!("{}-{:08x}.muserstate", slug(&stem), h.finish() as u32))
}

/// 记下一次采集：这个预设文件对应这份状态。
pub fn record_capture(p: &PluginRef, preset_file: &str, state: &Path) -> Result<(), String> {
    let mut idx = read_index();
    idx.captured.entry(p.id.clone()).or_default().insert(preset_file.to_string(), state.display().to_string());
    write_index(&idx)
}

fn walk(dir: &Path, ext: &str, depth: usize, skip: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(it) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = it.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') || skip.iter().any(|s| name == *s) {
            continue;
        }
        if p.is_dir() {
            if depth > 0 {
                walk(&p, ext, depth - 1, skip, out);
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}

fn home() -> PathBuf {
    crate::plugin::home_dir().unwrap_or_default()
}

/// VST3 规范的预设目录（用户、全局）下 `<厂商>/<插件>/`。
fn vst3_preset_dirs(p: &PluginRef) -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![home().join("Library/Audio/Presets"), PathBuf::from("/Library/Audio/Presets")]
    } else if cfg!(windows) {
        vec![home().join("Documents").join("VST3 Presets"), std::env::var_os("PROGRAMDATA").map(PathBuf::from).unwrap_or_default().join("VST3 Presets")]
    } else {
        vec![home().join(".vst3/presets"), PathBuf::from("/usr/share/vst3/presets"), PathBuf::from("/usr/local/share/vst3/presets")]
    };
    roots.into_iter().map(|r| r.join(&p.vendor).join(&p.name)).collect()
}

/// Native Access 记录的 NI 音色库目录。
fn ni_content_dirs() -> Vec<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("PUBLIC").unwrap_or_default()).join("Documents").join("Native Instruments").join("installed_products")
    } else {
        PathBuf::from("/Users/Shared/Native Instruments/installed_products")
    };
    let mut out = Vec::new();
    for f in std::fs::read_dir(base).into_iter().flatten().flatten() {
        let v: Option<serde_json::Value> = std::fs::read_to_string(f.path()).ok().and_then(|t| serde_json::from_str(&t).ok());
        if let Some(d) = v.as_ref().and_then(|v| v["ContentDir"].as_str()) {
            out.push(PathBuf::from(d));
        }
    }
    out
}

/// `.nki` 开头的明文元数据（UTF-16）：音色库、类别、风格标签。
fn nki_meta(path: &Path) -> (String, String, Vec<String>) {
    let Ok(b) = std::fs::read(path) else { return Default::default() };
    // 只看音色信息那一块（第 3 个 `hsin` 容器）：后面是快速控制的旋钮名，不是标签。
    let hsin: Vec<usize> = b.windows(4).take(4000).enumerate().filter(|(_, w)| *w == b"hsin").map(|(i, _)| i).collect();
    let end = hsin.get(3).map_or(2600, |&i| i.saturating_sub(12)).min(b.len());
    let head = &b[..end];
    // UTF-16 字符串不一定从偶数字节开始：两种对齐都扫，按位置合并。
    let mut found: Vec<(usize, String)> = Vec::new();
    for align in 0..2 {
        let mut cur = String::new();
        let mut start = align;
        for (k, pair) in head[align..].chunks_exact(2).enumerate() {
            match (pair[0], pair[1]) {
                (c @ 0x20..=0x7e, 0) => {
                    if cur.is_empty() {
                        start = align + 2 * k;
                    }
                    cur.push(c as char);
                }
                _ => {
                    if cur.len() >= 3 {
                        found.push((start, std::mem::take(&mut cur)));
                    }
                    cur.clear();
                }
            }
        }
    }
    found.sort();
    let strings: Vec<String> = found.into_iter().map(|(_, s)| s).collect();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // 第一次出现的音色名是名字本身；之后再出现（如 "Piano Uno" 库里的 "Piano Uno"）是音色库名，保留。
    let mut named = false;
    let mut kept: Vec<String> = strings
        .into_iter()
        .filter(|s| {
            let is_name = !named && s == &stem;
            named |= is_name;
            !is_name
        })
        .filter(|s| !s.starts_with("\\@") && s != "000" && s != "KontaktInstrument" && s != "Native Instruments")
        .filter(|s| !s.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .filter(|s| !(s.len() == 3 && s.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && s[1..].chars().all(|c| c.is_ascii_digit())))
        .flat_map(|s| s.split("\\:").map(|t| t.trim_start_matches("\\.").trim_matches('"').trim().to_string()).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect();
    let mut seen = std::collections::HashSet::new();
    kept.retain(|s| seen.insert(s.to_lowercase()));
    let library = if kept.is_empty() { String::new() } else { kept.remove(0) };
    let category = if kept.is_empty() { String::new() } else { kept.remove(0) };
    (library, category, kept)
}

/// 按关键词（空格隔开，都要出现在名字、类别、音色库或标签里）筛选 [`presets`]。
pub fn search(p: &PluginRef, filter: Option<&str>) -> Vec<Preset> {
    let terms: Vec<String> = filter.unwrap_or_default().to_lowercase().split_whitespace().map(str::to_string).collect();
    presets(p)
        .into_iter()
        .filter(|x| {
            let hay = format!("{} {} {} {}", x.name, x.category, x.library, x.tags.join(" ")).to_lowercase();
            terms.iter().all(|t| hay.contains(t))
        })
        .collect()
}

/// 插件的内置音色（最前面是用户自己存下的"My sounds"）。
pub fn presets(p: &PluginRef) -> Vec<Preset> {
    let mut out: Vec<Preset> = read_index()
        .mine
        .get(&p.id)
        .into_iter()
        .flatten()
        .filter(|(_, f)| Path::new(f).is_file())
        .map(|(name, f)| Preset { name: name.clone(), category: "My sounds".into(), library: "muser".into(), tags: Vec::new(), file: f.clone(), load: Load::Captured })
        .collect();
    let status = |file: &str, auto: bool| if captured_state(p, file).is_some() { Load::Captured } else if auto { Load::Auto } else { Load::Manual };
    // VST3 标准预设。
    if p.format == "vst3" {
        for dir in vst3_preset_dirs(p) {
            let mut files = Vec::new();
            walk(&dir, "vstpreset", 4, &[], &mut files);
            for f in files {
                let rel = f.parent().and_then(|d| d.strip_prefix(&dir).ok()).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                let file = f.display().to_string();
                out.push(Preset { name: stem(&f), category: rel, library: p.vendor.clone(), tags: Vec::new(), load: status(&file, true), file });
            }
        }
    }
    let lname = p.name.to_lowercase();
    if lname.starts_with("surge xt") {
        let roots = [
            PathBuf::from("/Library/Application Support/Surge XT/patches_factory"),
            PathBuf::from("/Library/Application Support/Surge XT/patches_3rdparty"),
            home().join("Documents/Surge XT/Patches"),
            home().join(".local/share/surge-xt/patches_factory"),
            PathBuf::from("/usr/share/surge-xt/patches_factory"),
        ];
        for root in roots {
            let mut files = Vec::new();
            walk(&root, "fxp", 4, &[], &mut files);
            let lib = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for f in files {
                let rel: Vec<String> = f.parent().and_then(|d| d.strip_prefix(&root).ok()).map(|r| r.iter().map(|c| c.to_string_lossy().into_owned()).collect()).unwrap_or_default();
                let file = f.display().to_string();
                out.push(Preset { name: stem(&f), category: rel.join(" / "), library: lib.clone(), tags: Vec::new(), load: status(&file, true), file });
            }
        }
    }
    if lname.starts_with("serum 2") {
        // Serum 2 是乐器的音色，Serum 2 FX 是效果链（单个效果和整条链）。
        let (sub, exts): (&str, &[&str]) = if lname.contains("fx") { ("Effect Chains", &["SerumFXRack", "SerumFX"]) } else { ("Presets", &["SerumPreset"]) };
        for root in serum_dirs(sub) {
            let mut files = Vec::new();
            for ext in exts {
                walk(&root, ext, 8, &[], &mut files);
            }
            for f in files {
                let rel: Vec<String> = f.parent().and_then(|d| d.strip_prefix(&root).ok()).map(|r| r.iter().map(|c| c.to_string_lossy().into_owned()).collect()).unwrap_or_default();
                let m = std::fs::read(&f).ok().and_then(|b| crate::xfer::meta(&b).ok()).unwrap_or_default();
                let file = f.display().to_string();
                let name = if m.name.is_empty() { stem(&f) } else { m.name };
                out.push(Preset { name, category: rel.join(" / "), library: m.author, tags: m.tags, load: status(&file, true), file });
            }
        }
    }
    if lname.starts_with("kontakt") {
        for dir in ni_content_dirs() {
            let mut files = Vec::new();
            walk(&dir, "nki", 6, &["Templates", "default"], &mut files);
            for f in files {
                let (library, category, tags) = nki_meta(&f);
                let file = f.display().to_string();
                let library = if library.is_empty() { dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { library };
                out.push(Preset { name: stem(&f), category, library, tags, load: status(&file, false), file });
            }
        }
    }
    out
}

/// Serum 2 预设树里的一个子目录（`Presets` 乐器音色、`Effect Chains` 效果链；出厂与装进去的第三方音色包），
/// 再加 `MUSER_SERUM_PATH`（用系统的路径分隔符隔开）列出的文件夹。
fn serum_dirs(sub: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("MUSER_SERUM_PATH").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    let tree = Path::new("Xfer Records").join("Serum 2 Presets");
    dirs.push(PathBuf::from("/Library/Audio/Presets").join(&tree).join(sub));
    dirs.push(home().join("Library/Audio/Presets").join(&tree).join(sub));
    for var in ["PROGRAMDATA", "PUBLIC"] {
        if let Some(v) = std::env::var_os(var) {
            let base = PathBuf::from(v);
            dirs.push(base.join("Xfer").join("Serum 2 Presets").join(sub));
            dirs.push(base.join("Documents").join("Xfer").join("Serum 2 Presets").join(sub));
            dirs.push(base.join(&tree).join(sub));
        }
    }
    dirs.push(home().join("Documents/Xfer/Serum 2 Presets").join(sub));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| d.is_dir() && seen.insert(d.canonicalize().unwrap_or_else(|_| d.clone())));
    dirs
}

/// Serum 2 FX 自己的初始 processor 状态（效果链要装进它里面）：第一次用时打开一次插件存下来，记在本机音色库里；
/// 插件升级（版本变了）后重新存。
fn fx_skeleton(p: &PluginRef) -> Result<Vec<u8>, String> {
    let file = library_dir().join(slug(&p.name)).join("_init.muserstate");
    let fresh = read_state_file(&file).ok().filter(|(h, _)| h.plugin_id == p.id && h.version == p.version);
    let data = match fresh {
        Some((_, d)) => d,
        None => {
            if let Some(d) = file.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            crate::plugin::save_state(&format!("{}#{}", p.path, p.id), &serde_json::Map::new(), &file)
                .map_err(|e| format!("cannot open {} to read its initial state (needed once for effect chains): {e}", p.name))?;
            read_state_file(&file)?.1
        }
    };
    // MBVST3 容器：魔数 8、类 id 16、u64 组件长度、组件（processor 状态）……
    let n = data.get(24..32).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()) as usize).ok_or("the saved Serum 2 FX state is too short")?;
    if !data.starts_with(b"MBVST3\0\x01") {
        return Err("the saved Serum 2 FX state is not a VST3 state".into());
    }
    data.get(32..32 + n).map(<[u8]>::to_vec).ok_or_else(|| "the saved Serum 2 FX state is truncated".into())
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn header(p: &PluginRef) -> StateHeader {
    StateHeader { plugin_id: p.id.clone(), plugin_name: p.name.clone(), version: p.version.clone(), format: p.format.clone() }
}

/// 本宿主的 VST3 状态容器（见 manguebyte-host 的 vst3::state）：类 id（十六进制的 16 字节）+ 组件状态 + 控制器状态。
fn vst3_container(class_hex: &str, component: &[u8], controller: &[u8]) -> Result<Vec<u8>, String> {
    let id: Vec<u8> = (0..class_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(class_hex.get(i..i + 2).unwrap_or("zz"), 16))
        .collect::<Result<_, _>>()
        .map_err(|_| format!("plugin id '{class_hex}' is not a 32-digit hexadecimal VST3 class id"))?;
    if id.len() != 16 {
        return Err(format!("plugin id '{class_hex}' is not 16 bytes"));
    }
    let mut b = b"MBVST3\0\x01".to_vec();
    b.extend_from_slice(&id);
    b.extend_from_slice(&(component.len() as u64).to_le_bytes());
    b.extend_from_slice(component);
    b.extend_from_slice(&(controller.len() as u64).to_le_bytes());
    b.extend_from_slice(controller);
    Ok(b)
}

/// `.vstpreset` → (组件状态, 控制器状态)。格式：`VST3`、版本、32 字符类 id、块表偏移；块表 `List`、数量、
/// 每项（4 字符 id、偏移、长度），`Comp` 是组件状态、`Cont` 是控制器状态。
pub(crate) fn read_vstpreset(b: &[u8], class_hex: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let bad = || "not a valid .vstpreset".to_string();
    if b.len() < 48 || &b[..4] != b"VST3" {
        return Err(bad());
    }
    let class = std::str::from_utf8(&b[8..40]).map_err(|_| bad())?;
    if !class.eq_ignore_ascii_case(class_hex) {
        return Err(format!("this preset belongs to another plugin (class id {class})"));
    }
    let list = i64::from_le_bytes(b[40..48].try_into().map_err(|_| bad())?) as usize;
    let at = |o: usize, n: usize| b.get(o..o + n).ok_or_else(bad);
    if at(list, 4)? != b"List" {
        return Err(bad());
    }
    let count = i32::from_le_bytes(at(list + 4, 4)?.try_into().map_err(|_| bad())?) as usize;
    let (mut comp, mut cont) = (Vec::new(), Vec::new());
    for k in 0..count {
        let e = list + 8 + k * 20;
        let id = at(e, 4)?;
        let off = i64::from_le_bytes(at(e + 4, 8)?.try_into().map_err(|_| bad())?) as usize;
        let size = i64::from_le_bytes(at(e + 12, 8)?.try_into().map_err(|_| bad())?) as usize;
        let data = at(off, size)?.to_vec();
        match id {
            b"Comp" => comp = data,
            b"Cont" => cont = data,
            _ => {}
        }
    }
    Ok((comp, cont))
}

/// Surge 的 `.fxp`：60 字节文件头（`CcnK` … `FPCh` … `cjs3`、名字、块长）之后是插件的状态块（`sub3` …）。
pub(crate) fn read_fxp(b: &[u8]) -> Result<Vec<u8>, String> {
    if b.len() < 60 || &b[..4] != b"CcnK" || &b[8..12] != b"FPCh" {
        return Err("not a valid .fxp preset".into());
    }
    let size = u32::from_be_bytes(b[56..60].try_into().unwrap_or_default()) as usize;
    b.get(60..60 + size).map(<[u8]>::to_vec).ok_or_else(|| "the fxp preset is truncated".into())
}

/// 这个预设的插件状态文件（在音色库里；能自动转换的第一次用时写出来）。要人工载入的返回错误，提示怎么采集。
pub fn state_for(p: &PluginRef, preset: &Preset) -> Result<PathBuf, String> {
    if let Some(s) = captured_state(p, &preset.file) {
        return Ok(s);
    }
    if preset.file.ends_with(".muserstate") && read_state_file(Path::new(&preset.file)).is_ok() {
        return Ok(PathBuf::from(&preset.file));
    }
    let out = library_state_path(p, &preset.file);
    if out.is_file() && read_state_file(&out).is_ok() {
        return Ok(out);
    }
    let bytes = std::fs::read(&preset.file).map_err(|e| format!("failed to read preset '{}': {e}", preset.file))?;
    let ext = Path::new(&preset.file).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let data = match (ext.as_str(), p.format.as_str()) {
        ("vstpreset", "vst3") => {
            let (comp, cont) = read_vstpreset(&bytes, &p.id)?;
            vst3_container(&p.id, &comp, &cont)?
        }
        ("fxp", "vst3") => vst3_container(&p.id, &read_fxp(&bytes)?, &[])?,
        // Serum 2：预设就是完整的 processor 状态，controller 段留空，插件照常读入。
        ("serumpreset", "vst3") => vst3_container(&p.id, &crate::xfer::preset_component(&bytes)?, &[])?,
        ("serumfx" | "serumfxrack", "vst3") => vst3_container(&p.id, &crate::xfer::fx_component(&fx_skeleton(p)?, &bytes)?, &[])?,
        ("fxp", _) => read_fxp(&bytes)?,
        _ => {
            return Err(format!(
                "'{}' must be loaded once in the plugin window: `muser sounds capture '{}' '{}'` (a person clicks through, or an agent uses computer use); after that it can be used directly",
                preset.name,
                p.spec(),
                preset.file
            ))
        }
    };
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    write_state_file(&out, &header(p), &data)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_vstpreset_and_fxp() {
        let class = "ABCDEF019182FAEB566D624153675854";
        // 一个最小的 .vstpreset：头 48 字节，Comp "abc"、Cont "de"，块表在后面。
        let mut v = b"VST3".to_vec();
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend_from_slice(class.as_bytes());
        let list_at = 48 + 5;
        v.extend_from_slice(&(list_at as i64).to_le_bytes());
        v.extend_from_slice(b"abcde");
        v.extend_from_slice(b"List");
        v.extend_from_slice(&2i32.to_le_bytes());
        for (id, off, size) in [(b"Comp", 48i64, 3i64), (b"Cont", 51, 2)] {
            v.extend_from_slice(id);
            v.extend_from_slice(&off.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
        }
        assert_eq!(read_vstpreset(&v, &class.to_lowercase()).unwrap(), (b"abc".to_vec(), b"de".to_vec()));
        assert!(read_vstpreset(&v, "00000000000000000000000000000000").unwrap_err().contains("another plugin"));
        let c = vst3_container(class, b"abc", b"de").unwrap();
        assert_eq!(&c[..8], b"MBVST3\0\x01");
        assert_eq!(c[8..24], [0xAB, 0xCD, 0xEF, 0x01, 0x91, 0x82, 0xFA, 0xEB, 0x56, 0x6D, 0x62, 0x41, 0x53, 0x67, 0x58, 0x54]);
        assert_eq!(c.len(), 8 + 16 + 8 + 3 + 8 + 2);

        let mut fxp = b"CcnK\0\0\0\0FPCh\0\0\0\x01cjs3".to_vec();
        fxp.resize(56, 0);
        fxp.extend_from_slice(&4u32.to_be_bytes());
        fxp.extend_from_slice(b"sub3extra");
        assert_eq!(read_fxp(&fxp).unwrap(), b"sub3");
    }
}
