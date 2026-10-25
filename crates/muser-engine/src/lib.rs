//! muser-engine —— 引擎适配层
//!
//! 把 [`muser_core::Project`] 编译成 manguebyte 的乐器/合成器并离线渲染为音频/WAV。
//! 这是「领域模型 → 声音」的翻译层，是三层架构的中间层。

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
}
