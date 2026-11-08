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
            return Err("from / until must be beat positions >= 0".into());
        }
        if let (Some(a), Some(z)) = (self.from, self.until) {
            if z.0 <= a.0 {
                return Err(format!("until ({}) must be later than from ({})", z.0, a.0));
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
    /// 失真：过载、失真、fuzz、折叠，带音色滤波和箱体模拟，响度自动补偿。
    Distortion {
        /// 类型：`overdrive` `distortion` `fuzz` `fold`
        #[serde(default = "default_distortion_mode")]
        mode: String,
        /// 驱动（dB）
        #[serde(default = "default_distortion_drive_db")]
        drive_db: f64,
        /// 失真后的低通（Hz）：越低越暗
        #[serde(default = "default_distortion_tone_hz")]
        tone_hz: f64,
        /// 失真前的高通（Hz）：去掉低频，失真不糊
        #[serde(default = "default_distortion_low_cut_hz")]
        low_cut_hz: f64,
        /// 箱体模拟：`none` `guitar` `bass`
        #[serde(default = "default_distortion_cabinet")]
        cabinet: String,
        /// 输出（dB），在自动响度补偿之后
        #[serde(default)]
        output_db: f64,
        /// 干湿比
        #[serde(default = "default_one")]
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
fn default_distortion_mode() -> String {
    "distortion".into()
}
fn default_distortion_drive_db() -> f64 {
    18.0
}
fn default_distortion_tone_hz() -> f64 {
    8000.0
}
fn default_distortion_low_cut_hz() -> f64 {
    100.0
}
fn default_distortion_cabinet() -> String {
    "none".into()
}
fn default_one() -> f64 {
    1.0
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
        summary: "Reverb: simulates the space and tail of a room or hall.",
        placement: "Put it at the end of the chain on melody, pad and vocal tracks; use little or none on bass and kick, or the low end gets muddy.",
        params: &[
            p("mix", 0.0, 1.0, 0.25, "", "Dry/wet mix. 0.1-0.2 slight space; 0.25-0.35 normal; >0.5 very wet, sound moves far away"),
            p("decay_seconds", 0.05, 120.0, 1.6, "s", "Tail decay time. 0.3-0.8 small room; 1.2-2.5 hall; >4 ambient"),
            p("damping", 0.0, 1.0, 0.5, "", "High-frequency damping. Higher = darker, warmer tail; lower = brighter, more metallic"),
            p("size", 0.0, 1.0, 0.5, "", "Space size. 0.2 small room; 0.5 medium; 0.8+ hall/cathedral"),
        ],
        choices: &[],
        example: r#"{ "type": "reverb", "mix": 0.3, "decay_seconds": 2.2, "size": 0.7 }"#,
    },
    EffectSpec {
        name: "delay",
        summary: "Delay/echo: repeats the signal after a set time.",
        placement: "Good for lead, chords and vocals; put it before the reverb. Setting time from the tempo keeps it in sync: 60/tempo seconds = one beat.",
        params: &[
            p("time_seconds", 0.0, 10.0, 0.3, "s", "Echo interval. Set it from the tempo: quarter note = 60/tempo, eighth = 30/tempo, dotted eighth = 45/tempo"),
            p("feedback", 0.0, 0.99, 0.35, "", "Echo repeats. 0.2 one or two; 0.4-0.5 many; >0.7 builds up for a long time"),
            p("mix", 0.0, 1.0, 0.3, "", "Dry/wet mix. 0.2-0.35 normal; >0.5 echoes louder than the original"),
        ],
        choices: &[],
        example: r#"{ "type": "delay", "time_seconds": 0.35, "feedback": 0.4, "mix": 0.3 }"#,
    },
    EffectSpec {
        name: "compressor",
        summary: "Compressor: turns down peaks above the threshold for steadier, tighter dynamics.",
        placement: "On a single track, put it early in the chain (after EQ) to control dynamics; on the master bus use it as \"glue\", before the limiter. With \"sidechain\": \"<track or bus name>\" it is triggered by that signal instead: kick ducking bass/pad for a pumping feel (typical threshold -30, ratio 6-10, attack 1-5, release 100-200).",
        params: &[
            p("threshold_db", -60.0, 0.0, -18.0, "dB", "Threshold. Lower = more compression. Light -12; normal -18; heavy -30"),
            p("ratio", 1.0, 100.0, 4.0, ":1", "Ratio. 2 gentle; 4 normal; 8+ close to limiting"),
            p("attack_ms", 0.01, 1000.0, 10.0, "ms", "Attack time. Short (1-5) clamps transients; long (20-50) keeps the punch of drums"),
            p("release_ms", 1.0, 5000.0, 120.0, "ms", "Release time. Short (50) follows tightly and pumps; long (200-500) sounds more natural"),
            p("makeup_db", -24.0, 36.0, 0.0, "dB", "Makeup gain, to recover the level lost to compression. Usually 0-6"),
        ],
        choices: &[],
        example: r#"{ "type": "compressor", "threshold_db": -16, "ratio": 3, "makeup_db": 2 }"#,
    },
    EffectSpec {
        name: "chorus",
        summary: "Chorus: layers slightly detuned copies to thicken and widen the sound.",
        placement: "Good for pad, electric piano, guitar and chords; put it before the reverb. Use with care on bass (low-frequency phase cancellation).",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.8, "Hz", "Modulation rate. 0.2-1 slow and natural; 2-5 obvious wobble"),
            p("depth_ms", 0.0, 50.0, 3.0, "ms", "Modulation depth. 1-3 subtle; 5-10 obviously detuned"),
            p("mix", 0.0, 1.0, 0.4, "", "Dry/wet mix. 0.3-0.5 normal"),
        ],
        choices: &[],
        example: r#"{ "type": "chorus", "rate_hz": 0.6, "depth_ms": 4, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "eq",
        summary: "EQ/filter: boosts, cuts or removes one frequency band. One eq handles one band; chain several for more.",
        placement: "Usually first in the chain. Common uses: highpass to cut low-end mud on non-bass tracks; high_shelf to brighten; peak to fix a single frequency.",
        params: &[
            p("frequency", 10.0, 24000.0, 1000.0, "Hz", "Target frequency. Low 60-250; mid 250-4000; high 4000-16000"),
            p("q", 0.05, 40.0, 0.707, "", "Bandwidth/resonance. 0.707 gentle; 1-2 medium; >5 very narrow (for fixing spikes)"),
            p("gain_db", -36.0, 36.0, 0.0, "dB", "Gain, only used by peak / low_shelf / high_shelf. Typically +/-2 to +/-6"),
        ],
        choices: &[ChoiceSpec {
            name: "shape",
            choices: &["lowpass", "highpass", "bandpass", "notch", "allpass", "peak", "low_shelf", "high_shelf"],
            default: "peak",
            description: "Filter shape. lowpass keeps what is below frequency (muffled); highpass keeps what is above (removes lows); peak boosts or cuts at frequency; low_shelf/high_shelf boost or cut all lows/highs",
        }],
        example: r#"{ "type": "eq", "shape": "high_shelf", "frequency": 8000, "gain_db": 3 }"#,
    },
    EffectSpec {
        name: "limiter",
        summary: "Limiter: keeps output peaks below the ceiling to prevent clipping.",
        placement: "Only as the last effect on the master bus (master_effects).",
        params: &[
            p("ceiling_db", -24.0, 0.0, -0.3, "dB", "Output ceiling. -0.3 to -1 is typical"),
            p("release_ms", 1.0, 2000.0, 50.0, "ms", "Release time. Short is louder but may distort; long is smoother"),
        ],
        choices: &[],
        example: r#"{ "type": "limiter", "ceiling_db": -0.5 }"#,
    },
    EffectSpec {
        name: "phaser",
        summary: "Phaser: sweeping notches that give a flowing, \"jet\" texture.",
        placement: "Good for pad, electric piano, guitar and synth lead; put it before the reverb.",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.5, "Hz", "Sweep rate. 0.1-0.5 slow; 1-3 obvious"),
            p("feedback", -0.95, 0.95, 0.3, "", "Resonance. A larger absolute value is sharper; negative values sound different"),
            p("mix", 0.0, 1.0, 0.5, "", "Dry/wet mix. 0.5 gives the strongest effect"),
        ],
        choices: &[],
        example: r#"{ "type": "phaser", "rate_hz": 0.3, "feedback": 0.5, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "flanger",
        summary: "Flanger: modulates a very short delay for a metallic sweep (jet sound).",
        placement: "Good as an accent on synths, guitar and drums; put it before the reverb, and usually not for a whole piece.",
        params: &[
            p("rate_hz", 0.0, 100.0, 0.2, "Hz", "Sweep rate. 0.1-0.3 slow sweep; >1 fast wobble"),
            p("feedback", -0.95, 0.95, 0.7, "", "Resonance. Higher = more metallic"),
            p("mix", 0.0, 1.0, 0.5, "", "Dry/wet mix. 0.5 gives the strongest effect"),
        ],
        choices: &[],
        example: r#"{ "type": "flanger", "rate_hz": 0.2, "feedback": 0.7, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "bitcrusher",
        summary: "Bitcrusher: coarse, dirty digital distortion (8-bit, old samplers, lo-fi texture).",
        placement: "Good for drums, lead and a whole lo-fi bus; bits 8-12 is an old sampler, 4-6 is very dirty. Put it before the reverb.",
        params: &[
            p("bits", 1.0, 24.0, 8.0, "bit", "Bit depth. 12 slightly grainy; 8 old game console; below 4 very dirty"),
            p("downsample", 1.0, 64.0, 1.0, "×", "Downsample factor (hold every Nth sample). 1 = none; 4-8 gives an obvious metallic, jagged sound"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix. 0.3-0.5 adds just a little grain"),
        ],
        choices: &[],
        example: r#"{ "type": "bitcrusher", "bits": 10, "downsample": 2, "mix": 0.6 }"#,
    },
    EffectSpec {
        name: "tremolo",
        summary: "Tremolo (volume): periodically raises and lowers the volume with an LFO.",
        placement: "Good for electric piano, guitar and pad. To sync to the beat: rate_hz = tempo / 60 / beats (once per beat = tempo/60).",
        params: &[
            p("rate_hz", 0.0, 40.0, 5.0, "Hz", "Modulation rate. 3-6 is classic electric-piano tremolo; setting it from the tempo keeps it in sync"),
            p("depth", 0.0, 1.0, 0.5, "", "Depth. 0.3 slight; 1 swells down to silence"),
            p("stereo_phase", 0.0, 1.0, 0.0, "", "Phase offset between left and right channels (cycles). 0.5 alternates left and right, like a little auto-pan"),
        ],
        choices: &[ChoiceSpec { name: "shape", choices: &["sine", "triangle", "square", "saw_up", "saw_down"], default: "sine", description: "LFO waveform: sine is soft; triangle; square gives a chopper effect; saw_up / saw_down are sawtooth" }],
        example: r#"{ "type": "tremolo", "rate_hz": 4, "depth": 0.6 }"#,
    },
    EffectSpec {
        name: "auto_pan",
        summary: "Auto-pan: moves the sound back and forth between left and right with an LFO.",
        placement: "Good for pad, arpeggios and percussion accents; do not use on bass or kick. To sync to the beat: rate_hz = tempo / 60 / beats.",
        params: &[
            p("rate_hz", 0.0, 40.0, 0.5, "Hz", "Movement rate. 0.1-0.5 slow drift; above 2 fast swaying"),
            p("width", 0.0, 1.0, 1.0, "", "Swing width. 1 goes from full left to full right; 0.3 stays near the center"),
        ],
        choices: &[ChoiceSpec { name: "shape", choices: &["sine", "triangle", "square", "saw_up", "saw_down"], default: "sine", description: "LFO waveform: sine is smooth; square jumps between left and right" }],
        example: r#"{ "type": "auto_pan", "rate_hz": 0.25, "width": 0.8 }"#,
    },
    EffectSpec {
        name: "ring_mod",
        summary: "Ring modulator: multiplies with a sine wave for inharmonic metallic, bell and robot sounds.",
        placement: "Good for synths, drum effect sections and sci-fi sounds. At low frequencies (<30 Hz) it wobbles; at high ones it sounds metallic.",
        params: &[
            p("frequency_hz", 0.0, 5000.0, 440.0, "Hz", "Carrier frequency. 20-30 wobble; 100-1000 metallic/bell"),
            p("mix", 0.0, 1.0, 0.5, "", "Dry/wet mix. 1 leaves only the modulated sound"),
        ],
        choices: &[],
        example: r#"{ "type": "ring_mod", "frequency_hz": 300, "mix": 0.4 }"#,
    },
    EffectSpec {
        name: "frequency_shifter",
        summary: "Frequency shifter: shifts every frequency by a fixed number of Hz (unlike pitch shifting, the harmonic relationships are broken).",
        placement: "Small shifts (1-10 Hz) give slow phase rotation and widening; large shifts give inharmonic effects; add feedback for rising/falling spiral tones.",
        params: &[
            p("shift_hz", -2000.0, 2000.0, 10.0, "Hz", "Shift amount. +/-1-10 subtle rotation; above +/-50 obviously detuned"),
            p("feedback", 0.0, 0.95, 0.0, "", "Feedback. Higher sounds more like an endless rising/falling spiral (Shepard tone)"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix. At 0.5 a small shift sounds like a phaser"),
        ],
        choices: &[],
        example: r#"{ "type": "frequency_shifter", "shift_hz": 4, "mix": 0.5 }"#,
    },
    EffectSpec {
        name: "widener",
        summary: "Stereo widener: increases the left/right difference for a wider sound; lows can stay mono.",
        placement: "Good for pad, chords, reverb bus and master bus (lightly). Keep bass, kick and lead vocal centered.",
        params: &[
            p("width", 0.0, 2.0, 1.5, "", "Width. 1 unchanged; 1.2-1.6 typical; 0 becomes mono"),
            p("bass_mono_hz", 0.0, 1000.0, 120.0, "Hz", "Frequencies below this stay mono (lows don't spread). 0 = off"),
        ],
        choices: &[],
        example: r#"{ "type": "widener", "width": 1.4, "bass_mono_hz": 150 }"#,
    },
    EffectSpec {
        name: "transient_shaper",
        summary: "Transient shaper: boosts or reduces the attack and sustain separately, regardless of level.",
        placement: "Most used on drums: attack_db +4 to +8 makes drums snappier and punchier; sustain_db -6 tightens the tail (removes room sound). Put it before the compressor.",
        params: &[
            p("attack_db", -24.0, 24.0, 0.0, "dB", "Attack gain. + punchier, snappier; - softer, more distant"),
            p("sustain_db", -24.0, 24.0, 0.0, "dB", "Sustain gain. + more tail and room; - drier and tighter"),
            p("attack_ms", 2.0, 200.0, 20.0, "ms", "Length of the attack stage. Drums 5-20"),
            p("sustain_ms", 60.0, 3000.0, 400.0, "ms", "Reaction time of the sustain stage"),
        ],
        choices: &[],
        example: r#"{ "type": "transient_shaper", "attack_db": 6, "sustain_db": -4 }"#,
    },
    EffectSpec {
        name: "multiband_compressor",
        summary: "Multiband compressor: splits into low, mid and high bands compressed separately, so heavy lows don't pull down the highs.",
        placement: "For mastering on the master bus, drum bus and bass; low_hz / high_hz are the crossover points. low_db / mid_db / high_db are the per-band makeup gains and can also work as a coarse EQ. Put it before the limiter.",
        params: &[
            p("low_hz", 20.0, 2000.0, 200.0, "Hz", "Crossover point between the low and mid bands"),
            p("high_hz", 200.0, 16000.0, 2000.0, "Hz", "Crossover point between the mid and high bands (must be above low_hz)"),
            p("threshold_db", -60.0, 0.0, -20.0, "dB", "Threshold shared by all three bands"),
            p("ratio", 1.0, 100.0, 3.0, ":1", "Ratio shared by all three bands. 2-3 mastering; 4+ tighter"),
            p("attack_ms", 0.01, 1000.0, 10.0, "ms", "Attack time"),
            p("release_ms", 1.0, 5000.0, 150.0, "ms", "Release time"),
            p("low_db", -24.0, 24.0, 0.0, "dB", "Makeup gain for the low band"),
            p("mid_db", -24.0, 24.0, 0.0, "dB", "Makeup gain for the mid band"),
            p("high_db", -24.0, 24.0, 0.0, "dB", "Makeup gain for the high band"),
        ],
        choices: &[],
        example: r#"{ "type": "multiband_compressor", "threshold_db": -18, "ratio": 2.5, "high_db": 1 }"#,
    },
    EffectSpec {
        name: "tape",
        summary: "Tape: warm tape-saturation distortion plus unsteady transport pitch wobble (wow / flutter).",
        placement: "For lo-fi, vintage and warmth: use lightly on a whole bus or the master bus (drive 3-6), heavier on a single track. wow_cents 5-15 gives the off-pitch feel of an old cassette.",
        params: &[
            p("drive_db", 0.0, 30.0, 6.0, "dB", "Drive. 3-6 warm; above 12 obviously distorted"),
            p("asymmetry", 0.0, 0.5, 0.15, "", "Asymmetry, adds even harmonics (warmer)"),
            p("wow_cents", 0.0, 30.0, 3.0, "cent", "Depth of the slow pitch wobble. 0 = off; 5-15 old cassette"),
            p("flutter_cents", 0.0, 10.0, 1.0, "cent", "Depth of the fast pitch flutter"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix"),
        ],
        choices: &[],
        example: r#"{ "type": "tape", "drive_db": 5, "wow_cents": 8 }"#,
    },
    EffectSpec {
        name: "distortion",
        summary: "Distortion modelled on a guitar amp (mid push before two clipping stages, cabinet with presence peak): overdrive, distortion, fuzz or wavefolding; loudness is compensated automatically, so more drive means more grit, not more volume.",
        placement: "On a guitar, bass, organ or synth track for rock / metal / grunge; lightly (overdrive, drive 6-12) on drums or a bus for grit. For a rock guitar: guitar.electric + { \"mode\": \"distortion\", \"drive_db\": 24, \"cabinet\": \"guitar\" }; play power chords (root + fifth), not full triads, since distortion turns thirds muddy.",
        params: &[
            p("drive_db", 0.0, 48.0, 18.0, "dB", "Drive. 6-12 crunch; 18-30 rock / metal; 36+ fuzz-like saturation"),
            p("tone_hz", 500.0, 12000.0, 8000.0, "Hz", "Low-pass after the distortion: lower is darker, higher adds fizz"),
            p("low_cut_hz", 20.0, 1000.0, 100.0, "Hz", "High-pass before the distortion so the lows don't turn to mud; 100-200 for tight metal rhythm, 40 for bass"),
            p("output_db", -24.0, 12.0, 0.0, "dB", "Output level after the automatic loudness compensation"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix; around 0.5 on bass keeps the clean low end under the grit"),
        ],
        choices: &[
            ChoiceSpec { name: "mode", choices: &["overdrive", "distortion", "fuzz", "fold"], default: "distortion", description: "overdrive: soft, asymmetric tube-like clipping; distortion: smooth saturation, rock / metal; fuzz: hard clipping, buzzy; fold: wavefolding, metallic synth sounds" },
            ChoiceSpec { name: "cabinet", choices: &["none", "guitar", "bass"], default: "none", description: "Speaker cabinet simulation after the distortion: guitar: a low bump, a scooped low-mid, a presence peak near 3 kHz and a steep roll-off above ~6.5 kHz; bass keeps the lows and rolls off above ~3 kHz" },
        ],
        example: r#"{ "type": "distortion", "mode": "distortion", "drive_db": 24, "cabinet": "guitar" }"#,
    },
    EffectSpec {
        name: "vocoder",
        summary: "Vocoder: shapes this track (usually synth chords) with the spectrum of another track (usually a vocal) - a singing robot.",
        placement: "Put it on the synth/chord track (the carrier) and set \"sidechain\" to the name of the vocal track or bus (the modulator). The carrier should be sustained and harmonically rich (saw pad, string machine).",
        params: &[
            p("bands", 8.0, 32.0, 16.0, "", "Number of bands. 8 rough and retro; 16 normal; 32 clearer"),
            p("low_hz", 40.0, 16000.0, 100.0, "Hz", "Lowest band"),
            p("high_hz", 40.0, 16000.0, 8000.0, "Hz", "Highest band"),
            p("formant_shift", -12.0, 12.0, 0.0, "semitones", "Formant shift. + higher (child, cartoon); - deeper"),
            p("attack_ms", 0.1, 500.0, 10.0, "ms", "Attack time of the envelope follower"),
            p("release_ms", 0.1, 2000.0, 10.0, "ms", "Release time of the envelope follower. Longer is smoother"),
            p("gain_db", -24.0, 24.0, 0.0, "dB", "Output gain"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix"),
        ],
        choices: &[],
        example: r#"{ "type": "vocoder", "sidechain": "voice", "bands": 16 }"#,
    },
    EffectSpec {
        name: "granular_delay",
        summary: "Granular delay: chops the delay line into small grains replayed randomly, which can shift pitch and scatter them for cloudy, shimmering ambience.",
        placement: "Good for ambient sections of pad, piano and vocals, often on a send bus. pitch_semitones 12 gives a \"shimmer\".",
        params: &[
            p("position_ms", 0.0, 1000.0, 250.0, "ms", "How far back the grains are taken from"),
            p("spray_ms", 0.0, 500.0, 50.0, "ms", "Random range of the grain position"),
            p("grain_ms", 5.0, 500.0, 60.0, "ms", "Grain length. Short (10-30) sounds grainy; long (100+) is smoother"),
            p("density_hz", 0.5, 200.0, 30.0, "Hz", "Grains per second"),
            p("pitch_semitones", -24.0, 24.0, 0.0, "semitones", "Grain pitch shift. 12 = octave-up shimmer; -12 = deep"),
            p("pitch_spray", 0.0, 24.0, 0.0, "semitones", "Random range of the pitch shift"),
            p("stereo_spread", 0.0, 1.0, 0.5, "", "How far the grains spread left and right"),
            p("feedback", 0.0, 0.95, 0.0, "", "Feedback. Higher = thicker, longer cloud"),
            p("mix", 0.0, 1.0, 0.5, "", "Dry/wet mix. Use 1 on a send bus"),
        ],
        choices: &[],
        example: r#"{ "type": "granular_delay", "pitch_semitones": 12, "feedback": 0.4, "mix": 0.4 }"#,
    },
    EffectSpec {
        name: "formant_filter",
        summary: "Vowel filter: filters by the formants of vocal vowels so a synth \"talks\" (a-e-i-o-u).",
        placement: "Good for synth lead, bass and pad. Moving vowel back and forth with automation gives a \"wah-wah\" talking feel.",
        params: &[
            p("vowel", 0.0, 4.0, 0.0, "", "Vowel position: 0 a, 1 e, 2 i, 3 o, 4 u, with smooth transitions in between"),
            p("shift", -12.0, 12.0, 0.0, "semitones", "Formant shift. + a smaller voice; - a bigger voice"),
            p("gain_db", -24.0, 24.0, 0.0, "dB", "Output gain"),
            p("mix", 0.0, 1.0, 1.0, "", "Dry/wet mix"),
        ],
        choices: &[],
        example: r#"{ "type": "formant_filter", "vowel": 1.5, "mix": 0.8 }"#,
    },
    EffectSpec {
        name: "plugin",
        summary: "Third-party plugin effect (VST3/CLAP/LV2).",
        placement: "Sits in the chain like a built-in effect. path is the absolute path of the plugin on this machine; params are set by plugin parameter name, and a value may be the text shown in the plugin UI (e.g. \"2000 Hz\"). Use `muser plugin params <path>` to list the parameters.",
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
            Effect::Distortion { .. } => "distortion",
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
            Effect::Distortion { drive_db, tone_hz, low_cut_hz, output_db, mix, .. } => vec![("drive_db", drive_db), ("tone_hz", tone_hz), ("low_cut_hz", low_cut_hz), ("output_db", output_db), ("mix", mix)],
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
                    "{name}.{field} = {value} is out of range [{}, {}]{}",
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
                        "eq.shape = '{shape}' is invalid; choose one of: {}",
                        cs.choices.join(" / ")
                    ));
                }
            }
            Effect::Distortion { mode, cabinet, .. } => {
                for (cs, v) in spec.choices.iter().zip([mode, cabinet]) {
                    if !cs.choices.contains(&v.trim().to_ascii_lowercase().as_str()) {
                        return Err(format!("distortion.{} = '{v}' is invalid; choose one of: {}", cs.name, cs.choices.join(" / ")));
                    }
                }
            }
            Effect::Tremolo { shape, .. } | Effect::AutoPan { shape, .. } => {
                let cs = &spec.choices[0];
                if !cs.choices.contains(&shape.trim().to_ascii_lowercase().as_str()) {
                    return Err(format!("{name}.shape = '{shape}' is invalid; choose one of: {}", cs.choices.join(" / ")));
                }
            }
            Effect::Vocoder { sidechain: None, .. } => {
                return Err("vocoder needs \"sidechain\": the name of the track or bus carrying the modulator signal (usually a vocal)".to_string());
            }
            Effect::Compressor { .. } | Effect::Vocoder { .. } if self.sidechain().is_some_and(|s| s.trim().is_empty()) => {
                return Err(format!("{name}.sidechain must not be an empty string"));
            }
            Effect::Plugin { path, .. } if path.trim().is_empty() => {
                return Err("plugin.path must not be empty".to_string());
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
