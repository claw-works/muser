//! 中国民族乐器（muser 自己的，yinse）：古筝 `yinse.pluck.guzheng`、二胡 `yinse.bow.erhu`。
//!
//! 都在 manguebyte 的物理模型上配参数：
//! - **古筝**：拨弦模型。钢丝缠尼龙的弦，义甲在靠右边岳山的地方拨（亮、带一点"铮"），桐木大面板的共鸣，
//!   弦响得很久（松开也不止音，筝的弦是自己慢慢停的）。左手的**揉弦**（按在雁柱左边的弦上，音高起伏）由
//!   渲染器按 `vibrato` 写成弯音：音符开头一段之后才揉；音符自己写了 `bend`（按音、滑音）就按它的来，不再加揉弦。
//! - **二胡**：拉弦模型。两根弦（D4、A4），蟒皮琴筒：没有木头箱体的密集共鸣，几个蟒皮与琴筒的共鸣让它带鼻音、
//!   发亮；弓毛夹在两弦之间，擦弦声多。揉弦宽而慢起，连奏（clip 的 `legato`）的音之间手指滑过去（单音，滑音）。

use manguebyte::dsp::instruments::{BodyMode, BowedParams, StringParams};

/// 古筝：弦 + 揉弦。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuzhengParams {
    pub string: StringParams,
    /// 揉弦的深浅（音分，上下各这么多）、快慢（Hz）、音符开头多久之后开始（秒）。
    pub vibrato: f64,
    pub vibrato_rate: f64,
    pub vibrato_delay: f64,
}

impl GuzhengParams {
    pub fn guzheng() -> Self {
        Self {
            string: StringParams {
                decay: 7.0,
                decay_per_octave: 1.4,
                high_decay: 0.3,
                release: 1.6,
                pluck_position: 0.1,
                pluck_brightness: 9_000.0,
                velocity_darkening: 1.8,
                velocity_range_db: 22.0,
                stiffness: 0.1,
                strike: 0.5,
                pluck_noise: 0.3,
                body: [BodyMode::new(90.0, 8.0, 0.6), BodyMode::new(185.0, 10.0, 0.6), BodyMode::new(320.0, 10.0, 0.5), BodyMode::new(520.0, 8.0, 0.4)],
                body_mix: 0.55,
                pickup_position: 0.0,
                tone: 10_000.0,
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
        }
    }
}

/// 二胡：D4、A4 两根弦，蟒皮琴筒，宽揉弦，连奏滑音。
pub fn erhu() -> BowedParams {
    let d4 = 293.66;
    BowedParams {
        // 比 D4 低的音也在内弦上（二胡最低就是 D4）。
        strings: [d4, d4, d4, 440.0],
        bow_position: 0.12,
        pressure: 0.6,
        attack: 0.05,
        bow_noise: 0.45,
        decay: 0.8,
        decay_per_octave: 1.3,
        high_decay: 0.2,
        release: 0.2,
        vibrato_rate: 6.2,
        vibrato_depth: 35.0,
        vibrato_delay: 0.2,
        vibrato_rise: 0.3,
        legato_time: 0.12,
        portamento: true,
        drift: 4.0,
        fluctuation: 0.05,
        swell: 0.12,
        scratch: 0.5,
        body: [
            BodyMode::new(300.0, 6.0, 0.3),
            BodyMode::new(500.0, 8.0, 1.0),
            BodyMode::new(820.0, 10.0, 0.7),
            BodyMode::new(1_150.0, 8.0, 0.6),
            BodyMode::new(1_700.0, 6.0, 0.9),
            BodyMode::new(2_600.0, 4.0, 0.8),
            BodyMode::new(3_400.0, 5.0, 0.5),
            BodyMode::new(4_500.0, 3.0, 0.3),
        ],
        wood: 0.25,
        body_mix: 0.9,
        tone: 8_000.0,
        ..BowedParams::violin()
    }
}

/// 这条音轨要由渲染器加的揉弦：(深浅音分, 快慢 Hz, 开始前的秒数)；不是古筝或深浅为 0 时为 None。
pub(crate) fn vibrato_for(instrument: &str, params: &muser_core::InstrumentParams) -> Option<(f64, f64, f64)> {
    if instrument.trim().to_lowercase() != "yinse.pluck.guzheng" {
        return None;
    }
    let d = GuzhengParams::guzheng();
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
    fn guzheng_vibrato_starts_late_and_swings_both_ways() {
        let v = vibrato_bend(4.0, 2.0, (25.0, 5.0, 0.3));
        assert!(v.iter().take_while(|p| p.0 <= 0.6).all(|p| p.1 == 0.0), "still for the first 0.3 s");
        let (lo, hi) = v.iter().fold((0.0f64, 0.0f64), |(a, b), p| (a.min(p.1), b.max(p.1)));
        assert!(hi > 0.24 && lo < -0.24 && hi <= 0.25 + 1e-9);
        assert!(vibrato_bend(0.5, 2.0, (25.0, 5.0, 0.3)).is_empty(), "too short to vibrate");
        assert!(vibrato_for("yinse.pluck.guzheng", &Default::default()).is_some());
        assert!(vibrato_for("guitar.nylon", &Default::default()).is_none());
    }
}
