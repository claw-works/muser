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
        /// 侧链（可省略）：用另一条音轨或总线的信号（按名称）来触发压缩，
        /// 例如 `"kick"`：底鼓一响就把本轨压下去（ducking / 泵感）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sidechain: Option<String>,
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
        /// 插件参数（可省略）：键为插件参数名（大小写不敏感）或数字 id；值为文字（交给插件解析，
        /// 如 "2000 Hz"）、数字（插件自动化刻度）或布尔。用 `muser plugin params` 查看。
        #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
        params: serde_json::Map<String, serde_json::Value>,
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
    /// 相位器：扫频的梳状滤波，营造流动的"喷气式"质感。
    Phaser {
        /// 调制速率（Hz）。
        #[serde(default = "default_phaser_rate")]
        rate_hz: f64,
        /// 反馈量 0.0-1.0（越高谐振越强）。
        #[serde(default = "default_phaser_feedback")]
        feedback: f64,
        /// 干湿比 0.0-1.0。
        #[serde(default = "default_phaser_mix")]
        mix: f64,
    },
    /// 镶边：短延迟的调制，营造金属感的扫动。
    Flanger {
        /// 调制速率（Hz）。
        #[serde(default = "default_flanger_rate")]
        rate_hz: f64,
        /// 反馈量 0.0-1.0。
        #[serde(default = "default_flanger_feedback")]
        feedback: f64,
        /// 干湿比 0.0-1.0。
        #[serde(default = "default_flanger_mix")]
        mix: f64,
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
fn default_phaser_rate() -> f64 {
    0.5
}
fn default_phaser_feedback() -> f64 {
    0.3
}
fn default_phaser_mix() -> f64 {
    0.5
}
fn default_flanger_rate() -> f64 {
    0.2
}
fn default_flanger_feedback() -> f64 {
    0.7
}
fn default_flanger_mix() -> f64 {
    0.5
}

// ============================================================================
// 效果器描述表：每个效果器的名称、参数、范围、默认值、说明只在这里写一份。
// CLI (`muser effects`)、MCP (`list_effects` / `describe_effect`) 与
// `Project::validate` 的参数范围校验都从这里取数据。
// ============================================================================

/// 一个数值参数的描述。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ParamSpec {
    /// JSON 字段名。
    pub name: &'static str,
    /// 允许的最小值（校验用，含边界）。
    pub min: f64,
    /// 允许的最大值（校验用，含边界）。
    pub max: f64,
    /// 省略时的默认值。
    pub default: f64,
    /// 单位（"" 表示无单位的比例值）。
    pub unit: &'static str,
    /// 作用与听感说明，含常用取值。
    pub description: &'static str,
}

/// 一个字符串（枚举型）参数的描述。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ChoiceSpec {
    pub name: &'static str,
    pub choices: &'static [&'static str],
    pub default: &'static str,
    pub description: &'static str,
}

/// 一个效果器的完整描述。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EffectSpec {
    /// `type` 字段的值。
    pub name: &'static str,
    /// 一句话说明作用。
    pub summary: &'static str,
    /// 适合放在哪里、和谁串联。
    pub placement: &'static str,
    /// 数值参数。
    pub params: &'static [ParamSpec],
    /// 字符串参数。
    pub choices: &'static [ChoiceSpec],
    /// 一个可直接使用的 JSON 示例。
    pub example: &'static str,
}

const fn p(
    name: &'static str,
    min: f64,
    max: f64,
    default: f64,
    unit: &'static str,
    description: &'static str,
) -> ParamSpec {
    ParamSpec { name, min, max, default, unit, description }
}

/// 所有效果器的描述表。
pub static EFFECT_SPECS: &[EffectSpec] = &[
    EffectSpec {
        name: "reverb",
        summary: "混响：模拟房间/大厅的空间感与尾音。",
        placement: "放在旋律、pad、人声类轨的链尾；贝斯、底鼓少用或不用，否则低频发浑。",
        params: &[
            p("mix", 0.0, 1.0, 0.25, "", "干湿比。0.1-0.2 轻微空间；0.25-0.35 常规；>0.5 很湿、声音变远"),
            p("decay_seconds", 0.05, 120.0, 1.6, "s", "尾音衰减时间。0.3-0.8 小房间；1.2-2.5 大厅；>4 氛围/环境"),
            p("damping", 0.0, 1.0, 0.5, "", "高频阻尼。越大尾音越暗越温暖；越小越亮越金属"),
            p("size", 0.0, 1.0, 0.5, "", "空间大小。0.2 小房间；0.5 中等；0.8+ 大厅/教堂"),
        ],
        choices: &[],
        example: r#"{ "type": "reverb", "mix": 0.3, "decay_seconds": 2.2, "size": 0.7 }"#,
    },
    EffectSpec {
        name: "delay",
        summary: "延迟/回声：把信号延后若干时间重复播放。",
        placement: "适合 lead、和弦、人声；放在混响之前。time 按节拍算更合拍：60/tempo 秒 = 一拍。",
        params: &[
            p("time_seconds", 0.0, 10.0, 0.3, "s", "回声间隔。按节拍取值：四分音符=60/tempo，八分=30/tempo，附点八分=45/tempo"),
            p("feedback", 0.0, 0.99, 0.35, "", "回声重复次数。0.2 一两次；0.4-0.5 多次；>0.7 会长时间堆积"),
            p("mix", 0.0, 1.0, 0.3, "", "干湿比。0.2-0.35 常规；>0.5 回声比原声还响"),
        ],
        choices: &[],
        example: r#"{ "type": "delay", "time_seconds": 0.35, "feedback": 0.4, "mix": 0.3 }"#,
    },
    EffectSpec {
        name: "compressor",
        summary: "压缩器：压低超过阈值的峰值，让动态更平稳、更紧实。",
        placement: "单轨上放在链首（EQ 之后）控制动态；主总线上做\"胶水\"，在 limiter 之前。写 \"sidechain\": \"<音轨或总线名>\" 时改由那条信号触发：底鼓压贝斯/pad 的泵感（常用 threshold -30、ratio 6-10、attack 1-5、release 100-200）。",
        params: &[
            p("threshold_db", -60.0, 0.0, -18.0, "dB", "阈值。越低压得越多。轻压 -12；常规 -18；重压 -30"),
            p("ratio", 1.0, 100.0, 4.0, ":1", "压缩比。2 温和；4 常规；8+ 接近限幅"),
            p("attack_ms", 0.01, 1000.0, 10.0, "ms", "启动时间。短(1-5)压住瞬态；长(20-50)保留鼓的冲击"),
            p("release_ms", 1.0, 5000.0, 120.0, "ms", "释放时间。短(50)跟得紧有泵感；长(200-500)更自然"),
            p("makeup_db", -24.0, 36.0, 0.0, "dB", "补偿增益，弥补压缩损失的音量。一般 0-6"),
        ],
        choices: &[],
        example: r#"{ "type": "compressor", "threshold_db": -16, "ratio": 3, "makeup_db": 2 }"#,
    },
    EffectSpec {
        name: "chorus",
        summary: "合唱：叠加轻微失谐的副本，让音色变厚变宽。",
        placement: "适合 pad、电钢、吉他、和弦；放在混响之前。贝斯慎用（低频相位抵消）。",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.8, "Hz", "调制速度。0.2-1 缓慢自然；2-5 明显颤动"),
            p("depth_ms", 0.0, 50.0, 3.0, "ms", "调制深度。1-3 细腻；5-10 明显失谐"),
            p("mix", 0.0, 1.0, 0.4, "", "干湿比。0.3-0.5 常规"),
        ],
        choices: &[],
        example: r#"{ "type": "chorus", "rate_hz": 0.6, "depth_ms": 4, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "eq",
        summary: "均衡/滤波：提升、衰减或切除某个频段。一个 eq 只处理一个频段，多段请串多个。",
        placement: "通常放在链首。常用：highpass 切掉非贝斯轨的低频浑浊；high_shelf 提亮；peak 修某个频点。",
        params: &[
            p("frequency", 10.0, 24000.0, 1000.0, "Hz", "作用频率。低频 60-250；中频 250-4000；高频 4000-16000"),
            p("q", 0.05, 40.0, 0.707, "", "带宽/谐振。0.707 平缓；1-2 中等；>5 很窄（修尖峰用）"),
            p("gain_db", -36.0, 36.0, 0.0, "dB", "增益，只对 peak / low_shelf / high_shelf 有效。常用 ±2 到 ±6"),
        ],
        choices: &[ChoiceSpec {
            name: "shape",
            choices: &["lowpass", "highpass", "bandpass", "notch", "allpass", "peak", "low_shelf", "high_shelf"],
            default: "peak",
            description: "滤波形状。lowpass 保留低于 frequency 的部分（变闷）；highpass 保留高于的部分（去低频）；peak 在 frequency 处增减；low_shelf/high_shelf 整体增减低/高频",
        }],
        example: r#"{ "type": "eq", "shape": "high_shelf", "frequency": 8000, "gain_db": 3 }"#,
    },
    EffectSpec {
        name: "limiter",
        summary: "限幅器：保证输出峰值不超过上限，防止削顶爆音。",
        placement: "只放在主总线（master_effects）的最后一个。",
        params: &[
            p("ceiling_db", -24.0, 0.0, -0.3, "dB", "输出上限。-0.3 到 -1 常用"),
            p("release_ms", 1.0, 2000.0, 50.0, "ms", "释放时间。短更响但可能失真；长更平滑"),
        ],
        choices: &[],
        example: r#"{ "type": "limiter", "ceiling_db": -0.5 }"#,
    },
    EffectSpec {
        name: "phaser",
        summary: "相位器：扫动的陷波，产生流动的\"喷气式\"质感。",
        placement: "适合 pad、电钢、吉他、合成器 lead；放在混响之前。",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.5, "Hz", "扫动速度。0.1-0.5 缓慢；1-3 明显"),
            p("feedback", -0.95, 0.95, 0.3, "", "谐振强度。绝对值越大越尖锐；负值音色不同"),
            p("mix", 0.0, 1.0, 0.5, "", "干湿比。0.5 效果最明显"),
        ],
        choices: &[],
        example: r#"{ "type": "phaser", "rate_hz": 0.3, "feedback": 0.5, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "flanger",
        summary: "镶边：极短延迟的调制，产生金属感的扫动（喷气机声）。",
        placement: "适合合成器、吉他、鼓的点缀段落；放在混响之前，通常不长时间整首使用。",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.2, "Hz", "扫动速度。0.1-0.3 慢扫；>1 快速颤动"),
            p("feedback", -0.95, 0.95, 0.7, "", "谐振强度。越大金属感越强"),
            p("mix", 0.0, 1.0, 0.5, "", "干湿比。0.5 效果最明显"),
        ],
        choices: &[],
        example: r#"{ "type": "flanger", "rate_hz": 0.2, "feedback": 0.7, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "plugin",
        summary: "第三方插件效果器（VST3/CLAP/LV2）。需要用 host 特性构建。",
        placement: "与内置效果器一样串在链上。path 是本机插件的绝对路径；params 按插件参数名设置，值可写插件界面上的文字（如 \"2000 Hz\"）。用 `muser plugin params <path>` 查看参数。",
        params: &[],
        choices: &[],
        example: r#"{ "type": "plugin", "path": "/usr/lib/clap/Surge XT Effects.clap", "params": { "FX Type": "Reverb 2" } }"#,
    },
];

/// 按名称查描述（大小写不敏感）。
pub fn effect_spec(name: &str) -> Option<&'static EffectSpec> {
    EFFECT_SPECS.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim()))
}

impl Effect {
    /// `type` 字段的值。
    pub fn type_name(&self) -> &'static str {
        match self {
            Effect::Reverb { .. } => "reverb",
            Effect::Delay { .. } => "delay",
            Effect::Compressor { .. } => "compressor",
            Effect::Plugin { .. } => "plugin",
            Effect::Chorus { .. } => "chorus",
            Effect::Eq { .. } => "eq",
            Effect::Limiter { .. } => "limiter",
            Effect::Phaser { .. } => "phaser",
            Effect::Flanger { .. } => "flanger",
        }
    }

    /// 数值参数（字段名, 当前值），与描述表中的 `params` 一一对应。
    pub fn numeric_params(&self) -> Vec<(&'static str, f64)> {
        match *self {
            Effect::Reverb { mix, decay_seconds, damping, size } => vec![
                ("mix", mix),
                ("decay_seconds", decay_seconds),
                ("damping", damping),
                ("size", size),
            ],
            Effect::Delay { time_seconds, feedback, mix } => {
                vec![("time_seconds", time_seconds), ("feedback", feedback), ("mix", mix)]
            }
            Effect::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup_db, .. } => vec![
                ("threshold_db", threshold_db),
                ("ratio", ratio),
                ("attack_ms", attack_ms),
                ("release_ms", release_ms),
                ("makeup_db", makeup_db),
            ],
            Effect::Chorus { rate_hz, depth_ms, mix } => {
                vec![("rate_hz", rate_hz), ("depth_ms", depth_ms), ("mix", mix)]
            }
            Effect::Eq { frequency, q, gain_db, .. } => {
                vec![("frequency", frequency), ("q", q), ("gain_db", gain_db)]
            }
            Effect::Limiter { ceiling_db, release_ms } => {
                vec![("ceiling_db", ceiling_db), ("release_ms", release_ms)]
            }
            Effect::Phaser { rate_hz, feedback, mix } | Effect::Flanger { rate_hz, feedback, mix } => {
                vec![("rate_hz", rate_hz), ("feedback", feedback), ("mix", mix)]
            }
            Effect::Plugin { .. } => Vec::new(),
        }
    }

    /// 按描述表检查参数范围与枚举取值。错误信息可直接展示给 AI。
    pub fn validate(&self) -> Result<(), String> {
        let name = self.type_name();
        let spec = effect_spec(name).expect("每个 Effect 变体都应有描述");
        for (field, value) in self.numeric_params() {
            let ps = spec
                .params
                .iter()
                .find(|p| p.name == field)
                .expect("numeric_params 字段应在描述表中");
            if !value.is_finite() || value < ps.min || value > ps.max {
                return Err(format!(
                    "{name}.{field} = {value} 超出范围 [{}, {}]{}",
                    ps.min,
                    ps.max,
                    if ps.unit.is_empty() { String::new() } else { format!(" {}", ps.unit) }
                ));
            }
        }
        match self {
            Effect::Eq { shape, .. } => {
                let cs = &spec.choices[0];
                let s = shape.trim().to_ascii_lowercase();
                if !cs.choices.contains(&s.as_str()) {
                    return Err(format!(
                        "eq.shape = '{shape}' 无效，可选: {}",
                        cs.choices.join(" / ")
                    ));
                }
            }
            Effect::Compressor { sidechain: Some(s), .. } if s.trim().is_empty() => {
                return Err("compressor.sidechain 不能为空字符串".to_string());
            }
            Effect::Plugin { path, .. } if path.trim().is_empty() => {
                return Err("plugin.path 不能为空".to_string());
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_cover_every_variant_and_defaults_match() {
        // 用最小 JSON 构造每种效果器，确认描述表的默认值与 serde 默认值一致。
        for spec in EFFECT_SPECS {
            if spec.name == "plugin" {
                continue;
            }
            let e: Effect = serde_json::from_str(&format!(r#"{{ "type": "{}" }}"#, spec.name))
                .unwrap_or_else(|err| panic!("{} 应能用最小 JSON 解析: {err}", spec.name));
            assert_eq!(e.type_name(), spec.name);
            let actual = e.numeric_params();
            assert_eq!(actual.len(), spec.params.len(), "{} 参数个数不一致", spec.name);
            for (field, value) in actual {
                let ps = spec.params.iter().find(|p| p.name == field).unwrap();
                assert_eq!(value, ps.default, "{}.{} 默认值不一致", spec.name, field);
            }
            e.validate().unwrap();
            // 示例 JSON 必须能解析并通过校验。
            let ex: Effect = serde_json::from_str(spec.example).unwrap();
            ex.validate().unwrap();
        }
    }

    #[test]
    fn validate_rejects_out_of_range() {
        let e: Effect = serde_json::from_str(r#"{ "type": "reverb", "mix": 1.5 }"#).unwrap();
        let err = e.validate().unwrap_err();
        assert!(err.contains("reverb.mix"), "{err}");
        let e: Effect = serde_json::from_str(r#"{ "type": "eq", "shape": "bogus" }"#).unwrap();
        assert!(e.validate().unwrap_err().contains("eq.shape"));
    }

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

        let ph: Effect = serde_json::from_str(r#"{ "type": "phaser" }"#).unwrap();
        assert!(matches!(ph, Effect::Phaser { .. }));
        let fl: Effect = serde_json::from_str(r#"{ "type": "flanger", "mix": 0.6 }"#).unwrap();
        match fl {
            Effect::Flanger { mix, feedback, .. } => {
                assert_eq!(mix, 0.6);
                assert_eq!(feedback, 0.7); // 默认
            }
            _ => panic!("应解析为 Flanger"),
        }
    }
}
