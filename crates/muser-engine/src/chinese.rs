//! 中国民族乐器（muser 自己的，yinse）：古筝 `yinse.pluck.guzheng`、琵琶 `yinse.pluck.pipa`、古琴 `yinse.pluck.guqin`、
//! 扬琴 `yinse.strike.yangqin`、二胡 `yinse.bow.erhu`、梆笛 `yinse.blow.bangdi`（别名 `yinse.blow.dizi`）、曲笛 `yinse.blow.qudi`、箫 `yinse.blow.xiao`。
//!
//! 都在 manguebyte 的物理模型上配参数：
//! - **古筝**：和古琴共用的模态弦模型（见 [`crate::zither`]）：21 根弦定 D 调五声，桐木面板与其余弦的共鸣；
//!   五声以外的音从下面一根弦按出来，每个音都跟着弯音（按音、滑音）和左手揉弦。
//! - **二胡**：拉弦模型。两根弦（D4、A4），蟒皮琴筒：没有木头箱体的密集共鸣，几个蟒皮与琴筒的共鸣让它带鼻音、
//!   发亮；弓毛夹在两弦之间，擦弦声多。揉弦宽而慢起，连奏（clip 的 `legato`）的音之间手指滑过去（单音，滑音）。
//! - **琵琶**：拨弦模型。梨形木头琴身，义甲弹得脆、颗粒感强，余音比古筝短（品位高，手指按弦就止住一些）。轮指是音符的 `tremolo`。
//! - **古琴**：拨弦模型。起音有指甲触弦的棱角，高频先退、低频留下柔和余韵；吟猱（揉弦）慢而深。
//! - **扬琴**：拨弦模型当作击弦（竹槌敲）：亮、金属味；每个音是两根同音的弦（略微走音，声音"活"），见 [`Courses`]。轮音是 `tremolo`。
//! - **笛子**：吹管（长笛）模型 + 笛膜：膜在高频上"嗡"出一片沙沙的泛音，越响越明显（[`Dizi`]）。单音，连着的音滑过去。
//! - **箫**：吹管模型，竖吹：低、暗、气声多，起音慢。单音。
//!
//! 揉弦（古筝、琵琶、古琴）与轮音（古筝、琵琶、扬琴）都由渲染器写成弯音 / 重复的音（见 [`vibrato_for`]、[`tremolo_rate`]）。

use manguebyte::dsp::instruments::{BodyMode, BowedParams, BowedString, PluckedString, StringParams, WindParams, WindVoice};
use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, Filter, FilterShape, NoteRef, ProcessSpec, Processor, Synth};

use crate::params;

/// 古筝：弦 + 揉弦。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuzhengParams {
    pub string: StringParams,
    /// 揉弦的深浅（音分，上下各这么多）、快慢（Hz）、音符开头多久之后开始（秒）。
    pub vibrato: f64,
    pub vibrato_rate: f64,
    pub vibrato_delay: f64,
    /// 轮音（音符的 `tremolo`）每秒弹几下。
    pub tremolo_rate: f64,
    /// 琴身放不出的低频：这个频率以下滤掉（0 不滤）。
    pub high_pass: f64,
}

/// 拨弦的默认值（古筝的弦，各乐器在上面改）。
const fn strings(body: [BodyMode; 4]) -> StringParams {
    StringParams {
        decay: 8.0,
        decay_per_octave: 1.4,
        high_decay: 0.4,
        release: 1.6,
        pluck_position: 0.08,
        pluck_brightness: 9_000.0,
        velocity_darkening: 1.8,
        velocity_range_db: 22.0,
        stiffness: 0.18,
        strike: 0.5,
        pluck_noise: 0.3,
        body,
        body_mix: 0.5,
        pickup_position: 0.0,
        tone: 12_000.0,
        tone_q: 0.6,
        buzz: 0.0,
        buzz_time: 0.05,
        accent_db: 3.0,
        accent_brightness: 0.6,
        accent_buzz: 0.0,
        level: 0.2,
    }
}

impl GuzhengParams {
    pub fn guzheng() -> Self {
        Self {
            string: StringParams {
                decay: 12.0,
                decay_per_octave: 1.4,
                high_decay: 0.12,
                release: 2.5,
                pluck_position: 0.12,
                pluck_brightness: 9_000.0,
                velocity_darkening: 1.8,
                velocity_range_db: 22.0,
                stiffness: 0.18,
                strike: 0.6,
                pluck_noise: 0.3,
                // 1.6 米长的桐木面板：低而宽的几个共鸣。"脆"：义甲拨下去那一下很亮、带拨片声，高频很快就没了（high_decay 小），
                // 剩下干净、以基音为主的长余音（对着一段古筝录音听过、量过，用户选的）。
                body: [BodyMode::new(75.0, 4.0, 1.0), BodyMode::new(150.0, 5.0, 1.0), BodyMode::new(260.0, 5.0, 0.6), BodyMode::new(430.0, 5.0, 0.45)],
                body_mix: 0.5,
                pickup_position: 0.0,
                tone: 12_000.0,
                tone_q: 0.6,
                buzz: 0.0,
                buzz_time: 0.05,
                accent_db: 3.0,
                accent_brightness: 0.6,
                accent_buzz: 0.0,
                level: 0.2,
            },
            vibrato: 25.0,
            vibrato_rate: 5.5,
            vibrato_delay: 0.3,
            tremolo_rate: 10.0,
            high_pass: 0.0,
        }
    }

    /// 琵琶：梨形木头琴身（中频的几个共鸣），义甲弹得脆；余音短一些。
    pub fn pipa() -> Self {
        let mut s = strings([BodyMode::new(180.0, 6.0, 0.12), BodyMode::new(330.0, 8.0, 0.35), BodyMode::new(560.0, 8.0, 0.5), BodyMode::new(900.0, 6.0, 0.4)]);
        s.decay = 1.5;
        s.decay_per_octave = 1.5;
        s.high_decay = 0.35;
        s.release = 0.12;
        s.pluck_position = 0.14;
        s.pluck_brightness = 4_500.0;
        s.stiffness = 0.08;
        s.strike = 0.6;
        s.pluck_noise = 0.25;
        s.body_mix = 0.6;
        s.tone = 9_000.0;
        s.buzz = 0.04;
        Self { string: s, vibrato: 15.0, vibrato_rate: 5.0, vibrato_delay: 0.35, tremolo_rate: 12.0, high_pass: 460.0 }
    }

    /// 古琴：清楚的拨弦起音接暗的余韵，不能只把整条弦低通成柔软的吉他音。
    /// 参考独奏录音中的起音与衰减趋势；琴体共鸣是模型近似，不是对某张琴的实测复原。
    pub fn guqin() -> Self {
        let mut s = strings([BodyMode::new(70.0, 4.0, 0.45), BodyMode::new(140.0, 5.0, 0.55), BodyMode::new(240.0, 5.0, 0.35), BodyMode::new(400.0, 4.0, 0.25)]);
        s.decay = 6.5;
        s.decay_per_octave = 1.15;
        s.high_decay = 0.045;
        // 松键不是立即按住弦：允许短音后仍留下余韵。
        s.release = 3.0;
        s.pluck_position = 0.09;
        s.pluck_brightness = 4_800.0;
        s.velocity_darkening = 2.0;
        s.stiffness = 0.03;
        s.strike = 0.55;
        s.pluck_noise = 0.22;
        s.body_mix = 0.45;
        s.tone = 7_000.0;
        Self { string: s, vibrato: 18.0, vibrato_rate: 3.5, vibrato_delay: 0.5, tremolo_rate: 8.0, high_pass: 0.0 }
    }

    /// 扬琴：竹槌敲钢丝弦（击弦：`strike` 1），亮、金属味，梯形琴箱；有制音（松开后不太长）。
    pub fn yangqin() -> Self {
        let mut s = strings([BodyMode::new(120.0, 6.0, 0.5), BodyMode::new(240.0, 8.0, 0.5), BodyMode::new(420.0, 8.0, 0.45), BodyMode::new(700.0, 6.0, 0.35)]);
        s.decay = 6.0;
        s.decay_per_octave = 1.5;
        s.high_decay = 0.35;
        s.release = 0.6;
        s.pluck_position = 0.12;
        s.pluck_brightness = 10_000.0;
        s.velocity_darkening = 2.2;
        s.stiffness = 0.15;
        s.strike = 1.0;
        s.pluck_noise = 0.15;
        s.body_mix = 0.45;
        s.tone = 11_000.0;
        Self { string: s, vibrato: 0.0, vibrato_rate: 5.0, vibrato_delay: 0.3, tremolo_rate: 14.0, high_pass: 0.0 }
    }
}

/// 按乐器名取拨弦类的预设（揉弦、轮音的默认值从这里来）。
fn pluck_preset(instrument: &str) -> Option<GuzhengParams> {
    Some(match instrument.trim().to_lowercase().as_str() {
        "yinse.pluck.pipa" => GuzhengParams::pipa(),
        "yinse.pluck.guzheng" => crate::zither::ZitherParams::guzheng().pluck,
        "yinse.pluck.guqin" => GuzhengParams::guqin(),
        "yinse.strike.yangqin" => GuzhengParams::yangqin(),
        _ => return None,
    })
}

/// 轮音每秒几下（音符写了 `tremolo` 时）：古筝、琵琶、古琴、扬琴；其它乐器为 None（不轮）。
pub(crate) fn tremolo_rate(instrument: &str, params: &muser_core::InstrumentParams) -> Option<f64> {
    let d = pluck_preset(instrument)?;
    Some(params.get("tremolo_rate").and_then(serde_json::Value::as_f64).unwrap_or(d.tremolo_rate).clamp(2.0, 30.0))
}

/// 把写了 `tremolo` 的音拆成一串重复的音（`bps` 每秒几拍）：每下长度约一下的九成，轮指的几个手指力度略有不同。
pub(crate) fn expand_tremolo(notes: &[muser_core::Note], rate: f64, bps: f64) -> Vec<muser_core::Note> {
    let mut out = Vec::with_capacity(notes.len());
    for n in notes {
        if !n.tremolo {
            out.push(n.clone());
            continue;
        }
        let step = bps / rate;
        let count = ((n.length.0 / step).floor() as usize).max(1);
        for i in 0..count {
            let mut m = n.clone();
            m.tremolo = false;
            m.start = muser_core::Beats(n.start.0 + i as f64 * step);
            m.length = muser_core::Beats(step * 0.9);
            // 轮指五个手指（扬琴两只槌）：力度按位置小小地起伏。
            m.velocity = (n.velocity * [1.0, 0.9, 0.95, 0.88, 0.93][i % 5]).clamp(0.0, 1.0);
            if i > 0 {
                m.bend.clear();
            }
            out.push(m);
        }
    }
    out
}

/// 颤音每秒几个来回（录音里笛子的指颤 7–10 个，取 8）。
const TRILL_RATE: f64 = 8.0;

/// 颤音（`trill`）拆成主音与上方音的交替，虚颤音（`half_trill`）写成小幅快速的弯音。`bps`：每秒几拍。
pub(crate) fn expand_trills(notes: &[muser_core::Note], bps: f64) -> Vec<muser_core::Note> {
    let mut out = Vec::with_capacity(notes.len());
    for n in notes {
        let mut n = n.clone();
        if n.half_trill && n.bend.is_empty() {
            // 手指在孔上半开半合：只往下（盖住一点就低一点），0 到 -25 音分，每秒 6 下；
            // 两边对称地摆就成了弦乐的揉弦。每个来回 8 个点。
            let cycle = bps / 6.0;
            let points = ((n.length.0 / cycle) * 8.0).ceil() as usize;
            n.bend = (0..=points).map(|i| {
                let beat = i as f64 * cycle / 8.0;
                (beat, -0.125 * (1.0 - (core::f64::consts::TAU * beat / cycle).cos()))
            }).collect();
        }
        if n.trill == 0 {
            out.push(n);
            continue;
        }
        let step = bps / (2.0 * TRILL_RATE);
        let count = ((n.length.0 / step).round() as usize).max(2);
        let step = n.length.0 / count as f64;
        for i in 0..count {
            let mut m = n.clone();
            m.trill = 0;
            m.start = muser_core::Beats(n.start.0 + i as f64 * step);
            m.length = muser_core::Beats(step * 0.95);
            if i % 2 == 1 {
                m.pitch = n.pitch.saturating_add(n.trill).min(127);
                m.velocity = (n.velocity * 0.9).clamp(0.0, 1.0);
            }
            if i > 0 {
                m.bend.clear();
                m.flutter = false;
            }
            out.push(m);
        }
    }
    out
}

/// 扬琴的一个音是两根同音的弦（一"组"），各自略微走音：两组拨弦一起弹，声音"活"、有一点拍频。
pub(crate) struct Courses {
    a: Synth<PluckedString, f32>,
    b: Synth<PluckedString, f32>,
    params: GuzhengParams,
    scratch: AudioBuffer<f32>,
}

impl Courses {
    pub(crate) fn new(params: GuzhengParams, voices: usize) -> Self {
        let p = params.string;
        let mut c = Self {
            a: Synth::with_voices(voices, move || PluckedString::new(p)),
            b: Synth::with_voices(voices, move || PluckedString::new(p)),
            params,
            scratch: AudioBuffer::new(2, 0),
        };
        c.set_tuning(&manguebyte::dsp::Tuning::default());
        c
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.a, spec);
        Processor::prepare(&mut self.b, spec);
        self.scratch = AudioBuffer::new(spec.num_channels.max(2), spec.max_block_size);
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.a.note_on_accented(note, velocity, accent);
        self.b.note_on_accented(note, velocity * 0.92, accent);
    }

    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.a.note_off(note, velocity);
        self.b.note_off(note, velocity);
    }

    pub(crate) fn midi(&mut self, message: MidiMessage) {
        let e = manguebyte::prelude::Event::from_midi(0, message);
        self.a.handle_event(&e);
        self.b.handle_event(&e);
    }

    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.a, buffer);
        let n = buffer.num_frames();
        if self.scratch.num_frames() < n || self.scratch.num_channels() < buffer.num_channels() {
            self.scratch = AudioBuffer::new(buffer.num_channels(), n);
        }
        self.scratch.set_num_frames(n);
        self.scratch.clear();
        Processor::process(&mut self.b, &mut self.scratch);
        for c in 0..buffer.num_channels() {
            for (y, x) in buffer.channel_mut(c).iter_mut().zip(self.scratch.channel(c)) {
                *y = 0.6 * (*y + *x);
            }
        }
    }

    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::GUZHENG, &mut self.params, name, value) {
            let p = self.params.string;
            self.a.voices_mut().for_each(|v| v.set_params(p));
            self.b.voices_mut().for_each(|v| v.set_params(p));
        }
    }

    pub(crate) fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        // 两组弦：一组高 2 音分，一组低 2 音分。
        let shift = |cents: f64| {
            let pitches: [f64; 128] = std::array::from_fn(|k| tuning.key_pitch(k as u8) + cents / 100.0);
            manguebyte::dsp::Tuning::from_pitches(&pitches).unwrap_or_else(|_| tuning.clone())
        };
        self.a.set_tuning(&shift(2.0));
        self.b.set_tuning(&shift(-2.0));
    }
}

/// 笛子：吹管 + 笛膜的嗡声。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiziParams {
    pub wind: WindParams,
    /// 笛膜：0 没有（像长笛），0.6 贴得正好、明显的"沙"，1 很响的嗡。
    pub membrane: f64,
}

impl DiziParams {
    pub fn dizi() -> Self {
        let mut w = WindParams::flute();
        w.noise = 0.45;
        w.attack = 0.03;
        w.vibrato_rate = 5.5;
        w.vibrato_depth = 12.0;
        w.vibrato_delay = 0.2;
        w.articulation = 0.8;
        w.portamento = true;
        w.legato_time = 0.06;
        // 七段竹笛独奏录音（喜相逢、喜洋洋、北京小曲等）的长音：二次谐波比基音低约 19 dB、三次约 16 dB、四次约 31 dB；
        // 长笛的默认（吹口偏 1.0）偶次谐波太强（二次只低 3 dB），像长笛不像笛子。0.2 时二、三、四次约 -17 / -12 / -25 dB。
        w.jet_offset = 0.2;
        // 剁音（北派）：重音的舌头打得更狠、更响，收得也快（见 [`Dizi::note_on`]）。
        w.accent_db = 5.0;
        w.accent_push = 1.0;
        Self { wind: w, membrane: 0.45 }
    }

    /// 曲笛（南派）：管更粗更长、音低（D 调曲笛筒音 A4），声音圆润、气声多，笛膜贴得松。
    /// 对着两段较低、较柔的录音（燕归来、月儿弯弯照九州）：二、三、四次谐波约 -11 / -10 / -21 dB，谐波之间的气声约 -52 … -57 dB；
    /// 长音的揉音很浅（约 ±7 … 14 音分，每秒 4.4 … 5.8 下）——揉深了、气声少了，长音就像拉弦。
    pub fn qudi() -> Self {
        let mut d = Self::dizi();
        let w = &mut d.wind;
        w.jet_offset = 0.4;
        w.noise = 0.75;
        // 气息不那么稳：吹管的"活"，不是弦乐那种匀的长音。
        w.fluctuation = 0.05;
        w.attack = 0.04;
        w.vibrato_rate = 4.8;
        w.vibrato_depth = 9.0;
        w.vibrato_delay = 0.35;
        w.articulation = 0.65;
        w.legato_time = 0.08;
        w.accent_db = 3.0;
        w.accent_push = 0.6;
        d.membrane = 0.25;
        d
    }
}

/// 箫（洞箫）：竖吹，低、暗，声音几乎只有基音。对着三段洞箫独奏录音（箫雨、梦人间、鼓山夜雨）的长音：
/// 二、三、四次谐波约 -16 / -26 / -37 dB（比笛子纯得多），谐波之间的气声约 -62 … -67 dB（录音里很干净，不是"气声多"），
/// 揉音浅（约 ±7 … 9 音分，每秒 4.7 … 5.3 下）；音大多连着吹，起音约 0.1 秒。
pub fn xiao() -> WindParams {
    let mut w = WindParams::flute();
    w.jet_offset = 0.4;
    w.opening = 0.22;
    w.noise = 0.15;
    w.breath_soft = 0.35;
    w.bell = 2_000.0;
    w.attack = 0.05;
    w.release = 0.15;
    w.vibrato_rate = 5.0;
    w.vibrato_depth = 8.0;
    w.vibrato_delay = 0.4;
    w.portamento = true;
    w.legato_time = 0.1;
    w.tone = 7_000.0;
    w
}

/// 笛子：一支单音的吹管，后面接笛膜（越响越"沙"的高频泛音）。
pub(crate) struct Dizi {
    synth: Synth<WindVoice, f32>,
    params: DiziParams,
    /// 笛膜的两个共鸣（约 3 kHz、6 kHz），左右声道各一份。
    membrane: [[Filter<f32>; 2]; 2],
    wet: AudioBuffer<f32>,
    wet2: AudioBuffer<f32>,
    /// 每个声道的峰值包络（膜按波形的形状削，削出的声音跟着音量走）。
    env: [f32; 2],
    /// 下一个音要不要花舌，正在吹的音花不花舌；花舌的相位与噪声种子。
    flutter_next: bool,
    flutter: bool,
    flutter_phase: f64,
    flutter_lp: f32,
    sr: f64,
    rng: u32,
}

/// 花舌的气声有多少（相对这一刻的音量）。
const FLUTTER_HISS: f32 = 0.06;

impl Dizi {
    pub(crate) fn new(params: DiziParams) -> Self {
        let w = params.wind;
        let mut synth = Synth::with_voices(1, move || WindVoice::new(w));
        synth.set_legato(true);
        let peak = || [Filter::new(FilterShape::Bandpass, 3_200.0, 2.0), Filter::new(FilterShape::Bandpass, 6_000.0, 2.5)];
        Self { synth, params, membrane: [peak(), peak()], wet: AudioBuffer::new(1, 0), wet2: AudioBuffer::new(1, 0), env: [0.0; 2],
            flutter_next: false, flutter: false, flutter_phase: 0.0, flutter_lp: 0.0, sr: 48_000.0, rng: 0x1234_5678 }
    }

    pub(crate) fn flutter_next(&mut self, on: bool) {
        self.flutter_next = on;
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.synth, spec);
        self.sr = spec.sample_rate;
        let mono = ProcessSpec { num_channels: 1, ..*spec };
        self.membrane.iter_mut().flatten().for_each(|f| f.prepare(&mono));
        self.wet = AudioBuffer::new(1, spec.max_block_size);
        self.wet2 = AudioBuffer::new(1, spec.max_block_size);
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.flutter = std::mem::take(&mut self.flutter_next);
        // 剁音：重音的音松开时一下就收住（舌头截断气流），不按平常的余音。
        let mut w = self.params.wind;
        if accent {
            w.release = w.release.min(0.02);
        }
        self.synth.voices_mut().for_each(|v| v.set_params(w));
        self.synth.note_on_accented(note, velocity, accent);
    }

    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
    }

    pub(crate) fn midi(&mut self, message: MidiMessage) {
        self.synth.handle_event(&manguebyte::prelude::Event::from_midi(0, message));
    }

    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.synth, buffer);
        if self.flutter {
            self.flutter_tongue(buffer);
        }
        let m = self.params.membrane.clamp(0.0, 1.0) as f32;
        if m == 0.0 {
            return;
        }
        let n = buffer.num_frames();
        if self.wet.num_frames() < n || self.wet2.num_frames() < n {
            self.wet = AudioBuffer::new(1, n);
            self.wet2 = AudioBuffer::new(1, n);
        }
        self.wet.set_num_frames(n);
        self.wet2.set_num_frames(n);
        for c in 0..buffer.num_channels().min(2) {
            // 膜被气柱推着：大的振动被它不对称地"削"一下，削出来的高频泛音经膜的共鸣放出来；吹得越响削得越多。
            let env = &mut self.env[c];
            for (w, x) in self.wet.channel_mut(0).iter_mut().zip(buffer.channel(c)) {
                *env = x.abs().max(*env * 0.9995);
                let d = 1.5 * *x / (*env + 1e-6);
                *w = (d + 0.5 * d * d).tanh() * *env;
            }
            // 膜的两个共鸣并联（各放一段）。
            self.wet2.channel_mut(0).copy_from_slice(&self.wet.channel(0)[..n]);
            let [lo, hi] = &mut self.membrane[c];
            lo.process(&mut self.wet);
            hi.process(&mut self.wet2);
            for ((y, a), b) in buffer.channel_mut(c).iter_mut().zip(self.wet.channel(0)).zip(self.wet2.channel(0)) {
                *y += m * 1.2 * (*a + 0.7 * *b);
            }
        }
    }

    /// 花舌：舌尖打嘟噜，每秒约 40 下把气流截断又放开。录音里长音的包络在 38–42 Hz 上起伏（起伏的标准差约 0.13–0.25），
    /// 每次放开时多一股气声。舌头的快慢不完全匀，频率随机晃一点。
    fn flutter_tongue(&mut self, buffer: &mut AudioBuffer<f32>) {
        let n = buffer.num_frames();
        for i in 0..n {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let rate = 40.0 * (1.0 + 0.06 * f64::from(noise));
            self.flutter_phase = (self.flutter_phase + rate / self.sr).fract();
            // 平滑的正弦起伏（只在谐波两边各出一条 40 Hz 的边带，不像整流波那样一路溅到谐波之间），
            // 气声只加在 3 kHz 以上、跟着舌头放开时出：录音里花舌长音谐波之间的噪声低于 3 kHz 约 -61 dB，3–8 kHz 约 -55 dB。
            let open = 0.5 + 0.5 * (core::f64::consts::TAU * self.flutter_phase).sin() as f32;
            let gain = 0.78 + 0.22 * (2.0 * open - 1.0);
            let hiss = noise - self.flutter_lp;
            self.flutter_lp += 0.33 * (noise - self.flutter_lp);
            for c in 0..buffer.num_channels() {
                let x = buffer.channel(c)[i];
                buffer.channel_mut(c)[i] = x * gain + FLUTTER_HISS * x.abs() * hiss * open;
            }
        }
    }

    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::DIZI, &mut self.params, name, value) {
            let w = self.params.wind;
            self.synth.voices_mut().for_each(|v| v.set_params(w));
        }
    }

    pub(crate) fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.synth.set_tuning(tuning);
    }
}

/// 二胡：D4、A4 两根弦，蟒皮琴筒，宽揉弦，连奏滑音。
pub fn erhu() -> BowedParams {
    let d4 = 293.66;
    BowedParams {
        // 比 D4 低的音也在内弦上（二胡最低就是 D4）。
        strings: [d4, d4, d4, 440.0],
        bow_position: 0.1,
        pressure: 0.8,
        attack: 0.05,
        // 二胡的"毛"：马尾夹在两弦之间、松香多，整个音里一直有擦弦的颗粒（不光是开头），弓速也不那么稳。
        bow_noise: 0.7,
        decay: 0.8,
        decay_per_octave: 1.3,
        high_decay: 0.2,
        release: 0.2,
        vibrato_rate: 6.5,
        vibrato_depth: 40.0,
        vibrato_delay: 0.1,
        vibrato_rise: 0.2,
        legato_time: 0.12,
        portamento: true,
        drift: 4.0,
        fluctuation: 0.12,
        swell: 0.12,
        scratch: 0.6,
        // 琴筒（对着《赛马》录音量过泛音）：低于约 400 Hz 放得很少（D4 的基音比第二泛音低 25 dB 以上），
        // 600 Hz 和 1.2–1.8 kHz 两片强共鸣，中间 900 Hz 左右是谷，2 kHz 以上很快就没了——二胡的鼻音、"哭腔"在这里。
        body: [
            BodyMode::new(330.0, 3.0, 0.0),
            BodyMode::new(600.0, 5.0, 1.6),
            BodyMode::new(1_250.0, 5.0, 0.7),
            BodyMode::new(1_800.0, 6.0, 2.2),
            BodyMode::new(2_200.0, 5.0, 0.0),
            BodyMode::new(3_000.0, 3.0, 0.08),
            BodyMode::new(4_800.0, 3.0, 0.6),
            BodyMode::new(6_000.0, 2.0, 0.01),
        ],
        wood: 0.05,
        body_mix: 1.0,
        tone: 8_000.0,
        ..BowedParams::violin()
    }
}

/// 二胡：单音的拉弦，连着的音滑过去；后面接琴筒的"出声口"：蟒皮放不出 400 Hz 以下，2.3 kHz 以上也很快没了
/// （拉弦模型的共鸣是一组带通，坡不够陡，这两道滤波把坡补上）。
pub(crate) struct Erhu {
    synth: Synth<BowedString, f32>,
    params: BowedParams,
    eq: Vec<Filter<f32>>,
}

impl Erhu {
    pub(crate) fn new(params: BowedParams) -> Self {
        let mut synth = Synth::with_voices(1, move || BowedString::new(params));
        synth.set_legato(true);
        Self {
            synth,
            params,
            // 照两段录音（《赛马》、F4 长音）量出来的琴筒出声：低于 400 Hz 少，1 kHz 附近是谷，1.4 kHz 一个峰，
            // 2–2.8 kHz 几乎没有，3.1 kHz 又有一小片亮，4–8 kHz 还有一层泛音（《赛马》、F4、B5 三段录音）。
            eq: vec![
                Filter::new(FilterShape::Highpass, 420.0, 0.7),
                Filter::new(FilterShape::Highpass, 420.0, 0.7),
                Filter::new(FilterShape::Peak { gain_db: -10.0 }, 980.0, 5.0),
                Filter::new(FilterShape::Peak { gain_db: 6.0 }, 1_300.0, 3.0),
                // 2 kHz 上下是谷，4–8 kHz 又有一层亮（高音更明显，见 B5 长音录音）。
                Filter::new(FilterShape::Lowpass, 3_000.0, 0.7),
                Filter::new(FilterShape::Peak { gain_db: 4.0 }, 5_000.0, 1.2),
                Filter::new(FilterShape::Peak { gain_db: -18.0 }, 2_450.0, 3.0),
                Filter::new(FilterShape::Peak { gain_db: 16.0 }, 3_100.0, 5.0),
            ],
        }
    }
    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.synth, spec);
        self.eq.iter_mut().for_each(|f| f.prepare(spec));
    }
    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.synth.note_on_accented(note, velocity, accent);
    }
    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
    }
    pub(crate) fn midi(&mut self, message: MidiMessage) {
        self.synth.handle_event(&manguebyte::prelude::Event::from_midi(0, message));
    }
    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.synth, buffer);
        self.eq.iter_mut().for_each(|f| f.process(buffer));
    }
    pub(crate) fn set_param(&mut self, name: &str, value: f64) {
        if params::set_number(params::BOWED, &mut self.params, name, value) {
            let p = self.params;
            self.synth.voices_mut().for_each(|v| v.set_params(p));
        }
    }
    pub(crate) fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.synth.set_tuning(tuning);
    }
}

/// 这条音轨要由渲染器加的揉弦：(深浅音分, 快慢 Hz, 开始前的秒数)；不是古筝或深浅为 0 时为 None。
pub(crate) fn vibrato_for(instrument: &str, params: &muser_core::InstrumentParams) -> Option<(f64, f64, f64)> {
    let d = pluck_preset(instrument)?;
    let get = |k: &str, v: f64| params.get(k).and_then(serde_json::Value::as_f64).unwrap_or(v);
    let depth = get("vibrato", d.vibrato).clamp(0.0, 100.0);
    (depth > 0.0).then(|| (depth, get("vibrato_rate", d.vibrato_rate).clamp(0.5, 12.0), get("vibrato_delay", d.vibrato_delay).clamp(0.0, 4.0)))
}

/// 揉弦写成的弯音折线（`[拍, 半音]`，拍相对音符开头）：开头 `delay` 秒不动，之后在 0.2 秒里揉到全深。
/// `bps` 是每秒几拍。音符太短（揉不到半圈）时为空。
pub(crate) fn vibrato_bend(length_beats: f64, bps: f64, (cents, rate, delay): (f64, f64, f64)) -> Vec<(f64, f64)> {
    let seconds = length_beats / bps;
    if seconds < delay + 0.5 / rate {
        return Vec::new();
    }
    let step = 1.0 / (rate * 16.0);
    let mut out = vec![(0.0, 0.0), (delay * bps, 0.0)];
    let mut t = delay + step;
    while t <= seconds {
        let rise = ((t - delay) / 0.2).min(1.0);
        let v = cents / 100.0 * rise * (core::f64::consts::TAU * rate * (t - delay)).sin();
        out.push((t * bps, v));
        t += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trills_alternate_and_half_trills_shake_the_pitch() {
        let n: muser_core::Note = serde_json::from_value(serde_json::json!({"pitch": 74, "start": 1, "length": 1, "trill": true})).unwrap();
        assert_eq!(n.trill, 2);
        // ♩=120：每拍 0.5 秒，8 个来回 / 秒 → 8 个音。
        let out = expand_trills(&[n], 2.0);
        assert_eq!(out.len(), 8);
        assert_eq!(out.iter().map(|m| m.pitch).collect::<Vec<_>>(), [74, 76, 74, 76, 74, 76, 74, 76]);
        assert!((out[7].start.0 + out[7].length.0 - 2.0).abs() < 0.01 && out.iter().all(|m| m.trill == 0));
        let h: muser_core::Note = serde_json::from_value(serde_json::json!({"pitch": 74, "start": 0, "length": 1, "half_trill": true})).unwrap();
        let shaken = &expand_trills(&[h], 2.0)[0];
        let (lo, hi) = shaken.bend.iter().fold((0.0f64, 0.0f64), |(a, b), p| (a.min(p.1), b.max(p.1)));
        assert!((lo + 0.25).abs() < 0.02 && hi <= 1e-9 && shaken.bend.len() > 20, "only down to -25 cents: {lo} {hi}");
    }

    #[test]
    fn flutter_tongue_shakes_the_note_about_forty_times_a_second() {
        let sr = 48_000.0;
        let play = |flutter: bool| {
            let mut d = Dizi::new(DiziParams::dizi());
            d.prepare(&ProcessSpec { sample_rate: sr, max_block_size: 480, num_channels: 1 });
            d.flutter_next(flutter);
            d.note_on(NoteRef::new(0, 81), 0.8, false);
            let mut b = AudioBuffer::<f32>::new(1, 480);
            let mut env = Vec::new();
            for _ in 0..200 {
                b.clear();
                d.process(&mut b);
                // 每 1 ms 一个峰值。
                env.extend(b.channel(0).chunks(48).map(|c| c.iter().fold(0f32, |m, x| m.max(x.abs()))));
            }
            env.drain(..500).count();
            // 40 Hz 处的起伏占比。
            let (mut re, mut im) = (0.0f64, 0.0f64);
            let mean = env.iter().map(|&x| f64::from(x)).sum::<f64>() / env.len() as f64;
            for (i, &x) in env.iter().enumerate() {
                let a = core::f64::consts::TAU * 40.0 * i as f64 / 1000.0;
                re += (f64::from(x) - mean) * a.cos();
                im += (f64::from(x) - mean) * a.sin();
            }
            (re.hypot(im) * 2.0 / env.len() as f64) / mean
        };
        let (plain, fluttered) = (play(false), play(true));
        assert!(fluttered > 0.15 && plain < 0.05, "plain {plain}, flutter {fluttered}");
    }

    #[test]
    fn guzheng_vibrato_starts_late_and_swings_both_ways() {
        let v = vibrato_bend(4.0, 2.0, (25.0, 5.0, 0.3));
        assert!(v.iter().take_while(|p| p.0 <= 0.6).all(|p| p.1 == 0.0), "still for the first 0.3 s");
        let (lo, hi) = v.iter().fold((0.0f64, 0.0f64), |(a, b), p| (a.min(p.1), b.max(p.1)));
        assert!(hi > 0.24 && lo < -0.24 && hi <= 0.25 + 1e-9);
        assert!(vibrato_bend(0.5, 2.0, (25.0, 5.0, 0.3)).is_empty(), "too short to vibrate");
        assert!(vibrato_for("yinse.pluck.guzheng", &Default::default()).is_some());
        assert!(vibrato_for("guitar.nylon", &Default::default()).is_none());
    }

    #[test]
    fn tremolo_notes_become_fast_repeats() {
        let mut n = muser_core::Note::new(62, 1.0, 2.0);
        n.tremolo = true;
        let plain = muser_core::Note::new(60, 0.0, 1.0);
        // 每秒 2 拍、每秒 12 下：2 拍 = 1 秒 = 12 下。
        let out = expand_tremolo(&[plain.clone(), n], 12.0, 2.0);
        assert_eq!(out[0], plain);
        assert_eq!(out.len(), 13);
        assert!(out[1..].iter().all(|m| m.pitch == 62 && !m.tremolo && m.length.0 < 2.0 / 12.0));
        assert!((out[2].start.0 - (1.0 + 2.0 / 12.0)).abs() < 1e-9);
        assert_eq!(tremolo_rate("yinse.pluck.pipa", &Default::default()), Some(12.0));
        assert_eq!(tremolo_rate("yinse.bow.erhu", &Default::default()), None);
    }
}
