//! SFZ 控制器（MIDI CC）与键位切换。
//!
//! 上游采样器实时处理按控制器的调制（`<opcode>_onccN`：音量、振幅、声像、音高、滤波、包络各级），
//! 并按 `set_ccN` 给出初始值。muser 再做两件它不做的事，都在加载时按控制器的值**静态展开**：
//!
//! - `loccN` / `hiccN`（及 `lohdccN` / `hihdccN`，0–1）不满足的区域去掉；CC 交叉淡化（`xfin_loccN` 等）折算成音量；
//! - 键位切换（`sw_last` / `sw_default` / `sw_label`）：只保留选中奏法的区域。默认 `sw_default`，
//!   工程里 `instrument_params.keyswitch` 可选（音高数字、音名如 `"c1"`，或 `sw_label` 名称）。
//!
//! 控制器的值 = 工程里 `instrument_params.cc` 给出的值，否则是 .sfz 的 `set_ccN`，否则 0；
//! 工程里给的值在乐器载入时设给采样器，调制按它实时生效。例外是 `amplitude_onccN`：按 SFZ 的惯例是乘法，
//! 上游当成加法，所以 muser 在加载时按控制器值折成固定振幅（不随 CC 实时变化）。

use std::collections::BTreeMap;
use std::path::Path;

use manguebyte::sampler::{FileSource, Instrument, Opcode, Sfz, WarningKind};

/// .sfz 里声明的控制器（`label_ccN`、`set_ccN` / `set_hdccN`）：编号 → (名称, 默认值 0–127)。
pub fn controls(sfz: &Sfz) -> BTreeMap<u8, (String, u8)> {
    let mut out: BTreeMap<u8, (String, u8)> = BTreeMap::new();
    for (n, name) in &sfz.control.label_cc {
        if let Ok(n) = u8::try_from(*n) {
            out.entry(n).or_insert((String::new(), 0)).0 = name.trim().to_string();
        }
    }
    for (n, v) in &sfz.control.set_cc {
        if let Ok(n) = u8::try_from(*n) {
            out.entry(n).or_insert((String::new(), 0)).1 = (v.clamp(0.0, 1.0) * 127.0).round() as u8;
        }
    }
    out.retain(|n, _| *n < 128);
    out
}

/// `$MUSER_SFZ_PATH` 里的 .sfz 文件（根下最多 5 层，最多 500 个，按路径排序）：(相对根的路径, 文件)。
/// 相对路径就是工程里写的 `sfz:<路径>`。
pub fn library() -> Vec<(String, std::path::PathBuf)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(it) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = it.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if out.len() >= 500 {
                return;
            }
            if p.is_dir() && depth < 5 {
                walk(root, &p, depth + 1, out);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sfz")) {
                if let Ok(rel) = p.strip_prefix(root) {
                    out.push((rel.to_string_lossy().replace('\\', "/"), p.clone()));
                }
            }
        }
    }
    let roots = std::env::var_os("MUSER_SFZ_PATH").map(|v| std::env::split_paths(&v).collect::<Vec<_>>()).unwrap_or_default();
    let mut out = Vec::new();
    for r in roots.iter().filter(|r| !r.as_os_str().is_empty()) {
        walk(r, r, 0, &mut out);
    }
    out
}

/// `sfz:` 后的路径 → 实际文件。绝对路径或相对当前目录已存在的原样使用；否则在 `$MUSER_SFZ_PATH`
/// （路径分隔符分隔的音色库根目录）里找：`根/路径`，以及根下两层以内的子目录 `根/a/路径`、`根/a/b/路径`，
/// 这样同一份工程在音色库目录层级不同的机器上也能用。都找不到时原样返回（由调用方报"找不到"）。
pub fn resolve(raw: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(raw);
    if path.is_absolute() || path.is_file() {
        return path;
    }
    let roots = std::env::var_os("MUSER_SFZ_PATH").map(|v| std::env::split_paths(&v).collect::<Vec<_>>()).unwrap_or_default();
    let mut level: Vec<std::path::PathBuf> = roots.into_iter().filter(|r| !r.as_os_str().is_empty()).collect();
    for depth in 0..3 {
        if let Some(found) = level.iter().map(|d| d.join(&path)).find(|p| p.is_file()) {
            return found;
        }
        if depth < 2 {
            let mut subdirs: Vec<_> = level
                .iter()
                .filter_map(|d| std::fs::read_dir(d).ok())
                .flat_map(|it| it.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>())
                .collect();
            subdirs.sort(); // 多处命中时结果确定
            level = subdirs;
        }
    }
    path
}

fn parse(path: &Path) -> Result<(Sfz, MuserSource), String> {
    let text = std::fs::read(path).map_err(|e| format!("读取 SFZ '{}' 失败: {e}", path.display()))?;
    let mut source = MuserSource(FileSource::new(path.parent().unwrap_or(Path::new(""))));
    let text = normalize(&String::from_utf8_lossy(&text));
    let sfz = Sfz::parse_with(&text, &mut source).map_err(|e| format!("解析 SFZ '{}' 失败: {e}", path.display()))?;
    Ok((sfz, source))
}

/// `sample=*noise`（白噪声发生器，上游采样器没有）换成的虚拟文件：读到时返回内存里生成的噪声。
const NOISE_FILE: &str = "__muser_noise__.wav";

/// 读文件时补上采样器不认的两种写法（见 [`normalize`]），并提供 [`NOISE_FILE`]。
pub struct MuserSource(FileSource);

impl manguebyte::sampler::SampleSource for MuserSource {
    fn read(&mut self, path: &str) -> std::io::Result<Vec<u8>> {
        if path.rsplit(['/', '\\']).next() == Some(NOISE_FILE) {
            return Ok(noise_wav());
        }
        let bytes = self.0.read(path)?;
        // 被 #include 的 .sfz 也要同样处理。
        Ok(if path.to_ascii_lowercase().ends_with(".sfz") { normalize(&String::from_utf8_lossy(&bytes)).into_bytes() } else { bytes })
    }
}

/// .sfz 文本里采样器不认的写法：
/// - 写在别的内容后面、同一行里的 `#include` / `#define`（如 `<group> #include "x.sfz"`）挪到新的一行；
/// - `sample=*noise` 换成 [`NOISE_FILE`]。
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.lines() {
        // 注释（`//` 之后）里的不动；位置来自 match_indices，都在字符边界上。
        let code = line.split("//").next().unwrap_or("").len();
        let mut cuts: Vec<usize> = ["#include", "#define"]
            .iter()
            .flat_map(|d| line.match_indices(d).map(|(i, _)| i))
            .filter(|&i| i < code && !line[..i].trim().is_empty())
            .collect();
        cuts.sort_unstable();
        let mut last = 0;
        for i in cuts {
            out.push_str(line[last..i].trim_end());
            out.push('\n');
            last = i;
        }
        let rest = &line[last..];
        out.push_str(&rest.replace("*noise", NOISE_FILE));
        out.push('\n');
    }
    out
}

/// 10 秒的白噪声，48 kHz 单声道 16 位，带一个整段循环（`smpl`），按多久响多久。
fn noise_wav() -> Vec<u8> {
    const SR: u32 = 48_000;
    let n = SR * 10;
    let mut x: u32 = 0x9E37_79B9;
    let mut data = Vec::with_capacity(n as usize * 2);
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let v = ((x >> 16) as i16 as f32 * 0.5) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut smpl = Vec::new();
    for v in [0u32, 0, 1_000_000_000 / SR, 60, 0, 0, 0, 1, 0, 0, 0, 0, n - 1, 0, 0] {
        smpl.extend_from_slice(&v.to_le_bytes());
    }
    let mut w = Vec::new();
    let chunk = |w: &mut Vec<u8>, id: &[u8], body: &[u8]| {
        w.extend_from_slice(id);
        w.extend_from_slice(&(body.len() as u32).to_le_bytes());
        w.extend_from_slice(body);
    };
    let mut fmt = Vec::new();
    for (v, size) in [(1u32, 2), (1, 2), (SR, 4), (SR * 2, 4), (2, 2), (16, 2)] {
        fmt.extend_from_slice(&v.to_le_bytes()[..size]);
    }
    let mut body = b"WAVE".to_vec();
    chunk(&mut body, b"fmt ", &fmt);
    chunk(&mut body, b"smpl", &smpl);
    chunk(&mut body, b"data", &data);
    chunk(&mut w, b"RIFF", &body);
    w
}

/// 读出 .sfz 声明的控制器（给 `muser instruments sfz:…` 显示）。
pub fn read_controls(path: &Path) -> Result<BTreeMap<u8, (String, u8)>, String> {
    Ok(controls(&parse(path)?.0))
}

/// SFZ 音名（`c4` = 60，`c#4`、`db4`）或数字 → MIDI 音高。
pub fn note_number(v: &str) -> Option<u8> {
    let v = v.trim().to_ascii_lowercase();
    if let Ok(n) = v.parse::<i32>() {
        return u8::try_from(n).ok().filter(|n| *n < 128);
    }
    let mut c = v.chars();
    let base = match c.next()? { 'c' => 0, 'd' => 2, 'e' => 4, 'f' => 5, 'g' => 7, 'a' => 9, 'b' => 11, _ => return None };
    let rest: String = c.collect();
    let (acc, oct) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') if rest.len() > 1 => (-1, &rest[1..]),
        _ => (0, &rest[..]),
    };
    let n = base + acc + 12 * (oct.parse::<i32>().ok()? + 1);
    u8::try_from(n).ok().filter(|n| *n < 128)
}

/// .sfz 里的键位切换奏法：键 → 名称（`sw_label`），以及默认键（`sw_default`）。
pub fn keyswitches(sfz: &Sfz) -> (BTreeMap<u8, String>, Option<u8>) {
    let mut out = BTreeMap::new();
    let mut default = None;
    for r in &sfz.regions {
        let get = |n: &str| r.unknown.iter().find(|o| &*o.name == n).map(|o| o.value.to_string());
        if let Some(k) = get("sw_last").and_then(|v| note_number(&v)) {
            let label = get("sw_label").unwrap_or_default();
            let e = out.entry(k).or_insert_with(String::new);
            if e.is_empty() {
                *e = label;
            }
        }
        if default.is_none() {
            default = get("sw_default").and_then(|v| note_number(&v));
        }
    }
    (out, default)
}

/// 读出 .sfz 的键位切换奏法。
pub fn read_keyswitches(path: &Path) -> Result<(BTreeMap<u8, String>, Option<u8>), String> {
    Ok(keyswitches(&parse(path)?.0))
}

/// 工程里的 sfz 选项。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    pub cc: BTreeMap<u8, u8>,
    /// 键位切换：音高数字/音名，或 `sw_label` 名称（加载时解析）。
    pub keyswitch: Option<String>,
}

/// 校验 `instrument_params`：`cc`（`{ "编号": 0–127 }`）与 `keyswitch`。
pub fn parse_params(params: &serde_json::Map<String, serde_json::Value>) -> Result<Options, String> {
    let mut opts = Options::default();
    for (k, v) in params {
        if k == "keyswitch" {
            opts.keyswitch = Some(match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => return Err("keyswitch 应为音高（数字或音名，如 \"c1\"）或奏法名称".into()),
            });
            continue;
        }
        if k != "cc" {
            return Err(format!("sfz 乐器的 instrument_params 只支持 \"cc\"（控制器值，如 {{ \"cc\": {{ \"105\": 127 }} }}）与 \"keyswitch\"，不认识 '{k}'"));
        }
        let obj = v.as_object().ok_or("cc 应为对象：{ \"控制器编号\": 0–127 }")?;
        for (n, val) in obj {
            let cc: u8 = n.trim().parse().ok().filter(|c| *c < 128).ok_or_else(|| format!("cc.{n}: 控制器编号应为 0–127"))?;
            let x = val.as_f64().filter(|x| (0.0..=127.0).contains(x)).ok_or_else(|| format!("cc.{n} 应为 0–127 的数字"))?;
            opts.cc.insert(cc, x.round() as u8);
        }
    }
    Ok(opts)
}

fn cc_num(name: &str, prefix: &str) -> Option<u8> {
    name.strip_prefix(prefix).and_then(|n| n.parse::<u8>().ok()).filter(|n| *n < 128)
}

/// 加载好的音色（采样数据在里面，克隆只复制引用）。
pub struct Loaded {
    pub instrument: Instrument,
    /// 按控制器去掉的区域数。
    pub dropped: usize,
    /// 不支持的内容：opcode 名（或 `<header>`、`取值越界: opcode`）→ 出现次数。
    pub warnings: BTreeMap<String, usize>,
    /// 工程里给的控制器值（0–127），采样器开始演奏前设上。
    pub cc: BTreeMap<u8, u8>,
}

/// 不支持内容的一行摘要，按次数从多到少，最多列 12 项。
pub fn summary(w: &BTreeMap<String, usize>) -> String {
    let mut v: Vec<(&String, &usize)> = w.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut s: Vec<String> = v.iter().take(12).map(|(k, n)| format!("{k} ×{n}")).collect();
    if v.len() > 12 {
        s.push(format!("…另 {} 项", v.len() - 12));
    }
    s.join("、")
}

type Slot = std::sync::Arc<std::sync::Mutex<std::sync::Weak<Loaded>>>;
static POOL: std::sync::Mutex<BTreeMap<String, Slot>> = std::sync::Mutex::new(BTreeMap::new());

/// 同 [`load`]，但同一份音色（同一 .sfz、文件没改、同样的控制器与奏法）在进程里只加载一次：
/// 声卡播放、后台渲染、跳转重建都共用同一份采样数据。只要还有人拿着返回的 `Arc` 就一直留着，都放下后释放。
/// 多处同时要同一份时，只有一处在加载，其余等它。
pub fn load_shared(path: &Path, opts: &Options) -> Result<std::sync::Arc<Loaded>, String> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = format!("{}|{mtime:?}|{:?}|{:?}", path.display(), opts.cc, opts.keyswitch);
    let slot = {
        let mut pool = POOL.lock().unwrap();
        pool.retain(|_, s| std::sync::Arc::strong_count(s) > 1 || s.lock().is_ok_and(|w| w.strong_count() > 0));
        std::sync::Arc::clone(pool.entry(key).or_default())
    };
    let mut held = slot.lock().unwrap();
    if let Some(l) = held.upgrade() {
        return Ok(l);
    }
    let (instrument, dropped, warnings) = load(path, opts)?;
    let l = std::sync::Arc::new(Loaded { instrument, dropped, warnings, cc: opts.cc.clone() });
    *held = std::sync::Arc::downgrade(&l);
    Ok(l)
}

/// 力度交叉淡化（`xfin_lovel` / `xfin_hivel` / `xfout_lovel` / `xfout_hivel`，`xf_velcurve`）与力度曲线
/// （`amp_velcurve_N`）上游采样器不演奏：把这样的区域按力度每 8 级切成一段，每段乘上那段中点的增益
/// （曲线换成阶梯，段内不再变化）；增益为 0 的段去掉。力度曲线取代 `amp_veltrack`。
fn split_by_velocity(regions: &mut Vec<manguebyte::sampler::Region>, handled: &mut std::collections::HashSet<String>) {
    let mut out = Vec::with_capacity(regions.len());
    for mut r in std::mem::take(regions) {
        let get = |name: &str| r.unknown.iter().rev().find(|o| &*o.name == name).and_then(|o| o.value.trim().parse::<f64>().ok());
        let (xin, xout) = ((get("xfin_lovel"), get("xfin_hivel")), (get("xfout_lovel"), get("xfout_hivel")));
        let mut curve: Vec<(f64, f64)> = r
            .unknown
            .iter()
            .filter_map(|o| o.name.strip_prefix("amp_velcurve_").and_then(|n| n.parse::<f64>().ok()).zip(o.value.trim().parse::<f64>().ok()))
            .filter(|(n, _)| (0.0..=127.0).contains(n))
            .collect();
        let crossfades = xin.0.is_some() || xin.1.is_some() || xout.0.is_some() || xout.1.is_some();
        if !crossfades && curve.is_empty() {
            out.push(r);
            continue;
        }
        let gain_curve = r.unknown.iter().any(|o| &*o.name == "xf_velcurve" && o.value.trim() == "gain");
        let ramp = |v: f64, (a, b): (Option<f64>, Option<f64>), fade_in: bool| -> f64 {
            let (a, b) = if fade_in { (a.unwrap_or(0.0), b.unwrap_or(0.0)) } else { (a.unwrap_or(127.0), b.unwrap_or(127.0)) };
            let t = if b <= a { if v >= b { 1.0 } else { 0.0 } } else { ((v - a) / (b - a)).clamp(0.0, 1.0) };
            if fade_in { t } else { 1.0 - t }
        };
        if !curve.is_empty() {
            // 没写的两端：0 级为 0、127 级为 1。
            if !curve.iter().any(|p| p.0 == 0.0) {
                curve.push((0.0, 0.0));
            }
            if !curve.iter().any(|p| p.0 == 127.0) {
                curve.push((127.0, 1.0));
            }
            curve.sort_by(|a, b| a.0.total_cmp(&b.0));
            r.amp_veltrack = 0.0;
        }
        let along = |v: f64| -> f64 {
            let i = curve.partition_point(|p| p.0 <= v).clamp(1, curve.len() - 1);
            let (a, b) = (curve[i - 1], curve[i]);
            if b.0 <= a.0 { b.1 } else { a.1 + (b.1 - a.1) * (v - a.0) / (b.0 - a.0) }
        };
        let gain = |v: f64| -> f64 {
            let mut g = 1.0;
            if crossfades {
                let w = ramp(v, xin, true) * ramp(v, xout, false);
                g *= if gain_curve { w } else { w.sqrt() };
            }
            if !curve.is_empty() {
                g *= along(v).max(0.0);
            }
            g
        };
        r.unknown.retain(|o| {
            let n = &*o.name;
            let ours = matches!(n, "xfin_lovel" | "xfin_hivel" | "xfout_lovel" | "xfout_hivel" | "xf_velcurve") || n.starts_with("amp_velcurve_");
            if ours {
                handled.insert(n.to_string());
            }
            !ours
        });
        let (lo, hi) = (r.lovel, r.hivel);
        let mut a = lo;
        while a <= hi {
            let b = a.saturating_add(7).min(hi);
            let g = gain((f64::from(a) + f64::from(b)) / 2.0);
            if g > 1e-4 {
                let mut piece = r.clone();
                (piece.lovel, piece.hivel) = (a, b);
                piece.volume += 20.0 * g.log10();
                out.push(piece);
            }
            if b == 127 {
                break;
            }
            a = b + 1;
        }
    }
    *regions = out;
}

/// 按控制器值展开后加载乐器。返回 (乐器, 去掉的区域数, 不支持的内容)。
pub fn load(path: &Path, opts: &Options) -> Result<(Instrument, usize, BTreeMap<String, usize>), String> {
    let overrides = &opts.cc;
    let (mut sfz, mut source) = parse(path)?;
    // 键位切换：选定奏法，去掉其它奏法的区域。
    let (switches, sw_default) = keyswitches(&sfz);
    let chosen: Option<u8> = match &opts.keyswitch {
        Some(k) => Some(
            note_number(k)
                .or_else(|| switches.iter().find(|(_, l)| l.eq_ignore_ascii_case(k.trim())).map(|(n, _)| *n))
                .filter(|n| switches.contains_key(n))
                .ok_or_else(|| {
                    format!(
                        "keyswitch '{k}' 不是这个音色的奏法，可用: {}",
                        switches.iter().map(|(n, l)| if l.is_empty() { n.to_string() } else { format!("{n}（{l}）") }).collect::<Vec<_>>().join("，")
                    )
                })?,
        ),
        None => sw_default.or_else(|| switches.keys().next().copied()),
    };
    if let Some(sel) = chosen {
        sfz.regions.retain(|r| match r.unknown.iter().find(|o| &*o.name == "sw_last") {
            Some(o) => note_number(&o.value) == Some(sel),
            None => true,
        });
    }
    let switched = !switches.is_empty();
    let mut value: BTreeMap<u8, f64> = controls(&sfz).into_iter().map(|(n, (_, v))| (n, f64::from(v))).collect();
    for (n, v) in overrides {
        value.insert(*n, f64::from(*v));
    }
    let cc = |n: u8| value.get(&n).copied().unwrap_or(0.0);
    let before = sfz.regions.len();
    // 上游把 `amplitude_onccN` 的深度截在 ±100%，但音色库常写 200%（配 0.5 的默认控制器值 = 原音量，如 Ixox Flute）：
    // 从越界警告里取回写的值（同名 opcode 的写法都一样时）。
    let mut written: BTreeMap<String, Option<f64>> = BTreeMap::new();
    for w in &sfz.warnings {
        if let WarningKind::OutOfRange { opcode, value, .. } = &w.kind {
            if opcode.starts_with("amplitude_") {
                let v = value.trim().parse::<f64>().ok();
                let e = written.entry(opcode.to_string()).or_insert(v);
                if *e != v {
                    *e = None;
                }
            }
        }
    }
    let amplitude_depth = |controller: u16, depth: f64| -> f64 {
        ["amplitude_oncc", "amplitude_cc"]
            .iter()
            .find_map(|p| written.get(&format!("{p}{controller}")).copied().flatten())
            .filter(|_| depth.abs() >= 100.0)
            .unwrap_or(depth)
    };
    let mut handled = std::collections::HashSet::<String>::new();
    sfz.regions.retain_mut(|r| {
        let (mut lo, mut hi): (BTreeMap<u8, f64>, BTreeMap<u8, f64>) = (BTreeMap::new(), BTreeMap::new());
        // 控制器交叉淡化：cc → (lo, hi)，淡入 / 淡出。
        let (mut xin, mut xout): (BTreeMap<u8, (f64, f64)>, BTreeMap<u8, (f64, f64)>) = (BTreeMap::new(), BTreeMap::new());
        let mut power = false;
        let mut keep: Vec<Opcode> = Vec::new();
        let num = |o: &Opcode| o.value.trim().parse::<f64>().ok();
        for o in std::mem::take(&mut r.unknown) {
            let n = &*o.name;
            let mut used = true;
            if let Some(c) = cc_num(n, "locc") {
                lo.insert(c, num(&o).unwrap_or(0.0));
            } else if let Some(c) = cc_num(n, "hicc") {
                hi.insert(c, num(&o).unwrap_or(127.0));
            } else if let Some(c) = cc_num(n, "lohdcc") {
                lo.insert(c, num(&o).unwrap_or(0.0) * 127.0);
            } else if let Some(c) = cc_num(n, "hihdcc") {
                hi.insert(c, num(&o).unwrap_or(1.0) * 127.0);
            } else if let Some(c) = cc_num(n, "xfin_locc") {
                xin.entry(c).or_insert((0.0, 127.0)).0 = num(&o).unwrap_or(0.0);
            } else if let Some(c) = cc_num(n, "xfin_hicc") {
                xin.entry(c).or_insert((0.0, 127.0)).1 = num(&o).unwrap_or(127.0);
            } else if let Some(c) = cc_num(n, "xfout_locc") {
                xout.entry(c).or_insert((0.0, 127.0)).0 = num(&o).unwrap_or(0.0);
            } else if let Some(c) = cc_num(n, "xfout_hicc") {
                xout.entry(c).or_insert((0.0, 127.0)).1 = num(&o).unwrap_or(127.0);
            } else if n == "xf_cccurve" {
                power = o.value.trim() == "power";
            } else if n == "group_volume" || n == "master_volume" || n == "global_volume" {
                r.volume += num(&o).unwrap_or(0.0);
            } else if switched && matches!(n, "sw_last" | "sw_label" | "sw_default" | "sw_lokey" | "sw_hikey") {
                // 已在上面按键位切换选好奏法。
            } else {
                used = false;
            }
            if used {
                handled.insert(n.to_string());
            } else {
                keep.push(o);
            }
        }
        r.unknown = keep;
        // `amplitude_onccN` 按 SFZ 的惯例（ARIA、sfizz）是乘法：振幅 × 深度% × 控制器/127（Karoryfer 用它让 mod wheel
        // 推到 0 时无声）。上游采样器当成加法（100% 加上去），会响出好几 dB：这里按当前控制器值折成固定振幅，
        // 不交给它（这一项不再随 CC 实时变化；其它调制照常实时）。
        r.modulations.retain(|m| {
            if m.target != manguebyte::sampler::ModulationTarget::Amplitude {
                return true;
            }
            let v = u8::try_from(m.controller).map_or(0.0, cc) / 127.0;
            r.amplitude *= (amplitude_depth(m.controller, m.depth) / 100.0 * v).max(0.0);
            false
        });
        // 交叉淡化折算成音量（gain 曲线线性，power 曲线取平方根，保持功率）。
        let ramp = |v: f64, (a, b): (f64, f64)| if b <= a { if v >= b { 1.0 } else { 0.0 } } else { ((v - a) / (b - a)).clamp(0.0, 1.0) };
        let mut w = 1.0;
        for (c, range) in &xin {
            w *= ramp(cc(*c), *range);
        }
        for (c, range) in &xout {
            w *= 1.0 - ramp(cc(*c), *range);
        }
        if power {
            w = w.sqrt();
        }
        if w < 1.0 {
            if w <= 1e-4 {
                return false;
            }
            r.volume += 20.0 * w.log10();
        }
        lo.keys().chain(hi.keys()).all(|c| {
            let v = cc(*c);
            v >= lo.get(c).copied().unwrap_or(0.0) && v <= hi.get(c).copied().unwrap_or(127.0)
        })
    });
    let dropped = before - sfz.regions.len();
    split_by_velocity(&mut sfz.regions, &mut handled);
    // 只报留下来、会发声的区域里还剩的 opcode：已经处理掉的、控制器声明、只出现在被去掉的区域
    // （别的奏法、控制器范围外）里的都不算"不支持"。
    let remaining: std::collections::HashSet<&str> = sfz.regions.iter().flat_map(|r| r.unknown.iter().map(|o| &*o.name)).collect();
    let remaining: std::collections::HashSet<String> = remaining.into_iter().map(str::to_string).collect();
    sfz.warnings.retain(|w| match &w.kind {
        // `region_label` 等是给人看的名字，不是"不支持"。
        WarningKind::UnknownOpcode { name, .. } => remaining.contains(&**name) && !handled.contains(&**name) && !name.ends_with("_label"),
        // 已从这条警告取回写的值（见上）。
        WarningKind::OutOfRange { opcode, .. } => !written.get(&**opcode).is_some_and(Option::is_some),
        _ => true,
    });
    let mut warnings: BTreeMap<String, usize> = BTreeMap::new();
    for w in &sfz.warnings {
        let k = match &w.kind {
            WarningKind::UnknownOpcode { name, .. } => name.to_string(),
            WarningKind::UnsupportedHeader(h) => format!("<{h}>"),
            WarningKind::OutOfRange { opcode, .. } => format!("取值越界: {opcode}"),
            WarningKind::OpcodeOutsideHeader(o) => format!("header 之外: {o}"),
            WarningKind::RegionWithoutSample => "没有 sample 的 region".into(),
            _ => "其它".into(),
        };
        *warnings.entry(k).or_default() += 1;
    }
    let inst = Instrument::load(&sfz, &mut source).map_err(|e| format!("加载 SFZ '{}' 失败: {e}", path.display()))?;
    Ok((inst, dropped, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_what_the_sampler_does_not_read() {
        // 同一行的 #include 挪到新行；注释里的、中文后面的不出错。
        assert_eq!(normalize("<group> lokey=1 #include \"a.sfz\""), "<group> lokey=1\n#include \"a.sfz\"\n");
        assert_eq!(normalize("// 注释 #include \"x\""), "// 注释 #include \"x\"\n");
        assert_eq!(normalize("<region> sample=*noise"), format!("<region> sample={NOISE_FILE}\n"));
        let wav = noise_wav();
        assert_eq!(&wav[..4], b"RIFF");
        // 力度交叉淡化：40–90 之间淡出的层，切成段后 30 级是原音量、65 级约 -3 dB、100 级没有。
        let dir = std::env::temp_dir().join(format!("muser-sfzxf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.sfz");
        std::fs::write(&path, "<region> sample=*sine xfout_lovel=40 xfout_hivel=90\n<region> sample=*noise amp_velcurve_64=0.5").unwrap();
        let (mut sfz, _) = parse(&path).unwrap();
        let mut handled = std::collections::HashSet::new();
        split_by_velocity(&mut sfz.regions, &mut handled);
        let first: Vec<_> = sfz.regions.iter().filter(|r| &*r.sample == "*sine").collect();
        let at = |v: u8| first.iter().find(|r| (r.lovel..=r.hivel).contains(&v)).map(|r| r.volume);
        assert_eq!(at(30), Some(0.0));
        assert!((at(65).unwrap() + 3.0).abs() < 1.5, "{:?}", at(65));
        assert_eq!(at(100), None);
        assert!(first.iter().all(|r| r.unknown.is_empty()));
        let noise: Vec<_> = sfz.regions.iter().filter(|r| r.sample.ends_with(NOISE_FILE)).collect();
        assert!(noise.iter().all(|r| r.amp_veltrack == 0.0) && noise.len() > 10);
        assert!(load(&path, &Options::default()).is_ok(), "噪声区域能载入");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cc_selects_regions_and_scales() {
        let dir = std::env::temp_dir().join(format!("muser-sfzcc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sfz");
        std::fs::write(
            &path,
            "<control> label_cc105=Mono set_cc105=0 label_cc100=Unison
             <region> sample=*sine lokey=0 hikey=127 locc105=0 hicc105=63
             <region> sample=*sine lokey=0 hikey=127 locc105=64 hicc105=127
             <region> sample=*sine lokey=0 hikey=127 amplitude=0 amplitude_oncc100=100 pan_oncc100=50",
        )
        .unwrap();
        let ctl = read_controls(&path).unwrap();
        assert_eq!(ctl[&105], ("Mono".to_string(), 0));
        assert_eq!(ctl[&100].0, "Unison");
        let (inst, dropped, warnings) = load(&path, &Options::default()).unwrap();
        assert_eq!((inst.num_regions(), dropped, warnings.len()), (2, 1, 0));
        let (_, dropped, _) = load(&path, &Options { cc: BTreeMap::from([(105, 127)]), keyswitch: None }).unwrap();
        assert_eq!(dropped, 1);
        assert!(parse_params(&serde_json::from_str(r#"{ "cc": { "105": 127 } }"#).unwrap()).is_ok());
        assert!(parse_params(&serde_json::from_str(r#"{ "cc": { "200": 1 } }"#).unwrap()).is_err());
        assert!(parse_params(&serde_json::from_str(r#"{ "cutoff": 1 }"#).unwrap()).unwrap_err().contains("cc"));
        let ks = dir.join("ks.sfz");
        std::fs::write(
            &ks,
            "<group> sw_lokey=c0 sw_hikey=d0 sw_default=c0
             <region> sample=*sine lokey=30 hikey=90 sw_last=c0 sw_label=Sustain
             <region> sample=*sine lokey=30 hikey=90 sw_last=d0 sw_label=Staccato",
        )
        .unwrap();
        assert_eq!(read_keyswitches(&ks).unwrap(), (BTreeMap::from([(12, "Sustain".into()), (14, "Staccato".into())]), Some(12)));
        let (i, _, w) = load(&ks, &Options::default()).unwrap();
        assert_eq!((i.num_regions(), w.len()), (1, 0));
        let (i, _, _) = load(&ks, &Options { keyswitch: Some("staccato".into()), ..Default::default() }).unwrap();
        assert_eq!(i.num_regions(), 1);
        assert!(load(&ks, &Options { keyswitch: Some("e5".into()), ..Default::default() }).unwrap_err().contains("Staccato"));
        assert_eq!(note_number("c4"), Some(60));
        assert_eq!(note_number("C#-1"), Some(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shared_load_reuses_samples() {
        let dir = std::env::temp_dir().join(format!("muser-sfzpool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.sfz");
        std::fs::write(&path, "<region> sample=*sine lokey=0 hikey=127").unwrap();
        let a = load_shared(&path, &Options::default()).unwrap();
        let b = load_shared(&path, &Options::default()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b), "同一份音色只加载一次");
        let c = load_shared(&path, &Options { cc: BTreeMap::from([(1, 64)]), keyswitch: None }).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&a, &c), "控制器不同是另一份");
        drop((a, b));
        let d = load_shared(&path, &Options::default()).unwrap();
        assert_eq!(d.instrument.num_regions(), 1, "都放下后重新加载");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_searches_library_roots() {
        let root = std::env::temp_dir().join(format!("muser-sfzroot-{}", std::process::id()));
        let deep = root.join("vendor/pack/Lib");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("p.sfz"), "<region> sample=*sine").unwrap();
        // 只有这个测试读写 MUSER_SFZ_PATH。
        std::env::set_var("MUSER_SFZ_PATH", std::env::join_paths(["/nonexistent".into(), root.clone()]).unwrap());
        assert_eq!(resolve("Lib/p.sfz"), deep.join("p.sfz"), "根下两层子目录");
        assert_eq!(resolve("pack/Lib/p.sfz"), deep.join("p.sfz"), "根下一层子目录");
        assert_eq!(resolve("missing/p.sfz"), std::path::PathBuf::from("missing/p.sfz"), "找不到时原样返回");
        assert_eq!(resolve("/abs/p.sfz"), std::path::PathBuf::from("/abs/p.sfz"));
        std::env::remove_var("MUSER_SFZ_PATH");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn note_bend_moves_only_its_own_note() {
        let dir = std::env::temp_dir().join(format!("muser-sfzbend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.sfz");
        std::fs::write(&path, "<region> sample=*sine").unwrap();
        // 60 bpm：推弦 0–0.5 s 推上两个半音并保持；之后一个不弯的音不受影响。
        let p: muser_core::Project = serde_json::from_value(serde_json::json!({ "tempo": 60, "tracks": [{ "instrument": format!("sfz:{}", path.display()),
            "clips": [{ "start": 0, "length": 3, "notes": [{ "pitch": 57, "start": 0, "length": 1.0, "bend": [[0, 0], [0.5, 2]] }, { "pitch": 57, "start": 1.5, "length": 1.0 }] }] }] })).unwrap();
        let a = crate::render(&p, crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap();
        let freq = |from: f64| {
            let x = &a.channel(0)[(from * 48_000.0) as usize..((from + 0.4) * 48_000.0) as usize];
            x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 / 0.4
        };
        let (bent, plain) = (freq(0.55), freq(1.55));
        assert!((bent / 220.0 - 2f64.powf(2.0 / 12.0)).abs() < 0.02, "推弦后 {bent}");
        assert!((plain / 220.0 - 1.0).abs() < 0.02, "不弯的音 {plain}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legato_trigger_uses_legato_regions() {
        // first 区域是 1 号正弦音高，legato 区域移高一个八度：重叠的第二个音应走 legato。
        let dir = std::env::temp_dir().join(format!("muser-sfzleg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("l.sfz");
        std::fs::write(&path, "<region> sample=*sine trigger=first\n<region> sample=*sine trigger=legato transpose=12").unwrap();
        let render = |second_start: f64| {
            let p: muser_core::Project = serde_json::from_value(serde_json::json!({ "tempo": 60, "tracks": [{ "instrument": format!("sfz:{}", path.display()),
                "clips": [{ "start": 0, "length": 3, "notes": [{ "pitch": 57, "start": 0, "length": 1.0 }, { "pitch": 57 + 2, "start": second_start, "length": 1.0 }] }] }] })).unwrap();
            crate::render(&p, crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap()
        };
        // 第二个音 1.5–1.9 s 的过零率估频率：连奏约 2× 音高。
        let freq = |a: &manguebyte::prelude::AudioBuffer<f32>| {
            let x = &a.channel(0)[72_000..91_200];
            x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 / 0.4
        };
        let detached = freq(&render(1.1));
        // clip 的 legato 字段：不重叠的写法也自动连起来。
        let p: muser_core::Project = serde_json::from_value(serde_json::json!({ "tempo": 60, "tracks": [{ "instrument": format!("sfz:{}", path.display()),
            "clips": [{ "start": 0, "length": 3, "legato": true, "notes": [{ "pitch": 57, "start": 0, "length": 0.9 }, { "pitch": 59, "start": 1.0, "length": 1.0 }] }] }] })).unwrap();
        let auto = freq(&crate::render(&p, crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap());
        let legato = freq(&render(0.95));
        let hz = 440.0 * 2f64.powf((59.0 - 69.0) / 12.0);
        assert!((detached - hz).abs() < hz * 0.05, "不重叠：普通音头 {detached}");
        assert!((legato - 2.0 * hz).abs() < hz * 0.1, "重叠：连奏区域 {legato}");
        assert!((auto - 2.0 * hz).abs() < hz * 0.1, "legato 字段：连奏区域 {auto}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
