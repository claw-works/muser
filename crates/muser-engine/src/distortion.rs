//! 失真效果器（`distortion`），照吉他箱头的做法：
//! 前级 EQ（高通 + 中频突起 + 高架提升：被削掉的是中高频，谐波才亮）→ 第一级削波 → 级间滤波 → 第二级削波
//! → 音色低通 → 箱体（低频鼓包、中低频凹、3 kHz 存在感、6.5 kHz 以上四阶滚降）→ 响度补偿 → 干湿。
//! 没有过采样（零延迟，现场演奏中途也能接入）：削波前低通到 10 kHz，混叠落在箱体滚降之下。
//! 响度补偿是静态的：在 `prepare` 时用一个和弦锯齿波把整条链跑一遍，按驱动 0–48 dB 每 1 dB 记下 RMS 变化，演奏时查表（不随信号起伏）。

use manguebyte::dsp::{DcBlocker, ShaperCurve, Waveshaper};
use manguebyte::prelude::{AudioBuffer, Filter, FilterShape, ProcessSpec, Processor};

/// 参考信号的峰值（约 -12 dBFS，乐器轨的常见电平）。
const REFERENCE_AMP: f32 = 0.25;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Overdrive,
    Distortion,
    Fuzz,
    Fold,
}

fn mode(m: &str) -> Mode {
    match m.trim().to_ascii_lowercase().as_str() {
        "overdrive" => Mode::Overdrive,
        "fuzz" => Mode::Fuzz,
        "fold" => Mode::Fold,
        _ => Mode::Distortion,
    }
}

/// 两级削波的曲线、各级分到的驱动比例、第一级的直流偏置（不对称，偶次谐波）。
fn stages(m: Mode) -> ([ShaperCurve; 2], [f64; 2], f32) {
    match m {
        Mode::Overdrive => ([ShaperCurve::SoftClip, ShaperCurve::SoftClip], [0.7, 0.3], 0.12),
        Mode::Distortion => ([ShaperCurve::Tanh, ShaperCurve::Tanh], [0.5, 0.5], 0.05),
        Mode::Fuzz => ([ShaperCurve::HardClip, ShaperCurve::Tanh], [0.8, 0.2], 0.2),
        Mode::Fold => ([ShaperCurve::Fold, ShaperCurve::Tanh], [0.9, 0.1], 0.0),
    }
}

fn cabinet(name: &str) -> Vec<Filter<f32>> {
    match name.trim().to_ascii_lowercase().as_str() {
        "guitar" => vec![
            // 削波之前低频切掉了（不然糊），削完以后靠箱头的音色旋钮和箱体把"厚"补回来：350 Hz 以下整体抬起来、
            // 110 Hz 箱体的鼓包（闷音的"咚"）、500 Hz 上下挖一点（不然是鼻音、电话声），3 kHz 的"亮"不用那么多。
            Filter::new(FilterShape::Highpass, 75.0, 0.7),
            Filter::new(FilterShape::LowShelf { gain_db: 11.0 }, 320.0, 0.7),
            Filter::new(FilterShape::Peak { gain_db: 3.0 }, 110.0, 1.0),
            Filter::new(FilterShape::Peak { gain_db: -4.0 }, 550.0, 0.7),
            Filter::new(FilterShape::Peak { gain_db: 4.0 }, 3000.0, 0.9),
            Filter::new(FilterShape::Lowpass, 6500.0, 0.7),
            Filter::new(FilterShape::Lowpass, 6500.0, 0.7),
        ],
        "bass" => vec![
            Filter::new(FilterShape::Highpass, 35.0, 0.7),
            Filter::new(FilterShape::Peak { gain_db: 2.0 }, 80.0, 1.0),
            Filter::new(FilterShape::Lowpass, 3500.0, 0.7),
            Filter::new(FilterShape::Lowpass, 3500.0, 0.7),
        ],
        _ => Vec::new(),
    }
}

/// 整条湿声链（不含补偿、干湿）。
struct Chain {
    pre: Vec<Filter<f32>>,
    shapers: [Waveshaper<f32>; 2],
    split: [f64; 2],
    bias: f32,
    dc: [DcBlocker<f32>; 2],
    inter: [Filter<f32>; 2],
    tone: Filter<f32>,
    cab: Vec<Filter<f32>>,
}

impl Chain {
    fn new(m: Mode, drive_db: f64, tone_hz: f64, low_cut_hz: f64, cab: &str) -> Self {
        let (curves, split, bias) = stages(m);
        // 过载 / 失真 / fuzz 前面推中频、提高频（吉他箱头与过载踏板的做法）；折叠用于合成器，不加。
        let pre = if m == Mode::Fold {
            vec![Filter::new(FilterShape::Highpass, low_cut_hz, 0.7), Filter::new(FilterShape::Lowpass, 10_000.0, 0.7)]
        } else {
            // 低频不进削波（过载踏板的削波级只放大 700 Hz 以上）：不然削出来的是低频方波和互调的"糊"。
            vec![
                Filter::new(FilterShape::Highpass, low_cut_hz, 0.7),
                Filter::new(FilterShape::LowShelf { gain_db: -12.0 }, 450.0, 0.7),
                Filter::new(FilterShape::Peak { gain_db: 6.0 }, 1000.0, 0.8),
                Filter::new(FilterShape::HighShelf { gain_db: 4.0 }, 2500.0, 0.7),
                Filter::new(FilterShape::Lowpass, 10_000.0, 0.7),
            ]
        };
        Self {
            pre,
            shapers: [Waveshaper::new(curves[0]).with_drive_db(drive_db * split[0]), Waveshaper::new(curves[1]).with_drive_db(drive_db * split[1])],
            split,
            bias,
            dc: [DcBlocker::new(), DcBlocker::new()],
            inter: [Filter::new(FilterShape::Highpass, 250.0, 0.7), Filter::new(FilterShape::Lowpass, 7000.0, 0.7)],
            tone: Filter::new(FilterShape::Lowpass, tone_hz, 0.7),
            cab: cabinet(cab),
        }
    }

    fn set_drive_db(&mut self, v: f64) {
        self.shapers[0].set_drive_db(v * self.split[0]);
        self.shapers[1].set_drive_db(v * self.split[1]);
    }

    fn filters(&mut self) -> impl Iterator<Item = &mut Filter<f32>> {
        self.pre.iter_mut().chain(self.inter.iter_mut()).chain(std::iter::once(&mut self.tone)).chain(self.cab.iter_mut())
    }

    fn prepare(&mut self, spec: &ProcessSpec) {
        self.filters().for_each(|f| f.prepare(spec));
        self.dc.iter_mut().for_each(|d| d.prepare(spec));
    }

    fn reset(&mut self) {
        self.filters().for_each(|f| f.reset());
        self.dc.iter_mut().for_each(|d| d.reset());
    }

    fn process(&mut self, buf: &mut AudioBuffer<f32>) {
        self.pre.iter_mut().for_each(|f| f.process(buf));
        if self.bias != 0.0 {
            buf.iter_channels_mut().for_each(|ch| ch.iter_mut().for_each(|x| *x += self.bias));
        }
        self.shapers[0].process(buf);
        self.dc[0].process(buf);
        self.inter.iter_mut().for_each(|f| f.process(buf));
        self.shapers[1].process(buf);
        self.dc[1].process(buf);
        self.tone.process(buf);
        self.cab.iter_mut().for_each(|f| f.process(buf));
    }
}

pub struct Distortion {
    mode: Mode,
    drive_db: f64,
    tone_hz: f64,
    low_cut_hz: f64,
    cabinet: String,
    output: f32,
    mix: f32,
    chain: Chain,
    dry: AudioBuffer<f32>,
    /// 驱动 0, 1, …, 48 dB 时的补偿增益（见模块说明）。
    table: Vec<f32>,
}

impl Distortion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(mode_name: &str, drive_db: f64, tone_hz: f64, low_cut_hz: f64, cabinet_name: &str, output_db: f64, mix: f64) -> Self {
        let m = mode(mode_name);
        Self {
            mode: m,
            drive_db,
            tone_hz,
            low_cut_hz,
            cabinet: cabinet_name.to_string(),
            output: db(output_db),
            mix: mix as f32,
            chain: Chain::new(m, drive_db, tone_hz, low_cut_hz, cabinet_name),
            dry: AudioBuffer::new(2, 0),
            table: Vec::new(),
        }
    }

    pub fn set_drive_db(&mut self, v: f64) {
        self.drive_db = v;
        self.chain.set_drive_db(v);
    }
    pub fn set_tone_hz(&mut self, v: f64) {
        self.tone_hz = v;
        self.chain.tone.set_frequency(v);
    }
    pub fn set_low_cut_hz(&mut self, v: f64) {
        self.low_cut_hz = v;
        self.chain.pre[0].set_frequency(v);
    }
    pub fn set_output_db(&mut self, v: f64) {
        self.output = db(v);
    }
    pub fn set_mix(&mut self, v: f64) {
        self.mix = v as f32;
    }

    /// 当前驱动的补偿增益（表里线性插值）。
    fn gain(&self) -> f32 {
        if self.table.is_empty() {
            return 1.0;
        }
        let x = self.drive_db.clamp(0.0, (self.table.len() - 1) as f64);
        let i = (x.floor() as usize).min(self.table.len() - 2);
        let t = (x - i as f64) as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * t
    }

    /// 用和弦锯齿波（E2 / B2 / E3）跑一遍整条链，量 RMS 的变化（音色、低切照当前设置）。
    fn measure(&self, spec: &ProcessSpec, drive_db: f64) -> f32 {
        let mut c = Chain::new(self.mode, drive_db, self.tone_hz, self.low_cut_hz, &self.cabinet);
        let mono = ProcessSpec { num_channels: 1, max_block_size: 2048, ..*spec };
        c.prepare(&mono);
        let sr = spec.sample_rate;
        let n = (sr * 0.2) as usize;
        let saw = |f: f64, i: usize| (2.0 * ((i as f64 * f / sr) % 1.0) - 1.0) as f32;
        let x: Vec<f32> = (0..n).map(|i| REFERENCE_AMP / 3.0 * (saw(82.41, i) + saw(123.47, i) + saw(164.81, i))).collect();
        let mut buf = AudioBuffer::<f32>::new(1, n);
        buf.channel_mut(0).copy_from_slice(&x);
        for chunk in 0..n.div_ceil(2048) {
            let (a, z) = (chunk * 2048, ((chunk + 1) * 2048).min(n));
            let mut b = AudioBuffer::<f32>::new(1, z - a);
            b.channel_mut(0).copy_from_slice(&buf.channel(0)[a..z]);
            c.process(&mut b);
            buf.channel_mut(0)[a..z].copy_from_slice(b.channel(0));
        }
        // 去掉开头（滤波器起振），量后 3/4。
        let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
        let (a, b) = (rms(&x[n / 4..]), rms(&buf.channel(0)[n / 4..]));
        if b > 1e-9 { (a / b).clamp(0.01, 8.0) } else { 1.0 }
    }
}

fn db(v: f64) -> f32 {
    10f64.powf(v / 20.0) as f32
}

impl Processor<f32> for Distortion {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.chain = Chain::new(self.mode, self.drive_db, self.tone_hz, self.low_cut_hz, &self.cabinet);
        self.chain.prepare(spec);
        self.dry = AudioBuffer::new(spec.num_channels.max(1), spec.max_block_size.max(1));
        self.table = (0..=48).map(|d| self.measure(spec, f64::from(d))).collect();
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        let (chans, n) = (buffer.num_channels(), buffer.num_frames());
        if self.dry.num_channels() < chans || self.dry.num_frames() < n {
            self.dry = AudioBuffer::new(chans, n);
        }
        for c in 0..chans {
            self.dry.channel_mut(c)[..n].copy_from_slice(&buffer.channel(c)[..n]);
        }
        self.chain.process(buffer);
        let (g, mix, out) = (self.gain(), self.mix, self.output);
        for c in 0..chans {
            let dry = &self.dry.channel(c)[..n];
            for (x, d) in buffer.channel_mut(c)[..n].iter_mut().zip(dry) {
                *x = (d + (*x * g - d) * mix) * out;
            }
        }
    }

    fn reset(&mut self) {
        self.chain.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(n: usize) -> AudioBuffer<f32> {
        let mut b = AudioBuffer::<f32>::new(2, n);
        for c in 0..2 {
            for (i, x) in b.channel_mut(c).iter_mut().enumerate() {
                let t = i as f64 / 48_000.0;
                // 带泛音的和弦（锯齿波，比正弦更像乐器）；初相各不同，不和补偿用的参考信号一模一样。
                *x = (REFERENCE_AMP as f64 / 3.0 * [82.41, 123.47, 164.81].iter().enumerate().map(|(k, f)| 2.0 * ((t * f + 0.3 * k as f64) % 1.0) - 1.0).sum::<f64>()) as f32;
            }
        }
        b
    }
    fn rms(b: &AudioBuffer<f32>, from: usize) -> f32 {
        let x = &b.channel(0)[from..];
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }
    fn centroid(b: &AudioBuffer<f32>, from: usize) -> f64 {
        let x: Vec<f64> = b.channel(0)[from..from + 16_384].iter().map(|&v| f64::from(v)).collect();
        // 粗略的 DFT（只看到 8 kHz，每 25 Hz 一格）。
        let (mut num, mut den) = (0.0, 0.0);
        for k in (25..8000).step_by(25) {
            let w = 2.0 * std::f64::consts::PI * f64::from(k) / 48_000.0;
            let (re, im) = x.iter().enumerate().fold((0.0, 0.0), |(r, i), (n, v)| (r + v * (w * n as f64).cos(), i + v * (w * n as f64).sin()));
            let m = (re * re + im * im).sqrt();
            num += f64::from(k) * m;
            den += m;
        }
        num / den
    }
    fn run(d: &mut Distortion, input: &AudioBuffer<f32>, block: usize) -> AudioBuffer<f32> {
        d.prepare(&ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 2 });
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
    }

    /// 驱动加大，响度大致不变（补偿），声音变亮（中高频的谐波）；分块方式不影响结果。
    #[test]
    fn drive_adds_bright_harmonics_not_level_and_blocks_do_not_matter() {
        let input = chord(48_000);
        let mut soft = Distortion::new("overdrive", 6.0, 6500.0, 100.0, "guitar", 0.0, 1.0);
        let mut hard = Distortion::new("distortion", 30.0, 6500.0, 100.0, "guitar", 0.0, 1.0);
        let (a, b) = (run(&mut soft, &input, 512), run(&mut hard, &input, 512));
        let (r_in, r_a, r_b) = (rms(&input, 24_000), rms(&a, 24_000), rms(&b, 24_000));
        // 约 ±3.5 dB 内（补偿是按一种和弦算的静态值，信号不同会差一些）。
        assert!((r_a / r_in - 1.0).abs() < 0.5 && (r_b / r_in - 1.0).abs() < 0.5, "{r_in} {r_a} {r_b}");
        let (c_in, c_b) = (centroid(&input, 24_000), centroid(&b, 24_000));
        assert!(c_b > 900.0 && c_in > 0.0, "失真后的重心 {c_b:.0} Hz（原来 {c_in:.0}）");
        let odd = run(&mut Distortion::new("distortion", 30.0, 6500.0, 100.0, "guitar", 0.0, 1.0), &input, 37);
        assert!(odd.channel(0) == b.channel(0), "分块方式不同，结果应逐样本相同");
    }
}
