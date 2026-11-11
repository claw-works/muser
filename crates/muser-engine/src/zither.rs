//! 古琴、古筝共用的模态弦模型：散音、按音、泛音分别决定激发、接触损耗和保留的振动模式。
//! 模式的相位连续累积，走手只改变频率与损耗，不重新拨弦，也不移动整数延迟读点。
//! 两件乐器的差别在 [`Profile`]：空弦、按音的做法（古琴按在徽位上弦段变短、有走手摩擦；
//! 古筝在雁柱左边按、只加张力）、琴体辐射。这仍是可调的合成模型，不是对某张琴、某种弦材的精确复原。

use core::f64::consts::{PI, TAU};

use manguebyte::dsp::{Tuning, instruments::{BodyMode, StringParams}};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec, Processor, Synth, Voice};

use crate::{chinese::GuzhengParams, params};

const MODES: usize = 48;
const CONTROL: u64 = 16;

/// 一件弦乐器的结构。
#[derive(Debug, PartialEq)]
pub(crate) struct Profile {
    /// 空弦的音（auto 奏法：这些音弹散音，别的音按出来；其余弦的共鸣也在这些音上）。
    open_keys: &'static [u8],
    /// 按音时发声弦段变短（古琴按在徽位上）；否则弦长不变、只加张力（古筝在雁柱左边按）。
    pressed_shortens: bool,
    /// 按弦的接触损耗（乘 `pressure`）、走手摩擦（损耗与噪声）、按音松手后余音的比例、按音的弦刚度。
    contact: f64,
    friction: f64,
    pressed_release: f64,
    pressed_stiffness: f64,
    /// 散音也跟着弯音、揉弦（古筝左手按在弹着的弦上）；否则只有按音。
    open_bends: bool,
    /// 弦的两个振动方向：另一个方向分走多少（0 没有）、频率最多差几音分、衰减慢几倍。
    twin: f64,
    twin_cents: f64,
    twin_decay: f64,
    /// 琴体辐射的高通：这以下的弦振动放不出来。
    radiation_hz: f64,
    /// 立体声（0 单声道）：低音在左、高音在右分开多少（-1 … 1 的声像），另一方向的弦摆到对面；
    /// 两支话筒听到的琴体共鸣也不一样。
    spread: f64,
    /// 敲下去那一下"脆"：`ping_hz` 以上的分音多响这么多倍，`ping_s` 秒就衰减 60 dB（竹槌碰钢丝弦那一下）。0 没有。
    ping: f64,
    ping_hz: f64,
    ping_s: f64,
    /// 槌子（拨片）碰弦那一下的噪声多响（倍数）：宽带的"嗒"，也经过琴码让其余的弦跟着响。
    knock: f64,
    /// 一敲琴码就晃，同一个琴码上下一个八度里的弦都跟着响：这些弦的振幅（相对这个音的基音，0 没有）。
    bridge: f64,
    voices: usize,
}

pub(crate) static GUQIN: Profile = Profile {
    open_keys: &[36, 38, 41, 43, 45, 48, 50],
    pressed_shortens: true,
    contact: 0.55,
    friction: 1.0,
    pressed_release: 0.35,
    pressed_stiffness: 0.001,
    open_bends: false,
    twin: 0.0,
    twin_cents: 0.0,
    twin_decay: 1.0,
    radiation_hz: 220.0,
    spread: 0.0,
    ping: 0.0,
    ping_hz: 2_000.0,
    ping_s: 0.1,
    knock: 1.0,
    bridge: 0.0,
    voices: 12,
};

/// 21 弦筝，D 调五声音阶 D2–D6。
pub(crate) static GUZHENG: Profile = Profile {
    open_keys: &[38, 40, 42, 45, 47, 50, 52, 54, 57, 59, 62, 64, 66, 69, 71, 74, 76, 78, 81, 83, 86],
    pressed_shortens: false,
    contact: 0.15,
    friction: 0.0,
    pressed_release: 1.0,
    pressed_stiffness: 0.0006,
    open_bends: true,
    twin: 0.0,
    twin_cents: 0.0,
    twin_decay: 1.0,
    radiation_hz: 80.0,
    spread: 0.0,
    ping: 0.0,
    ping_hz: 2_000.0,
    ping_s: 0.1,
    knock: 1.0,
    bridge: 0.0,
    voices: 24,
};

/// 琵琶：四弦 A2 D3 E3 A3，按在品上（弦段变短，品很少吸掉振动），用义甲弹；推拉（弯音）只在按音上。
pub(crate) static PIPA: Profile = Profile {
    open_keys: &[45, 50, 52, 57],
    pressed_shortens: true,
    contact: 0.08,
    friction: 0.0,
    pressed_release: 0.6,
    pressed_stiffness: 0.001,
    open_bends: false,
    twin: 0.35,
    twin_cents: 1.2,
    twin_decay: 1.6,
    radiation_hz: 80.0,
    spread: 0.0,
    ping: 0.0,
    ping_hz: 2_000.0,
    ping_s: 0.1,
    knock: 1.0,
    bridge: 0.0,
    voices: 12,
};

/// 扬琴：每个音一组 2 … 5 根同音的钢丝弦，竹槌敲（不按弦，每个音都是"散音"）；一组弦彼此略微走音，拍出起伏。
/// 小而浅的琴箱放不出低弦的基音（低音区泛音比基音强得多），制音器少，其余弦跟着响。
pub(crate) static YANGQIN: Profile = Profile {
    open_keys: &[
        43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
        78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
    ],
    pressed_shortens: false,
    contact: 0.0,
    friction: 0.0,
    pressed_release: 1.0,
    pressed_stiffness: 0.0006,
    open_bends: false,
    // 这一组和 [`ZitherParams::yangqin`] 的弦是对着 Kontakt Yangqin（Default）的 18 个单音自动拟合的
    // （对数梅尔谱，短窗看起音、长窗看余音），立体声宽度另外按左右相关度定。
    twin: 0.2,
    twin_cents: 19.0,
    twin_decay: 1.8,
    radiation_hz: 430.0,
    spread: 0.6,
    ping: 6.7,
    ping_hz: 880.0,
    ping_s: 0.3,
    knock: 0.74,
    bridge: 0.072,
    voices: 24,
};

/// 从录音提取的乐器（`yinse:`）的结构：录到的音高上下一个八度都是空弦；拍、起音、立体声都在表里，
/// 这里只留琴码带响其余弦的量。
impl Profile {
    pub(crate) fn voices(&self) -> usize {
        self.voices
    }
}

pub(crate) fn table_profile(table: &crate::modal::ModalTable) -> &'static Profile {
    let (lo, hi) = table.range();
    let keys: Vec<u8> = ((lo - 12.0).max(0.0).round() as u8..=(hi + 12.0).min(127.0).round() as u8).collect();
    Box::leak(Box::new(Profile {
        open_keys: Box::leak(keys.into_boxed_slice()),
        pressed_shortens: false,
        contact: 0.0,
        friction: 0.0,
        pressed_release: 1.0,
        pressed_stiffness: 0.0006,
        open_bends: true,
        twin: 0.0,
        twin_cents: 0.0,
        twin_decay: 1.0,
        radiation_hz: 20.0,
        spread: 0.0,
        ping: 0.0,
        ping_hz: 2_000.0,
        // 表里起音那一段最长 1 秒。
        ping_s: 1.0,
        knock: 1.0,
        bridge: table.resonance,
        voices: 32,
    }))
}

/// 用这个模型的乐器各是哪种结构。
pub(crate) fn profile(instrument: &str) -> Option<&'static Profile> {
    match instrument.trim().to_lowercase().as_str() {
        "yinse.pluck.guqin" => Some(&GUQIN),
        "yinse.pluck.guzheng" => Some(&GUZHENG),
        "yinse.pluck.pipa" => Some(&PIPA),
        "yinse.strike.yangqin" => Some(&YANGQIN),
        _ => None,
    }
}
pub(crate) const TECHNIQUES: &[&str] = &["auto", "open", "pressed", "harmonic"];
pub(crate) const BEND_RANGE: f64 = 24.0;
/// 止音：手按住弦后余音到 -60 dB 的时间。
const DAMP_SECONDS: f64 = 0.06;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ZitherParams {
    pub pluck: GuzhengParams,
    pub technique: f64,
    /// 触弦节点分母：2 是弦长的一半，3 是三分之一。
    pub harmonic: f64,
    pub pressure: f64,
    pub slide_time: f64,
    pub slide_noise: f64,
    pub sympathetic: f64,
    pub profile: &'static Profile,
    /// 这个音结束时止音（手按住弦，余音很快收掉）：音符的 `damp`。
    pub damp: bool,
    /// 用户从录音提取的分音表（`yinse:` 乐器）：有它时每个音的分音从表里查，不按公式算。
    pub table: Option<&'static crate::modal::ModalTable>,
}

impl Default for ZitherParams {
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
        // 参考录音中低弦的中段分音常比基音强，不能以低频弦位移作为最终声压。
        // 这是对录音频谱的近似着色，不是已测得的某张琴的传递函数。
        pluck.string.body = [
            BodyMode::new(260.0, 1.4, 0.6),
            BodyMode::new(540.0, 2.4, 5.5),
            BodyMode::new(1_100.0, 2.0, 1.4),
            BodyMode::new(1_850.0, 1.5, 0.5),
        ];
        Self { pluck, technique: 0.0, harmonic: 4.0, pressure: 0.65, slide_time: 0.008, slide_noise: 0.35, sympathetic: 0.18, profile: &GUQIN, damp: false, table: None }
    }
}

impl ZitherParams {
    pub(crate) fn guzheng() -> Self {
        let mut pluck = GuzhengParams::guzheng();
        // 揉弦浅：谱上标了颤音的音再写深的弯音。
        pluck.vibrato = 10.0;
        let s = &mut pluck.string;
        // 义甲拨下去亮而脆、高频很快退掉，剩下以基音为主的余音；1.6 米桐木面板低而宽的共鸣，加上其余弦的共鸣。
        // 用户在《渔舟唱晚》慢板上对比几版后选的（"共鸣4"）。
        s.decay = 9.0;
        s.release = 1.2;
        s.pluck_brightness = 9_000.0;
        s.high_decay = 0.12;
        s.strike = 0.6;
        s.body_mix = 0.6;
        s.tone = 12_000.0;
        s.body = [
            BodyMode::new(75.0, 4.0, 1.6),
            BodyMode::new(150.0, 5.0, 1.6),
            BodyMode::new(260.0, 5.0, 1.0),
            BodyMode::new(430.0, 5.0, 0.8),
        ];
        Self { pluck, sympathetic: 0.6, profile: &GUZHENG, ..Default::default() }
    }

    /// 琵琶：对着五段独奏录音（十面埋伏、阳春白雪、平湖秋月、金蛇狂舞、步步高）的单音：低频每秒衰减约 20 dB、
    /// 高频约 55 … 70 dB；义甲弹在靠近覆手的地方，起音的频谱重心约 2.5 … 3.4 kHz，150 ms 后约 1.3 … 2.4 kHz；
    /// 面板上还有一个 3 kHz 左右的亮点（"铮"）。轮指约每秒 16 下。
    pub(crate) fn pipa() -> Self {
        let mut pluck = GuzhengParams::pipa();
        pluck.tremolo_rate = 16.0;
        let s = &mut pluck.string;
        s.decay = 4.0;
        s.decay_per_octave = 1.3;
        s.high_decay = 0.25;
        s.release = 0.8;
        s.pluck_position = 0.05;
        s.pluck_brightness = 20_000.0;
        s.strike = 1.0;
        s.pluck_noise = 0.5;
        s.tone = 10_000.0;
        // 琴箱的低频共鸣（约 140 Hz 宽、280 Hz）加上其余弦一起响，琴身放得出 80 Hz 以上：用户在几版里选的"琴箱加弦共鸣"，原来的单薄。
        s.body_mix = 0.85;
        s.body = [
            BodyMode::new(140.0, 2.5, 2.0),
            BodyMode::new(280.0, 3.0, 1.8),
            BodyMode::new(560.0, 4.0, 2.1),
            BodyMode::new(2_800.0, 2.0, 3.0),
        ];
        Self { pluck, sympathetic: 0.6, profile: &PIPA, ..Default::default() }
    }

    /// 扬琴：对着 Kontakt 扬琴音源（Default）各音区、各力度的单音：低音区泛音比基音强 10 … 40 dB，高音区以基音为主；
    /// 各分音衰减得差不多快，松开槌子照样响完；击弦软、敲在靠近琴码处（拟合出来的，见 [`YANGQIN`]）。
    pub(crate) fn yangqin() -> Self {
        let mut pluck = GuzhengParams::yangqin();
        let s = &mut pluck.string;
        s.decay = 4.8;
        s.decay_per_octave = 1.2;
        s.high_decay = 0.51;
        // 扬琴没有制音器：松开槌子弦照样响完（Kontakt 的录音也是），短音不收。
        s.release = 10.0;
        s.pluck_position = 0.052;
        s.pluck_brightness = 3_900.0;
        s.velocity_darkening = 1.2;
        s.velocity_range_db = 17.0;
        s.stiffness = 0.15;
        s.strike = 0.4;
        s.pluck_noise = 0.013;
        s.tone = 14_900.0;
        s.body_mix = 0.58;
        s.body = [
            BodyMode::new(600.0, 2.0, 1.8),
            BodyMode::new(1_000.0, 2.5, 1.5),
            BodyMode::new(1_500.0, 2.5, 0.8),
            BodyMode::new(2_400.0, 2.0, 0.4),
        ];
        Self { pluck, sympathetic: 0.12, profile: &YANGQIN, ..Default::default() }
    }

    /// 从录音提取的乐器：声音全在表里，琴体、共鸣、低通都不再加；没有制音器（`release` 可以改短）。
    pub(crate) fn from_table(table: &'static crate::modal::ModalTable, profile: &'static Profile) -> Self {
        let mut pluck = GuzhengParams::yangqin();
        pluck.vibrato = 0.0;
        let s = &mut pluck.string;
        s.body_mix = 0.0;
        s.tone = 20_000.0;
        s.release = 10.0;
        s.pluck_noise = 1.0;
        s.decay = 5.0;
        s.decay_per_octave = 1.2;
        let mut hz: Vec<f64> = table.notes.iter().map(|n| n.noise.hz).collect();
        hz.sort_by(f64::total_cmp);
        s.pluck_brightness = hz[hz.len() / 2].clamp(200.0, 20_000.0);
        Self { pluck, sympathetic: 0.0, profile, table: Some(table), ..Default::default() }
    }

    /// 这个音跟着弯音、揉弦吗。
    fn bends(self, technique: Technique) -> bool {
        technique == Technique::Pressed || (technique == Technique::Open && self.profile.open_bends)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Technique {
    Open,
    Pressed,
    Harmonic,
}

/// 一个音的奏法与泛音节点（见 `muser_core::Note::string_technique`）。
pub(crate) type NoteTechnique = (muser_core::Technique, Option<u8>);

/// 渲染器在起音前投递的逐音标记：奏法、止音。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NoteTag {
    pub technique: Option<NoteTechnique>,
    pub damp: bool,
    /// 花舌、滑音（笛箫用，弦乐器不看）。
    pub flutter: bool,
    pub slide: bool,
}

impl ZitherParams {
    fn with_tag(mut self, tag: NoteTag) -> Self {
        self.damp = tag.damp;
        match tag.technique {
            Some(t) => self.with_note(t),
            None => self,
        }
    }

    fn with_note(mut self, (technique, harmonic): NoteTechnique) -> Self {
        self.technique = match technique {
            muser_core::Technique::Auto => 0.0,
            muser_core::Technique::Open => 1.0,
            muser_core::Technique::Pressed => 2.0,
            muser_core::Technique::Harmonic => 3.0,
        };
        if let Some(h) = harmonic {
            self.harmonic = f64::from(h);
        }
        self
    }

    fn technique(self, key: u8) -> Technique {
        match self.technique.round() as u8 {
            1 => Technique::Open,
            2 => Technique::Pressed,
            3 => Technique::Harmonic,
            _ if self.profile.open_keys.contains(&key) => Technique::Open,
            _ => Technique::Pressed,
        }
    }
}

/// 渲染器据此只给跟着弯音的音（古琴的按音，古筝的散音、按音）加吟猱、揉弦。显式弯音在引擎里同样只作用于这些音。
pub(crate) fn bends(instrument: &str, params: &muser_core::InstrumentParams, key: u8, note: Option<NoteTechnique>) -> bool {
    let Some(profile) = profile(instrument) else {
        return true;
    };
    let technique = match params.get("technique") {
        Some(serde_json::Value::String(s)) => TECHNIQUES.iter().position(|&name| name == s).unwrap_or(0) as f64,
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    };
    let p = ZitherParams { technique, profile, ..Default::default() };
    let p = note.map_or(p, |n| p.with_note(n));
    p.bends(p.technique(key))
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
    /// 弦的另一个振动方向（[`Profile::twin`] > 0 时）：频率差一点、衰减慢一点，和主方向拍出"活"的起伏。
    twins: [Mode; MODES],
    twin_on: bool,
    /// 起音那一下快衰减的高分音（[`Profile::ping`]）。
    pings: [Mode; MODES],
    ping_on: bool,
    /// 跟着响的其余弦（[`Profile::bridge`]），各自的频率；不跟着弯音。
    strings: [Mode; MODES],
    strings_hz: [f64; MODES],
    strings_pan: [[f64; 3]; MODES],
    strings_on: usize,
    /// 每个模式（主方向、另一方向）的左右增益（单声道时都是 1）。
    /// 左边的增益，右边的增益乘上相位差的 cos、sin（两支话筒离琴弦的远近不同）。
    pan: [[f64; 3]; MODES],
    twin_pan: [[f64; 3]; MODES],
    params: ZitherParams,
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
            twins: [Mode::default(); MODES],
            pings: [Mode::default(); MODES],
            ping_on: false,
            strings: [Mode::default(); MODES],
            strings_hz: [0.0; MODES],
            strings_pan: [[1.0, 1.0, 0.0]; MODES],
            strings_on: 0,
            twin_on: false,
            pan: [[1.0, 1.0, 0.0]; MODES],
            twin_pan: [[1.0, 1.0, 0.0]; MODES],
            params: ZitherParams::default(),
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
    fn start(p: ZitherParams, tuning: &Tuning, key: u8, bend: f64, velocity: f32, accent: bool, sr: f64) -> Self {
        let technique = p.technique(key);
        let s = p.pluck.string;
        let key_pitch = tuning.key_pitch(key);
        let pitch = key_pitch + if p.bends(technique) { bend } else { 0.0 };
        let f0 = 440.0 * ((pitch - 69.0) / 12.0).exp2();
        let anchor = if technique == Technique::Harmonic { p.harmonic.round().clamp(2.0, 8.0) as usize } else { 1 };
        let velocity = f64::from(velocity).clamp(0.0, 1.0);
        let strength = 10f64.powf(-s.velocity_range_db * (1.0 - velocity) / 20.0)
            * if accent { 10f64.powf(s.accent_db / 20.0) } else { 1.0 };
        let brightness = s.pluck_brightness * (-s.velocity_darkening * (1.0 - velocity)).exp2()
            * if accent { s.accent_brightness.exp2() } else { 1.0 };
        // 按音中发声弦段变短：同一处右手触弦，相对于该段的位置会向中间移动。
        let keys = p.profile.open_keys;
        let open = keys.iter().rev().find(|&&k| f64::from(k) <= key_pitch - 2.0).copied().unwrap_or(keys[0]);
        let open_hz = tuning.key_frequency(open);
        let shortened = technique == Technique::Pressed && p.profile.pressed_shortens;
        let position = if shortened { s.pluck_position * f0 / open_hz } else { s.pluck_position }.clamp(0.02, 0.45);
        let t60 = s.decay / s.decay_per_octave.powf((f0 / 261.625_565).log2());
        let contact = if technique == Technique::Pressed { 1.0 - p.profile.contact * p.pressure } else { 1.0 };
        let high_decay = if technique == Technique::Harmonic { s.high_decay.max(0.2) } else { s.high_decay };
        let slope = (1.0 / high_decay - 1.0) / ((3_000.0 / f0).max(2.0).powf(1.5) - 1.0);
        let stiffness = s.stiffness * if technique == Technique::Pressed { p.profile.pressed_stiffness } else { 0.0006 };
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
        let pr = p.profile;
        if let Some(table) = p.table {
            // 录音提取的分音：振幅就是录音里的振幅（满幅 1），不再按力度、触弦位置归一。
            let v = table.voice(pitch, velocity * if accent { 10f64.powf(s.accent_db / 20.0) } else { 1.0 });
            tone.modes = [Mode::default(); MODES];
            for (i, q) in v.partials.iter().take(MODES).enumerate() {
                let beat = q.beat.clamp(0.0, 0.45);
                // 表里的振幅是包络的对数平均；主弦 1 份、另一根 w/(1-w) 份合起来的对数平均正好是 1。
                tone.modes[i] = Mode { re: q.amp, ratio: q.ratio, t60: q.t60, ..Default::default() };
                tone.modes[i].tune(f0, sr, None);
                let pan = [q.left, q.right * q.phase.cos(), q.right * q.phase.sin()];
                tone.pan[i] = pan;
                tone.twin_pan[i] = pan;
                if beat > 0.0 {
                    tone.twin_on = true;
                    let c = q.beat_cents * if i % 2 == 0 { 1.0 } else { -1.0 };
                    tone.twins[i] = Mode { re: q.amp * beat / (1.0 - beat), ratio: q.ratio * (c / 1200.0).exp2(), t60: q.t60, ..Default::default() };
                    tone.twins[i].tune(f0, sr, None);
                }
                if q.fast_amp > 0.0 {
                    tone.ping_on = true;
                    tone.pings[i] = Mode { re: q.fast_amp, ratio: q.ratio, t60: q.fast_t60.min(q.t60), ..Default::default() };
                    tone.pings[i].tune(f0, sr, None);
                }
            }
            // 噪声的式子是 level · strength · pluck_noise · 0.035 · knock：倒推出 strength。
            tone.strength = v.noise.amp / (s.level * s.pluck_noise * 0.035 * pr.knock).max(1e-9);
        }
        if pr.spread > 0.0 {
            let at = pr.spread * ((key_pitch - 68.0) / 25.0).clamp(-1.0, 1.0);
            let gains = |x: f64| {
                let a = (x.clamp(-1.0, 1.0) + 1.0) * core::f64::consts::FRAC_PI_4;
                [a.cos() * core::f64::consts::SQRT_2, a.sin() * core::f64::consts::SQRT_2]
            };
            // 两支话筒离琴一两米：每个分音的辐射方向不同，左右听到的比例各不一样（同一组的几根弦也是）。
            // 到两支话筒的路程差（这个音的弦在琴上的位置决定）：每个分音左右的相位差 = 2π f · 时间差。
            let delay = pr.spread * (0.00025 - 0.00012 * (key_pitch - 68.0) / 25.0);
            for n in 0..MODES {
                let h = |salt: u64| ((n as u64 * 2 + salt + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64 * 2.0 - 1.0;
                let phase = TAU * f0 * tone.modes[n].ratio * delay;
                let with_phase = |[l, r]: [f64; 2]| [l, r * phase.cos(), r * phase.sin()];
                tone.pan[n] = with_phase(gains(at + pr.spread * h(0)));
                tone.twin_pan[n] = with_phase(gains(at + pr.spread * h(1)));
            }
        }
        let w = pr.twin.clamp(0.0, 0.9);
        let (cents, decay) = (pr.twin_cents, pr.twin_decay);
        if w > 0.0 {
            tone.twin_on = true;
            for (n, (mode, twin)) in tone.modes.iter_mut().zip(tone.twins.iter_mut()).enumerate() {
                // 每个模式差得不一样（-1 … 1 倍 `cents`，避开 0）：拍的快慢各不相同。
                let h = ((n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64;
                let c = cents * (0.3 + 0.7 * h) * if n % 2 == 0 { 1.0 } else { -1.0 };
                *twin = Mode { re: mode.re * w, ratio: mode.ratio * (c / 1200.0).exp2(), t60: mode.t60 * decay, ..Default::default() };
                twin.tune(f0, sr, None);
                mode.re *= 1.0 - w;
            }
        }
        if pr.bridge > 0.0 {
            let base = tone.modes[0].re.abs();
            for &k in pr.open_keys {
                let d = i32::from(k) - i32::from(key);
                if !(-14..=12).contains(&d) || d == 0 || d.abs() == 12 || tone.strings_on + 2 > MODES {
                    continue;
                }
                let hz = tuning.key_frequency(k);
                let h = ((u64::from(k) + 7).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64;
                let t60 = s.decay / s.decay_per_octave.powf((hz / 261.625_565).log2());
                let pan = tone.pan[(k % 7) as usize];
                for (n, amp) in [(1.0, 1.0), (2.0, 0.4)] {
                    let i = tone.strings_on;
                    // 弦一被晃动就从零起振：振幅放在 re 上（tick 输出 im），没有咔嗒。
                    tone.strings[i] = Mode { re: base * pr.bridge * amp * (0.3 + 0.7 * h), ratio: 1.0, t60: t60 / n, ..Default::default() };
                    tone.strings_hz[i] = hz * n;
                    tone.strings[i].tune(hz * n, sr, None);
                    tone.strings_pan[i] = pan;
                    tone.strings_on += 1;
                }
            }
        }
        if pr.ping > 0.0 {
            tone.ping_on = true;
            for (mode, ping) in tone.modes.iter().zip(tone.pings.iter_mut()) {
                let x = (f0 * mode.ratio / pr.ping_hz).powi(2);
                // 一个 `ping_hz` 到它的三倍之间的带：竹槌一碰，响的是 2 … 5 kHz 的那些分音。
                let weight = pr.ping * x / (1.0 + x) / (1.0 + x * x / 81.0);
                *ping = Mode { re: mode.re * weight, ratio: mode.ratio, t60: pr.ping_s.min(mode.t60), ..Default::default() };
                ping.tune(f0, sr, None);
            }
        }
        tone
    }

    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        2.0 * (self.rng >> 11) as f64 / (1u64 << 53) as f64 - 1.0
    }

    #[cfg(test)]
    fn sample(&mut self, sr: f64, bend: f64, live: ZitherParams) -> f64 {
        let [l, r] = self.sample2(sr, bend, live);
        0.5 * (l + r)
    }

    /// 左右两路（单声道的乐器两路相同）。
    fn sample2(&mut self, sr: f64, bend: f64, live: ZitherParams) -> [f64; 2] {
        if !self.active {
            return [0.0; 2];
        }
        let target = self.key_pitch + if self.params.bends(self.technique) { bend } else { 0.0 };
        let k = 1.0 - (-1.0 / (live.slide_time.max(0.001) * sr)).exp();
        let before = self.pitch;
        self.pitch += (target - self.pitch) * k;
        let speed = (self.pitch - before).abs() * sr;
        let km = 1.0 - (-1.0 / (0.012 * sr)).exp();
        self.motion += (speed.min(48.0) - self.motion) * km;
        if self.at.is_multiple_of(CONTROL) {
            let f0 = 440.0 * ((self.pitch - 69.0) / 12.0).exp2();
            let sliding = if self.technique == Technique::Pressed { (self.pitch - self.last_pitch).abs() } else { 0.0 };
            let friction_loss = (-0.09 * live.pressure * sliding * self.params.profile.friction).exp();
            let release = (!self.held).then_some(if self.params.damp { DAMP_SECONDS } else {
                self.params.pluck.string.release * if self.technique == Technique::Pressed { self.params.profile.pressed_release } else { 1.0 }
            });
            self.energy = 0.0;
            for mode in &mut self.modes {
                mode.re *= friction_loss;
                mode.im *= friction_loss;
                mode.tune(f0, sr, release);
                self.energy += mode.energy();
            }
            if self.twin_on {
                for mode in &mut self.twins {
                    mode.re *= friction_loss;
                    mode.im *= friction_loss;
                    mode.tune(f0, sr, release);
                    self.energy += mode.energy();
                }
            }
            for (mode, &hz) in self.strings[..self.strings_on].iter_mut().zip(&self.strings_hz) {
                mode.tune(hz, sr, release);
                self.energy += mode.energy();
            }
            if self.ping_on {
                self.ping_on = (self.at as f64) < 1.5 * self.params.profile.ping_s * sr;
                for mode in &mut self.pings {
                    mode.tune(f0, sr, release);
                }
            }
            self.last_pitch = self.pitch;
            if self.energy < 1e-11 && self.at > (0.1 * sr) as u64 {
                self.active = false;
                return [0.0; 2];
            }
        }
        let mut out = [0.0; 2];
        for (mode, g) in self.modes.iter_mut().zip(&self.pan) {
            let v = mode.tick();
            out[0] += v * g[0];
            out[1] += v * g[1] + mode.re * mode.fade * g[2];
        }
        if self.twin_on {
            for (mode, g) in self.twins.iter_mut().zip(&self.twin_pan) {
                let v = mode.tick();
                out[0] += v * g[0];
                out[1] += v * g[1] + mode.re * mode.fade * g[2];
            }
        }
        for (mode, g) in self.strings[..self.strings_on].iter_mut().zip(&self.strings_pan) {
            let v = mode.tick();
            out[0] += v * g[0];
            out[1] += v * g[1] + mode.re * mode.fade * g[2];
        }
        if self.ping_on {
            for (mode, g) in self.pings.iter_mut().zip(&self.pan) {
                let v = mode.tick();
                out[0] += v * g[0];
                out[1] += v * g[1] + mode.re * mode.fade * g[2];
            }
        }
        let mut x = 0.0;
        let t = self.at as f64 / sr;
        let n = self.noise();
        let s: StringParams = self.params.pluck.string;
        let kn = 1.0 - (-TAU * s.pluck_brightness.min(0.4 * sr) / sr).exp();
        self.noise_lp += (n - self.noise_lp) * kn;
        x += self.noise_lp * s.level * self.strength * s.pluck_noise * 0.035 * self.params.profile.knock * (-t / 0.006).exp();
        // 走手摩擦随手速和仍在振动的弦能量变化；停止移动后自己收掉，不再拨一次弦。
        let kf = 1.0 - (-TAU * 1_200.0 / sr).exp();
        self.friction_lp += (n - self.friction_lp) * kf;
        if self.technique == Technique::Pressed {
            x += self.friction_lp * self.energy.sqrt() * live.slide_noise * 0.16 * (self.motion / 8.0).sqrt().min(2.0) * self.params.profile.friction;
        }
        let attack = (t / 0.0015).clamp(0.0, 1.0);
        self.at += 1;
        let g = attack * attack * (3.0 - 2.0 * attack);
        out.map(|o| (o + x) * g)
    }
}

struct ZitherVoice {
    params: ZitherParams,
    tuning: Tuning,
    sr: f64,
    bend: f64,
    now: Tone,
    old: Tone,
    fade: f64,
    notes: u64,
}

impl ZitherVoice {
    fn new(params: ZitherParams) -> Self {
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

impl Voice<f32> for ZitherVoice {
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
            let mut x = self.now.sample2(self.sr, self.bend, self.params);
            if self.fade > 0.0 {
                // 被偷走的声部保持原音高，10 ms 内退下去，新的弯音不传给它。
                let old = self.old.sample2(self.sr, self.old.pitch - self.old.key_pitch, self.old.params);
                x = [x[0] + old[0] * self.fade, x[1] + old[1] * self.fade];
                self.fade = (self.fade - 1.0 / (0.01 * self.sr)).max(0.0);
            }
            for c in 0..buffer.num_channels() {
                buffer.channel_mut(c)[i] += x[c.min(1)] as f32;
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
    b1: f64,
    b2: f64,
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
        self.b1 = 0.0;
        self.b2 = -self.b0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
    fn highpass(&mut self, hz: f64, sr: f64) {
        let w = TAU * hz.min(0.45 * sr) / sr;
        let alpha = w.sin() / 2f64.sqrt();
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 + w.cos()) / (2.0 * a0);
        self.b1 = -2.0 * self.b0;
        self.b2 = self.b0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub(crate) struct Zither {
    synth: Synth<ZitherVoice, f32>,
    params: ZitherParams,
    sr: f64,
    tuning: Tuning,
    /// 左右两支话筒各一套（单声道的乐器只用左边那套）。
    body: [[Resonance; 4]; 2],
    radiation: [Resonance; 2],
    sympathetic: [Vec<Resonance>; 2],
    tilt: [f64; 2],
    dc: [f64; 2],
    pending: Vec<(NoteRef, NoteTag)>,
}

impl Zither {
    pub(crate) fn new(params: ZitherParams) -> Self {
        let mut synth = Synth::with_voices(params.profile.voices, move || ZitherVoice::new(params));
        synth.set_pitch_bend_range(BEND_RANGE);
        let sympathetic = vec![Resonance::default(); params.profile.open_keys.len() * 3];
        Self {
            synth, params, sr: 48_000.0, tuning: Tuning::default(), body: [[Resonance::default(); 4]; 2], radiation: [Resonance::default(); 2],
            sympathetic: [sympathetic.clone(), sympathetic], tilt: [0.0; 2], dc: [0.0; 2], pending: Vec::with_capacity(16),
        }
    }
    fn configure(&mut self) {
        for c in 0..2 {
            // 右边的话筒对着琴箱另一处：共鸣的位置差一点。
            let shift = if c == 1 && self.params.profile.spread > 0.0 { 1.07 } else { 1.0 };
            self.radiation[c].highpass(self.params.profile.radiation_hz, self.sr);
            for (r, mode) in self.body[c].iter_mut().zip(self.params.pluck.string.body) {
                r.set(mode.frequency * shift, mode.frequency * shift / mode.q, self.sr);
            }
            for (i, &key) in self.params.profile.open_keys.iter().enumerate() {
                for n in 1..=3 {
                    let hz = self.tuning.key_frequency(key) * n as f64;
                    self.sympathetic[c][i * 3 + n - 1].set(hz, 3.0 * 10f64.ln() / (PI * 3.5), self.sr);
                }
            }
        }
    }
    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        self.body = [[Resonance::default(); 4]; 2];
        self.radiation = [Resonance::default(); 2];
        self.sympathetic.iter_mut().for_each(|v| v.fill(Resonance::default()));
        self.tilt = [0.0; 2];
        self.dc = [0.0; 2];
        self.pending.clear();
        self.synth.prepare(spec);
        self.configure();
    }
    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let p = self.pending.iter().position(|(n, _)| *n == note)
            .map_or(self.params, |i| self.params.with_tag(self.pending.swap_remove(i).1));
        self.synth.voices_mut().for_each(|v| v.params = p);
        self.synth.note_on_accented(note, velocity, accent);
        self.synth.voices_mut().for_each(|v| v.params = self.params);
    }
    pub(crate) fn playing(&mut self, note: NoteRef, technique: NoteTag) {
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
        if params::set_number(params::ZITHER, &mut self.params, name, value) {
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
        let stereo = (self.params.profile.spread > 0.0 || self.params.table.is_some()) && buffer.num_channels() >= 2;
        for i in 0..buffer.num_frames() {
            for c in 0..if stereo { 2 } else { 1 } {
                let source = f64::from(buffer.channel(c)[i]);
                // body_mix=0 可听原始弦模型；默认以辐射后的频谱作为直接声，
                // 而不是在强基音上轻轻叠几个共鸣峰。走手时共鸣位置固定于琴体。
                let radiated = self.radiation[c].tick(source);
                let source = source + (radiated - source) * (s.body_mix / 0.45).clamp(0.0, 1.0);
                let mut x = source;
                for (r, mode) in self.body[c].iter_mut().zip(s.body) {
                    x += r.tick(source) * mode.gain * s.body_mix;
                }
                let mut sympathetic = 0.0;
                if self.params.sympathetic > 0.0 {
                    for (j, r) in self.sympathetic[c].iter_mut().enumerate() {
                        sympathetic += r.tick(source) * [1.0, 0.3, 0.14][j % 3];
                    }
                }
                x += sympathetic * self.params.sympathetic * 0.4;
                self.tilt[c] += (x - self.tilt[c]) * kt;
                self.dc[c] += (self.tilt[c] - self.dc[c]) * kd;
                let out = (self.tilt[c] - self.dc[c]) as f32;
                if stereo {
                    buffer.channel_mut(c)[i] = out;
                } else {
                    for ch in 0..buffer.num_channels() {
                        buffer.channel_mut(ch)[i] = out;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(p: ZitherParams, key: u8, seconds: f64) -> Vec<f64> {
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
    fn yangqin_is_stereo_and_the_other_strings_ring_along() {
        let p = ZitherParams::yangqin();
        let sr = 48_000.0;
        let mut tone = Tone::start(p, &Tuning::default(), 69, 0.0, 0.8, false, sr);
        let x: Vec<[f64; 2]> = (0..48_000).map(|_| tone.sample2(sr, 0.0, p)).collect();
        let (l, r): (Vec<f64>, Vec<f64>) = x[24_000..].iter().map(|v| (v[0], v[1])).unzip();
        let diff: f64 = l.iter().zip(&r).map(|(a, b)| (a - b).powi(2)).sum();
        let sum: f64 = l.iter().zip(&r).map(|(a, b)| (a + b).powi(2)).sum();
        assert!(diff > 0.05 * sum, "left and right differ: {diff} vs {sum}");
        // E5（659 Hz）不是 A4 的泛音，只能是琴码带响的那根弦。
        let e5 = energy_at(&l, 659.255);
        let between = energy_at(&l, 640.0);
        assert!(e5 > 30.0 * between, "E5 string rings along: {e5} vs {between}");
    }

    #[test]
    fn touching_a_node_removes_the_lower_string_modes() {
        for node in 2..=8 {
            let p = ZitherParams { technique: 3.0, harmonic: node as f64, ..Default::default() };
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
        let open = samples(ZitherParams { technique: 1.0, ..Default::default() }, 57, 0.4);
        let pressed = samples(ZitherParams { technique: 2.0, ..Default::default() }, 57, 0.4);
        let harmonic = samples(ZitherParams { technique: 3.0, harmonic: 2.0, ..Default::default() }, 57, 0.4);
        let ratio = |x: &[f64]| energy_at(&x[4_800..], 660.0) / energy_at(&x[4_800..], 220.0);
        assert!(ratio(&harmonic) < ratio(&open) * 0.5, "harmonics should have fewer upper partials");
        assert!((ratio(&pressed) - ratio(&open)).abs() > 0.005, "stopping changes the effective pluck position and damping");
    }

    #[test]
    fn an_octave_slide_preserves_phase_and_does_not_repluck() {
        let p = ZitherParams { technique: 2.0, slide_noise: 0.0, ..Default::default() };
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
            let p = ZitherParams { technique, ..Default::default() };
            let mut tone = Tone::start(p, &Tuning::default(), 57, 12.0, 0.8, false, 48_000.0);
            for _ in 0..1_000 {
                tone.sample(48_000.0, -12.0, p);
            }
            assert_eq!(tone.pitch, 57.0);
        }
    }

    #[test]
    fn a_damped_string_stops_within_a_tenth_of_a_second() {
        let tail = |damp: bool| {
            let p = ZitherParams { damp, ..ZitherParams::guzheng() };
            let mut tone = Tone::start(p, &Tuning::default(), 62, 0.0, 0.8, false, 48_000.0);
            (0..9_600).for_each(|_| { tone.sample(48_000.0, 0.0, p); });
            tone.held = false;
            (0..4_800).for_each(|_| { tone.sample(48_000.0, 0.0, p); });
            (0..480).map(|_| tone.sample(48_000.0, 0.0, p).abs()).fold(0.0, f64::max)
        };
        assert!(tail(true) < tail(false) * 0.01, "{} vs {}", tail(true), tail(false));
    }

    #[test]
    fn guzheng_open_strings_bend_and_auto_presses_off_scale_notes() {
        let p = ZitherParams::guzheng();
        assert_eq!(p.technique(62), Technique::Open);
        assert_eq!(p.technique(63), Technique::Pressed);
        let mut tone = Tone::start(p, &Tuning::default(), 62, 0.0, 0.8, false, 48_000.0);
        for _ in 0..4_800 {
            tone.sample(48_000.0, 2.0, p);
        }
        assert!((tone.pitch - 64.0).abs() < 0.01, "pressing behind the bridge raises an open string");
        assert!(bends("yinse.pluck.guzheng", &Default::default(), 62, None));
        assert!(!bends("yinse.pluck.guqin", &Default::default(), 50, None), "a guqin open string does not");
    }

    #[test]
    fn plucking_at_the_touched_node_cannot_explode_the_normalization() {
        for node in 2..=8 {
            let mut p = ZitherParams { technique: 3.0, harmonic: node as f64, ..Default::default() };
            p.pluck.string.pluck_position = 1.0 / node as f64;
            let x = samples(p, 69, 0.2);
            assert!(x.iter().all(|x| x.is_finite() && x.abs() < 0.25), "node {node}");
        }
    }

    #[test]
    fn simultaneous_note_techniques_are_not_overwritten_by_the_last_tag() {
        let mut q = Zither::new(ZitherParams::default());
        q.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 2, max_block_size: 256 });
        let open = NoteRef::new(0, 48);
        let harmonic = NoteRef::new(0, 62);
        q.playing(open, NoteTag { technique: Some((muser_core::Technique::Open, None)), damp: false, flutter: false, slide: false });
        q.playing(harmonic, NoteTag { technique: Some((muser_core::Technique::Harmonic, Some(2))), damp: false, flutter: false, slide: false });
        q.note_on(open, 0.8, false);
        q.note_on(harmonic, 0.8, false);
        let tones: Vec<_> = q.synth.voices_mut().filter(|v| v.now.active).map(|v| (v.now.key_pitch, v.now.technique)).collect();
        assert!(tones.contains(&(48.0, Technique::Open)));
        assert!(tones.contains(&(62.0, Technique::Harmonic)));
        assert!(q.pending.is_empty());
    }

    #[test]
    fn body_radiation_changes_low_string_spectrum_without_moving_its_pitch() {
        let render = |body_mix| {
            let mut p = ZitherParams { technique: 1.0, ..Default::default() };
            p.pluck.string.body_mix = body_mix;
            let mut q = Zither::new(p);
            q.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 128 });
            q.note_on(NoteRef::new(0, 36), 0.8, false);
            let mut buf = AudioBuffer::new(1, 128);
            let mut x = Vec::new();
            for _ in 0..400 {
                buf.clear();
                q.process(&mut buf);
                x.extend(buf.channel(0).iter().map(|&v| f64::from(v)));
            }
            x
        };
        let dry = render(0.0);
        let body = render(0.45);
        let f0 = Tuning::default().key_frequency(36);
        let upper = |x: &[f64]| (4..=8).map(|n| energy_at(&x[4_800..], f0 * f64::from(n))).sum::<f64>();
        let ratio = |x: &[f64]| upper(x) / energy_at(&x[4_800..], f0);
        assert!(ratio(&body) > ratio(&dry) * 20.0, "radiation must not leave the bass fundamental dominant");
        assert!(energy_at(&body[4_800..], f0) > energy_at(&body[4_800..], f0 * 1.05) * 100.0);
    }

    #[test]
    fn full_instrument_stays_finite_across_sample_rates_and_voice_stealing() {
        for sr in [22_050.0, 48_000.0, 96_000.0] {
            let mut q = Zither::new(ZitherParams { technique: 2.0, ..Default::default() });
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
