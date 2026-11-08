//! 电吉他（`guitar.electric`、`guitar.electric_clean`）：manguebyte 的拨弦物理模型，没有琴箱，用电磁拾音器听弦。
//!
//! 和原声吉他的区别都在参数上：实心琴身让弦响得久、高频留得住（失真之后才有"长延音"）；
//! 拾音器只听它下面那一点的振动，位置决定亮暗（琴桥亮而尖、琴颈圆而暖），线圈决定共振峰
//! （单线圈的共振峰高而尖，双线圈低而宽、输出更大）。
//!
//! 每个音可以掌闷（音符上的 `"mute": true`）：闷着的音由另一组弦来弹，衰减短、更暗；
//! 同一根弦（同一个音高）上一次的音被新音打断，开的闷的都一样。
//!
//! 不像合成器的两处：每一次拨弦都略有不同（拨的位置、亮度、力度方式按音符的序号确定地变一点，
//! 连着闷音的 chug 不再一模一样）；每个音高带一点固定的走音（±3 音分，像一把真吉他的弦与品），
//! 和弦里的五度、八度有慢慢的拍频。两者都由音高与第几个音决定，同一个工程每次渲染结果相同。

use manguebyte::dsp::instruments::{BodyMode, PluckedString, StringParams};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth};

use crate::params;

/// 拾音器的位置（离琴桥占弦长的比例）：琴桥、中间、琴颈。
const PICKUPS: [f64; 3] = [0.08, 0.15, 0.25];

/// 电吉他的音色：开着弹的弦，加上拾音器、线圈、掌闷的深浅与扫弦。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElectricParams {
    /// 开着弹的弦（拾音器位置、共振峰由下面三项算出，这里的不用）。
    pub string: StringParams,
    /// 拾音器：0 琴桥、1 中间、2 琴颈，中间值在两者之间。
    pub pickup: f64,
    /// 线圈：0 单线圈、1 双线圈。
    pub coil: f64,
    /// 掌闷的深浅（写了 `"mute": true` 的音），0–1。
    pub mute: f64,
    /// 扫弦：同时按下 3 个以上的音时，相邻两根弦先后响的间隔（毫秒）。
    pub strum_ms: f64,
}

impl ElectricParams {
    fn base(pickup: f64, coil: f64) -> Self {
        Self {
            string: StringParams {
                decay: 9.0,
                decay_per_octave: 1.35,
                high_decay: 0.45,
                release: 0.08,
                pluck_position: 0.13,
                pluck_brightness: 7_000.0,
                velocity_darkening: 1.5,
                velocity_range_db: 18.0,
                stiffness: 0.06,
                strike: 0.45,
                pluck_noise: 0.2,
                body: [BodyMode::NONE; 4],
                body_mix: 0.0,
                pickup_position: PICKUPS[0],
                tone: 3_400.0,
                tone_q: 1.8,
                buzz: 0.0,
                buzz_time: 0.05,
                accent_db: 3.0,
                accent_brightness: 0.5,
                accent_buzz: 0.0,
                level: 0.5,
            },
            pickup,
            coil,
            mute: 0.7,
            strum_ms: 8.0,
        }
    }

    /// 双线圈、琴桥拾音器：给失真、摇滚、金属。
    pub fn humbucker_bridge() -> Self {
        Self::base(0.0, 1.0)
    }

    /// 单线圈、琴颈拾音器：清音、放克、抒情。
    pub fn single_coil_neck() -> Self {
        Self::base(2.0, 0.0)
    }

    /// 开着弹、掌闷两组弦的参数。
    pub fn strings(&self) -> (StringParams, StringParams) {
        let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t.clamp(0.0, 1.0);
        let k = self.pickup.clamp(0.0, 2.0);
        let i = (k.floor() as usize).min(1);
        let mut open = self.string;
        open.pickup_position = lerp(PICKUPS[i], PICKUPS[i + 1], k - i as f64);
        // 单线圈：共振峰约 5 kHz、尖；双线圈：约 3 kHz、宽，输出大一些。
        open.tone = lerp(5_200.0, 3_000.0, self.coil);
        open.tone_q = lerp(2.4, 1.3, self.coil);
        open.level = self.string.level * lerp(0.8, 1.2, self.coil);
        let m = self.mute.clamp(0.0, 1.0);
        let mut muted = open;
        // 掌根压在琴桥上：弦很快停下来，高频先没。
        // 衰减按比例缩短（对数刻度）：深浅 1 时约 0.15 秒，0.7 时约半秒。
        muted.decay = (open.decay * (0.15 / open.decay).min(1.0).powf(m)).max(0.05);
        muted.decay_per_octave = 1.0;
        muted.high_decay = (open.high_decay * lerp(1.0, 0.35, m)).max(0.01);
        muted.pluck_brightness = (open.pluck_brightness * lerp(1.0, 0.5, m)).max(100.0);
        muted.release = open.release.min(0.05);
        muted.level = open.level * lerp(1.0, 1.25, m);
        (open, muted)
    }
}

/// 一把电吉他：开着弹的一组弦 + 掌闷的一组弦。
pub(crate) struct Electric {
    open: Synth<PluckedString, f32>,
    muted: Synth<PluckedString, f32>,
    params: ElectricParams,
    scratch: AudioBuffer<f32>,
    /// 拨过几次（每次拨弦的变化由它与音高决定）。
    plucks: u32,
}

/// 0–1 之间确定的伪随机数。
fn unit(a: u64, b: u64) -> f64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0xD1B5_4A32_D192_ED03);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// 工程的调律加上每个音高固定的一点走音（±3 音分）。
fn detuned(t: &manguebyte::dsp::Tuning) -> manguebyte::dsp::Tuning {
    let pitches: [f64; 128] = std::array::from_fn(|k| t.key_pitch(k as u8) + (unit(k as u64, 7) - 0.5) * 0.06);
    manguebyte::dsp::Tuning::from_pitches(&pitches).unwrap_or_else(|_| t.clone())
}

impl Electric {
    pub(crate) fn new(params: ElectricParams, voices: usize) -> Self {
        let (o, m) = params.strings();
        let mut e = Self {
            open: Synth::with_voices(voices, move || PluckedString::new(o)),
            muted: Synth::with_voices(voices, move || PluckedString::new(m)),
            params,
            scratch: AudioBuffer::new(2, 0),
            plucks: 0,
        };
        e.set_tuning(&manguebyte::dsp::Tuning::default());
        e
    }

    fn push(&mut self) {
        let (o, m) = self.params.strings();
        self.open.voices_mut().for_each(|v| v.set_params(o));
        self.muted.voices_mut().for_each(|v| v.set_params(m));
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.open, spec);
        Processor::prepare(&mut self.muted, spec);
        self.scratch = AudioBuffer::new(spec.num_channels.max(2), spec.max_block_size);
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool, mute: bool) {
        // 同一根弦重新拨：上一个音停下（不管它是开的还是闷的）。
        self.plucks = self.plucks.wrapping_add(1);
        let (o, m) = self.params.strings();
        let mut p = if mute { m } else { o };
        // 这一次拨弦的变化：只改"下一个音才用"的拨弦参数，正在响的音不受影响。
        let r = |i: u64| unit(u64::from(self.plucks), i * 131 + u64::from(note.key)) * 2.0 - 1.0;
        p.pluck_position = (p.pluck_position * (1.0 + 0.2 * r(1))).clamp(0.02, 0.5);
        p.pluck_brightness = (p.pluck_brightness * (1.0 + 0.3 * r(2))).clamp(100.0, 20_000.0);
        p.strike = (p.strike + 0.08 * r(3)).clamp(0.0, 1.0);
        p.pluck_noise = (p.pluck_noise * (1.0 + 0.3 * r(4))).clamp(0.0, 1.0);
        let (play, other) = if mute { (&mut self.muted, &mut self.open) } else { (&mut self.open, &mut self.muted) };
        play.voices_mut().for_each(|v| v.set_params(p));
        other.note_off(note, 0.0);
        play.note_on_accented(note, (f64::from(velocity) * (1.0 + 0.05 * r(5))).clamp(0.0, 1.0) as f32, accent);
    }

    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.open.note_off(note, velocity);
        self.muted.note_off(note, velocity);
    }

    pub(crate) fn midi(&mut self, message: MidiMessage) {
        let e = manguebyte::prelude::Event::from_midi(0, message);
        self.open.handle_event(&e);
        self.muted.handle_event(&e);
    }

    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.open, buffer);
        let n = buffer.num_frames();
        if self.scratch.num_frames() < n || self.scratch.num_channels() < buffer.num_channels() {
            self.scratch = AudioBuffer::new(buffer.num_channels(), n);
        }
        self.scratch.set_num_frames(n);
        for c in 0..self.scratch.num_channels() {
            self.scratch.channel_mut(c).fill(0.0);
        }
        Processor::process(&mut self.muted, &mut self.scratch);
        for c in 0..buffer.num_channels() {
            for (y, x) in buffer.channel_mut(c).iter_mut().zip(self.scratch.channel(c)) {
                *y += *x;
            }
        }
    }

    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::ELECTRIC, &mut self.params, name, value) {
            self.push();
        }
    }

    pub(crate) fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        let t = detuned(tuning);
        self.open.set_tuning(&t);
        self.muted.set_tuning(&t);
    }
}

/// 这条音轨的扫弦间隔（秒）：电吉他取参数 `strum`（缺省 [`ElectricParams`] 的），其它乐器不扫。
pub(crate) fn strum_seconds(instrument: &str, params: &muser_core::InstrumentParams) -> f64 {
    if !instrument.trim().to_lowercase().starts_with("guitar.electric") {
        return 0.0;
    }
    let ms = params.get("strum").and_then(serde_json::Value::as_f64).unwrap_or(ElectricParams::humbucker_bridge().strum_ms);
    ms.clamp(0.0, 50.0) / 1000.0
}

/// 一个和弦（同一刻起音的几个音）是不是上扫：落在八分音符的反拍、或十六分音符的反拍上的是上扫（高音弦先响），其余下扫。
pub(crate) fn upstroke(start_beats: f64) -> bool {
    let frac = start_beats.rem_euclid(1.0);
    (frac - 0.5).abs() < 0.02 || (frac - 0.25).abs() < 0.02 || (frac - 0.75).abs() < 0.02
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickups_and_mute_shape_the_strings() {
        let bridge = ElectricParams::humbucker_bridge().strings().0;
        let neck = ElectricParams { pickup: 2.0, ..ElectricParams::humbucker_bridge() }.strings().0;
        assert!(bridge.pickup_position < neck.pickup_position);
        let single = ElectricParams::single_coil_neck().strings().0;
        assert!(single.tone > neck.tone && single.tone_q > neck.tone_q);
        let (open, muted) = ElectricParams::humbucker_bridge().strings();
        assert!(muted.decay < open.decay * 0.1 && muted.pluck_brightness < open.pluck_brightness);
        let none = ElectricParams { mute: 0.0, ..ElectricParams::humbucker_bridge() }.strings();
        assert_eq!(none.1.decay, none.0.decay);
        assert!(upstroke(1.5) && upstroke(2.25) && !upstroke(3.0) && !upstroke(0.98));
    }
}
