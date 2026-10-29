//! 外置插件（CLAP / VST3 / LV2）的通用层：打开、描述、列参数、按名称设置参数、扫描本机插件。
//!
//! 参数一律**按名称**引用（大小写不敏感），也接受插件参数的数字 id。取值可以是：
//! - 字符串：交给插件自己解析（`"2000 Hz"`、`"-6 dB"`、`"Saw"`），与插件界面显示的文字一致；
//! - 数字：插件的自动化刻度（CLAP/LV2 为声明的 min..max，VST3 为 0..1）；
//! - 布尔：开关类参数，true = max，false = min。
//!
//! 打开插件会运行第三方代码，只用于用户本机、自己写进工程的插件路径。需要 `host` 特性。

use serde::Serialize;
use serde_json::Value;

/// 插件参数的描述（供 CLI / MCP / 文档展示）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginParam {
    pub id: u32,
    pub name: String,
    /// 画像里的别名（若有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub current: f64,
    /// 插件对当前值的文字表示。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub current_text: String,
    /// 离散参数的步数（None 为连续）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    pub automatable: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

/// 插件的基本信息。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginInfo {
    pub path: String,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub format: String,
    /// instrument / effect / note_effect / analyzer / other
    pub kind: String,
    pub categories: Vec<String>,
    pub takes_notes: bool,
    pub audio_inputs: Vec<String>,
    pub audio_outputs: Vec<String>,
    pub latency_samples: u32,
    pub params_total: usize,
    pub params_visible: usize,
    /// 参数按名称首个词分组的统计（大插件的参数结构一目了然）。
    pub param_groups: Vec<(String, usize)>,
    /// 同一文件里的其它插件（id, 名称）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub siblings: Vec<(String, String)>,
    /// 匹配到的画像摘要（若有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileSummary {
    pub origin: crate::profile::Origin,
    pub path: Option<String>,
    pub tested_version: String,
    pub aliases: usize,
    pub presets: Vec<String>,
    pub setup: serde_json::Map<String, Value>,
}

/// 扫描结果中的一项。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScannedPlugin {
    /// 可直接写进工程的引用：乐器 `plugin:<path>#<id>`，效果器用 path + id。
    pub path: String,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub format: String,
    pub kind: String,
    pub categories: Vec<String>,
}

/// 插件状态文件（`@state`）：插件自己的二进制音色数据，前面带一行 JSON 头记录插件 id 与版本，
/// 加载时据此拒绝装进别的插件。
pub const STATE_MAGIC: &[u8] = b"MUSER-STATE 1\n";

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct StateHeader {
    pub plugin_id: String,
    pub plugin_name: String,
    pub version: String,
    pub format: String,
}

pub fn write_state_file(path: &std::path::Path, header: &StateHeader, data: &[u8]) -> Result<(), String> {
    let mut out = STATE_MAGIC.to_vec();
    out.extend(serde_json::to_vec(header).map_err(|e| e.to_string())?);
    out.push(b'\n');
    out.extend_from_slice(data);
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
    }
    std::fs::write(path, out).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

pub fn read_state_file(path: &std::path::Path) -> Result<(StateHeader, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取状态文件 '{}' 失败: {e}", path.display()))?;
    let rest = bytes
        .strip_prefix(STATE_MAGIC)
        .ok_or_else(|| format!("'{}' 不是 muser 状态文件（用 `muser plugin save-state` 生成）", path.display()))?;
    let nl = rest.iter().position(|b| *b == b'\n').ok_or("状态文件头损坏")?;
    let header: StateHeader = serde_json::from_slice(&rest[..nl]).map_err(|e| format!("状态文件头损坏: {e}"))?;
    Ok((header, rest[nl + 1..].to_vec()))
}

/// 用户主目录：`HOME`，Windows 上为 `USERPROFILE`。
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(std::path::PathBuf::from)
}

/// 展开路径开头的 `~/`（用户主目录），让工程与画像里的插件路径可以跨机器通用。
pub fn expand_home(path: &str) -> std::borrow::Cow<'_, str> {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    match (rest, home_dir()) {
        (Some(rest), Some(home)) => std::borrow::Cow::Owned(home.join(rest).to_string_lossy().into_owned()),
        _ => std::borrow::Cow::Borrowed(path),
    }
}

/// 把 `path#id` 拆开。
pub fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.trim().rsplit_once('#') {
        Some((p, i)) if !i.trim().is_empty() => (p.trim(), Some(i.trim())),
        _ => (spec.trim(), None),
    }
}

#[cfg(feature = "host")]
pub use imp::*;

static LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 结束进程。加载过插件时跳过 C/C++ 的退出析构：插件实例按设计不销毁（见 `defer_release`），
/// 有的插件（Kontakt）的全局 `std::thread` 析构时还在运行，会让进程在文件都写完之后中止（SIGABRT）。
pub fn exit(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    if LOADED.load(std::sync::atomic::Ordering::Relaxed) {
        extern "C" {
            fn _exit(code: i32) -> !;
        }
        // SAFETY: 输出已刷新；工程文件、WAV、状态文件都在返回前写完并关闭。
        unsafe { _exit(code) }
    }
    std::process::exit(code)
}

#[cfg(not(feature = "host"))]
pub use stub::*;

#[cfg(not(feature = "host"))]
mod stub {
    use super::*;

    pub const HOST_DISABLED: &str = "这个 muser 构建没有启用插件宿主（`host` 特性）；发行版与默认构建都带，别用 --no-default-features 去掉它";

    pub fn info(_spec: &str) -> Result<PluginInfo, String> {
        Err(HOST_DISABLED.into())
    }
    pub fn params(_spec: &str, _filter: Option<&str>, _include_hidden: bool, _base: &serde_json::Map<String, Value>) -> Result<Vec<PluginParam>, String> {
        Err(HOST_DISABLED.into())
    }
    pub fn scan(_refresh: bool) -> Result<(Vec<ScannedPlugin>, Vec<(String, String)>), String> {
        Err(HOST_DISABLED.into())
    }
    pub fn check_params(_path: &str, _id: Option<&str>, _params: &serde_json::Map<String, Value>) -> Result<(), String> {
        Ok(())
    }
    pub fn values(_spec: &str, _key: &str, _samples: usize, _base: &serde_json::Map<String, Value>) -> Result<(PluginParam, Vec<(f64, String)>), String> {
        Err(HOST_DISABLED.into())
    }
    pub fn save_state(_spec: &str, _params: &serde_json::Map<String, Value>, _out: &std::path::Path) -> Result<StateHeader, String> {
        Err(HOST_DISABLED.into())
    }
    #[cfg(target_os = "macos")]
    pub fn edit_state(_spec: &str, _params: &serde_json::Map<String, Value>, _out: &std::path::Path) -> Result<StateHeader, String> {
        Err(HOST_DISABLED.into())
    }
    pub fn plugin_identity(_spec: &str) -> Result<(String, String, String), String> {
        Err(HOST_DISABLED.into())
    }
    pub fn draft_profile(_spec: &str, _audition: bool) -> Result<crate::profile::Profile, String> {
        Err(HOST_DISABLED.into())
    }
    pub fn record_observations(_spec: &str, _param: &str, _takes: &[crate::audition::Take]) -> Result<std::path::PathBuf, String> {
        Err(HOST_DISABLED.into())
    }
    pub fn check_params_in(_spec: &str, _profile: &crate::profile::Profile, _params: &serde_json::Map<String, Value>) -> Result<(), String> {
        Err(HOST_DISABLED.into())
    }
}

#[cfg(feature = "host")]
mod imp {
    use super::*;
    use manguebyte::host::any::{Kind, Library, Param, Plugin};
    use manguebyte::ParamId;

    pub const HOST_DISABLED: &str = "";

    /// 一个已创建（未激活）的插件。
    pub struct Opened {
        pub library: Library,
        /// 释放时不立即销毁，交给 [`defer_release`]。
        pub plugin: std::mem::ManuallyDrop<Plugin>,
        pub params: Vec<Param>,
        pub path: String,
        /// 已应用的参数（id, 值），激活后会经音频队列重发一次。
        applied: Vec<(ParamId, f64)>,
        /// 匹配到的插件画像。
        pub profile: Option<std::sync::Arc<crate::profile::Loaded>>,
        /// 已加载的状态（激活后检查是否保留）。
        state_loaded: bool,
    }

    fn kind_name(k: Kind) -> &'static str {
        match k {
            Kind::Instrument => "instrument",
            Kind::Effect => "effect",
            Kind::NoteEffect => "note_effect",
            Kind::Analyzer => "analyzer",
            _ => "other",
        }
    }

    /// 把 "-6.02dB" / "0.5 secs" / "16v" 拆成 (数值, 小写单位)。
    pub(crate) fn split_number(t: &str) -> Option<(f64, String)> {
        let t = t.trim();
        let end = t
            .char_indices()
            .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && (*i == 0 || matches!(t.as_bytes()[i - 1], b'e' | b'E'))) || ((*c == 'e' || *c == 'E') && *i > 0 && t[i + 1..].starts_with(|d: char| d.is_ascii_digit() || d == '-' || d == '+')))
            .map(|(i, c)| i + c.len_utf8())
            .last()?;
        let v: f64 = t[..end].parse().ok()?;
        Some((v, t[end..].trim().to_lowercase()))
    }

    /// 改写插件状态中 JUCE `copyXmlToBinary` 块（`VC2!`、u32 长度、XML、0）里根元素的一个属性。
    /// 状态可以就是这个块（CLAP），或包在本宿主的 VST3 容器里（组件状态，会同步修正长度）。
    pub(crate) fn patch_juce_state(data: &[u8], attr: &str, value: &str) -> Result<Vec<u8>, String> {
        const VST3: &[u8] = b"MBVST3\0\x01";
        let (prefix_len, comp_len) = if data.starts_with(VST3) {
            let at = VST3.len() + 16;
            let n = data.get(at..at + 8).ok_or("VST3 状态过短")?;
            (at + 8, Some(u64::from_le_bytes(n.try_into().unwrap()) as usize))
        } else {
            (0, None)
        };
        let block = &data[prefix_len..];
        if !block.starts_with(b"VC2!") || block.len() < 8 {
            return Err("插件状态不是 JUCE XML 格式（只支持 CLAP / VST3 版）".into());
        }
        let n = u32::from_le_bytes(block[4..8].try_into().unwrap()) as usize;
        let xml_bytes = block.get(8..8 + n).ok_or("状态长度不符")?;
        let xml = std::str::from_utf8(xml_bytes).map_err(|_| "状态里的 XML 不是 UTF-8")?;
        let xml = xml.trim_end_matches('\0');
        // 根元素：跳过 <?xml ...?> 声明。
        let root = xml
            .match_indices('<')
            .map(|(i, _)| i)
            .find(|&i| !xml[i..].starts_with("<?") && !xml[i..].starts_with("<!"))
            .ok_or("XML 没有根元素")?;
        let end = root + xml[root..].find('>').ok_or("XML 根元素未结束")?;
        let tag = &xml[root..end];
        let escaped = value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let needle = format!(" {attr}=\"");
        let new_tag = match tag.find(&needle) {
            Some(i) => {
                let vstart = i + needle.len();
                let vend = vstart + tag[vstart..].find('"').ok_or("属性值未结束")?;
                format!("{}{}{}", &tag[..vstart], escaped, &tag[vend..])
            }
            None => {
                let self_closing = tag.ends_with('/');
                let body = tag.trim_end_matches('/');
                format!("{body} {attr}=\"{escaped}\"{}", if self_closing { "/" } else { "" })
            }
        };
        let new_xml = format!("{}{}{}", &xml[..root], new_tag, &xml[end..]);
        let mut new_block = Vec::with_capacity(new_xml.len() + 9);
        new_block.extend_from_slice(b"VC2!");
        new_block.extend_from_slice(&((new_xml.len() + 1) as u32).to_le_bytes());
        new_block.extend_from_slice(new_xml.as_bytes());
        new_block.push(0);
        let old_block_len = 8 + n;
        let mut out = Vec::with_capacity(data.len() + new_block.len());
        out.extend_from_slice(&data[..prefix_len]);
        match comp_len {
            Some(cl) => {
                if cl < old_block_len {
                    return Err("VST3 组件状态长度不符".into());
                }
                let new_cl = cl - old_block_len + new_block.len();
                out.truncate(prefix_len - 8);
                out.extend_from_slice(&(new_cl as u64).to_le_bytes());
                out.extend_from_slice(&new_block);
                out.extend_from_slice(&data[prefix_len + old_block_len..]);
            }
            None => {
                out.extend_from_slice(&new_block);
                out.extend_from_slice(&block[old_block_len..]);
            }
        }
        Ok(out)
    }

    /// 进程内已加载的插件库：每个路径只加载一次，并且不再卸载。
    ///
    /// 有的插件在后台线程里做事（如 JUCE 插件的联网检查更新），卸载动态库时线程还在跑就会崩溃；
    /// 长时间运行的进程（`muser view --serve`、MCP server）会反复渲染，因此库常驻，后续打开直接复用。
    fn load_library(path: &str) -> Result<Library, String> {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static LOADED: OnceLock<Mutex<HashMap<String, Library>>> = OnceLock::new();
        let key = std::fs::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.display().to_string());
        let mut map = LOADED.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
        if let Some(lib) = map.get(&key) {
            return Ok(lib.clone());
        }
        // SAFETY: 加载会执行第三方代码；路径由用户在工程/命令中给出，视为可信的本机插件。
        let library = unsafe { Library::load(path) }.map_err(|e| format!("加载插件 '{path}' 失败: {e:?}"))?;
        map.insert(key, library.clone());
        Ok(library)
    }

    /// 延迟释放的插件实例：有的插件创建后会在后台线程里做事（如 JUCE 插件联网检查更新），
    /// 实例一释放、线程还在跑就会崩溃。实例先留在创建它的线程里，约一分钟后、该线程下次释放插件时再销毁；
    /// 长时间运行的渲染线程（`muser view`）反复渲染也不会崩，内存也不会无限增长。
    /// 线程结束时剩下的实例不再销毁（进程退出时由系统回收）。插件实例不能跨线程，所以按线程保存。
    fn defer_release(plugin: Plugin) {
        use std::cell::RefCell;
        use std::time::{Duration, Instant};
        const KEEP: Duration = Duration::from_secs(60);
        struct Grave(Vec<(Instant, Plugin)>);
        impl Drop for Grave {
            fn drop(&mut self) {
                std::mem::forget(std::mem::take(&mut self.0));
            }
        }
        thread_local! {
            static GRAVE: RefCell<Grave> = const { RefCell::new(Grave(Vec::new())) };
        }
        let expired: Vec<(Instant, Plugin)> = GRAVE.with(|g| {
            let mut g = g.borrow_mut();
            g.0.push((Instant::now(), plugin));
            let now = Instant::now();
            let keep = g.0.iter().position(|(t, _)| now.duration_since(*t) < KEEP).unwrap_or(g.0.len());
            g.0.drain(..keep).collect()
        });
        drop(expired);
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            // SAFETY: drop 只调用一次，之后不再访问 self.plugin。
            let plugin = unsafe { std::mem::ManuallyDrop::take(&mut self.plugin) };
            defer_release(plugin);
        }
    }

    impl Opened {
        /// 加载并创建插件。`id` 省略时用文件里的第一个插件。
        pub fn open(path: &str, id: Option<&str>) -> Result<Opened, String> {
            let expanded = super::expand_home(path);
            let path: &str = &expanded;
            if !std::path::Path::new(path).exists() {
                return Err(format!("找不到插件 '{path}'（用 `muser plugin scan` 查看本机插件）"));
            }
            super::LOADED.store(true, std::sync::atomic::Ordering::Relaxed);
            let library = load_library(path)?;
            let plugin_id = match id {
                Some(s) => s.to_string(),
                None => library
                    .plugins()
                    .first()
                    .map(|p| p.id.clone())
                    .ok_or_else(|| format!("'{path}' 里没有任何插件"))?,
            };
            if !library.plugins().iter().any(|p| p.id == plugin_id) {
                let ids: Vec<String> = library.plugins().iter().map(|p| format!("{} ({})", p.id, p.name)).collect();
                return Err(format!("'{path}' 里没有 id 为 '{plugin_id}' 的插件，可用: {}", ids.join(", ")));
            }
            let plugin = library
                .create(&plugin_id)
                .map_err(|e| format!("创建插件 '{plugin_id}' 失败: {e:?}"))?;
            let params = plugin.params();
            let profile = crate::profile::find(&plugin_id);
            if let Some(pr) = &profile {
                let tested = &pr.profile.source.tested_version;
                let installed = &plugin.entry().version;
                if !tested.is_empty() && tested != installed {
                    eprintln!(
                        "[muser] 提示: 画像 '{}' 验证于版本 {tested}，已安装 {installed}，参数可能有变化",
                        pr.profile.plugin.name
                    );
                }
            }
            Ok(Opened { library, plugin: std::mem::ManuallyDrop::new(plugin), params, path: path.to_string(), applied: Vec::new(), profile, state_loaded: false })
        }

        pub fn open_spec(spec: &str) -> Result<Opened, String> {
            let (path, id) = split_spec(spec);
            Opened::open(path, id)
        }

        /// 按名称（大小写不敏感）或数字 id 找参数；找不到时给出相近的名字。
        pub fn resolve(&self, key: &str) -> Result<&Param, String> {
            let k = key.trim();
            let k = self.profile.as_ref().and_then(|p| p.profile.alias(k)).unwrap_or(k);
            let lower = k.to_lowercase();
            let exact: Vec<&Param> = self.params.iter().filter(|p| p.name.to_lowercase() == lower).collect();
            match exact.len() {
                1 => return Ok(exact[0]),
                n if n > 1 => {
                    return Err(format!(
                        "参数名 '{k}' 不唯一，请改用数字 id: {}",
                        exact.iter().map(|p| p.id.0.to_string()).collect::<Vec<_>>().join(", ")
                    ))
                }
                _ => {}
            }
            if let Ok(id) = k.parse::<u32>() {
                if let Some(p) = self.params.iter().find(|p| p.id.0 == id) {
                    return Ok(p);
                }
            }
            let words: Vec<&str> = lower.split_whitespace().collect();
            let mut near: Vec<&str> = self
                .params
                .iter()
                .filter(|p| !p.hidden)
                .filter(|p| {
                    let n = p.name.to_lowercase();
                    words.iter().all(|w| n.contains(w))
                })
                .map(|p| p.name.as_str())
                .take(10)
                .collect();
            if near.is_empty() {
                near = self
                    .params
                    .iter()
                    .filter(|p| !p.hidden && words.iter().any(|w| p.name.to_lowercase().contains(w)))
                    .map(|p| p.name.as_str())
                    .take(10)
                    .collect();
            }
            Err(if near.is_empty() {
                format!("插件没有参数 '{k}'（用 `muser plugin params <插件> --filter <关键词>` 搜索）")
            } else {
                format!("插件没有参数 '{k}'，相近的: {}", near.join(" | "))
            })
        }

        /// 文字 → 数值。先用插件自己的解析；若解析结果换回文字后与输入不符（有的插件对选项类参数
        /// 解析失败时静默返回 0），就沿范围取样查找显示为这段文字的取值，取该区间的中点。
        pub fn text_value(&self, p: &Param, s: &str) -> Option<f64> {
            let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            let want = norm(s);
            let parsed = self.plugin.text_to_param(p.id, s);
            if let Some(v) = parsed {
                if self.plugin.param_to_text(p.id, v).is_some_and(|t| norm(&t) == want) {
                    return Some(v);
                }
            }
            let n = p.steps.map_or(2000, |k| (k as usize + 1) * 8).clamp(64, 4000);
            let (mut lo, mut hi) = (None, None);
            for i in 0..n {
                let x = p.min + (p.max - p.min) * i as f64 / (n - 1) as f64;
                let hit = self.plugin.param_to_text(p.id, x).is_some_and(|t| norm(&t) == want);
                if hit {
                    lo.get_or_insert(x);
                    hi = Some(x);
                } else if lo.is_some() {
                    break;
                }
            }
            match (lo, hi) {
                (Some(a), Some(b)) => Some((a + b) / 2.0),
                // 连续参数（如 "200 Hz" 显示为 "200.00 Hz"）：按显示出来的数值在范围内二分查找；
                // 找不到再信任插件自己的解析。纯文字又匹配不上任何档位（如 "banana"），视为无法解析。
                _ if s.chars().any(|c| c.is_ascii_digit()) => self.numeric_value(p, s).or(parsed),
                _ => None,
            }
        }

        /// 按显示数值查找：文字是"数字 + 单位"时，沿范围取样读出插件显示的数值（单位须相同），
        /// 在单调区间里二分到目标数值。插件自己的文字解析不可靠时（如 Vital 的非线性刻度）用它。
        fn numeric_value(&self, p: &Param, s: &str) -> Option<f64> {
            let (want, unit) = split_number(s)?;
            let read = |x: f64| -> Option<f64> {
                let t = self.plugin.param_to_text(p.id, x)?;
                let (v, u) = split_number(&t)?;
                (u == unit).then_some(v)
            };
            let n = 256;
            let xs: Vec<f64> = (0..n).map(|i| p.min + (p.max - p.min) * i as f64 / (n - 1) as f64).collect();
            let vs: Vec<Option<f64>> = xs.iter().map(|x| read(*x)).collect();
            for i in 0..n - 1 {
                let (Some(a), Some(b)) = (vs[i], vs[i + 1]) else { continue };
                if (a <= want && want <= b) || (b <= want && want <= a) {
                    let (mut lo, mut hi, rising) = (xs[i], xs[i + 1], b >= a);
                    for _ in 0..48 {
                        let mid = (lo + hi) / 2.0;
                        match read(mid) {
                            Some(v) if (v < want) == rising => lo = mid,
                            Some(_) => hi = mid,
                            None => break,
                        }
                    }
                    let x = (lo + hi) / 2.0;
                    let got = read(x)?;
                    let tol = (want.abs() * 0.02).max(1e-6);
                    return ((got - want).abs() <= tol.max((a - b).abs() * 1e-3)).then_some(x);
                }
            }
            None
        }

        /// 把 JSON 值换成插件自动化刻度上的数值。
        pub fn value(&self, p: &Param, v: &Value) -> Result<f64, String> {
            let x = match v {
                Value::String(s) => self.text_value(p, s).ok_or_else(|| {
                    let example = self.plugin.param_to_text(p.id, p.default).unwrap_or_default();
                    format!(
                        "插件无法解析 '{s}'（参数 '{}'，文字示例: \"{example}\"；用 `muser plugin values` 查看可写的文字，也可以写 {}–{} 的数字）",
                        p.name, p.min, p.max
                    )
                })?,
                Value::Bool(b) => {
                    if *b {
                        p.max
                    } else {
                        p.min
                    }
                }
                _ => v.as_f64().ok_or_else(|| format!("应为数字、文字或 true/false，实际 {v}"))?,
            };
            if !x.is_finite() || x < p.min - 1e-9 || x > p.max + 1e-9 {
                return Err(format!("{x} 超出参数 '{}' 的范围 [{}, {}]", p.name, p.min, p.max));
            }
            Ok(x.clamp(p.min, p.max))
        }

        /// 应用一组参数（激活前调用）。顺序：`@state` 状态文件 → 画像 `setup`（`"@setup": false` 关闭）
        /// → `@preset` 画像预设 → 其余参数（键可以是参数名、数字 id 或画像别名）。
        pub fn apply(&mut self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
            let format = self.profile.as_ref().map(|p| p.profile.state_format.clone()).unwrap_or_default();
            let extra = crate::profile::state_keys(&format);
            for k in params.keys().filter(|k| k.starts_with('@')) {
                if !["@state", "@setup", "@preset", "@label"].contains(&k.as_str()) && !extra.contains(&k.as_str()) {
                    let mut all = vec!["@state", "@setup", "@preset", "@label"];
                    all.extend_from_slice(extra);
                    return Err(format!("未知的保留键 '{k}'，可用: {}", all.join(" / ")));
                }
            }
            if let Some(v) = params.get("@state") {
                let path = v.as_str().ok_or("@state 应为状态文件路径")?;
                self.load_state_file(std::path::Path::new(&*super::expand_home(path)))?;
            }
            let use_setup = match params.get("@setup") {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(v) => return Err(format!("@setup 应为 true/false，实际 {v}")),
            };
            let profile = self.profile.clone();
            // 选择器：工程参数里的值优先，其次是预设里的值；先于 setup / 预设 / 其余参数应用。
            let selector = profile.as_ref().and_then(|p| p.profile.selector.clone());
            let preset_params = match params.get("@preset") {
                Some(v) => {
                    let name = v.as_str().ok_or("@preset 应为预设名")?;
                    let pr = profile.as_ref().ok_or("这个插件没有画像，不能用 @preset（`muser plugin profile draft` 生成一份）")?;
                    let preset = pr.profile.preset(name).ok_or_else(|| {
                        format!(
                            "画像里没有预设 '{name}'，可用: {}",
                            pr.profile.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    })?;
                    Some((name.to_string(), preset.params.clone()))
                }
                None => None,
            };
            // 状态改写（如 Vital 的波表、调制矩阵）：预设里的先、工程里的后（同键覆盖）。
            if format == "vital" {
                let mut patch = serde_json::Map::new();
                for (k, v) in preset_params.iter().flat_map(|(_, m)| m.iter()).chain(params.iter()) {
                    if extra.contains(&k.as_str()) {
                        patch.insert(k.clone(), v.clone());
                    }
                }
                if crate::vital::wants_patch(&patch) {
                    let data = self.plugin.save_state().map_err(|e| format!("插件保存状态失败: {e:?}"))?;
                    let mut doc = crate::vital::read_json(&data)?;
                    crate::vital::patch(&mut doc, &patch)?;
                    let out = crate::vital::write_json(&data, &doc)?;
                    self.plugin.load_state(&out).map_err(|e| format!("插件拒绝了改写后的状态: {e:?}"))?;
                    for _ in 0..4 {
                        let _ = self.plugin.pump();
                    }
                    self.params = self.plugin.params();
                    self.applied.clear();
                }
            }
            let is_sel = |k: &str| selector.as_ref().is_some_and(|s| s.key.eq_ignore_ascii_case(k.trim()));
            if let Some(sel) = &selector {
                let chosen = params
                    .iter()
                    .chain(preset_params.iter().flat_map(|(_, m)| m.iter()))
                    .find(|(k, _)| is_sel(k))
                    .map(|(_, v)| v.clone());
                if let Some(v) = chosen {
                    let name = v.as_str().ok_or_else(|| format!("{} 应为文字，实际 {v}", sel.key))?;
                    let name = sel.resolve(name)?;
                    self.select(sel, &name)?;
                }
            }
            if use_setup {
                if let Some(pr) = &profile {
                    let setup = pr.profile.setup.clone();
                    self.apply_plain(&setup).map_err(|e| format!("画像 setup: {e}"))?;
                }
            }
            if let Some((name, values)) = preset_params {
                let values: serde_json::Map<String, Value> =
                    values.into_iter().filter(|(k, _)| !is_sel(k) && !k.starts_with('@')).collect();
                self.apply_plain(&values).map_err(|e| format!("预设 '{name}': {e}"))?;
            }
            let rest: serde_json::Map<String, Value> = params
                .iter()
                .filter(|(k, _)| !k.starts_with('@') && !is_sel(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            self.apply_plain(&rest)
        }

        /// 切换选择器（见 [`crate::profile::Selector`]）：改写插件状态里 JUCE XML 根元素的属性并装回，
        /// 刷新参数表；`reset_params` 时把参数设为插件为新选项上报的默认值。
        fn select(&mut self, sel: &crate::profile::Selector, choice: &str) -> Result<(), String> {
            let data = self.plugin.save_state().map_err(|e| format!("插件保存状态失败: {e:?}"))?;
            let patched = patch_juce_state(&data, &sel.juce_xml_attribute, choice)
                .map_err(|e| format!("切换 {} 失败: {e}", sel.key))?;
            self.plugin.load_state(&patched).map_err(|e| format!("切换 {} 失败，插件拒绝了状态: {e:?}", sel.key))?;
            // 有的格式（VST3）在主线程消息里才宣布参数表变化。
            for _ in 0..4 {
                let _ = self.plugin.pump();
            }
            self.params = self.plugin.params();
            self.applied.clear();
            if sel.reset_params {
                let defaults: Vec<(ParamId, f64)> =
                    self.params.iter().filter(|p| !p.read_only).map(|p| (p.id, p.default)).collect();
                for (id, x) in defaults {
                    self.plugin.set_param(id, x).map_err(|e| format!("设置参数 {} 失败: {e:?}", id.0))?;
                    self.applied.push((id, x));
                }
            }
            Ok(())
        }

        /// 读取状态文件并装进插件（检查插件 id）。
        pub fn load_state_file(&mut self, path: &std::path::Path) -> Result<(), String> {
            let (h, data) = read_state_file(path)?;
            let e = self.plugin.entry();
            if h.plugin_id != e.id {
                return Err(format!(
                    "状态文件 '{}' 属于插件 {}（{}），不能装进 {}（{}）",
                    path.display(), h.plugin_name, h.plugin_id, e.name, e.id
                ));
            }
            if h.version != e.version {
                eprintln!("[muser] 提示: 状态文件保存于 {} {}，已安装 {}", h.plugin_name, h.version, e.version);
            }
            self.plugin.load_state(&data).map_err(|e| format!("插件拒绝了状态文件: {e:?}"))?;
            self.state_loaded = true;
            // 有的插件按状态换一套参数（如 Airwindows Consolidated 按所选效果改参数名）。
            self.params = self.plugin.params();
            Ok(())
        }

        /// 保存插件当前状态（含已应用的参数）。插件处于激活状态时也可调用。
        pub fn save_state_file(&mut self, path: &std::path::Path) -> Result<(), String> {
            let data = self.plugin.save_state().map_err(|e| format!("插件保存状态失败: {e:?}"))?;
            let e = self.plugin.entry();
            let header = StateHeader {
                plugin_id: e.id.clone(),
                plugin_name: e.name.clone(),
                version: e.version.clone(),
                format: format!("{:?}", e.format).to_lowercase(),
            };
            write_state_file(path, &header, &data)
        }

        fn apply_plain(&mut self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
            let mut writes = Vec::new();
            for (key, v) in params {
                let p = self.resolve(key)?;
                if p.read_only {
                    return Err(format!("参数 '{}' 是只读的", p.name));
                }
                let x = self.value(p, v).map_err(|e| format!("{key}: {e}"))?;
                writes.push((p.id, x));
            }
            for (id, x) in writes {
                self.plugin.set_param(id, x).map_err(|e| format!("设置参数 {} 失败: {e:?}", id.0))?;
                self.applied.retain(|(i, _)| *i != id);
                self.applied.push((id, x));
            }
            Ok(())
        }

        /// 激活后预热：用静音跑若干块，插件若把我们设的参数改回去（回报的值不同，或宣布参数变了），
        /// 就把已应用的参数按原顺序重发，直到插件回报的值与设定一致。
        ///
        /// 有的插件在第一次处理时才初始化内部状态，会覆盖激活前写入的值（如 Surge XT Effects 会把
        /// FX Type 重置为默认，并按类型重置其它参数），预热让这些参数在正式渲染前真正生效。
        /// 返回预热的块数；未收敛时返回 Err（附说明），调用方可以选择只打印警告。
        pub fn settle(
            &mut self,
            processor: &mut dyn manguebyte::prelude::Processor<f32>,
            spec: &manguebyte::prelude::ProcessSpec,
        ) -> Result<usize, String> {
            use manguebyte::host::any::Notice;
            use manguebyte::prelude::AudioBuffer;
            const MAX_BLOCKS: usize = 64;
            let block = spec.max_block_size.clamp(32, 1024);
            let mut buf = AudioBuffer::<f32>::new(spec.num_channels.max(2), block);
            // 是否已采纳：插件显示的文字一致即可（选项/步进参数会把值量化到档位上）。
            let same = |o: &Opened, id: ParamId, now: f64, want: f64| -> bool {
                if (now - want).abs() <= 1e-6 * (1.0 + want.abs()) {
                    return true;
                }
                match (o.plugin.param_to_text(id, now), o.plugin.param_to_text(id, want)) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                }
            };
            let mut quiet_rounds = 0;
            for i in 0..MAX_BLOCKS {
                for ch in 0..buf.num_channels() {
                    buf.channel_mut(ch).fill(0.0);
                }
                processor.process(&mut buf);
                let notices = self.plugin.pump();
                let disturbed = notices.iter().any(|n| match n {
                    Notice::ParamsChanged | Notice::RestartRequested => true,
                    Notice::ParamValue { param, value } => {
                        self.applied.iter().any(|(id, x)| id == param && !same(self, *id, *value, *x))
                    }
                    _ => false,
                });
                let drifted = self
                    .applied
                    .iter()
                    .any(|(id, x)| self.plugin.param_value(*id).is_some_and(|v| !same(self, *id, v, *x)));
                if disturbed || drifted {
                    quiet_rounds = 0;
                    for (id, x) in self.applied.clone() {
                        let _ = self.plugin.set_param(id, x);
                    }
                } else {
                    quiet_rounds += 1;
                    if quiet_rounds >= 2 {
                        return Ok(i + 1);
                    }
                }
            }
            let off: Vec<String> = self
                .applied
                .iter()
                .filter(|(id, x)| self.plugin.param_value(*id).is_some_and(|v| !same(self, *id, v, *x)))
                .filter_map(|(id, _)| self.params.iter().find(|p| p.id == *id).map(|p| p.name.clone()))
                .collect();
            Err(if off.is_empty() {
                format!("插件 '{}' 预热 {MAX_BLOCKS} 块后仍在报告参数变化", self.path)
            } else {
                format!("插件 '{}' 没有采纳这些参数: {}", self.path, off.join(", "))
            })
        }

        /// 有基础参数时激活并预热，让插件真正采纳它们（参数的文字表示随之更新）。
        /// 返回的 processor 必须先于本对象释放，调用方用变量接住即可。
        pub fn run_briefly(
            &mut self,
            base: &serde_json::Map<String, Value>,
        ) -> Result<Option<Box<dyn manguebyte::prelude::Processor<f32> + Send>>, String> {
            if base.is_empty() {
                return Ok(None);
            }
            let spec = manguebyte::prelude::ProcessSpec { sample_rate: 48_000.0, max_block_size: 256, num_channels: 2 };
            let mut proc = self.activate(spec.sample_rate, spec.max_block_size)?;
            proc.prepare(&spec);
            if let Err(w) = self.settle(proc.as_mut(), &spec) {
                eprintln!("[muser] 警告: {w}");
            }
            Ok(Some(proc))
        }

        /// 激活插件并把已应用的参数经音频队列再发一次。有的插件（如 Surge XT Effects 的 FX Type）
        /// 只在处理过程中收到参数事件时才真正切换，激活前写入的值会被忽略。
        pub fn activate(&mut self, sample_rate: f64, max_block_size: usize) -> Result<Box<dyn manguebyte::prelude::Processor<f32> + Send>, String> {
            let processor = self
                .plugin
                .activate(sample_rate, max_block_size)
                .map_err(|e| format!("activate 失败: {e:?}"))?;
            for (id, x) in self.applied.clone() {
                let _ = self.plugin.set_param(id, x);
            }
            Ok(processor)
        }

        pub fn describe_param(&self, p: &Param) -> PluginParam {
            let current = self.plugin.param_value(p.id).unwrap_or(p.default);
            PluginParam {
                id: p.id.0,
                name: p.name.clone(),
                alias: self.profile.as_ref().and_then(|l| {
                    l.profile.params.iter().find(|q| q.name == p.name).and_then(|q| q.alias.clone())
                }),
                unit: p.unit.clone(),
                min: p.min,
                max: p.max,
                default: p.default,
                current,
                current_text: self.plugin.param_to_text(p.id, current).unwrap_or_default(),
                steps: p.steps,
                automatable: p.automatable,
                hidden: p.hidden,
                read_only: p.read_only,
            }
        }

        pub fn info(&self) -> PluginInfo {
            let e = self.plugin.entry();
            let mut groups: Vec<(String, usize)> = Vec::new();
            for p in self.params.iter().filter(|p| !p.hidden) {
                let g = p.name.split_whitespace().next().unwrap_or("").to_string();
                match groups.iter_mut().find(|(n, _)| *n == g) {
                    Some((_, c)) => *c += 1,
                    None => groups.push((g, 1)),
                }
            }
            let bus = |b: manguebyte::host::any::Bus| format!("{} ({}ch{})", b.name, b.channels, if b.is_main { ", main" } else { "" });
            PluginInfo {
                path: self.path.clone(),
                id: e.id.clone(),
                name: e.name.clone(),
                vendor: e.vendor.clone(),
                version: e.version.clone(),
                format: format!("{:?}", e.format).to_lowercase(),
                kind: kind_name(e.kind).into(),
                categories: e.categories.clone(),
                takes_notes: self.plugin.takes_notes(),
                audio_inputs: self.plugin.audio_inputs().into_iter().map(bus).collect(),
                audio_outputs: self.plugin.audio_outputs().into_iter().map(bus).collect(),
                latency_samples: self.plugin.latency(),
                params_total: self.params.len(),
                params_visible: self.params.iter().filter(|p| !p.hidden).count(),
                param_groups: groups,
                siblings: self
                    .library
                    .plugins()
                    .iter()
                    .filter(|p| p.id != e.id)
                    .map(|p| (p.id.clone(), p.name.clone()))
                    .collect(),
                profile: self.profile.as_ref().map(|l| ProfileSummary {
                    origin: l.origin,
                    path: l.path.clone(),
                    tested_version: l.profile.source.tested_version.clone(),
                    aliases: l.profile.params.iter().filter(|q| q.alias.is_some()).count(),
                    presets: l.profile.presets.iter().map(|x| x.name.clone()).collect(),
                    setup: l.profile.setup.clone(),
                }),
            }
        }
    }

    /// 插件信息。
    pub fn info(spec: &str) -> Result<PluginInfo, String> {
        Ok(Opened::open_spec(spec)?.info())
    }

    /// 参数列表。`filter` 为空格分隔的关键词，全部命中才列出（大小写不敏感）。
    /// `base` 会先应用到插件上（如先选定效果类型，再看各参数的含义与文字）。
    pub fn params(spec: &str, filter: Option<&str>, include_hidden: bool, base: &serde_json::Map<String, Value>) -> Result<Vec<PluginParam>, String> {
        let mut o = Opened::open_spec(spec)?;
        o.apply(base)?;
        let _running = o.run_briefly(base)?;
        o.params = o.plugin.params();
        let words: Vec<String> = filter.unwrap_or("").to_lowercase().split_whitespace().map(String::from).collect();
        Ok(o.params
            .iter()
            .filter(|p| include_hidden || !p.hidden)
            .filter(|p| {
                let n = p.name.to_lowercase();
                words.iter().all(|w| n.contains(w.as_str()))
            })
            .map(|p| o.describe_param(p))
            .collect())
    }

    /// 一个参数可取的文字值：沿范围均匀取样，返回插件显示的不同文字及其对应数值（按数值递增）。
    /// 对选项类参数（滤波类型、波形…）就是全部选项；连续参数则得到一组刻度。
    pub fn values(spec: &str, key: &str, samples: usize, base: &serde_json::Map<String, Value>) -> Result<(PluginParam, Vec<(f64, String)>), String> {
        let mut o = Opened::open_spec(spec)?;
        o.apply(base)?;
        let _running = o.run_briefly(base)?;
        o.params = o.plugin.params();
        let p = o.resolve(key)?;
        let n = samples.clamp(2, 4096);
        let mut out: Vec<(f64, String)> = Vec::new();
        for i in 0..n {
            let x = p.min + (p.max - p.min) * i as f64 / (n - 1) as f64;
            if let Some(t) = o.plugin.param_to_text(p.id, x) {
                if out.last().is_none_or(|(_, last)| *last != t) {
                    out.push((x, t));
                }
            }
        }
        Ok((o.describe_param(p), out))
    }

    /// 打开插件、应用参数（含画像 setup / 预设）、激活并预热使其真正采纳，然后保存状态文件。
    pub fn save_state(spec: &str, params: &serde_json::Map<String, Value>, out: &std::path::Path) -> Result<StateHeader, String> {
        let mut o = Opened::open_spec(spec)?;
        o.apply(params)?;
        let spec_ = manguebyte::prelude::ProcessSpec { sample_rate: 48_000.0, max_block_size: 256, num_channels: 2 };
        let mut proc = o.activate(spec_.sample_rate, spec_.max_block_size)?;
        proc.prepare(&spec_);
        if let Err(w) = o.settle(proc.as_mut(), &spec_) {
            eprintln!("[muser] 警告: {w}");
        }
        o.save_state_file(out)?;
        drop(proc);
        Ok(read_state_file(out)?.0)
    }

    /// 打开插件自己的界面（原生窗口），人在里面选音色、调参数；关窗口时把状态存到 `out`。
    /// 只在 macOS、VST3 插件上；必须在主线程调用。
    #[cfg(target_os = "macos")]
    pub fn edit_state(spec: &str, params: &serde_json::Map<String, Value>, out: &std::path::Path) -> Result<StateHeader, String> {
        use manguebyte::host::editor::{EditorRequest, EditorSize, ParentWindow};
        use manguebyte::host::vst3::HostEvent;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSWindow, NSWindowStyleMask};
        use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};

        let mtm = MainThreadMarker::new().ok_or("插件界面须在主线程打开")?;
        let mut o = Opened::open_spec(spec)?;
        o.apply(params)?;
        let pspec = manguebyte::prelude::ProcessSpec { sample_rate: 48_000.0, max_block_size: 256, num_channels: 2 };
        let mut proc = o.activate(pspec.sample_rate, pspec.max_block_size)?;
        proc.prepare(&pspec);
        let name = o.plugin.entry().name.clone();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        app.finishLaunching();
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
        // SAFETY: 主线程上新建窗口；关窗口后不释放（下面还要读它是否可见）。
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), rect, style, NSBackingStoreType::Buffered, false) };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&format!("{name} — 关闭窗口即保存到 {}", out.display())));
        let view = window.contentView().ok_or("窗口没有内容视图")?;
        let parent = ParentWindow::cocoa(objc2::rc::Retained::as_ptr(&view) as usize).ok_or("无效的视图")?;
        let vst3 = o.plugin.as_vst3_mut().ok_or("目前只支持 VST3 插件的界面")?;
        let size = vst3.open_editor(parent, None).map_err(|e| format!("插件界面打不开: {e:?}"))?;
        window.setContentSize(NSSize::new(f64::from(size.width), f64::from(size.height)));
        window.center();
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let mut silent = manguebyte::prelude::AudioBuffer::<f32>::new(2, 256);
        while window.isVisible() {
            // SAFETY: AppKit 的默认运行循环模式常量。
            let mode = unsafe { NSDefaultRunLoopMode };
            let until = NSDate::dateWithTimeIntervalSinceNow(0.01);
            while let Some(ev) = app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), mode, true) {
                app.sendEvent(&ev);
            }
            // 让插件的音频半边也转起来（有的插件在处理中才读完采样）。
            for ch in 0..2 {
                silent.channel_mut(ch).fill(0.0);
            }
            proc.process(&mut silent);
            let vst3 = o.plugin.as_vst3_mut().expect("上面已检查是 VST3");
            for ev in vst3.pump() {
                if let HostEvent::Editor(EditorRequest::Resize(EditorSize { width, height })) = ev {
                    if let Ok(s) = vst3.set_editor_size(EditorSize::new(width, height)) {
                        window.setContentSize(NSSize::new(f64::from(s.width), f64::from(s.height)));
                    }
                }
            }
        }
        o.save_state_file(out)?;
        o.plugin.as_vst3_mut().expect("上面已检查是 VST3").close_editor();
        drop(proc);
        Ok(read_state_file(out)?.0)
    }

    /// 作用类别与关键词（按优先顺序）。名称里含关键词（大小写不敏感）的参数归入该类。
    const ROLES: &[(&str, &[&str])] = &[
        ("type", &["filter 1 type", "filter type", "fx type", "effect type", "algorithm", "mode"]),
        ("cutoff", &["cutoff", "filter freq", "lpf"]),
        ("resonance", &["filter resonance", "resonance", "reso", "filter q"]),
        ("filter_env", &["feg mod", "filter env", "env amount", "env mod"]),
        ("attack", &["amp eg attack", "attack"]),
        ("decay", &["amp eg decay", "decay"]),
        ("sustain", &["amp eg sustain", "sustain"]),
        ("release", &["amp eg release", "release"]),
        ("osc_type", &["osc 1 type", "waveform", "wave", "shape"]),
        ("detune", &["detune", "spread"]),
        ("unison", &["unison voices", "unison", "voices"]),
        ("glide", &["portamento", "glide"]),
        ("drive", &["drive", "distortion", "saturation"]),
        ("mix", &["mix", "dry/wet", "wet"]),
        ("time", &["delay time", "time"]),
        ("feedback", &["feedback"]),
        ("rate", &["lfo 1 rate", "rate", "speed"]),
        ("depth", &["depth", "amount"]),
        ("width", &["width", "stereo"]),
        ("volume", &["master volume", "global volume", "volume", "level", "gain", "output"]),
    ];

    /// 由插件自动生成画像草稿：按作用关键词挑出常用参数（每类优先名字短、出现早的一个），
    /// 列出插件自己显示的取值。`audition` 为真时对数值类参数做三点试听并写入观察结果（较慢）。
    pub fn draft_profile(spec: &str, audition: bool) -> Result<crate::profile::Profile, String> {
        use crate::profile::*;
        let mut o = Opened::open_spec(spec)?;
        o.profile = None; // 草稿不受已有画像影响
        let e = o.plugin.entry().clone();
        let visible: Vec<Param> = o.params.iter().filter(|p| !p.hidden && !p.read_only && !p.bypass).cloned().collect();
        // 按词匹配：关键词的每个词须依次是参数名中相邻词的开头（"reso" 匹配 "Resonance"，"q" 只匹配单独的 "Q"）。
        fn words_of(s: &str) -> Vec<String> {
            s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
        }
        fn hit(name: &[String], kw: &[String]) -> bool {
            name.windows(kw.len()).any(|win| {
                win.iter().zip(kw).enumerate().all(|(i, (n, k))| {
                    // 单字母关键词必须整词相等；最后一个词允许前缀，其余整词相等
                    if k.len() == 1 || i + 1 < kw.len() { n == k } else { n.starts_with(k.as_str()) }
                })
            })
        }
        const FILTER_ROLES: &[&str] = &["cutoff", "resonance", "filter_env"];
        let mut picked: Vec<(String, Param)> = Vec::new();
        for (role, words) in ROLES {
            let mut best: Option<&Param> = None;
            let score = |q: &Param| {
                let w = words_of(&q.name);
                let ctx = FILTER_ROLES.contains(role) && w.iter().any(|x| x == "filter");
                (std::cmp::Reverse(ctx), q.name.len())
            };
            for kw in words.iter() {
                let kw = words_of(kw);
                for q in &visible {
                    if hit(&words_of(&q.name), &kw) && !picked.iter().any(|(_, x)| x.id == q.id) && best.is_none_or(|b| score(q) < score(b)) {
                        best = Some(q);
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            if let Some(q) = best {
                picked.push((role.to_string(), q.clone()));
            }
        }
        let texts = |q: &Param| -> Vec<String> {
            let n = q.steps.map_or(200, |k| (k as usize + 1) * 4).clamp(16, 800);
            let mut all: Vec<String> = Vec::new();
            for i in 0..n {
                let x = q.min + (q.max - q.min) * i as f64 / (n - 1) as f64;
                if let Some(t) = o.plugin.param_to_text(q.id, x) {
                    if all.last() != Some(&t) {
                        all.push(t);
                    }
                }
            }
            if all.len() <= 40 {
                all.dedup();
                all
            } else {
                // 连续参数：取 5 个刻度
                [0.0, 0.25, 0.5, 0.75, 1.0]
                    .iter()
                    .filter_map(|f| o.plugin.param_to_text(q.id, q.min + (q.max - q.min) * f))
                    .collect()
            }
        };
        let kind = o.info().kind;
        let mut params: Vec<ProfileParam> = Vec::new();
        for (role, q) in &picked {
            let values = texts(q);
            let current = o.plugin.param_to_text(q.id, o.plugin.param_value(q.id).unwrap_or(q.default)).unwrap_or_default();
            params.push(ProfileParam {
                name: q.name.clone(),
                alias: Some(role.clone()),
                role: role.clone(),
                description: format!("自动草稿：名称含 '{role}' 相关关键词；初始值 \"{current}\"。请试听确认作用"),
                values,
                observed: Vec::new(),
            });
        }
        drop(o);
        if audition {
            use crate::audition::{audition as run, AuditionSettings, Subject};
            for pp in params.iter_mut().filter(|p| ["cutoff", "resonance", "attack", "release", "drive", "mix", "filter_env", "decay"].contains(&p.role.as_str())) {
                let values: Vec<Value> = vec![Value::from(0.1), Value::from(0.5), Value::from(0.9)];
                let subject = if kind == "instrument" {
                    Subject::Instrument { name: format!("plugin:{spec}"), params: serde_json::Map::new() }
                } else {
                    let (path, id) = split_spec(spec);
                    let fx = serde_json::json!({ "type": "plugin", "path": path, "id": id });
                    Subject::Effect { effect: serde_json::from_value(fx).map_err(|e| e.to_string())?, source: "synth.poly".into() }
                };
                match run(&subject, Some(&pp.name), &values, &AuditionSettings::default()) {
                    Ok(takes) => pp.observed = observations(spec, &pp.name, &takes),
                    Err(err) => eprintln!("[muser] 试听 {} 失败: {err}", pp.name),
                }
            }
        }
        let user = std::env::var("USER").unwrap_or_default();
        Profile {
            schema_version: SCHEMA_VERSION,
            plugin: ProfilePlugin { name: e.name.clone(), vendor: e.vendor.clone(), ids: vec![e.id.clone()] },
            source: ProfileSource {
                author: user,
                tested_version: e.version.clone(),
                updated: String::new(),
                notes: format!("由 `muser plugin profile draft` 自动生成的草稿（{} 个参数中挑出 {} 个）。请试听后修改说明、补充预设。", visible.len(), params.len()),
                lang: String::new(),
            },
            setup: serde_json::Map::new(),
            params,
            presets: Vec::new(),
            tips: Vec::new(),
            selector: None,
            state_format: String::new(),
        }
        .sanitized()
    }

    /// 把试听结果转成画像里的观察记录（文字取值由插件给出）。
    pub fn observations(spec: &str, param: &str, takes: &[crate::audition::Take]) -> Vec<crate::profile::Observation> {
        let o = Opened::open_spec(spec).ok();
        takes
            .iter()
            .map(|t| {
                let text = o.as_ref().and_then(|o| {
                    let q = o.resolve(param).ok()?;
                    match &t.value {
                        Value::String(s) => Some(s.clone()),
                        v => v.as_f64().and_then(|x| o.plugin.param_to_text(q.id, x)),
                    }
                });
                crate::profile::Observation {
                    value: t.value.clone(),
                    text,
                    lufs: t.analysis.lufs,
                    centroid_hz: t.analysis.centroid_hz,
                    attack_ms: t.analysis.attack_ms,
                    release_s: t.analysis.release_s,
                }
            })
            .collect()
    }

    /// 把一次试听的结论记进用户画像：没有画像则先生成草稿。返回画像路径。
    pub fn record_observations(spec: &str, param: &str, takes: &[crate::audition::Take]) -> Result<std::path::PathBuf, String> {
        use crate::profile::*;
        let o = Opened::open_spec(spec)?;
        let real = o.resolve(param)?.name.clone();
        let id = o.plugin.entry().id.clone();
        let existing = o.profile.clone();
        drop(o);
        let mut profile = match existing {
            Some(l) if l.origin != Origin::Bundled => l.profile.clone(),
            Some(l) => {
                // 内置画像不改：复制一份到用户目录再记。
                let mut p = l.profile.clone();
                p.source.notes = format!("{}（基于 muser 内置画像）", p.source.notes);
                p
            }
            None => draft_profile(spec, false)?,
        };
        let obs = observations(spec, &real, takes);
        match profile.params.iter_mut().find(|p| p.name == real) {
            Some(p) => {
                for ob in obs {
                    p.observed.retain(|x| x.value != ob.value);
                    p.observed.push(ob);
                }
            }
            None => profile.params.push(ProfileParam {
                name: real.clone(),
                alias: None,
                role: String::new(),
                description: String::new(),
                values: Vec::new(),
                observed: obs,
            }),
        }
        if !profile.plugin.ids.contains(&id) {
            profile.plugin.ids.push(id);
        }
        save_user(&profile.sanitized()?)
    }

    /// 用指定画像（而不是已安装/内置的那份）在插件上应用一组参数，检查能否生效。
    pub fn check_params_in(spec: &str, profile: &crate::profile::Profile, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        let mut o = Opened::open_spec(spec)?;
        o.profile = Some(std::sync::Arc::new(crate::profile::Loaded {
            origin: crate::profile::Origin::File,
            path: None,
            profile: profile.clone(),
        }));
        let mut m = params.clone();
        m.insert("@setup".into(), Value::Bool(false));
        o.apply(&m)
    }

    /// （插件 id, 名称, 版本）。
    pub fn plugin_identity(spec: &str) -> Result<(String, String, String), String> {
        let o = Opened::open_spec(spec)?;
        let e = o.plugin.entry();
        Ok((e.id.clone(), e.name.clone(), e.version.clone()))
    }

    /// 校验一组插件参数（会加载插件）。
    pub fn check_params(path: &str, id: Option<&str>, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        Opened::open(path, id)?.apply(params)
    }

    /// 自动化：解析参数，返回（参数 id, 范围, 应用 base_params 后的当前值）以及文本转换器所在的插件。
    pub fn automation_target(
        path: &str,
        id: Option<&str>,
        base_params: &serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<(Opened, ParamId), String> {
        let mut o = Opened::open(path, id)?;
        o.apply(base_params)?;
        let p = o.resolve(key)?;
        if p.read_only {
            return Err(format!("参数 '{}' 是只读的", p.name));
        }
        if !p.automatable {
            return Err(format!("插件声明参数 '{}' 不可自动化", p.name));
        }
        let pid = p.id;
        Ok((o, pid))
    }

    fn cache_path() -> std::path::PathBuf {
        // Linux: $XDG_CACHE_HOME 或 ~/.cache；macOS: ~/Library/Caches；Windows: %LOCALAPPDATA%。
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| if cfg!(windows) { std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from) } else { None })
            .or_else(|| super::home_dir().map(|h| if cfg!(target_os = "macos") { h.join("Library").join("Caches") } else { h.join(".cache") }))
            .unwrap_or_else(std::env::temp_dir);
        base.join("muser").join("plugins.cache")
    }

    /// 扫描本机标准插件目录。每个插件在独立进程里读取，崩溃或卡死的插件只会出现在失败列表里。
    /// 返回（插件列表, 失败列表[(路径, 原因)]）。调用方的 main 必须先调用 [`serve_scanner_if_asked`]。
    pub fn scan(refresh: bool) -> Result<(Vec<ScannedPlugin>, Vec<(String, String)>), String> {
        use manguebyte::host::any::Format;
        use manguebyte::host::scan::{find, standard_paths, ScanCache, Scanner};
        let formats = [Format::Clap, Format::Vst3, Format::Lv2];
        let mut places = Vec::new();
        for f in formats {
            places.extend(standard_paths(f));
        }
        let candidates = find(&places, &formats);
        let cache_file = cache_path();
        let mut cache = if refresh { ScanCache::default() } else { ScanCache::load(&cache_file) };
        let scanner = Scanner::this_program().map_err(|e| format!("无法启动扫描进程: {e}"))?;
        let report = scanner.scan(&candidates, &mut cache);
        if let Some(dir) = cache_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = cache.save(&cache_file);
        let plugins = report
            .plugins
            .into_iter()
            .map(|s| ScannedPlugin {
                path: s.path.display().to_string(),
                id: s.entry.id.clone(),
                name: s.entry.name.clone(),
                vendor: s.entry.vendor.clone(),
                version: s.entry.version.clone(),
                format: format!("{:?}", s.entry.format).to_lowercase(),
                kind: kind_name(s.entry.kind).into(),
                categories: s.entry.categories.clone(),
            })
            .collect();
        let failures = report
            .failures
            .into_iter()
            .map(|f| (f.path.display().to_string(), format!("{:?}", f.error)))
            .collect();
        Ok((plugins, failures))
    }

    /// 程序入口处调用：当本进程被扫描器以子进程方式启动时，读完插件就退出。
    pub fn serve_scanner_if_asked() {
        // SAFETY: 只有以扫描子进程身份启动时才会加载插件，加载完立即退出。
        unsafe { manguebyte::host::scan::serve_if_asked() };
    }
}

/// 非 host 构建下的空实现，便于入口统一调用。
#[cfg(not(feature = "host"))]
pub fn serve_scanner_if_asked() {}
