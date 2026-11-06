//! 失真效果器（`distortion`）：高通 → 抗混叠低通 → 驱动 + 整形曲线 → 隔直流 → 音色低通 → 箱体 → 响度补偿 → 干湿。
//! 没有过采样（零延迟，现场演奏中途也能接入）：整形前先低通到 9 kHz，混叠落在箱体和音色滤波之下。
//! 响度补偿是静态的：按整形曲线与驱动算出一个 -12 dBFS 正弦过曲线后的 RMS 变化，反过来乘上（不随信号起伏，不"抽"）。

use manguebyte::dsp::{DcBlocker, ShaperCurve, Waveshaper};
use manguebyte::prelude::{AudioBuffer, Filter, FilterShape, ProcessSpec, Processor};

/// 响度补偿按这个幅度的正弦算（约 -12 dBFS，乐器轨的常见电平）。
const REFERENCE_AMP: f32 = 0.25;
/// 整形前的抗混叠低通（Hz）。
const PRE_LOWPASS_HZ: f64 = 9000.0;

pub struct Distortion {
    curve: ShaperCurve,
    /// 整形前加的直流偏置（不对称，偶次谐波）；之后隔直流去掉。
    bias: f32,
    drive_db: f64,
    tone_hz: f64,
    low_cut_hz: f64,
    output: f32,
    mix: f32,
    cabinet: String,
    pre: [Filter<f32>; 2],
    shaper: Waveshaper<f32>,
    dc: DcBlocker<f32>,
    tone: Filter<f32>,
    cab: Vec<Filter<f32>>,
    dry: AudioBuffer<f32>,
    /// 静态响度补偿（见模块说明）。
    gain: f32,
}

/// 参考正弦过（偏置 + 驱动 + 曲线 + 去直流）后 RMS 变了多少：返回它的倒数。
fn compensation(curve: ShaperCurve, bias: f32, drive_db: f64) -> f32 {
    let shaper = Waveshaper::<f32>::new(curve).with_drive_db(drive_db);
    let n = 480;
    let x: Vec<f32> = (0..n).map(|i| REFERENCE_AMP * (2.0 * std::f32::consts::PI * i as f32 / n as f32).sin()).collect();
    let y: Vec<f32> = x.iter().map(|v| shaper.shape(v + bias)).collect();
    let mean = y.iter().sum::<f32>() / n as f32;
    let rms = |v: &mut dyn Iterator<Item = f32>| (v.map(|a| a * a).sum::<f32>() / n as f32).sqrt();
    let (a, b) = (rms(&mut x.iter().copied()), rms(&mut y.iter().map(|v| v - mean)));
    if b > 1e-9 { (a / b).clamp(0.02, 8.0) } else { 1.0 }
}

fn mode(m: &str) -> (ShaperCurve, f32) {
    match m.trim().to_ascii_lowercase().as_str() {
        "overdrive" => (ShaperCurve::SoftClip, 0.15),
        "fuzz" => (ShaperCurve::HardClip, 0.25),
        "fold" => (ShaperCurve::Fold, 0.0),
        _ => (ShaperCurve::Tanh, 0.05),
    }
}

fn cabinet(name: &str) -> Vec<Filter<f32>> {
    match name.trim().to_ascii_lowercase().as_str() {
        "guitar" => vec![
            Filter::new(FilterShape::Highpass, 90.0, 0.7),
            Filter::new(FilterShape::Peak { gain_db: 3.0 }, 1800.0, 0.8),
            Filter::new(FilterShape::Lowpass, 5000.0, 0.7),
            Filter::new(FilterShape::Lowpass, 5000.0, 0.7),
        ],
        "bass" => vec![Filter::new(FilterShape::Highpass, 35.0, 0.7), Filter::new(FilterShape::Lowpass, 3000.0, 0.7), Filter::new(FilterShape::Lowpass, 3000.0, 0.7)],
        _ => Vec::new(),
    }
}

impl Distortion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(mode_name: &str, drive_db: f64, tone_hz: f64, low_cut_hz: f64, cabinet_name: &str, output_db: f64, mix: f64) -> Self {
        let (curve, bias) = mode(mode_name);
        Self {
            curve,
            bias,
            drive_db,
            tone_hz,
            low_cut_hz,
            output: db(output_db),
            mix: mix as f32,
            cabinet: cabinet_name.to_string(),
            pre: [Filter::new(FilterShape::Highpass, low_cut_hz, 0.7), Filter::new(FilterShape::Lowpass, PRE_LOWPASS_HZ, 0.7)],
            shaper: Waveshaper::new(curve).with_drive_db(drive_db),
            dc: DcBlocker::new(),
            tone: Filter::new(FilterShape::Lowpass, tone_hz, 0.7),
            cab: cabinet(cabinet_name),
            dry: AudioBuffer::new(2, 0),
            gain: compensation(curve, bias, drive_db),
        }
    }

    pub fn set_drive_db(&mut self, v: f64) {
        self.drive_db = v;
        self.shaper.set_drive_db(v);
        self.gain = compensation(self.curve, self.bias, v);
    }
    pub fn set_tone_hz(&mut self, v: f64) {
        self.tone_hz = v;
        self.tone.set_frequency(v);
    }
    pub fn set_low_cut_hz(&mut self, v: f64) {
        self.low_cut_hz = v;
        self.pre[0].set_frequency(v);
    }
    pub fn set_output_db(&mut self, v: f64) {
        self.output = db(v);
    }
    pub fn set_mix(&mut self, v: f64) {
        self.mix = v as f32;
    }
}

fn db(v: f64) -> f32 {
    10f64.powf(v / 20.0) as f32
}

impl Processor<f32> for Distortion {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.pre.iter_mut().for_each(|f| f.prepare(spec));
        self.shaper = Waveshaper::new(self.curve).with_drive_db(self.drive_db);
        self.dc.prepare(spec);
        self.tone.prepare(spec);
        self.cab = cabinet(&self.cabinet);
        self.cab.iter_mut().for_each(|f| f.prepare(spec));
        self.dry = AudioBuffer::new(spec.num_channels.max(1), spec.max_block_size.max(1));
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        let (chans, n) = (buffer.num_channels(), buffer.num_frames());
        if self.dry.num_channels() < chans || self.dry.num_frames() < n {
            self.dry = AudioBuffer::new(chans, n);
        }
        for c in 0..chans {
            self.dry.channel_mut(c)[..n].copy_from_slice(&buffer.channel(c)[..n]);
        }
        self.pre.iter_mut().for_each(|f| f.process(buffer));
        if self.bias != 0.0 {
            buffer.iter_channels_mut().for_each(|ch| ch.iter_mut().for_each(|x| *x += self.bias));
        }
        self.shaper.process(buffer);
        self.dc.process(buffer);
        self.tone.process(buffer);
        self.cab.iter_mut().for_each(|f| f.process(buffer));
        let (g, mix, out) = (self.gain, self.mix, self.output);
        for c in 0..chans {
            let dry = &self.dry.channel(c)[..n];
            for (x, d) in buffer.channel_mut(c)[..n].iter_mut().zip(dry) {
                *x = (d + (*x * g - d) * mix) * out;
            }
        }
    }

    fn reset(&mut self) {
        self.pre.iter_mut().for_each(|f| f.reset());
        self.dc.reset();
        self.tone.reset();
        self.cab.iter_mut().for_each(|f| f.reset());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, amp: f32, n: usize) -> AudioBuffer<f32> {
        let mut b = AudioBuffer::<f32>::new(2, n);
        for c in 0..2 {
            for (i, x) in b.channel_mut(c).iter_mut().enumerate() {
                *x = amp * (2.0 * std::f64::consts::PI * f * i as f64 / 48_000.0).sin() as f32;
            }
        }
        b
    }
    fn rms(b: &AudioBuffer<f32>, from: usize) -> f32 {
        let x = &b.channel(0)[from..];
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// 驱动加大，响度不跟着涨（补偿），但谐波变多；分块方式不影响结果。
    #[test]
    fn drive_adds_harmonics_not_level_and_blocks_do_not_matter() {
        let spec = ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 2 };
        let input = sine(220.0, REFERENCE_AMP, 48_000);
        let run = |drive: f64, block: usize| {
            let mut d = Distortion::new("distortion", drive, 8000.0, 40.0, "none", 0.0, 1.0);
            d.prepare(&spec);
            let mut out = input.clone();
            let mut at = 0;
            while at < out.num_frames() {
                let m = block.min(out.num_frames() - at);
                let mut b = AudioBuffer::<f32>::new(2, m);
                for c in 0..2 {
                    b.channel_mut(c).copy_from_slice(&out.channel(c)[at..at + m]);
                }
                d.process(&mut b);
                for c in 0..2 {
                    out.channel_mut(c)[at..at + m].copy_from_slice(b.channel(c));
                }
                at += m;
            }
            out
        };
        let (soft, hard) = (run(6.0, 512), run(36.0, 512));
        let (r_in, r_soft, r_hard) = (rms(&input, 24_000), rms(&soft, 24_000), rms(&hard, 24_000));
        // 约 ±1.5 dB 内（音色低通、隔直流会去掉一点）。
        assert!((r_soft / r_in - 1.0).abs() < 0.19 && (r_hard / r_in - 1.0).abs() < 0.19, "{r_in} {r_soft} {r_hard}");
        // 谐波：和原来的正弦差得越多，失真越重。
        let diff = |b: &AudioBuffer<f32>| rms(&{
            let mut d = b.clone();
            for (x, y) in d.channel_mut(0).iter_mut().zip(input.channel(0)) { *x -= *y; }
            d
        }, 24_000);
        assert!(diff(&hard) > diff(&soft) * 1.5, "{} vs {}", diff(&hard), diff(&soft));
        let odd = run(36.0, 37);
        assert!(odd.channel(0) == hard.channel(0), "分块方式不同，结果应逐样本相同");
    }
}
