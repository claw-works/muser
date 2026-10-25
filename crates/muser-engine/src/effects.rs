//! 效果器适配：把 [`muser_core::Effect`] 解析为 manguebyte 的效果处理器，
//! 并在离线渲染时分块施加到音频缓冲上。
//!
//! 所有效果器都实现 manguebyte 的 `Processor` trait（in-place `process`），
//! 因此一条效果链就是一串 `Box<dyn Processor>`，依次对缓冲处理即可。

use muser_core::Effect;

use manguebyte::prelude::{AudioBuffer, Compressor, Delay, ProcessSpec, Processor, Reverb};

/// 把一组 [`Effect`] 构造成已就绪（prepare 过）的处理器链。
pub fn build_chain(effects: &[Effect], spec: &ProcessSpec) -> Vec<Box<dyn Processor<f32>>> {
    effects
        .iter()
        .map(|e| {
            let mut p = build_one(e, spec);
            p.prepare(spec);
            p
        })
        .collect()
}

fn build_one(effect: &Effect, _spec: &ProcessSpec) -> Box<dyn Processor<f32>> {
    match *effect {
        Effect::Reverb {
            mix,
            decay_seconds,
            damping,
            size,
        } => {
            let mut r = Reverb::<f32>::new()
                .with_mix(mix)
                .with_decay_seconds(decay_seconds)
                .with_damping(damping);
            r.set_size(size);
            Box::new(r) as Box<dyn Processor<f32>>
        }
        Effect::Delay {
            time_seconds,
            feedback,
            mix,
        } => {
            // 最大延迟时间给足余量（至少 2s 或所需时间）。
            let max = time_seconds.max(2.0);
            let d = Delay::<f32>::new(max)
                .with_time(time_seconds)
                .with_feedback(feedback)
                .with_mix(mix);
            Box::new(d) as Box<dyn Processor<f32>>
        }
        Effect::Compressor {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
        } => {
            let c = Compressor::<f32>::new()
                .with_threshold_db(threshold_db)
                .with_ratio(ratio)
                .with_attack_ms(attack_ms)
                .with_release_ms(release_ms)
                .with_makeup_db(makeup_db);
            Box::new(c) as Box<dyn Processor<f32>>
        }
    }
}
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
