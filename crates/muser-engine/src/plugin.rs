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

/// 把 `path#id` 拆开。
pub fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.trim().rsplit_once('#') {
        Some((p, i)) if !i.trim().is_empty() => (p.trim(), Some(i.trim())),
        _ => (spec.trim(), None),
    }
}

#[cfg(feature = "host")]
pub use imp::*;

#[cfg(not(feature = "host"))]
pub use stub::*;

#[cfg(not(feature = "host"))]
mod stub {
    use super::*;

    pub const HOST_DISABLED: &str = "插件功能需要 `host` 特性：请用 `cargo build -p muser-cli --features host` 构建";

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
        pub plugin: Plugin,
        pub params: Vec<Param>,
        pub path: String,
        /// 已应用的参数（id, 值），激活后会经音频队列重发一次。
        applied: Vec<(ParamId, f64)>,
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

    impl Opened {
        /// 加载并创建插件。`id` 省略时用文件里的第一个插件。
        pub fn open(path: &str, id: Option<&str>) -> Result<Opened, String> {
            if !std::path::Path::new(path).exists() {
                return Err(format!("找不到插件 '{path}'（用 `muser plugin scan` 查看本机插件）"));
            }
            // SAFETY: 加载会执行第三方代码；路径由用户在工程/命令中给出，视为可信的本机插件。
            let library = unsafe { Library::load(path) }.map_err(|e| format!("加载插件 '{path}' 失败: {e:?}"))?;
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
            Ok(Opened { library, plugin, params, path: path.to_string(), applied: Vec::new() })
        }

        pub fn open_spec(spec: &str) -> Result<Opened, String> {
            let (path, id) = split_spec(spec);
            Opened::open(path, id)
        }

        /// 按名称（大小写不敏感）或数字 id 找参数；找不到时给出相近的名字。
        pub fn resolve(&self, key: &str) -> Result<&Param, String> {
            let k = key.trim();
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
                // 连续参数（如 "200 Hz" 显示为 "200.00 Hz"）：查表无精确匹配时，只有带数字的文字才信任
                // 插件的解析；纯文字又匹配不上任何档位（如 "banana"），视为无法解析。
                _ if s.chars().any(|c| c.is_ascii_digit()) => parsed,
                _ => None,
            }
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

        /// 应用一组参数（激活前调用）。
        pub fn apply(&mut self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
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
        let p = o.resolve(key)?;
        let n = samples.clamp(2, 4096);
        let mut out: Vec<(f64, String)> = Vec::new();
        for i in 0..n {
            let x = p.min + (p.max - p.min) * i as f64 / (n - 1) as f64;
            if let Some(t) = o.plugin.param_to_text(p.id, x) {
                if out.last().map_or(true, |(_, last)| *last != t) {
                    out.push((x, t));
                }
            }
        }
        Ok((o.describe_param(p), out))
    }

    /// 校验一组插件参数（会加载插件）。
    pub fn check_params(path: &str, id: Option<&str>, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        if params.is_empty() {
            return Ok(());
        }
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
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
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
