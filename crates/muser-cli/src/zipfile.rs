//! 不压缩（stored）的 zip：`muser view` 把一个导出目录（DAW 包）打成一个下载。
//! 音频、MIDI 本来就压不了多少，不值得为它加一个依赖。

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// (zip 里的路径, 内容) → zip 文件。路径用 `/` 分隔。
pub fn store(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let (crc, size, offset) = (crc32(data), data.len() as u32, out.len() as u32);
        // 本地文件头：版本 2.0，标志 0x0800（文件名是 UTF-8），不压缩，时间 0。
        le32(&mut out, 0x0403_4b50);
        for x in [20u16, 0x0800, 0, 0, 0x21] {
            le16(&mut out, x);
        }
        for x in [crc, size, size] {
            le32(&mut out, x);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        le32(&mut central, 0x0201_4b50);
        for x in [20u16, 20, 0x0800, 0, 0, 0x21] {
            le16(&mut central, x);
        }
        for x in [crc, size, size] {
            le32(&mut central, x);
        }
        for x in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_size) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    for x in [0u16, 0, files.len() as u16, files.len() as u16] {
        le16(&mut out, x);
    }
    le32(&mut out, cd_size);
    le32(&mut out, cd_offset);
    le16(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    /// 系统的 unzip 能读出原样的内容。
    #[test]
    fn unzip_reads_it_back() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
        let dir = std::env::temp_dir().join(format!("muser-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let z = dir.join("a.zip");
        std::fs::write(&z, super::store(&[("x/a.txt".into(), b"hello".to_vec()), ("b.bin".into(), vec![0, 1, 2, 255])])).unwrap();
        if let Ok(o) = std::process::Command::new("unzip").arg("-p").arg(&z).arg("x/a.txt").output() {
            assert_eq!(o.stdout, b"hello");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
