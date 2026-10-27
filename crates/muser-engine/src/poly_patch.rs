//! poly 合成器的完整音色（patch）：JSON 与 [`PolySynthParams`] 互转、按范围校验、给编辑器的结构说明。
//!
//! 音色 JSON 的格式由 muser 定义并保持稳定（字段名、选项名都写在这里，不直接用上游的内部名字），
//! 存进音色库、写进工程、以后分享到社区的都是它。所有字段都来自这一张描述表：转换、校验、
//! `muser poly schema` 与编辑器页面的控件都由它派生。
//!
//! 套用（[`apply`]）时只改给出的字段：数组按下标覆盖（`oscillators[1]` 只改第二个振荡器），
//! 但 `matrix`（调制矩阵）整体替换：给出的就是全部调制，其余槽位清空。

use manguebyte::dsp::instruments::poly::{
    EnvParams, FilterModel, FilterParams, FilterRouting, LfoParams, ModDestination, ModSlot, ModSource, NoiseParams,
    OscParams, OscSource, PolySynthParams, SubParams, SubWave, TableShape, VoiceMode, Wave, ENVELOPES, FILTERS, LFOS,
    MAX_UNISON, MOD_SLOTS, OSCILLATORS,
};
use manguebyte::dsp::{LfoShape, NoiseColor};
use serde_json::{json, Map, Value};

use crate::params::ParamKind;

pub use manguebyte::dsp::instruments::poly::PolySynthParams as Params;

/// 一个字段：名称、取值类型（含范围）、说明，以及读写。
pub struct Field<T> {
    pub name: &'static str,
    pub kind: ParamKind,
    pub description: &'static str,
    get: fn(&T) -> Value,
    set: fn(&mut T, &Value) -> Result<(), String>,
}

fn number(v: &Value, min: f64, max: f64) -> Result<f64, String> {
    let x = v.as_f64().ok_or_else(|| format!("应为数字，得到 {v}"))?;
    if !(min..=max).contains(&x) {
        return Err(format!("{x} 超出范围 [{min}, {max}]"));
    }
    Ok(x)
}

fn integer(v: &Value, min: u8, max: u8) -> Result<u8, String> {
    let x = v.as_u64().filter(|x| (u64::from(min)..=u64::from(max)).contains(x));
    x.map(|x| x as u8).ok_or_else(|| format!("应为 {min} 到 {max} 的整数，得到 {v}"))
}

fn boolean(v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("应为 true / false，得到 {v}"))
}

/// 选项：名字与对应的上游取值，按下标一一对应。
type Choices<E> = (&'static [&'static str], &'static [E]);

fn name_of<E: PartialEq>((names, values): Choices<E>, e: &E) -> Value {
    values.iter().position(|x| x == e).map_or(Value::Null, |i| json!(names[i]))
}

fn pick<E: Copy>((names, values): Choices<E>, v: &Value) -> Result<E, String> {
    let s = v.as_str().ok_or_else(|| format!("应为字符串，可选: {}", names.join(", ")))?;
    names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(s.trim()))
        .map(|i| values[i])
        .ok_or_else(|| format!("未知选项 '{s}'，可选: {}", names.join(", ")))
}

macro_rules! num {
    ($name:literal, $min:expr, $max:expr, $unit:literal, $desc:literal, |$p:ident| $f:expr) => {
        Field {
            name: $name,
            kind: ParamKind::Number { min: $min, max: $max, unit: $unit, integer: false, names: &[] },
            description: $desc,
            get: |$p| json!($f),
            set: |$p, v| {
                $f = number(v, $min, $max)?;
                Ok(())
            },
        }
    };
}

macro_rules! int {
    ($name:literal, $min:expr, $max:expr, $desc:literal, |$p:ident| $f:expr) => {
        Field {
            name: $name,
            kind: ParamKind::Number { min: $min as f64, max: $max as f64, unit: "", integer: true, names: &[] },
            description: $desc,
            get: |$p| json!($f),
            set: |$p, v| {
                $f = integer(v, $min, $max)?;
                Ok(())
            },
        }
    };
}

macro_rules! flag {
    ($name:literal, $desc:literal, |$p:ident| $f:expr) => {
        Field {
            name: $name,
            kind: ParamKind::Bool,
            description: $desc,
            get: |$p| json!($f),
            set: |$p, v| {
                $f = boolean(v)?;
                Ok(())
            },
        }
    };
}

macro_rules! choice {
    ($name:literal, $choices:expr, $desc:literal, |$p:ident| $f:expr) => {
        Field {
            name: $name,
            kind: ParamKind::Choice { choices: $choices.0 },
            description: $desc,
            get: |$p| name_of($choices, &$f),
            set: |$p, v| {
                $f = pick($choices, v)?;
                Ok(())
            },
        }
    };
}

// —— 选项名（音色文件格式的一部分：改名会让已有音色读不出来）——

pub const WAVES: Choices<Wave> = (&["sine", "triangle", "saw", "pulse", "wavetable"], &[Wave::Sine, Wave::Triangle, Wave::Saw, Wave::Pulse, Wave::Table]);
pub const TABLES: Choices<TableShape> = (
    &["basic", "harmonics", "sync", "fold", "vowel", "choir", "digital", "glass", "organ", "metal", "spectral", "resonant"],
    &[
        TableShape::Basic,
        TableShape::Harmonics,
        TableShape::Sync,
        TableShape::Fold,
        TableShape::Vowel,
        TableShape::Choir,
        TableShape::Digital,
        TableShape::Glass,
        TableShape::Organ,
        TableShape::Metal,
        TableShape::Spectral,
        TableShape::Resonant,
    ],
);
pub const OSC_SOURCES: Choices<OscSource> = (&["osc1", "osc2", "osc3"], &[OscSource::Osc1, OscSource::Osc2, OscSource::Osc3]);
pub const SUB_WAVES: Choices<SubWave> = (&["sine", "triangle", "square", "saw"], &[SubWave::Sine, SubWave::Triangle, SubWave::Square, SubWave::Saw]);
pub const NOISE_COLORS: Choices<NoiseColor> = (&["white", "pink", "brown"], &[NoiseColor::White, NoiseColor::Pink, NoiseColor::Brown]);
pub const FILTER_MODELS: Choices<FilterModel> = (
    &[
        "off", "ladder_lp24", "ladder_lp12", "ladder_hp24", "ladder_bp12", "svf_lp", "svf_bp", "svf_hp", "svf_notch", "sem_notch",
        "sem_band", "ms20_lp", "ms20_hp", "comb_plus", "comb_minus", "formant",
    ],
    &[
        FilterModel::Off,
        FilterModel::LadderLp24,
        FilterModel::LadderLp12,
        FilterModel::LadderHp24,
        FilterModel::LadderBp12,
        FilterModel::SvfLp,
        FilterModel::SvfBp,
        FilterModel::SvfHp,
        FilterModel::SvfNotch,
        FilterModel::SemNotch,
        FilterModel::SemBand,
        FilterModel::Ms20Lp,
        FilterModel::Ms20Hp,
        FilterModel::CombPlus,
        FilterModel::CombMinus,
        FilterModel::Formant,
    ],
);
pub const ROUTINGS: Choices<FilterRouting> = (&["serial", "parallel"], &[FilterRouting::Serial, FilterRouting::Parallel]);
pub const LFO_SHAPES: Choices<LfoShape> = (
    &["sine", "triangle", "saw_up", "saw_down", "square", "sample_and_hold"],
    &[LfoShape::Sine, LfoShape::Triangle, LfoShape::SawUp, LfoShape::SawDown, LfoShape::Square, LfoShape::SampleAndHold],
);
pub const MOD_SOURCES: Choices<ModSource> = (
    &[
        "off", "env1", "env2", "env3", "env4", "lfo1", "lfo2", "lfo3", "velocity", "accent", "aftertouch", "mod_wheel", "key", "random",
        "brightness",
    ],
    &[
        ModSource::Off,
        ModSource::Env1,
        ModSource::Env2,
        ModSource::Env3,
        ModSource::Env4,
        ModSource::Lfo1,
        ModSource::Lfo2,
        ModSource::Lfo3,
        ModSource::Velocity,
        ModSource::Accent,
        ModSource::Aftertouch,
        ModSource::ModWheel,
        ModSource::Key,
        ModSource::Random,
        ModSource::Brightness,
    ],
);
pub const MOD_DESTINATIONS: Choices<ModDestination> = (
    &[
        "off", "pitch", "osc1_pitch", "osc2_pitch", "osc3_pitch", "osc1_level", "osc2_level", "osc3_level", "sub_level", "noise_level",
        "osc1_shape", "osc2_shape", "osc3_shape", "osc1_fm", "osc2_fm", "osc3_fm", "detune", "cutoff", "cutoff1", "cutoff2", "resonance",
        "morph", "drive", "volume", "pan", "lfo1_rate", "lfo2_rate", "lfo3_rate",
    ],
    &[
        ModDestination::Off,
        ModDestination::Pitch,
        ModDestination::Osc1Pitch,
        ModDestination::Osc2Pitch,
        ModDestination::Osc3Pitch,
        ModDestination::Osc1Level,
        ModDestination::Osc2Level,
        ModDestination::Osc3Level,
        ModDestination::SubLevel,
        ModDestination::NoiseLevel,
        ModDestination::Osc1Shape,
        ModDestination::Osc2Shape,
        ModDestination::Osc3Shape,
        ModDestination::Osc1Fm,
        ModDestination::Osc2Fm,
        ModDestination::Osc3Fm,
        ModDestination::Detune,
        ModDestination::Cutoff,
        ModDestination::Cutoff1,
        ModDestination::Cutoff2,
        ModDestination::Resonance,
        ModDestination::Morph,
        ModDestination::Drive,
        ModDestination::Volume,
        ModDestination::Pan,
        ModDestination::Lfo1Rate,
        ModDestination::Lfo2Rate,
        ModDestination::Lfo3Rate,
    ],
);
pub const MODES: Choices<VoiceMode> = (&["polyphonic", "mono", "legato"], &[VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato]);

// —— 描述表（范围与上游 `limited()` 一致）——

pub static OSC: &[Field<OscParams>] = &[
    choice!("wave", WAVES, "波形。pulse 的 shape 是脉宽；wavetable 的 shape 是波表位置", |p| p.wave),
    choice!("table", TABLES, "wave 为 wavetable 时用的波表", |p| p.table),
    num!("level", 0.0, 1.0, "", "电平。0 听不到，但仍可调制别的振荡器", |p| p.level),
    num!("semitones", -48.0, 48.0, "半音", "移调", |p| p.semitones),
    num!("cents", -100.0, 100.0, "音分", "微调", |p| p.cents),
    num!("shape", 0.0, 1.0, "", "pulse 的脉宽（0 = 方波）或波表位置", |p| p.shape),
    int!("unison", 1, MAX_UNISON as u8, "unison 声部数：叠加多个略微失谐的声部，更厚更宽", |p| p.unison),
    num!("detune", 0.0, 1.0, "", "unison 失谐量（最外侧 ±100 音分 × detune²）", |p| p.detune),
    num!("spread", 0.0, 1.0, "", "unison 声部在立体声中铺开的宽度", |p| p.spread),
    num!("pan", -1.0, 1.0, "", "声像：-1 左，1 右", |p| p.pan),
    num!("phase", 0.0, 1.0, "周", "每个音开始时的相位", |p| p.phase),
    num!("random_phase", 0.0, 1.0, "", "起始相位的随机量：0 起音干脆可重复，1 自由相位（模拟合成器式）", |p| p.random_phase),
    flag!("sync", "硬同步到振荡器 1（振荡器 1 自身忽略）", |p| p.sync),
    choice!("mod_source", OSC_SOURCES, "fm / ring 用哪个振荡器做调制源", |p| p.mod_source),
    num!("fm", 0.0, 16.0, "rad", "被 mod_source 相位调制的深度（FM 指数）", |p| p.fm),
    num!("ring", 0.0, 1.0, "", "与 mod_source 环形调制：1 只剩两者的乘积", |p| p.ring),
];

pub static SUB: &[Field<SubParams>] = &[
    choice!("wave", SUB_WAVES, "sub 振荡器波形", |p| p.wave),
    int!("octaves", 1, 2, "比振荡器 1 低几个八度", |p| p.octaves),
    num!("level", 0.0, 1.0, "", "电平", |p| p.level),
];

pub static NOISE: &[Field<NoiseParams>] = &[
    choice!("color", NOISE_COLORS, "噪声颜色：white 最亮，brown 最暗", |p| p.color),
    num!("level", 0.0, 1.0, "", "电平", |p| p.level),
];

pub static FILTER: &[Field<FilterParams>] = &[
    choice!("model", FILTER_MODELS, "滤波器模型：ladder（经典四极）、svf、sem（可变形）、ms20（激进）、comb（梳状）、formant（元音）；off = 不用", |p| p.model),
    num!("cutoff", 20.0, 20000.0, "Hz", "截止频率（comb 为第一个共振；formant 为共振峰偏移，1 kHz 不变）", |p| p.cutoff),
    num!("resonance", 0.0, 1.0, "", "谐振。ladder / ms20 到 1 自激振荡", |p| p.resonance),
    num!("drive", 0.0, 36.0, "dB", "进入滤波器饱和的增益", |p| p.drive),
    num!("key_track", 0.0, 1.0, "", "截止跟随音高：1 = 每高一个八度截止也高一个八度", |p| p.key_track),
    num!("env_amount", -8.0, 8.0, "oct", "包络 2 推动截止的八度数", |p| p.env_amount),
    num!("morph", 0.0, 1.0, "", "sem 的模式、formant 的元音、comb 的阻尼；其它模型忽略", |p| p.morph),
];

pub static ENV: &[Field<EnvParams>] = &[
    num!("attack", 0.0, 20.0, "s", "起音时间", |p| p.attack),
    num!("decay", 0.0, 20.0, "s", "衰减到 sustain 的时间", |p| p.decay),
    num!("sustain", 0.0, 1.0, "", "按住时的保持电平", |p| p.sustain),
    num!("release", 0.0, 20.0, "s", "松开后的释放时间", |p| p.release),
    num!("attack_curve", -1.0, 1.0, "", "起音曲线", |p| p.attack_curve),
    num!("decay_curve", -1.0, 1.0, "", "衰减曲线", |p| p.decay_curve),
    num!("release_curve", -1.0, 1.0, "", "释放曲线", |p| p.release_curve),
];

pub static LFO: &[Field<LfoParams>] = &[
    choice!("shape", LFO_SHAPES, "LFO 波形；sample_and_hold 每周期取一个随机值", |p| p.shape),
    num!("rate", 0.01, 50.0, "Hz", "速率（beats 为 0 时）", |p| p.rate),
    num!("beats", 0.0, 64.0, "拍", "大于 0 时按拍子同步：一个周期的四分音符数（0.25 = 十六分，4 = 4/4 一小节）", |p| p.beats),
    flag!("retrigger", "每个音从 phase 重新开始；否则连续运行", |p| p.retrigger),
    num!("phase", 0.0, 1.0, "周", "重新开始时的相位", |p| p.phase),
    num!("fade_in", 0.0, 10.0, "s", "深度从 0 渐入的时间（延迟颤音）", |p| p.fade_in),
];

pub static SLOT: &[Field<ModSlot>] = &[
    choice!("source", MOD_SOURCES, "调制源", |p| p.source),
    choice!("via", MOD_SOURCES, "再乘上这个源（如 mod_wheel 控制颤音深度）；off = 不乘", |p| p.via),
    choice!("destination", MOD_DESTINATIONS, "调制目标", |p| p.destination),
    num!("amount", -48.0, 48.0, "", "深度（音高类目标为半音，cutoff 类为八度，其余为该参数的满量程）", |p| p.amount),
];

pub static VOICE: &[Field<PolySynthParams>] = &[
    choice!("mode", MODES, "polyphonic 复音（每个音一个声部）；mono 单音（新音接管声部、重新起音）；legato 单音且连奏时只滑音、不重新起音", |p| p.mode),
    num!("glide", 0.0, 5.0, "s", "滑音时间（mono / legato）", |p| p.glide),
    num!("velocity", 0.0, 1.0, "", "力度对音量的影响", |p| p.velocity),
    num!("accent_db", 0.0, 12.0, "dB", "重音提升", |p| p.accent_db),
    num!("saturation", 0.0, 1.0, "", "输出饱和", |p| p.saturation),
    num!("pan", -1.0, 1.0, "", "整体声像", |p| p.pan),
    num!("level", 0.0, 4.0, "", "整体电平", |p| p.level),
    choice!("routing", ROUTINGS, "两个滤波器：serial 先 1 后 2；parallel 并联相加", |p| p.routing),
];

fn object<T>(fields: &[Field<T>], p: &T) -> Value {
    Value::Object(fields.iter().map(|f| (f.name.to_string(), (f.get)(p))).collect())
}

fn set_object<T>(fields: &[Field<T>], p: &mut T, v: &Value, path: &str) -> Result<(), String> {
    let obj = v.as_object().ok_or_else(|| format!("{path}: 应为对象"))?;
    for (key, value) in obj {
        let f = fields.iter().find(|f| f.name == key).ok_or_else(|| {
            format!("{path}.{key}: 未知字段，可用: {}", fields.iter().map(|f| f.name).collect::<Vec<_>>().join(", "))
        })?;
        (f.set)(p, value).map_err(|e| format!("{path}.{key}: {e}"))?;
    }
    Ok(())
}

fn set_array<T, const N: usize>(fields: &[Field<T>], items: &mut [T; N], v: &Value, path: &str) -> Result<(), String> {
    let list = v.as_array().ok_or_else(|| format!("{path}: 应为数组（最多 {N} 项）"))?;
    if list.len() > N {
        return Err(format!("{path}: 最多 {N} 项，给了 {}", list.len()));
    }
    for (i, item) in list.iter().enumerate() {
        set_object(fields, &mut items[i], item, &format!("{path}[{i}]"))?;
    }
    Ok(())
}

/// 完整音色 JSON（全部字段；调制矩阵只列出在用的槽位）。
pub fn to_json(p: &PolySynthParams) -> Value {
    let mut out = match object(VOICE, p) {
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    out.insert("oscillators".into(), p.oscillators.iter().map(|o| object(OSC, o)).collect());
    out.insert("sub".into(), object(SUB, &p.sub));
    out.insert("noise".into(), object(NOISE, &p.noise));
    out.insert("filters".into(), p.filters.iter().map(|f| object(FILTER, f)).collect());
    out.insert("envelopes".into(), p.envelopes.iter().map(|e| object(ENV, e)).collect());
    out.insert("lfos".into(), p.lfos.iter().map(|l| object(LFO, l)).collect());
    out.insert("matrix".into(), p.matrix.iter().filter(|s| s.is_active()).map(|s| object(SLOT, s)).collect());
    Value::Object(out)
}

/// 把音色 JSON 套到 `p` 上：只改给出的字段（数组按下标），`matrix` 整体替换。出错时 `p` 不变。
pub fn apply(p: &mut PolySynthParams, patch: &Value) -> Result<(), String> {
    let obj = patch.as_object().ok_or("patch: 应为对象（见 muser poly schema）")?;
    let mut next = *p;
    let mut voice = Map::new();
    for (key, value) in obj {
        let path = format!("patch.{key}");
        match key.as_str() {
            "oscillators" => set_array(OSC, &mut next.oscillators, value, &path)?,
            "filters" => set_array(FILTER, &mut next.filters, value, &path)?,
            "envelopes" => set_array(ENV, &mut next.envelopes, value, &path)?,
            "lfos" => set_array(LFO, &mut next.lfos, value, &path)?,
            "sub" => set_object(SUB, &mut next.sub, value, &path)?,
            "noise" => set_object(NOISE, &mut next.noise, value, &path)?,
            "matrix" => {
                let mut slots = [ModSlot::EMPTY; MOD_SLOTS];
                set_array(SLOT, &mut slots, value, &path)?;
                next.matrix = slots;
            }
            _ if VOICE.iter().any(|f| f.name == key) => {
                voice.insert(key.clone(), value.clone());
            }
            _ => {
                return Err(format!(
                    "{path}: 未知字段，可用: oscillators, sub, noise, filters, envelopes, lfos, matrix, {}",
                    VOICE.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
                ))
            }
        }
    }
    set_object(VOICE, &mut next, &Value::Object(voice), "patch")?;
    *p = next;
    Ok(())
}

fn fields_json<T>(fields: &[Field<T>]) -> Value {
    fields.iter().map(|f| json!({ "name": f.name, "kind": f.kind, "description": f.description })).collect()
}

/// 音色的结构说明：每个分区的字段（类型、范围、选项、说明）与个数。编辑器据此生成控件。
pub fn schema() -> Value {
    json!({
        "sections": [
            { "key": "voice", "label": "整体", "fields": fields_json(VOICE) },
            { "key": "oscillators", "label": "振荡器", "count": OSCILLATORS, "fields": fields_json(OSC) },
            { "key": "sub", "label": "Sub 振荡器", "fields": fields_json(SUB) },
            { "key": "noise", "label": "噪声", "fields": fields_json(NOISE) },
            { "key": "filters", "label": "滤波器", "count": FILTERS, "fields": fields_json(FILTER) },
            { "key": "envelopes", "label": "包络", "count": ENVELOPES, "names": ["1 · 音量", "2 · 滤波", "3", "4"], "fields": fields_json(ENV) },
            { "key": "lfos", "label": "LFO", "count": LFOS, "fields": fields_json(LFO) },
            { "key": "matrix", "label": "调制矩阵", "count": MOD_SLOTS, "list": true, "fields": fields_json(SLOT) },
        ],
        "note": "数组按下标覆盖（只写要改的项）；matrix 是在用的调制列表，整体替换。",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use manguebyte::dsp::instruments::poly::PRESETS;

    /// 每个预设：导出成 JSON 再套到初始音色上，与预设完全相同（音色文件不丢信息）。
    #[test]
    fn round_trips_every_preset() {
        for preset in PRESETS.iter() {
            let mut p = PolySynthParams::default();
            apply(&mut p, &to_json(&preset.params)).unwrap_or_else(|e| panic!("{}: {e}", preset.name));
            let mut want = preset.params;
            // 不在用的调制槽位导出时省略，套回来是空槽位。
            for s in want.matrix.iter_mut().filter(|s| !s.is_active()) {
                *s = ModSlot::EMPTY;
            }
            assert_eq!(p, want, "{}", preset.name);
        }
    }

    #[test]
    fn partial_patch_and_matrix_replace() {
        let base = PRESETS.iter().find(|p| p.name == "Reese").unwrap().params;
        let mut p = base;
        apply(&mut p, &json!({ "oscillators": [{}, { "wave": "pulse" }], "glide": 0.2, "matrix": [{ "source": "lfo2", "destination": "pan", "amount": 0.5 }] })).unwrap();
        assert_eq!(p.oscillators[0], base.oscillators[0]);
        assert_eq!(p.oscillators[1].wave, Wave::Pulse);
        assert_eq!(p.oscillators[1].semitones, base.oscillators[1].semitones);
        assert_eq!(p.glide, 0.2);
        assert_eq!(p.matrix.iter().filter(|s| s.is_active()).count(), 1, "matrix 整体替换");
        assert_eq!((p.matrix[0].source, p.matrix[0].destination), (ModSource::Lfo2, ModDestination::Pan));
    }

    #[test]
    fn errors_name_the_field_and_leave_params_unchanged() {
        let mut p = PolySynthParams::default();
        let before = p;
        let err = |v: Value, p: &mut PolySynthParams| apply(p, &v).unwrap_err();
        assert!(err(json!({ "filters": [{ "cutoff": 5.0 }] }), &mut p).starts_with("patch.filters[0].cutoff: 5 超出范围"));
        assert!(err(json!({ "oscillators": [{ "wave": "zigzag" }] }), &mut p).contains("可选: sine"));
        assert!(err(json!({ "oscillators": [{ "unison": 17 }] }), &mut p).contains("1 到 16 的整数"));
        assert!(err(json!({ "lfos": [{}, {}, {}, {}] }), &mut p).contains("最多 3 项"));
        assert!(err(json!({ "wobble": 1 }), &mut p).contains("未知字段"));
        assert!(err(json!({ "glide": 0.5, "level": 99 }), &mut p).starts_with("patch.level"));
        assert_eq!(p, before, "出错时不改");
    }

    /// 工程里内嵌完整音色（synth.poly + patch）与直接用预设渲染完全相同。
    #[test]
    fn embedded_patch_renders_like_the_preset() {
        let preset = PRESETS.iter().find(|p| p.name == "Reese").unwrap();
        let track = |instrument: &str, params: Value| {
            json!({ "tempo": 120, "tracks": [{ "instrument": instrument, "instrument_params": params,
                "clips": [{ "start": 0, "length": 2, "notes": [{ "pitch": 40, "start": 0, "length": 1 }, { "pitch": 43, "start": 1, "length": 1 }] }] }] })
        };
        let render = |v: Value| crate::render(&serde_json::from_value(v).unwrap(), crate::RenderSettings { tail_seconds: 0.2, ..Default::default() }).unwrap();
        let a = render(track("poly.reese", json!({})));
        let b = render(track("synth.poly", json!({ "sound": "x", "patch": to_json(&preset.params) })));
        assert!((0..2).all(|c| a.channel(c) == b.channel(c)));
        assert!(crate::validate(&serde_json::from_value(track("synth.poly", json!({ "patch": { "glide": 9 } }))).unwrap())
            .unwrap_err()
            .to_string()
            .contains("patch.glide"));
    }

    #[test]
    fn choice_tables_line_up() {
        fn check<E>((n, v): Choices<E>) {
            assert_eq!(n.len(), v.len());
        }
        check(WAVES);
        check(TABLES);
        check(OSC_SOURCES);
        check(SUB_WAVES);
        check(NOISE_COLORS);
        check(FILTER_MODELS);
        check(ROUTINGS);
        check(LFO_SHAPES);
        check(MOD_SOURCES);
        check(MOD_DESTINATIONS);
        check(MODES);
        // 每个上游取值都有名字（上游加了新选项时这里会失败，提醒补上）。
        assert_eq!(FILTER_MODELS.1.len(), FilterModel::ALL.len());
        assert_eq!(MOD_SOURCES.1.len(), ModSource::ALL.len());
        assert_eq!(MOD_DESTINATIONS.1.len(), ModDestination::ALL.len());
        assert_eq!(TABLES.1.len(), TableShape::ALL.len());
        assert_eq!(WAVES.1.len(), Wave::ALL.len());
    }
}
