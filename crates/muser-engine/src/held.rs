//! 拖长音的乐器（`muser-yinse-held`）：萨克斯、笛、箫、风琴、弦乐这类一直在给能量的声音。音色库里
//! `instruments/<blow|bow|…>/[<作者>/]<名>/<名>.yinse.json`，音轨写 `yinse:blow/<作者>/<名>`。
//!
//! 一个音是一组成整数倍的泛音（每个音高、几种力度各量各的：振幅和相对基频的相位）加泛音之间的噪声（气声、弓毛的摩擦，分频带）。
//! 演奏时按音高和力度在表里取泛音，一路正弦一路正弦地加起来：
//! - 力度是连续的（`dynamics`，不给就用音符的力度）：在两层之间过渡，轻的暗、重的亮，一个音里面也可以变；
//! - 稳定段不是死的：泛音的音量、音高都带一点慢的随机起伏（表里量到的大小）；
//! - 起音：泛音从低到高先后进来，带一下气声；收音：很快地退掉；
//! - 独奏乐器（`mono`）：前一个音还按着就来了下一个，是连奏——不重新起音，音高滑过去、音色跟着换；
//! - 揉音：`vibrato_depth` / `vibrato_rate` / `vibrato_delay`。
//!
//! 和弹拨、打击的区别：那两种是敲一下自己衰减，量的是每个分音的起始音量和衰减；这里分音不衰减，音量由"吹多大劲"决定。

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::Mutex;

use manguebyte::midi::MidiMessage;
use manguebyte::prelude::{AudioBuffer, NoteRef, ProcessSpec};
use serde::{Deserialize, Serialize};

use crate::instrument::Playable;
use crate::percussion::{biquad, butterworth};

pub const FORMAT: &str = "muser-yinse-held";
/// 这个版本的 muser 读得懂的最高版本。
pub const VERSION: u32 = 1;
/// 一个音最多几个泛音。
pub const MAX_PARTIALS: usize = 128;
const CONTROL: usize = 32;

/// 一件拖长音的乐器。`format` 必须是第一个键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldTable {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// 输出端补多少 dB，参考乐句才是 -20 LUFS。
    #[serde(default)]
    pub level_db: f64,
    /// 独奏乐器：一次一个音，连着的音是连奏。
    #[serde(default)]
    pub mono: bool,
    /// 起音、收音的时间常数（秒）。
    pub attack_s: f64,
    pub release_s: f64,
    /// 连奏时音高滑到下一个音的时间常数（秒）：管乐换指几乎是立刻的（几毫秒），弦乐的手指是滑过去的（几十毫秒）。
    #[serde(default = "glide_s")]
    pub glide_s: f64,
    /// 起音的那一下比稳定以后响多少 dB（吐音的那一下重音），几十毫秒里退回去。
    #[serde(default)]
    pub attack_db: f64,
    /// 稳定段的起伏。
    #[serde(default)]
    pub wobble: Wobble,
    /// 噪声的频带（Hz），比 `noise` 多一个数。
    pub bands: Vec<f64>,
    /// 量过的音，由低到高。
    pub notes: Vec<HeldNote>,
}

/// 稳定段的起伏：泛音的音量上下浮动多少（比例）、音高浮动多少音分、多快（Hz）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wobble {
    pub depth: f64,
    pub cents: f64,
    pub rate_hz: f64,
}

fn glide_s() -> f64 {
    0.005
}

impl Default for Wobble {
    fn default() -> Self {
        Self { depth: 0.05, cents: 2.0, rate_hz: 4.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldNote {
    pub pitch: f64,
    /// 量到的基频（Hz）。
    pub hz: f64,
    /// 几种力度，由轻到重。
    pub layers: Vec<HeldLayer>,
}

/// 一种力度：在力度 `at` 量的各次泛音的振幅、相对基频的相位（弧度，`sin(nθ + φ)`，基频是 0），各频带噪声的均方根。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldLayer {
    pub at: f64,
    pub partials: Vec<f64>,
    #[serde(default)]
    pub phases: Vec<f64>,
    #[serde(default)]
    pub noise: Vec<f64>,
}

/// 可调的参数（`instrument_params`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeldParams {
    /// 吹多大劲（0 … 1）；负的是跟着音符的力度。
    pub dynamics: f64,
    pub vibrato_depth: f64,
    pub vibrato_rate: f64,
    pub vibrato_delay: f64,
    /// 起音、收音是表里的几倍。
    pub attack: f64,
    pub release: f64,
    /// 连奏时音高滑过去用多久（秒）。
    pub glide: f64,
    /// 气声是表里的几倍。
    pub breath: f64,
    /// 稳定段的起伏是表里的几倍。
    pub wobble: f64,
}

impl HeldParams {
    /// 这件乐器的默认设定（连奏滑多快是乐器自己的）。
    pub fn of(table: &HeldTable) -> Self {
        Self { dynamics: -1.0, vibrato_depth: 0.0, vibrato_rate: 5.0, vibrato_delay: 0.25, attack: 1.0, release: 1.0, glide: table.glide_s, breath: 1.0, wobble: 1.0 }
    }
}

type Cached = (std::time::SystemTime, &'static HeldTable);
static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// `yinse:` 后面的路径是不是一件拖长音的乐器（只看文件头）。
pub fn is_held(path: &str) -> bool {
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = std::fs::File::open(crate::modal::resolve(path)).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains(FORMAT)
}

/// 检查一张表。
pub fn check(table: &HeldTable, origin: &str) -> Result<(), String> {
    if table.format != FORMAT {
        return Err(format!("'{origin}' is not a yinse held instrument (format {FORMAT})"));
    }
    if table.version > VERSION {
        return Err(format!("'{origin}' was written by a newer muser (table version {}, this muser reads up to {VERSION}): update muser", table.version));
    }
    if table.notes.is_empty() || table.notes.windows(2).any(|w| w[1].pitch <= w[0].pitch) {
        return Err(format!("'{origin}': needs one or more notes with rising pitches"));
    }
    for n in &table.notes {
        if n.layers.is_empty() || n.layers.windows(2).any(|w| w[1].at <= w[0].at) || n.layers.iter().any(|l| l.partials.is_empty() || !(l.at > 0.0 && l.at <= 1.0)) {
            return Err(format!("'{origin}': pitch {}: needs one or more dynamic layers with rising `at` in 0 … 1, each with partials", n.pitch));
        }
        if !(n.hz > 0.0) || n.layers.iter().flat_map(|l| l.partials.iter().chain(&l.noise).chain(&l.phases)).any(|v| !v.is_finite()) {
            return Err(format!("'{origin}': pitch {}: bad numbers", n.pitch));
        }
        if n.layers.iter().any(|l| !l.noise.is_empty() && l.noise.len() + 1 != table.bands.len()) {
            return Err(format!("'{origin}': pitch {}: `noise` must have one level per band (`bands` has one more number)", n.pitch));
        }
    }
    Ok(())
}

/// 读一件（`yinse:` 后面的路径），同一文件没改过就不再读。
pub fn load(path: &str) -> Result<&'static HeldTable, String> {
    let path = crate::modal::resolve(path);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|e| format!("cannot read yinse instrument '{}': {e}", path.display()))?;
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(&(t, table)) = cache.get(&path) {
        if t == modified {
            return Ok(table);
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read yinse instrument '{}': {e}", path.display()))?;
    let table: HeldTable = serde_json::from_str(&text).map_err(|e| format!("'{}' is not a yinse held instrument: {e}", path.display()))?;
    check(&table, &path.display().to_string())?;
    let table: &'static HeldTable = Box::leak(Box::new(table));
    cache.insert(path, (modified, table));
    Ok(table)
}

impl HeldTable {
    pub fn range(&self) -> (f64, f64) {
        (self.notes[0].pitch, self.notes[self.notes.len() - 1].pitch)
    }

    /// 一个量过的音在力度 `level` 上的泛音振幅、噪声（写进 `partials` / `noise`，不够的补零）：两层之间按振幅连直线，
    /// 比最轻的一层还轻照它小下去，比最重的还重照它大上去（最多 3 dB）。
    fn layer_at(&self, note: &HeldNote, level: f64, partials: &mut [f64; MAX_PARTIALS], noise: &mut [f64]) {
        let l = &note.layers;
        let above = l.iter().position(|x| x.at >= level).unwrap_or(l.len());
        let (a, b, x, scale) = if above == 0 {
            (0, 0, 0.0, (level / l[0].at).max(0.0).powf(1.2))
        } else if above == l.len() {
            (above - 1, above - 1, 0.0, (level / l[above - 1].at).powf(1.2).min(10f64.powf(3.0 / 20.0)))
        } else {
            (above - 1, above, (level - l[above - 1].at) / (l[above].at - l[above - 1].at), 1.0)
        };
        let get = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
        for (i, p) in partials.iter_mut().enumerate() {
            *p = scale * (get(&l[a].partials, i) * (1.0 - x) + get(&l[b].partials, i) * x);
        }
        for (i, n) in noise.iter_mut().enumerate() {
            *n = scale * (get(&l[a].noise, i) * (1.0 - x) + get(&l[b].noise, i) * x);
        }
    }

    /// 音高 `pitch`、力度 `level` 上的泛音振幅和噪声：相邻两个量过的音之间按泛音的次数连直线，范围以外用最近的那个音。
    fn timbre(&self, pitch: f64, level: f64, partials: &mut [f64; MAX_PARTIALS], noise: &mut [f64]) {
        let n = &self.notes;
        let above = n.iter().position(|x| x.pitch >= pitch).unwrap_or(n.len());
        if above == 0 || above == n.len() || (n[above].pitch - pitch).abs() < 1e-6 {
            return self.layer_at(&n[above.min(n.len() - 1)], level, partials, noise);
        }
        let x = (pitch - n[above - 1].pitch) / (n[above].pitch - n[above - 1].pitch);
        let mut other = [0.0; MAX_PARTIALS];
        let mut other_noise = [0.0; 24];
        let bands = noise.len().min(24);
        self.layer_at(&n[above - 1], level, partials, noise);
        self.layer_at(&n[above], level, &mut other, &mut other_noise[..bands]);
        partials.iter_mut().zip(&other).for_each(|(a, b)| *a = *a * (1.0 - x) + b * x);
        noise.iter_mut().zip(&other_noise).for_each(|(a, b)| *a = *a * (1.0 - x) + b * x);
    }

    /// 起音时各次泛音的相位：离这个音高最近的音、最接近这个力度的那一层量到的。
    fn phases(&self, pitch: f64, level: f64) -> &[f64] {
        let note = self.notes.iter().min_by(|a, b| (a.pitch - pitch).abs().total_cmp(&(b.pitch - pitch).abs())).expect("至少一个音");
        &note.layers.iter().min_by(|a, b| (a.at - level).abs().total_cmp(&(b.at - level).abs())).expect("至少一层").phases
    }
}

fn band_filter(lo: f64, hi: f64, sr: f64) -> [[f64; 5]; 4] {
    [butterworth(true, lo, 0.541_196, sr), butterworth(true, lo, 1.306_563, sr), butterworth(false, hi, 0.541_196, sr), butterworth(false, hi, 1.306_563, sr)]
}

/// 方差 1 的白噪声过这个频带以后均方根的倒数。
fn band_unit(lo: f64, hi: f64, sr: f64) -> f64 {
    let (coef, mut state, mut rng, mut sum) = (band_filter(lo, hi, sr), [[0.0; 2]; 4], 0x9E37_79B9_7F4A_7C15u64, 0.0);
    let n = 24_000;
    for i in 0..n {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let w = ((rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * 3f64.sqrt();
        let y = coef.iter().zip(&mut state).fold(w, |y, (c, z)| biquad(c, z, y));
        if i >= 4_000 {
            sum += y * y;
        }
    }
    1.0 / (sum / (n - 4_000) as f64).sqrt().max(1e-9)
}

/// 一路噪声频带的滤波器（四阶高通 + 四阶低通）与状态。
struct NoiseBand {
    coef: [[f64; 5]; 4],
    state: [[f64; 2]; 4],
    /// 方差 1 的白噪声过了这个频带只剩带宽那么多：乘上它，`level` 就是这个频带出来的均方根。
    unit: f64,
    level: f64,
    step: f64,
}

struct Voice {
    key: Option<u8>,
    /// 还按着（没松开）。
    held: bool,
    /// 这个音开始了多少帧（起音的那一段、揉音的延迟用）。
    age: u64,
    /// 现在的音高（MIDI 音高，带小数）与要去的音高。
    pitch: f64,
    target: f64,
    /// 现在的力度与音符给的力度。
    level: f64,
    velocity: f64,
    /// 整体的开合（起音、收音）：0 … 1。
    gate: f64,
    /// 各次泛音的转子（单位长度的复数，每帧转一个角度）、现在的振幅、每帧加多少。
    re: [f64; MAX_PARTIALS],
    im: [f64; MAX_PARTIALS],
    amp: [f64; MAX_PARTIALS],
    step: [f64; MAX_PARTIALS],
    rot: [(f64, f64); MAX_PARTIALS],
    count: usize,
    noise: Vec<NoiseBand>,
    rng: u64,
    /// 慢的随机起伏：音高一路、泛音四组各一路（低通过的噪声）。
    drift: [f64; 5],
    smooth: [[f64; 3]; 5],
    vibrato_phase: f64,
    /// 这一块还剩几帧才到下一次控制。
    until: usize,
}

impl Voice {
    fn new(bands: &[f64], units: &[f64], sr: f64, seed: u64) -> Self {
        let noise = bands
            .windows(2)
            .zip(units)
            .map(|(e, &unit)| NoiseBand { coef: band_filter(e[0], e[1], sr), state: [[0.0; 2]; 4], unit, level: 0.0, step: 0.0 })
            .collect();
        Self {
            key: None,
            held: false,
            age: 0,
            pitch: 60.0,
            target: 60.0,
            level: 0.0,
            velocity: 0.0,
            gate: 0.0,
            re: [1.0; MAX_PARTIALS],
            im: [0.0; MAX_PARTIALS],
            amp: [0.0; MAX_PARTIALS],
            step: [0.0; MAX_PARTIALS],
            rot: [(1.0, 0.0); MAX_PARTIALS],
            count: 0,
            noise,
            rng: 0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03),
            drift: [0.0; 5],
            smooth: [[0.0; 3]; 5],
            vibrato_phase: 0.0,
            until: 0,
        }
    }

    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }

    fn sounding(&self) -> bool {
        self.key.is_some()
    }
}

/// 一件拖长音的乐器的发声。
pub(crate) struct Held {
    table: &'static HeldTable,
    params: HeldParams,
    voices: Vec<Voice>,
    sr: f64,
    /// 弯音（半音）。
    bend: f64,
    order: u64,
    started: Vec<u64>,
}

impl Held {
    pub fn new(table: &'static HeldTable, params: HeldParams) -> Self {
        Self { table, params, voices: Vec::new(), sr: 48_000.0, bend: 0.0, order: 0, started: Vec::new() }
    }

    pub fn voices(table: &HeldTable) -> usize {
        if table.mono {
            1
        } else {
            12
        }
    }

    /// 一个控制块（[`CONTROL`] 帧）开头：音高、力度、起伏往前走一步，定出这一块里各次泛音、各带噪声的振幅从哪儿到哪儿。
    #[allow(clippy::needless_range_loop)] // 同一个泛音的序号用在好几个数组上
    fn control(table: &HeldTable, p: &HeldParams, v: &mut Voice, sr: f64, bend: f64) {
        let dt = CONTROL as f64 / sr;
        let seconds = v.age as f64 / sr;
        // 连奏：音高按指数滑到新的音上。
        v.pitch += (v.target - v.pitch) * (1.0 - (-dt / p.glide.max(0.001)).exp());
        // 力度：`dynamics` 给了就跟它（一个音里面也能变），没给跟音符的力度。
        let want = if p.dynamics >= 0.0 { p.dynamics.min(1.0) } else { v.velocity };
        v.level += (want - v.level) * (1.0 - (-dt / 0.03).exp());
        // 慢的随机起伏：白噪声过三级低通（大约 `rate_hz` 以下；只过一级的话上面漏出来的会在每个泛音两边调出一圈底噪）。
        let k = 1.0 - (-dt * TAU * table.wobble.rate_hz).exp();
        for i in 0..v.drift.len() {
            // 低通以后方差小了：乘回来，让最后一级的均方根大致是 1。
            let mut x = v.random() * 0.577 * (2.0 / k).sqrt() * 1.9;
            for stage in &mut v.smooth[i] {
                *stage += (x - *stage) * k;
                x = *stage;
            }
            v.drift[i] = x;
        }
        // 揉音：过了延迟以后慢慢加深。
        v.vibrato_phase = (v.vibrato_phase + dt * p.vibrato_rate) % 1.0;
        let depth = p.vibrato_depth * ((seconds - p.vibrato_delay) / 0.3).clamp(0.0, 1.0);
        let cents = depth * (TAU * v.vibrato_phase).sin() + table.wobble.cents * p.wobble * v.drift[0];
        let hz = 440.0 * ((v.pitch + bend - 69.0) / 12.0 + cents / 1200.0).exp2();
        // 这一刻的音色。
        let mut partials = [0.0; MAX_PARTIALS];
        let mut noise = [0.0; 24];
        let bands = v.noise.len().min(24);
        table.timbre(v.pitch, v.level, &mut partials, &mut noise[..bands]);
        // 开合：起音时往 1 去，松开以后往 0 去。
        let (attack, release) = (table.attack_s * p.attack, table.release_s * p.release);
        let open = if v.held { 1.0 } else { 0.0 };
        v.gate += (open - v.gate) * (1.0 - (-dt / if v.held { attack.max(0.001) } else { release.max(0.001) }).exp());
        // 吐音的那一下重音：起音时响一点，六十毫秒上下退回去。
        let accent = 10f64.powf(table.attack_db / 20.0 * (-seconds / 0.06).exp());
        v.count = 0;
        for n in 0..MAX_PARTIALS {
            let f = hz * (n + 1) as f64;
            if f > 0.45 * sr {
                partials[n] = 0.0;
            } else {
                v.count = n + 1;
            }
            let w = TAU * f / sr;
            v.rot[n] = (w.cos(), w.sin());
            // 起音：高的泛音进来得晚一点（每高一个八度晚 0.6 个起音时间）；之后跟着力度和起伏走。
            let late = (1.0 - (-(seconds / (attack * (1.0 + 0.6 * ((n + 1) as f64).log2())).max(0.001))).exp()).min(1.0);
            let shimmer = 1.0 + table.wobble.depth * p.wobble * v.drift[1 + n % 4];
            let target = partials[n] * shimmer.max(0.0) * v.gate * accent * if v.held { late } else { 1.0 };
            v.step[n] = (target - v.amp[n]) / CONTROL as f64;
        }
        // 气声：起音的头几十毫秒多一口。
        let puff = 1.0 + 2.0 * (-seconds / 0.04).exp();
        for (b, level) in v.noise.iter_mut().zip(&noise) {
            b.step = (level * p.breath * v.gate * puff - b.level) / CONTROL as f64;
        }
        // 转子转久了长度会漂：拉回 1。
        for n in 0..v.count {
            let m = (v.re[n] * v.re[n] + v.im[n] * v.im[n]).sqrt().max(1e-12);
            v.re[n] /= m;
            v.im[n] /= m;
        }
    }
}

impl Playable for Held {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.sr = spec.sample_rate;
        let n = Self::voices(self.table);
        let units: Vec<f64> = self.table.bands.windows(2).map(|e| band_unit(e[0], e[1], self.sr)).collect();
        self.voices = (0..n).map(|i| Voice::new(&self.table.bands, &units, self.sr, i as u64 + 1)).collect();
        self.started = vec![0; n];
    }

    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let velocity = (f64::from(velocity) * if accent { 1.2 } else { 1.0 }).clamp(0.0, 1.0);
        let key = f64::from(note.key);
        self.order += 1;
        // 独奏乐器：前一个音还按着，是连奏——不重新起音，滑过去。
        if self.table.mono && self.voices[0].sounding() && self.voices[0].held {
            let v = &mut self.voices[0];
            v.key = Some(note.key);
            v.target = key;
            v.velocity = velocity;
            return;
        }
        // 找一路空的；没有就抢最早开始的。
        let i = self.voices.iter().position(|v| !v.sounding()).unwrap_or_else(|| (0..self.voices.len()).min_by_key(|&i| self.started[i]).unwrap_or(0));
        self.started[i] = self.order;
        let (table, dynamics) = (self.table, self.params.dynamics);
        let v = &mut self.voices[i];
        // 这一路还在收上一个音的尾巴：从它现在的声音接着起（不咔嗒）；空着的从头起。
        let fresh = !v.sounding();
        v.key = Some(note.key);
        v.held = true;
        v.age = 0;
        v.target = key;
        v.pitch = key;
        v.velocity = velocity;
        v.level = if dynamics >= 0.0 { dynamics.min(1.0) } else { velocity };
        v.until = 0;
        if fresh {
            v.gate = 0.0;
            v.amp = [0.0; MAX_PARTIALS];
            v.noise.iter_mut().for_each(|b| b.level = 0.0);
            // 各次泛音按量到的相位起（波形的样子：尖还是圆，是相位定的）。
            let phases = table.phases(key, v.level);
            for n in 0..MAX_PARTIALS {
                let ph = phases.get(n).copied().unwrap_or(0.0);
                (v.re[n], v.im[n]) = (ph.cos(), ph.sin());
            }
        }
    }

    fn note_off(&mut self, note: NoteRef, _velocity: f32) {
        for v in self.voices.iter_mut().filter(|v| v.key == Some(note.key)) {
            v.held = false;
        }
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        buffer.clear();
        let (frames, channels) = (buffer.num_frames(), buffer.num_channels());
        let (table, params, sr, bend) = (self.table, self.params, self.sr, self.bend);
        for v in self.voices.iter_mut().filter(|v| v.sounding()) {
            for i in 0..frames {
                if v.until == 0 {
                    Self::control(table, &params, v, sr, bend);
                    v.until = CONTROL;
                }
                v.until -= 1;
                v.age += 1;
                let mut x = 0.0;
                for n in 0..v.count {
                    let (c, s) = v.rot[n];
                    let re = v.re[n] * c - v.im[n] * s;
                    v.im[n] = v.re[n] * s + v.im[n] * c;
                    v.re[n] = re;
                    v.amp[n] += v.step[n];
                    x += v.amp[n] * v.im[n];
                }
                // 噪声：一路白噪声分到各个频带（方差 1 的均匀分布）。
                let w = v.random() * 3f64.sqrt();
                for b in &mut v.noise {
                    b.level += b.step;
                    if b.level > 1e-7 {
                        let y = b.coef.iter().zip(&mut b.state).fold(w, |y, (c, z)| biquad(c, z, y));
                        x += y * b.level * b.unit;
                    }
                }
                // 两个声道一样（居中）：表里的振幅就是录音在每个声道上的振幅，放在音轨上和录音一样响。
                let y = x as f32;
                for c in 0..channels {
                    buffer.channel_mut(c)[i] += y;
                }
            }
            // 松开以后退干净了：这一路空出来。
            if !v.held && v.gate < 1e-4 {
                v.key = None;
                v.amp = [0.0; MAX_PARTIALS];
            }
        }
    }

    fn midi(&mut self, message: MidiMessage) {
        if let MidiMessage::PitchBend { value, .. } = message {
            self.bend = f64::from(value) / 8192.0 * 2.0;
        }
    }

    fn set_param(&mut self, name: &str, value: f64) {
        crate::params::set_number(crate::params::HELD, &mut self.params, name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个音高、轻重两层：重的一层多出高处的泛音。
    const FIXTURE: &str = r#"{"format":"muser-yinse-held","version":1,"name":"test","mono":true,"attack_s":0.02,"release_s":0.04,
      "wobble":{"depth":0.05,"cents":2,"rate_hz":4},"bands":[2000,4000,8000],
      "notes":[
        {"pitch":48,"hz":130.81,"layers":[{"at":0.3,"partials":[0.05,0.02],"noise":[0.0005,0.0002]},{"at":1.0,"partials":[0.2,0.15,0.1,0.08,0.06],"noise":[0.002,0.001]}]},
        {"pitch":60,"hz":261.63,"layers":[{"at":0.3,"partials":[0.05,0.01],"noise":[0.0005,0.0002]},{"at":1.0,"partials":[0.2,0.1,0.08,0.05],"noise":[0.002,0.001]}]}]}"#;

    fn table() -> &'static HeldTable {
        let t: HeldTable = serde_json::from_str(FIXTURE).unwrap();
        check(&t, "fixture").unwrap();
        Box::leak(Box::new(t))
    }

    /// (什么时候, 键, 力度；力度 0 是松开)。
    fn play(params: HeldParams, events: &[(f64, u8, f32)], seconds: f64) -> Vec<f32> {
        let mut h = Held::new(table(), params);
        h.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 480 });
        let mut out = Vec::new();
        let mut b = AudioBuffer::<f32>::new(1, 480);
        for block in 0..(seconds * 100.0) as usize {
            for &(_, key, v) in events.iter().filter(|e| (e.0 * 100.0).round() as usize == block) {
                if v > 0.0 {
                    h.note_on(NoteRef::new(0, key), v, false);
                } else {
                    h.note_off(NoteRef::new(0, key), 0.0);
                }
            }
            h.process(&mut b);
            out.extend_from_slice(b.channel(0));
        }
        out
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// `hz` 这个频率在 `x` 里有多少。
    fn tone(x: &[f32], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let p = TAU * hz * i as f64 / 48_000.0;
            re += f64::from(v) * p.cos();
            im += f64::from(v) * p.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn a_note_holds_steady_and_stops_after_release() {
        let x = play(HeldParams::of(table()), &[(0.0, 48, 1.0), (1.0, 48, 0.0)], 1.5);
        let (early, late) = (rms(&x[9_600..19_200]), rms(&x[28_800..43_200]));
        assert!(early > 0.05 && (late / early - 1.0).abs() < 0.15, "held: {early} then {late}");
        // 量到的振幅：一次泛音 0.2。
        assert!((tone(&x[14_400..43_200], 130.81) / 0.2 - 1.0).abs() < 0.1);
        assert!(x[x.len() - 4_800..].iter().all(|v| v.abs() < 1e-4), "gone half a second after release");
    }

    #[test]
    fn louder_is_brighter_and_dynamics_can_move_within_a_note() {
        let (soft, loud) = (play(HeldParams::of(table()), &[(0.0, 48, 0.3)], 0.6), play(HeldParams::of(table()), &[(0.0, 48, 1.0)], 0.6));
        let bright = |x: &[f32]| tone(&x[9_600..], 5.0 * 130.81) / tone(&x[9_600..], 130.81);
        assert!(bright(&soft) < 0.02 && bright(&loud) > 0.2, "soft {} loud {}", bright(&soft), bright(&loud));
        // `dynamics` 给了就不看音符的力度。
        let forced = play(HeldParams { dynamics: 1.0, ..HeldParams::of(table()) }, &[(0.0, 48, 0.3)], 0.6);
        assert!((rms(&forced[9_600..]) / rms(&loud[9_600..]) - 1.0).abs() < 0.1);
    }

    #[test]
    fn overlapping_notes_on_a_solo_instrument_are_slurred() {
        // 第二个音在第一个松开之前来：不重新起音（中间没有掉下去），音高到了新的音上，第一个音的松开不管用。
        let x = play(HeldParams::of(table()), &[(0.0, 48, 1.0), (0.5, 60, 1.0), (0.55, 48, 0.0)], 1.0);
        let around: Vec<f64> = (0..10).map(|k| rms(&x[23_520 + k * 240..23_520 + (k + 1) * 240])).collect();
        let steady = rms(&x[9_600..19_200]);
        assert!(around.iter().all(|v| *v > 0.5 * steady), "no gap at the slur: {around:?} vs {steady}");
        assert!(tone(&x[33_600..43_200], 261.63) > 5.0 * tone(&x[33_600..43_200], 130.81));
        assert!(rms(&x[43_200..]) > 0.5 * steady, "still sounding: the first note's release is ignored");
        // 分开的两个音：中间退掉过。
        let y = play(HeldParams::of(table()), &[(0.0, 48, 1.0), (0.3, 48, 0.0), (0.6, 60, 1.0)], 1.0);
        assert!(rms(&y[26_400..28_320]) < 0.02 * steady);
    }

    #[test]
    fn vibrato_moves_the_pitch_after_its_delay() {
        let p = HeldParams { vibrato_depth: 50.0, vibrato_rate: 5.0, vibrato_delay: 0.2, wobble: 0.0, ..HeldParams::of(table()) };
        let x = play(p, &[(0.0, 60, 1.0)], 1.5);
        // 过零点的间隔：没揉的时候一样长，揉起来以后时长时短。
        let periods = |x: &[f32]| {
            let z: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0).collect();
            let d: Vec<f64> = z.windows(2).map(|w| (w[1] - w[0]) as f64).filter(|d| *d > 100.0).collect();
            let m = d.iter().sum::<f64>() / d.len() as f64;
            (d.iter().map(|v| (v - m).powi(2)).sum::<f64>() / d.len() as f64).sqrt() / m
        };
        assert!(periods(&x[2_400..8_640]) < 0.01 && periods(&x[38_400..]) > 0.012, "{} then {}", periods(&x[2_400..8_640]), periods(&x[38_400..]));
    }

    #[test]
    fn bad_tables_are_refused() {
        let bad = |edit: &dyn Fn(&mut HeldTable)| {
            let mut t: HeldTable = serde_json::from_str(FIXTURE).unwrap();
            edit(&mut t);
            check(&t, "bad").err().unwrap_or_default()
        };
        assert!(bad(&|t| t.version = VERSION + 1).contains("newer muser"));
        assert!(bad(&|t| t.notes[1].pitch = 40.0).contains("rising pitches"));
        assert!(bad(&|t| t.notes[0].layers[1].at = 0.2).contains("rising `at`"));
        assert!(bad(&|t| t.notes[0].layers[0].noise.push(0.1)).contains("one level per band"));
    }
}
