#!/usr/bin/env python3
"""生成 skills/ 下的子 skill 文档。

参数名、范围、默认值、单位、说明、放置建议、示例全部来自代码里的描述表
（`muser effects --json` / `muser instruments --json`），保证文档与代码一致。
本脚本只补充手写的场景预设与经验。代码描述表变动后重新运行：

    scripts/gen_skills.py [path/to/muser]

脚本会把每个场景预设放进一个临时工程交给 `muser validate`，预设越界会直接报错。
"""

import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MUSER = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "muser")
OUT = os.path.join(ROOT, "skills")
HEADER = "<!-- 由 scripts/gen_skills.py 生成；参数表来自代码描述表，请改代码或脚本后重新生成 -->\n\n"


def muser_json(*args):
    out = subprocess.run([MUSER, *args, "--json"], check=True, capture_output=True, text=True)
    return json.loads(out.stdout)


EFFECTS = muser_json("effects")
INSTRUMENTS = muser_json("instruments")

# ---------------------------------------------------------------------------
# 效果器：场景预设 + 经验（手写）
# ---------------------------------------------------------------------------

EFFECT_EXTRA = {
    "reverb": {
        "presets": [
            ("小房间（鼓、原声乐器显得真实）", {"type": "reverb", "mix": 0.15, "decay_seconds": 0.6, "damping": 0.5, "size": 0.25}),
            ("常规大厅（pad、弦乐、人声）", {"type": "reverb", "mix": 0.3, "decay_seconds": 2.0, "damping": 0.5, "size": 0.6}),
            ("氛围长尾（ambient、lo-fi 键盘）", {"type": "reverb", "mix": 0.45, "decay_seconds": 5.0, "damping": 0.6, "size": 0.9}),
            ("明亮金属感（钟琴、合成器 pluck）", {"type": "reverb", "mix": 0.25, "decay_seconds": 1.8, "damping": 0.15, "size": 0.6}),
        ],
        "tips": [
            "多条轨都要空间感时，每条轨用小 mix（0.1-0.2），比单条轨大 mix 更自然。",
            "贝斯、底鼓尽量不加；非要加时 mix ≤ 0.1，并在前面串一个 `eq` highpass 150Hz。",
            "混响放在链尾（delay 之后）；放在 compressor 之前会让尾音被压缩而变得不自然。",
            "尾音会延续到工程结束之后，渲染时 `--tail` 要大于 decay_seconds，否则会被截断。",
        ],
    },
    "delay": {
        "presets": [
            ("八分音符回声 @120bpm（time = 30/tempo）", {"type": "delay", "time_seconds": 0.25, "feedback": 0.35, "mix": 0.3}),
            ("附点八分 @120bpm（time = 45/tempo，流行/摇滚 lead 经典）", {"type": "delay", "time_seconds": 0.375, "feedback": 0.4, "mix": 0.3}),
            ("slapback 短回声（复古人声/吉他）", {"type": "delay", "time_seconds": 0.11, "feedback": 0.0, "mix": 0.25}),
            ("dub 长回声（反复多次）", {"type": "delay", "time_seconds": 0.5, "feedback": 0.65, "mix": 0.4}),
        ],
        "tips": [
            "time_seconds 按 tempo 计算才合拍：四分 = 60/tempo，八分 = 30/tempo，附点八分 = 45/tempo，十六分 = 15/tempo。",
            "feedback > 0.7 时回声会堆积成持续轰鸣，只适合 dub 或过渡段。",
            "密集的快速音符加长回声会变糊；回声适合留有空隙的乐句。",
        ],
    },
    "compressor": {
        "presets": [
            ("主总线胶水（整体更紧实）", {"type": "compressor", "threshold_db": -14, "ratio": 2, "attack_ms": 30, "release_ms": 200, "makeup_db": 2}),
            ("鼓组冲击（保留瞬态）", {"type": "compressor", "threshold_db": -18, "ratio": 4, "attack_ms": 25, "release_ms": 100, "makeup_db": 3}),
            ("贝斯平稳（音量起伏变小）", {"type": "compressor", "threshold_db": -20, "ratio": 4, "attack_ms": 5, "release_ms": 150, "makeup_db": 3}),
            ("重压（明显的泵感）", {"type": "compressor", "threshold_db": -30, "ratio": 8, "attack_ms": 1, "release_ms": 60, "makeup_db": 8}),
        ],
        "tips": [
            "压缩后整体音量会变小，用 makeup_db 补回；补完后峰值可能变高，主总线再用 limiter 收住。",
            "attack 太短会削掉鼓的'啪'声，鼓组建议 ≥ 20ms。",
            "主总线常见顺序：eq → compressor → limiter。",
        ],
    },
    "chorus": {
        "presets": [
            ("轻微加厚（电钢、pad）", {"type": "chorus", "rate_hz": 0.5, "depth_ms": 2.0, "mix": 0.3}),
            ("80 年代宽阔合成器", {"type": "chorus", "rate_hz": 0.8, "depth_ms": 5.0, "mix": 0.5}),
            ("明显颤动（特殊效果）", {"type": "chorus", "rate_hz": 3.0, "depth_ms": 6.0, "mix": 0.5}),
        ],
        "tips": [
            "贝斯加 chorus 会让低频相位抵消、发虚；如需要，mix ≤ 0.2。",
            "chorus 放在 reverb 之前。",
        ],
    },
    "eq": {
        "presets": [
            ("去掉非贝斯轨的低频浑浊", {"type": "eq", "shape": "highpass", "frequency": 150, "q": 0.707}),
            ("提亮高频（空气感）", {"type": "eq", "shape": "high_shelf", "frequency": 8000, "q": 0.707, "gain_db": 3}),
            ("让声音变闷、变远（lo-fi）", {"type": "eq", "shape": "lowpass", "frequency": 3500, "q": 0.707}),
            ("压低中低频的'箱声'", {"type": "eq", "shape": "peak", "frequency": 350, "q": 1.2, "gain_db": -4}),
            ("加厚低频（底鼓、贝斯）", {"type": "eq", "shape": "low_shelf", "frequency": 100, "q": 0.707, "gain_db": 3}),
        ],
        "tips": [
            "一个 eq 只管一个频段；要多段就在 effects 里串多个 eq。",
            "减法比加法安全：先切掉不需要的频段，再小幅提升（±3dB 以内）。",
            "多条轨都在 100Hz 以下有能量时，会互相打架；只让底鼓和贝斯占这一段，其它轨用 highpass。",
            "gain_db 只对 peak / low_shelf / high_shelf 起作用，写在 highpass 上不会报错但没有效果。",
        ],
    },
    "limiter": {
        "presets": [
            ("常规母带（安全）", {"type": "limiter", "ceiling_db": -1.0, "release_ms": 80}),
            ("更响（流行/电子）", {"type": "limiter", "ceiling_db": -0.3, "release_ms": 50}),
        ],
        "tips": [
            "只放在 master_effects 的最后一个。",
            "limiter 只防止爆音，不会让作品自动变好听；输入推得太猛会挤压动态、失真。",
            "要更响时，先提 master_gain_db 或前面 compressor 的 makeup_db，再由 limiter 收住峰值。",
        ],
    },
    "phaser": {
        "presets": [
            ("缓慢流动（pad、电钢）", {"type": "phaser", "rate_hz": 0.2, "feedback": 0.3, "mix": 0.5}),
            ("70 年代放克吉他/电钢", {"type": "phaser", "rate_hz": 0.6, "feedback": 0.5, "mix": 0.5}),
            ("强烈喷气式", {"type": "phaser", "rate_hz": 1.5, "feedback": 0.8, "mix": 0.6}),
        ],
        "tips": [
            "mix 0.5 时效果最明显；更高或更低都会减弱扫频感。",
            "phaser 放在 reverb 之前。",
        ],
    },
    "flanger": {
        "presets": [
            ("慢扫喷气机（过渡段）", {"type": "flanger", "rate_hz": 0.15, "feedback": 0.7, "mix": 0.5}),
            ("轻微金属着色", {"type": "flanger", "rate_hz": 0.3, "feedback": 0.4, "mix": 0.35}),
        ],
        "tips": [
            "效果很强烈，适合点缀段落或单独的轨，不建议整首都开在主旋律上。",
            "feedback 接近 0.95 时谐振非常尖，注意音量。",
        ],
    },
    "plugin": {
        "presets": [],
        "tips": [
            "需要用 `cargo build -p muser-cli --features host` 构建的 muser；未启用时渲染报 HostDisabled。",
            "path 是本机绝对路径，常见目录：/usr/lib/vst3、~/.vst3、/usr/lib/clap、~/.clap、/usr/lib/lv2。",
            "一个插件文件里有多个插件时，用 `id` 指定；不写则用第一个。",
            "`params` 用插件自己的参数 id（整数）设置初始值；目前没有列出插件参数的命令，参数调节是后续工作。",
            "加载插件会运行第三方代码，只使用可信的本机插件。",
        ],
    },
}


def fmt_num(v):
    return str(int(v)) if float(v).is_integer() else str(v)


def effect_page(spec):
    extra = EFFECT_EXTRA[spec["name"]]
    lines = [HEADER, f"# 效果器：{spec['name']}\n\n", f"{spec['summary']}\n\n"]
    lines.append(f"**放在哪里**：{spec['placement']}\n\n")
    lines.append(f"查看最新参数：`muser effects {spec['name']}`（MCP：`describe_effect`）\n\n")
    if spec["params"] or spec["choices"]:
        lines.append("## 参数\n\n| 参数 | 默认 | 范围 | 说明 |\n|---|---|---|---|\n")
        for c in spec["choices"]:
            lines.append(f"| `{c['name']}` | `{c['default']}` | {' / '.join('`'+x+'`' for x in c['choices'])} | {c['description']} |\n")
        for p in spec["params"]:
            unit = f" {p['unit']}" if p["unit"] else ""
            lines.append(f"| `{p['name']}` | {fmt_num(p['default'])} | {fmt_num(p['min'])} – {fmt_num(p['max'])}{unit} | {p['description']} |\n")
        lines.append("\n所有参数都可省略，省略时取默认值；超出范围时 `muser validate` 会报出具体位置。\n\n")
    else:
        lines.append("## 字段\n\n| 字段 | 说明 |\n|---|---|\n| `path` | 必填，插件文件或包的绝对路径（.vst3 / .clap / .lv2） |\n| `id` | 可选，插件 id |\n| `params` | 可选，`{ 参数id: 值 }` |\n\n")
    lines.append(f"## 最小示例\n\n```json\n{spec['example']}\n```\n\n")
    if extra["presets"]:
        lines.append("## 场景预设\n\n")
        for title, preset in extra["presets"]:
            lines.append(f"- {title}\n\n  ```json\n  {json.dumps(preset, ensure_ascii=False)}\n  ```\n\n")
    lines.append("## 经验与常见错误\n\n")
    for t in extra["tips"]:
        lines.append(f"- {t}\n")
    return "".join(lines)


# ---------------------------------------------------------------------------
# 乐器（按类别分页）
# ---------------------------------------------------------------------------

GM_KEYS = [
    (35, "底鼓 2 (Acoustic Bass Drum)"), (36, "底鼓 (Bass Drum)"), (37, "边击 (Side Stick)"),
    (38, "军鼓 (Snare)"), (39, "拍手 (Hand Clap)"), (40, "电军鼓 (Electric Snare)"),
    (41, "落地桶鼓 低 (Low Floor Tom)"), (42, "闭镲 (Closed Hi-Hat)"), (43, "落地桶鼓 高 (High Floor Tom)"),
    (44, "踩镲 (Pedal Hi-Hat)"), (45, "低桶鼓 (Low Tom)"), (46, "开镲 (Open Hi-Hat)"),
    (47, "中低桶鼓 (Low-Mid Tom)"), (48, "中高桶鼓 (High-Mid Tom)"), (49, "吊镲 (Crash)"),
    (50, "高桶鼓 (High Tom)"), (51, "叮叮镲 (Ride)"), (52, "中国镲 (China)"), (53, "叮叮镲帽 (Ride Bell)"),
    (54, "铃鼓 (Tambourine)"), (55, "溅镲 (Splash)"), (56, "牛铃 (Cowbell)"), (57, "吊镲 2 (Crash 2)"),
    (59, "叮叮镲 2 (Ride 2)"), (60, "高音邦戈 (High Bongo)"), (61, "低音邦戈 (Low Bongo)"),
    (62, "康加 闷音 (Mute High Conga)"), (63, "康加 开音 (Open High Conga)"), (64, "低音康加 (Low Conga)"),
    (69, "卡巴萨 (Cabasa)"), (70, "沙锤 (Maracas)"), (73, "短刮瓜 (Short Guiro)"),
    (74, "长刮瓜 (Long Guiro)"), (75, "响棒 (Claves)"),
]

# 每套鼓实际映射的键（由 DrumKit::keys() 导出；manguebyte 升级后需复核）。
KIT_KEYS = {
    "drums.808": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,55,56,57,59,62,63,64,70,75",
    "drums.909": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,55,57,59",
    "drums.606": "35,36,38,40,41,42,43,44,45,46,47,48,49,50,51,52,55,57,59",
    "drums.cr78": "35,36,37,38,40,42,44,46,49,51,54,56,57,59,60,61,62,63,64,69,70,73,74,75",
    "drums.acoustic": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,59,60,61,62,63,64,69,70,73,74,75",
    "drums.trap": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,56,57,59",
    "drums.techno": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,53,56,57,59",
    "drums.house": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,53,54,56,57,59,62,63,64,69,70",
    "drums.dnb": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,55,57,59",
    "drums.lofi": "35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,53,54,57,59,69,70",
}

FAMILY_PAGES = {
    "drums": ("鼓组", "drums.md", [
        "音符的 `pitch` 是 **GM 鼓键**，不是音阶音高。用 60（中央 C）会触发高音邦戈（cr78 / acoustic）或没有声音。",
        "鼓的音符 `length` 用 0.1-0.25 即可；吊镲、开镲的余音由鼓件自身决定。",
        "开镲(46)之后紧接闭镲(42)或踩镲(44)会把开镲掐断，这是正常的鼓机行为。",
        "力度 `velocity` 用来做强弱：重拍 0.9-1.0，弱拍 0.6-0.8，鬼音(ghost) 0.3-0.4。",
    ]),
    "synth": ("通用合成器", "synth.md", [
        "`synth.poly` 是复音合成器的初始音色，比较朴素；要好听的音色请直接用 `poly.*` 预设（见 poly.md）。",
        "`synth.fm` 偏电子、带金属感，适合做电子贝斯和铃声类音色。",
    ]),
    "guitar": ("吉他", "guitar.md", [
        "物理建模拨弦：音符一开始就是拨弦，`length` 决定何时止音。分解和弦时让音符重叠更自然。",
        "扫弦：同一和弦的音符起点依次错开 0.01-0.03 拍，从低音到高音。",
    ]),
    "bass": ("贝斯", "bass.md", [
        "`bass.acid*` 是单音（复音数 1）：前一个音还没结束就开始下一个音，会产生 303 式滑音；想要分开的音就让音符之间留空隙。",
        "贝斯一般放在 28-55；低于 28 很多音箱放不出来。",
        "贝斯和底鼓同时出现在重拍上时，可以给贝斯加 compressor 让音量更稳定。",
    ]),
    "organ": ("风琴", "organ.md", [
        "拉杆风琴适合长音铺底：和弦音符 `length` 放长（2-4 拍）。",
        "配合 `chorus` 或较慢的 `phaser` 可以模拟旋转扬声器的晃动感。",
    ]),
    "mallet": ("打击旋律乐器", "mallet.md", [
        "音色本身会自然衰减，`length` 太长没有影响；适合琶音和短旋律。",
        "颤音琴、管钟余音很长，快速连奏会叠在一起；需要干净时降低速度或减少音符。",
    ]),
    "strings": ("弦乐", "strings.md", [
        "拉奏乐器有起音过程，`length` 太短（< 0.25 拍）听起来会弱；短音请用 `*_pizz` 拨奏版本。",
        "多声部弦乐：小提琴(旋律) + 中提琴(内声部) + 大提琴(低音) 分三条轨，比单条 `strings.ensemble` 更有层次。",
        "配一点 `reverb`（mix 0.25-0.35、decay 2s 左右）会更像在音乐厅里。",
    ]),
    "wind": ("管乐", "wind.md", [
        "管乐是单旋律乐器，同一时刻尽量只放一个音；复音数只有 4。",
        "音域外的音能发声，但会失真或不像该乐器，参考下表的推荐音域。",
    ]),
    "choir": ("人声 / 合唱", "choir.md", [
        "合唱适合长音和弦铺底（2-4 拍）；短促的音听起来不自然。",
        "`choir.solo` 复音数 2，只适合单旋律。",
    ]),
}

POLY_TIPS = [
    "这些是 manguebyte 自带的 68 个合成器预设，按用途分 7 类。写法：`\"instrument\": \"poly.<名称>\"`。",
    "按用途选：贝斯选 bass，主旋律选 lead，铺底选 pad，短促的琶音/拨奏选 pluck，和弦伴奏选 keys，铜管齐奏选 brass，转场音效选 fx。",
    "pad 类起音和释音都较慢，音符要长（2 拍以上），并提前一点开始。",
    "fx 类（riser / downlifter 等）通常放一个长音（4-8 拍）在段落转换前。",
]


def instrument_row(i):
    return f"| `{i['name']}` | {i['range'][0]}–{i['range'][1]} | {i['polyphony']} | {i['description']} |\n"


def describe(name):
    return muser_json("instruments", name)


def fmt_range(p):
    t = p["type"]
    if t == "number":
        unit = f" {p['unit']}" if p["unit"] else ""
        r = f"{fmt_num(p['min'])} – {fmt_num(p['max'])}{unit}"
        if p.get("integer"):
            r += "（整数）"
        if p.get("names"):
            r += "，或 " + " / ".join(f"`\"{n}\"`" for n in p["names"])
        return r
    if t == "choice":
        return " / ".join(f"`\"{c}\"`" for c in p["choices"])
    if t == "bool":
        return "`true` / `false`"
    if t == "drawbars":
        return "9 位数字串，每位 0-8"
    return ""


AUTO_LABEL = {"live": "✓", "next_note": "下个音", "no": "—"}


def fmt_val(v):
    if v is None:
        return "—"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str):
        return f'"{v}"'
    return fmt_num(round(v, 4))


# 每类乐器调音示例（会经 muser validate 校验）。
PARAM_EXAMPLES = {
    "drums": [("底鼓更沉更长、军鼓更脆、闭镲小声", "drums.808", {"kick": {"tune": -3, "decay": 1.8}, "snare": {"snap": 0.8}, "closed_hat": {"level": 0.6}})],
    "synth": [("FM 音色更长的尾巴", "synth.fm", {"release": 1.2})],
    "guitar": [("更暗、更闷的指弹（靠近琴颈拨弦）", "guitar.nylon", {"tone": 2500, "pluck_brightness": 1500, "pluck_position": 0.3}),
               ("闷音（palm mute 感）", "guitar.steel", {"decay": 0.3, "release": 0.05})],
    "bass": [("更闷更短的指弹贝斯", "bass.finger", {"tone": 1200, "decay": 1.0}),
             ("尖叫 acid（配合 accent 音符）", "bass.acid", {"cutoff": 250, "resonance": 0.9, "env_mod": 0.8, "decay": 0.3, "accent": 1.0})],
    "organ": [("柔和的抒情拉杆 + 打击泛音", "organ.jazz", {"drawbars": "838000000", "percussion": "second", "click": 0.2})],
    "mallet": [("软槌、长余音的马林巴", "mallet.marimba", {"softness": 5, "decay": 3.0}),
               ("颤音琴打开电机颤音", "mallet.vibraphone", {"motor_rate": 5, "motor_depth": 0.6})],
    "strings": [("慢起音、深揉弦的抒情大提琴", "strings.cello", {"attack": 0.4, "vibrato_depth": 25, "vibrato_delay": 0.3, "release": 1.0}),
                ("小编制弦乐组（2 人）", "strings.ensemble", {"players": 2})],
    "wind": [("气声多、颤音少的长笛", "wind.flute", {"breath_noise": 0.6, "vibrato_depth": 5}),
             ("滑音长号", "wind.trombone", {"portamento": True})],
    "choir": [("'ooh' 元音、6 人、偏暗", "choir.aah", {"vowel": "o", "singers": 6, "brightness": 2500})],
}

POLY_EXAMPLES = [
    ("把 moog bass 变得更亮更有'哇'声", "poly.moog_bass", {"cutoff": 900, "resonance": 0.5, "filter_env": 5}),
    ("pad 更慢地浮现、更长的尾巴", "poly.warm_analog_pad", {"attack": 1.5, "release": 4.0}),
    ("lead 加滑音", "poly.mono_saw_lead", {"glide": 0.12}),
]


def params_section(family, items):
    # 同类乐器参数表一致（鼓组除外），取第一个作代表；当前值逐个列出。
    infos = [describe(i["name"]) for i in items]
    rep = infos[0]
    if not rep["params"]:
        return ""
    lines = ["\n## 可调参数（instrument_params）\n\n",
             "在所选乐器/预设的音色上覆盖，只写想改的项；其余保持原样。`muser instruments <名称>` 可看到该预设的当前值。\n\n"]
    if family == "drums":
        lines.append(rep["params_note"].split("这套鼓支持的鼓键")[0].strip() + "\n\n")
        lines.append("每个鼓件可调（自动化时写成 `鼓件.参数`，如 `kick.tune`，从下一次击打生效）：\n\n| 参数 | 范围 | 自动化 | 说明 |\n|---|---|---|---|\n")
        for p in rep["params"]:
            lines.append(f"| `{p['name']}` | {fmt_range(p)} | {AUTO_LABEL[p['automation']]} | {p['description']} |\n")
    else:
        lines.append("| 参数 | 范围 | 自动化 | 说明 |\n|---|---|---|---|\n")
        for p in rep["params"]:
            lines.append(f"| `{p['name']}` | {fmt_range(p)} | {AUTO_LABEL[p['automation']]} | {p['description']} |\n")
        lines.append("\n自动化列：✓ = 正在发声的音也立即跟随；下个音 = 只影响之后开始的音；— = 不能自动化。见 [automation.md](../automation.md)。\n")
        if family != "synth_one":
            names = [p["name"] for p in rep["params"]]
            lines.append("\n各乐器的预设取值：\n\n| instrument | " + " | ".join(f"`{n}`" for n in names) + " |\n|---|" + "---|" * len(names) + "\n")
            for info in infos:
                cur = {p["name"]: p["current"] for p in info["params"]}
                if [p["name"] for p in info["params"]] != names:
                    continue
                lines.append(f"| `{info['name']}` | " + " | ".join(fmt_val(cur[n]) for n in names) + " |\n")
    ex = PARAM_EXAMPLES.get(family, [])
    if ex:
        lines.append("\n### 调音示例\n\n")
        for title, inst, params in ex:
            lines.append(f"- {title}\n\n  ```json\n  {{ \"instrument\": \"{inst}\", \"instrument_params\": {json.dumps(params, ensure_ascii=False)} }}\n  ```\n\n")
    return "".join(lines)


def family_page(family, title, tips):
    items = [i for i in INSTRUMENTS if i["family"] == family]
    note = "GM 鼓键" if family == "drums" else "MIDI 音高（60 = 中央 C）"
    lines = [HEADER, f"# 乐器：{title}\n\n", f"音符 `pitch` 使用 **{note}**。查看最新列表：`muser instruments --family {family}`。\n\n"]
    lines.append("| instrument | 推荐音域 | 复音数 | 说明 |\n|---|---|---|---|\n")
    for i in items:
        lines.append(instrument_row(i))
    if family == "synth":
        # synth.poly 与 synth.fm 参数不同，分别列出，不带"各乐器预设取值"表。
        for i in items:
            sec = params_section("synth_one", [i])
            lines.append(sec.replace("## 可调参数（instrument_params）", f"## `{i['name']}` 的可调参数"))
        if PARAM_EXAMPLES.get("synth"):
            lines.append("\n### 调音示例\n\n")
            for title, inst, params in PARAM_EXAMPLES["synth"]:
                lines.append(f"- {title}\n\n  ```json\n  {{ \"instrument\": \"{inst}\", \"instrument_params\": {json.dumps(params, ensure_ascii=False)} }}\n  ```\n\n")
    else:
        lines.append(params_section(family, items))
    if family == "drums":
        lines.append("\n## GM 鼓键\n\n| pitch | 鼓件 |\n|---|---|\n")
        for k, n in GM_KEYS:
            lines.append(f"| {k} | {n} |\n")
        lines.append("\n## 各套鼓支持的键\n\n不在列表里的键没有声音。\n\n| 套鼓 | 支持的 pitch |\n|---|---|\n")
        for name, keys in KIT_KEYS.items():
            lines.append(f"| `{name}` | {keys} |\n")
        lines.append("\n注意：`drums.606` 和 `drums.cr78` 没有拍手(39)。\n")
        lines.append("\n## 常用节奏骨架（4/4，一小节 = 4 拍）\n\n")
        lines.append("- 基础 pop/rock：底鼓 0、2；军鼓 1、3；闭镲每 0.5 拍一次。\n")
        lines.append("- house/techno 四踩：底鼓 0、1、2、3；拍手 1、3；开镲在 0.5、1.5、2.5、3.5。\n")
        lines.append("- trap（半速感）：底鼓 0、1.75、2.5；军鼓/拍手 2；闭镲每 0.25 拍，局部 0.125 拍连打。\n")
        lines.append("- drum & bass：底鼓 0、2.5；军鼓 1、3；闭镲每 0.5 拍。\n")
    lines.append("\n## 经验与常见错误\n\n")
    for t in tips:
        lines.append(f"- {t}\n")
    return "".join(lines)


def poly_page():
    cats = ["bass", "lead", "pad", "pluck", "keys", "brass", "fx"]
    lines = [HEADER, "# 乐器：poly 合成器预设\n\n", "音符 `pitch` 使用 **MIDI 音高（60 = 中央 C）**。查看最新列表：`muser instruments --family poly`。\n\n"]
    for t in POLY_TIPS:
        lines.append(f"- {t}\n")
    rep = describe("poly.moog_bass")
    nofilter = []
    for i in INSTRUMENTS:
        if i["family"].startswith("poly/"):
            cur = {p["name"]: p["current"] for p in describe(i["name"])["params"]}
            if cur.get("cutoff") is None:
                nofilter.append(f"`{i['name']}`")
    lines.append("\n## 可调参数（instrument_params，所有 poly.* 通用）\n\n在预设音色上覆盖，只写想改的项。各预设的当前值用 `muser instruments poly.<名称>` 查看。\n以下预设没有启用滤波器，`cutoff` / `resonance` / `filter_env` 对它们无效（校验会拒绝）：{nofilter}。\n\n| 参数 | 范围 | 自动化 | 说明 |\n|---|---|---|---|\n".replace("{nofilter}", "、".join(nofilter)))
    for p in rep["params"]:
        lines.append(f"| `{p['name']}` | {fmt_range(p)} | {AUTO_LABEL[p['automation']]} | {p['description']} |\n")
    lines.append("\n### 调音示例\n\n")
    for title, inst, params in POLY_EXAMPLES:
        lines.append(f"- {title}\n\n  ```json\n  {{ \"instrument\": \"{inst}\", \"instrument_params\": {json.dumps(params, ensure_ascii=False)} }}\n  ```\n\n")
    for c in cats:
        items = [i for i in INSTRUMENTS if i["family"] == f"poly/{c}"]
        lines.append(f"\n## {c}（{len(items)} 个，推荐音域 {items[0]['range'][0]}–{items[0]['range'][1]}）\n\n| instrument | 说明 |\n|---|---|\n")
        for i in items:
            lines.append(f"| `{i['name']}` | {i['description']} |\n")
    return "".join(lines)


AUTOMATION_EXAMPLES = [
    ("滤波扫频：8 小节内从闷到亮（exp 曲线让频率变化听起来均匀）", "bass.acid", [],
     {"target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp"}),
    ("淡入：前 4 拍从 -60 dB 升到 0 dB", "poly.warm_analog_pad", [],
     {"target": "gain_db", "shape": "points", "points": [[0, -60], [4, 0]], "curve": "smooth"}),
    ("声像自动摆动：每 2 小节左右来回一次", "poly.juno_keys", [],
     {"target": "pan", "shape": "lfo", "wave": "sine", "period_beats": 8, "min": -0.7, "max": 0.7}),
    ("acid 随机游走：每半拍滤波随机小幅移动，每次渲染一样，换 seed 换一种", "bass.acid", [],
     {"target": "instrument", "param": "cutoff", "shape": "random", "style": "walk", "step_beats": 0.5, "min": 200, "max": 2500, "max_step": 0.25}),
    ("合唱元音变化：a → o → u", "choir.aah", [],
     {"target": "instrument", "param": "vowel", "shape": "points", "points": [[0, "a"], [8, "o"], [16, "u"]], "curve": "smooth"}),
    ("效果器参数：第 9 拍起回声越来越多（只在 8-16 拍生效）", "synth.poly", [{"type": "delay", "time_seconds": 0.25, "feedback": 0.2, "mix": 0.3}],
     {"target": "effects[0]", "param": "feedback", "shape": "points", "points": [[8, 0.2], [16, 0.75]], "start": 8, "end": 16}),
    ("鼓件：每小节底鼓随机调音（下一次击打生效）", "drums.808", [],
     {"target": "instrument", "param": "kick.tune", "shape": "random", "style": "jump", "step_beats": 4, "min": -3, "max": 3}),
    ("门限式开关：方波 LFO 让 pad 每拍一开一关（trance gate）", "poly.warm_analog_pad", [],
     {"target": "gain_db", "shape": "lfo", "wave": "square", "period_beats": 0.5, "min": -40, "max": 0}),
]

MASTER_EXAMPLE = {"target": "master_effects[0]", "param": "mix", "shape": "points", "points": [[0, 0.05], [16, 0.4]], "curve": "smooth"}


def automation_page():
    lines = [HEADER, """# 自动化：让参数随时间变化

自动化让乐器参数、效果器参数、增益、声像在演奏中变化：滤波扫频、淡入淡出、声像摆动、
带种子的随机游走。时间单位是**拍**，从工程开头算起，与音符一致；取值用参数自己的单位（Hz、秒、dB…）。

## 写在哪里

- 音轨的 `automation`：目标 `instrument`（乐器参数）、`effects[i]`（该轨第 i 个效果器，从 0 起）、`gain_db`、`pan`。
- 工程的 `automation`：目标 `master_gain_db`、`master_effects[i]`。

```json
{
  "tempo": 128,
  "seed": 7,
  "master_effects": [{ "type": "reverb", "mix": 0.05 }],
  "automation": [ %s ],
  "tracks": [{
    "instrument": "bass.acid",
    "automation": [
      { "target": "instrument", "param": "cutoff", "shape": "points", "points": [[0, 200], [32, 4000]], "curve": "exp" }
    ],
    "clips": [ ... ]
  }]
}
```

`param` 与 `instrument_params` / 效果器参数同名；鼓组写 `鼓件.参数`（如 `kick.tune`）；目标是 `gain_db` / `pan` /
`master_gain_db` 时省略。哪些参数能自动化、变化何时生效，看 `muser instruments <名称>` 里的标注，
或各乐器子 skill 参数表的"自动化"列。

## 三种形状

### points：时间点曲线

```json
{ "shape": "points", "points": [[0, 200], [16, 3000], [32, 800]], "curve": "exp" }
```

- `points`：`[拍, 值]`，时间严格递增。第一个点之前取第一个值，最后一个点之后保持最后的值。
- `curve`：`linear`（默认，直线）、`exp`（按比例变化，频率/时间类参数听感更均匀；要求相邻点同号且非零）、
  `smooth`（两端平缓的 S 形）、`step`（保持前一个值直到下一个点，做"切换"）。
- 值可以写参数允许的名字，如合唱元音 `"a"` `"o"`。

### lfo：周期摆动

```json
{ "shape": "lfo", "wave": "sine", "period_beats": 4, "min": -0.5, "max": 0.5, "phase": 0 }
```

- `wave`：`sine`（默认）/ `triangle` / `saw`（从 min 升到 max 再跳回）/ `square`（min 与 max 间切换）。
- `period_beats`：一个周期多少拍。4 = 每小节一次（4/4），1 = 每拍，0.5 = 每八分音符。按拍计，所以跟着速度走。
- `phase`：起始相位 0-1，`sine` 在 0 处从 min 开始。

### random：带种子的随机

```json
{ "shape": "random", "style": "walk", "step_beats": 0.5, "min": 200, "max": 2500, "max_step": 0.2 }
```

- `style`：`jump`（每步跳到新随机值）/ `glide`（默认，平滑滑向新值）/ `walk`（随机游走：每步在当前值附近移动，
  最多 `max_step` × 范围）。
- `step_beats`：多少拍换一次目标值。
- `seed`（可省略）：省略时由工程的 `seed` 与这条自动化的位置推导。**同一种子每次渲染完全相同；换工程 `seed`
  就换一种随机变化**，适合"多渲染几版挑一个"。

## 生效区间

`start` / `end`（拍，可省略）限定自动化生效的时间；区间外参数回到静态值（`instrument_params` 或效果器里写的值）。
`lfo` / `random` 的相位与随机序列从 `start` 开始计。

## 行为说明

- 乐器与效果器参数每 64 帧（48 kHz 下约 1.3 ms）更新一次；增益与声像逐帧更新，淡入淡出没有台阶。
- "下个音生效"的参数（如弓弦的揉弦、鼓件的调音）只影响自动化之后开始的音符，已经在响的音不变。
- 同一目标多条自动化：效果器/乐器参数按顺序写入，后写的覆盖先写的；`gain_db` / `pan` 取最后一条在生效区间内的值。
- 取值超出参数范围、参数名错误、参数不能自动化（开关、选项、整数），`muser validate` 都会指出具体位置。
- 外置插件与 `sfz:` 乐器的参数自动化尚未支持。

## 示例

""" % json.dumps(MASTER_EXAMPLE, ensure_ascii=False)]
    for title, inst, effects, auto in AUTOMATION_EXAMPLES:
        track = {"instrument": inst}
        if effects:
            track["effects"] = effects
        track["automation"] = [auto]
        lines.append(f"- {title}\n\n  ```json\n  {json.dumps(track, ensure_ascii=False)}\n  ```\n\n")
    lines.append("""## 常见用法

- 段落推进：8-16 小节 `points` 把 `cutoff` 从低扫到高，配合主总线 `reverb.mix` 慢慢加大，营造 build-up。
- 让 loop 不重复：给 `cutoff` / `pan` / 鼓件 `tune` 加慢速 `random`（`walk` 或 `glide`），每一遍都略有不同。
- 换种子挑版本：同一工程改 `seed` 渲染几遍，挑最喜欢的一版；记下 seed 就能复现。
- 律动感：`square` LFO 作用在 `gain_db` 上做 trance gate；`sine` LFO 作用在 `pan` 上做自动声像。
""")
    return "".join(lines)


def sfz_page():
    return HEADER + """# 乐器：SFZ 采样音色库

用 muser 内置的 SFZ 采样器（manguebyte-sampler）播放 `.sfz` 音色库，不需要任何外部插件。

```json
{ "name": "piano", "instrument": "sfz:/path/to/Piano.sfz", "clips": [ ... ] }
```

- 语法：`sfz:<.sfz 文件路径>`；采样文件按 .sfz 里的相对路径从其所在目录读取（支持 WAV、FLAC 等）。
- 音符 `pitch` 用 MIDI 音高；音域、力度层、循环、释放采样都由 .sfz 文件定义。最多 32 个同时发声。
- `muser validate` 会检查 .sfz 文件是否存在；采样缺失或格式错误会在渲染时报出。
- 部分 SFZ opcode（如 `<curve>`、`<effect>`）不支持，渲染时会在 stderr 提示数量，音色可能与原设计略有差异。
- 音色由 .sfz 文件决定，暂不支持 `instrument_params`；需要调整时可在轨上加效果器（如 `eq`、`reverb`）。
- 免费音色库：freepats（钢琴、吉他等）、Versilian VSCO（管弦乐）、sfzinstruments.github.io。
- 路径是本机路径，工程换机器后需改成新位置。
"""


def plugin_page():
    return HEADER + """# 乐器：外置 VSTi（第三方虚拟乐器）

把本机安装的 VST3 / CLAP / LV2 乐器插件当作音源。音轨写法：

```json
{ "name": "lead", "instrument": "plugin:/usr/lib/vst3/Surge XT.vst3", "clips": [ ... ] }
```

- 语法：`plugin:<绝对路径>`；文件里有多个插件时用 `plugin:<路径>#<插件id>`。
- 需要用 `cargo build -p muser-cli --features host` 构建的 muser，否则报 HostDisabled。
- 音符 `pitch` 是 MIDI 音高；插件自己决定音色与音域。
- 采样器类插件（如 sfizz）需要加载音色文件才有声音；目前还不能通过工程设置插件参数或音色，这是后续工作。
- 常见插件目录：/usr/lib/vst3、~/.vst3、/usr/lib/clap、~/.clap、/usr/lib/lv2。
- 加载插件会运行第三方代码，只使用可信的本机插件。
- `muser validate` 只检查 `plugin:` 语法，不加载插件；路径错误会在 render 时报 PluginLoad。
"""


def validate_presets():
    """把所有场景预设放进一个工程，交给 muser validate。"""
    tracks = []
    for name, extra in EFFECT_EXTRA.items():
        for _, preset in extra["presets"]:
            tracks.append({"instrument": "synth.poly", "effects": [preset], "clips": []})
    for examples in list(PARAM_EXAMPLES.values()) + [POLY_EXAMPLES]:
        for _, inst, params in examples:
            tracks.append({"instrument": inst, "instrument_params": params, "clips": []})
    for _, inst, effects, auto in AUTOMATION_EXAMPLES:
        tracks.append({"instrument": inst, "effects": effects, "automation": [auto],
                       "clips": [{"start": 0, "length": 32, "notes": []}]})
    project = {"tempo": 120, "tracks": tracks, "master_effects": [{"type": "reverb"}], "automation": [MASTER_EXAMPLE]}
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(project, f)
        path = f.name
    try:
        r = subprocess.run([MUSER, "validate", path], capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"场景预设未通过校验:\n{r.stderr}")
    finally:
        os.unlink(path)
    return len(tracks)


def main():
    assert {e["name"] for e in EFFECTS} == set(EFFECT_EXTRA), "效果器与 EFFECT_EXTRA 不一致，请补充"
    families = {i["family"] for i in INSTRUMENTS if not i["family"].startswith("poly/")}
    assert set(PARAM_EXAMPLES) <= set(FAMILY_PAGES), "PARAM_EXAMPLES 有未知类别"
    assert families == set(FAMILY_PAGES), f"乐器类别与 FAMILY_PAGES 不一致: {families}"
    count = validate_presets()

    os.makedirs(os.path.join(OUT, "effects"), exist_ok=True)
    os.makedirs(os.path.join(OUT, "instruments"), exist_ok=True)
    for spec in EFFECTS:
        with open(os.path.join(OUT, "effects", f"{spec['name']}.md"), "w") as f:
            f.write(effect_page(spec))
    for family, (title, filename, tips) in FAMILY_PAGES.items():
        with open(os.path.join(OUT, "instruments", filename), "w") as f:
            f.write(family_page(family, title, tips))
    with open(os.path.join(OUT, "instruments", "poly.md"), "w") as f:
        f.write(poly_page())
    with open(os.path.join(OUT, "instruments", "plugin.md"), "w") as f:
        f.write(plugin_page())
    with open(os.path.join(OUT, "instruments", "sfz.md"), "w") as f:
        f.write(sfz_page())
    with open(os.path.join(OUT, "automation.md"), "w") as f:
        f.write(automation_page())
    print(f"已生成 {len(EFFECTS)} 个效果器页、{len(FAMILY_PAGES) + 3} 个乐器页、自动化页；{count} 个场景预设/调音/自动化示例通过校验")


if __name__ == "__main__":
    main()
