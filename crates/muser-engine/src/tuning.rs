//! 工程的 `tuning`（见 [`muser_core::tuning`]）→ manguebyte 的调律表。

use manguebyte::dsp::tuning::{KeyboardMapping, ScalaScale, Scale, Temperament, TuningSettings};
use manguebyte::dsp::Tuning;
use muser_core::Project;

/// 工程的调律；没写或就是十二平均律、A4 = 440 时为 `None`（乐器照常）。文件找不到、格式不对时报错。
pub fn build(project: &Project) -> Result<Option<Tuning>, String> {
    match project.tuning.as_ref() {
        Some(t) => build_one(project, t),
        None => Ok(None),
    }
}

/// 段落里换的调律，顺序同 [`Project::tuning_changes`]（起点在那里）。没有段落写 `tuning` 时为空。
/// 十二平均律、A4 = 440 的写成标准调律（换回平均律也要发出去）。
pub fn changes(project: &Project) -> Result<Vec<Tuning>, String> {
    project.tuning_changes().into_iter().map(|(_, t)| Ok(build_one(project, t)?.unwrap_or_default())).collect()
}

/// 一份 `tuning` → 调律表；是标准的十二平均律时为 `None`。
fn build_one(project: &Project, t: &muser_core::tuning::Tuning) -> Result<Option<Tuning>, String> {
    if t.is_standard() {
        return Ok(None);
    }
    t.validate()?;
    let key = muser_core::tuning::pitch_class(&t.key).unwrap_or(0);
    let temperament = |temperament: Temperament| Scale::Temperament { temperament, key };
    let scale = match t.scale.trim().to_ascii_lowercase().as_str() {
        "pythagorean" => temperament(Temperament::pythagorean()),
        "meantone" => temperament(Temperament::quarter_comma_meantone()),
        "werckmeister_iii" => temperament(Temperament::werckmeister_iii()),
        "kirnberger_iii" => temperament(Temperament::kirnberger_iii()),
        "vallotti" => temperament(Temperament::vallotti()),
        "young" => temperament(Temperament::young()),
        "just" => temperament(Temperament::just()),
        "custom" => {
            let d = t.deviations.as_deref().unwrap_or_default();
            let mut dev = [0.0; 12];
            dev.copy_from_slice(&d[..12]);
            temperament(Temperament::from_deviations(dev))
        }
        "scala" => {
            let read = |file: &str| {
                let path = crate::audio::resolve(project, file);
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("tuning: failed to read '{}': {e} (relative paths are relative to the project file's directory)", path.display()))
            };
            let scale = ScalaScale::parse(&read(t.scala.as_deref().unwrap_or_default())?).map_err(|e| format!("tuning.scala: {e}"))?;
            let mapping = match &t.kbm {
                Some(f) => Some(KeyboardMapping::parse(&read(f)?).map_err(|e| format!("tuning.kbm: {e}"))?),
                None => None,
            };
            Scale::Scala { scale, mapping }
        }
        _ => Scale::Equal { divisions: t.divisions, period_cents: t.period_cents },
    };
    let settings =
        TuningSettings { scale, reference_key: t.reference_key, reference_hz: t.reference_hz, detune_cents: t.detune_cents, ..TuningSettings::default() };
    settings.build().map(Some).map_err(|e| format!("tuning: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning(v: serde_json::Value) -> Tuning {
        let mut p = Project::new("t", 120.0);
        p.tuning = Some(serde_json::from_value(v).unwrap());
        build(&p).unwrap().unwrap()
    }

    #[test]
    fn builds_temperaments_and_divisions() {
        // 纯律：主音 C 上的大三度 5/4 = 386.3 音分（平均律 400）。
        let just = tuning(serde_json::json!({ "scale": "just" }));
        assert!(((just.key_pitch(64) - just.key_pitch(60)) * 100.0 - 386.31).abs() < 0.01);
        // 24 平均律：一个键 = 半个半音。
        let quarter = tuning(serde_json::json!({ "scale": "equal", "divisions": 24 }));
        assert!((quarter.key_pitch(70) - quarter.key_pitch(69) - 0.5).abs() < 1e-9);
        // 自定义：E（主音 C 起第 4 个半音）低 50 音分。
        let maqam = tuning(serde_json::json!({ "scale": "custom", "deviations": [0, 0, 0, 0, -50, 0, 0, 0, 0, 0, 0, 0] }));
        assert!((maqam.key_pitch(64) - maqam.key_pitch(60) - 3.5).abs() < 1e-9);
        // 巴洛克音高：A4 = 415。
        let baroque = tuning(serde_json::json!({ "reference_hz": 415 }));
        assert!((baroque.key_frequency(69) - 415.0).abs() < 1e-9);
        let mut p = Project::new("t", 120.0);
        assert!(build(&p).unwrap().is_none());
        p.tuning = Some(serde_json::from_value(serde_json::json!({ "scale": "scala", "scala": "/nope.scl" })).unwrap());
        assert!(build(&p).unwrap_err().contains("failed to read"));
    }

    /// 段落换调律：第一段照工程的（平均律），第二段起按纯律；和整首用那种律制的渲染对应的部分一样。
    #[test]
    fn a_section_can_change_the_tuning() {
        let notes = serde_json::json!([{ "pitch": 64, "start": 0, "length": 0.9 }, { "pitch": 64, "start": 4, "length": 0.9 }]);
        let render = |sections: serde_json::Value, tuning: Option<serde_json::Value>| {
            let mut v = serde_json::json!({ "tempo": 120, "sections": sections, "form": ["a", "b"],
                "tracks": [{ "instrument": "synth.poly", "clips": [{ "start": 0, "length": 8, "notes": notes }] }] });
            if let Some(t) = tuning {
                v["tuning"] = t;
            }
            crate::render(&serde_json::from_value(v).unwrap(), crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap()
        };
        let plain = serde_json::json!({ "a": { "bars": 1 }, "b": { "bars": 1 } });
        let switch = render(serde_json::json!({ "a": { "bars": 1 }, "b": { "bars": 1, "tuning": { "scale": "just" } } }), None);
        let equal = render(plain.clone(), None);
        let just = render(plain, Some(serde_json::json!({ "scale": "just" })));
        let bar = 96_000; // 120 BPM、4/4：一小节 2 秒
        let part = |b: &manguebyte::prelude::AudioBuffer<f32>, r: std::ops::Range<usize>| b.channel(0)[r].to_vec();
        assert_eq!(part(&switch, 0..bar), part(&equal, 0..bar), "第一段还是平均律");
        assert_eq!(part(&switch, bar..switch.num_frames()), part(&just, bar..just.num_frames()), "第二段起按纯律");
        assert_ne!(part(&switch, bar..2 * bar), part(&equal, bar..2 * bar));
        // 换回平均律：写 equal 的段落也发出去。
        let back = render(serde_json::json!({ "a": { "bars": 1, "tuning": { "scale": "just" } }, "b": { "bars": 1, "tuning": { "scale": "equal" } } }), None);
        assert_eq!(part(&back, bar..back.num_frames()), part(&equal, bar..equal.num_frames()));
    }

    /// 有音高的乐器照调律发音，鼓组不变。
    #[test]
    fn pitched_instruments_follow_the_tuning_and_drums_do_not() {
        let render = |inst: &str, tuning: Option<serde_json::Value>| {
            let mut v = serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": inst,
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 64, "start": 0, "length": 1 }, { "pitch": 38, "start": 1, "length": 0.5 }] }] }] });
            if let Some(t) = tuning {
                v["tuning"] = t;
            }
            crate::render(&serde_json::from_value(v).unwrap(), crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap()
        };
        let just = Some(serde_json::json!({ "scale": "just" }));
        let same = |a: &manguebyte::prelude::AudioBuffer<f32>, b: &manguebyte::prelude::AudioBuffer<f32>| (0..2).all(|c| a.channel(c) == b.channel(c));
        assert!(!same(&render("synth.poly", None), &render("synth.poly", just.clone())));
        assert!(same(&render("drums.808", None), &render("drums.808", just)));
    }
}
