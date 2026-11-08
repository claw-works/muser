//! Serum 2 的预设（`.SerumPreset`）→ 插件状态，读用户本机的文件、当场转换，不需要打开插件。
//!
//! Serum 2 的预设和它存给宿主的 VST3 processor 状态是同一种容器（XferJson）：
//! `XferJson\0`、`u64` 元数据长度、元数据 JSON、（有的文件多 1 字节填充）`u32` 解压后长度、`u32` 版本、zstd 压缩的 CBOR。
//! 预设里的 CBOR 就是完整的 processor 快照：去掉顶层的 `fileType`、加上 `component: "processor"` 就是插件认的状态
//! （controller 段可以为空）。改动在 CBOR 字节上做，其余条目逐字节保留（数值的宽度、键的顺序都不变）。
//!
//! 效果链（`.SerumFX` 单个效果、`.SerumFXRack` 整条链）给 Serum 2 FX 用：插件的 processor 状态不能只有效果链，
//! 所以拿插件自己存的一份初始状态（[`crate::presets`] 在用户机器上打开一次 Serum 2 FX 存下来）当骨架，只把其中
//! `FXRack0.FX` 换成文件里的效果链。
//!
//! 只支持 Serum 2 写的新格式；早期的 zlib 压缩格式和 Serum 1 的 `.fxp` 报错，提示在 Serum 2 里打开再另存。

use std::io::Read;

const MAGIC: &[u8] = b"XferJson\0";
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// 预设文件头里的元数据（名字、作者、标签、说明）。
#[derive(Debug, Default, Clone)]
pub struct Meta {
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
}

fn le(b: &[u8], at: usize, n: usize) -> Result<u64, String> {
    let s = b.get(at..at + n).ok_or("the Serum preset is truncated")?;
    Ok(s.iter().rev().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
}

/// (元数据 JSON 的字节, 压缩数据的起点（`u32` 解压后长度处）)。
fn header(b: &[u8]) -> Result<(&[u8], usize), String> {
    if !b.starts_with(MAGIC) {
        return Err("not a Serum 2 preset (no XferJson header)".into());
    }
    let n = usize::try_from(le(b, 9, 8)?).map_err(|_| "the Serum preset header is invalid")?;
    let meta = b.get(17..17 + n).ok_or("the Serum preset is truncated")?;
    Ok((meta, 17 + n))
}

/// 读预设文件头里的元数据（不解压）。
pub fn meta(b: &[u8]) -> Result<Meta, String> {
    let (js, _) = header(b)?;
    let v: serde_json::Value = serde_json::from_slice(js).map_err(|e| format!("the Serum preset metadata is not valid JSON: {e}"))?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let tags = v.get("tags").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(Meta { name: s("presetName"), author: s("presetAuthor"), description: s("presetDescription"), tags })
}

/// 解开一个 XferJson 块：(元数据 JSON 的字节, 格式版本, CBOR)。
fn unpack(b: &[u8]) -> Result<(&[u8], u32, Vec<u8>), String> {
    let (js, end) = header(b)?;
    let at = [end, end + 1]
        .into_iter()
        .find(|&p| b.get(p + 8..p + 12) == Some(&ZSTD_MAGIC[..]))
        .ok_or("this Serum file uses an older format that muser cannot read: open it in Serum 2 and save it again")?;
    let size = usize::try_from(le(b, at, 4)?).map_err(|_| "invalid size")?;
    let version = le(b, at + 4, 4)? as u32;
    let mut frame = &b[at + 8..];
    let mut raw = Vec::with_capacity(size);
    ruzstd::decoding::StreamingDecoder::new(&mut frame)
        .map_err(|e| format!("cannot decompress the Serum file: {e}"))?
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot decompress the Serum file: {e}"))?;
    if raw.len() != size {
        return Err(format!("the Serum file decompressed to {} bytes instead of {size}", raw.len()));
    }
    Ok((js, version, raw))
}

fn pack(js: &[u8], version: u32, raw: &[u8]) -> Vec<u8> {
    let packed = ruzstd::encoding::compress_to_vec(raw, ruzstd::encoding::CompressionLevel::Fastest);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(js.len() as u64).to_le_bytes());
    out.extend_from_slice(js);
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&packed);
    out
}

/// `.SerumPreset` 的内容 → VST3 processor 状态（一个 XferJson 块）。
pub fn preset_component(b: &[u8]) -> Result<Vec<u8>, String> {
    let (js, version, raw) = unpack(b)?;
    Ok(pack(js, version, &to_processor(&raw)?))
}

/// 效果链文件（`.SerumFX` / `.SerumFXRack`）+ Serum 2 FX 自己存的 processor 状态（骨架）→ 新的 processor 状态。
pub fn fx_component(skeleton: &[u8], chain: &[u8]) -> Result<Vec<u8>, String> {
    let fx = fx_list(chain)?;
    let (js, version, raw) = unpack(skeleton)?;
    let rack = value_of(&raw, 0, "FXRack0")?.ok_or("the Serum 2 FX state has no FXRack0")?;
    let slot = value_of(&raw, rack.start, "FX")?.ok_or("the Serum 2 FX state has no FXRack0.FX")?;
    let new = [&raw[..slot.start], &fx[..], &raw[slot.end..]].concat();
    Ok(pack(js, version, &new))
}

/// 效果链文件里的效果数组（CBOR 字节）：整条链取 `FXRack.FX`；单个效果包成一项 `[{<效果>: …, "type": …}]`。
fn fx_list(chain: &[u8]) -> Result<Vec<u8>, String> {
    let (js, _, raw) = unpack(chain)?;
    let m: serde_json::Value = serde_json::from_slice(js).map_err(|e| format!("the Serum file metadata is not valid JSON: {e}"))?;
    let file_type = m.get("fileType").and_then(|x| x.as_str()).unwrap_or_default();
    if file_type == "SerumFXRack" {
        // 空的 Init 链可能没有 FX：当空数组。
        return Ok(match value_of(&raw, 0, "FXRack")? {
            Some(r) => value_of(&raw, r.start, "FX")?.map_or(vec![0x80], |v| raw[v].to_vec()),
            None => vec![0x80],
        });
    }
    if file_type != "SerumFX" {
        return Err(format!("not a Serum effect file (fileType '{file_type}')"));
    }
    let entries = map_entries(&raw, 0)?;
    let name = |k: &std::ops::Range<usize>| text_value(&raw, k.start);
    let wanted = m.get("fxType").and_then(|x| x.as_str()).map(str::to_string);
    let (key, value) = entries
        .iter()
        .find(|(k, v)| {
            let n = name(k).unwrap_or_default();
            wanted.as_deref().map_or(n.starts_with("FX") && raw[v.start] >> 5 == 5, |w| n == w)
        })
        .ok_or("the Serum effect file has no effect in it")?;
    let kind = entries.iter().find(|(k, _)| name(k).as_deref() == Some("type")).map_or(&[0u8][..], |(_, v)| &raw[v.clone()]);
    Ok([&[0x81, 0xa2][..], &raw[key.clone()], &raw[value.clone()], &text("type"), kind].concat())
}

// —— CBOR：只认结构，跳过条目，不解释数值 ——

/// 一个条目的头：(主类型, 附加信息, 参数, 头之后的位置)。浮点数和简单值的数据算在头里。
fn head(b: &[u8], i: usize) -> Result<(u8, u8, u64, usize), String> {
    let x = *b.get(i).ok_or("the CBOR data is truncated")?;
    let (major, ai) = (x >> 5, x & 0x1f);
    let (arg, next) = match ai {
        0..=23 => (u64::from(ai), i + 1),
        24..=27 => {
            let n = 1usize << (ai - 24);
            let s = b.get(i + 1..i + 1 + n).ok_or("the CBOR data is truncated")?;
            (s.iter().fold(0u64, |a, &y| (a << 8) | u64::from(y)), i + 1 + n)
        }
        31 => (u64::MAX, i + 1),
        _ => return Err(format!("invalid CBOR at byte {i}")),
    };
    Ok((major, ai, arg, next))
}

/// 跳过位置 `i` 的一个条目，返回它之后的位置。
fn skip(b: &[u8], i: usize, depth: usize) -> Result<usize, String> {
    if depth > 256 {
        return Err("the CBOR data is nested too deeply".into());
    }
    let (major, ai, arg, mut j) = head(b, i)?;
    let indefinite = ai == 31;
    let until_break = |mut j: usize, each: &dyn Fn(usize) -> Result<usize, String>| -> Result<usize, String> {
        while *b.get(j).ok_or("the CBOR data is truncated")? != 0xff {
            j = each(j)?;
        }
        Ok(j + 1)
    };
    Ok(match major {
        0 | 1 | 7 => j,
        2 | 3 if indefinite => until_break(j, &|k| skip(b, k, depth + 1))?,
        2 | 3 => {
            let end = j.checked_add(usize::try_from(arg).map_err(|_| "invalid CBOR length")?).ok_or("invalid CBOR length")?;
            if end > b.len() {
                return Err("the CBOR data is truncated".into());
            }
            end
        }
        4 | 5 if indefinite => until_break(j, &|k| skip(b, k, depth + 1))?,
        4 | 5 => {
            for _ in 0..arg * if major == 5 { 2 } else { 1 } {
                j = skip(b, j, depth + 1)?;
            }
            j
        }
        6 => skip(b, j, depth + 1)?,
        _ => unreachable!(),
    })
}

fn text(s: &str) -> Vec<u8> {
    let n = s.len();
    let mut v = if n < 24 { vec![0x60 | n as u8] } else if n < 256 { vec![0x78, n as u8] } else { [&[0x79][..], &(n as u16).to_be_bytes()].concat() };
    v.extend_from_slice(s.as_bytes());
    v
}

/// 位置 `i` 的文字条目（不是文字时为空）。
fn text_value(b: &[u8], i: usize) -> Option<String> {
    let (major, ai, n, j) = head(b, i).ok()?;
    (major == 3 && ai != 31).then(|| b.get(j..j + n as usize).map(|t| String::from_utf8_lossy(t).into_owned())).flatten()
}

/// 位置 `i` 的定长 map 的每一项：(键的范围, 值的范围)。
fn map_entries(b: &[u8], i: usize) -> Result<Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>, String> {
    let (major, ai, n, mut j) = head(b, i)?;
    if major != 5 || ai == 31 {
        return Err("the Serum data is not in the expected shape (a CBOR map)".into());
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let k = j;
        let v = skip(b, k, 0)?;
        j = skip(b, v, 0)?;
        out.push((k..v, v..j));
    }
    Ok(out)
}

/// 位置 `i` 的 map 里键为 `key` 的值的范围。
fn value_of(b: &[u8], i: usize, key: &str) -> Result<Option<std::ops::Range<usize>>, String> {
    let k = text(key);
    Ok(map_entries(b, i)?.into_iter().find(|(r, _)| b[r.clone()] == k[..]).map(|(_, v)| v))
}

/// 顶层 map：去掉 `fileType`（和已有的 `component`），在末尾加 `component: "processor"`。
fn to_processor(raw: &[u8]) -> Result<Vec<u8>, String> {
    let entries = map_entries(raw, 0)?;
    if entries.last().map_or(1, |(_, v)| v.end) != raw.len() {
        return Err("the Serum preset data has trailing bytes".into());
    }
    let drop = [text("fileType"), text("component")];
    let kept: Vec<_> = entries.into_iter().filter(|(k, _)| !drop.iter().any(|d| raw[k.clone()] == d[..])).collect();
    let count = kept.len() as u64 + 1;
    let mut out = if count < 24 {
        vec![0xa0 | count as u8]
    } else if count < 256 {
        vec![0xb8, count as u8]
    } else {
        [&[0xb9][..], &(count as u16).to_be_bytes()].concat()
    };
    for (k, v) in kept {
        out.extend_from_slice(&raw[k.start..v.end]);
    }
    out.extend(text("component"));
    out.extend(text("processor"));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自己拼一个最小的 `.SerumPreset`：{"fileType": "SerumPreset", "Osc0": {"a": 1.5f32, "b": [1, -2, null]}}。
    fn sample(pad: bool) -> (Vec<u8>, Vec<u8>) {
        let mut cbor = vec![0xa2];
        cbor.extend(text("fileType"));
        cbor.extend(text("SerumPreset"));
        let osc = [&text("Osc0")[..], &[0xa2], &text("a"), &[0xfa, 0x3f, 0xc0, 0, 0], &text("b"), &[0x83, 0x01, 0x21, 0xf6]].concat();
        cbor.extend_from_slice(&osc);
        let js = br#"{"fileType":"SerumPreset","presetName":"Test Pad","presetAuthor":"me","tags":["Pad","Warm"]}"#;
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&(js.len() as u64).to_le_bytes());
        f.extend_from_slice(js);
        if pad {
            f.push(0);
        }
        f.extend_from_slice(&(cbor.len() as u32).to_le_bytes());
        f.extend_from_slice(&2u32.to_le_bytes());
        f.extend(ruzstd::encoding::compress_to_vec(cbor.as_slice(), ruzstd::encoding::CompressionLevel::Fastest));
        f.extend_from_slice(b"trailing");
        (f, osc)
    }

    #[test]
    fn converts_a_preset_to_a_processor_state() {
        for pad in [false, true] {
            let (file, osc) = sample(pad);
            let m = meta(&file).unwrap();
            assert_eq!((m.name.as_str(), m.author.as_str(), m.tags.len()), ("Test Pad", "me", 2));
            let block = preset_component(&file).unwrap();
            // 转换后的块本身也是一个能读的 XferJson：再解一次，看 CBOR。
            let (_, end) = header(&block).unwrap();
            let mut frame = &block[end + 8..];
            let mut raw = Vec::new();
            ruzstd::decoding::StreamingDecoder::new(&mut frame).unwrap().read_to_end(&mut raw).unwrap();
            assert_eq!(le(&block, end, 4).unwrap() as usize, raw.len());
            let expected = [&[0xa2][..], &osc, &text("component"), &text("processor")].concat();
            assert_eq!(raw, expected);
        }
    }

    fn file(meta: &str, cbor: &[u8]) -> Vec<u8> {
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&(meta.len() as u64).to_le_bytes());
        f.extend_from_slice(meta.as_bytes());
        f.extend_from_slice(&(cbor.len() as u32).to_le_bytes());
        f.extend_from_slice(&2u32.to_le_bytes());
        f.extend(ruzstd::encoding::compress_to_vec(cbor, ruzstd::encoding::CompressionLevel::Fastest));
        f
    }

    fn cbor_of(block: &[u8]) -> Vec<u8> {
        unpack(block).unwrap().2
    }

    #[test]
    fn puts_an_effect_chain_into_the_fx_state() {
        // 骨架：{"Global0": 1, "FXRack0": {"FX": [], "plainParams": 7}, "component": "processor"}
        let rack = [&[0xa2][..], &text("FX"), &[0x80], &text("plainParams"), &[0x07]].concat();
        let skel = file(r#"{"fileType":"x"}"#, &[&[0xa3][..], &text("Global0"), &[0x01], &text("FXRack0"), &rack, &text("component"), &text("processor")].concat());
        let chorus = [&[0xa1][..], &text("kParamWet"), &[0xf9, 0x3c, 0x00]].concat();
        // 整条链：{"FXRack": {"FX": [chorus]}}
        let chain = [&[0xa1][..], &text("FXRack"), &[0xa1], &text("FX"), &[0x81], &chorus].concat();
        let out = fx_component(&skel, &file(r#"{"fileType":"SerumFXRack"}"#, &chain)).unwrap();
        let want_rack = [&[0xa2][..], &text("FX"), &[0x81], &chorus, &text("plainParams"), &[0x07]].concat();
        let want = [&[0xa3][..], &text("Global0"), &[0x01], &text("FXRack0"), &want_rack, &text("component"), &text("processor")].concat();
        assert_eq!(cbor_of(&out), want);
        // 单个效果：{"FXChorus": {...}, "type": 3}
        let one = [&[0xa2][..], &text("FXChorus"), &chorus, &text("type"), &[0x03]].concat();
        let out = fx_component(&skel, &file(r#"{"fileType":"SerumFX","fxType":"FXChorus"}"#, &one)).unwrap();
        let item = [&[0x81, 0xa2][..], &text("FXChorus"), &chorus, &text("type"), &[0x03]].concat();
        let want_rack = [&[0xa2][..], &text("FX"), &item, &text("plainParams"), &[0x07]].concat();
        assert_eq!(cbor_of(&out), [&[0xa3][..], &text("Global0"), &[0x01], &text("FXRack0"), &want_rack, &text("component"), &text("processor")].concat());
        // 空的 Init 链。
        let out = fx_component(&skel, &file(r#"{"fileType":"SerumFXRack"}"#, &[0xa0])).unwrap();
        assert_eq!(cbor_of(&out), cbor_of(&skel));
        assert!(fx_component(&skel, &file(r#"{"fileType":"SerumPreset"}"#, &[0xa0])).is_err());
    }

    #[test]
    fn rejects_other_files() {
        assert!(preset_component(b"CcnK....").unwrap_err().contains("XferJson"));
        let mut old = MAGIC.to_vec();
        old.extend_from_slice(&2u64.to_le_bytes());
        old.extend_from_slice(b"{}\0\x05\0\0\0\0\0\0\0\x78\x01");
        assert!(preset_component(&old).unwrap_err().contains("older format"));
        assert!(skip(&[0x82, 0x01], 0, 0).is_err());
    }
}
