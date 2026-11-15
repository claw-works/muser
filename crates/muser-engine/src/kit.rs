//! yinse 的一套打击乐（`muser-yinse-kit`）：muser-yinse 音色库里 `instruments/kit/<作者>/<名>/<名>.yinse.json`，音轨写 `yinse:kit/<作者>/<名>`。
//!
//! 一套里有很多件（底鼓、军鼓、大锣……），每件占一个或几个 MIDI 键；一件由一个或几个部分一起敲
//! （底鼓：鼓腔一部分、鼓槌一部分），每个部分是一组振动模式加分频带的噪声（[`crate::percussion`] 的发声），
//! 各自随力度变各自的；或者分力度层（`velocities`）：几种力度各量各的音色，两层之间此消彼长。件与件之间可以掐断（闭镲夹住开镲）、可以各带一段 EQ；整套可以带一个压缩；
//! 这一套没有的键可以交给一套内置的鼓组（`fallback`）。
//!
//! 文件由 `muser yinse extract --kit` 从单击的录音（或一个 SFZ 鼓组）里量出来，也可以手工拟合后写出同样的格式。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use manguebyte::prelude::{AudioBuffer, Filter, FilterShape, NoteRef, ProcessSpec, Processor};
use serde::{Deserialize, Serialize};

use crate::instrument::Playable;
use crate::percussion::{Kit, KitParams, NoiseLayer, PieceDef, PieceSet};

pub const FORMAT: &str = "muser-yinse-kit";
/// 这个版本的 muser 读得懂的最高版本。
pub const VERSION: u32 = 1;

/// 一套打击乐的文件。`format` 必须是第一个键（读乐器时只看文件头就知道是不是一套鼓）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KitFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// 输出端补多少 dB，参考乐句（底鼓 / 军鼓 / 闭镲的基本节奏）才是 -20 LUFS（见 [`crate::levels`]）。
    #[serde(default)]
    pub level_db: f64,
    /// 整套的压缩；没有就不压。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compress: Option<Compress>,
    /// 这一套没有的键交给哪套内置鼓组。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
    pub pieces: Vec<Piece>,
}

/// 峰值压缩：超过门限的按比例压下去，再整体补回 `makeup_db`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Compress {
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_s: f64,
    pub release_s: f64,
    #[serde(default)]
    pub makeup_db: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fallback {
    /// 一套内置鼓组的名字（`drums.acoustic` …）。
    pub instrument: String,
    /// 力度的指数（1 不变，越小轻敲越响）。
    #[serde(default = "one")]
    pub velocity: f64,
    /// 那套鼓组的 `instrument_params`。
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, serde_json::Value>,
}

/// 一件：名字（网格行名、`instrument_params` 的键）、中文名、占哪些键（第一个是它自己的键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zh: String,
    pub keys: Vec<u8>,
    /// 这一件是什么、怎么量的（给人看）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// 敲它的时候掐断哪些键上正在响的（可以有它自己：再敲一下，上一下收掉）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chokes: Vec<u8>,
    /// 这一件出声之后过的 EQ。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eq: Vec<EqBand>,
    /// 一起敲的几个部分（只有一种音色的件：轻重靠每个部分的 `velocity` 曲线）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    /// 力度分层：几种力度各量各的音色（轻敲不只是重击变小了，鼓皮没绷那么紧、音高和泛音都不一样），由轻到重。
    /// 力度落在两层之间的两层一起敲、此消彼长；比最轻的一层还轻、比最重的还重的照那一层放小、放大。有它就不用 `parts`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub velocities: Vec<Stroke>,
    /// 力度分层的件：录音在每个力度有多响（力度 → dB，只看相对的高低；可以比层多，没做成层的那几下也在里面）。
    /// 两层之间的音量照这张表走（中间连直线），不是照两层此消彼长加出来多少算多少。没有这张表就不校。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<[f64; 2]>,
}

/// 一种力度的音色：在力度 `at` 量的，各部分的 `gain` 就是这个力度的音量（不用 `velocity` 曲线）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stroke {
    pub at: f64,
    pub parts: Vec<Part>,
}

impl Piece {
    /// 录音在力度 `v` 有多响（dB，[`Piece::levels`]：中间连直线；比最轻的还轻照力度的 1.4 次方小下去，比最重的还重照它大上去、最多 3 dB）。
    fn level_at(&self, v: f64) -> f64 {
        let p = &self.levels;
        let (first, last) = (p[0], p[p.len() - 1]);
        if v <= first[0] {
            return first[1] + 28.0 * (v.max(1e-3) / first[0]).log10();
        }
        if v >= last[0] {
            return last[1] + (28.0 * (v / last[0]).log10()).min(3.0);
        }
        let i = p.windows(2).position(|w| v < w[1][0]).unwrap_or(p.len() - 2);
        p[i][1] + (v - p[i][0]) * (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0])
    }

    /// 所有的部分：没分层的就是 `parts`，分了层的是各层的挨着排。
    pub fn all_parts(&self) -> impl Iterator<Item = &Part> {
        let layered = !self.velocities.is_empty();
        self.parts.iter().filter(move |_| !layered).chain(self.velocities.iter().flat_map(|s| s.parts.iter()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    /// `peak` / `high_shelf` / `low_shelf`。
    pub shape: String,
    pub hz: f64,
    pub q: f64,
    pub gain_db: f64,
}

/// 一件里一起敲的一个部分：一组模式（频率比、振幅、衰减到 -60 dB 的秒数）和噪声。字段的意思见 [`PieceDef`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// 这个部分随力度怎么变；不写就是引擎自己的（音量是力度的 1.4 次方）。
    #[serde(default, skip_serializing_if = "Velocity::is_plain")]
    pub velocity: Velocity,
    pub f0: f64,
    #[serde(default = "one")]
    pub gain: f64,
    #[serde(default)]
    pub pan: f64,
    /// 噪声的随机数用第几路（同一套里各件不同）。
    #[serde(default)]
    pub seed: usize,
    /// 老式的那一层噪声用两级滤波（镲）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub steep: bool,
    #[serde(default)]
    pub modes: Vec<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glide: Option<Glide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<Noise>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f64,
    #[serde(default = "mode_vel", skip_serializing_if = "is_mode_vel")]
    pub mode_vel: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mode_attack: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub swell_above: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drive: f64,
}

/// 敲下去之后音高滑多少半音、用多久；`from` 与 `velocity` 见 [`PieceDef::glide_from`]、[`PieceDef::glide_vel`]。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Glide {
    pub semitones: f64,
    pub seconds: f64,
    #[serde(default)]
    pub from: f64,
    #[serde(default)]
    pub velocity: f64,
}

/// 老式的一层敲击噪声：多响、频带（Hz）、衰减的时间常数（秒）、分几下打出来（拍手）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Noise {
    pub level: f64,
    pub lo: f64,
    pub hi: f64,
    pub seconds: f64,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub bursts: u8,
}

/// 一层带通噪声（见 [`NoiseLayer`]）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Layer {
    pub lo: f64,
    pub hi: f64,
    pub fast: f64,
    pub fast_s: f64,
    pub slow: f64,
    pub slow_s: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pan: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub steep: bool,
    /// 下沿也用四阶（见 [`NoiseLayer::sharp`]）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub sharp: bool,
}

/// 一个部分随力度怎么变（写其中一种）：
/// - `{"curve": 0.8}`：力度先取 0.8 次方；
/// - `{"power": 2.4, "at": 0.95}`：音量是力度的 2.4 次方，力度 0.95 时是量到的那么响；
/// - `{"db": [[0.35, -12.8], [0.6, -5.7], [0.95, 0]]}`：量到的力度表（力度 → 比最重的轻多少 dB），中间连直线；
///   加 `"line": true` 两头照直线延出去（`floor_db` 是下限），不加的话比表里最轻的还轻时至少跟着力度一起小下去、最多比表高 3 dB。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Velocity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub db: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub line: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_db: Option<f64>,
}

fn one() -> f64 {
    1.0
}
fn mode_vel() -> f64 {
    1.4
}
fn is_mode_vel(v: &f64) -> bool {
    *v == 1.4
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

impl Velocity {
    fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// 音符的力度 → 交给 [`Kit`] 的力度（它的音量是力度的 1.4 次方）。
    fn apply(&self, velocity: f32) -> f32 {
        let raw = f64::from(velocity.clamp(0.0, 1.0));
        if self.db.len() >= 2 {
            let p = &self.db;
            let i = p.windows(2).position(|w| raw < w[1][0]).unwrap_or(p.len() - 2);
            let ([x0, y0], [x1, y1]) = (p[i], p[i + 1]);
            let db = y0 + (raw - x0) * (y1 - y0) / (x1 - x0);
            let db = if self.line {
                db.max(self.floor_db.unwrap_or(-50.0))
            } else {
                // 比表里最轻的还轻：至少跟着力度一起小下去（表的头一段有的很平，照直线延出去的话再轻也不变小）。
                let db = if raw < p[0][0] { db.min(p[0][1] + 20.0 * (raw.max(1e-3) / p[0][0]).log10()) } else { db };
                // 表外顺着延出去，但不超过 +3 dB（鼓槌那一层在重击附近升得很陡）。
                db.clamp(self.floor_db.unwrap_or(-50.0), 3.0)
            };
            10f64.powf(db / 20.0 / 1.4) as f32
        } else if let Some(power) = self.power {
            (raw / self.at.unwrap_or(1.0).max(1e-3)).powf(power / 1.4) as f32
        } else if let Some(curve) = self.curve {
            raw.powf(curve) as f32
        } else {
            velocity
        }
    }
}

/// 读进来的一套：文件和每件每个部分（[`Piece::all_parts`] 的顺序）的设计表（给 [`Kit`] 用的，一个部分一张单件的表）。
pub struct Loaded {
    pub path: PathBuf,
    pub file: KitFile,
    defs: Vec<Vec<&'static [PieceDef]>>,
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn def(piece: &Piece, part: &Part) -> PieceDef {
    let noise = part.noise.unwrap_or(Noise { level: 0.0, lo: 1_000.0, hi: 8_000.0, seconds: 0.001, bursts: 0 });
    let glide = part.glide.unwrap_or(Glide { semitones: 0.0, seconds: 0.1, from: 0.0, velocity: 0.0 });
    PieceDef {
        name: Box::leak(piece.name.to_ascii_lowercase().into_boxed_str()),
        zh: Box::leak(piece.zh.clone().into_boxed_str()),
        key: piece.keys[0],
        f0: part.f0,
        modes: leak(part.modes.iter().map(|m| (m[0], m[1], m[2])).collect()),
        glide: glide.semitones,
        glide_s: glide.seconds,
        glide_from: glide.from,
        glide_vel: glide.velocity,
        noise: noise.level,
        noise_lo: noise.lo,
        noise_hi: noise.hi,
        noise_s: noise.seconds,
        // 掐断在整套这一层做（各件各是一个 Kit）。
        choke: 0,
        gain: part.gain,
        pan: part.pan,
        bursts: noise.bursts,
        layers: leak(
            part.layers
                .iter()
                .map(|l| NoiseLayer {
                    lo: l.lo,
                    hi: l.hi,
                    fast: l.fast,
                    fast_s: l.fast_s,
                    slow: l.slow,
                    slow_s: l.slow_s,
                    pan: l.pan,
                    steep: l.steep,
                    sharp: l.sharp,
                })
                .collect(),
        ),
        phases: leak(part.phases.clone()),
        width: part.width,
        mode_vel: part.mode_vel,
        mode_attack: part.mode_attack,
        swell_above: part.swell_above,
        drive: part.drive,
    }
}

/// 检查一套的内容，组成 [`Loaded`]。`origin` 只用在报错里。
pub fn build(file: KitFile, path: PathBuf) -> Result<Loaded, String> {
    let origin = path.display();
    if file.format != FORMAT {
        return Err(format!("'{origin}' is not a yinse kit (format {FORMAT})"));
    }
    if file.version > VERSION {
        return Err(format!("'{origin}' was written by a newer muser (kit version {}, this muser reads up to {VERSION}): update muser", file.version));
    }
    if file.pieces.is_empty() {
        return Err(format!("'{origin}': a kit needs at least one piece"));
    }
    let mut seen: Vec<u8> = Vec::new();
    for p in &file.pieces {
        if p.keys.is_empty() || p.all_parts().next().is_none() || p.velocities.iter().any(|s| s.parts.is_empty()) {
            return Err(format!("'{origin}': piece '{}' needs at least one key and one part (in every velocity layer)", p.name));
        }
        if p.velocities.iter().any(|s| !(s.at > 0.0 && s.at <= 1.0)) || p.velocities.windows(2).any(|w| w[1].at <= w[0].at) {
            return Err(format!("'{origin}': piece '{}': velocity layers need rising `at` values in 0 … 1", p.name));
        }
        if p.levels.len() == 1 || p.levels.windows(2).any(|w| w[1][0] <= w[0][0]) || p.levels.iter().flatten().any(|v| !v.is_finite()) {
            return Err(format!("'{origin}': piece '{}': `levels` needs two or more points with rising velocities", p.name));
        }
        if let Some(k) = p.keys.iter().find(|k| seen.contains(k) || **k > 127) {
            return Err(format!("'{origin}': key {k} of piece '{}' is out of range or already taken by another piece", p.name));
        }
        seen.extend(&p.keys);
        for part in p.all_parts() {
            if part.modes.is_empty() && part.layers.is_empty() && part.noise.is_none_or(|n| n.level <= 0.0) {
                return Err(format!("'{origin}': a part of piece '{}' has neither modes nor noise", p.name));
            }
            if part.f0 <= 0.0 || !part.f0.is_finite() || part.modes.iter().flatten().any(|v| !v.is_finite()) {
                return Err(format!("'{origin}': piece '{}' has a part with a bad frequency", p.name));
            }
            if part.velocity.db.len() == 1 || part.velocity.db.windows(2).any(|w| w[1][0] <= w[0][0]) {
                return Err(format!("'{origin}': piece '{}': a velocity table needs two or more points with rising velocities", p.name));
            }
        }
        if let Some(e) = p.eq.iter().find(|e| !["peak", "high_shelf", "low_shelf"].contains(&e.shape.as_str())) {
            return Err(format!("'{origin}': piece '{}': eq shape '{}' is not peak / high_shelf / low_shelf", p.name, e.shape));
        }
    }
    if let Some(k) = file.pieces.iter().flat_map(|p| &p.chokes).find(|k| !seen.contains(k)) {
        return Err(format!("'{origin}': chokes key {k}, which no piece of this kit has"));
    }
    let defs = file.pieces.iter().map(|p| p.all_parts().map(|part| leak(vec![def(p, part)])).collect()).collect();
    Ok(Loaded { path, file, defs })
}

type Cached = (std::time::SystemTime, &'static Loaded);
static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// `yinse:` 后面的路径是不是一套打击乐（只看文件头）。
pub fn is_kit(path: &str) -> bool {
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = std::fs::File::open(crate::modal::resolve(path)).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains(FORMAT)
}

/// 读一套（`yinse:` 后面的路径），同一文件没改过就不再读。
/// shortcut: 每读一次新版本就留下一份（&'static），和 [`crate::modal::load`] 一样。
pub fn load(path: &str) -> Result<&'static Loaded, String> {
    let path = crate::modal::resolve(path);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|e| format!("cannot read yinse kit '{}': {e}", path.display()))?;
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(&(t, kit)) = cache.get(&path) {
        if t == modified {
            return Ok(kit);
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read yinse kit '{}': {e}", path.display()))?;
    let file: KitFile = serde_json::from_str(&text).map_err(|e| format!("'{}' is not a yinse kit: {e}", path.display()))?;
    let kit: &'static Loaded = Box::leak(Box::new(build(file, path.clone())?));
    cache.insert(path, (modified, kit));
    Ok(kit)
}

impl Loaded {
    /// 名字、MIDI 键或 GM 鼓件名 → 第几件。
    pub fn index(&self, name: &str) -> Option<usize> {
        let n = name.trim().to_ascii_lowercase();
        let by_key = |key: u8| self.file.pieces.iter().position(|p| p.keys.contains(&key));
        self.file
            .pieces
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&n))
            .or_else(|| n.parse::<u8>().ok().and_then(by_key))
            .or_else(|| crate::params::piece_key(&n).and_then(by_key))
    }

    /// 这一套自己的键（不算 `fallback` 的）。
    pub fn keys(&self) -> Vec<u8> {
        let mut keys: Vec<u8> = self.file.pieces.iter().flat_map(|p| p.keys.iter().copied()).collect();
        keys.sort_unstable();
        keys
    }

    /// `36 dagu (大鼓), 37 gubian (鼓边) …`
    pub fn pieces_text(&self) -> String {
        let mut list: Vec<&Piece> = self.file.pieces.iter().collect();
        list.sort_by_key(|p| p.keys[0]);
        list.iter()
            .map(|p| if p.zh.is_empty() { format!("{} {}", p.keys[0], p.name) } else { format!("{} {} ({})", p.keys[0], p.name, p.zh) })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 一套加上每件的设定（`instrument_params`）。
#[derive(Clone)]
pub(crate) struct KitPatch {
    pub kit: &'static Loaded,
    /// 每件的设定；声像没动过的话各个部分用各自量到的。
    pub set: Vec<PieceSet>,
}

impl KitPatch {
    pub fn new(kit: &'static Loaded) -> Self {
        let set = kit
            .file
            .pieces
            .iter()
            .map(|p| PieceSet { level: 1.0, tune: 0.0, decay: 1.0, tone: 0.5, pan: p.all_parts().next().map_or(0.0, |part| part.pan) })
            .collect();
        Self { kit, set }
    }

    /// 第 `piece` 件的一个部分的设定。
    fn part_set(&self, piece: usize, part: &Part) -> PieceSet {
        let (p, s) = (&self.kit.file.pieces[piece], self.set[piece]);
        let moved = (s.pan - p.all_parts().next().map_or(0.0, |first| first.pan)).abs() > 1e-9;
        PieceSet { pan: if moved { s.pan } else { part.pan }, ..s }
    }
}

struct Sounding {
    /// 每个部分一个 [`Kit`]（[`Piece::all_parts`] 的顺序）。
    parts: Vec<Kit>,
    /// 力度分层的件：每层在什么力度量的、是 `parts` 里的哪一段。
    strokes: Vec<(f64, core::ops::Range<usize>)>,
    eq: Vec<Filter<f32>>,
    /// 敲它的时候掐断哪几件。
    chokes: Vec<usize>,
}

/// 一套打击乐的发声。
pub(crate) struct KitInstrument {
    kit: &'static Loaded,
    pieces: Vec<Sounding>,
    fallback: Option<Box<dyn Playable>>,
    scratch: AudioBuffer<f32>,
    sum: AudioBuffer<f32>,
    comp_env: f64,
    sr: f64,
}

impl KitInstrument {
    pub fn new(patch: &KitPatch, fallback: Option<Box<dyn Playable>>) -> Self {
        let kit = patch.kit;
        let pieces = kit
            .file
            .pieces
            .iter()
            .enumerate()
            .map(|(i, p)| Sounding {
                parts: p
                    .all_parts()
                    .enumerate()
                    .map(|(j, part)| {
                        Kit::new(KitParams {
                            defs: kit.defs[i][j],
                            set: vec![patch.part_set(i, part)],
                            steep: part.steep,
                            compress: None,
                            noise_seed: part.seed,
                            max_hits: 24,
                        })
                    })
                    .collect(),
                strokes: p
                    .velocities
                    .iter()
                    .scan(0, |at, s| {
                        let range = *at..*at + s.parts.len();
                        *at = range.end;
                        Some((s.at, range))
                    })
                    .collect(),
                eq: p
                    .eq
                    .iter()
                    .map(|e| {
                        let shape = match e.shape.as_str() {
                            "high_shelf" => FilterShape::HighShelf { gain_db: e.gain_db },
                            "low_shelf" => FilterShape::LowShelf { gain_db: e.gain_db },
                            _ => FilterShape::Peak { gain_db: e.gain_db },
                        };
                        Filter::new(shape, e.hz, e.q)
                    })
                    .collect(),
                chokes: p.chokes.iter().filter_map(|k| kit.file.pieces.iter().position(|o| o.keys.contains(k))).collect(),
            })
            .collect();
        Self { kit, pieces, fallback, scratch: AudioBuffer::new(2, 0), sum: AudioBuffer::new(2, 0), comp_env: 0.0, sr: 48_000.0 }
    }
}

fn add(to: &mut AudioBuffer<f32>, from: &AudioBuffer<f32>) {
    for c in 0..to.num_channels() {
        for (y, x) in to.channel_mut(c).iter_mut().zip(from.channel(c)) {
            *y += *x;
        }
    }
}

impl Playable for KitInstrument {
    fn prepare(&mut self, spec: &ProcessSpec) {
        if let Some(f) = &mut self.fallback {
            f.prepare(spec);
        }
        for p in &mut self.pieces {
            p.parts.iter_mut().for_each(|k| k.prepare(spec));
            p.eq.iter_mut().for_each(|f| f.prepare(spec));
        }
        self.scratch = AudioBuffer::new(spec.num_channels.max(1), spec.max_block_size);
        self.sum = AudioBuffer::new(spec.num_channels.max(1), spec.max_block_size);
        self.sr = spec.sample_rate;
        self.comp_env = 0.0;
    }

    fn note_on(&mut self, note: NoteRef, velocity: f32, accent: bool) {
        let Some(i) = self.kit.file.pieces.iter().position(|p| p.keys.contains(&note.key)) else {
            if let (Some(f), Some(spec)) = (&mut self.fallback, &self.kit.file.fallback) {
                f.note_on(note, velocity.clamp(0.0, 1.0).powf(spec.velocity as f32), accent);
            }
            return;
        };
        for c in 0..self.pieces[i].chokes.len() {
            let target = self.pieces[i].chokes[c];
            self.pieces[target].parts.iter_mut().for_each(Kit::choke);
        }
        let piece = &self.kit.file.pieces[i];
        let key = NoteRef::new(note.channel, piece.keys[0]);
        let sounding = &mut self.pieces[i];
        if sounding.strokes.is_empty() {
            for (kit, part) in sounding.parts.iter_mut().zip(&piece.parts) {
                kit.note_on(key, part.velocity.apply(velocity), accent);
            }
            return;
        }
        // 力度分层：落在两层之间的两层一起敲，各乘 (1 − x)^0.75、x^0.75（音色的此消彼长）。
        // [`Kit`] 的音量是力度的 1.4 次方，所以交给它的是 1.4 次方根。
        let v = f64::from(velocity.clamp(0.0, 1.0));
        let strokes = &sounding.strokes;
        let above = strokes.iter().position(|s| s.0 >= v).unwrap_or(strokes.len());
        let measured = !piece.levels.is_empty();
        // 这一层相对它自己量的那个力度该放大多少。
        let own = |k: usize| {
            if measured {
                10f64.powf((piece.level_at(v) - piece.level_at(strokes[k].0)) / 20.0)
            } else {
                (v / strokes[k].0).powf(1.4).min(10f64.powf(3.0 / 20.0))
            }
        };
        let mut mix = [(0, 0.0); 2];
        if above == 0 || above == strokes.len() {
            let k = above.saturating_sub(1).min(strokes.len() - 1);
            mix[0] = (k, own(k));
        } else {
            let x = (v - strokes[above - 1].0) / (strokes[above].0 - strokes[above - 1].0);
            let (lo, hi) = ((1.0 - x).powf(0.75), x.powf(0.75));
            // 两层照自己的音量加起来（中间的力度主要是重的那一层变小了，轻的那一层垫在下面：对着录音比过，
            // 比把两层先拉到一样响再混更像），再整体乘一个数让合起来的音量是录音在这个力度的音量（按两层不相干估）。
            let scale = if measured {
                let (a, b) = (10f64.powf(piece.level_at(strokes[above - 1].0) / 20.0) * lo, 10f64.powf(piece.level_at(strokes[above].0) / 20.0) * hi);
                10f64.powf(piece.level_at(v) / 20.0) / (a * a + b * b).sqrt().max(1e-30)
            } else {
                1.0
            };
            mix = [(above - 1, lo * scale), (above, hi * scale)];
        }
        for (k, gain) in mix {
            if gain > 0.003 {
                for kit in &mut sounding.parts[strokes[k].1.clone()] {
                    kit.note_on(key, gain.powf(1.0 / 1.4) as f32, accent);
                }
            }
        }
    }

    fn note_off(&mut self, note: NoteRef, velocity: f32) {
        if !self.kit.file.pieces.iter().any(|p| p.keys.contains(&note.key)) {
            if let Some(f) = &mut self.fallback {
                f.note_off(note, velocity);
            }
        }
    }

    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        match &mut self.fallback {
            Some(f) => f.process(buffer),
            None => buffer.clear(),
        }
        let (n, channels) = (buffer.num_frames(), buffer.num_channels());
        if self.scratch.num_frames() < n || self.scratch.num_channels() < channels {
            self.scratch = AudioBuffer::new(channels, n);
            self.sum = AudioBuffer::new(channels, n);
        }
        for p in &mut self.pieces {
            if p.parts.iter().all(Kit::idle) {
                continue;
            }
            if p.eq.is_empty() {
                for kit in p.parts.iter_mut().filter(|k| !k.idle()) {
                    self.scratch.set_num_frames(n);
                    kit.process(&mut self.scratch);
                    add(buffer, &self.scratch);
                }
            } else {
                self.sum.set_num_frames(n);
                self.sum.clear();
                for kit in p.parts.iter_mut().filter(|k| !k.idle()) {
                    self.scratch.set_num_frames(n);
                    kit.process(&mut self.scratch);
                    add(&mut self.sum, &self.scratch);
                }
                p.eq.iter_mut().for_each(|f| f.process(&mut self.sum));
                add(buffer, &self.sum);
            }
        }
        // 压缩：峰值包络（快起慢落），超过门限的按比例压下去，再整体补回来。
        if let Some(c) = self.kit.file.compress.filter(|c| c.ratio > 1.0) {
            let thr = 10f64.powf(c.threshold_db / 20.0);
            let makeup = 10f64.powf(c.makeup_db / 20.0);
            let (ka, kr) = (1.0 - (-1.0 / (c.attack_s * self.sr)).exp(), 1.0 - (-1.0 / (c.release_s * self.sr)).exp());
            for i in 0..n {
                let peak = (0..channels).map(|ch| f64::from(buffer.channel(ch)[i].abs())).fold(0.0, f64::max);
                self.comp_env += (peak - self.comp_env) * if peak > self.comp_env { ka } else { kr };
                let g = (makeup * if self.comp_env > thr { (thr / self.comp_env).powf(1.0 - 1.0 / c.ratio) } else { 1.0 }) as f32;
                for ch in 0..channels {
                    buffer.channel_mut(ch)[i] *= g;
                }
            }
        }
    }

    /// 自动化：`<件>.<参数>`，如 `daluo.tune`；下一下起作用。这一套没有的件交给 `fallback`。
    fn set_param(&mut self, name: &str, value: f64) {
        let Some((piece, param)) = name.split_once('.') else { return };
        match self.kit.index(piece) {
            Some(i) => {
                let key = self.kit.file.pieces[i].keys[0];
                let name = format!("{key}.{param}");
                self.pieces[i].parts.iter_mut().for_each(|k| k.set_param(&name, value));
            }
            None => {
                if let Some(f) = &mut self.fallback {
                    f.set_param(name, value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一套小的：一面往下滑的锣、一面往上滑的小锣、一只两部分的鼓（鼓腔按力度表、鼓槌只在重击时有）、一对互相掐断的镲。
    const FIXTURE: &str = r#"{"format":"muser-yinse-kit","version":1,"name":"test kit",
      "compress":{"threshold_db":-14,"ratio":3.5,"attack_s":0.00002,"release_s":0.1,"makeup_db":7},
      "pieces":[
        {"name":"gong","zh":"大锣","keys":[49],"parts":[{"f0":250,"gain":4,"width":0.5,"modes":[[1,1,2.5],[2.98,0.6,1.0]],
          "glide":{"semitones":-5,"seconds":0.6,"from":0.32,"velocity":2},
          "layers":[{"lo":2500,"hi":5000,"fast":0,"fast_s":0.1,"slow":1.0,"slow_s":0.5}]}]},
        {"name":"small_gong","keys":[52],"parts":[{"f0":684,"gain":3,"modes":[[1,1,3.0]],"glide":{"semitones":3.5,"seconds":0.2,"from":1,"velocity":1.5}}]},
        {"name":"drum","keys":[36,35],"parts":[
          {"f0":90,"gain":3,"modes":[[1,1,0.8],[1.6,0.4,0.5]],"velocity":{"db":[[0.4,-7.9],[0.7,-1.1],[0.95,0]],"line":true,"floor_db":-40}},
          {"f0":2000,"gain":1,"modes":[],"layers":[{"lo":1000,"hi":5000,"fast":3,"fast_s":0.01,"slow":0,"slow_s":1,"steep":true}],"velocity":{"power":2.4,"at":0.95}}]},
        {"name":"tom","keys":[45],"levels":[[0.3,-30],[0.6,-18],[0.9,-6]],"velocities":[
          {"at":0.3,"parts":[{"f0":120,"gain":0.2,"modes":[[1,1,0.5]]}]},
          {"at":0.9,"parts":[{"f0":150,"gain":3.2,"modes":[[1,1,0.5]]},{"f0":3000,"gain":1,"modes":[],"layers":[{"lo":1600,"hi":5000,"fast":2,"fast_s":0.01,"slow":0,"slow_s":1}]}]}]},
        {"name":"open","keys":[46],"chokes":[46],"parts":[{"f0":8000,"gain":0.5,"steep":true,"width":1,"modes":[],"layers":[{"lo":4000,"hi":12000,"fast":1,"fast_s":0.02,"slow":1,"slow_s":0.5}]}]},
        {"name":"closed","keys":[42],"chokes":[42,46],"eq":[{"shape":"high_shelf","hz":9000,"q":0.7,"gain_db":3}],
          "parts":[{"f0":8000,"gain":0.5,"steep":true,"seed":1,"modes":[],"layers":[{"lo":4000,"hi":12000,"fast":1,"fast_s":0.03,"slow":0,"slow_s":1}]}]}
      ]}"#;

    fn kit() -> &'static Loaded {
        Box::leak(Box::new(build(serde_json::from_str(FIXTURE).unwrap(), PathBuf::from("fixture")).unwrap()))
    }

    fn play(hits: &[(f64, u8, f32)], seconds: f64) -> Vec<f32> {
        play_on(kit(), hits, seconds)
    }

    fn play_on(kit: &'static Loaded, hits: &[(f64, u8, f32)], seconds: f64) -> Vec<f32> {
        let mut k = KitInstrument::new(&KitPatch::new(kit), None);
        k.prepare(&ProcessSpec { sample_rate: 48_000.0, num_channels: 1, max_block_size: 480 });
        let mut out = Vec::new();
        let mut b = AudioBuffer::<f32>::new(1, 480);
        for block in 0..(seconds * 100.0) as usize {
            for &(_, key, v) in hits.iter().filter(|h| (h.0 * 100.0).round() as usize == block) {
                k.note_on(NoteRef::new(9, key), v, false);
            }
            k.process(&mut b);
            out.extend_from_slice(b.channel(0));
        }
        out
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn every_piece_sounds_and_stops() {
        for p in &kit().file.pieces {
            let x = play(&[(0.0, p.keys[0], 0.8)], 8.0);
            let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.01 && peak < 1.0, "{} peak {peak}", p.name);
            assert!(x[x.len() - 480..].iter().all(|v| v.abs() < 1e-4), "{} still ringing", p.name);
        }
    }

    #[test]
    fn the_big_gong_falls_and_the_small_gong_rises() {
        let pitch = |x: &[f32], at: f64, around: f64| {
            let w = &x[(at * 48_000.0) as usize..][..9_600];
            (0..400)
                .map(|k| around * (0.85 + 0.3 * f64::from(k) / 400.0))
                .max_by(|a, b| {
                    let e = |hz: f64| {
                        let (mut re, mut im) = (0.0, 0.0);
                        for (i, &v) in w.iter().enumerate() {
                            let p = core::f64::consts::TAU * hz * i as f64 / 48_000.0;
                            re += f64::from(v) * p.cos();
                            im += f64::from(v) * p.sin();
                        }
                        re * re + im * im
                    };
                    e(*a).total_cmp(&e(*b))
                })
                .unwrap()
        };
        let big = play(&[(0.0, 49, 0.8)], 2.0);
        assert!(pitch(&big, 1.2, 245.0) < pitch(&big, 0.0, 245.0) * 0.96, "大锣往下");
        let small = play(&[(0.0, 52, 0.8)], 1.5);
        assert!(pitch(&small, 0.8, 650.0) > pitch(&small, 0.0, 650.0) * 1.04, "小锣往上");
        // 轻敲几乎不滑（滑多少跟着力度的平方）。
        let soft = play(&[(0.0, 49, 0.3)], 2.0);
        assert!(pitch(&soft, 1.2, 250.0) > pitch(&soft, 0.0, 250.0) * 0.97, "轻敲的大锣几乎不滑");
    }

    #[test]
    fn parts_follow_their_own_velocity_curves() {
        // 鼓腔轻敲只轻几 dB（量到的表），鼓槌那一层轻敲几乎没有。
        let band = |x: &[f32], lo: f64, hi: f64| {
            let n = 4_800;
            let (mut e, mut hz) = (0.0, lo);
            while hz < hi {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, &v) in x[..n].iter().enumerate() {
                    let p = core::f64::consts::TAU * hz * i as f64 / 48_000.0;
                    re += f64::from(v) * p.cos();
                    im += f64::from(v) * p.sin();
                }
                e += re * re + im * im;
                hz += (hi - lo) / 24.0;
            }
            10.0 * e.log10()
        };
        let (hard, soft) = (play(&[(0.0, 36, 0.95)], 0.5), play(&[(0.0, 36, 0.4)], 0.5));
        let (body, stick) = (band(&hard, 60.0, 160.0) - band(&soft, 60.0, 160.0), band(&hard, 1_500.0, 4_500.0) - band(&soft, 1_500.0, 4_500.0));
        assert!(stick > body + 6.0, "鼓槌随力度变得比鼓腔多：鼓腔 {body:.1} dB，鼓槌 {stick:.1} dB");
        // 第二个键敲的是同一件。
        assert!((rms(&play(&[(0.0, 35, 0.8)], 0.5)) / rms(&play(&[(0.0, 36, 0.8)], 0.5)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn velocity_layers_cross_fade_and_follow_the_measured_levels() {
        // 轻的一层是 120 Hz、重的一层是 150 Hz 加鼓槌：轻敲只有 120 Hz，重敲只有 150 Hz，中间两个都有。
        let tone = |x: &[f32], hz: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &v) in x[..9_600].iter().enumerate() {
                let p = core::f64::consts::TAU * hz * i as f64 / 48_000.0;
                re += f64::from(v) * p.cos();
                im += f64::from(v) * p.sin();
            }
            (re * re + im * im).sqrt()
        };
        // 不带整套的压缩（它会把重击压下来，音量就比不出来了）。
        let mut file: KitFile = serde_json::from_str(FIXTURE).unwrap();
        file.compress = None;
        let dry: &'static Loaded = Box::leak(Box::new(build(file, PathBuf::from("fixture")).unwrap()));
        let play = |hits: &[(f64, u8, f32)], seconds: f64| play_on(dry, hits, seconds);
        let (soft, middle, hard) = (play(&[(0.0, 45, 0.3)], 0.5), play(&[(0.0, 45, 0.6)], 0.5), play(&[(0.0, 45, 0.9)], 0.5));
        assert!(tone(&soft, 120.0) > 10.0 * tone(&soft, 150.0) && tone(&hard, 150.0) > 10.0 * tone(&hard, 120.0));
        assert!(tone(&middle, 120.0) > 0.03 * tone(&middle, 150.0) && tone(&middle, 150.0) > tone(&middle, 120.0));
        // 中间的音量照 `levels`：比重的轻 12 dB 上下（两层此消彼长加出来的会偏到重的那一层去）。
        let db = 20.0 * (rms(&middle[..9_600]) / rms(&hard[..9_600])).log10();
        assert!((db + 12.0).abs() < 2.5, "middle is {db:.1} dB under hard");
        // 比最轻的一层还轻：照它小下去；比最重的还重：大不了 3 dB。
        assert!(rms(&play(&[(0.0, 45, 0.15)], 0.5)) < 0.6 * rms(&soft));
        assert!(rms(&play(&[(0.0, 45, 1.0)], 0.5)[..9_600]) < 1.45 * rms(&hard[..9_600]));
    }

    #[test]
    fn the_closed_hat_chokes_the_open_one() {
        let ringing = play(&[(0.0, 46, 0.9)], 0.6);
        let choked = play(&[(0.0, 46, 0.9), (0.2, 42, 0.9)], 0.6);
        let tail = |x: &[f32]| rms(&x[(0.4 * 48_000.0) as usize..(0.6 * 48_000.0) as usize]);
        assert!(tail(&choked) < tail(&ringing) * 0.1, "{} vs {}", tail(&choked), tail(&ringing));
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        let bad = |edit: &dyn Fn(&mut KitFile)| {
            let mut f: KitFile = serde_json::from_str(FIXTURE).unwrap();
            edit(&mut f);
            build(f, PathBuf::from("bad")).err().unwrap_or_default()
        };
        assert!(bad(&|f| f.version = VERSION + 1).contains("newer muser"));
        assert!(bad(&|f| f.pieces[1].keys = vec![49]).contains("already taken"));
        assert!(bad(&|f| f.pieces[4].chokes = vec![99]).contains("chokes key 99"));
        assert!(bad(&|f| f.pieces[5].eq[0].shape = "bell".into()).contains("eq shape"));
        assert!(bad(&|f| f.pieces[3].velocities[1].at = 0.2).contains("rising `at`"));
        assert!(bad(&|f| f.pieces[3].levels.truncate(1)).contains("`levels`"));
        assert!(bad(&|f| f.pieces.clear()).contains("at least one piece"));
    }

    #[test]
    fn pieces_are_found_by_name_key_or_gm_name() {
        let k = kit();
        assert_eq!(k.index("gong"), Some(0));
        assert_eq!(k.index("52"), Some(1));
        assert_eq!(k.index("35"), Some(2));
        assert_eq!(k.index("kick"), Some(2));
        assert_eq!(k.index("snare"), None);
        assert_eq!(k.keys(), vec![35, 36, 42, 45, 46, 49, 52]);
    }
}
