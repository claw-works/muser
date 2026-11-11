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
//! - **扬琴**：和古筝同一个模态弦模型（[`crate::zither::YANGQIN`]），竹槌敲：起音脆、没有制音器、一组弦彼此走音，
//!   其余的弦跟着琴码响，立体声。轮音是 `tremolo`。
//! - **笛子**：吹管（长笛）模型 + 笛膜：膜在高频上"嗡"出一片沙沙的泛音，越响越明显（[`Dizi`]）。单音，连着的音滑过去。
//! - **箫**：吹管模型，竖吹：低、暗、气声多，起音慢。单音。
//!
//! 揉弦（古筝、琵琶、古琴）与轮音（古筝、琵琶、扬琴）都由渲染器写成弯音 / 重复的音（见 [`vibrato_for`]、[`tremolo_rate`]）。

use manguebyte::dsp::instruments::{BodyMode, BowedParams, BowedString, StringParams, WindParams, WindVoice};
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
        Self { string: s, vibrato: 15.0, vibrato_rate: 5.0, vibrato_delay: 0.35, tremolo_rate: 12.0 }
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
        Self { string: s, vibrato: 18.0, vibrato_rate: 3.5, vibrato_delay: 0.5, tremolo_rate: 8.0 }
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
        Self { string: s, vibrato: 0.0, vibrato_rate: 5.0, vibrato_delay: 0.3, tremolo_rate: 14.0 }
    }
}

/// 按乐器名取拨弦类的预设（揉弦、轮音的默认值从这里来）。
fn pluck_preset(instrument: &str) -> Option<GuzhengParams> {
    if instrument.trim().starts_with("yinse:") {
        // 录音提取的乐器：轮音按量到的快慢（没量过就照扬琴每秒 14 下），不自动揉弦（揉音写在音符的 articulation 上）。
        let rate = crate::modal::table_of(instrument).and_then(|t| t.gestures.tremolo).map_or(14.0, |t| t.rate);
        return Some(GuzhengParams { vibrato: 0.0, tremolo_rate: rate, ..GuzhengParams::yangqin() });
    }
    Some(match instrument.trim().to_lowercase().as_str() {
        "yinse.pluck.pipa" => crate::zither::ZitherParams::pipa().pluck,
        "yinse.pluck.guzheng" => crate::zither::ZitherParams::guzheng().pluck,
        "yinse.pluck.guqin" => GuzhengParams::guqin(),
        "yinse.strike.yangqin" => crate::zither::ZitherParams::yangqin().pluck,
        _ => return None,
    })
}

/// 轮音每秒几下（音符写了 `tremolo` 时）：古筝、琵琶、古琴、扬琴；其它乐器为 None（不轮）。
pub(crate) fn tremolo_rate(instrument: &str, params: &muser_core::InstrumentParams) -> Option<f64> {
    let d = pluck_preset(instrument)?;
    Some(params.get("tremolo_rate").and_then(serde_json::Value::as_f64).unwrap_or(d.tremolo_rate).clamp(2.0, 30.0))
}

/// 把写了 `tremolo` 的音拆成一串重复的音（`bps` 每秒几拍）：每下长度约一下的九成，轮指的几个手指力度略有不同。
/// 录音量出来的轮音 `shape`（[`crate::modal::Tremolo`]）：两只槌交替的力度比、越轮越快；也认 `"articulation": "tremolo"`。
pub(crate) fn expand_tremolo(notes: &[muser_core::Note], rate: f64, bps: f64, shape: Option<crate::modal::Tremolo>) -> Vec<muser_core::Note> {
    let mut out = Vec::with_capacity(notes.len());
    for n in notes {
        if !n.tremolo && n.articulation.as_deref() != Some("tremolo") {
            out.push(n.clone());
            continue;
        }
        // 一下一下排开：量过的轮音从开头的速度渐变到结尾的速度（accel 倍）。
        let accel = shape.map_or(1.0, |s| s.accel);
        let mut starts = Vec::new();
        let mut t = 0.0;
        while t < n.length.0 - 1e-9 || starts.is_empty() {
            starts.push(t);
            let f = (t / n.length.0.max(1e-9)).clamp(0.0, 1.0);
            t += bps / (rate * accel.powf(f - 0.5));
        }
        for (i, &at) in starts.iter().enumerate() {
            let step = starts.get(i + 1).map_or(n.length.0 - at, |&b| b - at).max(bps / rate * 0.5);
            let mut m = n.clone();
            m.tremolo = false;
            m.articulation = None;
            m.start = muser_core::Beats(n.start.0 + at);
            m.length = muser_core::Beats(step * 0.9);
            // 轮指五个手指（扬琴两只槌）：力度按位置小小地起伏；量过的就按两只槌的力度比。
            let pattern = match shape {
                Some(s) => if i % 2 == 0 { 1.0 } else { s.alternate },
                None => [1.0, 0.9, 0.95, 0.88, 0.93][i % 5],
            };
            m.velocity = (n.velocity * pattern as f32).clamp(0.0, 1.0);
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
        if (n.scoop > 0.0 || n.fall > 0.0) && n.bend.is_empty() {
            // 滑进来：从下面 `scoop` 个半音，0.12 秒滑到本音；甩出去：最后 0.15 秒往下滑 `fall` 个半音。
            let (into, out) = ((0.12 * bps).min(n.length.0 * 0.4), (0.15 * bps).min(n.length.0 * 0.4));
            let mut b = vec![(0.0, -n.scoop), (into, 0.0)];
            if n.fall > 0.0 {
                b.push(((n.length.0 - out).max(into), 0.0));
                b.push((n.length.0, -n.fall));
            }
            n.bend = b;
        }
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


/// 笛子：吹管 + 笛膜的嗡声。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiziParams {
    pub wind: WindParams,
    /// 笛膜：0 没有（像长笛），0.6 贴得正好、明显的"沙"，1 很响的嗡。
    pub membrane: f64,
    /// 偶次谐波（0 没有）：簧片吹管（葫芦丝）在单簧管的奇次谐波上加的偶次谐波，按音量的形状平方出来。
    pub even: f64,
    /// 附管（葫芦丝两边的管）：一直响着的持续音的音量（0 关），和它的音高（MIDI）。吹着就响，换气、断奏时一起停。
    pub drone: f64,
    pub drone_note: f64,
    /// 音色的固定着色：900 Hz 以下的低搁架（dB，0 不动）、`presence_hz` 一带的"鼻音"峰（dB，0 不动）。唢呐用。
    pub low_shelf: f64,
    pub presence: f64,
    pub presence_hz: f64,
    /// 沙哑（0 … 1）：跟着每个周期的波形起落的噪声（簧片拍打得不规则），唢呐的毛、"破"。
    pub rasp: f64,
    /// 高音更尖更毛：从 `high_from`（MIDI）起，每高一个八度，沙哑与笛膜（毛刺）各乘上 1 + `high`。0 不变。
    pub high: f64,
    pub high_from: f64,
    /// 宽度（0 … 1）：几个很近、左右不同的反射，让声音有左右的宽度（不是混响）。
    pub width: f64,
    /// 气息强弱（乘在吹气的力度上，0.3 … 1.5）：1 照常，小了虚、暗、气多，大了实、亮。可写自动化。
    pub breath: f64,
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
        // 连奏（音重叠）时手指一换音就到，不滑；滑音只在音符写了 `slide` 时（见 [`Dizi::note_on`]）。
        w.portamento = false;
        Self { wind: w, membrane: 0.45, breath: 1.0, even: 0.0, drone: 0.0, drone_note: 72.0, width: 0.0, low_shelf: 0.0, presence: 0.0, presence_hz: 2_200.0, rasp: 0.0, high: 0.0, high_from: 127.0 }
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
pub fn xiao() -> DiziParams {
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
    w.legato_time = 0.1;
    w.tone = 7_000.0;
    w.portamento = false;
    // 没有笛膜：和笛子共用同一套处理（花舌、剁音、滑音、气息），膜为 0 时声音就是吹管本身。
    DiziParams { wind: w, membrane: 0.0, breath: 1.0, even: 0.0, drone: 0.0, drone_note: 72.0, width: 0.0, low_shelf: 0.0, presence: 0.0, presence_hz: 2_200.0, rasp: 0.0, high: 0.0, high_from: 127.0 }
}

/// 葫芦丝：自由簧（铜簧片）装在葫芦做的气室里，下面一根主管、两边附管。和单簧管一样以奇次谐波为主，
/// 偶次谐波也不弱。对着四段独奏录音（侗乡之夜、山寨情歌、瑶族舞曲、守望之夜）的长音：二、三、四、五次谐波
/// 约 -12 / 0 / -15 / -10 dB；长音的颤动几乎全在气息上（腹震音、气震音：音量起伏标准差约 0.16 … 0.25，每秒约 3 … 4 下），
/// 音高几乎不揉（±2 … 3 音分）；附管默认不响（`drone` 打开）。
pub fn hulusi() -> DiziParams {
    let mut w = WindParams::clarinet();
    w.opening = 0.7;
    // 簧片的共鸣要远高于最高音的三倍：3.5 kHz 时 E5 以上就翻到十二度（三次谐波）上去，"劈"了。
    w.reed_frequency = 6_000.0;
    w.reed_damping = 0.5;
    w.bell = 4_000.0;
    w.tone = 6_000.0;
    w.vibrato_rate = 4.0;
    w.vibrato_depth = 0.0;
    w.tremolo = 0.5;
    w.vibrato_delay = 0.3;
    w.attack = 0.03;
    w.portamento = false;
    w.legato_time = 0.08;
    DiziParams { wind: w, membrane: 0.0, breath: 1.0, even: 0.4, drone: 0.0, drone_note: 72.0, width: 0.0, low_shelf: 0.0, presence: 0.0, presence_hz: 2_200.0, rasp: 0.0, high: 0.0, high_from: 127.0 }
}

/// 唢呐：双簧片、锥形木管、铜碗（喇叭口）——响、亮、带沙。对着七段独奏录音（百鸟朝凤、正月十五闹花灯、小放牛、喜事儿等）：
/// 长音的二、三、四、五次谐波约 0 / -6 / -11 / -20 dB，能量集中在 1.6 … 4 kHz；揉音宽（约 ±40 音分）；
/// 音很碎（一半的音短于 70 ms），大量从下面滑进来、结尾往下甩的滑音。萨克斯的锥形管模型，
/// 簧片共鸣放到最高、阻尼加大（不然有的音翻到八度上去），笛膜的"沙"当作铜碗和簧片的响亮的毛刺。
pub fn suona() -> DiziParams {
    let mut w = WindParams::saxophone();
    w.opening = 0.6;
    w.reed_frequency = 6_000.0;
    w.reed_damping = 1.6;
    w.bell = 2_500.0;
    w.brassiness = 0.3;
    w.tone = 14_000.0;
    // 簧片不那么稳：气声多一倍，气息和音高晃得多（用户在几版里选的"更粗更不稳"）。
    w.noise = 0.6;
    w.fluctuation = 0.07;
    w.drift = 6.0;
    w.breath_soft = 0.4;
    w.breath_loud = 1.0;
    w.vibrato_depth = 30.0;
    w.vibrato_rate = 5.5;
    w.vibrato_delay = 0.25;
    w.tremolo = 0.3;
    w.portamento = false;
    w.legato_time = 0.06;
    // 录音的长时频谱：最响的是 1.3 … 3 kHz 一带（鼻音、"亮"），900 Hz 以下比我们的管子弱得多。
    // 谐波之间的毛（噪声）录音里约 -49 dB（相对最强的谐波），管子本身只有约 -59：加沙哑补上。
    // 高音越吹越尖、越毛（管子本身越往上越干净）：D6 往上每个八度，沙哑与毛刺多 80%。
    DiziParams { wind: w, membrane: 1.0, breath: 1.0, even: 0.0, drone: 0.0, drone_note: 72.0, width: 0.0, low_shelf: -8.0, presence: 6.0, presence_hz: 2_200.0, rasp: 0.35, high: 0.8, high_from: 86.0 }
}

/// 大唢呐（大杆）：管长、喇叭口大，低四五度；浑厚、粗、低中频多，喇叭口放得出更多低频，揉得更宽。
pub fn suona_big() -> DiziParams {
    let mut d = suona();
    d.wind.bell = 1_600.0;
    d.wind.reed_frequency = 4_500.0;
    d.wind.reed_damping = 1.6;
    d.wind.opening = 0.6;
    d.wind.vibrato_depth = 35.0;
    d.wind.vibrato_rate = 5.0;
    d.low_shelf = -2.0;
    d.presence = 3.0;
    d.presence_hz = 1_600.0;
    d.membrane = 0.8;
    d.rasp = 0.45;
    d.high_from = 79.0;
    d
}

/// 海笛（小唢呐）：管短、哨片小，高四度到八度；尖、细、刺，最亮，鸟叫、花腔。
pub fn haidi() -> DiziParams {
    let mut d = suona();
    d.wind.bell = 5_000.0;
    d.wind.reed_damping = 1.6;
    d.wind.opening = 0.6;
    d.wind.vibrato_depth = 22.0;
    d.wind.vibrato_rate = 6.2;
    d.low_shelf = -12.0;
    d.presence = 8.0;
    d.presence_hz = 4_000.0;
    d.wind.tone = 18_000.0;
    d.rasp = 0.3;
    d.high_from = 93.0;
    d
}

/// 笛子：一支单音的吹管，后面接笛膜（越响越"沙"的高频泛音）。
pub(crate) struct Dizi {
    synth: Synth<WindVoice, f32>,
    /// 附管（`drone` > 0 时），按着几个音、它在吹哪个音，混音用的缓冲。
    drone: Synth<WindVoice, f32>,
    held: usize,
    drone_key: Option<u8>,
    drone_buf: AudioBuffer<f32>,
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
    /// 下一个音、正在吹的音：滑不滑过去，是不是剁音。
    slide_next: bool,
    slide: bool,
    accent: bool,
    breath_gain: f32,
    /// [`DiziParams::low_shelf`]、[`DiziParams::presence`] 的滤波。
    color: Vec<Filter<f32>>,
    rasp_lp: [f32; 2],
    /// 这个音的 [`DiziParams::high`] 系数。
    high_gain: f64,
    room_buf: [Vec<f32>; 2],
    room_lp: [f32; 2],
    room_at: usize,
    even_env: [f32; 2],
    even_dc: [f32; 2],
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
        let drone = Synth::with_voices(1, move || WindVoice::new(w));
        let peak = || [Filter::new(FilterShape::Bandpass, 3_200.0, 2.0), Filter::new(FilterShape::Bandpass, 6_000.0, 2.5)];
        Self { synth, drone, held: 0, drone_key: None, drone_buf: AudioBuffer::new(1, 0), params, membrane: [peak(), peak()], wet: AudioBuffer::new(1, 0), wet2: AudioBuffer::new(1, 0), env: [0.0; 2],
            flutter_next: false, flutter: false, flutter_phase: 0.0, flutter_lp: 0.0, slide_next: false, slide: false, accent: false, breath_gain: params.breath.clamp(0.3, 1.5).powf(1.2) as f32, even_env: [0.0; 2], even_dc: [0.0; 2], room_buf: [Vec::new(), Vec::new()], room_lp: [0.0; 2], room_at: 0, color: Vec::new(), rasp_lp: [0.0; 2], high_gain: 1.0, sr: 48_000.0, rng: 0x1234_5678 }
    }

    pub(crate) fn flutter_next(&mut self, on: bool) {
        self.flutter_next = on;
    }

    pub(crate) fn slide_next(&mut self, on: bool) {
        self.slide_next = on;
    }

    /// 这一个音的吹管设置：气息强弱缩放吹气的范围；写了 `slide` 的音滑过去；剁音（重音）松开时一下收住。
    fn voice_params(&self, accent: bool, slide: bool) -> WindParams {
        let mut w = self.params.wind;
        let b = self.params.breath.clamp(0.3, 1.5);
        w.breath_soft = (w.breath_soft * b).min(1.0);
        w.breath_loud = (w.breath_loud * b).min(1.0);
        w.portamento = slide;
        if accent {
            w.release = w.release.min(0.02);
        }
        w
    }

    pub(crate) fn prepare(&mut self, spec: &ProcessSpec) {
        Processor::prepare(&mut self.synth, spec);
        Processor::prepare(&mut self.drone, spec);
        self.drone_buf = AudioBuffer::new(spec.num_channels, spec.max_block_size);
        self.color.clear();
        if self.params.low_shelf != 0.0 {
            self.color.push(Filter::new(FilterShape::LowShelf { gain_db: self.params.low_shelf }, 900.0, 0.7));
        }
        if self.params.presence != 0.0 {
            self.color.push(Filter::new(FilterShape::Peak { gain_db: self.params.presence }, self.params.presence_hz, 0.8));
        }
        self.color.iter_mut().for_each(|f| f.prepare(spec));
        let room = (0.04 * spec.sample_rate) as usize;
        self.room_buf = [vec![0.0; room], vec![0.0; room]];
        self.room_at = 0;
        self.held = 0;
        self.drone_key = None;
        self.sr = spec.sample_rate;
        let mono = ProcessSpec { num_channels: 1, ..*spec };
        self.membrane.iter_mut().flatten().for_each(|f| f.prepare(&mono));
        self.wet = AudioBuffer::new(1, spec.max_block_size);
        self.wet2 = AudioBuffer::new(1, spec.max_block_size);
    }

    pub(crate) fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        self.flutter = std::mem::take(&mut self.flutter_next);
        self.accent = accent;
        self.high_gain = 1.0 + self.params.high * ((f64::from(note.key) - self.params.high_from) / 12.0).max(0.0);
        self.slide = std::mem::take(&mut self.slide_next);
        let w = self.voice_params(accent, self.slide);
        self.synth.voices_mut().for_each(|v| v.set_params(w));
        self.synth.note_on_accented(note, velocity, accent);
        self.held += 1;
        if self.params.drone > 0.0 && self.drone_key.is_none() {
            let key = self.params.drone_note.round().clamp(24.0, 108.0) as u8;
            let mut d = w;
            d.portamento = false;
            self.drone.voices_mut().for_each(|v| v.set_params(d));
            self.drone.note_on_accented(NoteRef::new(0, key), velocity, false);
            self.drone_key = Some(key);
        }
    }

    pub(crate) fn note_off(&mut self, note: NoteRef, velocity: f32) {
        self.synth.note_off(note, velocity);
        self.held = self.held.saturating_sub(1);
        if self.held == 0 {
            if let Some(key) = self.drone_key.take() {
                self.drone.note_off(NoteRef::new(0, key), velocity);
            }
        }
    }

    pub(crate) fn midi(&mut self, message: MidiMessage) {
        self.synth.handle_event(&manguebyte::prelude::Event::from_midi(0, message));
    }

    pub(crate) fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        Processor::process(&mut self.synth, buffer);
        if self.params.drone > 0.0 || self.drone.voices_mut().any(|v| manguebyte::prelude::Voice::<f32>::is_active(v)) {
            let n = buffer.num_frames();
            if self.drone_buf.num_frames() < n || self.drone_buf.num_channels() < buffer.num_channels() {
                self.drone_buf = AudioBuffer::new(buffer.num_channels(), n);
            }
            self.drone_buf.set_num_frames(n);
            self.drone_buf.clear();
            Processor::process(&mut self.drone, &mut self.drone_buf);
            let g = self.params.drone.clamp(0.0, 1.0) as f32;
            for c in 0..buffer.num_channels() {
                for (y, d) in buffer.channel_mut(c).iter_mut().zip(self.drone_buf.channel(c.min(self.drone_buf.num_channels() - 1))) {
                    *y += g * *d;
                }
            }
        }
        if self.flutter {
            self.flutter_tongue(buffer);
        }
        // 气息强弱：吹管模型里它只改音色（吹得越足越亮），响度要另外跟上；平滑地跟，免得自动化一段一段地跳。
        let target = self.params.breath.clamp(0.3, 1.5).powf(1.2) as f32;
        if target != 1.0 || self.breath_gain != 1.0 {
            let k = (1.0 - (-1.0 / (0.02 * self.sr)).exp()) as f32;
            for i in 0..buffer.num_frames() {
                self.breath_gain += (target - self.breath_gain) * k;
                for c in 0..buffer.num_channels() {
                    buffer.channel_mut(c)[i] *= self.breath_gain;
                }
            }
            if (self.breath_gain - target).abs() < 1e-4 {
                self.breath_gain = target;
            }
        }
        if self.params.even > 0.0 {
            let k = self.params.even as f32;
            for c in 0..buffer.num_channels().min(2) {
                let env = &mut self.even_env[c];
                let dc = &mut self.even_dc[c];
                for x in buffer.channel_mut(c).iter_mut() {
                    *env = x.abs().max(*env * 0.9995);
                    let d = *x / (*env + 1e-6);
                    // 平方出二次谐波（及奇次谐波的和差：偶次），减掉它自己的直流。
                    let y = k * *env * d * d;
                    *dc += 0.0005 * (y - *dc);
                    *x += y - *dc;
                }
            }
        }
        let m = (self.params.membrane * self.high_gain).clamp(0.0, 1.5) as f32;
        if m > 0.0 {
            self.buzz(buffer, m);
        }
        if self.params.rasp > 0.0 {
            self.rasp(buffer);
        }
        if !self.color.is_empty() {
            self.color.iter_mut().for_each(|f| f.process(buffer));
        }
        if self.params.width > 0.0 {
            self.room(buffer);
        }
    }

    /// 笛膜（唢呐：铜碗与簧片的毛刺）：按音量的形状削出高频泛音，经两个共鸣放出来。
    fn buzz(&mut self, buffer: &mut AudioBuffer<f32>, m: f32) {
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

    /// 沙哑：白噪声乘上波形的绝对值（每个周期波峰处一阵噪声），去掉 800 Hz 以下，加回去。
    fn rasp(&mut self, buffer: &mut AudioBuffer<f32>) {
        let k = (self.params.rasp * self.high_gain).clamp(0.0, 1.5) as f32 * 0.5;
        let a = (1.0 - (-core::f64::consts::TAU * 800.0 / self.sr).exp()) as f32;
        for i in 0..buffer.num_frames() {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
            for c in 0..buffer.num_channels().min(2) {
                let x = buffer.channel(c)[i];
                let r = noise * x.abs();
                self.rasp_lp[c] += a * (r - self.rasp_lp[c]);
                buffer.channel_mut(c)[i] = x + k * (r - self.rasp_lp[c]);
            }
        }
    }

    /// 宽度：几个很近的反射（左右的时刻不同），像在屋里吹，声音有了左右的宽度，又不像混响那样拖尾。
    fn room(&mut self, buffer: &mut AudioBuffer<f32>) {
        const TAPS: [[f64; 3]; 2] = [[0.0071, 0.0133, 0.0229], [0.0089, 0.0167, 0.0293]];
        let len = self.room_buf[0].len();
        if len == 0 || buffer.num_channels() < 2 {
            return;
        }
        let w = self.params.width.clamp(0.0, 1.0) as f32;
        for i in 0..buffer.num_frames() {
            let mono = 0.5 * (buffer.channel(0)[i] + buffer.channel(1)[i]);
            for (c, taps) in TAPS.iter().enumerate() {
                self.room_lp[c] += 0.35 * (mono - self.room_lp[c]);
                self.room_buf[c][self.room_at] = self.room_lp[c];
                let mut wet = 0.0;
                for (t, g) in taps.iter().zip([0.5f32, 0.35, 0.25]) {
                    let d = (t * self.sr) as usize;
                    wet += self.room_buf[c][(self.room_at + len - d) % len] * g;
                }
                buffer.channel_mut(c)[i] += w * wet;
            }
            self.room_at = (self.room_at + 1) % len;
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
            let w = self.voice_params(self.accent, self.slide);
            self.synth.voices_mut().for_each(|v| v.set_params(w));
        }
    }

    pub(crate) fn set_tuning(&mut self, tuning: &manguebyte::dsp::Tuning) {
        self.synth.set_tuning(tuning);
        self.drone.set_tuning(tuning);
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
    fn the_suona_keeps_its_fundamental_and_scoops_and_falls() {
        let sr = 48_000.0;
        for (preset, lo, hi) in [(suona as fn() -> DiziParams, 67u8, 95u8), (suona_big, 55, 84), (haidi, 79, 103)] {
        for key in (lo..=hi).step_by(2) {
            for velocity in [0.5f32, 1.0] {
                let mut d = Dizi::new(preset());
                d.prepare(&ProcessSpec { sample_rate: sr, max_block_size: 480, num_channels: 1 });
                d.note_on(NoteRef::new(0, key), velocity, false);
                let mut b = AudioBuffer::<f32>::new(1, 480);
                let mut x = Vec::new();
                for _ in 0..40 {
                    b.clear();
                    d.process(&mut b);
                    x.extend_from_slice(b.channel(0));
                }
                let f0 = 440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0);
                let at = |hz: f64| {
                    let (mut re, mut im) = (0.0f64, 0.0f64);
                    for (i, &v) in x[9_600..].iter().enumerate() {
                        let a = core::f64::consts::TAU * hz * i as f64 / sr;
                        re += f64::from(v) * a.cos();
                        im += f64::from(v) * a.sin();
                    }
                    re.hypot(im)
                };
                // 翻到八度上时基音几乎没了（低 40 dB 以上）；低搁架只把它压低几 dB。
                assert!(at(f0) > 0.1 * at(2.0 * f0), "key {key} velocity {velocity} jumped to its octave: {:.3}", at(f0) / at(2.0 * f0));
            }
        }
        }
        let n: muser_core::Note = serde_json::from_value(serde_json::json!({"pitch": 74, "start": 0, "length": 2, "scoop": 1.5, "fall": 2})).unwrap();
        let bend = &expand_trills(&[n], 2.0)[0].bend;
        assert_eq!((bend[0], bend[1].1), ((0.0, -1.5), 0.0));
        assert_eq!(*bend.last().unwrap(), (2.0, -2.0));
    }

    #[test]
    fn the_hulusi_does_not_crack_into_its_twelfth() {
        let sr = 48_000.0;
        for key in [67u8, 72, 76, 79, 81, 84, 86] {
            for velocity in [0.4f32, 0.8, 1.0] {
                let mut d = Dizi::new(hulusi());
                d.prepare(&ProcessSpec { sample_rate: sr, max_block_size: 480, num_channels: 1 });
                d.note_on(NoteRef::new(0, key), velocity, velocity >= 1.0);
                let mut b = AudioBuffer::<f32>::new(1, 480);
                let mut x = Vec::new();
                for _ in 0..60 {
                    b.clear();
                    d.process(&mut b);
                    x.extend_from_slice(b.channel(0));
                }
                let f0 = 440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0);
                let at = |hz: f64| {
                    let (mut re, mut im) = (0.0f64, 0.0f64);
                    for (i, &v) in x[14_400..].iter().enumerate() {
                        let a = core::f64::consts::TAU * hz * i as f64 / sr;
                        re += f64::from(v) * a.cos();
                        im += f64::from(v) * a.sin();
                    }
                    re.hypot(im)
                };
                assert!(at(f0) > 0.7 * at(3.0 * f0), "key {key} velocity {velocity}: the fundamental is gone");
            }
        }
    }

    #[test]
    fn the_drone_pipe_sounds_while_the_player_blows() {
        let sr = 48_000.0;
        let mut p = hulusi();
        p.drone = 0.5;
        p.drone_note = 60.0;
        let mut d = Dizi::new(p);
        d.prepare(&ProcessSpec { sample_rate: sr, max_block_size: 480, num_channels: 1 });
        let mut b = AudioBuffer::<f32>::new(1, 480);
        let mut run = |d: &mut Dizi, blocks: usize| {
            let mut x = Vec::new();
            for _ in 0..blocks {
                b.clear();
                d.process(&mut b);
                x.extend_from_slice(b.channel(0));
            }
            x
        };
        let energy_at = |x: &[f32], hz: f64| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &v) in x.iter().enumerate() {
                let a = core::f64::consts::TAU * hz * i as f64 / sr;
                re += f64::from(v) * a.cos();
                im += f64::from(v) * a.sin();
            }
            re.hypot(im) / x.len() as f64
        };
        let c4 = 261.63;
        d.note_on(NoteRef::new(0, 72), 0.8, false);
        let blowing = run(&mut d, 40);
        d.note_off(NoteRef::new(0, 72), 0.0);
        run(&mut d, 60);
        let after = run(&mut d, 20);
        assert!(energy_at(&blowing[9600..], c4) > 0.002, "the drone sounds under the melody");
        assert!(energy_at(&after, c4) < energy_at(&blowing[9600..], c4) * 0.05, "and stops with the breath");
    }

    #[test]
    fn slurs_change_at_once_and_only_slides_glide() {
        let sr = 48_000.0;
        // 从 D5 连到 G5，30 ms 之后的音高（过零点数）。
        let pitch_after_slur = |slide: bool| {
            let mut d = Dizi::new(DiziParams::qudi());
            d.prepare(&ProcessSpec { sample_rate: sr, max_block_size: 480, num_channels: 1 });
            d.note_on(NoteRef::new(0, 74), 0.8, false);
            let mut b = AudioBuffer::<f32>::new(1, 480);
            for _ in 0..40 {
                b.clear();
                d.process(&mut b);
            }
            d.slide_next(slide);
            d.note_on(NoteRef::new(0, 79), 0.8, false);
            d.note_off(NoteRef::new(0, 74), 0.0);
            let mut x = Vec::new();
            for _ in 0..8 {
                b.clear();
                d.process(&mut b);
                x.extend_from_slice(b.channel(0));
            }
            let w = &x[1440..2400];
            let crossings = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
            crossings as f64 / (w.len() as f64 / sr)
        };
        let g5 = 783.99;
        let (slur, slide) = (pitch_after_slur(false), pitch_after_slur(true));
        assert!((slur - g5).abs() < 60.0, "a slur is at the new note: {slur} Hz");
        assert!(slide < g5 - 60.0, "a slide is still on its way: {slide} Hz");
    }

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
        let out = expand_tremolo(&[plain.clone(), n], 12.0, 2.0, None);
        assert_eq!(out[0], plain);
        assert_eq!(out.len(), 13);
        assert!(out[1..].iter().all(|m| m.pitch == 62 && !m.tremolo && m.length.0 < 2.0 / 12.0));
        assert!((out[2].start.0 - (1.0 + 2.0 / 12.0)).abs() < 1e-9);
        assert_eq!(tremolo_rate("yinse.pluck.pipa", &Default::default()), Some(16.0));
        assert_eq!(tremolo_rate("yinse.bow.erhu", &Default::default()), None);
    }

    #[test]
    fn a_measured_tremolo_alternates_and_speeds_up() {
        let mut n = muser_core::Note::new(69, 0.0, 2.0);
        n.articulation = Some("tremolo".into());
        let shape = crate::modal::Tremolo { rate: 13.0, alternate: 0.75, accel: 1.5 };
        let out = expand_tremolo(&[n], 13.0, 1.0, Some(shape));
        assert!((24..=30).contains(&out.len()), "{} strokes", out.len());
        assert!((out[1].velocity / out[0].velocity - 0.75).abs() < 0.01);
        let first = out[1].start.0 - out[0].start.0;
        let last = out[out.len() - 1].start.0 - out[out.len() - 2].start.0;
        assert!(last < first * 0.8, "speeds up: {first} -> {last}");
        assert!(out.iter().all(|m| m.articulation.is_none()));
    }

}
