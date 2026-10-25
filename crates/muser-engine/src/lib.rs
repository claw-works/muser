//! muser-engine —— 引擎适配层
//!
//! 把 [`muser_core::Project`] 编译成 manguebyte 的乐器/合成器并离线渲染为音频/WAV。
//! 这是「领域模型 → 声音」的翻译层，是三层架构的中间层。

pub mod effects;
pub mod instrument;
pub mod render;

pub use instrument::{available as available_instruments, TrackInstrument};
pub use render::{render, render_to_wav, EngineError, RenderSettings};

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(matches!(err, EngineError::Instrument(_)));
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
        assert!(matches!(
            err,
            EngineError::Effect(crate::effects::EffectError::UnknownEqShape(_))
        ));
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
}
