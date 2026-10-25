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
}
