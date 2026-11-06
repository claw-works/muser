//! `muser migrate`：把为旧版 muser 写的工程换算到当前版本，听起来和在旧版里一样。
//!
//! 工程文件里没有"为哪个版本写的"标记，所以由调用方说出来源版本（`--from 0.3.0`）；
//! 每条迁移记着它从哪个版本起生效，只跑来源版本之后的那些。在 JSON 上改（保留工程里 muser 不认识的字段）。

use serde_json::Value;

use crate::instrument::calibration_db;

/// 一条迁移：从这个版本起行为变了（来源版本早于它才需要），一句说明。
pub struct Migration {
    pub since: &'static str,
    pub what: &'static str,
    apply: fn(&mut Value, &mut Vec<Change>),
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    since: "0.3.1",
    what: "built-in instrument level calibration (gain_db 0 = about -20 LUFS): the gain_db and gain_db automation of built-in instrument tracks have the instrument's calibration gain subtracted",
    apply: calibrate,
}];

/// 一处改动（给人看的一行）。
pub struct Change {
    pub track: String,
    pub detail: String,
}

/// 把 `project` 从 `from` 版本换算到当前版本，返回改了什么。`from` 写成 `0.3.0` 或 `v0.3.0`。
pub fn migrate(project: &mut Value, from: &str) -> Result<Vec<Change>, String> {
    let from = parse_version(from).ok_or_else(|| format!("unrecognized version '{from}': write it like 0.3.0"))?;
    let mut changes = Vec::new();
    for m in MIGRATIONS.iter().filter(|m| from < parse_version(m.since).expect("迁移表里的版本")) {
        (m.apply)(project, &mut changes);
    }
    Ok(changes)
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// 0.3.1：内置乐器在输出端补了校准增益 `trim`；旧工程的 `gain_db` 减去 `trim`，平衡与旧版一样。
fn calibrate(project: &mut Value, changes: &mut Vec<Change>) {
    let Some(tracks) = project.get_mut("tracks").and_then(Value::as_array_mut) else { return };
    for (i, t) in tracks.iter_mut().enumerate() {
        let name = t.get("instrument").and_then(Value::as_str).unwrap_or_default().to_string();
        let params = t.get("instrument_params").and_then(Value::as_object).cloned().unwrap_or_default();
        let trim = calibration_db(&name, &params);
        if trim.abs() < 0.05 {
            continue;
        }
        let label = t.get("name").and_then(Value::as_str).map_or_else(|| format!("tracks[{i}]"), str::to_string);
        let shift = |db: f64| (((db - trim) * 10.0).round() / 10.0).max(-80.0); // −80 dB 封底，再低也听不见
        let old = t.get("gain_db").and_then(Value::as_f64).unwrap_or(0.0);
        let new = shift(old);
        t["gain_db"] = Value::from(new);
        let mut detail = format!("{name}: gain_db {old} -> {new} (the new version adds {trim:+.1} dB at the instrument output)");
        let mut autos = 0;
        if let Some(list) = t.get_mut("automation").and_then(Value::as_array_mut) {
            for a in list.iter_mut().filter(|a| a.get("target").and_then(Value::as_str) == Some("gain_db")) {
                if let Some(points) = a.get_mut("points").and_then(Value::as_array_mut) {
                    for p in points.iter_mut().filter_map(Value::as_array_mut) {
                        if let Some(v) = p.get(1).and_then(Value::as_f64) {
                            p[1] = Value::from(shift(v));
                        }
                    }
                }
                for k in ["min", "max"] {
                    if let Some(v) = a.get(k).and_then(Value::as_f64) {
                        a[k] = Value::from(shift(v));
                    }
                }
                autos += 1;
            }
        }
        if autos > 0 {
            detail += &format!(", {autos} gain_db automation lanes converted the same way");
        }
        changes.push(Change { track: label, detail });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calibration_shifts_builtin_gain_and_its_automation_only() {
        let trim = calibration_db("drums.909", &Default::default());
        assert!(trim.abs() > 0.05, "drums.909 应有校准增益");
        let mut p = json!({ "tracks": [
            { "name": "kick", "instrument": "drums.909", "gain_db": -3.0,
              "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, -3.0], [4, -90.0]] },
                             { "target": "gain_db", "shape": "lfo", "min": -6.0, "max": 0.0 },
                             { "target": "pan", "shape": "points", "points": [[0, 0.5]] }] },
            { "name": "vst", "instrument": "plugin:/x.vst3", "gain_db": -2.0 },
            { "name": "sample", "instrument": "audio", "gain_db": 1.0 },
        ]});
        let changes = migrate(&mut p, "0.3.0").unwrap();
        assert_eq!(changes.len(), 1);
        let want = |db: f64| (((db - trim) * 10.0).round() / 10.0).max(-80.0);
        let t = &p["tracks"][0];
        assert_eq!(t["gain_db"], json!(want(-3.0)));
        assert_eq!(t["automation"][0]["points"][0][1], json!(want(-3.0)));
        assert_eq!(t["automation"][0]["points"][1][1], json!(-80.0));
        assert_eq!(t["automation"][1]["min"], json!(want(-6.0)));
        assert_eq!(t["automation"][2]["points"][0][1], json!(0.5), "别的自动化不动");
        assert_eq!(p["tracks"][1]["gain_db"], json!(-2.0));
        assert_eq!(p["tracks"][2]["gain_db"], json!(1.0));
    }

    #[test]
    fn nothing_to_do_from_the_calibrated_version_on() {
        let mut p = json!({ "tracks": [{ "instrument": "drums.909", "gain_db": -3.0 }] });
        assert!(migrate(&mut p, "v0.3.1").unwrap().is_empty());
        assert!(migrate(&mut p, "0.4").unwrap().is_empty());
        assert_eq!(p["tracks"][0]["gain_db"], json!(-3.0));
        assert!(migrate(&mut p, "abc").is_err());
    }
}
