//! 内置乐器参数表。
//!
//! 每类乐器开放少量、有音乐意义的参数，名称在各乐器间统一（attack / release / tone /
//! vibrato_* …），单位用真实单位（Hz、秒、音分、半音）。工程里的 `instrument_params`
//! 在所选乐器/预设的音色之上覆盖这些值。
//!
//! 每个参数一份定义：规格（给 CLI/MCP/文档）、读取当前值、写入。校验和描述都来自这里。

use manguebyte::dsp::instruments::poly::{FilterModel, FilterParams};
use manguebyte::dsp::instruments::{
    AcidParams, BowedParams, ChoirParams, FmParams, MalletParams, OrganParams, Percussion,
    PercussionDecay, PercussionHarmonic, Piece, PolySynthParams, StringParams, WindParams,
};
use serde::Serialize;
use serde_json::{json, Value};

/// 参数的取值类型。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    /// 数值。`names` 非空时也接受这些名字（第 i 个名字 = 数值 i）。
    Number {
        min: f64,
        max: f64,
        unit: &'static str,
        integer: bool,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        names: &'static [&'static str],
    },
    /// 从固定选项中选一个（字符串）。
    Choice { choices: &'static [&'static str] },
    /// 开关。
    Bool,
    /// 风琴拉杆：9 位数字字符串，每位 0-8，如 "888000000"。
    Drawbars,
}

/// 一个乐器参数的规格。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct InstParamSpec {
    pub name: &'static str,
    #[serde(flatten)]
    pub kind: ParamKind,
    pub description: &'static str,
}

/// 规格 + 该乐器当前（预设）的取值，用于展示。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamInfo {
    #[serde(flatten)]
    pub spec: InstParamSpec,
    /// 该乐器/预设当前的取值；鼓组按鼓件设置时为 null。
    pub current: Value,
}

type Set<P> = fn(&mut P, f64) -> Result<(), String>;
type SetText<P> = fn(&mut P, &str) -> Result<(), String>;

pub(crate) enum Setter<P: 'static> {
    Num(Set<P>),
    Text(SetText<P>),
}

pub(crate) struct ParamDef<P: 'static> {
    pub spec: InstParamSpec,
    pub get: fn(&P) -> Value,
    pub set: Setter<P>,
}

const fn number(min: f64, max: f64, unit: &'static str) -> ParamKind {
    ParamKind::Number { min, max, unit, integer: false, names: &[] }
}
const fn integer(min: f64, max: f64) -> ParamKind {
    ParamKind::Number { min, max, unit: "", integer: true, names: &[] }
}

/// 简单的 f64 字段参数。
macro_rules! num {
    ($name:literal, $min:expr, $max:expr, $unit:literal, $desc:literal, |$p:ident| $field:expr) => {
        ParamDef {
            spec: InstParamSpec { name: $name, kind: number($min, $max, $unit), description: $desc },
            get: |$p| json!($field),
            set: Setter::Num(|$p, v| {
                $field = v;
                Ok(())
            }),
        }
    };
}

/// usize 字段参数。
macro_rules! int {
    ($name:literal, $min:expr, $max:expr, $desc:literal, |$p:ident| $field:expr) => {
        ParamDef {
            spec: InstParamSpec { name: $name, kind: integer($min, $max), description: $desc },
            get: |$p| json!($field),
            set: Setter::Num(|$p, v| {
                $field = v as usize;
                Ok(())
            }),
        }
    };
}

// ---------------------------------------------------------------------------
// 各类乐器
// ---------------------------------------------------------------------------

/// poly 当前生效的滤波器：第一个未关闭的。
fn active_filter(p: &mut PolySynthParams) -> Result<&mut FilterParams, String> {
    p.filters
        .iter_mut()
        .find(|f| !matches!(f.model, FilterModel::Off))
        .ok_or_else(|| "该音色没有启用滤波器，此参数无效；换一个带滤波的预设".to_string())
}
fn active_filter_ref(p: &PolySynthParams) -> Option<&FilterParams> {
    p.filters.iter().find(|f| !matches!(f.model, FilterModel::Off))
}

pub(crate) static POLY: &[ParamDef<PolySynthParams>] = &[
    num!("attack", 0.0, 20.0, "s", "音量起音时间。0.001-0.01 干脆；0.1-0.5 柔和；>1 慢慢浮现（pad）", |p| p.envelopes[0].attack),
    num!("decay", 0.0, 20.0, "s", "音量从峰值落到 sustain 的时间", |p| p.envelopes[0].decay),
    num!("sustain", 0.0, 1.0, "", "按住时保持的音量比例。0 = 只有起音（pluck）；1 = 一直满音量（pad/风琴）", |p| p.envelopes[0].sustain),
    num!("release", 0.0, 20.0, "s", "松开后余音长度。0.05-0.2 利落；0.5-2 有尾巴；>3 很长的铺底", |p| p.envelopes[0].release),
    ParamDef {
        spec: InstParamSpec { name: "cutoff", kind: number(20.0, 20000.0, "Hz"), description: "滤波截止频率：越低越闷越暗，越高越亮。贝斯 200-1500；lead 1500-6000" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.cutoff)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.cutoff = v;
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "resonance", kind: number(0.0, 1.0, ""), description: "滤波谐振：截止频率处的突起。0.2-0.4 有个性；0.7+ 啸叫感（acid）" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.resonance)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.resonance = v;
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "filter_env", kind: number(-8.0, 8.0, "oct"), description: "滤波包络把截止频率推开多少个八度。正值 = 起音时变亮再回落（'哇'声）；0 = 不动" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.env_amount)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.env_amount = v;
            Ok(())
        }),
    },
    num!("filter_attack", 0.0, 20.0, "s", "滤波包络起音时间", |p| p.envelopes[1].attack),
    num!("filter_decay", 0.0, 20.0, "s", "滤波包络衰减时间：越短'哇'声越短促", |p| p.envelopes[1].decay),
    num!("filter_sustain", 0.0, 1.0, "", "滤波包络保持比例", |p| p.envelopes[1].sustain),
    num!("filter_release", 0.0, 20.0, "s", "滤波包络释放时间", |p| p.envelopes[1].release),
    num!("glide", 0.0, 5.0, "s", "滑音时间；只在单音/连奏型预设上起作用（重叠的音符之间滑过去）", |p| p.glide),
];

fn fm_max(p: &FmParams, f: fn(&manguebyte::dsp::instruments::FmOperator) -> f64) -> Value {
    json!(p.operators.iter().map(f).fold(0.0_f64, f64::max))
}

pub(crate) static FM: &[ParamDef<FmParams>] = &[
    ParamDef {
        spec: InstParamSpec { name: "attack", kind: number(0.0, 10.0, "s"), description: "所有算子的起音时间（同时会改变音色的演变）" },
        get: |p| fm_max(p, |o| o.attack),
        set: Setter::Num(|p, v| {
            p.operators.iter_mut().for_each(|o| o.attack = v);
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "release", kind: number(0.001, 100.0, "s"), description: "所有算子的释放时间" },
        get: |p| fm_max(p, |o| o.release),
        set: Setter::Num(|p, v| {
            p.operators.iter_mut().for_each(|o| o.release = v);
            Ok(())
        }),
    },
];

pub(crate) static STRING: &[ParamDef<StringParams>] = &[
    num!("decay", 0.05, 60.0, "s", "按住时弦振动持续多久（中央 C 衰减 60dB 的时间）。短 = 闷音；长 = 余音绵长", |p| p.decay),
    num!("release", 0.01, 10.0, "s", "松开（止音）后的衰减时间", |p| p.release),
    num!("tone", 200.0, 20000.0, "Hz", "最终低通：琴体/拾音的明暗。2000 暗；5000 常规；10000+ 很亮", |p| p.tone),
    num!("pluck_brightness", 100.0, 20000.0, "Hz", "拨弦的亮度：指甲/拨片亮（5000+），指腹暗（1000-2000）", |p| p.pluck_brightness),
    num!("pluck_position", 0.02, 0.5, "", "拨弦位置（离琴桥的比例）。0.05 靠近琴桥，薄而亮；0.5 在弦中间，圆而空", |p| p.pluck_position),
    num!("buzz", 0.0, 1.0, "", "弦打品丝的嗡声。0 干净；0.3+ 有击弦感（slap）", |p| p.buzz),
];

pub(crate) static ACID: &[ParamDef<AcidParams>] = &[
    num!("cutoff", 30.0, 8000.0, "Hz", "滤波截止频率（包络关闭时）。200-400 闷；800+ 亮。最适合做自动化扫频", |p| p.cutoff),
    num!("resonance", 0.0, 1.0, "", "谐振。0.5 常规；0.8+ 典型的 303 '吱吱'声", |p| p.resonance),
    num!("env_mod", 0.0, 1.0, "", "滤波包络深度：每个音起音时滤波打开多少", |p| p.env_mod),
    num!("decay", 0.05, 4.0, "s", "滤波包络衰减：短 = 短促的'啵'；长 = 拖长的'哇'", |p| p.decay),
    num!("accent", 0.0, 1.0, "", "重音强度：标了 `\"accent\": true` 的音符更响、滤波开得更大、更\"吱\"", |p| p.accent),
    num!("slide", 0.005, 1.0, "s", "滑音时间：前后两个音重叠时从前一个音滑到后一个音", |p| p.slide),
    num!("drive_db", -12.0, 24.0, "dB", "过载：滤波前的增益。6-12 更肥更脏", |p| p.drive_db),
    num!("tune", -24.0, 24.0, "st", "整体移调（半音）", |p| p.tune),
];

fn drawbars_text(p: &OrganParams) -> Value {
    json!(p.drawbars.iter().map(|d| char::from(b'0' + d.round().clamp(0.0, 8.0) as u8)).collect::<String>())
}

pub(crate) static ORGAN: &[ParamDef<OrganParams>] = &[
    ParamDef {
        spec: InstParamSpec { name: "drawbars", kind: ParamKind::Drawbars, description: "九根拉杆（16' 5⅓' 8' 4' 2⅔' 2' 1⅗' 1⅓' 1'），每位 0-8。\"888000000\" 经典爵士；\"838000000\" 柔和；\"888888888\" 全开" },
        get: drawbars_text,
        set: Setter::Text(|p, s| {
            let digits: Vec<u32> = s.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_digit(10).unwrap_or(99)).collect();
            if digits.len() != 9 || digits.iter().any(|d| *d > 8) {
                return Err(format!("应为 9 位、每位 0-8 的数字串，如 \"888000000\"，实际 \"{s}\""));
            }
            for (bar, d) in p.drawbars.iter_mut().zip(digits) {
                *bar = f64::from(d);
            }
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "percussion", kind: ParamKind::Choice { choices: &["off", "second", "third"] }, description: "打击泛音：按键瞬间加一声短促的二次/三次谐波，爵士风琴的'叮'" },
        get: |p| match p.percussion {
            None => json!("off"),
            Some(x) if x.harmonic == PercussionHarmonic::Second => json!("second"),
            Some(_) => json!("third"),
        },
        set: Setter::Num(|p, v| {
            p.percussion = match v as usize {
                0 => None,
                i => Some(Percussion {
                    harmonic: if i == 1 { PercussionHarmonic::Second } else { PercussionHarmonic::Third },
                    decay: PercussionDecay::Fast,
                    soft: false,
                }),
            };
            Ok(())
        }),
    },
    num!("click", 0.0, 1.0, "", "按键咔哒声。0 干净；0.5 复古", |p| p.click),
];

pub(crate) static MALLET: &[ParamDef<MalletParams>] = &[
    num!("decay", 0.05, 60.0, "s", "敲击后自然衰减的时间（中央 C）", |p| p.decay),
    num!("release", 0.01, 60.0, "s", "松开后的止音时间（颤音琴/钢片琴/管钟有制音器）", |p| p.release),
    num!("softness", 1.0, 8.0, "", "槌头软硬：1 硬而亮；4-8 软而圆", |p| p.softness),
    num!("strike_position", 0.0, 1.0, "", "敲击位置。0.5 在中心（最饱满）；靠边泛音更多", |p| p.strike_position),
    num!("motor_rate", 0.0, 16.0, "Hz", "颤音琴电机转速（颤音速度），0 = 关闭", |p| p.motor_rate),
    num!("motor_depth", 0.0, 1.0, "", "颤音琴电机颤音深度", |p| p.motor_depth),
];

pub(crate) static BOWED: &[ParamDef<BowedParams>] = &[
    num!("attack", 0.005, 2.0, "s", "运弓起音时间。0.02 有力；0.3+ 柔和渐入", |p| p.attack),
    num!("release", 0.15, 10.0, "s", "松开后的衰减时间。下限 0.15：更短时上游弓弦模型会数值发散", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "揉弦速度。5-6 常规", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 100.0, "cent", "揉弦深度（上下各多少音分）。0 不揉；10-20 常规；40+ 夸张", |p| p.vibrato_depth),
    num!("vibrato_delay", 0.0, 4.0, "s", "音开始后多久才开始揉弦", |p| p.vibrato_delay),
    num!("tone", 500.0, 20000.0, "Hz", "最终低通：明暗", |p| p.tone),
    num!("pressure", 0.0, 1.0, "", "弓压：小 = 轻柔带气声；大 = 浓密带颗粒感", |p| p.pressure),
    int!("players", 1.0, 4.0, "每个音由几把琴演奏：1 独奏；3-4 组合", |p| p.players),
];

pub(crate) static WIND: &[ParamDef<WindParams>] = &[
    num!("attack", 0.002, 1.0, "s", "气息建立的时间：短 = 吐音清楚；长 = 柔和进入", |p| p.attack),
    num!("release", 0.005, 2.0, "s", "松开后的收尾时间", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "颤音速度。4-6 常规", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 100.0, "cent", "颤音深度（音分）。0 直音；10-25 常规", |p| p.vibrato_depth),
    num!("vibrato_delay", 0.0, 4.0, "s", "音开始后多久才开始颤音", |p| p.vibrato_delay),
    num!("tone", 500.0, 20000.0, "Hz", "最终低通：明暗", |p| p.tone),
    num!("breath_noise", 0.0, 1.0, "", "气声：越大越多气流噪声", |p| p.noise),
    num!("brassiness", 0.0, 1.0, "", "铜管的'炸'（小号/长号在大声时变亮）", |p| p.brassiness),
    ParamDef {
        spec: InstParamSpec { name: "portamento", kind: ParamKind::Bool, description: "连音（重叠的音符）之间滑音，像长号的滑管" },
        get: |p| json!(p.portamento),
        set: Setter::Num(|p, v| {
            p.portamento = v != 0.0;
            Ok(())
        }),
    },
];

pub(crate) static CHOIR: &[ParamDef<ChoirParams>] = &[
    ParamDef {
        spec: InstParamSpec {
            name: "vowel",
            kind: ParamKind::Number { min: 0.0, max: 4.0, unit: "", integer: false, names: &["a", "e", "i", "o", "u"] },
            description: "元音：\"a\" \"e\" \"i\" \"o\" \"u\"，或 0-4 之间的数（如 1.5 = e 与 i 之间）。适合做自动化",
        },
        get: |p| json!(p.vowel),
        set: Setter::Num(|p, v| {
            p.vowel = v;
            Ok(())
        }),
    },
    num!("attack", 0.005, 2.0, "s", "起音时间", |p| p.attack),
    num!("release", 0.01, 4.0, "s", "松开后淡出时间", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "颤音速度", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 200.0, "cent", "颤音深度（音分）", |p| p.vibrato_depth),
    num!("brightness", 500.0, 12000.0, "Hz", "音色亮度", |p| p.brightness),
    num!("breathiness", 0.0, 1.0, "", "气声", |p| p.breathiness),
    int!("singers", 1.0, 6.0, "每个音由几个人唱：1 独唱；4-6 合唱", |p| p.singers),
    num!("formant_shift", 0.7, 1.4, "", "共振峰缩放：<1 更深更大的声音；>1 更细更小", |p| p.formant_shift),
];

pub(crate) static PIECE: &[ParamDef<Piece>] = &[
    num!("tune", -24.0, 24.0, "st", "鼓件调音（半音）。底鼓 -3 更沉；军鼓 +2 更紧", |p| p.tune),
    num!("decay", 0.1, 10.0, "x", "余音长度（原长的倍数）。0.5 短促；2 更长（808 长尾底鼓）", |p| p.decay),
    num!("level", 0.0, 4.0, "x", "音量（倍数），1 = 原样", |p| p.level),
    num!("tone", 0.0, 1.0, "", "音色（随鼓件而异，一般是明暗）", |p| p.tone),
    num!("snap", 0.0, 1.0, "", "起音的'脆'（随鼓件而异，军鼓为响弦）", |p| p.snap),
    num!("texture", 0.0, 1.0, "", "质感（随鼓件而异）", |p| p.texture),
    num!("pan", -1.0, 1.0, "", "声像：-1 左 ~ 1 右", |p| p.pan),
];

/// 鼓件别名 → GM 鼓键。
pub(crate) const PIECE_ALIASES: &[(&str, u8)] = &[
    ("kick", 36), ("kick2", 35), ("side_stick", 37), ("rim", 37), ("snare", 38), ("clap", 39),
    ("snare2", 40), ("floor_tom_low", 41), ("closed_hat", 42), ("floor_tom", 43),
    ("pedal_hat", 44), ("tom_low", 45), ("open_hat", 46), ("tom_mid_low", 47), ("tom_mid", 48),
    ("crash", 49), ("tom_high", 50), ("ride", 51), ("china", 52), ("ride_bell", 53),
    ("tambourine", 54), ("splash", 55), ("cowbell", 56), ("crash2", 57), ("ride2", 59),
    ("bongo_high", 60), ("bongo_low", 61), ("conga_mute", 62), ("conga_high", 63), ("conga_low", 64),
    ("cabasa", 69), ("shaker", 70), ("maracas", 70), ("guiro_short", 73), ("guiro_long", 74), ("claves", 75),
];

pub(crate) fn piece_key(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    n.parse::<u8>()
        .ok()
        .or_else(|| PIECE_ALIASES.iter().find(|(a, _)| *a == n).map(|(_, k)| *k))
}

// ---------------------------------------------------------------------------
// 通用逻辑
// ---------------------------------------------------------------------------

fn to_number(spec: &InstParamSpec, v: &Value) -> Result<f64, String> {
    match spec.kind {
        ParamKind::Number { min, max, unit, integer, names } => {
            let x = if let Some(s) = v.as_str() {
                names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(s.trim()))
                    .map(|i| i as f64)
                    .ok_or_else(|| {
                        if names.is_empty() {
                            format!("应为数字，实际 \"{s}\"")
                        } else {
                            format!("应为数字或 {}，实际 \"{s}\"", names.join("/"))
                        }
                    })?
            } else {
                v.as_f64().ok_or_else(|| format!("应为数字，实际 {v}"))?
            };
            if !x.is_finite() || x < min || x > max {
                let unit = if unit.is_empty() { String::new() } else { format!(" {unit}") };
                return Err(format!("{x} 超出范围 [{min}, {max}]{unit}"));
            }
            if integer && x.fract() != 0.0 {
                return Err(format!("应为整数，实际 {x}"));
            }
            Ok(x)
        }
        ParamKind::Choice { choices } => {
            let s = v.as_str().ok_or_else(|| format!("应为字符串 {}，实际 {v}", choices.join("/")))?;
            choices
                .iter()
                .position(|c| c.eq_ignore_ascii_case(s.trim()))
                .map(|i| i as f64)
                .ok_or_else(|| format!("\"{s}\" 无效，可选 {}", choices.join(" / ")))
        }
        ParamKind::Bool => v
            .as_bool()
            .map(|b| if b { 1.0 } else { 0.0 })
            .ok_or_else(|| format!("应为 true/false，实际 {v}")),
        ParamKind::Drawbars => Err("内部错误：拉杆参数应走文本写入".into()),
    }
}

fn apply_one<P>(def: &ParamDef<P>, p: &mut P, v: &Value) -> Result<(), String> {
    match def.set {
        Setter::Num(f) => f(p, to_number(&def.spec, v)?),
        Setter::Text(f) => f(p, v.as_str().ok_or_else(|| format!("应为字符串，实际 {v}"))?),
    }
}

/// 把一组参数应用到乐器参数结构上。错误信息以参数名开头。
pub(crate) fn apply<P>(
    defs: &[ParamDef<P>],
    p: &mut P,
    params: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    for (key, value) in params {
        let def = defs.iter().find(|d| d.spec.name == key).ok_or_else(|| {
            format!(
                "未知参数 '{key}'，可用: {}",
                defs.iter().map(|d| d.spec.name).collect::<Vec<_>>().join(", ")
            )
        })?;
        apply_one(def, p, value).map_err(|e| format!("{key}: {e}"))?;
    }
    Ok(())
}

/// 参数规格 + 当前值。
pub(crate) fn describe<P>(defs: &[ParamDef<P>], p: &P) -> Vec<ParamInfo> {
    defs.iter()
        .map(|d| ParamInfo { spec: d.spec, current: (d.get)(p) })
        .collect()
}

/// 只要规格（不带当前值）。
pub(crate) fn specs<P>(defs: &[ParamDef<P>]) -> Vec<ParamInfo> {
    defs.iter()
        .map(|d| ParamInfo { spec: d.spec, current: Value::Null })
        .collect()
}
