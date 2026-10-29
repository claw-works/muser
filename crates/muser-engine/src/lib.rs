//! muser-engine —— 引擎适配层
//!
//! 把 [`muser_core::Project`] 编译成 manguebyte 的乐器/合成器并离线渲染为音频/WAV。
//! 这是「领域模型 → 声音」的翻译层，是三层架构的中间层。

pub mod abc_export;
pub mod analyze;
pub mod arrange;
pub mod audio;
pub mod audition;
pub mod automate;
pub mod drummer;
pub mod effects;
pub mod hum;
pub mod instrument;
pub mod loop_library;
pub mod midi;
pub mod mix;
pub mod params;
pub mod poly_patch;
pub mod poly_library;
#[cfg(feature = "devices")]
pub mod playback;
pub mod plugin;
#[cfg(all(feature = "host", target_os = "macos"))]
pub mod plugin_edit;
pub mod profile;
pub mod vital;

/// 当前线程正在渲染的工程速度与拍号，交给插件的走带（tempo 同步的 LFO、延迟等）。
pub mod transport {
    use std::cell::Cell;

    thread_local! {
        static CURRENT: Cell<(f64, u16, u16)> = const { Cell::new((120.0, 4, 4)) };
    }

    /// 设置当前线程的速度与拍号；返回的守卫在离开作用域时恢复原值。
    pub fn set(tempo: f64, numerator: u16, denominator: u16) -> Guard {
        let old = CURRENT.with(|c| c.replace((tempo, numerator, denominator)));
        Guard(old)
    }

    /// 从工程开头开始播放的走带。
    pub fn start() -> manguebyte::prelude::Transport {
        let (tempo, n, d) = CURRENT.with(|c| c.get());
        manguebyte::prelude::Transport { time_signature: Some((n, d)), ..manguebyte::prelude::Transport::playing_at(tempo) }
    }

    pub struct Guard((f64, u16, u16));

    impl Drop for Guard {
        fn drop(&mut self) {
            CURRENT.with(|c| c.set(self.0));
        }
    }
}
pub mod render;
pub mod sfz;

pub use instrument::{
    available as available_instruments, catalog as instrument_catalog, check as check_instrument_with, check_name as check_instrument,
    describe as describe_instrument, InstrumentInfo, NoteConvention, TrackInstrument,
};
pub use params::Automatable;
/// 渲染结果用的立体声缓冲（来自 manguebyte）。
pub use manguebyte::prelude::AudioBuffer;
pub use mix::{mix_report, MixReport, NodeReport};
pub use render::{render, render_cached, render_loudness, render_metered, Level, NodeLoudness, render_to_wav, render_to_wav_with_progress, EngineError, Meter, RenderCache, RenderSettings, Renderer, METER_WINDOW};

/// 完整校验：工程结构与效果器参数（core 描述表）+ 每条音轨的乐器名是否可用。
/// 不加载插件，`plugin:` 只做语法检查。错误信息带出错位置，可直接反馈给 AI。
/// 一个 clip 的全部音符（相对 clip 起点）：`notes` 加上 `abc`、`grid`、`drummer` 展开的音符。
/// `drummer` 没写种子时由工程 seed 与音轨、clip 位置推导。
pub fn clip_notes(project: &muser_core::Project, ti: usize, ci: usize) -> Result<Vec<muser_core::Note>, String> {
    let track = &project.tracks[ti];
    let clip = &track.clips[ci];
    let mut notes = clip.all_notes()?.into_owned();
    if let Some(d) = &clip.drummer {
        let seed = d.seed.unwrap_or_else(|| muser_core::automation::derive_seed(project.seed, Some(ti), 0x4452_0000 + ci));
        notes.extend(drummer::expand(d, clip.length.0, seed, project.base_dir.as_deref())?);
    }
    if !clip.transform.is_empty() {
        let seed = muser_core::automation::derive_seed(project.seed, Some(ti), 0x5452_0000 + ci);
        let drums = track.instrument.trim().starts_with("drums.");
        arrange::apply(&clip.transform, &mut notes, &mut clip.length.0.clone(), 0, &[], drums, seed);
    }
    Ok(notes)
}

/// 只校验谱面（工程结构、音符写法、鼓手展开），不加载乐器、插件、音频文件。导出 MIDI / ABC 用：
/// 本机没有装工程用到的 SFZ 音色库或插件时也能导出。
pub fn validate_score(project: &muser_core::Project) -> Result<(), EngineError> {
    project.validate()?;
    // 编曲层能展开（和弦、chordal、鼓手都写对了）。
    let project = &*arrange::arranged(project).map_err(|e| EngineError::Core(muser_core::CoreError::Invalid(e)))?;
    for (ti, track) in project.tracks.iter().enumerate() {
        for (ci, clip) in track.clips.iter().enumerate() {
            if clip.drummer.is_some() {
                clip_notes(project, ti, ci).map_err(|e| EngineError::Core(muser_core::CoreError::Invalid(format!("tracks[{ti}].clips[{ci}].{e}"))))?;
            }
        }
    }
    Ok(())
}

pub fn validate(project: &muser_core::Project) -> Result<(), EngineError> {
    validate_score(project)?;
    for (ti, track) in project.tracks.iter().enumerate() {
        for (ci, clip) in track.clips.iter().enumerate() {
            if let Some(a) = &clip.audio {
                let path = audio::resolve(project, &a.file);
                if !path.is_file() {
                    return Err(EngineError::Core(muser_core::CoreError::Invalid(format!(
                        "tracks[{ti}].clips[{ci}].audio: 找不到音频文件 '{}'（相对路径相对工程文件所在目录）",
                        path.display()
                    ))));
                }
            }
        }
    }
    for (i, track) in project.tracks.iter().enumerate() {
        instrument::check(&track.instrument, &track.instrument_params).map_err(|e| {
            EngineError::Core(muser_core::CoreError::Invalid(format!(
                "tracks[{i}].instrument: {e}"
            )))
        })?;
    }
    // 插件效果器的参数（host 特性下会加载插件检查参数名与取值）。
    let plugin_fx = |fx: &muser_core::Effect, at: String| -> Result<(), EngineError> {
        if let muser_core::Effect::Plugin { path, id, params } = fx {
            plugin::check_params(path, id.as_deref(), params)
                .map_err(|m| EngineError::Core(muser_core::CoreError::Invalid(format!("{at}.params: {m}"))))?;
        }
        Ok(())
    };
    for (i, fx) in project.master_effects.iter().enumerate() {
        plugin_fx(fx, format!("master_effects[{i}]"))?;
    }
    for (ti, track) in project.tracks.iter().enumerate() {
        for (i, fx) in track.effects.iter().enumerate() {
            plugin_fx(fx, format!("tracks[{ti}].effects[{i}]"))?;
        }
    }
    for (bi, bus) in project.buses.iter().enumerate() {
        for (i, fx) in bus.effects.iter().enumerate() {
            plugin_fx(fx, format!("buses[{bi}].effects[{i}]"))?;
        }
    }
    automate::check(project).map_err(|m| EngineError::Core(muser_core::CoreError::Invalid(m)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypassed_effect_passes_sound_through() {
        let project = |fx: serde_json::Value| -> muser_core::Project {
            serde_json::from_value(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": "poly.wurly", "effects": fx,
                "automation": [{ "target": "effects[1]", "param": "frequency", "shape": "points", "points": [[0, 400], [2, 2000]] }],
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 60, "start": 0, "length": 1 }] }] }] })).unwrap()
        };
        let render = |p: &muser_core::Project| crate::render(p, RenderSettings::default()).unwrap().channel(0).to_vec();
        let eq = serde_json::json!({ "type": "eq", "shape": "lowpass", "frequency": 800 });
        let bypassed = project(serde_json::json!([{ "type": "reverb", "mix": 0.9, "bypass": true }, eq]));
        assert!(bypassed.tracks[0].effects[0].bypass);
        assert_eq!(serde_json::to_value(&bypassed.tracks[0].effects[0]).unwrap()["bypass"], true, "写回 JSON 时保留");
        // 旁通的混响不出声；第二个效果器的自动化仍指向 effects[1]。
        let only_eq = project(serde_json::json!([{ "type": "eq", "shape": "lowpass", "frequency": 20 }, eq]));
        let mut no_reverb = only_eq.clone();
        no_reverb.tracks[0].effects[0].bypass = true;
        assert_eq!(render(&bypassed), render(&no_reverb));
        assert_ne!(render(&project(serde_json::json!([{ "type": "reverb", "mix": 0.9 }, eq]))), render(&bypassed));
    }
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
        p.tracks[0].effects.push((Effect::Compressor {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            makeup_db: 0.0,
            sidechain: None,
        }).into());
        // 主总线加混响
        p.master_effects.push((Effect::Reverb {
            mix: 0.25,
            decay_seconds: 1.6,
            damping: 0.5,
            size: 0.5,
        }).into());
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
        wet.master_effects.push((Effect::Reverb {
            mix: 0.6,
            decay_seconds: 2.5,
            damping: 0.3,
            size: 0.8,
        }).into());

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
        p.tracks[0].effects.push((Effect::Chorus {
            rate_hz: 0.8,
            depth_ms: 3.0,
            mix: 0.4,
        }).into());
        p.tracks[0].effects.push((Effect::Eq {
            shape: "high_shelf".to_string(),
            frequency: 6000.0,
            q: 0.707,
            gain_db: 3.0,
        }).into());
        // 主总线 limiter
        p.master_effects.push((Effect::Limiter {
            ceiling_db: -0.3,
            release_ms: 50.0,
        }).into());
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
        p.tracks[0].effects.push((Effect::Phaser {
            rate_hz: 0.5,
            feedback: 0.4,
            mix: 0.5,
        }).into());
        p.master_effects.push((Effect::Flanger {
            rate_hz: 0.2,
            feedback: 0.7,
            mix: 0.5,
        }).into());
        let audio = render(&p, RenderSettings { tail_seconds: 1.0, ..Default::default() })
            .expect("phaser/flanger 渲染应成功");
        assert!(audio.peak() > 0.0, "应发声");
    }

    #[test]
    fn eq_unknown_shape_errors() {
        use muser_core::Effect;
        let mut p = demo_project();
        p.master_effects.push((Effect::Eq {
            shape: "bogus".to_string(),
            frequency: 1000.0,
            q: 0.7,
            gain_db: 0.0,
        }).into());
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
        p.master_effects.push((Effect::Plugin {
            path: "/nonexistent/x.clap".to_string(),
            id: None,
            params: Default::default(),
        }).into());
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
        p.master_effects.push((Effect::Plugin {
            path: "/definitely/not/here.clap".to_string(),
            id: None,
            params: Default::default(),
        }).into());
        let err = render(&p, RenderSettings::default()).unwrap_err();
        // host 下 validate 会加载插件检查参数，路径不存在在校验阶段就报出，并指明位置。
        let msg = err.to_string();
        assert!(msg.contains("master_effects[0]") && msg.contains("找不到插件"), "{msg}");
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

    /// 用本机的 freepats 音色库（MUSER_SFZ_PATH）做真实测试；不存在则跳过。
    #[test]
    fn sfz_real_library_makes_sound() {
        let path = crate::sfz::resolve("FM-Piano1 SFZ+FLAC-20190916/FM-Piano1 20190916.sfz");
        if !path.is_file() {
            eprintln!("跳过：未找到 {}（设 MUSER_SFZ_PATH）", path.display());
            return;
        }
        let audio = render_one(&format!("sfz:{}", path.display()), serde_json::json!({}), &[(60, 0.0, 1.0), (64, 0.0, 1.0)]).unwrap();
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
        p.tracks[0].effects.push((muser_core::Effect::Eq { shape: "lowpass".into(), frequency: 300.0, q: 0.707, gain_db: 0.0 }).into());
        let a = render(&p, quiet()).unwrap();
        assert!(seg_brightness(&a, 3 * SEC, 7 * SEC / 2) > seg_brightness(&a, SEC / 2, SEC) * 3.0);
    }

    #[test]
    fn automation_master_effect_and_gain() {
        let mut p = auto_project("synth.poly", &[(60, 0.0, 2.0)], serde_json::json!([]));
        p.master_effects.push((muser_core::Effect::Reverb { mix: 0.0, decay_seconds: 2.0, damping: 0.5, size: 0.6 }).into());
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

    #[test]
    fn audition_reports_brightness_and_attack() {
        use crate::audition::{audition, AuditionSettings, Subject};
        let s = AuditionSettings::default();
        let cutoff = audition(
            &Subject::Instrument { name: "poly.moog_bass".into(), params: Default::default() },
            Some("cutoff"),
            &[serde_json::json!(200), serde_json::json!(6000)],
            &AuditionSettings { pitch: 36, ..s.clone() },
        )
        .unwrap();
        assert!(cutoff[1].analysis.centroid_hz > cutoff[0].analysis.centroid_hz * 2.0, "{cutoff:?}");
        let attack = audition(
            &Subject::Instrument { name: "synth.poly".into(), params: Default::default() },
            Some("attack"),
            &[serde_json::json!(0.005), serde_json::json!(0.5)],
            &s,
        )
        .unwrap();
        assert!(attack[0].analysis.attack_ms < 100.0 && attack[1].analysis.attack_ms > 300.0, "{attack:?}");
        // 效果器：混响让余音变长。
        let rev = audition(
            &Subject::Effect { effect: serde_json::from_str(r#"{"type":"reverb","decay_seconds":3}"#).unwrap(), source: "synth.poly".into() },
            Some("mix"),
            &[serde_json::json!(0.0), serde_json::json!(0.6)],
            &AuditionSettings { hold_seconds: 0.3, tail_seconds: 3.0, ..s },
        )
        .unwrap();
        assert!(rev[1].analysis.release_s > rev[0].analysis.release_s + 0.3, "{rev:?}");
        // 未知效果器字段
        assert!(audition(
            &Subject::Effect { effect: serde_json::from_str(r#"{"type":"reverb"}"#).unwrap(), source: "synth.poly".into() },
            Some("mixx"), &[serde_json::json!(1)], &AuditionSettings::default()
        ).unwrap_err().to_string().contains("没有参数"));
    }

    #[test]
    fn audition_parse_value() {
        use crate::audition::parse_value;
        assert_eq!(parse_value("0.5"), serde_json::json!(0.5));
        assert_eq!(parse_value("true"), serde_json::json!(true));
        assert_eq!(parse_value(" 200 Hz "), serde_json::json!("200 Hz"));
    }

    /// 以下测试使用本机安装的 Surge XT（CLAP）；没有则跳过。
    #[cfg(feature = "host")]
    const SURGE: &str = "/usr/lib/clap/Surge XT.clap";
    #[cfg(feature = "host")]
    const SURGE_FX: &str = "/usr/lib/clap/Surge XT Effects.clap";
    #[cfg(feature = "host")]
    fn have(path: &str) -> bool {
        let ok = std::path::Path::new(&*crate::plugin::expand_home(path)).exists();
        if !ok {
            eprintln!("跳过：未安装 {path}");
        }
        ok
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_params_by_name_and_text() {
        if !have(SURGE) {
            return;
        }
        let o = crate::plugin::Opened::open(SURGE, None).unwrap();
        // 大小写不敏感
        let p = o.resolve("a filter 1 cutoff").unwrap().clone();
        assert_eq!(p.name, "A Filter 1 Cutoff");
        // 连续参数：文字交给插件解析
        let v = o.value(&p, &serde_json::json!("1000 Hz")).unwrap();
        assert_eq!(o.plugin.param_to_text(p.id, v).unwrap(), "1000.00 Hz");
        // 选项参数
        let t = o.resolve("A Filter 1 Type").unwrap().clone();
        let v = o.value(&t, &serde_json::json!("LP 24 dB")).unwrap();
        assert_eq!(o.plugin.param_to_text(t.id, v).unwrap(), "LP 24 dB");
        // 错误：名称、文字、范围
        assert!(o.resolve("A Filter 1 Cutof").unwrap_err().contains("A Filter 1 Cutoff"));
        assert!(o.value(&t, &serde_json::json!("banana")).is_err());
        assert!(o.value(&p, &serde_json::json!(5.0)).unwrap_err().contains("超出"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_option_text_fallback() {
        // Surge XT Effects 对 FX Type 的文字解析总是返回 0；应回退为查表。
        if !have(SURGE_FX) {
            return;
        }
        let o = crate::plugin::Opened::open(SURGE_FX, None).unwrap();
        let t = o.resolve("FX Type").unwrap().clone();
        for name in ["Reverb 2", "Distortion", "Chorus"] {
            let v = o.value(&t, &serde_json::json!(name)).unwrap();
            assert_eq!(o.plugin.param_to_text(t.id, v).unwrap(), name);
        }
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_effect_type_is_applied() {
        // 不同 FX Type 渲染结果应不同（验证激活后预热重发生效）。
        if !have(SURGE_FX) {
            return;
        }
        use crate::audition::{audition, AuditionSettings, Subject};
        let fx: muser_core::Effect = serde_json::from_value(serde_json::json!({ "type": "plugin", "path": SURGE_FX })).unwrap();
        let takes = audition(
            &Subject::Effect { effect: fx, source: "synth.poly".into() },
            Some("FX Type"),
            &[serde_json::json!("Delay"), serde_json::json!("Distortion")],
            &AuditionSettings { hold_seconds: 0.5, tail_seconds: 1.0, ..Default::default() },
        )
        .unwrap();
        assert_ne!(takes[0].analysis, takes[1].analysis, "{takes:?}");
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_instrument_params_and_automation() {
        if !have(SURGE) {
            return;
        }
        let mut p = auto_project(&format!("plugin:{SURGE}"), &[(48, 0.0, 8.0)], serde_json::json!([
            { "target": "instrument", "param": "A Filter 1 Cutoff", "shape": "points", "points": [[0, "150 Hz"], [8, "6000 Hz"]] }
        ]));
        p.tracks[0].instrument_params = serde_json::from_value(serde_json::json!({
            "A Filter 1 Type": "LP 24 dB", "A Osc 1 Retrigger": "On"
        })).unwrap();
        let a = render(&p, quiet()).unwrap();
        assert!(seg_brightness(&a, 3 * SEC, 7 * SEC / 2) > seg_brightness(&a, SEC / 2, SEC) * 5.0, "插件 cutoff 自动化应变亮");
        // Retrigger 开启后可复现
        let b = render(&p, quiet()).unwrap();
        assert_eq!(a.channel(0), b.channel(0));
        // 错误参数名
        p.tracks[0].automation[0].param = "A Filter 1 Cutof".into();
        let e = crate::validate(&p).unwrap_err().to_string();
        assert!(e.contains("automation[0]") && e.contains("A Filter 1 Cutoff"), "{e}");
    }

    #[test]
    fn state_file_header_roundtrip() {
        let dir = std::env::temp_dir().join(format!("muser-state-{}", std::process::id()));
        let path = dir.join("a.muserstate");
        let h = crate::plugin::StateHeader { plugin_id: "x.y".into(), plugin_name: "X".into(), version: "1".into(), format: "clap".into() };
        crate::plugin::write_state_file(&path, &h, b"\x00\x01binary\nwith newline").unwrap();
        let (h2, data) = crate::plugin::read_state_file(&path).unwrap();
        assert_eq!(h, h2);
        assert_eq!(data, b"\x00\x01binary\nwith newline");
        std::fs::write(&path, b"not a state").unwrap();
        assert!(crate::plugin::read_state_file(&path).unwrap_err().contains("不是 muser 状态文件"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(feature = "host")]
    fn bundled_surge() -> crate::profile::Profile {
        crate::profile::Profile::from_json(include_str!("../../../profiles/surge-xt.json")).unwrap()
    }

    #[cfg(feature = "host")]
    #[test]
    fn profile_aliases_presets_and_reserved_keys() {
        if !have(SURGE) {
            return;
        }
        let pr = bundled_surge();
        let m = |v: serde_json::Value| v.as_object().unwrap().clone();
        crate::plugin::check_params_in(SURGE, &pr, &m(serde_json::json!({ "cutoff": "800 Hz", "filter_type": "LP 24 dB" }))).unwrap();
        for preset in &pr.presets {
            crate::plugin::check_params_in(SURGE, &pr, &preset.params).unwrap_or_else(|e| panic!("{}: {e}", preset.name));
        }
        let mut o = crate::plugin::Opened::open(SURGE, None).unwrap();
        o.profile = Some(std::sync::Arc::new(crate::profile::Loaded { origin: crate::profile::Origin::File, path: None, profile: pr }));
        assert_eq!(o.resolve("cutoff").unwrap().name, "A Filter 1 Cutoff");
        assert!(o.apply(&m(serde_json::json!({ "@preset": "nope" }))).unwrap_err().contains("bass"));
        assert!(o.apply(&m(serde_json::json!({ "@bogus": 1 }))).unwrap_err().contains("保留键"));
        assert!(o.apply(&m(serde_json::json!({ "@setup": "yes" }))).is_err());
        o.apply(&m(serde_json::json!({ "@preset": "pad" }))).unwrap();
        let attack = o.resolve("attack").unwrap().clone();
        let v = o.plugin.param_value(attack.id).unwrap();
        assert_eq!(o.plugin.param_to_text(attack.id, v).unwrap().split_whitespace().next().unwrap().parse::<f64>().unwrap().round(), 800.0);
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_state_roundtrip_matches_direct_params() {
        if !have(SURGE) || !have(SURGE_FX) {
            return;
        }
        use crate::audition::{audition, AuditionSettings, Subject};
        let dir = std::env::temp_dir().join(format!("muser-state-rt-{}", std::process::id()));
        let file = dir.join("lead.muserstate");
        let direct = serde_json::json!({ "A Filter 1 Type": "LP 24 dB", "A Filter 1 Cutoff": "700 Hz", "A Amp EG Attack": "200 ms",
            "A Osc 1 Retrigger": "On", "A Osc 2 Retrigger": "On", "A Osc 3 Retrigger": "On" }).as_object().unwrap().clone();
        crate::plugin::save_state(SURGE, &direct, &file).unwrap();
        let s = AuditionSettings { hold_seconds: 0.5, tail_seconds: 0.5, ..Default::default() };
        let run = |params: serde_json::Map<String, serde_json::Value>| {
            audition(&Subject::Instrument { name: format!("plugin:{SURGE}"), params }, None, &[], &s).unwrap()[0].analysis.clone()
        };
        let mut from_state = serde_json::Map::new();
        from_state.insert("@state".into(), serde_json::json!(file.display().to_string()));
        let (a, b) = (run(direct), run(from_state.clone()));
        // 起音时间可能差几毫秒（插件内部包络的起点），其余特征应一致。
        assert!((a.attack_ms - b.attack_ms).abs() <= 10.0, "{a:?} vs {b:?}");
        assert!((a.lufs - b.lufs).abs() <= 0.2 && (a.centroid_hz - b.centroid_hz).abs() <= 20.0, "{a:?} vs {b:?}");
        // 别的插件的状态被拒
        let fx: muser_core::Effect = serde_json::from_value(serde_json::json!({ "type": "plugin", "path": SURGE_FX, "params": from_state })).unwrap();
        let e = audition(&Subject::Effect { effect: fx, source: "synth.poly".into() }, None, &[], &s).unwrap_err().to_string();
        assert!(e.contains("不能装进"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(feature = "host")]
    #[test]
    fn draft_profile_picks_common_params() {
        if !have(SURGE) {
            return;
        }
        let d = crate::plugin::draft_profile(SURGE, false).unwrap();
        let by = |alias: &str| d.params.iter().find(|p| p.alias.as_deref() == Some(alias)).map(|p| p.name.as_str());
        assert_eq!(by("cutoff"), Some("A Filter 1 Cutoff"));
        assert_eq!(by("resonance"), Some("A Filter 1 Resonance"));
        assert_eq!(by("attack"), Some("A Amp EG Attack"));
        assert!(d.plugin.ids.contains(&"org.surge-synth-team.surge-xt".to_string()));
    }

    fn proj(v: serde_json::Value) -> Project {
        serde_json::from_value(v).unwrap()
    }
    fn energy(a: &AudioBuffer<f32>, from: usize, to: usize) -> f64 {
        (0..2).map(|ch| a.channel(ch)[from..to.min(a.num_frames())].iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()).sum()
    }

    #[test]
    fn routing_bus_output_and_gain() {
        let base = serde_json::json!({ "tempo": 120, "tracks": [
            { "name": "a", "instrument": "synth.poly", "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 60, "start": 0, "length": 2 }] }] } ] });
        let direct = render(&proj(base.clone()), quiet()).unwrap();
        let mut v = base.clone();
        v["buses"] = serde_json::json!([{ "name": "grp", "gain_db": -6 }]);
        v["tracks"][0]["output"] = serde_json::json!("grp");
        let via = render(&proj(v), quiet()).unwrap();
        let ratio = energy(&via, 0, SEC) / energy(&direct, 0, SEC);
        assert!((ratio - 0.251).abs() < 0.01, "经 -6 dB 总线，能量应为 1/4，实际 {ratio}");
    }

    #[test]
    fn routing_send_reverb_adds_tail_and_keeps_dry() {
        let v = |send: Option<f64>| {
            let mut t = serde_json::json!({ "name": "a", "instrument": "synth.poly",
                "clips": [{ "start": 0, "length": 1, "notes": [{ "pitch": 60, "start": 0, "length": 0.5 }] }] });
            if let Some(db) = send {
                t["sends"] = serde_json::json!({ "verb": db });
            }
            proj(serde_json::json!({ "tempo": 120,
                "buses": [{ "name": "verb", "effects": [{ "type": "reverb", "mix": 1.0, "decay_seconds": 3.0 }] }],
                "tracks": [t] }))
        };
        let s = RenderSettings { tail_seconds: 2.5, ..Default::default() };
        let dry = render(&v(None), s).unwrap();
        let wet = render(&v(Some(-6.0)), s).unwrap();
        assert!(energy(&wet, SEC, 2 * SEC) > energy(&dry, SEC, 2 * SEC) * 50.0, "发送到混响总线应有尾音");
        // 干声仍在：开头 0.2 秒基本一致（混响尚未建立）。
        let (a, b) = (energy(&dry, 0, SEC / 5), energy(&wet, 0, SEC / 5));
        assert!(b >= a * 0.95, "干声应保留 {a} {b}");
    }

    #[test]
    fn routing_sidechain_ducks_on_kick() {
        // pad 持续发声；kick 每拍一下。pad 的压缩器由 kick 触发。
        let v = |sidechain: bool, kick_muted: bool| {
            let mut comp = serde_json::json!({ "type": "compressor", "threshold_db": -40, "ratio": 20, "attack_ms": 1, "release_ms": 80 });
            if sidechain {
                comp["sidechain"] = serde_json::json!("kick");
            }
            proj(serde_json::json!({ "tempo": 120, "tracks": [
                { "name": "kick", "instrument": "drums.909", "muted": kick_muted,
                  "clips": [{ "start": 0, "length": 4, "notes": (0..4).map(|b| serde_json::json!({ "pitch": 36, "start": b, "length": 0.1 })).collect::<Vec<_>>() }] },
                { "name": "pad", "instrument": "synth.poly", "gain_db": -12, "effects": if sidechain { vec![comp] } else { vec![] },
                  "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 60, "start": 0, "length": 4 }] }] } ] }))
        };
        let s = quiet();
        // 只听 pad：kick 静音（但仍作为侧链键）。
        let plain = render(&v(false, true), s).unwrap();
        let ducked = render(&v(true, true), s).unwrap();
        // 第 2 拍（0.5 s 起）的头 50 ms：被压；拍子中段（0.75 s 附近）基本恢复。
        let hit = SEC / 2;
        let (p_hit, d_hit) = (energy(&plain, hit, hit + SEC / 20), energy(&ducked, hit, hit + SEC / 20));
        let mid = SEC * 3 / 4 + SEC / 10;
        let (p_mid, d_mid) = (energy(&plain, mid, mid + SEC / 20), energy(&ducked, mid, mid + SEC / 20));
        assert!(d_hit < p_hit * 0.25, "底鼓处应被压下 {p_hit} → {d_hit}");
        assert!(d_mid > p_mid * 0.5, "拍子中段应恢复 {p_mid} → {d_mid}");
        assert!(d_mid > d_hit * 3.0);
    }

    #[test]
    fn routing_send_automation() {
        let p = proj(serde_json::json!({ "tempo": 120,
            "buses": [{ "name": "fx", "effects": [] }, { "name": "sink", "muted": true }],
            "tracks": [{ "name": "a", "instrument": "synth.poly", "output": "sink",
                "sends": { "fx": -80 },
                "automation": [{ "target": "sends.fx", "shape": "points", "points": [[0, -80], [4, 0]] }],
                "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 60, "start": 0, "length": 4 }] }] }] }));
        let a = render(&p, quiet()).unwrap();
        assert!(energy(&a, 3 * SEC / 2, 2 * SEC) > energy(&a, SEC / 10, SEC / 2) * 100.0, "发送量从 -80 升到 0 dB");
        // 发送目标不存在 / 发送里没有该总线
        let mut bad = p.clone();
        bad.tracks[0].automation[0].target = "sends.nope".into();
        assert!(crate::validate(&bad).unwrap_err().to_string().contains("nope"));
    }

    #[test]
    fn routing_latency_is_compensated() {
        // 同一个短促音：一份经过带前瞻的 limiter，一份不经过；起音位置应一致。
        let v = |limiter: bool| {
            proj(serde_json::json!({ "tempo": 120, "tracks": [{ "name": "a", "instrument": "drums.909",
                "effects": if limiter { vec![serde_json::json!({ "type": "limiter", "ceiling_db": -1 })] } else { vec![] },
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 36, "start": 1, "length": 0.1 }] }] }] }))
        };
        let onset = |a: &AudioBuffer<f32>| a.channel(0).iter().position(|s| s.abs() > 1e-3).unwrap();
        let (x, y) = (onset(&render(&v(false), quiet()).unwrap()), onset(&render(&v(true), quiet()).unwrap()));
        assert!((x as i64 - y as i64).abs() <= 2, "limiter 延迟应被补偿：{x} vs {y}");
    }

    #[test]
    fn routing_bus_automation_and_errors() {
        let p = proj(serde_json::json!({ "tempo": 120,
            "buses": [{ "name": "grp", "automation": [{ "target": "gain_db", "shape": "points", "points": [[0, -60], [4, 0]] }] }],
            "tracks": [{ "name": "a", "instrument": "synth.poly", "output": "grp",
                "clips": [{ "start": 0, "length": 4, "notes": [{ "pitch": 60, "start": 0, "length": 4 }] }] }] }));
        let a = render(&p, quiet()).unwrap();
        assert!(energy(&a, 3 * SEC / 2, 2 * SEC) > energy(&a, SEC / 10, SEC / 2) * 100.0);
        let mut bad = p.clone();
        bad.buses[0].automation[0].target = "instrument".into();
        bad.buses[0].automation[0].param = "cutoff".into();
        assert!(crate::validate(&bad).unwrap_err().to_string().contains("总线没有乐器"));
        let mut bad = p.clone();
        bad.tracks[0].effects.push(serde_json::from_value(serde_json::json!({ "type": "compressor", "sidechain": "grp" })).unwrap());
        assert!(crate::validate(&bad).unwrap_err().to_string().contains("环"));
    }

    #[test]
    fn render_cache_reuses_instruments() {
        let p = proj(serde_json::json!({ "tempo": 120,
            "buses": [{ "name": "verb", "effects": [{ "type": "reverb", "mix": 1.0 }] }],
            "tracks": [
                { "name": "a", "instrument": "synth.poly", "sends": { "verb": -6 }, "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 60, "start": 0.5, "length": 1 }] }] },
                { "name": "b", "instrument": "drums.909", "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 36, "start": 0, "length": 0.2 }] }] },
                { "name": "c", "instrument": "synth.poly", "muted": true, "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 64, "start": 0, "length": 1 }] }] } ] }));
        let mut cache = RenderCache::new();
        let first = render_cached(&p, quiet(), &mut cache).unwrap();
        assert_eq!(first.channel(0), render(&p, quiet()).unwrap().channel(0), "缓存与直接渲染一致");
        assert_eq!(cache.len(), 2, "静音且未缓存的音轨不渲染");
        // 改音量：结果与全新渲染一致。
        let mut q = p.clone();
        q.tracks[0].gain_db = -6.0;
        q.tracks[2].muted = false;
        let cached = render_cached(&q, quiet(), &mut cache).unwrap();
        assert_eq!(cached.channel(1), render(&q, quiet()).unwrap().channel(1));
        assert_eq!(cache.len(), 3);
        // 改音符：只有这条轨的缓存换掉。
        let mut r = q.clone();
        r.tracks[1].clips[0].notes[0].pitch = 38;
        let changed = render_cached(&r, quiet(), &mut cache).unwrap();
        assert_eq!(changed.channel(0), render(&r, quiet()).unwrap().channel(0));
        assert_eq!(cache.len(), 3);
    }

    #[cfg(feature = "host")]
    #[test]
    fn juce_state_patch_and_numbers() {
        use crate::plugin::{patch_juce_state, split_number};
        let xml = r#"<?xml version="1.0"?> <root a="1" currentProcessorName="Galactic"/>"#;
        let mut clap = b"VC2!".to_vec();
        clap.extend_from_slice(&((xml.len() + 1) as u32).to_le_bytes());
        clap.extend_from_slice(xml.as_bytes());
        clap.push(0);
        let out = patch_juce_state(&clap, "currentProcessorName", "Density \"2\"").unwrap();
        let text = String::from_utf8_lossy(&out[8..]).to_string();
        assert!(text.contains(r#"currentProcessorName="Density &quot;2&quot;""#), "{text}");
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize, out.len() - 8);
        // 没有该属性时追加；VST3 容器修正组件长度。
        let mut vst3 = b"MBVST3\0\x01".to_vec();
        vst3.extend_from_slice(&[0u8; 16]);
        vst3.extend_from_slice(&(clap.len() as u64).to_le_bytes());
        vst3.extend_from_slice(&clap);
        vst3.extend_from_slice(&0u64.to_le_bytes());
        let out = patch_juce_state(&vst3, "effect", "Kalman").unwrap();
        let comp = u64::from_le_bytes(out[24..32].try_into().unwrap()) as usize;
        assert_eq!(comp, out.len() - 32 - 8);
        assert!(String::from_utf8_lossy(&out).contains(r#"effect="Kalman"/>"#));
        assert!(patch_juce_state(b"not juce", "a", "b").is_err());

        assert_eq!(split_number("-6.02dB"), Some((-6.02, "db".into())));
        assert_eq!(split_number("0.5 secs"), Some((0.5, "secs".into())));
        assert_eq!(split_number("1.26e-09 secs"), Some((1.26e-9, "secs".into())));
        assert_eq!(split_number("16v"), Some((16.0, "v".into())));
        assert_eq!(split_number("Off"), None);
    }

    #[test]
    fn profile_selector_and_english_markdown() {
        use crate::profile::{Loaded, Origin, Profile, render_markdown};
        let p = Profile::from_json(
            r#"{ "schema_version": 1, "plugin": { "name": "X", "ids": ["x"] }, "source": { "lang": "en" },
                 "selector": { "key": "Effect", "juce_xml_attribute": "cur",
                   "choices": [{ "name": "Galactic", "group": "Reverb" }, { "name": "Density", "group": "Saturation", "tags": ["recommended"] }] } }"#,
        )
        .unwrap();
        let sel = p.selector.as_ref().unwrap();
        assert_eq!(sel.key, "effect");
        assert_eq!(sel.resolve("density").unwrap(), "Density");
        assert!(sel.resolve("Densty").unwrap_err().contains("Density"));
        let md = render_markdown(&Loaded { origin: Origin::Bundled, path: None, profile: p });
        assert!(md.contains("# Plugin: X") && md.contains("## Choosing the effect") && md.contains("### Saturation"), "{md}");
        let bad = |extra: &str| Profile::from_json(&format!(r#"{{ "schema_version": 1, "plugin": {{ "name": "X", "ids": ["x"] }} {extra} }}"#)).is_err();
        assert!(bad(r#", "selector": { "key": "e", "juce_xml_attribute": "a b" }"#));
        assert!(bad(r#", "selector": { "key": "e", "juce_xml_attribute": "a", "choices": [{ "name": "A" }, { "name": "a" }] }"#));
        assert!(bad(r#", "source": { "lang": "fr" }"#));
        assert!(bad(r#", "state_format": "other""#));
        // 只有 state_format 声明的保留键能写进预设。
        assert!(bad(r#", "presets": [{ "name": "p", "params": { "@wavetable": "sine" } }]"#));
        assert!(Profile::from_json(r#"{ "schema_version": 1, "plugin": { "name": "X", "ids": ["x"] }, "state_format": "vital",
            "presets": [{ "name": "p", "params": { "@wavetable": "sine", "@modulations": [{ "source": "env_2", "destination": "filter_1_cutoff" }] } }] }"#).is_ok());
    }

    #[cfg(feature = "host")]
    const AIRWINDOWS: &str = "~/.clap/Airwindows Consolidated.clap";
    #[cfg(feature = "host")]
    const VITAL: &str = "/usr/lib/vst3/Vital.vst3";
    #[cfg(feature = "host")]
    const PLASMA: &str = "~/.vst3/Plasma.vst3";

    #[cfg(feature = "host")]
    #[test]
    fn airwindows_selector_switches_effect() {
        if !have(AIRWINDOWS) {
            return;
        }
        let params = |base: serde_json::Value| {
            crate::plugin::params(AIRWINDOWS, None, false, &serde_json::from_value(base).unwrap()).unwrap()
        };
        let names = |v: Vec<crate::plugin::PluginParam>| v.into_iter().map(|p| p.name).take(3).collect::<Vec<_>>();
        assert_eq!(names(params(serde_json::json!({ "effect": "Pressure4" }))), ["Pressure", "Speed", "Mewiness"]);
        assert_eq!(names(params(serde_json::json!({ "@preset": "plate" }))), ["Input", "Regen", "DeRez"]);
        let err = crate::plugin::params(AIRWINDOWS, None, false, &serde_json::from_value(serde_json::json!({ "effect": "Pressur4" })).unwrap()).unwrap_err();
        assert!(err.contains("Pressure4"), "{err}");
        // 工程里：选效果 + 设参数 + 自动化效果参数，渲染与干声不同。
        let run = |fx: Option<serde_json::Value>| {
            let mut t = serde_json::json!({ "instrument": "synth.poly", "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 60, "start": 0, "length": 0.5 }] }] });
            if let Some(fx) = fx {
                t["effects"] = serde_json::json!([fx]);
                t["automation"] = serde_json::json!([{ "target": "effects[0]", "param": "Wetness", "shape": "points", "points": [[0, 0.1], [2, 0.8]] }]);
            }
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [t] })), quiet()).unwrap()
        };
        let dry = run(None);
        let wet = run(Some(serde_json::json!({ "type": "plugin", "path": AIRWINDOWS, "params": { "effect": "kPlate140" } })));
        assert!(energy(&wet, SEC / 2, SEC * 2) > energy(&dry, SEC / 2, SEC * 2) * 3.0, "plate 应有尾音");
    }

    #[cfg(feature = "host")]
    #[test]
    fn vital_wavetable_modulation_and_tempo() {
        if !have(VITAL) {
            return;
        }
        let spec = format!("plugin:{VITAL}");
        let run = |params: serde_json::Value| {
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": spec, "instrument_params": params,
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 48, "start": 0, "length": 1.5 }] }] }] })), quiet()).unwrap()
        };
        let bright = |a: &AudioBuffer<f32>| seg_brightness(a, 0, SEC / 2);
        let saw = run(serde_json::json!({}));
        let sine = run(serde_json::json!({ "@wavetable": "sine" }));
        assert!(bright(&saw) > bright(&sine) * 2.0, "sine 波表应更暗");
        // 低截止 + 包络调制：开头亮、之后暗。
        let filt = run(serde_json::json!({ "filter": "On", "cutoff": "-24 semitones", "resonance": "0%", "env2_decay": "0.15 secs", "env2_sustain": "0",
            "@modulations": [{ "source": "env_2", "destination": "filter_1_cutoff", "amount": 0.6 }] }));
        assert!(seg_brightness(&filt, 0, SEC / 20) > seg_brightness(&filt, SEC / 2, SEC) * 1.5, "滤波包络应先亮后暗");
        // 文字取值按显示数值解析（Vital 自己的解析会把 0.5 secs 读成 16 秒）。
        let slow = run(serde_json::json!({ "attack": "0.5 secs" }));
        assert!(energy(&slow, 0, SEC / 10) < energy(&slow, SEC / 2, SEC * 6 / 10) * 0.2, "attack 0.5 s");
        assert!(energy(&slow, SEC / 2, SEC * 6 / 10) > 0.0);
        let err = crate::validate(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": spec,
            "instrument_params": { "@modulations": [{ "source": "env_9", "destination": "filter_1_cutoff" }] }, "clips": [] }] }))).unwrap_err();
        assert!(err.to_string().contains("env_1"), "{err}");
    }

    #[cfg(feature = "host")]
    #[test]
    fn plasma_preset_distorts() {
        if !have(PLASMA) {
            return;
        }
        let run = |fx: serde_json::Value| {
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": "bass.finger", "effects": [fx],
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 40, "start": 0, "length": 1 }] }] }] })), quiet()).unwrap()
        };
        let clean = run(serde_json::json!({ "type": "plugin", "path": PLASMA, "params": { "mix": "0.00" } }));
        let dirty = run(serde_json::json!({ "type": "plugin", "path": PLASMA, "params": { "@preset": "scream" } }));
        assert!(seg_brightness(&dirty, 0, SEC / 2) > seg_brightness(&clean, 0, SEC / 2) * 2.0);
    }

    #[test]
    fn sustain_pedal_holds_released_notes() {
        // 短音（0.25 拍）；踏板踩到第 3 拍。踩着时松键后仍发声，抬起后很快消失。
        let run = |pedal: serde_json::Value| {
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": "synth.poly", "instrument_params": { "release": 0.05 },
                "clips": [{ "start": 0, "length": 4, "pedal": pedal, "notes": [{ "pitch": 60, "start": 0, "length": 0.25 }] }] }] })), quiet()).unwrap()
        };
        let dry = run(serde_json::json!([]));
        let held = run(serde_json::json!([[0, 3]]));
        let mid = (SEC * 3 / 4, SEC);                 // 第 1.5–2 拍
        assert!(energy(&held, mid.0, mid.1) > energy(&dry, mid.0, mid.1) * 100.0, "踏板延音");
        let after = (SEC * 3 / 2 + SEC / 5, SEC * 2); // 抬起（1.5 s）之后
        assert!(energy(&held, after.0, after.1) < energy(&held, mid.0, mid.1) * 0.01, "抬起后释音");
        // 同一时刻换踏板：新音在新踏板下继续。
        let two = render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": "synth.poly", "instrument_params": { "release": 0.05 },
            "clips": [{ "start": 0, "length": 4, "pedal": [[0, 1.0], [1.0, 3]],
                "notes": [{ "pitch": 60, "start": 0, "length": 0.25 }, { "pitch": 67, "start": 1.0, "length": 0.25 }] }] }] })), quiet()).unwrap();
        assert!(energy(&two, SEC, SEC * 3 / 2) > energy(&dry, SEC / 2, SEC) * 100.0, "换踏板后新音被保持");
    }

    #[test]
    fn sfz_sustain_pedal() {
        let sfz = crate::sfz::resolve("FM-Piano1 SFZ+FLAC-20190916/FM-Piano1 20190916.sfz");
        if !sfz.is_file() {
            return;
        }
        let run = |pedal: serde_json::Value| {
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": format!("sfz:{}", sfz.display()),
                "clips": [{ "start": 0, "length": 4, "pedal": pedal, "notes": [{ "pitch": 60, "start": 0, "length": 0.2 }] }] }] })), quiet()).unwrap()
        };
        let (dry, held) = (run(serde_json::json!([])), run(serde_json::json!([[0, 4]])));
        assert!(energy(&held, SEC, SEC * 3 / 2) > energy(&dry, SEC, SEC * 3 / 2) * 10.0);
    }

    #[cfg(feature = "host")]
    #[test]
    fn plugin_sustain_pedal() {
        if !have(SURGE) {
            return;
        }
        let run = |pedal: serde_json::Value| {
            render(&proj(serde_json::json!({ "tempo": 120, "tracks": [{ "instrument": format!("plugin:{SURGE}"),
                "clips": [{ "start": 0, "length": 4, "pedal": pedal, "notes": [{ "pitch": 60, "start": 0, "length": 0.25 }] }] }] })), quiet()).unwrap()
        };
        let (dry, held) = (run(serde_json::json!([])), run(serde_json::json!([[0, 3]])));
        assert!(energy(&held, SEC * 3 / 4, SEC) > energy(&dry, SEC * 3 / 4, SEC) * 100.0, "插件收到 CC64");
    }
}
