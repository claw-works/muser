//! 效果器的结构化表示。
//!
//! 效果器挂在 [`crate::Track`] 上（轨级）或 [`crate::Project`] 上（主总线级），
//! 按数组顺序串联处理。AI 在 JSON 里用 `type` 字段区分效果类型，其余字段是参数，
//! 全部有合理默认值——AI 只需写 `{ "type": "reverb" }` 就能得到一个像样的混响。
//!
//! engine 层（muser-engine）负责把这里的 [`Effect`] 解析成 manguebyte 的具体效果器。

use serde::{Deserialize, Serialize};

/// 效果链里的一格：效果器本身，加上是否旁通（`"bypass": true` 时不处理声音，位置与参数保留，
/// 自动化的 `effects[i]` 序号不变）。JSON 里 `bypass` 与效果器的字段写在一起。
///
/// `from` / `until`（拍）：只在这段时间里起作用——之前不处理（状态是空的，像中途才插上），
/// 之后撤下（尾巴截断）。`muser live` 中途加、撤效果器就写成这样，离线渲染结果相同。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectSlot {
    #[serde(flatten)]
    pub effect: Effect,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<crate::Beats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<crate::Beats>,
}

impl EffectSlot {
    /// 参数与起止时间是否合理。
    pub fn validate(&self) -> Result<(), String> {
        let ok = |b: Option<crate::Beats>| b.is_none_or(|b| b.0.is_finite() && b.0 >= 0.0);
        if !ok(self.from) || !ok(self.until) {
            return Err("from / until 应为不小于 0 的拍数".into());
        }
        if let (Some(a), Some(z)) = (self.from, self.until) {
            if z.0 <= a.0 {
                return Err(format!("until（{}）要晚于 from（{}）", z.0, a.0));
            }
        }
        self.effect.validate()
    }
}

impl std::ops::Deref for EffectSlot {
    type Target = Effect;
    fn deref(&self) -> &Effect {
        &self.effect
    }
}

impl std::ops::DerefMut for EffectSlot {
    fn deref_mut(&mut self) -> &mut Effect {
        &mut self.effect
    }
}

impl From<Effect> for EffectSlot {
    fn from(effect: Effect) -> Self {
        Self { effect, bypass: false, from: None, until: None }
    }
}

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
    /// 降位降采样：变粗、变脏的数码失真（8-bit、老采样器、lo-fi 质感）。
    Bitcrusher {
        /// 位深。12 轻微颗粒；8 老游戏机；4 以下非常脏
        #[serde(default = "default_bitcrusher_bits")]
        bits: f64,
        /// 降采样倍数（保持每 N 个样本）。1 不降；4-8 明显的金属锯齿感
        #[serde(default = "default_bitcrusher_downsample")]
        downsample: f64,
        /// 干湿比。0.3-0.5 只加一点颗粒
        #[serde(default = "default_bitcrusher_mix")]
        mix: f64,
    },
    /// 颤音（音量）：按 LFO 周期性地起伏音量。
    Tremolo {
        /// 起伏速度。3-6 经典电钢颤音；按节拍取值更合拍
        #[serde(default = "default_tremolo_rate_hz")]
        rate_hz: f64,
        /// 深度。0.3 轻微；1 起伏到无声
        #[serde(default = "default_tremolo_depth")]
        depth: f64,
        /// 左右声道的相位差（周）。0.5 左右交替，带一点自动声像感
        #[serde(default = "default_tremolo_stereo_phase")]
        stereo_phase: f64,
        /// LFO 波形：sine 柔和；triangle；square 斩波（chopper）效果；saw_up / saw_down 锯齿
        #[serde(default = "default_tremolo_shape")]
        shape: String,
    },
    /// 自动声像：按 LFO 让声音在左右之间来回移动。
    AutoPan {
        /// 移动速度。0.1-0.5 缓慢漂移；2 以上快速晃动
        #[serde(default = "default_auto_pan_rate_hz")]
        rate_hz: f64,
        /// 摆动幅度。1 从最左到最右；0.3 只在中间附近晃
        #[serde(default = "default_auto_pan_width")]
        width: f64,
        /// LFO 波形：sine 平滑；square 左右跳
        #[serde(default = "default_auto_pan_shape")]
        shape: String,
    },
    /// 环形调制：和一个正弦相乘，产生不和谐的金属、钟、机器人音色。
    RingMod {
        /// 载波频率。20-30 颤动；100-1000 金属/钟声
        #[serde(default = "default_ring_mod_frequency_hz")]
        frequency_hz: f64,
        /// 干湿比。1 只剩调制后的声音
        #[serde(default = "default_ring_mod_mix")]
        mix: f64,
    },
    /// 移频：把所有频率平移固定的赫兹数（不同于变调，泛音关系被打破）。
    FrequencyShifter {
        /// 平移量。±1-10 细微旋转；±50 以上明显失谐
        #[serde(default = "default_frequency_shifter_shift_hz")]
        shift_hz: f64,
        /// 反馈。越大越像无尽的上升/下降螺旋（巴伯音）
        #[serde(default = "default_frequency_shifter_feedback")]
        feedback: f64,
        /// 干湿比。0.5 时小移频有相位器般的效果
        #[serde(default = "default_frequency_shifter_mix")]
        mix: f64,
    },
    /// 立体声扩展：加大左右差异，让声音更宽；低频可保持单声道。
    Widener {
        /// 宽度。1 不变；1.2-1.6 常用；0 变单声道
        #[serde(default = "default_widener_width")]
        width: f64,
        /// 低于这个频率保持单声道（低频不散）。0 关闭
        #[serde(default = "default_widener_bass_mono_hz")]
        bass_mono_hz: f64,
    },
    /// 瞬态塑形：单独增强/减弱起音和延音，不管音量多大。
    TransientShaper {
        /// 起音增益。+ 更冲、更脆；- 更软、更远
        #[serde(default = "default_transient_shaper_attack_db")]
        attack_db: f64,
        /// 延音增益。+ 更多尾音和房间感；- 更干更紧
        #[serde(default = "default_transient_shaper_sustain_db")]
        sustain_db: f64,
        /// 起音段的长度。鼓 5-20
        #[serde(default = "default_transient_shaper_attack_ms")]
        attack_ms: f64,
        /// 延音段的反应时间
        #[serde(default = "default_transient_shaper_sustain_ms")]
        sustain_ms: f64,
    },
    /// 三段压缩：分成低、中、高三段各自压缩，低频的重音不会压到高频。
    MultibandCompressor {
        /// 低频段与中频段的分频点
        #[serde(default = "default_multiband_compressor_low_hz")]
        low_hz: f64,
        /// 中频段与高频段的分频点（要高于 low_hz）
        #[serde(default = "default_multiband_compressor_high_hz")]
        high_hz: f64,
        /// 三段共用的阈值
        #[serde(default = "default_multiband_compressor_threshold_db")]
        threshold_db: f64,
        /// 三段共用的压缩比。2-3 母带；4+ 更紧
        #[serde(default = "default_multiband_compressor_ratio")]
        ratio: f64,
        /// 启动时间
        #[serde(default = "default_multiband_compressor_attack_ms")]
        attack_ms: f64,
        /// 释放时间
        #[serde(default = "default_multiband_compressor_release_ms")]
        release_ms: f64,
        /// 低频段补偿增益
        #[serde(default = "default_multiband_compressor_low_db")]
        low_db: f64,
        /// 中频段补偿增益
        #[serde(default = "default_multiband_compressor_mid_db")]
        mid_db: f64,
        /// 高频段补偿增益
        #[serde(default = "default_multiband_compressor_high_db")]
        high_db: f64,
    },
    /// 磁带：磁带饱和的温暖失真，加上走带不稳的音高晃动（wow / flutter）。
    Tape {
        /// 驱动。3-6 温暖；12 以上明显失真
        #[serde(default = "default_tape_drive_db")]
        drive_db: f64,
        /// 不对称，加偶次谐波（更暖）
        #[serde(default = "default_tape_asymmetry")]
        asymmetry: f64,
        /// 慢速音高晃动的深度。0 关；5-15 老卡带
        #[serde(default = "default_tape_wow_cents")]
        wow_cents: f64,
        /// 快速音高抖动的深度
        #[serde(default = "default_tape_flutter_cents")]
        flutter_cents: f64,
        /// 干湿比
        #[serde(default = "default_tape_mix")]
        mix: f64,
    },
    /// 声码器：用另一条轨（多是人声）的频谱去塑造这条轨（多是合成器和弦）——机器人唱歌。
    Vocoder {
        /// 频带数。8 粗糙复古；16 常规；32 更清楚
        #[serde(default = "default_vocoder_bands")]
        bands: f64,
        /// 最低频带
        #[serde(default = "default_vocoder_low_hz")]
        low_hz: f64,
        /// 最高频带
        #[serde(default = "default_vocoder_high_hz")]
        high_hz: f64,
        /// 共振峰平移。+ 更尖（小孩、卡通）；- 更粗
        #[serde(default = "default_vocoder_formant_shift")]
        formant_shift: f64,
        /// 包络跟随的启动时间
        #[serde(default = "default_vocoder_attack_ms")]
        attack_ms: f64,
        /// 包络跟随的释放时间。长一点更连贯
        #[serde(default = "default_vocoder_release_ms")]
        release_ms: f64,
        /// 输出增益
        #[serde(default = "default_vocoder_gain_db")]
        gain_db: f64,
        /// 干湿比
        #[serde(default = "default_vocoder_mix")]
        mix: f64,
        /// 调制信号（音轨或总线名，多是人声）：它的频谱塑造本轨的声音。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sidechain: Option<String>,
    },
    /// 颗粒延迟：把延迟线切成小颗粒随机重放，能变调、散开，做出云雾、闪烁的氛围。
    GranularDelay {
        /// 颗粒从多久之前取
        #[serde(default = "default_granular_delay_position_ms")]
        position_ms: f64,
        /// 取样位置的随机范围
        #[serde(default = "default_granular_delay_spray_ms")]
        spray_ms: f64,
        /// 颗粒长度。短（10-30）颗粒感强；长（100+）更平滑
        #[serde(default = "default_granular_delay_grain_ms")]
        grain_ms: f64,
        /// 每秒多少颗粒
        #[serde(default = "default_granular_delay_density_hz")]
        density_hz: f64,
        /// 颗粒变调。12 高八度闪光；-12 低沉
        #[serde(default = "default_granular_delay_pitch_semitones")]
        pitch_semitones: f64,
        /// 变调的随机范围
        #[serde(default = "default_granular_delay_pitch_spray")]
        pitch_spray: f64,
        /// 颗粒在左右的散开程度
        #[serde(default = "default_granular_delay_stereo_spread")]
        stereo_spread: f64,
        /// 反馈。越大云雾越厚越长
        #[serde(default = "default_granular_delay_feedback")]
        feedback: f64,
        /// 干湿比。在发送总线上用 1
        #[serde(default = "default_granular_delay_mix")]
        mix: f64,
    },
    /// 元音滤波：按人声元音的共振峰滤波，让合成器"说话"（a-e-i-o-u）。
    FormantFilter {
        /// 元音位置：0 a、1 e、2 i、3 o、4 u，中间平滑过渡
        #[serde(default = "default_formant_filter_vowel")]
        vowel: f64,
        /// 共振峰平移。+ 更小的嗓子；- 更大的嗓子
        #[serde(default = "default_formant_filter_shift")]
        shift: f64,
        /// 输出增益
        #[serde(default = "default_formant_filter_gain_db")]
        gain_db: f64,
        /// 干湿比
        #[serde(default = "default_formant_filter_mix")]
        mix: f64,
    },
}

// —— 各参数的默认值（选取音乐上常用、安全的起点）——
fn default_bitcrusher_bits() -> f64 {
    8.0
}
fn default_bitcrusher_downsample() -> f64 {
    1.0
}
fn default_bitcrusher_mix() -> f64 {
    1.0
}
fn default_tremolo_rate_hz() -> f64 {
    5.0
}
fn default_tremolo_depth() -> f64 {
    0.5
}
fn default_tremolo_stereo_phase() -> f64 {
    0.0
}
fn default_tremolo_shape() -> String {
    "sine".into()
}
fn default_auto_pan_rate_hz() -> f64 {
    0.5
}
fn default_auto_pan_width() -> f64 {
    1.0
}
fn default_auto_pan_shape() -> String {
    "sine".into()
}
fn default_ring_mod_frequency_hz() -> f64 {
    440.0
}
fn default_ring_mod_mix() -> f64 {
    0.5
}
fn default_frequency_shifter_shift_hz() -> f64 {
    10.0
}
fn default_frequency_shifter_feedback() -> f64 {
    0.0
}
fn default_frequency_shifter_mix() -> f64 {
    1.0
}
fn default_widener_width() -> f64 {
    1.5
}
fn default_widener_bass_mono_hz() -> f64 {
    120.0
}
fn default_transient_shaper_attack_db() -> f64 {
    0.0
}
fn default_transient_shaper_sustain_db() -> f64 {
    0.0
}
fn default_transient_shaper_attack_ms() -> f64 {
    20.0
}
fn default_transient_shaper_sustain_ms() -> f64 {
    400.0
}
fn default_multiband_compressor_low_hz() -> f64 {
    200.0
}
fn default_multiband_compressor_high_hz() -> f64 {
    2000.0
}
fn default_multiband_compressor_threshold_db() -> f64 {
    -20.0
}
fn default_multiband_compressor_ratio() -> f64 {
    3.0
}
fn default_multiband_compressor_attack_ms() -> f64 {
    10.0
}
fn default_multiband_compressor_release_ms() -> f64 {
    150.0
}
fn default_multiband_compressor_low_db() -> f64 {
    0.0
}
fn default_multiband_compressor_mid_db() -> f64 {
    0.0
}
fn default_multiband_compressor_high_db() -> f64 {
    0.0
}
fn default_tape_drive_db() -> f64 {
    6.0
}
fn default_tape_asymmetry() -> f64 {
    0.15
}
fn default_tape_wow_cents() -> f64 {
    3.0
}
fn default_tape_flutter_cents() -> f64 {
    1.0
}
fn default_tape_mix() -> f64 {
    1.0
}
fn default_vocoder_bands() -> f64 {
    16.0
}
fn default_vocoder_low_hz() -> f64 {
    100.0
}
fn default_vocoder_high_hz() -> f64 {
    8000.0
}
fn default_vocoder_formant_shift() -> f64 {
    0.0
}
fn default_vocoder_attack_ms() -> f64 {
    10.0
}
fn default_vocoder_release_ms() -> f64 {
    10.0
}
fn default_vocoder_gain_db() -> f64 {
    0.0
}
fn default_vocoder_mix() -> f64 {
    1.0
}
fn default_granular_delay_position_ms() -> f64 {
    250.0
}
fn default_granular_delay_spray_ms() -> f64 {
    50.0
}
fn default_granular_delay_grain_ms() -> f64 {
    60.0
}
fn default_granular_delay_density_hz() -> f64 {
    30.0
}
fn default_granular_delay_pitch_semitones() -> f64 {
    0.0
}
fn default_granular_delay_pitch_spray() -> f64 {
    0.0
}
fn default_granular_delay_stereo_spread() -> f64 {
    0.5
}
fn default_granular_delay_feedback() -> f64 {
    0.0
}
fn default_granular_delay_mix() -> f64 {
    0.5
}
fn default_formant_filter_vowel() -> f64 {
    0.0
}
fn default_formant_filter_shift() -> f64 {
    0.0
}
fn default_formant_filter_gain_db() -> f64 {
    0.0
}
fn default_formant_filter_mix() -> f64 {
    1.0
}
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
        name: "bitcrusher",
        summary: "降位降采样：变粗、变脏的数码失真（8-bit、老采样器、lo-fi 质感）。",
        placement: "适合鼓、lead、整条 lo-fi 总线；bits 8-12 是老采样器，4-6 很脏。放在混响之前。",
        params: &[
            p("bits", 1.0, 24.0, 8.0, "bit", "位深。12 轻微颗粒；8 老游戏机；4 以下非常脏"),
            p("downsample", 1.0, 64.0, 1.0, "×", "降采样倍数（保持每 N 个样本）。1 不降；4-8 明显的金属锯齿感"),
            p("mix", 0.0, 1.0, 1.0, "", "干湿比。0.3-0.5 只加一点颗粒"),
        ],
        choices: &[],
        example: r#"{ "type": "bitcrusher", "bits": 10, "downsample": 2, "mix": 0.6 }"#,
    },
    EffectSpec {
        name: "tremolo",
        summary: "颤音（音量）：按 LFO 周期性地起伏音量。",
        placement: "适合电钢、吉他、pad。按节拍：rate_hz = tempo / 60 / 拍数（每拍一次 = tempo/60）。",
        params: &[
            p("rate_hz", 0.0, 40.0, 5.0, "Hz", "起伏速度。3-6 经典电钢颤音；按节拍取值更合拍"),
            p("depth", 0.0, 1.0, 0.5, "", "深度。0.3 轻微；1 起伏到无声"),
            p("stereo_phase", 0.0, 1.0, 0.0, "", "左右声道的相位差（周）。0.5 左右交替，带一点自动声像感"),
        ],
        choices: &[ChoiceSpec { name: "shape", choices: &["sine", "triangle", "square", "saw_up", "saw_down"], default: "sine", description: "LFO 波形：sine 柔和；triangle；square 斩波（chopper）效果；saw_up / saw_down 锯齿" }],
        example: r#"{ "type": "tremolo", "rate_hz": 4, "depth": 0.6 }"#,
    },
    EffectSpec {
        name: "auto_pan",
        summary: "自动声像：按 LFO 让声音在左右之间来回移动。",
        placement: "适合 pad、琶音、打击乐点缀；不要用在贝斯和底鼓上。按节拍：rate_hz = tempo / 60 / 拍数。",
        params: &[
            p("rate_hz", 0.0, 40.0, 0.5, "Hz", "移动速度。0.1-0.5 缓慢漂移；2 以上快速晃动"),
            p("width", 0.0, 1.0, 1.0, "", "摆动幅度。1 从最左到最右；0.3 只在中间附近晃"),
        ],
        choices: &[ChoiceSpec { name: "shape", choices: &["sine", "triangle", "square", "saw_up", "saw_down"], default: "sine", description: "LFO 波形：sine 平滑；square 左右跳" }],
        example: r#"{ "type": "auto_pan", "rate_hz": 0.25, "width": 0.8 }"#,
    },
    EffectSpec {
        name: "ring_mod",
        summary: "环形调制：和一个正弦相乘，产生不和谐的金属、钟、机器人音色。",
        placement: "适合合成器、鼓的特效段落、科幻音效。频率低（<30 Hz）时是颤动，高时是金属感。",
        params: &[
            p("frequency_hz", 0.0, 5000.0, 440.0, "Hz", "载波频率。20-30 颤动；100-1000 金属/钟声"),
            p("mix", 0.0, 1.0, 0.5, "", "干湿比。1 只剩调制后的声音"),
        ],
        choices: &[],
        example: r#"{ "type": "ring_mod", "frequency_hz": 300, "mix": 0.4 }"#,
    },
    EffectSpec {
        name: "frequency_shifter",
        summary: "移频：把所有频率平移固定的赫兹数（不同于变调，泛音关系被打破）。",
        placement: "小移频（1-10 Hz）做缓慢的相位旋转、加宽；大移频做不和谐的特效；加反馈做螺旋上升/下降的音。",
        params: &[
            p("shift_hz", -2000.0, 2000.0, 10.0, "Hz", "平移量。±1-10 细微旋转；±50 以上明显失谐"),
            p("feedback", 0.0, 0.95, 0.0, "", "反馈。越大越像无尽的上升/下降螺旋（巴伯音）"),
            p("mix", 0.0, 1.0, 1.0, "", "干湿比。0.5 时小移频有相位器般的效果"),
        ],
        choices: &[],
        example: r#"{ "type": "frequency_shifter", "shift_hz": 4, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "widener",
        summary: "立体声扩展：加大左右差异，让声音更宽；低频可保持单声道。",
        placement: "适合 pad、和弦、混响总线、主总线（轻用）。贝斯、底鼓、主唱保持居中。",
        params: &[
            p("width", 0.0, 2.0, 1.5, "", "宽度。1 不变；1.2-1.6 常用；0 变单声道"),
            p("bass_mono_hz", 0.0, 1000.0, 120.0, "Hz", "低于这个频率保持单声道（低频不散）。0 关闭"),
        ],
        choices: &[],
        example: r#"{ "type": "widener", "width": 1.4, "bass_mono_hz": 150 }"#,
    },
    EffectSpec {
        name: "transient_shaper",
        summary: "瞬态塑形：单独增强/减弱起音和延音，不管音量多大。",
        placement: "鼓最常用：attack_db +4 到 +8 让鼓更脆更冲；sustain_db -6 收紧余音（去房间声）。放在压缩器之前。",
        params: &[
            p("attack_db", -24.0, 24.0, 0.0, "dB", "起音增益。+ 更冲、更脆；- 更软、更远"),
            p("sustain_db", -24.0, 24.0, 0.0, "dB", "延音增益。+ 更多尾音和房间感；- 更干更紧"),
            p("attack_ms", 2.0, 200.0, 20.0, "ms", "起音段的长度。鼓 5-20"),
            p("sustain_ms", 60.0, 3000.0, 400.0, "ms", "延音段的反应时间"),
        ],
        choices: &[],
        example: r#"{ "type": "transient_shaper", "attack_db": 6, "sustain_db": -4 }"#,
    },
    EffectSpec {
        name: "multiband_compressor",
        summary: "三段压缩：分成低、中、高三段各自压缩，低频的重音不会压到高频。",
        placement: "主总线做母带、鼓总线、贝斯；low_hz / high_hz 是分频点。low_db / mid_db / high_db 是每段的补偿增益，也能当粗调 EQ。放在 limiter 之前。",
        params: &[
            p("low_hz", 20.0, 2000.0, 200.0, "Hz", "低频段与中频段的分频点"),
            p("high_hz", 200.0, 16000.0, 2000.0, "Hz", "中频段与高频段的分频点（要高于 low_hz）"),
            p("threshold_db", -60.0, 0.0, -20.0, "dB", "三段共用的阈值"),
            p("ratio", 1.0, 100.0, 3.0, ":1", "三段共用的压缩比。2-3 母带；4+ 更紧"),
            p("attack_ms", 0.01, 1000.0, 10.0, "ms", "启动时间"),
            p("release_ms", 1.0, 5000.0, 150.0, "ms", "释放时间"),
            p("low_db", -24.0, 24.0, 0.0, "dB", "低频段补偿增益"),
            p("mid_db", -24.0, 24.0, 0.0, "dB", "中频段补偿增益"),
            p("high_db", -24.0, 24.0, 0.0, "dB", "高频段补偿增益"),
        ],
        choices: &[],
        example: r#"{ "type": "multiband_compressor", "threshold_db": -18, "ratio": 2.5, "high_db": 1 }"#,
    },
    EffectSpec {
        name: "tape",
        summary: "磁带：磁带饱和的温暖失真，加上走带不稳的音高晃动（wow / flutter）。",
        placement: "lo-fi、复古、温暖：整条总线或主总线轻用（drive 3-6），单轨可以更重。wow_cents 5-15 是老卡带的跑调感。",
        params: &[
            p("drive_db", 0.0, 30.0, 6.0, "dB", "驱动。3-6 温暖；12 以上明显失真"),
            p("asymmetry", 0.0, 0.5, 0.15, "", "不对称，加偶次谐波（更暖）"),
            p("wow_cents", 0.0, 30.0, 3.0, "cent", "慢速音高晃动的深度。0 关；5-15 老卡带"),
            p("flutter_cents", 0.0, 10.0, 1.0, "cent", "快速音高抖动的深度"),
            p("mix", 0.0, 1.0, 1.0, "", "干湿比"),
        ],
        choices: &[],
        example: r#"{ "type": "tape", "drive_db": 5, "wow_cents": 8 }"#,
    },
    EffectSpec {
        name: "vocoder",
        summary: "声码器：用另一条轨（多是人声）的频谱去塑造这条轨（多是合成器和弦）——机器人唱歌。",
        placement: "挂在合成器/和弦轨（载波）上，\"sidechain\" 写人声那条轨或总线的名字（调制信号）。载波要持续、泛音丰富（锯齿 pad、string machine）。",
        params: &[
            p("bands", 8.0, 32.0, 16.0, "", "频带数。8 粗糙复古；16 常规；32 更清楚"),
            p("low_hz", 40.0, 16000.0, 100.0, "Hz", "最低频带"),
            p("high_hz", 40.0, 16000.0, 8000.0, "Hz", "最高频带"),
            p("formant_shift", -12.0, 12.0, 0.0, "半音", "共振峰平移。+ 更尖（小孩、卡通）；- 更粗"),
            p("attack_ms", 0.1, 500.0, 10.0, "ms", "包络跟随的启动时间"),
            p("release_ms", 0.1, 2000.0, 10.0, "ms", "包络跟随的释放时间。长一点更连贯"),
            p("gain_db", -24.0, 24.0, 0.0, "dB", "输出增益"),
            p("mix", 0.0, 1.0, 1.0, "", "干湿比"),
        ],
        choices: &[],
        example: r#"{ "type": "vocoder", "sidechain": "voice", "bands": 16 }"#,
    },
    EffectSpec {
        name: "granular_delay",
        summary: "颗粒延迟：把延迟线切成小颗粒随机重放，能变调、散开，做出云雾、闪烁的氛围。",
        placement: "适合 pad、钢琴、人声的氛围段，常放在发送总线上。pitch_semitones 12 做\"闪光\"（shimmer）。",
        params: &[
            p("position_ms", 0.0, 1000.0, 250.0, "ms", "颗粒从多久之前取"),
            p("spray_ms", 0.0, 500.0, 50.0, "ms", "取样位置的随机范围"),
            p("grain_ms", 5.0, 500.0, 60.0, "ms", "颗粒长度。短（10-30）颗粒感强；长（100+）更平滑"),
            p("density_hz", 0.5, 200.0, 30.0, "Hz", "每秒多少颗粒"),
            p("pitch_semitones", -24.0, 24.0, 0.0, "半音", "颗粒变调。12 高八度闪光；-12 低沉"),
            p("pitch_spray", 0.0, 24.0, 0.0, "半音", "变调的随机范围"),
            p("stereo_spread", 0.0, 1.0, 0.5, "", "颗粒在左右的散开程度"),
            p("feedback", 0.0, 0.95, 0.0, "", "反馈。越大云雾越厚越长"),
            p("mix", 0.0, 1.0, 0.5, "", "干湿比。在发送总线上用 1"),
        ],
        choices: &[],
        example: r#"{ "type": "granular_delay", "pitch_semitones": 12, "feedback": 0.4, "mix": 0.4 }"#,
    },
    EffectSpec {
        name: "formant_filter",
        summary: "元音滤波：按人声元音的共振峰滤波，让合成器\"说话\"（a-e-i-o-u）。",
        placement: "适合合成器 lead、贝斯、pad。vowel 用自动化来回移动就是\"哇哇\"的说话感。",
        params: &[
            p("vowel", 0.0, 4.0, 0.0, "", "元音位置：0 a、1 e、2 i、3 o、4 u，中间平滑过渡"),
            p("shift", -12.0, 12.0, 0.0, "半音", "共振峰平移。+ 更小的嗓子；- 更大的嗓子"),
            p("gain_db", -24.0, 24.0, 0.0, "dB", "输出增益"),
            p("mix", 0.0, 1.0, 1.0, "", "干湿比"),
        ],
        choices: &[],
        example: r#"{ "type": "formant_filter", "vowel": 1.5, "mix": 0.8 }"#,
    },
    EffectSpec {
        name: "plugin",
        summary: "第三方插件效果器（VST3/CLAP/LV2）。",
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
            Effect::Bitcrusher { .. } => "bitcrusher",
            Effect::Tremolo { .. } => "tremolo",
            Effect::AutoPan { .. } => "auto_pan",
            Effect::RingMod { .. } => "ring_mod",
            Effect::FrequencyShifter { .. } => "frequency_shifter",
            Effect::Widener { .. } => "widener",
            Effect::TransientShaper { .. } => "transient_shaper",
            Effect::MultibandCompressor { .. } => "multiband_compressor",
            Effect::Tape { .. } => "tape",
            Effect::Vocoder { .. } => "vocoder",
            Effect::GranularDelay { .. } => "granular_delay",
            Effect::FormantFilter { .. } => "formant_filter",
        }
    }

    /// 侧链（触发 / 调制信号）的音轨或总线名：压缩器的 `sidechain`、声码器的调制信号。
    pub fn sidechain(&self) -> Option<&str> {
        match self {
            Effect::Compressor { sidechain, .. } | Effect::Vocoder { sidechain, .. } => sidechain.as_deref(),
            _ => None,
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
            Effect::Bitcrusher { bits, downsample, mix, .. } => vec![("bits", bits), ("downsample", downsample), ("mix", mix)],
            Effect::Tremolo { rate_hz, depth, stereo_phase, .. } => vec![("rate_hz", rate_hz), ("depth", depth), ("stereo_phase", stereo_phase)],
            Effect::AutoPan { rate_hz, width, .. } => vec![("rate_hz", rate_hz), ("width", width)],
            Effect::RingMod { frequency_hz, mix, .. } => vec![("frequency_hz", frequency_hz), ("mix", mix)],
            Effect::FrequencyShifter { shift_hz, feedback, mix, .. } => vec![("shift_hz", shift_hz), ("feedback", feedback), ("mix", mix)],
            Effect::Widener { width, bass_mono_hz, .. } => vec![("width", width), ("bass_mono_hz", bass_mono_hz)],
            Effect::TransientShaper { attack_db, sustain_db, attack_ms, sustain_ms, .. } => vec![("attack_db", attack_db), ("sustain_db", sustain_db), ("attack_ms", attack_ms), ("sustain_ms", sustain_ms)],
            Effect::MultibandCompressor { low_hz, high_hz, threshold_db, ratio, attack_ms, release_ms, low_db, mid_db, high_db, .. } => vec![("low_hz", low_hz), ("high_hz", high_hz), ("threshold_db", threshold_db), ("ratio", ratio), ("attack_ms", attack_ms), ("release_ms", release_ms), ("low_db", low_db), ("mid_db", mid_db), ("high_db", high_db)],
            Effect::Tape { drive_db, asymmetry, wow_cents, flutter_cents, mix, .. } => vec![("drive_db", drive_db), ("asymmetry", asymmetry), ("wow_cents", wow_cents), ("flutter_cents", flutter_cents), ("mix", mix)],
            Effect::Vocoder { bands, low_hz, high_hz, formant_shift, attack_ms, release_ms, gain_db, mix, .. } => vec![("bands", bands), ("low_hz", low_hz), ("high_hz", high_hz), ("formant_shift", formant_shift), ("attack_ms", attack_ms), ("release_ms", release_ms), ("gain_db", gain_db), ("mix", mix)],
            Effect::GranularDelay { position_ms, spray_ms, grain_ms, density_hz, pitch_semitones, pitch_spray, stereo_spread, feedback, mix, .. } => vec![("position_ms", position_ms), ("spray_ms", spray_ms), ("grain_ms", grain_ms), ("density_hz", density_hz), ("pitch_semitones", pitch_semitones), ("pitch_spray", pitch_spray), ("stereo_spread", stereo_spread), ("feedback", feedback), ("mix", mix)],
            Effect::FormantFilter { vowel, shift, gain_db, mix, .. } => vec![("vowel", vowel), ("shift", shift), ("gain_db", gain_db), ("mix", mix)],
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
            Effect::Tremolo { shape, .. } | Effect::AutoPan { shape, .. } => {
                let cs = &spec.choices[0];
                if !cs.choices.contains(&shape.trim().to_ascii_lowercase().as_str()) {
                    return Err(format!("{name}.shape = '{shape}' 无效，可选: {}", cs.choices.join(" / ")));
                }
            }
            Effect::Vocoder { sidechain: None, .. } => {
                return Err("vocoder 需要 \"sidechain\"：写调制信号（多是人声）那条音轨或总线的名字".to_string());
            }
            Effect::Compressor { .. } | Effect::Vocoder { .. } if self.sidechain().is_some_and(|s| s.trim().is_empty()) => {
                return Err(format!("{name}.sidechain 不能为空字符串"));
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
            // 声码器必须写调制信号（sidechain），最小 JSON 校验不过；示例里有。
            if spec.name != "vocoder" {
                e.validate().unwrap();
            }
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
