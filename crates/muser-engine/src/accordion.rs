//! 手风琴（`yinse.blow.accordion`、`accordion_dry`、`bandoneon`）：muser 自己的自由簧乐器，用 manguebyte 的
//! [`PartialBank`]（一组正弦泛音，各自有起振、气声）搭出来。
//!
//! - **簧片**：一片簧是一串谐波，频谱明亮（约按 1/k 衰减），簧片室（cassotto）把高频收暗。簧片起振要 20–40 毫秒，
//!   高次泛音比基音晚一点出来，开头先有一口风声，音高略低再稳住。
//! - **几组簧叠在一起**（音栓 `register`）：16 尺（低八度）、8 尺的一到三片（相互走音 `musette` 音分，就是"颤"）、
//!   4 尺（高八度）。各片簧在左右略微分开。
//! - **风箱**：整件乐器一个风压 `bellows`，越大越响也越亮；`shake` 是风箱抖动（音量的颤动）。风压、抖动、簧片室
//!   正在响的音也跟着变；音栓、musette、起振从下一个音起。
//!
//! 同一个工程每次渲染结果相同（簧片的气声、风的不稳都按音高与第几个音定种子）。

use manguebyte::dsp::{Partial, PartialAir, PartialAttack, PartialBank, Tuning, WindJitter};
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Voice};

/// 一片簧最多几个泛音。
const PARTIALS: usize = 24;
/// 最多几片簧（16 尺 + 三片 8 尺 + 4 尺）。
const REEDS: usize = 5;
/// 控制块（帧）：风压、抖动、包络每块更新一次，增益在块里直线过渡。
const BLOCK: usize = 64;

/// 音栓：哪几片簧（相对 8 尺的倍数，在 musette 里的位置 -1 / 0 / 1）。
const REGISTERS: [&[(f64, f64)]; 8] = [
    // master：16 尺 + 三片 8 尺 + 4 尺，最满。
    &[(0.5, 0.0), (1.0, -1.0), (1.0, 0.0), (1.0, 1.0), (2.0, 0.0)],
    // musette：三片 8 尺，颤得最明显。
    &[(1.0, -1.0), (1.0, 0.0), (1.0, 1.0)],
    // violin：两片 8 尺，轻轻地颤。
    &[(1.0, 0.0), (1.0, 1.0)],
    // clarinet：一片 8 尺，干净。
    &[(1.0, 0.0)],
    // bassoon：一片 16 尺，低沉。
    &[(0.5, 0.0)],
    // oboe：8 尺 + 4 尺，明亮。
    &[(1.0, 0.0), (2.0, 0.0)],
    // bandoneon：16 尺 + 8 尺（八度），探戈的声音。
    &[(0.5, 0.0), (1.0, 0.0)],
    // organ：16 尺 + 4 尺，空心。
    &[(0.5, 0.0), (2.0, 0.0)],
];
pub(crate) const REGISTER_NAMES: &[&str] = &["master", "musette", "violin", "clarinet", "bassoon", "oboe", "bandoneon", "organ"];

/// 手风琴的音色。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccordionParams {
    /// 音栓（[`REGISTER_NAMES`] 里的序号）。
    pub register: f64,
    /// 8 尺簧之间走音多少音分：0 干净，15 轻颤，25–35 法式 musette。
    pub musette: f64,
    /// 风压，0–1：越大越响越亮。
    pub bellows: f64,
    /// 风箱抖动的深浅，0–1，与快慢（Hz）。
    pub shake: f64,
    pub shake_rate: f64,
    /// 簧片室：0 开着（亮、直），1 关进去（暗、圆）。
    pub cassotto: f64,
    /// 簧片起振（秒）、松键后停下（秒）。
    pub attack: f64,
    pub release: f64,
    /// 输出（倍数）。
    pub level: f64,
}

impl AccordionParams {
    /// 法式 musette：三片 8 尺，颤得明显。
    pub fn musette() -> Self {
        Self { register: 1.0, musette: 28.0, bellows: 0.7, shake: 0.0, shake_rate: 5.5, cassotto: 0.2, attack: 0.03, release: 0.06, level: 0.12 }
    }
    /// 干净的手风琴：两片 8 尺几乎不走音，加 16 尺，民谣、探戈伴奏。
    pub fn dry() -> Self {
        Self { register: 0.0, musette: 4.0, cassotto: 0.45, ..Self::musette() }
    }
    /// 班多钮：16 尺 + 8 尺，偏暗，起振稍慢。
    pub fn bandoneon() -> Self {
        Self { register: 6.0, musette: 3.0, cassotto: 0.75, attack: 0.04, ..Self::musette() }
    }
}

/// 0–1 之间确定的伪随机数。
fn unit(a: u64, b: u64) -> f64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0xD1B5_4A32_D192_ED03);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// 一个手风琴音。
pub(crate) struct AccordionVoice {
    params: AccordionParams,
    bank: PartialBank,
    sr: f64,
    tuning: Tuning,
    key: u8,
    bend: f64,
    velocity: f64,
    /// 用着的簧：(频率倍数（含走音）, 声像)；泛音按簧排，每片 `PARTIALS` 个（太高的不用）。
    reeds: Vec<(f64, f32)>,
    /// 每片簧用了几个泛音。
    counts: [usize; REEDS],
    /// 包络：1 按着，松开后线性降到 0。
    env: f64,
    held: bool,
    active: bool,
    /// 风箱抖动的相位（转）。
    shake_phase: f64,
    /// 块里还剩几帧（到了就更新一次增益）。
    left: usize,
    notes: u64,
}

impl AccordionVoice {
    pub(crate) fn new(params: AccordionParams) -> Self {
        Self {
            params,
            bank: PartialBank::new(PARTIALS * REEDS),
            sr: 48_000.0,
            tuning: Tuning::default(),
            key: 60,
            bend: 0.0,
            velocity: 0.8,
            reeds: Vec::with_capacity(REEDS),
            counts: [0; REEDS],
            env: 0.0,
            held: false,
            active: false,
            shake_phase: 0.0,
            left: 0,
            notes: 0,
        }
    }

    pub(crate) fn set_params(&mut self, p: AccordionParams) {
        self.params = p;
    }

    fn f0(&self) -> f64 {
        440.0 * ((self.tuning.key_pitch(self.key) + self.bend - 69.0) / 12.0).exp2()
    }

    /// 第 `k` 个泛音（从 1 起）在风压 `p`、簧片室 `c` 下的相对音量。
    fn amp(k: usize, f: f64, p: f64, c: f64) -> f64 {
        // 风压大，簧片摆得更狠，高次泛音更多（倾斜变缓）。
        let tilt = 1.25 - 0.45 * p;
        let cut = 7_000.0 - 4_500.0 * c;
        let lp = 1.0 / (1.0 + (f / cut).powi(4));
        // 簧片室还有一个共鸣（约 1.2 kHz），关得越深越明显。
        let body = 1.0 + 0.6 * c * (-((f / 1_200.0).ln()).powi(2) * 4.0).exp();
        (k as f64).powf(-tilt) * lp * body
    }

    /// 重新排泛音的频率（音高、弯音、调律变了时）。
    fn tune(&mut self) {
        let f0 = self.f0();
        let top = (0.45 * self.sr).min(16_000.0);
        for (r, &(ratio, _)) in self.reeds.iter().enumerate() {
            self.counts[r] = ((top / (f0 * ratio)) as usize).clamp(1, PARTIALS);
        }
        self.bank.set_len(self.counts[..self.reeds.len()].iter().sum());
        let mut i = 0;
        for (r, &(ratio, pan)) in self.reeds.iter().enumerate() {
            for k in 1..=self.counts[r] {
                self.bank.set(i, Partial { freq_hz: f0 * ratio * k as f64, gain: 0.0, pan });
                i += 1;
            }
        }
    }

    /// 按当前的风压、抖动、包络，把每个泛音的增益在 `frames` 帧里直线推到目标。
    fn update(&mut self, frames: usize) {
        let p = self.params;
        let pressure = (p.bellows * (0.55 + 0.45 * self.velocity)).clamp(0.0, 1.0);
        let shake = 1.0 + p.shake.clamp(0.0, 1.0) * 0.35 * (core::f64::consts::TAU * self.shake_phase).sin();
        let loud = p.level * (0.25 + 0.75 * pressure) * shake * self.env;
        let f0 = self.f0();
        let mut i = 0;
        for (r, &(ratio, _)) in self.reeds.iter().enumerate() {
            // 16 尺、4 尺的簧比 8 尺的轻一点。
            let reed_gain = if (ratio - 1.0).abs() < 0.2 { 1.0 } else { 0.7 };
            for k in 1..=self.counts[r] {
                let f = f0 * ratio * k as f64;
                let g = loud * reed_gain * Self::amp(k, f, pressure, p.cassotto.clamp(0.0, 1.0));
                self.bank.ramp_to(i, g as f32, frames);
                i += 1;
            }
        }
    }
}

impl Voice<f32> for AccordionVoice {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.bank.set_sample_rate(spec.sample_rate);
    }

    fn note_on(&mut self, note: NoteRef, velocity: f32) {
        self.notes += 1;
        self.key = note.key;
        self.velocity = f64::from(velocity);
        let p = self.params;
        let set = REGISTERS[(p.register.round().max(0.0) as usize).min(REGISTERS.len() - 1)];
        let cents = p.musette.clamp(0.0, 60.0);
        self.reeds.clear();
        for (j, &(ratio, wet)) in set.iter().enumerate() {
            let pan = match set.len() {
                1 => 0.0,
                n => (j as f32 / (n - 1) as f32 - 0.5) * 0.5,
            };
            // 每片簧再有一点自己的走音（±1.5 音分，按音高固定）。
            let own = (unit(u64::from(note.key), j as u64 + 11) - 0.5) * 3.0;
            self.reeds.push((ratio * ((wet * cents + own) / 1200.0).exp2(), pan));
        }
        self.bank.set_len(0);
        self.tune();
        self.bank.set_voicing_seed(u64::from(note.key) * 1_000_003 + self.notes);
        self.bank.set_wind_jitter(WindJitter { cents: 1.2, db: 0.25, rate_hz: 3.0 });
        let rise = (p.attack.clamp(0.002, 0.5) * self.sr) as u32;
        let mut i = 0;
        for r in 0..self.reeds.len() {
            for k in 1..=self.counts[r] {
                let late = 1.0 + 0.8 * (k as f64 / PARTIALS as f64);
                self.bank.set_air(i, PartialAir { depth: 0.012, bandwidth_hz: 30.0 });
                self.bank.start_attack(
                    i,
                    &PartialAttack {
                        delay_frames: 0,
                        rise_frames: (f64::from(rise) * late) as u32,
                        overshoot: 0.06,
                        settle_frames: rise,
                        breath: 0.35,
                        breath_frames: rise * 2,
                        breath_bandwidth_hz: 400.0,
                        pitch_cents: -6.0,
                        pitch_frames: rise * 2,
                    },
                );
                i += 1;
            }
        }
        self.env = 1.0;
        self.held = true;
        self.active = true;
        self.left = 0;
        self.update(1);
    }

    fn note_off(&mut self, _velocity: f32) {
        self.held = false;
    }

    fn choke(&mut self) {
        self.bank.set_len(0);
        self.env = 0.0;
        self.held = false;
        self.active = false;
        self.shake_phase = 0.0;
        self.left = 0;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn render(&mut self, buffer: &mut AudioBuffer<f32>) {
        if !self.active {
            return;
        }
        let n = buffer.num_frames();
        let mut at = 0;
        while at < n {
            if self.left == 0 {
                if !self.held {
                    let step = BLOCK as f64 / (self.params.release.clamp(0.005, 5.0) * self.sr);
                    self.env = (self.env - step).max(0.0);
                }
                self.shake_phase = (self.shake_phase + self.params.shake_rate.clamp(0.5, 12.0) * BLOCK as f64 / self.sr).fract();
                self.update(BLOCK);
                self.left = BLOCK;
                if self.env <= 0.0 && !self.held {
                    // 推到 0 的这一块放完就停。
                    self.active = (0..self.bank.len()).any(|i| self.bank.gain(i) > 1e-6);
                }
            }
            let m = self.left.min(n - at);
            if buffer.num_channels() < 2 {
                return;
            }
            let (l, r) = buffer.channel_pair_mut(0, 1);
            self.bank.render_add(&mut l[at..at + m], &mut r[at..at + m]);
            self.left -= m;
            at += m;
        }
    }

    fn pitch_bend(&mut self, semitones: f64) {
        self.bend = semitones;
        if self.active {
            let f0 = self.f0();
            let mut i = 0;
            for r in 0..self.reeds.len() {
                for k in 1..=self.counts[r] {
                    self.bank.set_frequency(i, f0 * self.reeds[r].0 * k as f64);
                    i += 1;
                }
            }
        }
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        self.tuning.clone_from(tuning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(p: AccordionParams, seconds: f64, release_at: f64) -> Vec<f32> {
        let mut v = AccordionVoice::new(p);
        v.prepare(&ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 2 });
        v.note_on(NoteRef::new(0, 69), 0.8);
        let mut out = Vec::new();
        let mut buf = AudioBuffer::new(2, 512);
        let mut t = 0.0;
        while t < seconds {
            if t >= release_at && v.held {
                v.note_off(0.0);
            }
            buf.clear();
            v.render(&mut buf);
            out.extend_from_slice(buf.channel(0));
            t += 512.0 / 48_000.0;
        }
        assert!(!v.is_active() || release_at > seconds, "the note should end after its release");
        out
    }

    #[test]
    fn sounds_shimmers_and_stops() {
        let a = play(AccordionParams::musette(), 1.5, 1.0);
        assert_eq!(a, play(AccordionParams::musette(), 1.5, 1.0), "the same note renders the same");
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        assert!(rms(&a[4_800..48_000]) > 0.01, "it sounds");
        assert!(rms(&a[60_000..]) < 1e-4, "and stops after the release");
        // musette：8 尺簧之间走音，音量有拍频；干的几乎没有。
        let wobble = |x: &[f32]| {
            let e: Vec<f32> = x[9_600..57_600].chunks(480).map(rms).collect();
            let m = e.iter().sum::<f32>() / e.len() as f32;
            e.iter().map(|v| (v - m).abs()).sum::<f32>() / e.len() as f32 / m
        };
        let wet = play(AccordionParams::musette(), 1.5, 9.0);
        let dry = play(AccordionParams { musette: 0.0, register: 3.0, ..AccordionParams::musette() }, 1.5, 9.0);
        assert!(wobble(&wet) > 3.0 * wobble(&dry), "musette beats: {} vs {}", wobble(&wet), wobble(&dry));
    }
}
