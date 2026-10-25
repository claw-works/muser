//! 效果器的结构化表示。
//!
//! 效果器挂在 [`crate::Track`] 上（轨级）或 [`crate::Project`] 上（主总线级），
//! 按数组顺序串联处理。AI 在 JSON 里用 `type` 字段区分效果类型，其余字段是参数，
//! 全部有合理默认值——AI 只需写 `{ "type": "reverb" }` 就能得到一个像样的混响。
//!
//! engine 层（muser-engine）负责把这里的 [`Effect`] 解析成 manguebyte 的具体效果器。

use serde::{Deserialize, Serialize};

/// 一个效果器及其参数。JSON 中以 `type` 字段区分（serde 内部标签）。
///
/// 示例：
/// ```json
/// { "type": "reverb", "mix": 0.25, "decay_seconds": 1.8 }
/// { "type": "delay", "time_seconds": 0.3, "feedback": 0.4, "mix": 0.3 }
/// { "type": "compressor", "threshold_db": -18, "ratio": 4 }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    /// 混响。
    Reverb {
        /// 干湿比 0.0-1.0。
        #[serde(default = "default_reverb_mix")]
        mix: f64,
        /// 衰减时间（秒）。
        #[serde(default = "default_reverb_decay")]
        decay_seconds: f64,
        /// 高频阻尼 0.0-1.0。
        #[serde(default = "default_reverb_damping")]
        damping: f64,
        /// 空间大小 0.0-1.0。
        #[serde(default = "default_reverb_size")]
        size: f64,
    },
    /// 延迟/回声。
    Delay {
        /// 延迟时间（秒）。
        #[serde(default = "default_delay_time")]
        time_seconds: f64,
        /// 反馈量 0.0-1.0（越高回声越多）。
        #[serde(default = "default_delay_feedback")]
        feedback: f64,
        /// 干湿比 0.0-1.0。
        #[serde(default = "default_delay_mix")]
        mix: f64,
    },
    /// 压缩器。
    Compressor {
        /// 阈值（dB），低于此值不压缩。
        #[serde(default = "default_comp_threshold")]
        threshold_db: f64,
        /// 压缩比（如 4 表示 4:1）。
        #[serde(default = "default_comp_ratio")]
        ratio: f64,
        /// 启动时间（毫秒）。
        #[serde(default = "default_comp_attack")]
        attack_ms: f64,
        /// 释放时间（毫秒）。
        #[serde(default = "default_comp_release")]
        release_ms: f64,
        /// 补偿增益（dB）。
        #[serde(default)]
        makeup_db: f64,
    },
    /// 第三方插件作为效果器（CLAP/VST3/LV2）。
    ///
    /// 需要在构建 muser-engine 时启用 `host` 特性，否则渲染会报错。
    Plugin {
        /// 插件文件/包路径（如 `.clap` / `.vst3` / `.lv2`）。
        path: String,
        /// 当一个文件内含多个插件时，指定插件 id；缺省用第一个。
        #[serde(default)]
        id: Option<String>,
        /// 要设置的插件参数（参数 id → 值），可省略。
        #[serde(default)]
        params: std::collections::BTreeMap<u32, f64>,
    },
    /// 合唱：加厚、加宽音色。
    Chorus {
        /// 调制速率（Hz）。
        #[serde(default = "default_chorus_rate")]
        rate_hz: f64,
        /// 调制深度（毫秒）。
        #[serde(default = "default_chorus_depth")]
        depth_ms: f64,
        /// 干湿比 0.0-1.0。
        #[serde(default = "default_chorus_mix")]
        mix: f64,
    },
    /// 均衡器：一个滤波/EQ 频段。
    Eq {
        /// 滤波形状：`lowpass` `highpass` `bandpass` `notch` `allpass`
        /// `peak` `low_shelf` `high_shelf`。
        #[serde(default = "default_eq_shape")]
        shape: String,
        /// 频率（Hz）。
        #[serde(default = "default_eq_freq")]
        frequency: f64,
        /// Q 值（带宽/谐振）。
        #[serde(default = "default_eq_q")]
        q: f64,
        /// 增益（dB），仅对 `peak` / `low_shelf` / `high_shelf` 有意义。
        #[serde(default)]
        gain_db: f64,
    },
    /// 限幅器：限制峰值，放在主总线末端防爆。
    Limiter {
        /// 输出上限（dB），一般 0 到 -1。
        #[serde(default = "default_limiter_ceiling")]
        ceiling_db: f64,
        /// 释放时间（毫秒）。
        #[serde(default = "default_limiter_release")]
        release_ms: f64,
    },
}

// —— 各参数的默认值（选取音乐上常用、安全的起点）——
fn default_reverb_mix() -> f64 {
    0.25
}
fn default_reverb_decay() -> f64 {
    1.6
}
fn default_reverb_damping() -> f64 {
    0.5
}
fn default_reverb_size() -> f64 {
    0.5
}
fn default_delay_time() -> f64 {
    0.3
}
fn default_delay_feedback() -> f64 {
    0.35
}
fn default_delay_mix() -> f64 {
    0.3
}
fn default_comp_threshold() -> f64 {
    -18.0
}
fn default_comp_ratio() -> f64 {
    4.0
}
fn default_comp_attack() -> f64 {
    10.0
}
fn default_comp_release() -> f64 {
    120.0
}
fn default_chorus_rate() -> f64 {
    0.8
}
fn default_chorus_depth() -> f64 {
    3.0
}
fn default_chorus_mix() -> f64 {
    0.4
}
fn default_eq_shape() -> String {
    "peak".to_string()
}
fn default_eq_freq() -> f64 {
    1000.0
}
fn default_eq_q() -> f64 {
    0.707
}
fn default_limiter_ceiling() -> f64 {
    -0.3
}
fn default_limiter_release() -> f64 {
    50.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverb_minimal_json_uses_defaults() {
        let e: Effect = serde_json::from_str(r#"{ "type": "reverb" }"#).unwrap();
        match e {
            Effect::Reverb { mix, decay_seconds, .. } => {
                assert_eq!(mix, 0.25);
                assert_eq!(decay_seconds, 1.6);
            }
            _ => panic!("应解析为 Reverb"),
        }
    }

    #[test]
    fn delay_roundtrip() {
        let e = Effect::Delay {
            time_seconds: 0.25,
            feedback: 0.5,
            mix: 0.4,
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: Effect = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"type\":\"delay\""));
    }

    #[test]
    fn compressor_partial_json() {
        let e: Effect =
            serde_json::from_str(r#"{ "type": "compressor", "ratio": 8 }"#).unwrap();
        match e {
            Effect::Compressor { ratio, threshold_db, .. } => {
                assert_eq!(ratio, 8.0);
                assert_eq!(threshold_db, -18.0); // 默认
            }
            _ => panic!("应解析为 Compressor"),
        }
    }

    #[test]
    fn new_effects_minimal_json() {
        let chorus: Effect = serde_json::from_str(r#"{ "type": "chorus" }"#).unwrap();
        assert!(matches!(chorus, Effect::Chorus { .. }));

        let eq: Effect =
            serde_json::from_str(r#"{ "type": "eq", "shape": "high_shelf", "frequency": 8000, "gain_db": 3 }"#)
                .unwrap();
        match eq {
            Effect::Eq { shape, frequency, gain_db, .. } => {
                assert_eq!(shape, "high_shelf");
                assert_eq!(frequency, 8000.0);
                assert_eq!(gain_db, 3.0);
            }
            _ => panic!("应解析为 Eq"),
        }

        let lim: Effect = serde_json::from_str(r#"{ "type": "limiter" }"#).unwrap();
        match lim {
            Effect::Limiter { ceiling_db, .. } => assert_eq!(ceiling_db, -0.3),
            _ => panic!("应解析为 Limiter"),
        }
    }
}
