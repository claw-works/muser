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
use std::f64::consts::{PI, TAU};
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
/// 起音以后这些时刻（秒）上的响度记在 [`HeldTable::onset`] 里。
pub(crate) const ONSET_AT: [f64; 16] = [0.03, 0.06, 0.09, 0.12, 0.16, 0.2, 0.25, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0, 1.3, 1.6, 2.0];
/// 量起音那一口气的时刻（秒）。
pub(crate) const PUFF_AT: [f64; 10] = [0.015, 0.03, 0.045, 0.06, 0.09, 0.12, 0.16, 0.2, 0.25, 0.3];
/// 头几次泛音带"管子里的气"（见 [`HeldLayer::air`]）。
pub(crate) const AIR: usize = 32;
/// 那一圈噪声的样子：离泛音 [`AIR_HZ`] 以外按距离的平方掉下去（长笛的录音里十几到一百多赫兹都是这样）；
/// [`AIR_SLOW_HZ`] 以下的不算在里面。
pub(crate) const AIR_HZ: f64 = 5.0;
/// 噪声一阵一阵的快慢（Hz）。
const GRAIN_HZ: f64 = 30.0;
const AIR_SLOW_HZ: f64 = 3.0;

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
    /// 起音的那一下重音多久退掉（时间常数，秒）。
    #[serde(default = "attack_len")]
    pub attack_len_s: f64,
    /// 一个音起来以后头两秒的响度怎么走（dB，相对稳定以后；[`ONSET_AT`] 那些时刻上，起音的那一下爬升不算在里面）：
    /// 头一弓重重地下去、松一下、再慢慢推上来……各个录音的中位数。有它就不用 `attack_db` 那个简单的重音了。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub onset: Vec<f64>,
    /// 起音那一段的音色（[`HeldLayer::early`]）多久退回稳定的音色（时间常数，秒）。
    #[serde(default = "early_s")]
    pub early_s: f64,
    /// 起音时高次泛音比基音晚到多少：每高一个八度多几个起音时间（管乐 0.6 上下；弓擦弦是高处先到，0）。
    #[serde(default = "attack_late")]
    pub attack_late: f64,
    /// 起音的重音在高处比低处多突出多少（dB / 八度）：弓头那一下亮而短的擦弦声。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attack_tilt_db: f64,
    /// 重音退之前先保持多久（秒）。震音的头一弓是长长的一弓，这么久以后才抖起来。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attack_hold_s: f64,
    /// 这件乐器平常揉音多深（音分）、多快（Hz）、起音后多久才揉起来（秒）：`vibrato_*` 参数的默认值。
    /// 量的时候录的是不揉的长音，这三个数是另外给的（没给就不揉）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vibrato_cents: f64,
    #[serde(default = "vibrato_hz")]
    pub vibrato_hz: f64,
    #[serde(default = "vibrato_delay")]
    pub vibrato_delay_s: f64,
    /// 过了延迟以后多少秒揉到头，刚过延迟时已经揉到几成（录音里的揉音常常一起音就在揉）。
    #[serde(default = "vibrato_ramp")]
    pub vibrato_ramp_s: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vibrato_start: f64,
    /// 稳定段的起伏。
    #[serde(default)]
    pub wobble: Wobble,
    /// 一下一下的奏法（跳弓、断奏）：每个音不管谱上写多长，都只响这么久（秒）就收。长音的乐器没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_s: Option<f64>,
    /// 震音的乐器（弓飞快地来回拉）：每个音都这样拉——一秒几弓、谷底多深、各弓长短和轻重差多少，量自震音的录音。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<Tremolo>,
    /// 合奏（一个声部好几个人拉）：每个音由几个人各拉各的，音高互相差多少音分（标准差）。独奏没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensemble: Option<Ensemble>,
    /// 噪声的频带（Hz），比 `noise` 多一个数。
    pub bands: Vec<f64>,
    /// 量过的音，由低到高。
    pub notes: Vec<HeldNote>,
}

/// 震音：一秒几弓；换弓时音量掉下去多少（0 不掉 … 1 掉到底）；相邻两弓的间隔、各弓的轻重各起伏多少（标准差 / 平均）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Tremolo {
    pub rate_hz: f64,
    pub depth: f64,
    pub uneven: f64,
    pub weight: f64,
}

/// 合奏：几个人、音高互相差多少音分。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Ensemble {
    pub voices: usize,
    pub cents: f64,
}

/// 稳定段的起伏：泛音的音量上下浮动多少（比例）、音高浮动多少音分、多快（Hz）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wobble {
    pub depth: f64,
    pub cents: f64,
    pub rate_hz: f64,
    /// 揉着的时候各次泛音另外的闪动（比例，按默认的揉音深度）：录音的屋子里，直达声和晚到的反射声音高差一点，
    /// 互相拍，每个泛音忽大忽小、各不相干，比揉音快一两倍。不揉就没有。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vibrato: f64,
}

fn early_s() -> f64 {
    0.6
}
/// 力度 `level` 在两层（`lo`、`hi`）之间的哪儿（0 … 1）：按力度的对数算。响度大致跟着力度的平方走（采样器也是这么放一层里的采样的），
/// 按对数算，两层之间的音量才是这个走法；按力度本身连直线的话，隔得远的两层中间会轻五六 dB。
fn between(lo: f64, hi: f64, level: f64) -> f64 {
    if lo <= 0.0 || hi <= lo {
        return 0.0;
    }
    ((level.max(1e-6) / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

fn attack_late() -> f64 {
    0.6
}
fn attack_len() -> f64 {
    0.06
}
fn glide_s() -> f64 {
    0.005
}
fn vibrato_hz() -> f64 {
    5.0
}
fn vibrato_ramp() -> f64 {
    0.3
}

fn vibrato_delay() -> f64 {
    0.25
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Default for Wobble {
    fn default() -> Self {
        Self { depth: 0.05, cents: 2.0, rate_hz: 4.0, vibrato: 0.0 }
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
    /// 管子里的气：每个泛音两边贴着它的那一圈噪声（气流不稳，泛音的大小和相位跟着乱动），
    /// 是泛音振幅的多少（均方根）。和 `noise`（泛音之间平平的那层"嘶"）不是一回事：这一圈带着音高，听着是"吹管子"。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub air: f64,
    /// 揉音带着音色变：头几次泛音的音量跟着揉音怎么变，每个泛音四个数——cos φ、sin φ、cos 2φ、sin 2φ 的系数（dB），
    /// φ 是揉音的相位（音高 = 深度 · sin φ），按这件乐器默认的揉音深度算。琴身的共鸣固定在频率上，泛音揉上去揉下来扫过它们。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sway: Vec<f64>,
    /// 这个录音自己揉得多深（音分）：重的、高的音常常揉得深些。演奏时的揉音深度按它和整件乐器默认值的比例走。没量到是 0。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vibrato: f64,
    /// 立体声的录音：各次泛音在左声道上是两个声道均方根的几倍（0 … √2，右声道是剩下的），右声道的相位比左声道多转多少（弧度）。
    /// 两支话筒听到的同一个泛音大小、相位都不一样——这就是录音里的"宽"和"厚"。单声道的录音没有，出来两边一样。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn: Vec<f64>,
    /// 一个音刚起来时各次泛音比稳定以后多出（少了）多少 dB：弓刚咬住弦、气刚吹进去的那一段音色。[`HeldTable::early_s`] 秒里退掉。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub early: Vec<f64>,
    /// 噪声跟着音高一下一下出（弓每个周期擦一下弦，簧片每个周期开合一次）：一个周期里它的大小起伏多少（0 匀的 … 1），
    /// 最响的那一刻在基频（正弦）的哪个相位上（弧度）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pulse: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pulse_at: f64,
    /// 噪声一阵一阵出（弓的噼啪）：增益的对数的标准差。吹气是匀的（0）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub grain: f64,
    /// 起音那一下泛音之间多出来的东西（气声、簧片没稳住的"咯"、弓毛的擦声）：各个频带最多的时候的振幅（和 `noise` 一个单位），
    /// 和它在 [`PUFF_AT`] 那些时刻是最多时的几成。没量到就用一口短的气。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub puff: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub puff_shape: Vec<f64>,
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
    /// 写了 `slide` 的音滑过去用多久（秒，时间常数）。
    pub slide: f64,
    /// 花舌（写了 `flutter` 的音）把声音截掉多少（0 … 1）、每秒几下（管乐的花舌 40 上下；拉弦的震音——弓来回快拉——十几下）。
    pub flutter: f64,
    pub flutter_rate: f64,
    /// 管子里的气是表里的几倍。
    pub air: f64,
    /// 独奏（1：一次一个音，叠着的音是连奏）还是能同时发几个音（0：几个人一起吹的和弦）。默认看乐器。
    pub solo: f64,
}

impl HeldParams {
    /// 这件乐器的默认设定（连奏滑多快是乐器自己的）。
    pub fn of(table: &HeldTable) -> Self {
        Self {
            dynamics: -1.0,
            vibrato_depth: table.vibrato_cents,
            vibrato_rate: table.vibrato_hz,
            vibrato_delay: table.vibrato_delay_s,
            attack: 1.0,
            release: 1.0,
            glide: table.glide_s,
            breath: 1.0,
            wobble: 1.0,
            slide: 0.02,
            // 管乐的花舌；拉弦乐器（每个音一弓的）这个标记是震音，一秒十三弓上下，谷底留三成半。
            flutter: if table.mono { 0.44 } else { 0.65 },
            flutter_rate: if table.mono { 40.0 } else { 13.0 },
            air: 1.0,
            solo: if table.mono { 1.0 } else { 0.0 },
        }
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
            (above - 1, above, between(l[above - 1].at, l[above].at, level), 1.0)
        };
        let get = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
        // 两层之间按 dB 连直线（振幅取几何平均）。按振幅连的话，轻重两层差二三十 dB 的时候，
        // 中间的力度几乎全是重的那一层缩小了的样子——又亮、又带着它的底噪，轻的那一层听不见。
        let blend = |lo: f64, hi: f64| {
            if x <= 0.0 || lo == hi {
                lo
            } else if lo.max(hi) <= 0.0 {
                0.0
            } else {
                (lo.max(1e-6).ln() * (1.0 - x) + hi.max(1e-6).ln() * x).exp()
            }
        };
        for (i, p) in partials.iter_mut().enumerate() {
            *p = scale * blend(get(&l[a].partials, i), get(&l[b].partials, i));
        }
        for (i, n) in noise.iter_mut().enumerate() {
            *n = scale * blend(get(&l[a].noise, i), get(&l[b].noise, i));
        }
    }

    /// 音高 `pitch`、力度 `level` 上每层一个数的那些量（[`HeldLayer::air`]、`pulse`、`grain`）：同泛音一样在相邻的音、相邻的层之间连直线。
    fn scalar(&self, pitch: f64, level: f64, get: impl Fn(&HeldLayer) -> f64) -> f64 {
        let of = |note: &HeldNote| {
            let l = &note.layers;
            let above = l.iter().position(|x| x.at >= level).unwrap_or(l.len());
            if above == 0 || above == l.len() {
                return get(&l[above.min(l.len() - 1)]);
            }
            let x = between(l[above - 1].at, l[above].at, level);
            get(&l[above - 1]) * (1.0 - x) + get(&l[above]) * x
        };
        let n = &self.notes;
        let above = n.iter().position(|x| x.pitch >= pitch).unwrap_or(n.len());
        if above == 0 || above == n.len() {
            return of(&n[above.min(n.len() - 1)]);
        }
        let x = (pitch - n[above - 1].pitch) / (n[above].pitch - n[above - 1].pitch);
        of(&n[above - 1]) * (1.0 - x) + of(&n[above]) * x
    }

    /// 左右声道怎么分（[`HeldLayer::left`]、`turn`）：离得最近的音、最近的层量到的。
    fn stereo(&self, pitch: f64, level: f64) -> (&[f64], &[f64]) {
        let note = self.notes.iter().min_by(|a, b| (a.pitch - pitch).abs().total_cmp(&(b.pitch - pitch).abs())).expect("至少一个音");
        let layer = note.layers.iter().min_by(|a, b| (a.at - level).abs().total_cmp(&(b.at - level).abs())).expect("至少一层");
        (&layer.left, &layer.turn)
    }

    /// 起音那一段的音色：离得最近的音、最近的层量到的。
    fn early(&self, pitch: f64, level: f64) -> &[f64] {
        let note = self.notes.iter().min_by(|a, b| (a.pitch - pitch).abs().total_cmp(&(b.pitch - pitch).abs())).expect("至少一个音");
        &note.layers.iter().min_by(|a, b| (a.at - level).abs().total_cmp(&(b.at - level).abs())).expect("至少一层").early
    }

    /// 各次泛音跟着揉音变多少：离得最近的音、最近的层量到的。
    fn sway(&self, pitch: f64, level: f64) -> &[f64] {
        let note = self.notes.iter().min_by(|a, b| (a.pitch - pitch).abs().total_cmp(&(b.pitch - pitch).abs())).expect("至少一个音");
        &note.layers.iter().min_by(|a, b| (a.at - level).abs().total_cmp(&(b.at - level).abs())).expect("至少一层").sway
    }

    /// 噪声最响的那一刻在基频的哪个相位上：离得最近的音、最近的层量到的（相位不能连直线）。
    fn pulse_at(&self, pitch: f64, level: f64) -> f64 {
        let note = self.notes.iter().min_by(|a, b| (a.pitch - pitch).abs().total_cmp(&(b.pitch - pitch).abs())).expect("至少一个音");
        note.layers.iter().min_by(|a, b| (a.at - level).abs().total_cmp(&(b.at - level).abs())).expect("至少一层").pulse_at
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

pub(crate) fn band_filter(lo: f64, hi: f64, sr: f64) -> [[f64; 5]; 4] {
    [butterworth(true, lo, 0.541_196, sr), butterworth(true, lo, 1.306_563, sr), butterworth(false, hi, 0.541_196, sr), butterworth(false, hi, 1.306_563, sr)]
}

/// 方差 1 的白噪声过这个频带以后均方根的倒数。
pub(crate) fn band_unit(lo: f64, hi: f64, sr: f64) -> f64 {
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
    /// 这个音在哪个通道上（弯音按通道来）。
    channel: u8,
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
    /// 各次泛音分到"中间"（两个声道的和的一半）和"两边"（差的一半）上的系数：各乘正弦、余弦那一份。
    mid: [(f64, f64); MAX_PARTIALS],
    side: [(f64, f64); MAX_PARTIALS],
    /// 左右声道的分法要去的地方（接在没收完的音后面起的时候慢慢过去，一下子换是"啪"的一声）。
    mid_to: [(f64, f64); MAX_PARTIALS],
    side_to: [(f64, f64); MAX_PARTIALS],
    wide: bool,
    /// 管子里的气：头 [`AIR`] 次泛音各自乱动的那一份——和泛音差九十度的分量现在多大、每帧加多少，
    /// 以及生成它的两路低通噪声（同相、正交）和各自的慢的部分（减掉：慢的起伏归 `wobble` 管）。
    quad: [f64; AIR],
    quad_step: [f64; AIR],
    air: [[f64; 2]; AIR],
    air_slow: [[f64; 2]; AIR],
    /// 噪声跟着音高的包络：(κ/2, cos 相位, sin 相位, 归一)；一阵一阵的增益现在多大、每帧加多少、它的低通噪声。
    pulse: (f64, f64, f64, f64),
    grain: f64,
    grain_step: f64,
    grain_state: f64,
    noise: Vec<NoiseBand>,
    rng: u64,
    /// 慢的随机起伏：音高一路、泛音四组各一路（低通过的噪声）。
    drift: [f64; 5],
    smooth: [[f64; 3]; 5],
    /// 揉音带出来的快的闪动：八组泛音各一路。
    quick: [f64; 8],
    quick_smooth: [[f64; 3]; 8],
    vibrato_phase: f64,
    /// 合奏里的一个人：比谱上的音高偏多少音分、揉得比别人快慢多少（倍）、占整个音的多少（振幅）。
    detune: f64,
    vibrato_pace: f64,
    share: f64,
    /// 这个音是滑过来的（音符写了 `slide`，滑到了以后清掉）：从哪个音高滑起、滑到几成了；揉音从这个音的第几秒算起（滑完了才揉）。
    slide_from: f64,
    slide_at: f64,
    vibrato_from: f64,
    /// 这个音打花舌（`flutter`）。
    slide: bool,
    flutter: bool,
    /// 泛音：手指轻轻碰在弦的 1/`harmonic` 处（1 是平常按实了拉）。
    harmonic: usize,
    /// 这个音是接在还没收完的上一个音后面起的。
    rejoin: bool,
    flutter_phase: f64,
    flutter_lp: f64,
    /// 震音的这一弓：每帧走多少（一弓是 1）、这一弓多重。
    stroke: (f64, f64),
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
            channel: 0,
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
            mid: [(1.0, 0.0); MAX_PARTIALS],
            side: [(0.0, 0.0); MAX_PARTIALS],
            mid_to: [(1.0, 0.0); MAX_PARTIALS],
            side_to: [(0.0, 0.0); MAX_PARTIALS],
            wide: false,
            quad: [0.0; AIR],
            quad_step: [0.0; AIR],
            air: [[0.0; 2]; AIR],
            air_slow: [[0.0; 2]; AIR],
            pulse: (0.0, 1.0, 0.0, 1.0),
            grain: 1.0,
            grain_step: 0.0,
            grain_state: 0.0,
            noise,
            rng: 0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03),
            drift: [0.0; 5],
            smooth: [[0.0; 3]; 5],
            quick: [0.0; 8],
            quick_smooth: [[0.0; 3]; 8],
            vibrato_phase: 0.0,
            detune: 0.0,
            vibrato_pace: 1.0,
            share: 1.0,
            slide_from: 60.0,
            slide_at: 0.0,
            vibrato_from: 0.0,
            slide: false,
            flutter: false,
            harmonic: 1,
            rejoin: false,
            flutter_phase: 0.0,
            flutter_lp: 0.0,
            stroke: (13.0 / 48_000.0, 1.0),
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
    /// 弯音（半音），一个通道一个：带弯音的音符各走各的通道，弯完了（比如句尾往下一滑）不带着后面的音一起跑调。
    bend: [f64; 16],
    order: u64,
    started: Vec<u64>,
    /// 下一个音的技法（[`Playable::technique`] 先到，起音后到）。
    slide_next: bool,
    flutter_next: bool,
    /// 下一个音是泛音：碰在几分之一处（0 是让乐器自己挑）。
    harmonic_next: Option<u8>,
}

impl Held {
    pub fn new(table: &'static HeldTable, params: HeldParams) -> Self {
        Self { table, params, voices: Vec::new(), sr: 48_000.0, bend: [0.0; 16], order: 0, started: Vec::new(), slide_next: false, flutter_next: false, harmonic_next: None }
    }

    pub fn voices(table: &HeldTable) -> usize {
        // 独奏的乐器也留着十二路：`solo` 设成 0 就能吹和弦。
        12 * table.ensemble.map_or(1, |e| e.voices.clamp(1, 8))
    }

    /// 一个控制块（[`CONTROL`] 帧）开头：音高、力度、起伏往前走一步，定出这一块里各次泛音、各带噪声的振幅从哪儿到哪儿。
    #[allow(clippy::needless_range_loop)] // 同一个泛音的序号用在好几个数组上
    fn control(table: &HeldTable, p: &HeldParams, v: &mut Voice, sr: f64, bend: f64) {
        let dt = CONTROL as f64 / sr;
        let seconds = v.age as f64 / sr;
        // 连奏：音高按指数滑到新的音上。
        // 写了 `slide` 的音是有意滑的（滑音），慢得多。
        // 那是手指在弦上（管上）滑过去：慢慢离开、快快经过、稳稳落下（S 形），滑的时候压得轻一点（音量凹下去一块）、不揉，
        // 落稳了再揉起来。按指数滑（一下窜出去、再慢吞吞地靠上去）是合成器的滑音。
        let mut dip = 1.0;
        if v.slide {
            let whole = (3.0 * p.slide).max(0.01);
            v.slide_at = (v.slide_at + dt / whole).min(1.0);
            let x = v.slide_at;
            v.pitch = v.slide_from + (v.target - v.slide_from) * x * x * (3.0 - 2.0 * x);
            // 滑的途中压得轻一点：音量凹下去一块（拉弦的凹得浅些）。
            dip = 1.0 - if table.mono { 0.45 } else { 0.3 } * (PI * x).sin();
            if x >= 1.0 {
                v.slide = false;
            }
            v.vibrato_from = seconds;
        } else {
            v.pitch += (v.target - v.pitch) * (1.0 - (-dt / p.glide.max(0.001)).exp());
        }
        // 力度：`dynamics` 给了就跟它（一个音里面也能变），没给跟音符的力度。
        let want = if p.dynamics >= 0.0 { p.dynamics.min(1.0) } else { v.velocity };
        v.level += (want - v.level) * (1.0 - (-dt / 0.03).exp());
        // 慢的随机起伏：白噪声过三级低通（大约 `rate_hz` 以下；只过一级的话上面漏出来的会在每个泛音两边调出一圈底噪）。
        let k = 1.0 - (-dt * TAU * table.wobble.rate_hz).exp();
        for i in 0..v.drift.len() {
            // 低通以后方差小了：乘回来，让最后一级的均方根大致是 1。
            let mut x = v.random() * (2.0 / k).sqrt() * 2.85;
            for stage in &mut v.smooth[i] {
                *stage += (x - *stage) * k;
                x = *stage;
            }
            v.drift[i] = x;
        }
        if table.wobble.vibrato > 0.0 {
            let k = 1.0 - (-dt * TAU * 2.0 * p.vibrato_rate.max(1.0)).exp();
            for i in 0..v.quick.len() {
                let mut x = v.random() * (2.0 / k).sqrt() * 2.85;
                for stage in &mut v.quick_smooth[i] {
                    *stage += (x - *stage) * k;
                    x = *stage;
                }
                v.quick[i] = x;
            }
        }
        // 揉音：过了延迟以后慢慢加深。
        v.vibrato_phase = (v.vibrato_phase + dt * p.vibrato_rate * v.vibrato_pace) % 1.0;
        // `deep`：揉音是这件乐器默认的几倍（参数调的）；这个音高、这个力度的录音自己揉得比默认深还是浅，再乘上。
        let since = seconds - v.vibrato_from - p.vibrato_delay;
        let ramp = if since < 0.0 { 0.0 } else { table.vibrato_start + (1.0 - table.vibrato_start) * (since / table.vibrato_ramp_s.max(0.01)).min(1.0) };
        let deep = if table.vibrato_cents > 0.0 { (p.vibrato_depth * ramp / table.vibrato_cents).min(3.0) } else { 0.0 };
        let own = table.scalar(v.pitch, v.level, |l| l.vibrato);
        let depth = if own > 0.0 && table.vibrato_cents > 0.0 { deep * own } else { p.vibrato_depth * ramp };
        // 泛音上手指只是轻轻碰着，揉不深。
        let depth = if v.harmonic > 1 { 0.3 * depth } else { depth };
        let cents = depth * (TAU * v.vibrato_phase).sin() + table.wobble.cents * p.wobble * v.drift[0];
        let hz = 440.0 * ((v.pitch + bend - 69.0) / 12.0 + (cents + v.detune) / 1200.0).exp2();
        // 这一刻的音色。
        let mut partials = [0.0; MAX_PARTIALS];
        let mut noise = [0.0; 24];
        let bands = v.noise.len().min(24);
        if v.harmonic > 1 {
            // 泛音：弦还是那根长弦（低 `harmonic` 倍的那个音），手指碰着的地方只让它的第 harmonic、2·harmonic …… 次泛音响，
            // 别的都被手指按死了。这些泛音的频率正好是听到的这个音的 1、2、3 …… 次：照那根长弦量到的大小搬过来。
            //（试过再让手指多吃掉一些高次泛音，每个八度 3 dB：听着太纯，不像琴了——真琴的泛音和平常拉的音色差不多，只是高。）
            let mut string = [0.0; MAX_PARTIALS];
            table.timbre(v.pitch - 12.0 * (v.harmonic as f64).log2(), v.level, &mut string, &mut noise[..bands]);
            for (k, a) in partials.iter_mut().enumerate() {
                *a = string.get(v.harmonic * (k + 1) - 1).copied().unwrap_or(0.0);
            }
            // 弓噪跟着轻下来：泛音是轻轻地拉的，弓噪和有音高的部分的比例还是那根弦平常的样子
            //（原样留着的话，声音轻了十几 dB、噪声没轻，听着是吹出来的气）。
            let (whole, kept) = (string.iter().map(|a| a * a).sum::<f64>(), partials.iter().map(|a| a * a).sum::<f64>());
            let quiet = (kept / whole.max(1e-30)).sqrt().min(1.0);
            noise[..bands].iter_mut().for_each(|b| *b *= quiet);
        } else {
            table.timbre(v.pitch, v.level, &mut partials, &mut noise[..bands]);
        }
        let sway = table.sway(v.pitch, v.level);
        let early = table.early(v.pitch, v.level);
        let young = 0.115_129 * (-seconds / table.early_s.max(0.05)).exp();
        // 揉到哪儿了（sin φ、cos φ，和两倍的），现在揉的深度是默认的几倍。
        let (turn, twice) = ((TAU * v.vibrato_phase).sin_cos(), (2.0 * TAU * v.vibrato_phase).sin_cos());
        // 起伏是在 dB 上加减的，平均下来会偏响：按它的方差补回去（平均能量不变）。
        let even = (-(table.wobble.depth.powi(2) + (table.wobble.vibrato * deep.min(1.5)).powi(2)) * p.wobble * p.wobble).exp();
        partials.iter_mut().for_each(|a| *a *= v.share);
        noise.iter_mut().for_each(|a| *a *= v.share);
        // 管子里的气：每个泛音各有两路低通过的噪声（方差 1），一路动它的大小，一路动它的相位。
        let air = table.scalar(v.pitch, v.level, |l| l.air) * p.air;
        // 噪声的样子：跟着音高一下一下（κ：von Mises 的尖锐程度，一阶分量正好是量到的 pulse）、一阵一阵（慢的随机增益）。
        let pulse = table.scalar(v.pitch, v.level, |l| l.pulse).clamp(0.0, 0.9);
        let kappa = pulse * (2.0 - pulse * pulse) / (1.0 - pulse * pulse);
        let at = table.pulse_at(v.pitch, v.level);
        // I0(κ)：包络的平方平均是 1。
        let i0 = (1..20).fold((1.0, 1.0), |(sum, term), k| { let t = term * (kappa / 2.0).powi(2) / (k * k) as f64; (sum + t, t) }).0;
        v.pulse = (kappa / 2.0, at.cos(), at.sin(), 1.0 / i0.sqrt());
        let grain = table.scalar(v.pitch, v.level, |l| l.grain);
        let kg = 1.0 - (-dt * TAU * GRAIN_HZ).exp();
        let w = (v.random() + v.random() + v.random()) * ((2.0 - kg) / kg).sqrt();
        v.grain_state += (w - v.grain_state) * kg;
        let gain = (grain * v.grain_state - grain * grain).exp();
        v.grain_step = (gain - v.grain) / CONTROL as f64;
        let (k, slow) = (1.0 - (-dt * TAU * AIR_HZ).exp(), 1.0 - (-dt * TAU * AIR_SLOW_HZ).exp());
        let unit = ((2.0 - k) / k).sqrt();
        let mut shake = [[0.0; 2]; AIR];
        if air > 0.0 {
            for n in 0..AIR {
                for c in 0..2 {
                    let w = v.random() * 3f64.sqrt() * unit;
                    v.air[n][c] += (w - v.air[n][c]) * k;
                    v.air_slow[n][c] += (v.air[n][c] - v.air_slow[n][c]) * slow;
                    shake[n][c] = air * (v.air[n][c] - v.air_slow[n][c]);
                }
            }
        }
        // 开合：起音时往 1 去，松开以后往 0 去。
        let (attack, release) = (table.attack_s * p.attack, table.release_s * p.release);
        // 一下一下的奏法：到时候自己收。
        if table.stroke_s.is_some_and(|stroke| seconds >= stroke) {
            v.held = false;
        }
        let open = if v.held { 1.0 } else { 0.0 };
        v.gate += (open - v.gate) * (1.0 - (-dt / if v.held { attack.max(0.001) } else { release.max(0.001) }).exp());
        // 吐音的那一下重音：起音时响一点，六十毫秒上下退回去。
        let fade = (-(seconds - table.attack_hold_s).max(0.0) / table.attack_len_s.max(0.01)).exp();
        let accent = if table.onset.len() == ONSET_AT.len() {
            // 量到的响度曲线：两个时刻之间按 dB 连直线，头一个时刻以前用头一个数，两秒以后半秒里回到 0。
            let i = ONSET_AT.partition_point(|t| *t <= seconds);
            let db = if i == 0 {
                table.onset[0]
            } else if i == ONSET_AT.len() {
                table.onset[i - 1] * (1.0 - (seconds - ONSET_AT[i - 1]) / 0.5).clamp(0.0, 1.0)
            } else {
                table.onset[i - 1] + (table.onset[i] - table.onset[i - 1]) * (seconds - ONSET_AT[i - 1]) / (ONSET_AT[i] - ONSET_AT[i - 1])
            };
            10f64.powf(db / 20.0)
        } else {
            10f64.powf(table.attack_db / 20.0 * fade)
        };
        // 重音在高处多出来的那一份（每个八度乘一次）。
        let bright = 10f64.powf(table.attack_tilt_db / 20.0 * fade);
        v.count = 0;
        // 左右声道的分法：几十毫秒过去（每个控制周期挪一小步，步子大了听得出台阶）。
        let pan = 1.0 - (-dt / 0.04).exp();
        for n in 0..MAX_PARTIALS {
            v.mid[n] = (v.mid[n].0 + (v.mid_to[n].0 - v.mid[n].0) * pan, v.mid[n].1 + (v.mid_to[n].1 - v.mid[n].1) * pan);
            v.side[n] = (v.side[n].0 + (v.side_to[n].0 - v.side[n].0) * pan, v.side[n].1 + (v.side_to[n].1 - v.side[n].1) * pan);
            let f = hz * (n + 1) as f64;
            if f > 0.45 * sr {
                partials[n] = 0.0;
            } else {
                v.count = n + 1;
            }
            let w = TAU * f / sr;
            v.rot[n] = (w.cos(), w.sin());
            // 起音：高的泛音进来得晚一点（每高一个八度晚 0.6 个起音时间）；之后跟着力度和起伏走。
            let octaves = ((n + 1) as f64).log2();
            let late = (1.0 - (-(seconds / (attack * (1.0 + table.attack_late * octaves)).max(0.001))).exp()).min(1.0) * bright.powf(octaves);
            // 起伏按 dB 走（乘出来不会是负的），再加上跟着揉音变的那一份。
            let swing = if v.harmonic > 1 { None } else { sway.get(4 * n..4 * n + 4) }.map_or(0.0, |c| deep * (c[0] * turn.1 + c[1] * turn.0) + deep * deep * (c[2] * twice.1 + c[3] * twice.0));
            let shimmer = ((table.wobble.depth * v.drift[1 + n % 4] + table.wobble.vibrato * deep.min(1.5) * v.quick[n % 8]) * p.wobble).clamp(-1.5, 1.5).exp() * even * (0.115_129 * swing).clamp(-2.0, 2.0).exp();
            let onset = if v.harmonic > 1 { 1.0 } else { early.get(n).map_or(1.0, |db| (db * young).exp()) };
            // 接在没收完的上一个音后面起的：不从零起（那样上一个音的尾巴一下子被掐掉，是"啪"的一声），尾巴在二三十毫秒里收掉，新的音照常起来。
            let late = if v.rejoin { late.max((-seconds / 0.012).exp()) } else { late };
            let target = partials[n] * shimmer * onset * v.gate * dip * accent * if v.held { late } else { 1.0 };
            if n < AIR {
                v.step[n] = (target * (1.0 + shake[n][0]) - v.amp[n]) / CONTROL as f64;
                v.quad_step[n] = (target * shake[n][1] - v.quad[n]) / CONTROL as f64;
            } else {
                v.step[n] = (target - v.amp[n]) / CONTROL as f64;
            }
        }
        // 气声：起音的头几十毫秒多一口。
        // 量到了的：另外加上那一份（按能量加），那几个时刻之间连直线，头一个时刻以前从 0 连上去，最后一个时刻以后 0.2 秒里退完；
        // 它比音先到，不跟着音慢慢起来。连奏里后面的音没有这一下（舌头没动）。
        let last = PUFF_AT[PUFF_AT.len() - 1];
        let shape = |i: usize| table.scalar(v.pitch, v.level, |l| l.puff_shape.get(i).copied().unwrap_or(0.0));
        let burst = if !v.held || seconds >= last + 0.2 {
            0.0
        } else {
            let i = PUFF_AT.partition_point(|t| *t <= seconds);
            if i <= 1 {
                // 头一个时刻量的那一小段有四十毫秒长，盖住了出声以前的静：不用它，从 0 直接连到第二个时刻。
                shape(1) * seconds / PUFF_AT[1]
            } else if i == PUFF_AT.len() {
                shape(i - 1) * (1.0 - (seconds - last) / 0.2).max(0.0)
            } else {
                shape(i - 1) + (shape(i) - shape(i - 1)) * (seconds - PUFF_AT[i - 1]) / (PUFF_AT[i] - PUFF_AT[i - 1])
            }
        };
        // 没量到那一下的：起音的头几十毫秒多一口气。
        let has = seconds < 0.3 && table.scalar(v.pitch, v.level, |l| if l.puff.is_empty() { 0.0 } else { 1.0 }) > 0.0;
        let puff = if has { 1.0 } else { 1.0 + 2.0 * (-seconds / 0.04).exp() } * if table.onset.is_empty() { 1.0 } else { accent };
        for (n, (b, level)) in v.noise.iter_mut().zip(&noise).enumerate() {
            // （稳定段的噪声一阵大一阵小，那个随机的增益不该乘到这一下上：先除掉。）
            let more = if burst > 0.0 { burst * v.share * table.scalar(v.pitch, v.level, |l| l.puff.get(n).copied().unwrap_or(0.0)) / v.grain.max(0.05) } else { 0.0 };
            let steady = level * v.gate * puff;
            b.step = ((steady * steady + more * more).sqrt() * p.breath - b.level) / CONTROL as f64;
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
    fn technique(&mut self, _note: NoteRef, tag: crate::zither::NoteTag) {
        self.slide_next = tag.slide;
        self.flutter_next = tag.flutter;
        self.harmonic_next = match tag.technique {
            Some((muser_core::Technique::Harmonic, node)) => Some(node.unwrap_or(0)),
            _ => None,
        };
    }

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
        let (slide, flutter) = (std::mem::take(&mut self.slide_next), std::mem::take(&mut self.flutter_next));
        // 泛音：音符的音高是听到的音高。弦的空弦（或按住的那个音）在它下面两个八度（碰四分之一处）、一个八度加五度（三分之一）、
        // 一个八度（一半）。没写碰在哪儿，就挑这件乐器够得着的里面最低的那根弦（人工泛音一般是碰四度，也就是四分之一处）。
        let lowest = self.table.notes.first().map_or(0.0, |n| n.pitch);
        let harmonic = match self.harmonic_next.take() {
            None => 1,
            Some(0) => [4usize, 3, 2].into_iter().find(|h| key - 12.0 * (*h as f64).log2() >= lowest - 0.5).unwrap_or(2),
            Some(h) => usize::from(h).clamp(2, 8),
        };
        // 独奏乐器：前一个音还按着，是连奏——不重新起音，滑过去。
        let solo = self.params.solo >= 0.5 && self.table.ensemble.is_none();
        if solo && self.voices[0].sounding() && self.voices[0].held {
            let bowed = !self.table.mono;
            let v = &mut self.voices[0];
            (v.slide, v.flutter) = (slide, flutter);
            v.harmonic = harmonic;
            v.channel = note.channel;
            // 拉弦乐器一弓里换音（不是滑音的时候）：手指换了把位，音会轻轻断一下再起来，揉音也重新揉起来——
            // 不然几个音连成一条线、只有音高在变，听着是合成器。
            if bowed && !slide {
                v.gate *= 0.5;
                v.vibrato_from = v.age as f64 / self.sr;
                v.pitch = key;
            }
            // 管乐的连奏气不断，但换了指法，管子里的驻波要重新立起来：音量轻轻凹一下（3 dB 上下、一个起音时间就回来）。
            // 一点不凹的话，几个音之间音量是一条直线、只有音高在跳，有电子琴的味道。凹多少是定的，没有连奏的录音可量。
            if !bowed && !slide {
                v.gate *= 0.7;
            }
            if slide {
                // 从上一个音不停地滑过去，中间不断开、不跳（试过"手先换过去、只滑最后两个半音"：换手的那一下是个顿音，更不自然）。
                (v.slide_from, v.slide_at) = (v.pitch, 0.0);
            }
            v.key = Some(note.key);
            v.target = key;
            v.velocity = velocity;
            return;
        }
        // 合奏：这个音由几个人各拉各的（音高差一点、揉得不齐、起的相位各不一样）；独奏就一个人。
        let people = self.table.ensemble.map_or(1, |e| e.voices.clamp(1, 8));
        let spread = self.table.ensemble.map_or(0.0, |e| e.cents);
        for person in 0..people {
            // 找一路空的；没有就抢最早开始的。
            let i = if solo { 0 } else { self.voices.iter().position(|v| !v.sounding()).unwrap_or_else(|| (0..self.voices.len()).min_by_key(|&i| self.started[i]).unwrap_or(0)) };
            self.started[i] = self.order;
            let (table, dynamics) = (self.table, self.params.dynamics);
            let v = &mut self.voices[i];
            // 这一路还在收上一个音的尾巴：从它现在的声音接着起（不咔嗒）；空着的从头起。
            let fresh = !v.sounding();
            v.rejoin = !fresh;
            // 滑音只在连奏里有（从上一个音滑过来）；单独起的音没处可滑。
            (v.slide, v.flutter) = (false, flutter);
            v.harmonic = harmonic;
            v.channel = note.channel;
            v.key = Some(note.key);
            v.held = true;
            v.age = 0;
            v.vibrato_from = 0.0;
            v.target = key;
            v.pitch = key;
            v.velocity = velocity;
            v.level = if dynamics >= 0.0 { dynamics.min(1.0) } else { velocity };
            v.until = 0;
            v.share = 1.0 / (people as f64).sqrt();
            // 震音：头一弓从弓的中段起（不是从换弓的谷底起）、下得重一点——录音里起音的那一弓比后面的响得多。
            if table.tremolo.is_some() || (flutter && !table.mono) {
                let rate = table.tremolo.map_or(self.params.flutter_rate, |t| t.rate_hz).max(1.0);
                v.flutter_phase = 0.5;
                v.stroke = (rate / self.sr, 1.0);
            }
            let turn = if people > 1 {
                // 三个均匀分布的数加起来近似正态（方差 1）。
                v.detune = spread * (v.random() + v.random() + v.random());
                v.vibrato_pace = 1.0 + 0.06 * v.random();
                v.vibrato_phase = 0.5 + 0.5 * v.random();
                PI * v.random() * if person == 0 { 0.0 } else { 1.0 }
            } else {
                // 独奏乐器同时响的几个音（双音、和弦）各揉各的：从哪儿揉起、快慢都差一点。一起起、一样快的话，
                // 几个音的音高一齐动，耳朵把它们并成一个音（采样里每个音是各录各的，揉得不齐）。
                (v.detune, v.vibrato_pace) = (0.0, if solo { 1.0 } else { 1.0 + 0.06 * v.random() });
                if !solo {
                    v.vibrato_phase = 0.5 + 0.5 * v.random();
                }
                0.0
            };
            if fresh {
                v.gate = 0.0;
                v.amp = [0.0; MAX_PARTIALS];
                v.quad = [0.0; AIR];
                v.noise.iter_mut().for_each(|b| b.level = 0.0);
                // 各次泛音按量到的相位起（波形的样子：尖还是圆，是相位定的）；合奏里别的人整个波形错开一点。
                let phases = table.phases(key, v.level);
                for n in 0..MAX_PARTIALS {
                    let ph = phases.get(n).copied().unwrap_or(0.0) + turn * (n + 1) as f64;
                    (v.re[n], v.im[n]) = (ph.cos(), ph.sin());
                }
            }
            // 左右声道：左 = g·sin，右 = g'·sin(φ + Δ)；拆成中间（和的一半）、两边（差的一半）。
            let (left, angle) = table.stereo(key, v.level);
            // 接在没收完的音后面起的：上一个音是单声道的分法，就从"左右一样"慢慢过去；上一个是立体声的，不能一下子变回单声道。
            if !fresh && !v.wide {
                (v.mid, v.side) = ([(1.0, 0.0); MAX_PARTIALS], [(0.0, 0.0); MAX_PARTIALS]);
            }
            v.wide = !left.is_empty() || (!fresh && v.wide);
            for n in 0..MAX_PARTIALS {
                let gl = left.get(n).copied().unwrap_or(1.0).clamp(0.0, core::f64::consts::SQRT_2);
                let gr = (2.0 - gl * gl).max(0.0).sqrt();
                let (sin, cos) = angle.get(n).copied().unwrap_or(0.0).sin_cos();
                v.mid_to[n] = ((gl + gr * cos) / 2.0, gr * sin / 2.0);
                v.side_to[n] = ((gl - gr * cos) / 2.0, -gr * sin / 2.0);
                if fresh {
                    (v.mid[n], v.side[n]) = (v.mid_to[n], v.side_to[n]);
                }
            }
            {
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
        let depth = params.flutter.clamp(0.0, 1.0);
        let bowed = !table.mono;
        for v in self.voices.iter_mut().filter(|v| v.sounding()) {
            for i in 0..frames {
                if v.until == 0 {
                    Self::control(table, &params, v, sr, bend[usize::from(v.channel & 15)]);
                    v.until = CONTROL;
                }
                v.until -= 1;
                v.age += 1;
                // x：中间（两个声道的和的一半）；d：两边（差的一半）。单声道的录音 d 是 0。
                let (mut x, mut d, mut shape) = (0.0, 0.0, 1.0);
                if v.wide {
                    for n in 0..v.count {
                        let (c, s) = v.rot[n];
                        let re = v.re[n] * c - v.im[n] * s;
                        v.im[n] = v.re[n] * s + v.im[n] * c;
                        v.re[n] = re;
                        v.amp[n] += v.step[n];
                        x += v.amp[n] * (v.mid[n].0 * v.im[n] + v.mid[n].1 * v.re[n]);
                        d += v.amp[n] * (v.side[n].0 * v.im[n] + v.side[n].1 * v.re[n]);
                    }
                } else {
                    for n in 0..v.count {
                        let (c, s) = v.rot[n];
                        let re = v.re[n] * c - v.im[n] * s;
                        v.im[n] = v.re[n] * s + v.im[n] * c;
                        v.re[n] = re;
                        v.amp[n] += v.step[n];
                        x += v.amp[n] * v.im[n];
                    }
                }
                for n in 0..v.count.min(AIR) {
                    v.quad[n] += v.quad_step[n];
                    x += v.quad[n] * v.re[n];
                }
                // 噪声：每个频带各用各的白噪声（方差 1 的均匀分布）。共用一路的话相邻的频带在交界上相位相反，加起来互相抵掉一大截。
                let mut hiss = 0.0;
                for k in 0..v.noise.len() {
                    let w = v.random() * 3f64.sqrt();
                    let b = &mut v.noise[k];
                    b.level += b.step;
                    if b.level > 1e-7 {
                        let y = b.coef.iter().zip(&mut b.state).fold(w, |y, (c, z)| biquad(c, z, y));
                        hiss += y * b.level * b.unit;
                    }
                }
                // 跟着音高一下一下、一阵一阵（见 [`HeldLayer::pulse`]、`grain`）：基频的转子转到量到的那个相位时最响。
                v.grain += v.grain_step;
                let (half, c, s, unit) = v.pulse;
                x += hiss * v.grain * if half > 0.0 { (half * (v.im[0] * s + v.re[0] * c)).exp() * unit } else { 1.0 };
                if ((v.flutter && bowed) || table.tremolo.is_some()) && v.age as f64 >= table.attack_hold_s * sr {
                    // 拉弦的震音：弓飞快地来回拉，一秒十几弓。每一弓长短不一（差两成上下）、轻重不一（差三成上下），
                    // 换弓的那一下音量掉下去但不到底，还带出一点弓毛擦弦的噪声。量自独奏小提琴的震音录音：
                    // 每秒 12 … 14 弓，谷底是平均音量的三四成。匀匀地一秒抖十三下是机器，听着发毛。
                    v.flutter_phase += v.stroke.0;
                    if v.flutter_phase >= 1.0 {
                        v.flutter_phase -= 1.0;
                        // 两个均匀分布的数加起来，标准差 0.816。
                        let (a, b) = ((v.random() + v.random()) / 0.816, (v.random() + v.random()) / 0.816);
                        let (rate, uneven, weight) = table.tremolo.map_or((params.flutter_rate, 0.16, 0.31), |t| (t.rate_hz, t.uneven, t.weight));
                        let seconds = (1.0 + uneven * a).max(0.4) / rate.max(1.0);
                        v.stroke = (1.0 / (seconds * sr), (1.0 + weight * b).max(0.3));
                    }
                    // 一弓的音量：谷底圆圆地过去（升余弦）。带尖角的（正弦的 0.7 次方那样）每换一弓就"啪"一下，是破音。
                    let bow = 0.5 - 0.5 * (TAU * v.flutter_phase).cos();
                    let r = v.random();
                    let hiss = r - v.flutter_lp;
                    v.flutter_lp += 0.33 * (r - v.flutter_lp);
                    // 表里的深度是按"谷底是平均的几成"记的：换成这个形状下的深度。
                    let floor = table.tremolo.map_or(1.0 - depth, |t| { let d = t.depth.clamp(0.0, 0.95); (1.0 - d) / (1.0 - 0.3 * d) });
                    let deep = (1.0 - floor) / (1.0 - 0.5 * floor);
                    let gain = ((1.0 - deep) + deep * v.stroke.1 * bow) / (1.0 - 0.5 * deep);
                    // 头一弓（长长的一弓）之后抖起来：四十毫秒里慢慢换过去，不是一下子切过去。
                    let enter = ((v.age as f64 / sr - table.attack_hold_s) / 0.04).clamp(0.0, 1.0);
                    let enter = enter * enter * (3.0 - 2.0 * enter);
                    shape = 1.0 + enter * (gain - 1.0);
                    x = x * shape + 0.06 * x.abs() * hiss * (1.0 - bow) * enter;
                } else if v.flutter {
                    // 花舌：舌尖打嘟噜，每秒 40 下上下把气流截断又放开（快慢不完全匀），每次放开多一股高处的气声
                    //（和内置的笛子一样，量自笛子的录音；别的管乐器打花舌也是这个速度）。
                    let r = v.random();
                    v.flutter_phase = (v.flutter_phase + params.flutter_rate * (1.0 + 0.06 * r) / sr).fract();
                    let open = 0.5 + 0.5 * (TAU * v.flutter_phase).sin();
                    let hiss = r - v.flutter_lp;
                    v.flutter_lp += 0.33 * (r - v.flutter_lp);
                    shape = 1.0 - depth * (1.0 - open);
                    x = x * shape + 0.06 * x.abs() * hiss * open * (depth / 0.44);
                }
                // 两个声道一样（居中）：表里的振幅就是录音在每个声道上的振幅，放在音轨上和录音一样响。
                let d = d * shape;
                for c in 0..channels {
                    buffer.channel_mut(c)[i] += match c {
                        0 => x + d,
                        1 => x - d,
                        _ => x,
                    } as f32;
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
        if let MidiMessage::PitchBend { channel, value } = message {
            self.bend[usize::from(channel & 15)] = f64::from(value) / 8192.0 * 2.0;
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

    /// 一段声音的基频（过零点数）。
    fn hz(x: &[f32]) -> f64 {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 * 48_000.0 / x.len() as f64
    }

    #[test]
    fn a_slide_glides_where_a_slur_changes_at_once() {
        let after = |slide: bool| {
            let mut h = Held::new(table(), HeldParams { wobble: 0.0, breath: 0.0, slide: 0.05, ..HeldParams::of(table()) });
            h.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 480 });
            let mut b = AudioBuffer::<f32>::new(1, 480);
            h.note_on(NoteRef::new(0, 48), 1.0, false);
            (0..50).for_each(|_| h.process(&mut b));
            h.technique(NoteRef::new(0, 60), crate::zither::NoteTag { technique: None, damp: false, flutter: false, slide, articulation: 0 });
            h.note_on(NoteRef::new(0, 60), 1.0, false);
            h.note_off(NoteRef::new(0, 48), 0.0);
            // 换音后 20 … 40 ms。
            let mut out = Vec::new();
            for _ in 0..4 {
                h.process(&mut b);
                out.extend_from_slice(b.channel(0));
            }
            hz(&out[960..])
        };
        let (slur, slide) = (after(false), after(true));
        assert!(slur > 250.0, "a slur is there within 20 ms: {slur} Hz");
        assert!(slide < 235.0 && slide > 95.0, "a slide is still on its way: {slide} Hz");
    }

    #[test]
    fn flutter_tongue_shakes_the_note_forty_times_a_second() {
        let shake = |flutter: bool| {
            let mut h = Held::new(table(), HeldParams { wobble: 0.0, breath: 0.0, ..HeldParams::of(table()) });
            h.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 480 });
            let mut b = AudioBuffer::<f32>::new(1, 480);
            h.technique(NoteRef::new(0, 60), crate::zither::NoteTag { technique: None, damp: false, flutter, slide: false, articulation: 0 });
            h.note_on(NoteRef::new(0, 60), 1.0, false);
            let mut out = Vec::new();
            for _ in 0..100 {
                h.process(&mut b);
                out.extend_from_slice(b.channel(0));
            }
            // 每两个周期（367 帧）的均方根，看它起伏多少、一秒过平均值几次。
            let env: Vec<f64> = out[24_000..].as_chunks::<367>().0.iter().map(|c| (c.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / 367.0).sqrt()).collect();
            let mean = env.iter().sum::<f64>() / env.len() as f64;
            let dev = (env.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / env.len() as f64).sqrt() / mean;
            let ups = env.windows(2).filter(|w| w[0] <= mean && w[1] > mean).count() as f64 / (env.len() as f64 * 367.0 / 48_000.0);
            (dev, ups)
        };
        let ((plain, _), (shaken, rate)) = (shake(false), shake(true));
        assert!(plain < 0.05 && shaken > 0.12, "plain {plain}, flutter {shaken}");
        assert!((34.0..46.0).contains(&rate), "flutter rate {rate} Hz");
    }
}
