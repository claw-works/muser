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

/// 把一组 [`Effect`] 构造成已就绪（prepare 过）的处理器链。
pub fn build_chain(
    effects: &[Effect],
    spec: &ProcessSpec,
) -> Result<Vec<Box<dyn Processor<f32>>>, EffectError> {
    let mut chain = Vec::with_capacity(effects.len());
    for e in effects {
        let mut p = build_one(e, spec)?;
        p.prepare(spec);
        chain.push(p);
    }
    Ok(chain)
}

fn build_one(effect: &Effect, spec: &ProcessSpec) -> Result<Box<dyn Processor<f32>>, EffectError> {
    let boxed: Box<dyn Processor<f32>> = match effect {
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
            Box::new(r)
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
            Box::new(d)
        }
        Effect::Compressor {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
        } => {
            let c = Compressor::<f32>::new()
                .with_threshold_db(*threshold_db)
                .with_ratio(*ratio)
                .with_attack_ms(*attack_ms)
                .with_release_ms(*release_ms)
                .with_makeup_db(*makeup_db);
            Box::new(c)
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
            Box::new(c)
        }
        Effect::Eq {
            shape,
            frequency,
            q,
            gain_db,
        } => {
            let fshape = parse_filter_shape(shape, *gain_db)?;
            Box::new(Filter::<f32>::new(fshape, *frequency, *q))
        }
        Effect::Limiter {
            ceiling_db,
            release_ms,
        } => {
            let l = Limiter::<f32>::new()
                .with_ceiling_db(*ceiling_db)
                .with_release_ms(*release_ms);
            Box::new(l)
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
            Box::new(p)
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
            Box::new(f)
        }
        Effect::Plugin { path, id, params } => build_plugin(path, id.as_deref(), params, spec)?,
    };
    Ok(boxed)
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
    params: &std::collections::BTreeMap<u32, f64>,
    spec: &ProcessSpec,
) -> Result<Box<dyn Processor<f32>>, EffectError> {
    use manguebyte::host::any::Library;

    // 加载插件库。调用方须保证插件路径可信——加载会执行第三方代码。
    let library = unsafe { Library::load(path) }.map_err(|e| EffectError::PluginLoad {
        path: path.to_string(),
        detail: format!("{e:?}"),
    })?;

    // 选择插件 id：显式指定，否则取库中第一个。
    let plugin_id = match id {
        Some(s) => s.to_string(),
        None => library
            .plugins()
            .first()
            .map(|p| p.id.clone())
            .ok_or_else(|| EffectError::PluginLoad {
                path: path.to_string(),
                detail: "库中没有任何插件".to_string(),
            })?,
    };

    let mut plugin = library
        .create(&plugin_id)
        .map_err(|e| EffectError::PluginLoad {
            path: path.to_string(),
            detail: format!("create('{plugin_id}') 失败: {e:?}"),
        })?;

    // 设置参数（best-effort）。
    for (&pid, &value) in params {
        let _ = plugin.set_param(manguebyte::ParamId(pid), value);
    }

    // 激活，拿到音频处理半（它本身就是 Processor）。
    let processor = plugin
        .activate(spec.sample_rate, spec.max_block_size)
        .map_err(|e| EffectError::PluginLoad {
            path: path.to_string(),
            detail: format!("activate 失败: {e:?}"),
        })?;

    // activate 返回 Box<dyn Processor + Send>；为了持有 plugin 控制半不被 drop，
    // 这里把二者打包进一个包装器。
    Ok(Box::new(HostedPlugin {
        _plugin: plugin,
        processor,
    }))
}

#[cfg(not(feature = "host"))]
fn build_plugin(
    _path: &str,
    _id: Option<&str>,
    _params: &std::collections::BTreeMap<u32, f64>,
    _spec: &ProcessSpec,
) -> Result<Box<dyn Processor<f32>>, EffectError> {
    Err(EffectError::HostDisabled)
}

/// 包装一个已激活的插件：控制半与音频半一起持有，音频处理委托给音频半。
/// 控制半 drop 晚于音频半才安全，这里用字段顺序（Rust 按声明顺序逆序 drop，
/// 即先 drop processor 再 drop _plugin）保证之。
#[cfg(feature = "host")]
struct HostedPlugin {
    processor: Box<dyn Processor<f32> + Send>,
    _plugin: manguebyte::host::any::Plugin,
}

#[cfg(feature = "host")]
impl Processor<f32> for HostedPlugin {
    fn prepare(&mut self, spec: &ProcessSpec) {
        self.processor.prepare(spec);
    }
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        self.processor.process(buffer);
    }
    fn reset(&mut self) {
        self.processor.reset();
    }
    fn latency_samples(&self) -> usize {
        self.processor.latency_samples()
    }
}

/// 把整条效果链分块施加到 `buffer`（in-place）。
pub fn apply_chain(
    chain: &mut [Box<dyn Processor<f32>>],
    buffer: &mut AudioBuffer<f32>,
    block_size: usize,
) {
    if chain.is_empty() {
        return;
    }
    let frames = buffer.num_frames();
    for processor in chain.iter_mut() {
        let mut start = 0;
        while start < frames {
            let end = (start + block_size).min(frames);
            buffer.windowed(start..end, |block| processor.process(block));
            start = end;
        }
    }
}
