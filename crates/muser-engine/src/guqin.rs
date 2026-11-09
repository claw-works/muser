//! 古琴的模态弦模型：散音、按音、泛音分别决定激发、接触损耗和保留的振动模式。
//! 模式的相位连续累积，走手只改变频率与损耗，不重新拨弦，也不移动整数延迟读点。
//! 这仍是可调的合成模型，不是对某张琴、某种弦材的精确复原。

use core::f64::consts::{PI, TAU};

use manguebyte::dsp::{Tuning, instruments::StringParams};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth, Voice};

use crate::{chinese::GuzhengParams, params};

const MODES: usize = 48;
const CONTROL: u64 = 16;
const OPEN_KEYS: [u8; 7] = [36, 38, 41, 43, 45, 48, 50];
pub(crate) const TECHNIQUES: &[&str] = &["auto", "open", "pressed", "harmonic"];
pub(crate) const BEND_RANGE: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GuqinParams {
    pub pluck: GuzhengParams,
    pub technique: f64,
    /// 触弦节点分母：2 是弦长的一半，3 是三分之一。
    pub harmonic: f64,
    pub pressure: f64,
    pub slide_time: f64,
    pub slide_noise: f64,
    pub sympathetic: f64,
}

impl Default for GuqinParams {
    fn default() -> Self {
        let mut pluck = GuzhengParams::guqin();
        pluck.string.decay = 9.0;
        pluck.string.decay_per_octave = 1.15;
        pluck.string.release = 6.0;
        pluck.string.high_decay = 0.08;
        pluck.string.pluck_position = 0.12;
        pluck.string.pluck_brightness = 5_000.0;
        pluck.string.strike = 0.35;
        pluck.string.pluck_noise = 0.15;
        pluck.string.tone = 7_000.0;
        Self { pluck, technique: 0.0, harmonic: 4.0, pressure: 0.65, slide_time: 0.008, slide_noise: 0.35, sympathetic: 0.18 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Technique {
    Open,
    Pressed,
    Harmonic,
}

impl GuqinParams {
    fn with_note(mut self, note: muser_core::GuqinNote) -> Self {
        self.technique = match note.technique {
            muser_core::GuqinTechnique::Auto => 0.0,
            muser_core::GuqinTechnique::Open => 1.0,
            muser_core::GuqinTechnique::Pressed => 2.0,
            muser_core::GuqinTechnique::Harmonic => 3.0,
        };
        if let Some(h) = note.harmonic {
            self.harmonic = f64::from(h);
        }
        self
    }

    fn technique(self, key: u8) -> Technique {
        match self.technique.round() as u8 {
            1 => Technique::Open,
            2 => Technique::Pressed,
            3 => Technique::Harmonic,
            _ if OPEN_KEYS.contains(&key) => Technique::Open,
            _ => Technique::Pressed,
        }
    }
}

/// 渲染器据此只给按音加吟猱。显式弯音在引擎里同样只作用于按音。
pub(crate) fn pressed(instrument: &str, params: &muser_core::InstrumentParams, key: u8, note: Option<muser_core::GuqinNote>) -> bool {
    if !instrument.trim().eq_ignore_ascii_case("yinse.pluck.guqin") {
        return true;
    }
    let technique = match params.get("technique") {
        Some(serde_json::Value::String(s)) => TECHNIQUES.iter().position(|&name| name == s).unwrap_or(0) as f64,
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    };
    let p = GuqinParams { technique, ..Default::default() };
    note.map_or(p, |n| p.with_note(n)).technique(key) == Technique::Pressed
}

#[derive(Clone, Copy, Default)]
struct Mode {
    re: f64,
    im: f64,
    rot_re: f64,
    rot_im: f64,
    ratio: f64,
    t60: f64,
    fade: f64,
}

impl Mode {
    fn tune(&mut self, fundamental: f64, sr: f64, released: Option<f64>) {
        let hz = fundamental * self.ratio;
        // 高音和大幅滑音中让超出可用带宽的模式渐退，不折回成混叠音。
        self.fade = ((0.45 * sr - hz) / (0.05 * sr)).clamp(0.0, 1.0);
        let seconds = released.map_or(self.t60, |r| self.t60.min(r)).max(0.005);
        let loss = (-3.0 * 10f64.ln() / (seconds * sr)).exp();
        let angle = TAU * hz.min(0.45 * sr) / sr;
        self.rot_re = loss * angle.cos();
        self.rot_im = loss * angle.sin();
    }

    fn tick(&mut self) -> f64 {
        let re = self.re * self.rot_re - self.im * self.rot_im;
        self.im = self.re * self.rot_im + self.im * self.rot_re;
        self.re = re;
        self.im * self.fade
    }

    fn energy(&self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

#[derive(Clone, Copy)]
struct Tone {
    modes: [Mode; MODES],
    params: GuqinParams,
    technique: Technique,
    key_pitch: f64,
    pitch: f64,
    last_pitch: f64,
    at: u64,
    held: bool,
    active: bool,
    rng: u64,
    noise_lp: f64,
    friction_lp: f64,
    motion: f64,
    strength: f64,
    energy: f64,
}

impl Default for Tone {
    fn default() -> Self {
        Self {
            modes: [Mode::default(); MODES],
            params: GuqinParams::default(),
            technique: Technique::Open,
            key_pitch: 60.0,
            pitch: 60.0,
            last_pitch: 60.0,
            at: 0,
            held: false,
            active: false,
            rng: 1,
            noise_lp: 0.0,
            friction_lp: 0.0,
            motion: 0.0,
            strength: 0.0,
            energy: 0.0,
        }
    }
}

impl Tone {
    fn start(p: GuqinParams, tuning: &Tuning, key: u8, bend: f64, velocity: f32, accent: bool, sr: f64) -> Self {
        let technique = p.technique(key);
        let s = p.pluck.string;
        let key_pitch = tuning.key_pitch(key);
        let pitch = key_pitch + if technique == Technique::Pressed { bend } else { 0.0 };
        let f0 = 440.0 * ((pitch - 69.0) / 12.0).exp2();
        let anchor = if technique == Technique::Harmonic { p.harmonic.round().clamp(2.0, 8.0) as usize } else { 1 };
        let velocity = f64::from(velocity).clamp(0.0, 1.0);
        let strength = 10f64.powf(-s.velocity_range_db * (1.0 - velocity) / 20.0)
            * if accent { 10f64.powf(s.accent_db / 20.0) } else { 1.0 };
        let brightness = s.pluck_brightness * (-s.velocity_darkening * (1.0 - velocity)).exp2()
            * if accent { s.accent_brightness.exp2() } else { 1.0 };
        // 按音中发声弦段变短：同一处右手触弦，相对于该段的位置会向中间移动。
        let open = OPEN_KEYS.iter().rev().find(|&&k| f64::from(k) <= key_pitch - 2.0).copied().unwrap_or(36);
        let open_hz = tuning.key_frequency(open);
        let position = if technique == Technique::Pressed { s.pluck_position * f0 / open_hz } else { s.pluck_position }.clamp(0.02, 0.45);
        let t60 = s.decay / s.decay_per_octave.powf((f0 / 261.625_565).log2());
        let contact = if technique == Technique::Pressed { 1.0 - 0.55 * p.pressure } else { 1.0 };
        let high_decay = if technique == Technique::Harmonic { s.high_decay.max(0.2) } else { s.high_decay };
        let slope = (1.0 / high_decay - 1.0) / ((3_000.0 / f0).max(2.0).powf(1.5) - 1.0);
        let stiffness = s.stiffness * if technique == Technique::Pressed { 0.001 } else { 0.0006 };
        let mut tone = Self {
            params: p, technique, key_pitch, pitch, last_pitch: pitch,
            held: true, active: true, rng: 0x9E37_79B9_7F4A_7C15 ^ (u64::from(key) << 32), strength, ..Default::default()
        };
        let mut norm = 0.0;
        for (i, mode) in tone.modes.iter_mut().enumerate() {
            let n = i + 1;
            let ratio = n as f64 / anchor as f64
                * ((1.0 + stiffness * (n * n) as f64) / (1.0 + stiffness * (anchor * anchor) as f64)).sqrt();
            let shape = (PI * n as f64 * position).sin()
                * ((1.0 - s.strike) / (n * n) as f64 + s.strike / n as f64);
            let bright = 1.0 / (1.0 + (f0 * ratio / brightness.max(100.0)).powi(2));
            let allowed = technique != Technique::Harmonic || n % anchor == 0;
            let amplitude = shape * bright * if allowed { 1.0 } else { 0.025 };
            let mut seconds = t60 * contact / (1.0 + slope * (ratio.powf(1.5) - 1.0).max(0.0));
            if !allowed {
                // 触弦点不在节点的模式迅速耗掉；只有整数倍 anchor 的模式可以留下。
                let touch_loss = 480.0 * (PI * n as f64 / anchor as f64).sin().powi(2);
                seconds = 3.0 * 10f64.ln() / (3.0 * 10f64.ln() / seconds + touch_loss);
            }
            *mode = Mode { re: amplitude, ratio, t60: seconds, ..Default::default() };
            mode.tune(f0, sr, None);
            if allowed && ratio <= 6.1 {
                norm += amplitude * amplitude;
            }
        }
        // 若右手也拨在触弦节点，保留模式几乎没有能量，不能用它们反向放大其余瞬态。
        let total_energy = tone.modes.iter().map(Mode::energy).sum::<f64>();
        let gain = 0.22 * s.level * strength / norm.max(0.5 * total_energy).max(1e-12).sqrt()
            * if technique == Technique::Harmonic { 0.65 } else { 1.0 };
        for mode in &mut tone.modes {
            mode.re *= gain;
        }
        tone
    }

    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        2.0 * (self.rng >> 11) as f64 / (1u64 << 53) as f64 - 1.0
    }

    fn sample(&mut self, sr: f64, bend: f64, live: GuqinParams) -> f64 {
        if !self.active {
            return 0.0;
        }
        let target = self.key_pitch + if self.technique == Technique::Pressed { bend } else { 0.0 };
        let k = 1.0 - (-1.0 / (live.slide_time.max(0.001) * sr)).exp();
        let before = self.pitch;
        self.pitch += (target - self.pitch) * k;
        let speed = (self.pitch - before).abs() * sr;
        let km = 1.0 - (-1.0 / (0.012 * sr)).exp();
        self.motion += (speed.min(48.0) - self.motion) * km;
        if self.at.is_multiple_of(CONTROL) {
            let f0 = 440.0 * ((self.pitch - 69.0) / 12.0).exp2();
            let sliding = if self.technique == Technique::Pressed { (self.pitch - self.last_pitch).abs() } else { 0.0 };
            let friction_loss = (-0.09 * live.pressure * sliding).exp();
            let release = (!self.held).then_some(self.params.pluck.string.release
                * if self.technique == Technique::Pressed { 0.35 } else { 1.0 });
            self.energy = 0.0;
            for mode in &mut self.modes {
                mode.re *= friction_loss;
                mode.im *= friction_loss;
                mode.tune(f0, sr, release);
                self.energy += mode.energy();
            }
            self.last_pitch = self.pitch;
            if self.energy < 1e-11 && self.at > (0.1 * sr) as u64 {
                self.active = false;
                return 0.0;
            }
        }
        let mut x = 0.0;
        for mode in &mut self.modes {
            x += mode.tick();
        }
        let t = self.at as f64 / sr;
        let n = self.noise();
        let s: StringParams = self.params.pluck.string;
        let kn = 1.0 - (-TAU * s.pluck_brightness.min(0.4 * sr) / sr).exp();
        self.noise_lp += (n - self.noise_lp) * kn;
        x += self.noise_lp * s.level * self.strength * s.pluck_noise * 0.035 * (-t / 0.006).exp();
        // 走手摩擦随手速和仍在振动的弦能量变化；停止移动后自己收掉，不再拨一次弦。
        let kf = 1.0 - (-TAU * 1_200.0 / sr).exp();
        self.friction_lp += (n - self.friction_lp) * kf;
        if self.technique == Technique::Pressed {
            x += self.friction_lp * self.energy.sqrt() * live.slide_noise * 0.16 * (self.motion / 8.0).sqrt().min(2.0);
        }
        let attack = (t / 0.0015).clamp(0.0, 1.0);
        self.at += 1;
        x * attack * attack * (3.0 - 2.0 * attack)
    }
}

struct GuqinVoice {
    params: GuqinParams,
    tuning: Tuning,
    sr: f64,
    bend: f64,
    now: Tone,
    old: Tone,
    fade: f64,
    notes: u64,
}

impl GuqinVoice {
    fn new(params: GuqinParams) -> Self {
        Self { params, tuning: Tuning::default(), sr: 48_000.0, bend: 0.0, now: Tone::default(), old: Tone::default(), fade: 0.0, notes: 0 }
    }

    fn retire(&mut self) {
        if self.now.active {
            self.old = self.now;
            self.fade = 1.0;
            self.now.active = false;
        }
    }
}

impl Voice<f32> for GuqinVoice {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.now = Tone::default();
        self.old = Tone::default();
        self.fade = 0.0;
    }
    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        self.note_on_accented(note, velocity, false);
    }
    fn note_on_accented(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.retire();
        self.notes += 1;
        self.now = Tone::start(self.params, &self.tuning, note.key, self.bend, velocity, accent, self.sr);
        self.now.rng = (self.now.rng ^ self.notes).max(1);
    }
    fn note_off(&mut self, _velocity: f32) {
        self.now.held = false;
    }
    fn choke(&mut self) {
        self.retire();
    }
    fn is_active(&self) -> bool {
        self.now.active || (self.fade > 0.0 && self.old.active)
    }
    fn render(&mut self, buffer: &mut AudioBuffer<f32>) {
        for i in 0..buffer.num_frames() {
            let mut x = self.now.sample(self.sr, self.bend, self.params);
            if self.fade > 0.0 {
                // 被偷走的声部保持原音高，10 ms 内退下去，新的弯音不传给它。
                x += self.old.sample(self.sr, self.old.pitch - self.old.key_pitch, self.old.params) * self.fade;
                self.fade = (self.fade - 1.0 / (0.01 * self.sr)).max(0.0);
            }
            for c in 0..buffer.num_channels() {
                buffer.channel_mut(c)[i] += x as f32;
            }
        }
    }
    fn pitch_bend(&mut self, semitones: f64) {
        if semitones.is_finite() {
            self.bend = semitones.clamp(-BEND_RANGE, BEND_RANGE);
        }
    }
    fn set_tuning(&mut self, tuning: &Tuning) {
        self.tuning.clone_from(tuning);
    }
}

/// 峰值归一的带通。琴体用宽的模式，其余弦的共鸣用窄而长的模式。
#[derive(Clone, Copy, Default)]
struct Resonance {
    b0: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Resonance {
    fn set(&mut self, hz: f64, bw: f64, sr: f64) {
        let w = TAU * hz.min(0.45 * sr) / sr;
        let alpha = w.sin() * bw / (2.0 * hz.max(1.0));
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * (x - self.x2) - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub(crate) struct Guqin {
    synth: Synth<GuqinVoice, f32>,
    params: GuqinParams,
    sr: f64,
    tuning: Tuning,
    body: [Resonance; 4],
    sympathetic: [Resonance; 21],
    tilt: f64,
    dc: f64,
    pending: Vec<(NoteRef, muser_core::GuqinNote)>,
}

impl Guqin {
    pub(crate) fn new(params: GuqinParams) -> Self {
        let mut synth = Synth::with_voices(12, move || GuqinVoice::new(params));
        synth.set_pitch_bend_range(BEND_RANGE);
        Self { synth, params, sr: 48_000.0, tuning: Tuning::default(), body: [Resonance::default(); 4], sympathetic: [Resonance::default(); 21], tilt: 0.0, dc: 0.0, pending: Vec::with_capacity(16) }
    }
    fn configure(&mut self) {
        for (r, mode) in self.body.iter_mut().zip(self.params.pluck.string.body) {
            r.set(mode.frequency, mode.frequency / mode.q, self.sr);
        }
        for (i, &key) in OPEN_KEYS.iter().enumerate() {
            for n in 1..=3 {
                let hz = self.tuning.key_frequency(key) * n as f64;
                self.sympathetic[i * 3 + n - 1].set(hz, 3.0 * 10f64.ln() / (PI * 3.5), self.sr);
            }
        }
    }
    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.body = [Resonance::default(); 4];
        self.sympathetic = [Resonance::default(); 21];
        self.tilt = 0.0;
        self.dc = 0.0;
        self.pending.clear();
        self.synth.prepare(spec);
        self.configure();
    }
    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let p = self.pending.iter().position(|(n, _)| *n == note)
            .map_or(self.params, |i| self.params.with_note(self.pending.swap_remove(i).1));
        self.synth.voices_mut().for_each(|v| v.params = p);
        self.synth.note_on_accented(note, velocity, accent);
        self.synth.voices_mut().for_each(|v| v.params = self.params);
    }
    pub(crate) fn playing(&mut self, note: NoteRef, technique: muser_core::GuqinNote) {
        self.pending.retain(|(n, _)| *n != note);
        self.pending.push((note, technique));
    }
    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
    }
    pub(crate) fn midi(&mut self, message: MidiMessage) {
        self.synth.handle_event(&manguebyte::prelude::Event::from_midi(0, message));
    }
    pub(crate) fn set_tuning(&mut self, tuning: &Tuning) {
        self.tuning.clone_from(tuning);
        self.synth.set_tuning(tuning);
        self.configure();
    }
    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::GUQIN, &mut self.params, name, value) {
            self.synth.voices_mut().for_each(|v| v.params = self.params);
        }
    }
    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.synth.process(buffer);
        if buffer.num_channels() == 0 {
            return;
        }
        let s = self.params.pluck.string;
        let kt = 1.0 - (-TAU * s.tone.min(0.4 * self.sr) / self.sr).exp();
        let kd = 1.0 - (-TAU * 20.0 / self.sr).exp();
        for i in 0..buffer.num_frames() {
            let source = f64::from(buffer.channel(0)[i]);
            let mut x = source;
            for (r, mode) in self.body.iter_mut().zip(s.body) {
                x += r.tick(source) * mode.gain * s.body_mix;
            }
            let mut sympathetic = 0.0;
            for (j, r) in self.sympathetic.iter_mut().enumerate() {
                sympathetic += r.tick(source) * [1.0, 0.3, 0.14][j % 3];
            }
            x += sympathetic * self.params.sympathetic * 0.4;
            self.tilt += (x - self.tilt) * kt;
            self.dc += (self.tilt - self.dc) * kd;
            let out = (self.tilt - self.dc) as f32;
            for c in 0..buffer.num_channels() {
                buffer.channel_mut(c)[i] = out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(p: GuqinParams, key: u8, seconds: f64) -> Vec<f64> {
        let sr = 48_000.0;
        let mut tone = Tone::start(p, &Tuning::default(), key, 0.0, 0.8, false, sr);
        (0..(seconds * sr) as usize).map(|_| tone.sample(sr, 0.0, p)).collect()
    }

    fn energy_at(x: &[f64], hz: f64) -> f64 {
        let mut re = 0.0;
        let mut im = 0.0;
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (TAU * i as f64 / x.len() as f64).cos();
            let angle = TAU * hz * i as f64 / 48_000.0;
            re += v * w * angle.cos();
            im += v * w * angle.sin();
        }
        re * re + im * im
    }

    #[test]
    fn touching_a_node_removes_the_lower_string_modes() {
        for node in 2..=8 {
            let p = GuqinParams { technique: 3.0, harmonic: node as f64, ..Default::default() };
            let x = samples(p, 69, 0.5);
            let held = &x[4_800..];
            let fundamental = energy_at(held, 440.0);
            for n in 1..node {
                assert!(energy_at(held, 440.0 * n as f64 / node as f64) < fundamental * 1e-4, "node {node}, unwanted mode {n}");
            }
            assert!(fundamental > 1e-3);
        }
    }

    #[test]
    fn open_pressed_and_harmonic_notes_have_different_spectra() {
        let open = samples(GuqinParams { technique: 1.0, ..Default::default() }, 57, 0.4);
        let pressed = samples(GuqinParams { technique: 2.0, ..Default::default() }, 57, 0.4);
        let harmonic = samples(GuqinParams { technique: 3.0, harmonic: 2.0, ..Default::default() }, 57, 0.4);
        let ratio = |x: &[f64]| energy_at(&x[4_800..], 660.0) / energy_at(&x[4_800..], 220.0);
        assert!(ratio(&harmonic) < ratio(&open) * 0.5, "harmonics should have fewer upper partials");
        assert!((ratio(&pressed) - ratio(&open)).abs() > 0.005, "stopping changes the effective pluck position and damping");
    }

    #[test]
    fn an_octave_slide_preserves_phase_and_does_not_repluck() {
        let p = GuqinParams { technique: 2.0, slide_noise: 0.0, ..Default::default() };
        let mut tone = Tone::start(p, &Tuning::default(), 57, 0.0, 0.8, false, 48_000.0);
        for _ in 0..9_600 {
            tone.sample(48_000.0, 0.0, p);
        }
        let age = tone.at;
        let energy = tone.modes.iter().map(Mode::energy).sum::<f64>();
        let mut last = tone.sample(48_000.0, 0.0, p);
        let mut biggest = 0.0f64;
        for i in 0..24_000 {
            let bend = (i as f64 / 24_000.0) * 12.0;
            let x = tone.sample(48_000.0, bend, p);
            biggest = biggest.max((x - last).abs());
            last = x;
        }
        assert_eq!(tone.at, age + 24_001);
        assert!(tone.modes.iter().map(Mode::energy).sum::<f64>() < energy);
        assert!((tone.pitch - 69.0).abs() < 0.25);
        assert!(biggest < 0.03, "continuous sliding must not create a click: {biggest}");
    }

    #[test]
    fn only_pressed_strings_follow_pitch_bend() {
        for technique in [1.0, 3.0] {
            let p = GuqinParams { technique, ..Default::default() };
            let mut tone = Tone::start(p, &Tuning::default(), 57, 12.0, 0.8, false, 48_000.0);
            for _ in 0..1_000 {
                tone.sample(48_000.0, -12.0, p);
            }
            assert_eq!(tone.pitch, 57.0);
        }
    }

    #[test]
    fn plucking_at_the_touched_node_cannot_explode_the_normalization() {
        for node in 2..=8 {
            let mut p = GuqinParams { technique: 3.0, harmonic: node as f64, ..Default::default() };
            p.pluck.string.pluck_position = 1.0 / node as f64;
            let x = samples(p, 69, 0.2);
            assert!(x.iter().all(|x| x.is_finite() && x.abs() < 0.25), "node {node}");
        }
    }

    #[test]
    fn simultaneous_note_techniques_are_not_overwritten_by_the_last_tag() {
        let mut q = Guqin::new(GuqinParams::default());
        q.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 2, max_block_size: 256 });
        let open = NoteRef::new(0, 48);
        let harmonic = NoteRef::new(0, 62);
        q.playing(open, muser_core::GuqinNote { technique: muser_core::GuqinTechnique::Open, harmonic: None });
        q.playing(harmonic, muser_core::GuqinNote { technique: muser_core::GuqinTechnique::Harmonic, harmonic: Some(2) });
        q.note_on(open, 0.8, false);
        q.note_on(harmonic, 0.8, false);
        let tones: Vec<_> = q.synth.voices_mut().filter(|v| v.now.active).map(|v| (v.now.key_pitch, v.now.technique)).collect();
        assert!(tones.contains(&(48.0, Technique::Open)));
        assert!(tones.contains(&(62.0, Technique::Harmonic)));
        assert!(q.pending.is_empty());
    }

    #[test]
    fn full_instrument_stays_finite_across_sample_rates_and_voice_stealing() {
        for sr in [22_050.0, 48_000.0, 96_000.0] {
            let mut q = Guqin::new(GuqinParams { technique: 2.0, ..Default::default() });
            q.prepare(&ProcessSpec { sample_rate: sr, num_channels: 2, max_block_size: 128 });
            let mut buf = AudioBuffer::new(2, 128);
            for i in 0..40 {
                q.midi(MidiMessage::PitchBend { channel: 0, value: if i % 2 == 0 { 8191 } else { -8192 } });
                q.note_on(NoteRef::new(0, 36 + i), 0.8, false);
                for _ in 0..8 {
                    buf.clear();
                    q.process(&mut buf);
                    assert!(buf.channel(0).iter().all(|x| x.is_finite() && x.abs() < 2.0));
                }
            }
        }
    }
}
