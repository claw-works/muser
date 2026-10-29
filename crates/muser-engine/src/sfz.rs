//! SFZ 控制器（MIDI CC）。
//!
//! 上游采样器不处理按控制器选择/调制的 opcode（`loccN` / `hiccN`、`amplitude_onccN` 等），它们原样留在
//! 区域里、全部同时发声（如 Black And Blue Basses 的"单音开关""低音弦选择"会把几套采样叠在一起）。
//! muser 在加载时按控制器的值**静态展开**：
//!
//! - 控制器的值 = 工程里 `instrument_params.cc` 给出的值，否则是 .sfz 的 `set_ccN` 默认值，否则 0；
//! - `loccN` / `hiccN`（及 `lohdccN` / `hihdccN`，0–1）不满足的区域去掉；
//! - `volume_onccN`（dB）、`amplitude_onccN`（%）、`pan_onccN` 按控制器值折算成静态的 `volume` / `amplitude` / `pan`。
//!
//! - 键位切换（`sw_last` / `sw_default` / `sw_label`）：只保留选中奏法的区域。默认 `sw_default`，
//!   工程里 `instrument_params.keyswitch` 可选（音高数字、音名如 `"c1"`，或 `sw_label` 名称）。
//!
//! 只在加载时生效，演奏中改控制器（CC 自动化）或用键位切换奏法不支持。

use std::collections::BTreeMap;
use std::path::Path;

use manguebyte::sampler::{FileSource, Instrument, Opcode, Sfz, WarningKind};

/// .sfz 里声明的控制器：编号 → (名称, 默认值)。
pub fn controls(sfz: &Sfz) -> BTreeMap<u8, (String, u8)> {
    let mut out: BTreeMap<u8, (String, u8)> = BTreeMap::new();
    for w in &sfz.warnings {
        if let WarningKind::UnknownOpcode { name, value } = &w.kind {
            if let Some(n) = name.strip_prefix("label_cc").and_then(|n| n.parse::<u8>().ok()) {
                out.entry(n).or_insert((String::new(), 0)).0 = value.trim().to_string();
            } else if let Some(n) = name.strip_prefix("set_cc").and_then(|n| n.parse::<u8>().ok()) {
                let v = value.trim().parse::<f64>().unwrap_or(0.0).clamp(0.0, 127.0) as u8;
                out.entry(n).or_insert((String::new(), 0)).1 = v;
            } else if let Some(n) = name.strip_prefix("set_hdcc").and_then(|n| n.parse::<u8>().ok()) {
                // 高精度写法：0–1。
                let v = (value.trim().parse::<f64>().unwrap_or(0.0).clamp(0.0, 1.0) * 127.0).round() as u8;
                out.entry(n).or_insert((String::new(), 0)).1 = v;
            }
        }
    }
    out.retain(|n, _| *n < 128);
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

fn parse(path: &Path) -> Result<(Sfz, FileSource), String> {
    let text = std::fs::read(path).map_err(|e| format!("读取 SFZ '{}' 失败: {e}", path.display()))?;
    let mut source = FileSource::new(path.parent().unwrap_or(Path::new("")));
    let sfz = Sfz::parse_with(&String::from_utf8_lossy(&text), &mut source).map_err(|e| format!("解析 SFZ '{}' 失败: {e}", path.display()))?;
    Ok((sfz, source))
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
    let l = std::sync::Arc::new(Loaded { instrument, dropped, warnings });
    *held = std::sync::Arc::downgrade(&l);
    Ok(l)
}

/// `<base>_oncc<N>`、`<base>_cc<N>`、`<base>cc<N>`（SFZ 1 的写法，如 `pitchlfo_depthcc1`）→ (base, N)。
fn cc_suffix(name: &str) -> Option<(&str, u8)> {
    let digits = name.bytes().rev().take_while(u8::is_ascii_digit).count();
    let (head, n) = name.split_at(name.len() - digits);
    let n: u8 = n.parse().ok().filter(|n| *n < 128)?;
    let base = head.strip_suffix("_oncc").or_else(|| head.strip_suffix("_cc")).or_else(|| head.strip_suffix("cc"))?;
    (!base.is_empty()).then_some((base, n))
}

/// LFO 的 opcode 名 → (槽位, 字段)：amplfo_ / pitchlfo_ / fillfo_ 是 0–2 号，lfo1–lfo4 是 3–6 号（与采样器一致）。
fn lfo(name: &str) -> Option<(usize, &str)> {
    for (prefix, slot) in [("amplfo_", 0), ("pitchlfo_", 1), ("fillfo_", 2)] {
        if let Some(f) = name.strip_prefix(prefix) {
            return Some((slot, f));
        }
    }
    let rest = name.strip_prefix("lfo")?;
    let d = rest.bytes().take_while(u8::is_ascii_digit).count();
    let k: usize = rest[..d].parse().ok()?;
    (1..=4).contains(&k).then(|| rest[d..].strip_prefix('_').map(|f| (2 + k, f))).flatten()
}

/// 把按控制器调制的量（已按当前控制器值折算成 `delta`）加到区域的对应 opcode 上。
/// 采样器不认识的 base 返回 false（留给"不支持"的提示）。
fn fold_cc(r: &mut manguebyte::sampler::Region, base: &str, delta: f64) -> bool {
    let eg = |e: &mut manguebyte::sampler::EnvelopeSpec, f: &str| -> bool {
        let field = match f {
            "delay" => &mut e.delay,
            "start" => &mut e.start,
            "attack" => &mut e.attack,
            "hold" => &mut e.hold,
            "decay" => &mut e.decay,
            "sustain" => &mut e.sustain,
            "release" => &mut e.release,
            _ => return false,
        };
        *field = (*field + delta).max(0.0);
        true
    };
    match base {
        "tune" | "pitch" => r.tune += delta,
        "offset" => r.offset = (r.offset as f64 + delta).max(0.0) as u64,
        // cutoff 的调制量是音分。
        "cutoff" => r.filter.cutoff = r.filter.cutoff.map(|c| c * 2f64.powf(delta / 1_200.0)),
        "resonance" => r.filter.resonance = (r.filter.resonance + delta).clamp(0.0, 40.0),
        "fileg_depth" => r.fileg_depth += delta,
        "pitcheg_depth" => r.pitcheg_depth += delta,
        b if b.starts_with("ampeg_") => return eg(&mut r.ampeg, &b[6..]),
        b if b.starts_with("fileg_") => return eg(&mut r.fileg, &b[6..]),
        b if b.starts_with("pitcheg_") => return eg(&mut r.pitcheg, &b[8..]),
        b => {
            let Some((slot, f)) = lfo(b) else { return false };
            let l = &mut r.lfos[slot];
            let field = match (slot, f) {
                (_, "freq") => &mut l.freq,
                (_, "delay") => &mut l.delay,
                (_, "fade") => &mut l.fade,
                (0, "depth") | (3.., "volume") => &mut l.volume,
                (1, "depth") | (3.., "pitch") => &mut l.pitch,
                (2, "depth") | (3.., "cutoff") => &mut l.cutoff,
                (3.., "amplitude") => &mut l.amplitude,
                (3.., "pan") => &mut l.pan,
                (3.., "phase") => &mut l.phase,
                _ => return false,
            };
            *field += delta;
            if f == "freq" {
                l.freq = l.freq.max(0.0);
            }
        }
    }
    true
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
    let mut handled = std::collections::HashSet::<String>::new();
    sfz.regions.retain_mut(|r| {
        let (mut lo, mut hi): (BTreeMap<u8, f64>, BTreeMap<u8, f64>) = (BTreeMap::new(), BTreeMap::new());
        // 控制器交叉淡化：cc → (lo, hi)，淡入 / 淡出。
        let (mut xin, mut xout): (BTreeMap<u8, (f64, f64)>, BTreeMap<u8, (f64, f64)>) = (BTreeMap::new(), BTreeMap::new());
        let mut power = false;
        // 同名的调制 opcode（如重复写了两次 `gain_cc1`）以最后一个为准，不累加。
        let mut vol_mod: BTreeMap<u8, f64> = BTreeMap::new();
        let mut amp_mod: BTreeMap<u8, f64> = BTreeMap::new();
        let mut pan_mod: BTreeMap<u8, f64> = BTreeMap::new();
        let mut keep: Vec<Opcode> = Vec::new();
        let num = |o: &Opcode| o.value.trim().parse::<f64>().ok();
        let mut folds: Vec<(Opcode, String, f64)> = Vec::new();
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
            } else if let Some(c) = cc_num(n, "volume_oncc").or_else(|| cc_num(n, "gain_oncc")).or_else(|| cc_num(n, "gain_cc")).or_else(|| cc_num(n, "volume_cc")) {
                vol_mod.insert(c, num(&o).unwrap_or(0.0));
            } else if let Some(c) = cc_num(n, "amplitude_oncc").or_else(|| cc_num(n, "amplitude_cc")) {
                // SFZ v2：amplitude 加上 depth × 控制器/127（百分比），如 `amplitude=0 amplitude_oncc100=100` 由 CC100 决定音量。
                amp_mod.insert(c, num(&o).unwrap_or(0.0));
            } else if switched && matches!(n, "sw_last" | "sw_label" | "sw_default" | "sw_lokey" | "sw_hikey") {
                // 已在上面按键位切换选好奏法。
            } else if let Some(c) = cc_num(n, "pan_oncc") {
                pan_mod.insert(c, num(&o).unwrap_or(0.0));
            } else if let Some((base, c)) = cc_suffix(n) {
                // 循环之后再折算（这时还在遍历区域的 opcode）。
                let delta = num(&o).unwrap_or(0.0) * cc(c) / 127.0;
                folds.push((o.clone(), base.to_string(), delta));
                continue;
            } else {
                used = false;
            }
            if used {
                handled.insert(n.to_string());
            } else {
                keep.push(o);
            }
        }
        for (o, base, delta) in folds {
            if fold_cc(r, &base, delta) {
                handled.insert(o.name.to_string());
            } else {
                keep.push(o);
            }
        }
        r.unknown = keep;
        for (c, d) in &vol_mod {
            r.volume += d * cc(*c) / 127.0;
        }
        for (c, d) in &amp_mod {
            r.amplitude = (r.amplitude + d * cc(*c) / 127.0).clamp(0.0, 100.0);
        }
        for (c, d) in &pan_mod {
            r.pan = (r.pan + d * cc(*c) / 127.0).clamp(-100.0, 100.0);
        }
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
    // 只报留下来、会发声的区域里还剩的 opcode：已经处理掉的、控制器声明、只出现在被去掉的区域
    // （别的奏法、控制器范围外）里的都不算"不支持"。
    let remaining: std::collections::HashSet<&str> = sfz.regions.iter().flat_map(|r| r.unknown.iter().map(|o| &*o.name)).collect();
    let remaining: std::collections::HashSet<String> = remaining.into_iter().map(str::to_string).collect();
    sfz.warnings.retain(|w| match &w.kind {
        WarningKind::UnknownOpcode { name, .. } => remaining.contains(&**name) && !handled.contains(&**name),
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

    /// 按控制器调制的包络、LFO、滤波、音高按当前控制器值折算进去；控制器为 0 时不变。
    #[test]
    fn folds_cc_modulation_into_the_new_opcodes() {
        let text = "<region> sample=*sine cutoff=500 lfo1_freq=5 lfo1_pitch_oncc1=100 pitchlfo_freq=4 pitchlfo_depthcc1=50
            ampeg_attack_oncc11=0.5 cutoff_cc1=1200 tune_oncc2=20 fileg_decay_oncc1=1";
        let fold = |cc1: f64, cc11: f64| {
            let mut sfz = manguebyte::sampler::Sfz::parse(text).unwrap();
            let r = &mut sfz.regions[0];
            let ops: Vec<Opcode> = r.unknown.drain(..).collect();
            for o in &ops {
                let (base, c) = cc_suffix(&o.name).unwrap();
                let v: f64 = o.value.parse().unwrap();
                let cc = match c { 1 => cc1, 11 => cc11, _ => 0.0 };
                assert!(fold_cc(r, base, v * cc / 127.0), "{}", o.name);
            }
            r.clone()
        };
        let r = fold(127.0, 127.0);
        assert_eq!((r.lfos[3].pitch, r.lfos[1].pitch, r.ampeg.attack, r.tune, r.fileg.decay), (100.0, 50.0, 0.5, 0.0, 1.0));
        assert!((r.filter.cutoff.unwrap() - 1000.0).abs() < 1e-6);
        let r = fold(0.0, 0.0);
        assert_eq!((r.lfos[3].pitch, r.ampeg.attack, r.filter.cutoff), (0.0, 0.0, Some(500.0)));
        assert_eq!(cc_suffix("pitchlfo_depthcc1"), Some(("pitchlfo_depth", 1)));
        assert_eq!(cc_suffix("lfo2_freq_lfo1_oncc3"), Some(("lfo2_freq_lfo1", 3)));
        assert!(!fold_cc(&mut fold(0.0, 0.0), "lfo2_freq_lfo1", 1.0), "LFO 调 LFO 不支持");
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
