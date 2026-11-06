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
    let x = v.as_f64().ok_or_else(|| format!("expected a number, got {v}"))?;
    if !(min..=max).contains(&x) {
        return Err(format!("{x} out of range [{min}, {max}]"));
    }
    Ok(x)
}

fn integer(v: &Value, min: u8, max: u8) -> Result<u8, String> {
    let x = v.as_u64().filter(|x| (u64::from(min)..=u64::from(max)).contains(x));
    x.map(|x| x as u8).ok_or_else(|| format!("expected an integer from {min} to {max}, got {v}"))
}

fn boolean(v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("expected true / false, got {v}"))
}

/// 选项：名字与对应的上游取值，按下标一一对应。
type Choices<E> = (&'static [&'static str], &'static [E]);

fn name_of<E: PartialEq>((names, values): Choices<E>, e: &E) -> Value {
    values.iter().position(|x| x == e).map_or(Value::Null, |i| json!(names[i]))
}

fn pick<E: Copy>((names, values): Choices<E>, v: &Value) -> Result<E, String> {
    let s = v.as_str().ok_or_else(|| format!("expected a string, choose from: {}", names.join(", ")))?;
    names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(s.trim()))
        .map(|i| values[i])
        .ok_or_else(|| format!("unknown option '{s}', choose from: {}", names.join(", ")))
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
    choice!("wave", WAVES, "Waveform. For pulse, shape is the pulse width; for wavetable, shape is the wavetable position", |p| p.wave),
    choice!("table", TABLES, "Wavetable used when wave is wavetable", |p| p.table),
    num!("level", 0.0, 1.0, "", "Level. 0 is silent but the oscillator can still modulate others", |p| p.level),
    num!("semitones", -48.0, 48.0, "semitones", "Transpose", |p| p.semitones),
    num!("cents", -100.0, 100.0, "cents", "Fine tune", |p| p.cents),
    num!("shape", 0.0, 1.0, "", "Pulse width for pulse (0 = square wave) or the wavetable position", |p| p.shape),
    int!("unison", 1, MAX_UNISON as u8, "Number of unison voices: stacks several slightly detuned voices for a thicker, wider sound", |p| p.unison),
    num!("detune", 0.0, 1.0, "", "Unison detune amount (outermost voices ±100 cents × detune²)", |p| p.detune),
    num!("spread", 0.0, 1.0, "", "Stereo width the unison voices spread across", |p| p.spread),
    num!("pan", -1.0, 1.0, "", "Pan: -1 left, 1 right", |p| p.pan),
    num!("phase", 0.0, 1.0, "cycles", "Phase at the start of each note", |p| p.phase),
    num!("random_phase", 0.0, 1.0, "", "Amount of random start phase: 0 gives a crisp, repeatable attack, 1 is free-running phase (analog-synth style)", |p| p.random_phase),
    flag!("sync", "Hard sync to oscillator 1 (ignored on oscillator 1 itself)", |p| p.sync),
    choice!("mod_source", OSC_SOURCES, "Which oscillator is the modulation source for fm / ring", |p| p.mod_source),
    num!("fm", 0.0, 16.0, "rad", "Depth of phase modulation by mod_source (FM index)", |p| p.fm),
    num!("ring", 0.0, 1.0, "", "Ring modulation with mod_source: 1 leaves only the product of the two", |p| p.ring),
];

pub static SUB: &[Field<SubParams>] = &[
    choice!("wave", SUB_WAVES, "Sub oscillator waveform", |p| p.wave),
    int!("octaves", 1, 2, "Octaves below oscillator 1", |p| p.octaves),
    num!("level", 0.0, 1.0, "", "Level", |p| p.level),
];

pub static NOISE: &[Field<NoiseParams>] = &[
    choice!("color", NOISE_COLORS, "Noise color: white is brightest, brown is darkest", |p| p.color),
    num!("level", 0.0, 1.0, "", "Level", |p| p.level),
];

pub static FILTER: &[Field<FilterParams>] = &[
    choice!("model", FILTER_MODELS, "Filter model: ladder (classic four-pole), svf, sem (morphable), ms20 (aggressive), comb, formant (vowel); off = unused", |p| p.model),
    num!("cutoff", 20.0, 20000.0, "Hz", "Cutoff (for comb, the first resonance; for formant, the formant shift, unchanged at 1 kHz)", |p| p.cutoff),
    num!("resonance", 0.0, 1.0, "", "Resonance. ladder / ms20 self-oscillate at 1", |p| p.resonance),
    num!("drive", 0.0, 36.0, "dB", "Gain into the filter saturation", |p| p.drive),
    num!("key_track", 0.0, 1.0, "", "Cutoff follows pitch: 1 = cutoff rises one octave per octave of pitch", |p| p.key_track),
    num!("env_amount", -8.0, 8.0, "oct", "Octaves that envelope 2 pushes the cutoff", |p| p.env_amount),
    num!("morph", 0.0, 1.0, "", "Mode for sem, vowel for formant, damping for comb; ignored by other models", |p| p.morph),
];

pub static ENV: &[Field<EnvParams>] = &[
    num!("attack", 0.0, 20.0, "s", "Attack time", |p| p.attack),
    num!("decay", 0.0, 20.0, "s", "Time to decay to sustain", |p| p.decay),
    num!("sustain", 0.0, 1.0, "", "Level held while the key is down", |p| p.sustain),
    num!("release", 0.0, 20.0, "s", "Release time after the key is released", |p| p.release),
    num!("attack_curve", -1.0, 1.0, "", "Attack curve", |p| p.attack_curve),
    num!("decay_curve", -1.0, 1.0, "", "Decay curve", |p| p.decay_curve),
    num!("release_curve", -1.0, 1.0, "", "Release curve", |p| p.release_curve),
];

pub static LFO: &[Field<LfoParams>] = &[
    choice!("shape", LFO_SHAPES, "LFO waveform; sample_and_hold picks a random value each cycle", |p| p.shape),
    num!("rate", 0.01, 50.0, "Hz", "Rate (when beats is 0)", |p| p.rate),
    num!("beats", 0.0, 64.0, "beats", "When above 0, sync to the tempo: quarter notes per cycle (0.25 = a sixteenth, 4 = one bar of 4/4)", |p| p.beats),
    flag!("retrigger", "Restart from phase on every note; otherwise free-running", |p| p.retrigger),
    num!("phase", 0.0, 1.0, "cycles", "Phase on restart", |p| p.phase),
    num!("fade_in", 0.0, 10.0, "s", "Time for the depth to fade in from 0 (delayed vibrato)", |p| p.fade_in),
];

pub static SLOT: &[Field<ModSlot>] = &[
    choice!("source", MOD_SOURCES, "Modulation source", |p| p.source),
    choice!("via", MOD_SOURCES, "Multiply by this source as well (e.g. mod_wheel controls vibrato depth); off = no multiplication", |p| p.via),
    choice!("destination", MOD_DESTINATIONS, "Modulation destination", |p| p.destination),
    num!("amount", -48.0, 48.0, "", "Amount (semitones for pitch destinations, octaves for cutoff, full range of the parameter otherwise)", |p| p.amount),
];

pub static VOICE: &[Field<PolySynthParams>] = &[
    choice!("mode", MODES, "polyphonic (one voice per note); mono (a new note takes over the voice and retriggers); legato (mono, and overlapping notes only glide without retriggering)", |p| p.mode),
    num!("glide", 0.0, 5.0, "s", "Glide time (mono / legato)", |p| p.glide),
    num!("velocity", 0.0, 1.0, "", "Effect of velocity on volume", |p| p.velocity),
    num!("accent_db", 0.0, 12.0, "dB", "Accent boost", |p| p.accent_db),
    num!("saturation", 0.0, 1.0, "", "Output saturation", |p| p.saturation),
    num!("pan", -1.0, 1.0, "", "Overall pan", |p| p.pan),
    num!("level", 0.0, 4.0, "", "Overall level", |p| p.level),
    choice!("routing", ROUTINGS, "Two filters: serial runs 1 then 2; parallel sums them side by side", |p| p.routing),
];

fn object<T>(fields: &[Field<T>], p: &T) -> Value {
    Value::Object(fields.iter().map(|f| (f.name.to_string(), (f.get)(p))).collect())
}

fn set_object<T>(fields: &[Field<T>], p: &mut T, v: &Value, path: &str) -> Result<(), String> {
    let obj = v.as_object().ok_or_else(|| format!("{path}: expected an object"))?;
    for (key, value) in obj {
        let f = fields.iter().find(|f| f.name == key).ok_or_else(|| {
            format!("{path}.{key}: unknown field, available: {}", fields.iter().map(|f| f.name).collect::<Vec<_>>().join(", "))
        })?;
        (f.set)(p, value).map_err(|e| format!("{path}.{key}: {e}"))?;
    }
    Ok(())
}

fn set_array<T, const N: usize>(fields: &[Field<T>], items: &mut [T; N], v: &Value, path: &str) -> Result<(), String> {
    let list = v.as_array().ok_or_else(|| format!("{path}: expected an array (at most {N} items)"))?;
    if list.len() > N {
        return Err(format!("{path}: at most {N} items, got {}", list.len()));
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
    let obj = patch.as_object().ok_or("patch: expected an object (see muser poly schema)")?;
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
                    "{path}: unknown field, available: oscillators, sub, noise, filters, envelopes, lfos, matrix, {}",
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
            { "key": "voice", "label": "Overall", "fields": fields_json(VOICE) },
            { "key": "oscillators", "label": "Oscillators", "count": OSCILLATORS, "fields": fields_json(OSC) },
            { "key": "sub", "label": "Sub oscillator", "fields": fields_json(SUB) },
            { "key": "noise", "label": "Noise", "fields": fields_json(NOISE) },
            { "key": "filters", "label": "Filters", "count": FILTERS, "fields": fields_json(FILTER) },
            { "key": "envelopes", "label": "Envelopes", "count": ENVELOPES, "names": ["1 · Volume", "2 · Filter", "3", "4"], "fields": fields_json(ENV) },
            { "key": "lfos", "label": "LFO", "count": LFOS, "fields": fields_json(LFO) },
            { "key": "matrix", "label": "Modulation matrix", "count": MOD_SLOTS, "list": true, "fields": fields_json(SLOT) },
        ],
        "note": "Arrays are overwritten by index (write only the items to change); matrix is the list of active modulations and is replaced as a whole.",
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
        assert_eq!(p.matrix.iter().filter(|s| s.is_active()).count(), 1, "matrix replaced as a whole");
        assert_eq!((p.matrix[0].source, p.matrix[0].destination), (ModSource::Lfo2, ModDestination::Pan));
    }

    #[test]
    fn errors_name_the_field_and_leave_params_unchanged() {
        let mut p = PolySynthParams::default();
        let before = p;
        let err = |v: Value, p: &mut PolySynthParams| apply(p, &v).unwrap_err();
        assert!(err(json!({ "filters": [{ "cutoff": 5.0 }] }), &mut p).starts_with("patch.filters[0].cutoff: 5 out of range"));
        assert!(err(json!({ "oscillators": [{ "wave": "zigzag" }] }), &mut p).contains("choose from: sine"));
        assert!(err(json!({ "oscillators": [{ "unison": 17 }] }), &mut p).contains("an integer from 1 to 16"));
        assert!(err(json!({ "lfos": [{}, {}, {}, {}] }), &mut p).contains("at most 3 items"));
        assert!(err(json!({ "wobble": 1 }), &mut p).contains("unknown field"));
        assert!(err(json!({ "glide": 0.5, "level": 99 }), &mut p).starts_with("patch.level"));
        assert_eq!(p, before, "unchanged on error");
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
