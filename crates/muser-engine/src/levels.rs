//! 内置乐器的音量校准：每个内置乐器（含 poly 预设）用同一段参考乐句（[`reference`]）测出的响度，
//! 在乐器输出端补成 [`TARGET_LUFS`]，`gain_db` 为 0 时各乐器大致一样响。SFZ 与插件不校准（`muser sounds describe` 给出它们的响度）。
//!
//! 表是生成的：乐器的合成或参考乐句改了以后，运行
//! `cargo test -p muser-engine --release levels::tests::print_table -- --ignored --nocapture`，把输出替换 [`MEASURED`]。

use muser_core::{Clip, Note, Project, Track};

/// 校准后参考乐句的响度（LUFS）。
pub const TARGET_LUFS: f64 = -20.0;

/// 内置乐器的参考乐句（音域、是不是鼓组取自乐器描述）。
pub fn reference(name: &str) -> Project {
    let info = crate::instrument::describe(name).expect("built-in instrument");
    phrase(name, &Default::default(), info.range, info.family == "drums")
}

/// 参考乐句：100 BPM、16 拍、力度 0.8。旋律乐器在音域中间上下一个八度弹分解和弦（每拍一个音）；
/// 鼓组是底鼓 / 军鼓 / 闭镲的基本节奏。SFZ、插件也用它测音量（[`crate::sounds::describe`]）。
pub fn phrase(spec: &str, params: &muser_core::InstrumentParams, (lo, hi): (u8, u8), drums: bool) -> Project {
    let mut c = Clip::new(0.0, 16.0);
    let mut note = |pitch: u8, start: f64, length: f64| {
        let mut n = Note::new(pitch, start, length);
        n.velocity = 0.8;
        c.notes.push(n);
    };
    if drums {
        for b in 0..16 {
            note(if b % 2 == 0 { 36 } else { 38 }, f64::from(b), 0.25);
            note(42, f64::from(b), 0.25);
            note(42, f64::from(b) + 0.5, 0.25);
        }
    } else {
        let mid = (i32::from(lo) + i32::from(hi)) / 2;
        for (b, off) in [-12, -5, 0, 4, 7, 12, 7, 4, -12, -5, 0, 4, 7, 12, 7, 0].into_iter().enumerate() {
            note((mid + off).clamp(i32::from(lo), i32::from(hi)) as u8, b as f64, 0.9);
        }
    }
    let mut t = Track::new("t", spec);
    t.instrument_params = params.clone();
    t.clips.push(c);
    let mut p = Project::new("level", 100.0);
    p.tracks.push(t);
    p
}

/// 乐器输出端补的增益（dB）。不在表里（SFZ、插件、新加还没测的）为 0。
pub fn trim_db(name: &str) -> f64 {
    MEASURED.iter().find(|(n, _)| *n == name).map_or(0.0, |&(_, lufs)| (TARGET_LUFS - f64::from(lufs)).clamp(-30.0, 30.0))
}

/// 未校准时参考乐句的响度（LUFS，乐器名 → 响度）。生成的，见模块说明。
pub static MEASURED: &[(&str, f32)] = &[
    ("drums.808", -30.8),
    ("drums.909", -30.3),
    ("drums.606", -35.0),
    ("drums.cr78", -35.0),
    ("drums.acoustic", -34.4),
    ("drums.trap", -26.0),
    ("drums.techno", -30.1),
    ("drums.house", -30.5),
    ("drums.dnb", -30.7),
    ("drums.lofi", -32.2),
    ("synth.poly", -15.3),
    ("synth.fm", -25.0),
    ("guitar.nylon", -21.1),
    ("guitar.steel", -31.0),
    ("yinse.pluck.electric_guitar", -15.4),
    ("yinse.pluck.electric_guitar_clean", -17.4),
    ("yinse.pluck.guzheng", -27.2),
    ("yinse.pluck.pipa", -31.3),
    ("yinse.pluck.guqin", -30.3),
    ("yinse.strike.yangqin", -29.2),
    ("yinse.blow.dizi", -19.9),
    ("yinse.blow.xiao", -24.4),
    ("yinse.bow.erhu", -24.1),
    ("yinse.blow.accordion", -19.5),
    ("yinse.blow.accordion_dry", -17.8),
    ("yinse.blow.bandoneon", -22.0),
    ("bass.finger", -18.6),
    ("bass.slap", -22.8),
    ("bass.acid", -22.0),
    ("bass.acid_squelch", -23.9),
    ("bass.acid_square", -20.3),
    ("organ.jazz", -26.7),
    ("organ.gospel", -24.9),
    ("organ.full", -23.2),
    ("organ.ballad", -25.8),
    ("mallet.marimba", -22.5),
    ("mallet.xylophone", -26.7),
    ("mallet.vibraphone", -24.1),
    ("mallet.glockenspiel", -24.3),
    ("mallet.celesta", -26.6),
    ("mallet.kalimba", -21.2),
    ("mallet.steel_pan", -21.3),
    ("mallet.tubular_bells", -17.0),
    ("strings.violin", -28.6),
    ("strings.viola", -30.2),
    ("strings.cello", -31.8),
    ("strings.contrabass", -31.8),
    ("strings.ensemble", -31.0),
    ("strings.violin_pizz", -45.0),
    ("strings.cello_pizz", -38.8),
    ("strings.contrabass_pizz", -36.3),
    ("wind.flute", -22.8),
    ("wind.clarinet", -21.3),
    ("wind.sax", -22.9),
    ("wind.trumpet", -23.4),
    ("wind.trombone", -25.6),
    ("choir.aah", -22.0),
    ("choir.ooh", -21.1),
    ("choir.solo", -18.0),
    ("poly.moog_bass", -23.0),
    ("poly.reese", -17.3),
    ("poly.wobble_bass", -18.7),
    ("poly.deep_sub", -14.9),
    ("poly.fm_growl", -16.8),
    ("poly.house_organ_bass", -19.5),
    ("poly.acid_line", -18.6),
    ("poly.garage_square", -19.8),
    ("poly.wavetable_growl", -15.0),
    ("poly.synthpop_bass", -19.1),
    ("poly.funk_bass", -28.5),
    ("poly.808_glide", -16.1),
    ("poly.rolling_bass", -28.0),
    ("poly.rubber_bass", -18.1),
    ("poly.talking_bass", -19.3),
    ("poly.trance_supersaw", -18.3),
    ("poly.square_lead", -15.4),
    ("poly.sync_lead", -14.7),
    ("poly.chiptune", -15.5),
    ("poly.hoover", -19.9),
    ("poly.mono_saw_lead", -18.4),
    ("poly.whistle_lead", -15.4),
    ("poly.digital_lead", -18.0),
    ("poly.talking_lead", -24.7),
    ("poly.screamer", -13.0),
    ("poly.future_lead", -18.3),
    ("poly.warm_analog_pad", -19.5),
    ("poly.string_machine", -21.0),
    ("poly.glass_pad", -21.3),
    ("poly.dark_drone", -23.4),
    ("poly.choir_pad", -18.9),
    ("poly.formant_pad", -17.5),
    ("poly.pwm_pad", -16.8),
    ("poly.shimmer_pad", -20.5),
    ("poly.evolving_spectra", -24.0),
    ("poly.soft_sine_pad", -18.5),
    ("poly.comb_wash", -20.1),
    ("poly.sweep_pad", -28.3),
    ("poly.house_pluck", -26.8),
    ("poly.bell_pluck", -16.3),
    ("poly.marimba", -20.4),
    ("poly.plucked_string", -31.1),
    ("poly.future_pluck", -23.0),
    ("poly.glass_pluck", -21.1),
    ("poly.trance_pluck", -26.5),
    ("poly.pizzicato", -27.7),
    ("poly.kalimba", -19.0),
    ("poly.polysynth_piano", -23.0),
    ("poly.clavinet", -27.5),
    ("poly.fm_electric_piano", -17.8),
    ("poly.drawbar_organ", -15.7),
    ("poly.juno_keys", -19.0),
    ("poly.wurly", -17.1),
    ("poly.vibraphone", -18.7),
    ("poly.synth_brass", -20.7),
    ("poly.brass_stab", -26.6),
    ("poly.jump_brass", -19.4),
    ("poly.rave_chord_stab", -27.7),
    ("poly.soft_horns", -18.1),
    ("poly.fanfare", -19.6),
    ("poly.riser", -21.3),
    ("poly.downlifter", -20.0),
    ("poly.noise_sweep", -31.0),
    ("poly.laser_zap", -24.2),
    ("poly.random_bleeps", -17.7),
    ("poly.wind", -22.7),
    ("poly.impact_boom", -15.5),
    ("poly.siren", -11.3),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{render_loudness, RenderCache, RenderSettings};

    fn measure(name: &str) -> f64 {
        let l = render_loudness(&reference(name), RenderSettings { tail_seconds: 1.0, ..Default::default() }, &mut RenderCache::new()).unwrap();
        l[0].output.lufs.unwrap()
    }

    /// 生成 [`MEASURED`]：先把表清空（或只测未校准的输出），这里减掉已补的增益得到乐器本来的响度。
    #[test]
    #[ignore]
    fn print_table() {
        for name in crate::instrument::available() {
            println!("    (\"{name}\", {:.1}),", measure(&name) - trim_db(&name));
        }
    }

    #[test]
    fn calibrated_instruments_are_about_equally_loud() {
        for name in ["strings.cello", "strings.violin_pizz", "poly.siren", "drums.808", "guitar.steel"] {
            let l = measure(name);
            assert!((l - TARGET_LUFS).abs() < 1.0, "{name}: {l} LUFS");
        }
    }
}
