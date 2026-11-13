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

/// 参数能否被自动化、以及变化何时被听到。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Automatable {
    /// 正在发声的音符也会立即跟随。
    Live,
    /// 只影响之后开始的音符（如弓弦的揉弦、鼓件的调音）。
    NextNote,
    /// 不能自动化（开关、选项、整数、拉杆串）。
    No,
}

/// 规格 + 该乐器当前（预设）的取值，用于展示。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamInfo {
    #[serde(flatten)]
    pub spec: InstParamSpec,
    /// 该乐器/预设当前的取值；鼓组按鼓件设置时为 null。
    pub current: Value,
    /// 自动化时的行为。
    pub automation: Automatable,
}

pub(crate) fn automatable(spec: &InstParamSpec, next_note: &[&str]) -> Automatable {
    match spec.kind {
        ParamKind::Number { integer: false, .. } if next_note.contains(&spec.name) => Automatable::NextNote,
        ParamKind::Number { integer: false, .. } => Automatable::Live,
        _ => Automatable::No,
    }
}

// 各类乐器中"只影响下一个音"的参数（依据 manguebyte 各 Voice::set_params 的文档）。
pub(crate) const NEXT_FM: &[&str] = &["attack", "release"];
pub(crate) const NEXT_STRING: &[&str] = &["decay", "release", "pluck_brightness", "pluck_position", "buzz"];
pub(crate) const NEXT_ELECTRIC: &[&str] = &["decay", "release", "pluck_brightness", "pluck_position", "mute"];
pub(crate) const NEXT_MALLET: &[&str] = &["decay", "release", "softness", "strike_position"];
pub(crate) const NEXT_BOWED: &[&str] = &["attack", "release", "vibrato_rate", "vibrato_depth", "vibrato_delay"];
pub(crate) const NEXT_PIECE: &[&str] = &["tune", "decay", "level", "tone", "snap", "texture", "pan"];

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
        .ok_or_else(|| "This sound has no filter enabled, so this parameter has no effect; pick a preset with a filter".to_string())
}
fn active_filter_ref(p: &PolySynthParams) -> Option<&FilterParams> {
    p.filters.iter().find(|f| !matches!(f.model, FilterModel::Off))
}

pub(crate) static POLY: &[ParamDef<PolySynthParams>] = &[
    num!("attack", 0.0, 20.0, "s", "Volume attack time. 0.001-0.01 crisp; 0.1-0.5 soft; >1 slowly fades in (pad)", |p| p.envelopes[0].attack),
    num!("decay", 0.0, 20.0, "s", "Time for the volume to fall from the peak to sustain", |p| p.envelopes[0].decay),
    num!("sustain", 0.0, 1.0, "", "Volume level held while the key is down. 0 = attack only (pluck); 1 = full volume throughout (pad/organ)", |p| p.envelopes[0].sustain),
    num!("release", 0.0, 20.0, "s", "Tail length after release. 0.05-0.2 tight; 0.5-2 with a tail; >3 a very long bed", |p| p.envelopes[0].release),
    ParamDef {
        spec: InstParamSpec { name: "cutoff", kind: number(20.0, 20000.0, "Hz"), description: "Filter cutoff: lower is duller and darker, higher is brighter. Bass 200-1500; lead 1500-6000" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.cutoff)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.cutoff = v;
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "resonance", kind: number(0.0, 1.0, ""), description: "Filter resonance: the peak at the cutoff. 0.2-0.4 adds character; 0.7+ screaming (acid)" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.resonance)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.resonance = v;
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "filter_env", kind: number(-8.0, 8.0, "oct"), description: "How many octaves the filter envelope pushes the cutoff. Positive = brightens on attack then falls back (a 'wah'); 0 = no movement" },
        get: |p| active_filter_ref(p).map_or(Value::Null, |f| json!(f.env_amount)),
        set: Setter::Num(|p, v| {
            active_filter(p)?.env_amount = v;
            Ok(())
        }),
    },
    num!("filter_attack", 0.0, 20.0, "s", "Filter envelope attack time", |p| p.envelopes[1].attack),
    num!("filter_decay", 0.0, 20.0, "s", "Filter envelope decay time: shorter makes a snappier \"wah\"", |p| p.envelopes[1].decay),
    num!("filter_sustain", 0.0, 1.0, "", "Filter envelope sustain level", |p| p.envelopes[1].sustain),
    num!("filter_release", 0.0, 20.0, "s", "Filter envelope release time", |p| p.envelopes[1].release),
    num!("glide", 0.0, 5.0, "s", "Glide time; only works on mono/legato presets (slides between overlapping notes)", |p| p.glide),
];

fn fm_max(p: &FmParams, f: fn(&manguebyte::dsp::instruments::FmOperator) -> f64) -> Value {
    json!(p.operators.iter().map(f).fold(0.0_f64, f64::max))
}

pub(crate) static FM: &[ParamDef<FmParams>] = &[
    ParamDef {
        spec: InstParamSpec { name: "attack", kind: number(0.0, 10.0, "s"), description: "Attack time of all operators (also changes how the sound evolves)" },
        get: |p| fm_max(p, |o| o.attack),
        set: Setter::Num(|p, v| {
            p.operators.iter_mut().for_each(|o| o.attack = v);
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "release", kind: number(0.001, 100.0, "s"), description: "Release time of all operators" },
        get: |p| fm_max(p, |o| o.release),
        set: Setter::Num(|p, v| {
            p.operators.iter_mut().for_each(|o| o.release = v);
            Ok(())
        }),
    },
];

pub(crate) static STRING: &[ParamDef<StringParams>] = &[
    num!("decay", 0.05, 60.0, "s", "How long the string rings while held (time for middle C to decay 60 dB). Short = muted; long = long ringing tail", |p| p.decay),
    num!("release", 0.01, 10.0, "s", "Decay time after release (damping)", |p| p.release),
    num!("tone", 200.0, 20000.0, "Hz", "Final low-pass: brightness of the body/pickup. 2000 dark; 5000 normal; 10000+ very bright", |p| p.tone),
    num!("pluck_brightness", 100.0, 20000.0, "Hz", "Pluck brightness: fingernail/pick is bright (5000+), fingertip is dark (1000-2000)", |p| p.pluck_brightness),
    num!("pluck_position", 0.02, 0.5, "", "Pluck position (fraction of the way from the bridge). 0.05 near the bridge, thin and bright; 0.5 mid-string, round and hollow", |p| p.pluck_position),
    num!("buzz", 0.0, 1.0, "", "Fret buzz. 0 clean; 0.3+ percussive (slap)", |p| p.buzz),
];

pub(crate) static ELECTRIC: &[ParamDef<crate::electric::ElectricParams>] = &[
    ParamDef {
        spec: InstParamSpec {
            name: "pickup",
            kind: ParamKind::Number { min: 0.0, max: 2.0, unit: "", integer: false, names: &["bridge", "middle", "neck"] },
            description: "Which pickup: \"bridge\" (0) bright and biting, the rock / metal sound; \"middle\" (1); \"neck\" (2) round and warm, for clean, jazz, blues leads. In-between numbers blend",
        },
        get: |p| json!(p.pickup),
        set: Setter::Num(|p, v| {
            p.pickup = v;
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec {
            name: "pickup_type",
            kind: ParamKind::Number { min: 0.0, max: 1.0, unit: "", integer: false, names: &["single", "humbucker"] },
            description: "Pickup coils: \"single\" (0) a high, sharp resonance, glassy and twangy; \"humbucker\" (1) a lower, broader one with more output, thick and smooth under distortion",
        },
        get: |p| json!(p.coil),
        set: Setter::Num(|p, v| {
            p.coil = v;
            Ok(())
        }),
    },
    num!("mute", 0.0, 1.0, "", "Palm mute depth for notes marked `\"mute\": true`: 0.3 a little damped; 0.85 (default) the tight chug of rock / metal riffs; 1 a dead thud", |p| p.mute),
    ParamDef {
        spec: InstParamSpec {
            name: "strum",
            kind: ParamKind::Number { min: 0.0, max: 50.0, unit: "ms", integer: true, names: &[] },
            description: "Strum: when 3 or more notes start together, each string sounds this long after the previous one. Downstrokes (low string first) on the beat and the eighth notes, upstrokes (high string first) on the off-beats. 0 = all at once; 5-10 a pick strum; 20+ a slow rake",
        },
        get: |p| json!(p.strum_ms),
        set: Setter::Num(|p, v| {
            p.strum_ms = v;
            Ok(())
        }),
    },
    num!("decay", 0.05, 60.0, "s", "How long an open string rings while held (middle C to -60 dB). Long sustain is what a distorted guitar needs", |p| p.string.decay),
    num!("release", 0.01, 10.0, "s", "How fast a note stops after it is released", |p| p.string.release),
    num!("pluck_brightness", 100.0, 20000.0, "Hz", "Pick attack brightness: 7000 a hard pick; 2000 the thumb or a soft pick", |p| p.string.pluck_brightness),
    num!("pluck_position", 0.02, 0.5, "", "Where the pick hits (fraction of the way from the bridge): 0.06 near the bridge, thin and twangy; 0.3 over the neck, round", |p| p.string.pluck_position),
];

pub(crate) const NEXT_ZITHER: &[&str] = &[
    "decay", "release", "high_decay", "strike", "pluck_noise", "pluck_brightness", "pluck_position",
    "vibrato", "vibrato_rate", "vibrato_delay", "pressure",
];

pub(crate) static ZITHER: &[ParamDef<crate::zither::ZitherParams>] = &[
    ParamDef {
        spec: InstParamSpec {
            name: "technique",
            kind: ParamKind::Number { min: 0.0, max: 3.0, unit: "", integer: true, names: crate::zither::TECHNIQUES },
            description: "Playing technique: auto chooses open strings at C2 D2 F2 G2 A2 C3 D3, pressed elsewhere; open rings freely; pressed allows slides; harmonic touches a node. Changes the next pluck",
        },
        get: |p| json!(p.technique),
        set: Setter::Num(|p, v| { p.technique = v; Ok(()) }),
    },
    ParamDef {
        spec: InstParamSpec {
            name: "harmonic",
            kind: ParamKind::Number { min: 2.0, max: 8.0, unit: "", integer: true, names: &[] },
            description: "Harmonic node denominator (harmonic technique only): 2 at half the string, 3 at one third, 4 at one quarter. Note pitch is the SOUNDING pitch, not the open-string pitch",
        },
        get: |p| json!(p.harmonic),
        set: Setter::Num(|p, v| { p.harmonic = v; Ok(()) }),
    },
    num!("pressure", 0.0, 1.0, "", "Stopped-string finger pressure: more contact shortens and darkens pressed notes; moving the hand adds loss. Initial pressure changes the next pluck", |p| p.pressure),
    num!("slide_time", 0.001, 0.1, "s", "Smoothing of hand movement: small values follow bend curves closely, larger values soften changes. Phase remains continuous", |p| p.slide_time),
    num!("slide_noise", 0.0, 1.0, "", "Stopped-string hand-motion noise, proportional to slide speed and remaining string vibration; 0 clean, 0.35 subtle", |p| p.slide_noise),
    num!("sympathetic", 0.0, 1.0, "", "Unplayed strings resonating with the played string: 0 isolated, 0.18 subtle shared resonance", |p| p.sympathetic),
    num!("vibrato", 0.0, 100.0, "cents", "Automatic yin/nao on pressed notes only; explicit bend curves override it; 0 no automatic movement", |p| p.pluck.vibrato),
    num!("vibrato_rate", 0.5, 12.0, "Hz", "How fast pressed-string vibrato moves", |p| p.pluck.vibrato_rate),
    num!("vibrato_delay", 0.0, 4.0, "s", "Delay after the pluck before pressed-string vibrato begins", |p| p.pluck.vibrato_delay),
    num!("tremolo_rate", 2.0, 30.0, "Hz", "Repeated plucks per second on notes marked tremolo", |p| p.pluck.tremolo_rate),
    num!("decay", 0.05, 60.0, "s", "Open-string fundamental decay at middle C to -60 dB; pressed-string contact adds damping", |p| p.pluck.string.decay),
    num!("release", 0.01, 30.0, "s", "Decay after note end: open and harmonic strings keep ringing; pressed strings retain more contact damping", |p| p.pluck.string.release),
    num!("high_decay", 0.01, 1.0, "", "Upper-mode decay at 3 kHz relative to the fundamental; harmonics keep a minimum of 0.2 for a clear ring", |p| p.pluck.string.high_decay),
    num!("pluck_brightness", 100.0, 20000.0, "Hz", "Brightness of the initial excitation; softer velocity also darkens the pluck", |p| p.pluck.string.pluck_brightness),
    num!("pluck_position", 0.02, 0.5, "", "Right-hand plucking position from the bridge; pressed strings account for their shorter vibrating length", |p| p.pluck.string.pluck_position),
    num!("strike", 0.0, 1.0, "", "Excitation shape: 0 a soft pulled string, 0.35 a light nail touch, 1 a struck string", |p| p.pluck.string.strike),
    num!("pluck_noise", 0.0, 1.0, "", "Brief nail contact noise at the pluck; distinct from sustained hand-motion noise", |p| p.pluck.string.pluck_noise),
    num!("body_mix", 0.0, 1.0, "", "Body radiation and shared wooden resonances: 0 raw string; 0.45 full radiation colour, with resonances ringing across notes", |p| p.pluck.string.body_mix),
    num!("tone", 200.0, 20000.0, "Hz", "Final overall low-pass colour, independent of technique-specific mode damping", |p| p.pluck.string.tone),
];

pub(crate) static DIZI: &[ParamDef<crate::chinese::DiziParams>] = &[
    num!("membrane", 0.0, 1.0, "", "The dizi's membrane (笛膜): 0 none, a plain flute; 0.6 the bright, buzzing dizi sound; 1 a loud buzz. Louder notes buzz more", |p| p.membrane),
    num!("attack", 0.002, 1.0, "s", "Breath build-up: short = clear tonguing; long = soft entry", |p| p.wind.attack),
    num!("release", 0.005, 2.0, "s", "Wind-down time after release", |p| p.wind.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "Breath vibrato rate (5-6)", |p| p.wind.vibrato_rate),
    num!("vibrato_depth", 0.0, 100.0, "cent", "Vibrato depth: 0 straight; 10-20 normal; 30+ the wide folk vibrato", |p| p.wind.vibrato_depth),
    num!("vibrato_delay", 0.0, 4.0, "s", "Delay after a note starts before the vibrato", |p| p.wind.vibrato_delay),
    num!("breath_noise", 0.0, 1.0, "", "Breath noise: more airflow", |p| p.wind.noise),
    num!("breath", 0.3, 1.5, "", "Breath strength (气息): scales how hard the player blows; 1 as preset, 0.6 a soft, airy, darker breath, 1.3 a full, bright one. Automate it to shape a phrase", |p| p.breath),
    num!("embouchure", 0.05, 1.0, "", "Embouchure (口型): how far the lips let the air through; smaller is darker and softer, larger is fuller and brighter", |p| p.wind.opening),
    num!("articulation", 0.0, 1.0, "", "Tonguing (舌法): how hard the tongue starts a note, 0 soft (legato-like), 1 a hard, clear attack with more chiff (staccato)", |p| p.wind.articulation),
    num!("flutter_depth", 0.0, 1.0, "", "Flutter-tongue depth (花舌的颗粒): how far the sound drops each time the rolling tongue cuts the air on a `flutter` note; 0.44 as in recordings of long notes, 0.8 a coarse, grainy roll, 1 silent between each cut. Automate it to roughen one note", |p| p.flutter_depth),
    num!("rasp", 0.0, 1.0, "", "Rasp (沙哑): noise that rides on every cycle of the wave, the irregular beating of a reed; 0 clean, 0.35 the suona's grain, 0.8 a hoarse, cracking tone (automate it for a 破音 effect)", |p| p.rasp),
    num!("width", 0.0, 1.0, "", "Width: a few very close reflections, different left and right, as if played in a room; the sound gets breadth without a reverb's tail. 0 a dry, centred source", |p| p.width),
    num!("drone", 0.0, 1.0, "", "Drone pipe (附管, the hulusi's side pipes): how loud the held note under the melody is, sounding while the player blows; 0 off (stopped), 0.3 a soft drone", |p| p.drone),
    ParamDef {
        spec: InstParamSpec { name: "drone_note", kind: integer(24.0, 108.0), description: "The drone pipe's note (MIDI): usually the key's tonic or fifth (72 = C5 on a C hulusi)" },
        get: |p| json!(p.drone_note),
        set: Setter::Num(|p, v| { p.drone_note = v; Ok(()) }),
    },
    num!("breath_vibrato", 0.0, 1.0, "", "Breath vibrato (气震音): how much the vibrato also swells the breath, louder and softer at `vibrato_rate`; 0.25 a little, 0.8 the pulsing breath vibrato (with a small `vibrato_depth` it is almost only in the breath)", |p| p.wind.tremolo),
    num!("tone", 500.0, 20000.0, "Hz", "Final low-pass: brightness", |p| p.wind.tone),
];

pub(crate) const NEXT_DIZI: &[&str] = &["attack", "release", "vibrato_rate", "vibrato_depth", "vibrato_delay", "breath_vibrato", "articulation"];

pub(crate) const NEXT_SINGER: &[&str] = &["attack", "vibrato_delay"];

pub(crate) static SINGER: &[ParamDef<crate::singer::SingerParams>] = &[
    ParamDef {
        spec: InstParamSpec {
            name: "part",
            kind: ParamKind::Number { min: 0.0, max: 4.0, unit: "", integer: true, names: crate::singer::PART_NAMES },
            description: "Voice part (vocal tract size and colour): \"soprano\", \"alto\", \"tenor\", \"bass\", \"child\"",
        },
        get: |p| json!(p.part),
        set: Setter::Num(|p, v| {
            p.part = v;
            Ok(())
        }),
    },
    num!("formant_shift", 0.7, 1.4, "", "Vocal tract size: 1 natural; 0.9 a bigger, darker voice; 1.1 smaller and brighter (gender and age)", |p| p.formant_shift),
    num!("breath", 0.0, 6.0, "", "Breathiness (times the part's own): 0 pure; 1 natural; 3+ an airy, whispery voice", |p| p.breath),
    num!("brightness", 0.0, 1.0, "", "Vocal brightness (glottal spectrum): 0.3 soft and round; 0.55 natural; 0.9 bright, pressed, belting", |p| p.brightness),
    num!("vibrato_depth", 0.0, 120.0, "cents", "Vibrato depth either way: 0 straight (pop, early music); 30-40 natural; 60+ operatic", |p| p.vibrato_depth),
    num!("vibrato_rate", 0.5, 9.0, "Hz", "Vibrato rate (5-6)", |p| p.vibrato_rate),
    num!("vibrato_delay", 0.0, 3.0, "s", "How long a held note stays straight before the vibrato comes in", |p| p.vibrato_delay),
    num!("glide", 0.005, 0.5, "s", "Legato glide: how fast the voice slides to the next (overlapping) note", |p| p.glide),
    num!("jitter", 0.0, 1.0, "", "Cycle-to-cycle unsteadiness of pitch and level: 0 machine-perfect; 0.4 natural; 1 rough", |p| p.jitter),
    num!("attack", 0.005, 1.0, "s", "How fast a note starts after a break", |p| p.attack),
    num!("release", 0.01, 2.0, "s", "How fast a note ends", |p| p.release),
];

pub(crate) const NEXT_ACCORDION: &[&str] = &["musette", "attack"];

pub(crate) static ACCORDION: &[ParamDef<crate::accordion::AccordionParams>] = &[
    ParamDef {
        spec: InstParamSpec {
            name: "register",
            kind: ParamKind::Number { min: 0.0, max: 7.0, unit: "", integer: true, names: crate::accordion::REGISTER_NAMES },
            description: "Register (which reeds sound): \"master\" all of them, the fullest; \"musette\" three middle reeds, the shimmering French sound; \"violin\" two, a light shimmer; \"clarinet\" one, plain; \"bassoon\" the low reed alone; \"oboe\" middle + high, bright; \"bandoneon\" low + middle, the tango sound; \"organ\" low + high, hollow. From the next note",
        },
        get: |p| json!(p.register),
        set: Setter::Num(|p, v| {
            p.register = v;
            Ok(())
        }),
    },
    num!("musette", 0.0, 60.0, "cents", "How far the middle reeds are tuned apart: 0 dry; 10-15 a light tremolo; 25-35 the wet French musette (beats about 6 times a second at A4)", |p| p.musette),
    num!("bellows", 0.0, 1.0, "", "Bellows pressure: more is louder and brighter. Automate it for the push and pull of a phrase (swells, accents)", |p| p.bellows),
    num!("shake", 0.0, 1.0, "", "Bellows shake: a tremolo of the volume, as players shake the bellows in tango and folk music. 0 off; 0.5 clear", |p| p.shake),
    num!("shake_rate", 0.5, 12.0, "Hz", "How fast the bellows shake (about 4-7)", |p| p.shake_rate),
    num!("cassotto", 0.0, 1.0, "", "Tone chamber: 0 open, bright and direct; 1 the reeds sit in the chamber, darker and rounder", |p| p.cassotto),
    num!("attack", 0.002, 0.5, "s", "How long the reeds take to speak: 0.02 crisp; 0.05+ a soft swell", |p| p.attack),
    num!("release", 0.005, 5.0, "s", "How fast a note stops after the key is released", |p| p.release),
];

pub(crate) static ACID: &[ParamDef<AcidParams>] = &[
    num!("cutoff", 30.0, 8000.0, "Hz", "Filter cutoff (with the envelope off). 200-400 dull; 800+ bright. Ideal for automated sweeps", |p| p.cutoff),
    num!("resonance", 0.0, 1.0, "", "Resonance. 0.5 normal; 0.8+ the classic 303 squelch", |p| p.resonance),
    num!("env_mod", 0.0, 1.0, "", "Filter envelope depth: how far the filter opens at each note's attack", |p| p.env_mod),
    num!("decay", 0.05, 4.0, "s", "Filter envelope decay: short = a short \"blip\"; long = a drawn-out \"wah\"", |p| p.decay),
    num!("accent", 0.0, 1.0, "", "Accent strength: notes marked `\"accent\": true` are louder, open the filter further and squelch more", |p| p.accent),
    num!("slide", 0.005, 1.0, "s", "Slide time: glide from one note to the next when they overlap", |p| p.slide),
    num!("drive_db", -12.0, 24.0, "dB", "Overdrive: gain before the filter. 6-12 is fatter and dirtier", |p| p.drive_db),
    num!("tune", -24.0, 24.0, "st", "Overall transpose (semitones)", |p| p.tune),
];

fn drawbars_text(p: &OrganParams) -> Value {
    json!(p.drawbars.iter().map(|d| char::from(b'0' + d.round().clamp(0.0, 8.0) as u8)).collect::<String>())
}

pub(crate) static ORGAN: &[ParamDef<OrganParams>] = &[
    ParamDef {
        spec: InstParamSpec { name: "drawbars", kind: ParamKind::Drawbars, description: "Nine drawbars (16' 5⅓' 8' 4' 2⅔' 2' 1⅗' 1⅓' 1'), each 0-8. \"888000000\" classic jazz; \"838000000\" soft; \"888888888\" all out" },
        get: drawbars_text,
        set: Setter::Text(|p, s| {
            let digits: Vec<u32> = s.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_digit(10).unwrap_or(99)).collect();
            if digits.len() != 9 || digits.iter().any(|d| *d > 8) {
                return Err(format!("expected a 9-digit string with each digit 0-8, e.g. \"888000000\", got \"{s}\""));
            }
            for (bar, d) in p.drawbars.iter_mut().zip(digits) {
                *bar = f64::from(d);
            }
            Ok(())
        }),
    },
    ParamDef {
        spec: InstParamSpec { name: "percussion", kind: ParamKind::Choice { choices: &["off", "second", "third"] }, description: "Percussion: adds a short 2nd/3rd harmonic at key-down, the jazz organ \"ping\"" },
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
    num!("click", 0.0, 1.0, "", "Key click. 0 clean; 0.5 vintage", |p| p.click),
];

pub(crate) static MALLET: &[ParamDef<MalletParams>] = &[
    num!("decay", 0.05, 60.0, "s", "Natural decay time after the strike (middle C)", |p| p.decay),
    num!("release", 0.01, 60.0, "s", "Damping time after release (vibraphone/celesta/tubular bells have dampers)", |p| p.release),
    num!("softness", 1.0, 8.0, "", "Mallet hardness: 1 hard and bright; 4-8 soft and round", |p| p.softness),
    num!("strike_position", 0.0, 1.0, "", "Strike position. 0.5 is the center (fullest); toward the edge has more overtones", |p| p.strike_position),
    num!("motor_rate", 0.0, 16.0, "Hz", "Vibraphone motor speed (tremolo rate), 0 = off", |p| p.motor_rate),
    num!("motor_depth", 0.0, 1.0, "", "Vibraphone motor tremolo depth", |p| p.motor_depth),
];

pub(crate) static BOWED: &[ParamDef<BowedParams>] = &[
    num!("attack", 0.005, 2.0, "s", "Bow attack time. 0.02 forceful; 0.3+ soft fade-in", |p| p.attack),
    num!("release", 0.15, 10.0, "s", "Decay time after release. Minimum 0.15: below that the upstream bowed-string model diverges numerically", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "Vibrato rate. 5-6 normal", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 100.0, "cent", "Vibrato depth (cents up and down). 0 none; 10-20 normal; 40+ exaggerated", |p| p.vibrato_depth),
    num!("vibrato_delay", 0.0, 4.0, "s", "Delay after note start before vibrato begins", |p| p.vibrato_delay),
    num!("tone", 500.0, 20000.0, "Hz", "Final low-pass: brightness", |p| p.tone),
    num!("pressure", 0.0, 1.0, "", "Bow pressure: low = gentle and breathy; high = dense and grainy", |p| p.pressure),
    int!("players", 1.0, 4.0, "Number of players per note: 1 solo; 3-4 ensemble", |p| p.players),
];

pub(crate) static WIND: &[ParamDef<WindParams>] = &[
    num!("attack", 0.002, 1.0, "s", "Breath build-up time: short = clear tonguing; long = soft entry", |p| p.attack),
    num!("release", 0.005, 2.0, "s", "Wind-down time after release", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "Vibrato rate. 4-6 normal", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 100.0, "cent", "Vibrato depth (cents). 0 straight tone; 10-25 normal", |p| p.vibrato_depth),
    num!("vibrato_delay", 0.0, 4.0, "s", "Delay after note start before vibrato begins", |p| p.vibrato_delay),
    num!("tone", 500.0, 20000.0, "Hz", "Final low-pass: brightness", |p| p.tone),
    num!("breath_noise", 0.0, 1.0, "", "Breath noise: higher means more airflow noise", |p| p.noise),
    num!("brassiness", 0.0, 1.0, "", "Brass blare (trumpet/trombone brighten when loud)", |p| p.brassiness),
    ParamDef {
        spec: InstParamSpec { name: "portamento", kind: ParamKind::Bool, description: "Glide between legato (overlapping) notes, like a trombone slide" },
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
            description: "Vowel: \"a\" \"e\" \"i\" \"o\" \"u\", or a number 0-4 (e.g. 1.5 = between e and i). Good for automation",
        },
        get: |p| json!(p.vowel),
        set: Setter::Num(|p, v| {
            p.vowel = v;
            Ok(())
        }),
    },
    num!("attack", 0.005, 2.0, "s", "Attack time", |p| p.attack),
    num!("release", 0.01, 4.0, "s", "Fade-out time after release", |p| p.release),
    num!("vibrato_rate", 0.0, 12.0, "Hz", "Vibrato rate", |p| p.vibrato_rate),
    num!("vibrato_depth", 0.0, 200.0, "cent", "Vibrato depth (cents)", |p| p.vibrato_depth),
    num!("brightness", 500.0, 12000.0, "Hz", "Tone brightness", |p| p.brightness),
    num!("breathiness", 0.0, 1.0, "", "Breathiness", |p| p.breathiness),
    int!("singers", 1.0, 6.0, "Number of singers per note: 1 solo; 4-6 choir", |p| p.singers),
    num!("formant_shift", 0.7, 1.4, "", "Formant scaling: <1 deeper, bigger voice; >1 thinner, smaller", |p| p.formant_shift),
];

pub(crate) static PIECE: &[ParamDef<Piece>] = &[
    num!("tune", -24.0, 24.0, "st", "Drum piece tuning (semitones). Kick -3 is deeper; snare +2 is tighter", |p| p.tune),
    num!("decay", 0.1, 10.0, "x", "Tail length (multiple of the original). 0.5 short; 2 longer (808 long-tail kick)", |p| p.decay),
    num!("level", 0.0, 4.0, "x", "Volume (multiplier), 1 = unchanged", |p| p.level),
    num!("tone", 0.0, 1.0, "", "Tone (varies by drum piece, usually brightness)", |p| p.tone),
    num!("snap", 0.0, 1.0, "", "Attack snap (varies by drum piece; snare wires for a snare)", |p| p.snap),
    num!("texture", 0.0, 1.0, "", "Texture (varies by drum piece)", |p| p.texture),
    num!("pan", -1.0, 1.0, "", "Pan: -1 left to 1 right", |p| p.pan),
];

/// 鼓件别名 → GM 鼓键。
pub(crate) use muser_core::grid::{drum_key as piece_key, DRUM_PIECES as PIECE_ALIASES};

/// yinse 打击乐（[`crate::percussion`]）每件的参数。
pub(crate) static KIT_PIECE: &[ParamDef<crate::percussion::PieceSet>] = &[
    num!("tune", -24.0, 24.0, "st", "Tuning in semitones: a gong tuned down is bigger and darker", |p| p.tune),
    num!("decay", 0.1, 10.0, "x", "How long it rings, as a multiple of its own decay: 0.5 shorter, 2 longer", |p| p.decay),
    num!("level", 0.0, 4.0, "x", "Volume (multiplier), 1 = unchanged", |p| p.level),
    num!("tone", 0.0, 1.0, "", "Brightness of the strike: 0 soft and dark, 0.5 as designed, 1 bright and hard", |p| p.tone),
    num!("pan", -1.0, 1.0, "", "Pan: -1 left to 1 right", |p| p.pan),
];

pub(crate) const NEXT_KIT_PIECE: &[&str] = &["tune", "decay", "level", "tone", "pan"];

// ---------------------------------------------------------------------------
// 通用逻辑
// ---------------------------------------------------------------------------

pub(crate) fn to_number(spec: &InstParamSpec, v: &Value) -> Result<f64, String> {
    match spec.kind {
        ParamKind::Number { min, max, unit, integer, names } => {
            let x = if let Some(s) = v.as_str() {
                names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(s.trim()))
                    .map(|i| i as f64)
                    .ok_or_else(|| {
                        if names.is_empty() {
                            format!("expected a number, got \"{s}\"")
                        } else {
                            format!("expected a number or {}, got \"{s}\"", names.join("/"))
                        }
                    })?
            } else {
                v.as_f64().ok_or_else(|| format!("expected a number, got {v}"))?
            };
            if !x.is_finite() || x < min || x > max {
                let unit = if unit.is_empty() { String::new() } else { format!(" {unit}") };
                return Err(format!("{x} out of range [{min}, {max}]{unit}"));
            }
            if integer && x.fract() != 0.0 {
                return Err(format!("expected an integer, got {x}"));
            }
            Ok(x)
        }
        ParamKind::Choice { choices } => {
            let s = v.as_str().ok_or_else(|| format!("expected a string {}, got {v}", choices.join("/")))?;
            choices
                .iter()
                .position(|c| c.eq_ignore_ascii_case(s.trim()))
                .map(|i| i as f64)
                .ok_or_else(|| format!("\"{s}\" is invalid, choose from {}", choices.join(" / ")))
        }
        ParamKind::Bool => v
            .as_bool()
            .map(|b| if b { 1.0 } else { 0.0 })
            .ok_or_else(|| format!("expected true/false, got {v}")),
        ParamKind::Drawbars => Err("internal error: drawbar parameters must be written as text".into()),
    }
}

fn apply_one<P>(def: &ParamDef<P>, p: &mut P, v: &Value) -> Result<(), String> {
    match def.set {
        Setter::Num(f) => f(p, to_number(&def.spec, v)?),
        Setter::Text(f) => f(p, v.as_str().ok_or_else(|| format!("expected a string, got {v}"))?),
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
                "unknown parameter '{key}', available: {}",
                defs.iter().map(|d| d.spec.name).collect::<Vec<_>>().join(", ")
            )
        })?;
        apply_one(def, p, value).map_err(|e| format!("{key}: {e}"))?;
    }
    Ok(())
}

/// 参数规格 + 当前值。
pub(crate) fn describe<P>(defs: &[ParamDef<P>], p: &P, next_note: &[&str]) -> Vec<ParamInfo> {
    defs.iter()
        .map(|d| ParamInfo { spec: d.spec, current: (d.get)(p), automation: automatable(&d.spec, next_note) })
        .collect()
}

/// 只要规格（不带当前值）。
pub(crate) fn specs<P>(defs: &[ParamDef<P>], next_note: &[&str]) -> Vec<ParamInfo> {
    defs.iter()
        .map(|d| ParamInfo { spec: d.spec, current: Value::Null, automation: automatable(&d.spec, next_note) })
        .collect()
}

/// 自动化：按名字写一个数值（已校验过范围；这里再夹一次，防御性）。
pub(crate) fn set_number<P>(defs: &[ParamDef<P>], p: &mut P, name: &str, v: f64) -> bool {
    let Some(def) = defs.iter().find(|d| d.spec.name == name) else { return false };
    let v = match def.spec.kind {
        ParamKind::Number { min, max, .. } => v.clamp(min, max),
        _ => return false,
    };
    match def.set {
        Setter::Num(f) => f(p, v).is_ok(),
        Setter::Text(_) => false,
    }
}

