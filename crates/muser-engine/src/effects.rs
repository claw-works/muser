//! 效果器适配：把 [`muser_core::Effect`] 解析为 manguebyte 的效果处理器，
//! 并在离线渲染时分块施加到音频缓冲上。
//!
//! 所有效果器都实现 manguebyte 的 `Processor` trait（in-place `process`），
//! 因此一条效果链就是一串 `Box<dyn Processor>`，依次对缓冲处理即可。
//!
//! 第三方插件（CLAP/VST3/LV2）作为效果器时，由 `manguebyte-host` 加载并
//! `activate` 成一个 `Box<dyn Processor>`，天然融入同一条链。此能力在
//! `host` 特性下启用。

use muser_core::Effect;

use manguebyte::dsp::{
    AutoPan, Bitcrusher, FormantFilter, FrequencyShifter, GranularDelay, LfoShape, MultibandCompressor, RingModulator,
    StereoWidener, TapeSaturation, TransientShaper, Tremolo, Vocoder,
};
use manguebyte::prelude::{
    AudioBuffer, Chorus, Compressor, Delay, Filter, FilterShape, Limiter, Phaser, ProcessSpec,
    Processor, Reverb,
};

/// 效果链构建错误。
#[derive(Debug, thiserror::Error)]
pub enum EffectError {
    #[error("plugin effect unavailable: this muser build does not have the plugin host enabled (the `host` feature); release and default builds include it, so do not remove it with --no-default-features")]
    HostDisabled,
    #[error("failed to load plugin '{path}': {detail}")]
    PluginLoad { path: String, detail: String },
    #[error("unknown EQ filter shape '{0}' (available: lowpass/highpass/bandpass/notch/allpass/peak/low_shelf/high_shelf）")]
    UnknownEqShape(String),
}

/// 一个已就绪、可在运行中改参数的效果器。
pub enum Fx {
    Reverb(Reverb<f32>),
    Delay(Delay<f32>),
    Compressor(Compressor<f32>),
    /// 合唱与镶边共用（镶边是 Chorus::flanger 预设）。
    Chorus(Chorus<f32>),
    Eq { filter: Filter<f32>, shape: String, gain_db: f64 },
    Limiter(Limiter<f32>),
    Phaser(Phaser<f32>),
    Bitcrusher(Bitcrusher<f32>),
    Tremolo(Tremolo<f32>),
    AutoPan(AutoPan<f32>),
    RingMod(RingModulator<f32>),
    FrequencyShifter(FrequencyShifter<f32>),
    Widener(StereoWidener<f32>),
    TransientShaper(TransientShaper<f32>),
    MultibandCompressor(MultibandCompressor<f32>),
    Tape(TapeSaturation<f32>),
    Vocoder(MakeupVocoder),
    GranularDelay(GranularDelay<f32>),
    FormantFilter(FormantFilter<f32>),
    Other(Box<dyn Processor<f32>>),
    #[cfg(feature = "host")]
    Plugin(HostedPlugin),
    /// 旁通（`"bypass": true`）：声音原样通过。
    Bypass,
}

impl Fx {
    fn processor(&mut self) -> Option<&mut dyn Processor<f32>> {
        Some(match self {
            Fx::Reverb(p) => p,
            Fx::Delay(p) => p,
            Fx::Compressor(p) => p,
            Fx::Chorus(p) => p,
            Fx::Eq { filter, .. } => filter,
            Fx::Limiter(p) => p,
            Fx::Phaser(p) => p,
            Fx::Bitcrusher(p) => p,
            Fx::Tremolo(p) => p,
            Fx::AutoPan(p) => p,
            Fx::RingMod(p) => p,
            Fx::FrequencyShifter(p) => p,
            Fx::Widener(p) => p,
            Fx::TransientShaper(p) => p,
            Fx::MultibandCompressor(p) => p,
            Fx::Tape(p) => p,
            Fx::Vocoder(p) => p,
            Fx::GranularDelay(p) => p,
            Fx::FormantFilter(p) => p,
            Fx::Other(p) => p.as_mut(),
            #[cfg(feature = "host")]
            Fx::Plugin(p) => p,
            Fx::Bypass => return None,
        })
    }

    pub fn prepare(&mut self, spec: &ProcessSpec) {
        if let Some(p) = self.processor() {
            p.prepare(spec);
        }
    }

    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        if let Some(p) = self.processor() {
            p.process(buffer);
        }
    }

    /// 自动化：运行中改一个参数（名称与效果器 JSON 字段相同；已按描述表校验过范围）。
    pub fn set_param(&mut self, name: &str, v: f64) {
        match (self, name) {
            (Fx::Reverb(r), "mix") => r.set_mix(v),
            (Fx::Reverb(r), "decay_seconds") => r.set_decay_seconds(v),
            (Fx::Reverb(r), "damping") => r.set_damping(v),
            (Fx::Reverb(r), "size") => r.set_size(v),
            (Fx::Delay(d), "time_seconds") => d.set_time(v),
            (Fx::Delay(d), "feedback") => d.set_feedback(v),
            (Fx::Delay(d), "mix") => d.set_mix(v),
            (Fx::Compressor(c), "threshold_db") => c.set_threshold_db(v),
            (Fx::Compressor(c), "ratio") => c.set_ratio(v),
            (Fx::Compressor(c), "attack_ms") => c.set_attack_ms(v),
            (Fx::Compressor(c), "release_ms") => c.set_release_ms(v),
            (Fx::Compressor(c), "makeup_db") => c.set_makeup_db(v),
            (Fx::Chorus(c), "rate_hz") => c.set_rate_hz(v),
            (Fx::Chorus(c), "depth_ms") => c.set_depth_ms(v),
            (Fx::Chorus(c), "feedback") => c.set_feedback(v),
            (Fx::Chorus(c), "mix") => c.set_mix(v),
            (Fx::Eq { filter, .. }, "frequency") => filter.set_frequency(v),
            (Fx::Eq { filter, .. }, "q") => filter.set_q(v),
            (Fx::Eq { filter, shape, gain_db }, "gain_db") => {
                *gain_db = v;
                if let Ok(s) = parse_filter_shape(shape, v) {
                    filter.set_shape(s);
                }
            }
            (Fx::Limiter(l), "ceiling_db") => l.set_ceiling_db(v),
            (Fx::Limiter(l), "release_ms") => l.set_release_ms(v),
            (Fx::Phaser(p), "rate_hz") => p.set_rate_hz(v),
            (Fx::Phaser(p), "feedback") => p.set_feedback(v),
            (Fx::Phaser(p), "mix") => p.set_mix(v),
            (Fx::Bitcrusher(b), "bits") => b.set_bits(v),
            (Fx::Bitcrusher(b), "downsample") => b.set_downsample(v),
            (Fx::Bitcrusher(b), "mix") => b.set_mix(v),
            (Fx::Tremolo(t), "rate_hz") => t.set_rate_hz(v),
            (Fx::Tremolo(t), "depth") => t.set_depth(v),
            (Fx::Tremolo(t), "stereo_phase") => t.set_stereo_phase(v),
            (Fx::AutoPan(a), "rate_hz") => a.set_rate_hz(v),
            (Fx::AutoPan(a), "width") => a.set_width(v),
            (Fx::RingMod(r), "frequency_hz") => r.set_frequency_hz(v),
            (Fx::RingMod(r), "mix") => r.set_mix(v),
            (Fx::FrequencyShifter(f), "shift_hz") => f.set_shift_hz(v),
            (Fx::FrequencyShifter(f), "feedback") => f.set_feedback(v),
            (Fx::FrequencyShifter(f), "mix") => f.set_mix(v),
            (Fx::Widener(w), "width") => w.set_width(v),
            (Fx::Widener(w), "bass_mono_hz") => {
                w.set_bass_mono(v > 0.0);
                w.set_bass_mono_hz(v.max(20.0));
            }
            (Fx::TransientShaper(t), "attack_db") => t.set_attack_db(v),
            (Fx::TransientShaper(t), "sustain_db") => t.set_sustain_db(v),
            (Fx::TransientShaper(t), "attack_ms") => t.set_attack_ms(v),
            (Fx::TransientShaper(t), "sustain_ms") => t.set_sustain_ms(v),
            (Fx::MultibandCompressor(m), name) => multiband_set(m, name, v),
            (Fx::Tape(t), "drive_db") => t.set_drive_db(v),
            (Fx::Tape(t), "asymmetry") => t.set_asymmetry(v),
            (Fx::Tape(t), "wow_cents") => t.set_wow_cents(v),
            (Fx::Tape(t), "flutter_cents") => t.set_flutter_cents(v),
            (Fx::Tape(t), "mix") => t.set_mix(v),
            (Fx::Vocoder(voc), name) => voc.set(name, v),
            (Fx::GranularDelay(g), "position_ms") => g.set_position_ms(v),
            (Fx::GranularDelay(g), "spray_ms") => g.set_spray_ms(v),
            (Fx::GranularDelay(g), "grain_ms") => g.set_grain_ms(v),
            (Fx::GranularDelay(g), "density_hz") => g.set_density_hz(v),
            (Fx::GranularDelay(g), "pitch_semitones") => g.set_pitch_semitones(v),
            (Fx::GranularDelay(g), "pitch_spray") => g.set_pitch_spray(v),
            (Fx::GranularDelay(g), "stereo_spread") => g.set_stereo_spread(v),
            (Fx::GranularDelay(g), "feedback") => g.set_feedback(v),
            (Fx::GranularDelay(g), "mix") => g.set_mix(v),
            (Fx::FormantFilter(f), "vowel") => f.set_vowel(v),
            (Fx::FormantFilter(f), "shift") => f.set_shift(v),
            (Fx::FormantFilter(f), "gain_db") => f.set_gain_db(v),
            (Fx::FormantFilter(f), "mix") => f.set_mix(v),
            #[cfg(feature = "host")]
            (Fx::Plugin(p), id) => p.set_param(id, v),
            _ => {}
        }
    }
}

/// 把一组 [`Effect`] 构造成已就绪（prepare 过）的效果链。
pub fn build_chain(effects: &[muser_core::EffectSlot], spec: &ProcessSpec) -> Result<Vec<Fx>, EffectError> {
    let mut chain = Vec::with_capacity(effects.len());
    for e in effects {
        // 旁通的效果器仍占一格，自动化的 `effects[i]` 序号不变。
        if e.bypass {
            chain.push(Fx::Bypass);
            continue;
        }
        let mut fx = build_one(e, spec)?;
        fx.prepare(spec);
        chain.push(fx);
    }
    Ok(chain)
}

fn build_one(effect: &Effect, spec: &ProcessSpec) -> Result<Fx, EffectError> {
    let fx = match effect {
        Effect::Reverb {
            mix,
            decay_seconds,
            damping,
            size,
        } => {
            let mut r = Reverb::<f32>::new()
                .with_mix(*mix)
                .with_decay_seconds(*decay_seconds)
                .with_damping(*damping);
            r.set_size(*size);
            Fx::Reverb(r)
        }
        Effect::Delay {
            time_seconds,
            feedback,
            mix,
        } => {
            let max = time_seconds.max(2.0);
            let d = Delay::<f32>::new(max)
                .with_time(*time_seconds)
                .with_feedback(*feedback)
                .with_mix(*mix);
            Fx::Delay(d)
        }
        Effect::Compressor {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
            sidechain,
        } => {
            let mut c = Compressor::<f32>::new()
                .with_threshold_db(*threshold_db)
                .with_ratio(*ratio)
                .with_attack_ms(*attack_ms)
                .with_release_ms(*release_ms)
                .with_makeup_db(*makeup_db);
            if sidechain.is_some() {
                c.set_key_source(manguebyte::prelude::KeySource::Sidechain);
            }
            Fx::Compressor(c)
        }
        Effect::Chorus {
            rate_hz,
            depth_ms,
            mix,
        } => {
            let c = Chorus::<f32>::new()
                .with_rate_hz(*rate_hz)
                .with_depth_ms(*depth_ms)
                .with_mix(*mix);
            Fx::Chorus(c)
        }
        Effect::Eq {
            shape,
            frequency,
            q,
            gain_db,
        } => {
            let fshape = parse_filter_shape(shape, *gain_db)?;
            Fx::Eq { filter: Filter::<f32>::new(fshape, *frequency, *q), shape: shape.clone(), gain_db: *gain_db }
        }
        Effect::Limiter {
            ceiling_db,
            release_ms,
        } => {
            let l = Limiter::<f32>::new()
                .with_ceiling_db(*ceiling_db)
                .with_release_ms(*release_ms);
            Fx::Limiter(l)
        }
        Effect::Phaser {
            rate_hz,
            feedback,
            mix,
        } => {
            let p = Phaser::<f32>::new()
                .with_rate_hz(*rate_hz)
                .with_feedback(*feedback)
                .with_mix(*mix);
            Fx::Phaser(p)
        }
        Effect::Flanger {
            rate_hz,
            feedback,
            mix,
        } => {
            // Chorus::flanger() 预设（2ms 中心延迟），再覆盖 rate/feedback/mix。
            let f = Chorus::<f32>::flanger()
                .with_rate_hz(*rate_hz)
                .with_feedback(*feedback)
                .with_mix(*mix);
            Fx::Chorus(f)
        }
        Effect::Bitcrusher { bits, downsample, mix } => {
            Fx::Bitcrusher(Bitcrusher::new().with_bits(*bits).with_downsample(*downsample).with_mix(*mix))
        }
        Effect::Tremolo { rate_hz, depth, stereo_phase, shape } => Fx::Tremolo(
            Tremolo::new().with_rate_hz(*rate_hz).with_depth(*depth).with_stereo_phase(*stereo_phase).with_shape(lfo_shape(shape)),
        ),
        Effect::AutoPan { rate_hz, width, shape } => {
            Fx::AutoPan(AutoPan::new().with_rate_hz(*rate_hz).with_width(*width).with_shape(lfo_shape(shape)))
        }
        Effect::RingMod { frequency_hz, mix } => Fx::RingMod(RingModulator::new().with_frequency_hz(*frequency_hz).with_mix(*mix)),
        Effect::FrequencyShifter { shift_hz, feedback, mix } => {
            Fx::FrequencyShifter(FrequencyShifter::new().with_shift_hz(*shift_hz).with_feedback(*feedback).with_mix(*mix))
        }
        Effect::Widener { width, bass_mono_hz } => Fx::Widener(
            StereoWidener::new().with_width(*width).with_bass_mono(*bass_mono_hz > 0.0).with_bass_mono_hz(bass_mono_hz.max(20.0)),
        ),
        Effect::TransientShaper { attack_db, sustain_db, attack_ms, sustain_ms } => Fx::TransientShaper(
            TransientShaper::new().with_attack_db(*attack_db).with_sustain_db(*sustain_db).with_attack_ms(*attack_ms).with_sustain_ms(*sustain_ms),
        ),
        Effect::MultibandCompressor { low_hz, high_hz, threshold_db, ratio, attack_ms, release_ms, low_db, mid_db, high_db } => {
            let mut m = MultibandCompressor::new(*low_hz, *high_hz);
            for (k, makeup) in [*low_db, *mid_db, *high_db].into_iter().enumerate() {
                let b = m.band_mut(k);
                b.set_threshold_db(*threshold_db);
                b.set_ratio(*ratio);
                b.set_attack_ms(*attack_ms);
                b.set_release_ms(*release_ms);
                b.set_makeup_db(makeup);
            }
            Fx::MultibandCompressor(m)
        }
        Effect::Tape { drive_db, asymmetry, wow_cents, flutter_cents, mix } => Fx::Tape(
            TapeSaturation::new().with_drive_db(*drive_db).with_asymmetry(*asymmetry).with_wow_cents(*wow_cents).with_flutter_cents(*flutter_cents).with_mix(*mix),
        ),
        Effect::Vocoder { bands, low_hz, high_hz, formant_shift, attack_ms, release_ms, gain_db, mix, .. } => Fx::Vocoder(MakeupVocoder {
            vocoder: Vocoder::new()
                .with_bands(bands.round() as usize)
                .with_range_hz(*low_hz, *high_hz)
                .with_formant_shift(*formant_shift)
                .with_attack_ms(*attack_ms)
                .with_release_ms(*release_ms)
                .with_gain_db(0.0)
                .with_mix(1.0),
            low_hz: *low_hz,
            high_hz: *high_hz,
            gain: manguebyte::prelude::decibels::db_to_gain(gain_db + VOCODER_MAKEUP_DB) as f32,
            mix: *mix as f32,
            dry: AudioBuffer::new(0, 0),
        }),
        Effect::GranularDelay { position_ms, spray_ms, grain_ms, density_hz, pitch_semitones, pitch_spray, stereo_spread, feedback, mix } => {
            Fx::GranularDelay(
                GranularDelay::new()
                    .with_position_ms(*position_ms)
                    .with_spray_ms(*spray_ms)
                    .with_grain_ms(*grain_ms)
                    .with_density_hz(*density_hz)
                    .with_pitch_semitones(*pitch_semitones)
                    .with_pitch_spray(*pitch_spray)
                    .with_stereo_spread(*stereo_spread)
                    .with_feedback(*feedback)
                    .with_mix(*mix),
            )
        }
        Effect::FormantFilter { vowel, shift, gain_db, mix } => {
            Fx::FormantFilter(FormantFilter::new().with_vowel(*vowel).with_shift(*shift).with_gain_db(*gain_db).with_mix(*mix))
        }
        Effect::Plugin { path, id, params } => build_plugin(path, id.as_deref(), params, spec)?,
    };
    Ok(fx)
}

/// 声码器输出的固定补偿：输出是载波乘调制信号的频带包络，比输入低约 40 dB。补了以后，调制信号是校准过的内置乐器
/// （约 -20 LUFS）时，`gain_db` 为 0 大致和干声一样响。
const VOCODER_MAKEUP_DB: f64 = 40.0;

/// 声码器加补偿增益；干湿在这里混（上游的 mix 固定为 1），补偿不影响干声。
pub struct MakeupVocoder {
    vocoder: Vocoder<f32>,
    /// 频带范围要两端一起设，记下当前值。
    low_hz: f64,
    high_hz: f64,
    gain: f32,
    mix: f32,
    dry: AudioBuffer<f32>,
}

impl MakeupVocoder {
    fn set(&mut self, name: &str, v: f64) {
        match name {
            "bands" => self.vocoder.set_bands(v.round() as usize),
            "low_hz" | "high_hz" => {
                if name == "low_hz" {
                    self.low_hz = v;
                } else {
                    self.high_hz = v;
                }
                self.vocoder.set_range_hz(self.low_hz, self.high_hz);
            }
            "formant_shift" => self.vocoder.set_formant_shift(v),
            "attack_ms" => self.vocoder.set_attack_ms(v),
            "release_ms" => self.vocoder.set_release_ms(v),
            "gain_db" => self.gain = manguebyte::prelude::decibels::db_to_gain(v + VOCODER_MAKEUP_DB) as f32,
            "mix" => self.mix = v as f32,
            _ => {}
        }
    }
    fn run(&mut self, buffer: &mut AudioBuffer<f32>, context: Option<&manguebyte::prelude::ProcessContext<'_, f32>>) {
        let (ch, n) = (buffer.num_channels(), buffer.num_frames());
        if self.dry.num_channels() != ch || self.dry.capacity() < n {
            self.dry = AudioBuffer::new(ch, n);
        }
        self.dry.set_num_frames(n);
        for c in 0..ch {
            self.dry.channel_mut(c).copy_from_slice(buffer.channel(c));
        }
        match context {
            Some(ctx) => self.vocoder.process_with(buffer, ctx),
            None => self.vocoder.process(buffer),
        }
        let (wet, dry) = (self.gain * self.mix, 1.0 - self.mix);
        for c in 0..ch {
            for (o, d) in buffer.channel_mut(c).iter_mut().zip(self.dry.channel(c)) {
                *o = *o * wet + *d * dry;
            }
        }
    }
}

impl Processor<f32> for MakeupVocoder {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.vocoder.prepare(spec);
        self.dry = AudioBuffer::new(spec.num_channels, spec.max_block_size);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.run(buffer, None);
    }
    fn process_with(&mut self, buffer: &mut AudioBuffer<f32>, context: &manguebyte::prelude::ProcessContext<'_, f32>) {
        self.run(buffer, Some(context));
    }
    fn reset(&mut self) {
        self.vocoder.reset();
    }
}

/// tremolo / auto_pan 的 `shape`（已校验过取值）。
fn lfo_shape(shape: &str) -> LfoShape {
    match shape.trim().to_ascii_lowercase().as_str() {
        "triangle" => LfoShape::Triangle,
        "square" => LfoShape::Square,
        "saw_up" => LfoShape::SawUp,
        "saw_down" => LfoShape::SawDown,
        _ => LfoShape::Sine,
    }
}

/// 三段压缩的自动化：分频点、三段共用的动态参数、每段的补偿增益。
fn multiband_set(m: &mut MultibandCompressor<f32>, name: &str, v: f64) {
    match name {
        "low_hz" => m.set_crossover_hz(0, v),
        "high_hz" => m.set_crossover_hz(1, v),
        "low_db" => m.band_mut(0).set_makeup_db(v),
        "mid_db" => m.band_mut(1).set_makeup_db(v),
        "high_db" => m.band_mut(2).set_makeup_db(v),
        _ => {
            for k in 0..MultibandCompressor::<f32>::BANDS {
                let b = m.band_mut(k);
                match name {
                    "threshold_db" => b.set_threshold_db(v),
                    "ratio" => b.set_ratio(v),
                    "attack_ms" => b.set_attack_ms(v),
                    "release_ms" => b.set_release_ms(v),
                    _ => {}
                }
            }
        }
    }
}

/// 把 EQ 的 shape 字符串解析为 manguebyte 的 FilterShape。
fn parse_filter_shape(shape: &str, gain_db: f64) -> Result<FilterShape, EffectError> {
    let s = shape.trim().to_ascii_lowercase();
    Ok(match s.as_str() {
        "lowpass" | "lp" => FilterShape::Lowpass,
        "highpass" | "hp" => FilterShape::Highpass,
        "bandpass" | "bp" => FilterShape::Bandpass,
        "notch" => FilterShape::Notch,
        "allpass" | "ap" => FilterShape::Allpass,
        "peak" | "peaking" | "bell" => FilterShape::Peak { gain_db },
        "low_shelf" | "lowshelf" | "lowshelf_eq" => FilterShape::LowShelf { gain_db },
        "high_shelf" | "highshelf" => FilterShape::HighShelf { gain_db },
        other => {
            return Err(EffectError::UnknownEqShape(other.to_string()));
        }
    })
}

/// 构建第三方插件效果器。仅在 `host` 特性下可用。
#[cfg(feature = "host")]
fn build_plugin(
    path: &str,
    id: Option<&str>,
    params: &serde_json::Map<String, serde_json::Value>,
    spec: &ProcessSpec,
) -> Result<Fx, EffectError> {
    let load_err = |detail: String| EffectError::PluginLoad { path: path.to_string(), detail };
    let mut opened = crate::plugin::Opened::open(path, id).map_err(load_err)?;
    opened.apply(params).map_err(load_err)?;
    let processor = opened.activate(spec.sample_rate, spec.max_block_size).map_err(load_err)?;
    Ok(Fx::Plugin(HostedPlugin {
        processor,
        opened,
        blocks: 0,
        clock: crate::transport::clock(),
        events: manguebyte::prelude::EventBuffer::with_capacity(1),
        sample_rate: spec.sample_rate,
    }))
}

#[cfg(not(feature = "host"))]
fn build_plugin(
    _path: &str,
    _id: Option<&str>,
    _params: &serde_json::Map<String, serde_json::Value>,
    _spec: &ProcessSpec,
) -> Result<Fx, EffectError> {
    Err(EffectError::HostDisabled)
}

/// 一个已激活的插件：控制半与音频半一起持有，音频处理委托给音频半。
/// 字段按声明顺序 drop：先音频半 `processor`，再控制半 `opened`。
#[cfg(feature = "host")]
pub struct HostedPlugin {
    processor: Box<dyn Processor<f32> + Send>,
    opened: crate::plugin::Opened,
    blocks: u32,
    /// 按块推进的走带：tempo 同步的插件效果（延迟、LFO）跟随工程速度。
    clock: crate::transport::Clock,
    events: manguebyte::prelude::EventBuffer,
    sample_rate: f64,
}

#[cfg(feature = "host")]
impl HostedPlugin {
    /// 自动化：`id` 为插件参数的数字 id（字符串形式），值为插件自动化刻度。
    /// 激活后由控制半排队送达，插件在下一块的第一帧收到。
    pub fn set_param(&mut self, id: &str, v: f64) {
        if let Ok(id) = id.parse::<u32>() {
            let _ = self.opened.plugin.set_param(manguebyte::ParamId(id), v);
        }
    }
}

#[cfg(feature = "host")]
impl Processor<f32> for HostedPlugin {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor.prepare(spec);
        if let Err(w) = self.opened.settle(self.processor.as_mut(), spec) {
            eprintln!("[muser] warning: {w}");
        }
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        let transport = self.clock.now(self.sample_rate);
        let ctx = manguebyte::prelude::ProcessContext::new(&self.events, &transport);
        self.processor.process_with(buffer, &ctx);
        self.clock.advance(buffer.num_frames());
        // 定期回应插件的请求（参数回报、重启请求等），避免队列积压。
        self.blocks = self.blocks.wrapping_add(1);
        if self.blocks % 16 == 0 {
            let _ = self.opened.plugin.pump();
        }
    }
    fn reset(&mut self) {
        self.processor.reset();
    }
    fn latency_samples(&self) -> usize {
        self.processor.latency_samples()
    }
}

/// 整条效果链的延迟（帧）：各效果器上报的延迟之和（如 limiter 的前瞻）。
pub fn chain_latency(chain: &[Fx]) -> usize {
    chain
        .iter()
        .map(|fx| match fx {
            Fx::Limiter(l) => l.latency_samples(),
            Fx::Other(p) => p.latency_samples(),
            #[cfg(feature = "host")]
            Fx::Plugin(p) => p.latency_samples(),
            _ => 0,
        })
        .sum()
}

/// 把整条效果链分块施加到 `buffer`（in-place）。`lanes[i]` 是第 i 个效果器的自动化，
/// 有自动化的效果器按 [`crate::automate::BLOCK`] 帧一块，每块开始前写入当前值。
/// `keys[i]` 是第 i 个效果器的侧链信号（压缩器的触发、声码器的调制），与 `buffer` 帧对齐。
/// `windows[i]`：第 i 个效果器起作用的帧 `[from, until)`（`from` / `until`），`at` 是 `buffer` 第一帧的位置；之外原样通过。
#[allow(clippy::too_many_arguments)]
pub fn apply_chain(
    chain: &mut [Fx],
    buffer: &mut AudioBuffer<f32>,
    block_size: usize,
    lanes: &mut [Vec<crate::automate::Lane>],
    beat_at: &dyn Fn(usize) -> f64,
    keys: &[Option<&AudioBuffer<f32>>],
    windows: &[(usize, usize)],
    at: usize,
) {
    let frames = buffer.num_frames();
    for (i, fx) in chain.iter_mut().enumerate() {
        let lanes = lanes.get_mut(i).map(|l| l.as_mut_slice()).unwrap_or(&mut []);
        let key = keys.get(i).copied().flatten();
        let (w0, w1) = windows.get(i).copied().unwrap_or((0, usize::MAX));
        if at + frames <= w0 || at >= w1 {
            continue;
        }
        // 这一段里起作用的部分（相对 buffer）。
        let (lo, hi) = (w0.saturating_sub(at), (w1 - at.min(w1)).min(frames));
        let step = if lanes.is_empty() { block_size } else { block_size.min(crate::automate::BLOCK) };
        let mut start = lo;
        while start < hi {
            let beat = beat_at(start);
            for lane in lanes.iter_mut() {
                if let Some(v) = lane.poll(beat) {
                    fx.set_param(&lane.param, v);
                }
            }
            let end = (start + step).min(hi);
            match (key, fx.processor()) {
                (Some(k), Some(p)) => {
                    let mut ctx = manguebyte::prelude::ProcessContext::empty().with_sidechain(k);
                    ctx.offset = start;
                    buffer.windowed(start..end, |block| p.process_with(block, &ctx));
                }
                _ => buffer.windowed(start..end, |block| fx.process(block)),
            }
            start = end;
        }
    }
}
