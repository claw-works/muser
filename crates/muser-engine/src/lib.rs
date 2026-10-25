//! muser-engine —— 引擎适配层
//!
//! 把 [`muser_core::Project`] 编译成 manguebyte 的乐器/合成器并离线渲染为音频/WAV。
//! 这是「领域模型 → 声音」的翻译层，是三层架构的中间层。

pub mod automate;
pub mod effects;
pub mod instrument;
pub mod params;
pub mod render;

pub use instrument::{
    available as available_instruments, catalog as instrument_catalog, check as check_instrument_with, check_name as check_instrument,
    describe as describe_instrument, InstrumentInfo, NoteConvention, TrackInstrument,
};
pub use params::Automatable;
pub use render::{render, render_to_wav, EngineError, RenderSettings};

/// 完整校验：工程结构与效果器参数（core 描述表）+ 每条音轨的乐器名是否可用。
/// 不加载插件，`plugin:` 只做语法检查。错误信息带出错位置，可直接反馈给 AI。
pub fn validate(project: &muser_core::Project) -> Result<(), EngineError> {
    project.validate()?;
    for (i, track) in project.tracks.iter().enumerate() {
        instrument::check(&track.instrument, &track.instrument_params).map_err(|e| {
            EngineError::Core(muser_core::CoreError::Invalid(format!(
                "tracks[{i}].instrument: {e}"
            )))
        })?;
    }
    automate::check(project).map_err(|m| EngineError::Core(muser_core::CoreError::Invalid(m)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use manguebyte::prelude::AudioBuffer;
    use muser_core::{Clip, Note, Project, Track};

    /// 构造一个最小工程：808 鼓组，4 拍里 kick+snare。
    fn demo_project() -> Project {
        let mut p = Project::new("engine-test", 120.0);
        let mut track = Track::new("drums", "drums.808");
        let mut clip = Clip::new(0.0, 4.0);
        // kick(36) 在第1、3拍，snare(38) 在第2、4拍
        clip.notes.push(Note::new(36, 0.0, 0.25));
        clip.notes.push(Note::new(38, 1.0, 0.25));
        clip.notes.push(Note::new(36, 2.0, 0.25));
        clip.notes.push(Note::new(38, 3.0, 0.25));
        track.clips.push(clip);
        p.tracks.push(track);
        p
    }

    #[test]
    fn renders_nonempty_audio() {
        let p = demo_project();
        let settings = RenderSettings {
            tail_seconds: 0.5,
            ..RenderSettings::default()
        };
        let audio = render(&p, settings).expect("渲染应成功");
        // 应有合理时长：4拍@120bpm=2s + 0.5s 余响
        assert!(audio.num_frames() > 48_000, "时长应 > 1s");
        // 应真的发出了声音（峰值非零）
        assert!(audio.peak() > 0.0, "峰值应 > 0，实际 {}", audio.peak());
    }

    #[test]
    fn unknown_instrument_errors() {
        let mut p = Project::new("x", 120.0);
        p.tracks.push(Track::new("t", "does.not.exist"));
        let err = render(&p, RenderSettings::default()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("tracks[0].instrument") && msg.contains("does.not.exist"), "{msg}");
    }

    #[cfg(not(feature = "host"))]
    #[test]
    fn plugin_instrument_without_host_errors() {
        let mut p = Project::new("x", 120.0);
        p.tracks.push(Track::new("t", "plugin:/some/inst.vst3"));
        let err = render(&p, RenderSettings::default()).unwrap_err();
        match err {
            EngineError::Instrument(crate::instrument::InstrumentError::HostDisabled) => {}
            other => panic!("应报 HostDisabled，实际: {other:?}"),
        }
    }

    #[test]
    fn muted_track_is_silent() {
        let mut p = demo_project();
        p.tracks[0].muted = true;
        let audio = render(&p, RenderSettings::default()).unwrap();
        assert_eq!(audio.peak(), 0.0, "静音轨应无声");
    }

    #[test]
    fn poly_synth_renders() {
        let mut p = Project::new("poly", 120.0);
        let mut track = Track::new("lead", "synth.poly");
        let mut clip = Clip::new(0.0, 2.0);
        clip.notes.push(Note::new(60, 0.0, 1.0)); // 中央C 一拍
        track.clips.push(clip);
        p.tracks.push(track);
        let audio = render(&p, RenderSettings { tail_seconds: 0.5, ..Default::default() }).unwrap();
        assert!(audio.peak() > 0.0, "poly synth 应发声");
    }

    #[test]
    fn effects_chain_renders() {
        use muser_core::Effect;
        let mut p = demo_project();
        // 轨级加压缩器
        p.tracks[0].effects.push(Effect::Compressor {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            makeup_db: 0.0,
        });
        // 主总线加混响
        p.master_effects.push(Effect::Reverb {
            mix: 0.25,
            decay_seconds: 1.6,
            damping: 0.5,
            size: 0.5,
        });
        let audio = render(&p, RenderSettings { tail_seconds: 1.0, ..Default::default() })
            .expect("带效果链渲染应成功");
        assert!(audio.peak() > 0.0, "带效果链仍应发声");
    }

    #[test]
    fn master_reverb_adds_tail() {
        use muser_core::Effect;
        // 一个短促的鼓击，加大混响后，尾部应有残响（非零样本延续更久）。
        let mut dry = Project::new("dry", 120.0);
        let mut t = Track::new("d", "drums.808");
        let mut c = Clip::new(0.0, 1.0);
        c.notes.push(Note::new(36, 0.0, 0.1));
        t.clips.push(c);
        dry.tracks.push(t);

        let mut wet = dry.clone();
        wet.master_effects.push(Effect::Reverb {
            mix: 0.6,
            decay_seconds: 2.5,
            damping: 0.3,
            size: 0.8,
        });

        let s = RenderSettings { tail_seconds: 2.0, ..Default::default() };
        let dry_audio = render(&dry, s).unwrap();
        let wet_audio = render(&wet, s).unwrap();

        // 取后半段的能量：湿版应明显更高（有残响拖尾）。
        let tail_energy = |a: &manguebyte::prelude::AudioBuffer<f32>| -> f64 {
            let n = a.num_frames();
            let half = n / 2;
            let mut e = 0.0;
            for f in half..n {
                let s = a.channel(0)[f] as f64;
                e += s * s;
            }
            e
        };
        let dry_tail = tail_energy(&dry_audio);
        let wet_tail = tail_energy(&wet_audio);
        assert!(
            wet_tail > dry_tail * 2.0,
            "混响应在尾部注入明显能量: dry={dry_tail:.6}, wet={wet_tail:.6}"
        );
    }

    #[test]
    fn new_builtin_effects_render() {
        use muser_core::Effect;
        let mut p = demo_project();
        // 轨级 chorus + EQ
        p.tracks[0].effects.push(Effect::Chorus {
            rate_hz: 0.8,
            depth_ms: 3.0,
            mix: 0.4,
        });
        p.tracks[0].effects.push(Effect::Eq {
            shape: "high_shelf".to_string(),
            frequency: 6000.0,
            q: 0.707,
            gain_db: 3.0,
        });
        // 主总线 limiter
        p.master_effects.push(Effect::Limiter {
            ceiling_db: -0.3,
            release_ms: 50.0,
        });
        let audio = render(&p, RenderSettings { tail_seconds: 1.0, ..Default::default() })
            .expect("chorus/eq/limiter 渲染应成功");
        assert!(audio.peak() > 0.0, "应发声");
        // limiter 上限 -0.3 dBFS，峰值不应超过它（给一点点数值裕度）。
        let peak_db = manguebyte::prelude::decibels::gain_to_db(f64::from(audio.peak()));
        assert!(peak_db <= 0.0, "limiter 应把峰值压在 0dB 内, 实际 {peak_db:.2} dBFS");
    }

    #[test]
    fn phaser_flanger_render() {
        use muser_core::Effect;
        let mut p = demo_project();
        p.tracks[0].effects.push(Effect::Phaser {
            rate_hz: 0.5,
            feedback: 0.4,
            mix: 0.5,
        });
        p.master_effects.push(Effect::Flanger {
            rate_hz: 0.2,
            feedback: 0.7,
            mix: 0.5,
        });
        let audio = render(&p, RenderSettings { tail_seconds: 1.0, ..Default::default() })
            .expect("phaser/flanger 渲染应成功");
        assert!(audio.peak() > 0.0, "应发声");
    }

    #[test]
    fn eq_unknown_shape_errors() {
        use muser_core::Effect;
        let mut p = demo_project();
        p.master_effects.push(Effect::Eq {
            shape: "bogus".to_string(),
            frequency: 1000.0,
            q: 0.7,
            gain_db: 0.0,
        });
        let err = render(&p, RenderSettings::default()).unwrap_err();
        // 工程校验阶段（core 描述表）就会拦截，错误信息指明位置与可选值。
        let msg = err.to_string();
        assert!(matches!(err, EngineError::Core(_)), "{msg}");
        assert!(msg.contains("master_effects[0]") && msg.contains("eq.shape"), "{msg}");
    }

    #[test]
    fn catalog_instruments_make_sound() {
        // 抽查各类别的代表乐器都能真实发声。
        for name in [
            "guitar.nylon",
            "bass.finger",
            "bass.acid",
            "organ.jazz",
            "mallet.marimba",
            "strings.cello",
            "wind.flute",
            "choir.aah",
            "poly.moog_bass",
            "poly.warm_analog_pad",
        ] {
            let mut p = Project::new(name, 120.0);
            let mut t = Track::new("t", name);
            let mut c = Clip::new(0.0, 2.0);
            c.notes.push(Note::new(48, 0.0, 1.0));
            c.notes.push(Note::new(55, 0.0, 1.0));
            t.clips.push(c);
            p.tracks.push(t);
            let audio = render(&p, RenderSettings { tail_seconds: 0.5, ..Default::default() })
                .unwrap_or_else(|e| panic!("{name} 渲染失败: {e}"));
            assert!(audio.peak() > 1e-4, "{name} 应发声，峰值 {}", audio.peak());
        }
    }

    /// 默认（未启用 host 特性）：plugin 效果器应报 HostDisabled。
    #[cfg(not(feature = "host"))]
    #[test]
    fn plugin_without_host_feature_errors() {
        use muser_core::Effect;
        let mut p = demo_project();
        p.master_effects.push(Effect::Plugin {
            path: "/nonexistent/x.clap".to_string(),
            id: None,
            params: Default::default(),
        });
        let err = render(&p, RenderSettings::default()).unwrap_err();
        match err {
            EngineError::Effect(crate::effects::EffectError::HostDisabled) => {}
            other => panic!("应报 HostDisabled，实际: {other:?}"),
        }
    }

    /// 启用 host 特性但插件路径不存在：应报清晰的 PluginLoad 错误（而非 panic）。
    #[cfg(feature = "host")]
    #[test]
    fn plugin_missing_path_errors() {
        use muser_core::Effect;
        let mut p = demo_project();
        p.master_effects.push(Effect::Plugin {
            path: "/definitely/not/here.clap".to_string(),
            id: None,
            params: Default::default(),
        });
        let err = render(&p, RenderSettings::default()).unwrap_err();
        match err {
            EngineError::Effect(crate::effects::EffectError::PluginLoad { .. }) => {}
            other => panic!("应报 PluginLoad，实际: {other:?}"),
        }
    }

    fn render_one(instrument: &str, params: serde_json::Value, notes: &[(u8, f64, f64)]) -> Result<AudioBuffer<f32>, EngineError> {
        let mut p = Project::new("t", 120.0);
        let mut t = Track::new("t", instrument);
        t.instrument_params = params.as_object().cloned().unwrap_or_default();
        let mut c = Clip::new(0.0, 4.0);
        for &(pitch, start, len) in notes {
            c.notes.push(Note::new(pitch, start, len));
        }
        t.clips.push(c);
        p.tracks.push(t);
        render(&p, RenderSettings { tail_seconds: 1.0, ..Default::default() })
    }

    fn rms(a: &AudioBuffer<f32>) -> f64 {
        let n = a.num_frames().max(1);
        (a.channel(0).iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / n as f64).sqrt()
    }

    /// 高频能量占比的粗略指标：一阶差分能量 / 信号能量。越亮越大。
    fn brightness(a: &AudioBuffer<f32>) -> f64 {
        let x = a.channel(0);
        let diff: f64 = x.windows(2).map(|w| f64::from(w[1] - w[0]).powi(2)).sum();
        let total: f64 = x.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>().max(1e-12);
        diff / total
    }

    #[test]
    fn every_instrument_accepts_its_own_current_params() {
        // 把 describe 给出的当前值原样写回，必须通过校验——保证范围与预设一致。
        for info in crate::instrument::catalog() {
            let full = crate::instrument::describe(&info.name).unwrap();
            if full.notes == crate::NoteConvention::GmDrums {
                continue;
            }
            let mut params = muser_core::InstrumentParams::new();
            for pi in &full.params {
                if !pi.current.is_null() {
                    params.insert(pi.spec.name.to_string(), pi.current.clone());
                }
            }
            crate::instrument::check(&info.name, &params)
                .unwrap_or_else(|e| panic!("{} 的当前参数应能写回: {e}\n{params:?}", info.name));
        }
    }

    #[test]
    fn poly_cutoff_changes_brightness() {
        let notes = [(48, 0.0, 2.0), (55, 0.0, 2.0)];
        let dark = render_one("poly.moog_bass", serde_json::json!({ "cutoff": 200 }), &notes).unwrap();
        let bright = render_one("poly.moog_bass", serde_json::json!({ "cutoff": 8000 }), &notes).unwrap();
        assert!(brightness(&bright) > brightness(&dark) * 1.5, "cutoff 应改变明暗: {} vs {}", brightness(&dark), brightness(&bright));
    }

    #[test]
    fn release_lengthens_tail() {
        // 音符 1 拍（0.5s）；比较 0.8s 之后的能量。
        let notes = [(48, 0.0, 1.0)];
        let tail = |a: &AudioBuffer<f32>| a.channel(0)[38_400..].iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        for inst in ["strings.cello", "guitar.nylon", "choir.aah", "poly.warm_analog_pad"] {
            let short = render_one(inst, serde_json::json!({ "release": 0.15 }), &notes).unwrap();
            let long = render_one(inst, serde_json::json!({ "release": 3.0 }), &notes).unwrap();
            assert!(tail(&long) > tail(&short) * 3.0, "{inst}: release 应拉长尾音 {} vs {}", tail(&short), tail(&long));
        }
    }

    #[test]
    fn bowed_release_floor_is_enforced() {
        let e = render_one("strings.violin", serde_json::json!({ "release": 0.05 }), &[]).unwrap_err().to_string();
        assert!(e.contains("超出范围"), "{e}");
    }

    #[test]
    fn param_errors_are_precise() {
        let e = render_one("poly.moog_bass", serde_json::json!({ "cutof": 500 }), &[]).unwrap_err().to_string();
        assert!(e.contains("cutof") && e.contains("可用") && e.contains("cutoff"), "{e}");
        let e = render_one("strings.cello", serde_json::json!({ "vibrato_depth": 500 }), &[]).unwrap_err().to_string();
        assert!(e.contains("vibrato_depth") && e.contains("超出范围"), "{e}");
        let e = render_one("organ.jazz", serde_json::json!({ "drawbars": "999" }), &[]).unwrap_err().to_string();
        assert!(e.contains("drawbars") && e.contains("9 位"), "{e}");
        let e = render_one("organ.jazz", serde_json::json!({ "percussion": "fourth" }), &[]).unwrap_err().to_string();
        assert!(e.contains("second"), "{e}");
    }

    #[test]
    fn named_and_typed_values() {
        render_one("choir.aah", serde_json::json!({ "vowel": "o", "singers": 3 }), &[(60, 0.0, 1.0)]).unwrap();
        render_one("organ.gospel", serde_json::json!({ "drawbars": "888000000", "percussion": "third" }), &[(60, 0.0, 1.0)]).unwrap();
        render_one("wind.trombone", serde_json::json!({ "portamento": true }), &[(50, 0.0, 1.0)]).unwrap();
        let e = render_one("choir.aah", serde_json::json!({ "singers": 2.5 }), &[]).unwrap_err().to_string();
        assert!(e.contains("整数"), "{e}");
    }

    #[test]
    fn drum_piece_params() {
        let notes = [(36, 0.0, 0.25)];
        let base = render_one("drums.808", serde_json::json!({}), &notes).unwrap();
        let quiet = render_one("drums.808", serde_json::json!({ "kick": { "level": 0.25 } }), &notes).unwrap();
        assert!(rms(&quiet) < rms(&base) * 0.5, "kick.level 应降低音量");
        let e = render_one("drums.606", serde_json::json!({ "clap": { "level": 1 } }), &[]).unwrap_err().to_string();
        assert!(e.contains("没有鼓键 39"), "{e}");
        let e = render_one("drums.808", serde_json::json!({ "bogus": { "level": 1 } }), &[]).unwrap_err().to_string();
        assert!(e.contains("未知鼓件"), "{e}");
    }

    #[test]
    fn acid_accent_is_louder() {
        let mk = |accent: bool| {
            let mut p = Project::new("a", 120.0);
            let mut t = Track::new("b", "bass.acid");
            let mut c = Clip::new(0.0, 2.0);
            let mut n = Note::new(36, 0.0, 0.5);
            n.accent = accent;
            c.notes.push(n);
            t.clips.push(c);
            p.tracks.push(t);
            render(&p, RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap()
        };
        assert!(rms(&mk(true)) > rms(&mk(false)) * 1.2, "重音应更响");
    }

    #[test]
    fn sfz_missing_file_errors_on_validate() {
        let mut p = Project::new("s", 120.0);
        p.tracks.push(Track::new("s", "sfz:/no/such/file.sfz"));
        let e = crate::validate(&p).unwrap_err().to_string();
        assert!(e.contains("找不到 SFZ 文件"), "{e}");
    }

    /// 用本机的 freepats 音色库做真实测试；不存在则跳过。
    #[test]
    fn sfz_real_library_makes_sound() {
        let path = "/data/Music/SFZ-samples/freepats/FM-Piano1/FM-Piano1 SFZ+FLAC-20190916/FM-Piano1 20190916.sfz";
        if !std::path::Path::new(path).is_file() {
            eprintln!("跳过：未找到 {path}");
            return;
        }
        let audio = render_one(&format!("sfz:{path}"), serde_json::json!({}), &[(60, 0.0, 1.0), (64, 0.0, 1.0)]).unwrap();
        assert!(audio.peak() > 1e-3, "sfz 应发声，峰值 {}", audio.peak());
    }

    fn auto_project(instrument: &str, notes: &[(u8, f64, f64)], automation: serde_json::Value) -> Project {
        let mut p = Project::new("auto", 120.0);
        let mut t = Track::new("t", instrument);
        let mut c = Clip::new(0.0, 8.0);
        for &(pitch, start, len) in notes {
            c.notes.push(Note::new(pitch, start, len));
        }
        t.clips.push(c);
        t.automation = serde_json::from_value(automation).unwrap();
        p.tracks.push(t);
        p
    }

    fn seg_brightness(a: &AudioBuffer<f32>, from: usize, to: usize) -> f64 {
        let x = &a.channel(0)[from..to];
        let diff: f64 = x.windows(2).map(|w| f64::from(w[1] - w[0]).powi(2)).sum();
        diff / x.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>().max(1e-12)
    }
    fn seg_energy(a: &AudioBuffer<f32>, ch: usize, from: usize, to: usize) -> f64 {
        a.channel(ch)[from..to].iter().map(|s| f64::from(*s).powi(2)).sum()
    }
    const SEC: usize = 48_000;
    fn quiet() -> RenderSettings {
        RenderSettings { tail_seconds: 0.5, ..Default::default() }
    }

    #[test]
    fn automation_cutoff_sweep_brightens() {
        // 一个 4 秒长音，cutoff 从 200 扫到 6000（exp）。
        let p = auto_project("poly.moog_bass", &[(36, 0.0, 8.0)], serde_json::json!([
            { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [8, 6000]], "curve": "exp" }
        ]));
        let a = render(&p, quiet()).unwrap();
        let early = seg_brightness(&a, SEC / 2, SEC);
        let late = seg_brightness(&a, 3 * SEC, 7 * SEC / 2);
        assert!(late > early * 3.0, "扫频后应明显变亮: {early} → {late}");
    }

    #[test]
    fn automation_gain_fade_in() {
        let p = auto_project("synth.poly", &[(60, 0.0, 8.0)], serde_json::json!([
            { "target": "gain_db", "shape": "points", "points": [[0, -60], [8, 0]] }
        ]));
        let a = render(&p, quiet()).unwrap();
        assert!(seg_energy(&a, 0, 3 * SEC, 4 * SEC) > seg_energy(&a, 0, SEC / 4, SEC) * 100.0);
    }

    #[test]
    fn automation_pan_lfo_alternates() {
        // 声像 LFO 方波，周期 4 拍（2 秒）：前 1 秒在右（max），后 1 秒在左。
        let p = auto_project("synth.poly", &[(60, 0.0, 8.0)], serde_json::json!([
            { "target": "pan", "shape": "lfo", "wave": "square", "period_beats": 4, "min": -1, "max": 1 }
        ]));
        let a = render(&p, quiet()).unwrap();
        let (l1, r1) = (seg_energy(&a, 0, SEC / 4, SEC * 3 / 4), seg_energy(&a, 1, SEC / 4, SEC * 3 / 4));
        let (l2, r2) = (seg_energy(&a, 0, SEC * 5 / 4, SEC * 7 / 4), seg_energy(&a, 1, SEC * 5 / 4, SEC * 7 / 4));
        assert!(r1 > l1 * 100.0 && l2 > r2 * 100.0, "应左右交替: ({l1},{r1}) ({l2},{r2})");
    }

    #[test]
    fn automation_effect_param() {
        // eq lowpass 的 frequency 从 300 扫到 12000：后段更亮。
        let mut p = auto_project("synth.poly", &[(60, 0.0, 8.0), (64, 0.0, 8.0)], serde_json::json!([
            { "target": "effects[0]", "param": "frequency", "shape": "points", "points": [[0, 300], [8, 12000]], "curve": "exp" }
        ]));
        p.tracks[0].effects.push(muser_core::Effect::Eq { shape: "lowpass".into(), frequency: 300.0, q: 0.707, gain_db: 0.0 });
        let a = render(&p, quiet()).unwrap();
        assert!(seg_brightness(&a, 3 * SEC, 7 * SEC / 2) > seg_brightness(&a, SEC / 2, SEC) * 3.0);
    }

    #[test]
    fn automation_master_effect_and_gain() {
        let mut p = auto_project("synth.poly", &[(60, 0.0, 2.0)], serde_json::json!([]));
        p.master_effects.push(muser_core::Effect::Reverb { mix: 0.0, decay_seconds: 2.0, damping: 0.5, size: 0.6 });
        p.automation = serde_json::from_value(serde_json::json!([
            { "target": "master_effects[0]", "param": "mix", "shape": "points", "points": [[0, 0], [2, 0.8]] },
            { "target": "master_gain_db", "shape": "points", "points": [[0, 0], [8, -6]] }
        ])).unwrap();
        render(&p, quiet()).unwrap();
    }

    #[test]
    fn automation_random_seed_reproducible() {
        let mk = |seed: u64, explicit: Option<u64>| {
            let mut auto = serde_json::json!({ "target": "instrument", "param": "cutoff", "shape": "random",
                "style": "walk", "step_beats": 0.5, "min": 200, "max": 4000 });
            if let Some(s) = explicit {
                auto["seed"] = serde_json::json!(s);
            }
            let mut p = auto_project("poly.moog_bass", &[(36, 0.0, 8.0)], serde_json::json!([auto]));
            p.seed = seed;
            render(&p, quiet()).unwrap()
        };
        let same = |a: &AudioBuffer<f32>, b: &AudioBuffer<f32>| a.channel(0) == b.channel(0);
        assert!(same(&mk(1, None), &mk(1, None)), "同一工程种子应完全相同");
        assert!(!same(&mk(1, None), &mk(2, None)), "工程种子不同应不同");
        assert!(same(&mk(1, Some(7)), &mk(2, Some(7))), "显式 seed 不受工程种子影响");
    }

    #[test]
    fn automation_drum_piece_next_hit() {
        // 每拍一个底鼓，kick.tune 从 0 跳到 +12：后段音高更高（过零率更高）。
        let hits: Vec<(u8, f64, f64)> = (0..8).map(|b| (36u8, b as f64, 0.25)).collect();
        let p = auto_project("drums.808", &hits, serde_json::json!([
            { "target": "instrument", "param": "kick.tune", "shape": "points", "points": [[0, 0], [3.9, 0], [4, 12]], "curve": "step" }
        ]));
        let a = render(&p, quiet()).unwrap();
        let zc = |from: usize, to: usize| a.channel(0)[from..to].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!(zc(SEC / 2 * 4, SEC / 2 * 4 + 8000) > zc(0, 8000), "调高后过零应更多");
    }

    #[test]
    fn automation_validation_errors() {
        let err = |instrument: &str, auto: serde_json::Value| {
            crate::validate(&auto_project(instrument, &[], serde_json::json!([auto]))).unwrap_err().to_string()
        };
        let e = err("poly.moog_bass", serde_json::json!({ "target": "instrument", "param": "cutof", "shape": "points", "points": [[0, 1]] }));
        assert!(e.contains("automation[0]") && e.contains("cutoff"), "{e}");
        let e = err("poly.moog_bass", serde_json::json!({ "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 10], [4, 99999]] }));
        assert!(e.contains("points[0]") && e.contains("超出范围"), "{e}");
        let e = err("strings.cello", serde_json::json!({ "target": "instrument", "param": "players", "shape": "lfo", "period_beats": 4, "min": 1, "max": 3 }));
        assert!(e.contains("不能自动化"), "{e}");
        let e = err("synth.poly", serde_json::json!({ "target": "effects[0]", "param": "mix", "shape": "points", "points": [[0, 0]] }));
        assert!(e.contains("越界"), "{e}");
        let e = err("synth.poly", serde_json::json!({ "target": "master_gain_db", "shape": "points", "points": [[0, 0]] }));
        assert!(e.contains("工程级"), "{e}");
        let e = err("synth.poly", serde_json::json!({ "target": "pan", "shape": "points", "points": [[0, -1], [4, 1]], "curve": "exp" }));
        assert!(e.contains("exp"), "{e}");
        let e = err("poly.bell_pluck", serde_json::json!({ "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 500]] }));
        assert!(e.contains("滤波器"), "{e}");
        // 元音可用名字
        crate::validate(&auto_project("choir.aah", &[], serde_json::json!([
            { "target": "instrument", "param": "vowel", "shape": "points", "points": [[0, "a"], [4, "o"], [8, "u"]] }
        ]))).unwrap();
    }
}
