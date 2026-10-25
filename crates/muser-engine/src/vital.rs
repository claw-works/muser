//! Vital 的状态扩展（画像 `state_format: "vital"` 时启用）。
//!
//! Vital 的波表与调制矩阵（谁调制谁）不是插件参数，只存在状态里：一份 JSON 预设（与 `.vital` 文件同格式），
//! 包在 VST2 风格的 chunk（`VstW` / `CcnK` / `FBCh`）里，再由本宿主包进 VST3 状态容器。
//! 这里解析出 JSON、按工程里的保留键改写、再装回：
//!
//! - `"@modulations": [{"source": "env_2", "destination": "filter_1_cutoff", "amount": 0.4}, ...]`
//! - `"@wavetable": "basic"` 或 `{"1": "basic", "2": "square"}`（按振荡器）。
//!
//! 只依赖 JSON 结构，不依赖插件代码；格式不符时报错，不会写坏状态。

use serde_json::{Map, Value, json};

/// 可用的调制源。
pub const SOURCES: &[&str] = &[
    "env_1", "env_2", "env_3", "env_4", "env_5", "env_6", "lfo_1", "lfo_2", "lfo_3", "lfo_4", "lfo_5", "lfo_6", "lfo_7", "lfo_8",
    "random_1", "random_2", "random_3", "random_4", "macro_control_1", "macro_control_2", "macro_control_3", "macro_control_4",
    "velocity", "note", "note_in_octave", "aftertouch", "mod_wheel", "pitch_wheel", "stereo", "random", "lift", "slide",
];

/// 内置可生成的波表。
pub const WAVETABLES: &[(&str, &str)] = &[
    ("saw", "single-frame saw (Vital's Init)"),
    ("sine", "single-frame sine"),
    ("triangle", "single-frame triangle"),
    ("square", "single-frame square"),
    ("basic", "morphs sine → triangle → saw → square across the frames (Wave Frame 0 / 85 / 170 / 256)"),
    ("pwm", "square whose pulse width narrows from 50 % to 5 % across the frames"),
    ("harmonics", "adds harmonics one by one: frame 0 = sine, frame 256 = 16 harmonics (bright saw-like)"),
    ("formant", "sine bursts of rising frequency: vocal / sync-like timbres when the frame moves"),
];

const FRAME: usize = 2048;
const MAX_MODS: usize = 64;

/// 状态里 JSON 的位置。
struct Located {
    /// JSON 在 `data` 中的起止。
    start: usize,
    end: usize,
    /// 需要随 JSON 长度修正的长度字段：(偏移, 字节数, 大端?)。
    lengths: Vec<(usize, usize, bool)>,
}

fn locate(data: &[u8]) -> Result<Located, String> {
    const VST3: &[u8] = b"MBVST3\0\x01";
    if !data.starts_with(VST3) {
        return Err("只支持 Vital 的 VST3 版（状态里找不到 VST3 容器）".into());
    }
    let cl_at = VST3.len() + 16;
    let comp = cl_at + 8;
    let c = data.get(comp..comp + 180).ok_or("Vital 状态过短")?;
    if &c[0..4] != b"VstW" || &c[16..20] != b"CcnK" || &c[24..28] != b"FBCh" {
        return Err("Vital 状态格式不符（不是 VST2 chunk）".into());
    }
    let start = comp + 176;
    if data.get(start) != Some(&b'{') {
        return Err("Vital 状态里找不到 JSON".into());
    }
    let text = std::str::from_utf8(&data[start..]).unwrap_or_else(|e| std::str::from_utf8(&data[start..start + e.valid_up_to()]).unwrap());
    let mut de = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    de.next().ok_or("Vital 状态 JSON 为空")?.map_err(|e| format!("Vital 状态 JSON 无法解析: {e}"))?;
    let end = start + de.byte_offset();
    Ok(Located {
        start,
        end,
        lengths: vec![(cl_at, 8, false), (comp + 20, 4, true), (comp + 172, 4, true)],
    })
}

fn read_len(d: &[u8], at: usize, n: usize, be: bool) -> u64 {
    let b = &d[at..at + n];
    match (n, be) {
        (8, false) => u64::from_le_bytes(b.try_into().unwrap()),
        (4, true) => u64::from(u32::from_be_bytes(b.try_into().unwrap())),
        _ => unreachable!(),
    }
}

fn write_len(d: &mut [u8], at: usize, n: usize, be: bool, v: u64) {
    match (n, be) {
        (8, false) => d[at..at + 8].copy_from_slice(&v.to_le_bytes()),
        (4, true) => d[at..at + 4].copy_from_slice(&(v as u32).to_be_bytes()),
        _ => unreachable!(),
    }
}

/// 取出状态里的 JSON 文档。
pub fn read_json(data: &[u8]) -> Result<Value, String> {
    let l = locate(data)?;
    serde_json::from_slice(&data[l.start..l.end]).map_err(|e| format!("Vital 状态 JSON 无法解析: {e}"))
}

/// 用新的 JSON 文档替换状态里的 JSON，修正各级长度。
pub fn write_json(data: &[u8], doc: &Value) -> Result<Vec<u8>, String> {
    let l = locate(data)?;
    let new = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
    let old_len = (l.end - l.start) as i64;
    let delta = new.len() as i64 - old_len;
    let mut out = Vec::with_capacity(data.len() + new.len());
    out.extend_from_slice(&data[..l.start]);
    out.extend_from_slice(&new);
    out.extend_from_slice(&data[l.end..]);
    for (at, n, be) in l.lengths {
        let v = read_len(&out, at, n, be) as i64 + delta;
        if v < 0 {
            return Err("Vital 状态长度字段异常".into());
        }
        write_len(&mut out, at, n, be, v as u64);
    }
    Ok(out)
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 一帧（2048 点），峰值归一化到 1：各帧响度一致，切换/扫动波表时音量不跳。
fn frame(f: impl Fn(f64) -> f64) -> String {
    let xs: Vec<f64> = (0..FRAME).map(|i| f(i as f64 / FRAME as f64)).collect();
    let peak = xs.iter().fold(0f64, |m, x| m.max(x.abs())).max(1e-9);
    let mut bytes = Vec::with_capacity(FRAME * 4);
    for x in xs {
        bytes.extend_from_slice(&((x / peak) as f32).to_le_bytes());
    }
    b64(&bytes)
}

use std::f64::consts::TAU;

fn saw(p: f64) -> f64 {
    // 与 Vital Init 一致：0 → -1 下降，中点跳到 +1。
    if p < 0.5 { -2.0 * p } else { 2.0 - 2.0 * p }
}
fn sine(p: f64) -> f64 {
    (TAU * p).sin()
}
fn triangle(p: f64) -> f64 {
    let q = (p + 0.25) % 1.0;
    if q < 0.5 { 4.0 * q - 1.0 } else { 3.0 - 4.0 * q }
}
fn pulse(p: f64, width: f64) -> f64 {
    if p < width { 1.0 } else { -1.0 }
}

/// 生成一张波表（Vital 的 wavetable JSON）。
pub fn wavetable(name: &str) -> Result<Value, String> {
    let keys: Vec<(u32, String)> = match name {
        "saw" => vec![(0, frame(saw))],
        "sine" => vec![(0, frame(sine))],
        "triangle" => vec![(0, frame(triangle))],
        "square" => vec![(0, frame(|p| pulse(p, 0.5)))],
        "basic" => vec![(0, frame(sine)), (85, frame(triangle)), (170, frame(saw)), (256, frame(|p| pulse(p, 0.5)))],
        "pwm" => (0..=8).map(|k| (k * 32, frame(move |p| pulse(p, 0.5 - 0.45 * f64::from(k) / 8.0)))).collect(),
        "harmonics" => (0..16)
            .map(|k| {
                let pos = (k as u32 * 256) / 15;
                (pos, frame(move |p| (1..=k + 1).map(|h| (TAU * p * h as f64).sin() / h as f64).sum::<f64>() * 0.6))
            })
            .collect(),
        "formant" => (0..=8)
            .map(|k| {
                let ratio = 1.0 + f64::from(k) * 1.5;
                (k * 32, frame(move |p| (TAU * p * ratio).sin() * (0.5 - 0.5 * (TAU * p).cos())))
            })
            .collect(),
        other => {
            return Err(format!(
                "没有内置波表 '{other}'，可用: {}",
                WAVETABLES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ));
        }
    };
    let keyframes: Vec<Value> = keys.into_iter().map(|(pos, data)| json!({ "position": pos, "wave_data": data })).collect();
    Ok(json!({
        "author": "muser",
        "full_normalize": false,
        "groups": [{ "components": [{ "interpolation": 1, "interpolation_style": 1, "keyframes": keyframes, "type": "Wave Source" }] }],
        "name": name,
        "remove_all_dc": true,
        "version": "1.0.7",
    }))
}

/// 本次需要改写的内容是否存在。
pub fn wants_patch(params: &Map<String, Value>) -> bool {
    params.contains_key("@modulations") || params.contains_key("@wavetable")
}

/// 按 `@modulations` / `@wavetable` 改写 JSON 文档。
pub fn patch(doc: &mut Value, params: &Map<String, Value>) -> Result<(), String> {
    let settings = doc
        .get_mut("settings")
        .and_then(Value::as_object_mut)
        .ok_or("Vital 状态 JSON 里没有 settings")?;
    if let Some(w) = params.get("@wavetable") {
        let table: Vec<(usize, String)> = match w {
            Value::String(s) => (0..3).map(|i| (i, s.trim().to_lowercase())).collect(),
            Value::Object(m) => m
                .iter()
                .map(|(k, v)| {
                    let i: usize = k.trim().parse().ok().filter(|i| (1..=3).contains(i)).ok_or_else(|| format!("@wavetable 的键应为振荡器 1 / 2 / 3，实际 '{k}'"))?;
                    let name = v.as_str().ok_or_else(|| format!("@wavetable.{k} 应为波表名"))?;
                    Ok((i - 1, name.trim().to_lowercase()))
                })
                .collect::<Result<_, String>>()?,
            _ => return Err("@wavetable 应为波表名，或 {\"1\": 名称, \"2\": 名称}".into()),
        };
        let tables = settings
            .get_mut("wavetables")
            .and_then(Value::as_array_mut)
            .ok_or("Vital 状态里没有 wavetables")?;
        for (i, name) in table {
            let t = wavetable(&name)?;
            *tables.get_mut(i).ok_or("Vital 状态里振荡器波表数量不符")? = t;
        }
    }
    if let Some(m) = params.get("@modulations") {
        let list = m.as_array().ok_or("@modulations 应为数组：[{\"source\": ..., \"destination\": ..., \"amount\": ...}]")?;
        if list.len() > MAX_MODS {
            return Err(format!("@modulations 最多 {MAX_MODS} 条"));
        }
        let mut slots = Vec::new();
        for (i, item) in list.iter().enumerate() {
            let at = |e: String| format!("@modulations[{i}]: {e}");
            let o = item.as_object().ok_or_else(|| at("应为对象".into()))?;
            for k in o.keys() {
                if !["source", "destination", "amount", "bipolar", "stereo", "power"].contains(&k.as_str()) {
                    return Err(at(format!("未知字段 '{k}'（可用: source / destination / amount / bipolar / stereo / power）")));
                }
            }
            let source = o.get("source").and_then(Value::as_str).ok_or_else(|| at("缺少 source".into()))?.trim().to_lowercase();
            if !SOURCES.contains(&source.as_str()) {
                return Err(at(format!("未知的调制源 '{source}'，可用: {}", SOURCES.join(", "))));
            }
            let dest = o.get("destination").and_then(Value::as_str).ok_or_else(|| at("缺少 destination".into()))?.trim().to_lowercase();
            let valid_dest = settings.get(&dest).is_some_and(Value::is_number) && !dest.starts_with("modulation_");
            if !valid_dest {
                let near: Vec<&String> = settings
                    .keys()
                    .filter(|k| !k.starts_with("modulation_") && settings[*k].is_number())
                    .filter(|k| k.contains(dest.split('_').next_back().unwrap_or("")) && dest.split('_').next().is_some_and(|p| k.starts_with(p)))
                    .take(12)
                    .collect();
                return Err(at(format!(
                    "未知的调制目标 '{dest}'（目标是 Vital 的内部参数名，如 filter_1_cutoff / osc_1_wave_frame / osc_1_level）{}",
                    if near.is_empty() { String::new() } else { format!("；相近: {}", near.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) }
                )));
            }
            let num = |k: &str, lo: f64, hi: f64, def: f64| -> Result<f64, String> {
                match o.get(k) {
                    None => Ok(def),
                    Some(Value::Bool(b)) => Ok(if *b { 1.0 } else { 0.0 }),
                    Some(v) => v.as_f64().filter(|x| (lo..=hi).contains(x)).ok_or_else(|| at(format!("{k} 应为 {lo}..{hi} 的数字"))),
                }
            };
            slots.push((source, dest, num("amount", -1.0, 1.0, 0.5)?, num("bipolar", 0.0, 1.0, 0.0)?, num("stereo", 0.0, 1.0, 0.0)?, num("power", -10.0, 10.0, 0.0)?));
        }
        let mods = settings
            .get_mut("modulations")
            .and_then(Value::as_array_mut)
            .ok_or("Vital 状态里没有 modulations")?;
        for (i, slot) in mods.iter_mut().enumerate() {
            *slot = match slots.get(i) {
                Some((s, d, ..)) => json!({ "source": s, "destination": d }),
                None => json!({ "source": "", "destination": "" }),
            };
        }
        for i in 0..MAX_MODS {
            let (amount, bipolar, stereo, power) = slots.get(i).map_or((0.0, 0.0, 0.0, 0.0), |s| (s.2, s.3, s.4, s.5));
            let n = i + 1;
            settings.insert(format!("modulation_{n}_amount"), json!(amount));
            settings.insert(format!("modulation_{n}_bipolar"), json!(bipolar));
            settings.insert(format!("modulation_{n}_stereo"), json!(stereo));
            settings.insert(format!("modulation_{n}_power"), json!(power));
            settings.insert(format!("modulation_{n}_bypass"), json!(0.0));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_state(doc: &Value) -> Vec<u8> {
        let json = serde_json::to_vec(doc).unwrap();
        let tail = b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0JUCEPrivateData";
        let chunk_len = json.len() + tail.len();
        let mut comp = Vec::new();
        comp.extend_from_slice(b"VstW");
        comp.extend_from_slice(&8u32.to_be_bytes());
        comp.extend_from_slice(&1u32.to_be_bytes());
        comp.extend_from_slice(&0u32.to_be_bytes());
        comp.extend_from_slice(b"CcnK");
        comp.extend_from_slice(&((152 + chunk_len) as u32).to_be_bytes());
        comp.extend_from_slice(b"FBCh");
        comp.extend_from_slice(&[0u8; 144]);
        comp.extend_from_slice(&(chunk_len as u32).to_be_bytes());
        comp.extend_from_slice(&json);
        comp.extend_from_slice(tail);
        let mut d = b"MBVST3\0\x01".to_vec();
        d.extend_from_slice(&[0u8; 16]);
        d.extend_from_slice(&(comp.len() as u64).to_le_bytes());
        d.extend_from_slice(&comp);
        d.extend_from_slice(&0u64.to_le_bytes());
        d
    }

    #[test]
    fn roundtrip_and_patch() {
        let mut settings = Map::new();
        settings.insert("filter_1_cutoff".into(), json!(60.0));
        settings.insert("osc_1_level".into(), json!(0.7));
        settings.insert("modulations".into(), json!((0..64).map(|_| json!({ "source": "", "destination": "" })).collect::<Vec<_>>()));
        settings.insert("wavetables".into(), json!([{}, {}, {}]));
        let doc = json!({ "settings": settings });
        let state = fake_state(&doc);
        assert_eq!(read_json(&state).unwrap(), doc);

        let mut d = read_json(&state).unwrap();
        let p: Map<String, Value> = serde_json::from_value(json!({
            "@modulations": [{ "source": "env_2", "destination": "filter_1_cutoff", "amount": 0.4 }],
            "@wavetable": { "1": "basic" },
        }))
        .unwrap();
        patch(&mut d, &p).unwrap();
        let out = write_json(&state, &d).unwrap();
        let back = read_json(&out).unwrap();
        assert_eq!(back["settings"]["modulations"][0]["destination"], "filter_1_cutoff");
        assert_eq!(back["settings"]["modulation_1_amount"], 0.4);
        assert_eq!(back["settings"]["wavetables"][0]["name"], "basic");
        assert_eq!(back["settings"]["wavetables"][0]["groups"][0]["components"][0]["keyframes"].as_array().unwrap().len(), 4);
        // 长度字段一致：组件长度 = 总长 - 头 - 控制器段。
        let comp_len = u64::from_le_bytes(out[24..32].try_into().unwrap()) as usize;
        assert_eq!(comp_len, out.len() - 32 - 8);

        let bad = |v: Value| patch(&mut read_json(&state).unwrap(), &serde_json::from_value(v).unwrap()).unwrap_err();
        assert!(bad(json!({ "@modulations": [{ "source": "env_9", "destination": "filter_1_cutoff" }] })).contains("调制源"));
        assert!(bad(json!({ "@modulations": [{ "source": "env_2", "destination": "filter_1_cutof" }] })).contains("filter_1_cutoff"));
        assert!(bad(json!({ "@wavetable": "nope" })).contains("basic"));
    }

    #[test]
    fn wavetables_are_normalised_frames() {
        for (name, _) in WAVETABLES {
            let t = wavetable(name).unwrap();
            for k in t["groups"][0]["components"][0]["keyframes"].as_array().unwrap() {
                let pos = k["position"].as_u64().unwrap();
                assert!(pos <= 256);
                assert_eq!(k["wave_data"].as_str().unwrap().len(), 10924, "与 Vital Init 的帧编码长度一致");
            }
        }
    }
}
