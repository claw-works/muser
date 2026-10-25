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

use manguebyte::prelude::{
    AudioBuffer, Chorus, Compressor, Delay, Filter, FilterShape, Limiter, Phaser, ProcessSpec,
    Processor, Reverb,
};

/// 效果链构建错误。
#[derive(Debug, thiserror::Error)]
pub enum EffectError {
    #[error("插件宿主未启用：请在构建 muser-engine 时开启 `host` 特性以使用 plugin 效果器")]
    HostDisabled,
    #[error("加载插件 '{path}' 失败: {detail}")]
    PluginLoad { path: String, detail: String },
    #[error("未知的 EQ 滤波形状 '{0}'（可用: lowpass/highpass/bandpass/notch/allpass/peak/low_shelf/high_shelf）")]
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
    Other(Box<dyn Processor<f32>>),
    #[cfg(feature = "host")]
    Plugin(HostedPlugin),
}

impl Fx {
    fn processor(&mut self) -> &mut dyn Processor<f32> {
        match self {
            Fx::Reverb(p) => p,
            Fx::Delay(p) => p,
            Fx::Compressor(p) => p,
            Fx::Chorus(p) => p,
            Fx::Eq { filter, .. } => filter,
            Fx::Limiter(p) => p,
            Fx::Phaser(p) => p,
            Fx::Other(p) => p.as_mut(),
            #[cfg(feature = "host")]
            Fx::Plugin(p) => p,
        }
    }

    pub fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor().prepare(spec);
    }

    pub fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.processor().process(buffer);
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
            #[cfg(feature = "host")]
            (Fx::Plugin(p), id) => p.set_param(id, v),
            _ => {}
        }
    }
}

/// 把一组 [`Effect`] 构造成已就绪（prepare 过）的效果链。
pub fn build_chain(effects: &[Effect], spec: &ProcessSpec) -> Result<Vec<Fx>, EffectError> {
    let mut chain = Vec::with_capacity(effects.len());
    for e in effects {
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
        Effect::Plugin { path, id, params } => build_plugin(path, id.as_deref(), params, spec)?,
    };
    Ok(fx)
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
    Ok(Fx::Plugin(HostedPlugin { processor, opened, blocks: 0 }))
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
            eprintln!("[muser] 警告: {w}");
        }
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.processor.process(buffer);
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

/// 让压缩器改由外部键信号触发（分轨分解时：用全信号作键，各分轨得到与成品相同的增益衰减）。
pub fn key_from_sidechain(fx: &mut Fx) {
    if let Fx::Compressor(c) = fx {
        c.set_key_source(manguebyte::prelude::KeySource::Sidechain);
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
/// `keys[i]` 是第 i 个效果器（侧链压缩器）的触发信号，与 `buffer` 帧对齐。
pub fn apply_chain(
    chain: &mut [Fx],
    buffer: &mut AudioBuffer<f32>,
    block_size: usize,
    lanes: &mut [Vec<crate::automate::Lane>],
    beat_at: &dyn Fn(usize) -> f64,
    keys: &[Option<&AudioBuffer<f32>>],
) {
    let frames = buffer.num_frames();
    for (i, fx) in chain.iter_mut().enumerate() {
        let lanes = lanes.get_mut(i).map(|l| l.as_mut_slice()).unwrap_or(&mut []);
        let key = keys.get(i).copied().flatten();
        let step = if lanes.is_empty() { block_size } else { block_size.min(crate::automate::BLOCK) };
        let mut start = 0;
        while start < frames {
            let beat = beat_at(start);
            for lane in lanes.iter_mut() {
                if let Some(v) = lane.poll(beat) {
                    fx.set_param(&lane.param, v);
                }
            }
            let end = (start + step).min(frames);
            match (key, &mut *fx) {
                (Some(k), Fx::Compressor(c)) => {
                    let mut ctx = manguebyte::prelude::ProcessContext::empty().with_sidechain(k);
                    ctx.offset = start;
                    buffer.windowed(start..end, |block| c.process_with(block, &ctx));
                }
                _ => buffer.windowed(start..end, |block| fx.process(block)),
            }
            start = end;
        }
    }
}
