//! loop 库：可复用的鼓（或任何步进）节奏型。muser 自带一批（仓库的 `loops/`，编译进二进制），
//! 用户 / agent 攒下的放在 `~/.config/muser/loops/<名字>.json`（同名时用户的优先）。
//!
//! 每个 loop 是一段 [`Grid`] 加元数据（风格、速度范围、小节数、建议鼓组、律动 `humanize`）。鼓件用名字写
//! （kick / snare / hat …），换任意 `drums.*` 鼓组都能播。用进工程时**内嵌内容**：`muser loop show <名字>`
//! 输出一个 clip（`grid` + `humanize` + `"from": "loop:<名字>"`），工程因此自足，内置 loop 以后改了也不影响旧工程。

use std::path::{Path, PathBuf};

use muser_core::grid::Grid;
use muser_core::humanize::Humanize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const FORMAT: &str = "muser.loop";
pub const VERSION: u32 = 1;

/// 一个 loop。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loop {
    pub format: String,
    pub version: u32,
    /// 名字：小写字母、数字、`_`、`-`，同时是文件名。
    pub name: String,
    /// 风格（house / dnb / hiphop …），`muser loop list --style` 按它筛。
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 合适的速度范围 [最低, 最高]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<[f64; 2]>,
    /// 一遍几小节（按 4/4 计，一小节 4 拍）。
    pub bars: u32,
    #[serde(default = "default_meter")]
    pub meter: String,
    /// 建议的鼓组（如 `drums.909`）；用别的也行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    pub grid: Grid,
    /// 这个节奏型的律动（swing、随机偏移）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<Humanize>,
}

fn default_meter() -> String {
    "4/4".into()
}

/// 内置 loop。
const BUILTIN: &[(&str, &str)] = &[
    ("afrobeat", include_str!("../../../loops/afrobeat.json")),
    ("boom_bap", include_str!("../../../loops/boom_bap.json")),
    ("breakbeat", include_str!("../../../loops/breakbeat.json")),
    ("disco", include_str!("../../../loops/disco.json")),
    ("dnb_amen", include_str!("../../../loops/dnb_amen.json")),
    ("dnb_two_step", include_str!("../../../loops/dnb_two_step.json")),
    ("dubstep_halftime", include_str!("../../../loops/dubstep_halftime.json")),
    ("funk", include_str!("../../../loops/funk.json")),
    ("house_basic", include_str!("../../../loops/house_basic.json")),
    ("jazz_swing", include_str!("../../../loops/jazz_swing.json")),
    ("lofi", include_str!("../../../loops/lofi.json")),
    ("pop_ballad", include_str!("../../../loops/pop_ballad.json")),
    ("reggaeton_dembow", include_str!("../../../loops/reggaeton_dembow.json")),
    ("rock_basic", include_str!("../../../loops/rock_basic.json")),
    ("techno_drive", include_str!("../../../loops/techno_drive.json")),
    ("trap", include_str!("../../../loops/trap.json")),
    ("ukg_2step", include_str!("../../../loops/ukg_2step.json")),
];

/// 列表里的一项：loop 与它来自哪（`builtin` / 用户目录）。
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub item: Loop,
    pub source: &'static str,
}

impl Loop {
    /// 一遍多少拍。
    pub fn beats(&self) -> f64 {
        f64::from(self.bars) * 4.0
    }

    /// 检查：格式、名字、grid 能展开。
    pub fn check(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("不是 loop 文件（format 应为 \"{FORMAT}\"）"));
        }
        crate::poly_library::valid_name(&self.name)
            .then_some(())
            .ok_or_else(|| format!("loop 名 '{}' 不可用：只能用小写字母、数字、_、-", self.name))?;
        if self.bars == 0 {
            return Err("bars 应至少为 1".into());
        }
        if let Some(h) = &self.humanize {
            h.validate("humanize")?;
        }
        self.grid.expand(self.beats()).map(|_| ())
    }

    /// 写进工程的 clip：从 `start` 拍开始，`repeat` 遍（grid 在 clip 长度里循环）。
    pub fn clip(&self, start: f64, repeat: u32) -> Value {
        let mut clip = json!({ "start": start, "length": self.beats() * f64::from(repeat.max(1)), "grid": self.grid, "from": format!("loop:{}", self.name) });
        if let Some(h) = &self.humanize {
            clip["humanize"] = json!(h);
        }
        clip
    }
}

fn parse(text: &str) -> Result<Loop, String> {
    let l: Loop = serde_json::from_str(text).map_err(|e| e.to_string())?;
    l.check().map(|_| l)
}

/// 用户 loop 目录。
pub fn dir() -> PathBuf {
    crate::profile::config_dir().join("loops")
}

fn builtin() -> Vec<Loop> {
    BUILTIN.iter().map(|(name, text)| parse(text).unwrap_or_else(|e| panic!("内置 loop {name}: {e}"))).collect()
}

fn user_in(dir: &Path) -> Vec<Loop> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Loop> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|t| parse(&t))
                .map_err(|e| eprintln!("[muser] 跳过 loop 文件 {}：{e}", p.display()))
                .ok()
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn list_in(dir: &Path) -> Vec<Entry> {
    let user = user_in(dir);
    let mut out: Vec<Entry> = builtin()
        .into_iter()
        .filter(|b| !user.iter().any(|u| u.name == b.name))
        .map(|item| Entry { item, source: "builtin" })
        .collect();
    out.extend(user.into_iter().map(|item| Entry { item, source: "user" }));
    out.sort_by(|a, b| (a.item.style.as_str(), a.item.name.as_str()).cmp(&(b.item.style.as_str(), b.item.name.as_str())));
    out
}

/// 全部 loop（内置 + 用户，同名时用户的优先），按风格、名字排序。
pub fn list() -> Vec<Entry> {
    list_in(&dir())
}

pub fn load(name: &str) -> Result<Loop, String> {
    list()
        .into_iter()
        .find(|e| e.item.name == name.trim())
        .map(|e| e.item)
        .ok_or_else(|| format!("没有叫 '{name}' 的 loop（muser loop list 查看全部）"))
}

/// 保存到用户目录。`overwrite` 为假时同名的用户 loop 已存在则报错（与内置同名会覆盖内置的显示）。
pub fn save(l: &Loop, overwrite: bool) -> Result<PathBuf, String> {
    save_in(&dir(), l, overwrite)
}

fn save_in(dir: &Path, l: &Loop, overwrite: bool) -> Result<PathBuf, String> {
    l.check()?;
    let path = dir.join(format!("{}.json", l.name));
    if path.exists() && !overwrite {
        return Err(format!("loop '{}' 已存在", l.name));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{}.json.tmp", l.name));
    let text = serde_json::to_string_pretty(l).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(path)
}

/// 把一段音符（相对 clip 起点）量化成网格：每个音高一行，`step` 拍一格。偏离网格超过 1/4 格的音报错。
pub fn grid_from_notes(notes: &[muser_core::Note], length: f64, step: f64) -> Result<Grid, String> {
    let steps = (length / step).round().max(1.0) as usize;
    let mut rows: std::collections::BTreeMap<u8, Vec<char>> = std::collections::BTreeMap::new();
    for n in notes {
        let pos = n.start.0 / step;
        if (pos - pos.round()).abs() > 0.25 {
            return Err(format!("第 {:.3} 拍的音不在 {step} 拍的网格上，换更细的 --step 或先量化", n.start.0));
        }
        let k = pos.round() as usize;
        if k >= steps {
            continue;
        }
        let row = rows.entry(n.pitch).or_insert_with(|| vec!['.'; steps]);
        row[k] = if n.accent || n.velocity >= 0.95 {
            'X'
        } else if n.velocity <= 0.5 {
            'o'
        } else {
            'x'
        };
    }
    let name = |p: u8| {
        muser_core::grid::DRUM_PIECES.iter().find(|(_, k)| *k == p).map_or_else(|| p.to_string(), |(a, _)| a.to_string())
    };
    Ok(Grid {
        step: (step != 0.25).then_some(step),
        rows: rows.into_iter().map(|(p, cells)| (name(p), cells.into_iter().collect())).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个内置 loop：能解析、能展开；建议鼓组里有它用到的每个鼓件；按建议速度渲染出声。
    #[test]
    fn builtins_are_sound() {
        let all = builtin();
        assert_eq!(all.len(), BUILTIN.len());
        for l in &all {
            assert_eq!(BUILTIN.iter().find(|(n, _)| *n == l.name).map(|(n, _)| *n), Some(l.name.as_str()), "文件名与 name 一致");
            let kit = l.kit.as_deref().expect("内置 loop 都有建议鼓组");
            let keys = crate::instrument::drum_kit_keys(kit).unwrap_or_else(|| panic!("{}: {kit} 不是鼓组", l.name));
            for row in l.grid.rows.keys() {
                let k = muser_core::grid::drum_key(row).unwrap_or_else(|| panic!("{}: 行 {row} 不是鼓件", l.name));
                assert!(keys.contains(&k), "{}: {kit} 没有鼓件 {row}（{k}）", l.name);
            }
            let tempo = l.tempo.map_or(120.0, |[a, b]| (a + b) / 2.0);
            let mut clip = l.clip(0.0, 1);
            clip["length"] = json!(l.beats().min(4.0));
            let p: muser_core::Project =
                serde_json::from_value(json!({ "tempo": tempo, "tracks": [{ "instrument": kit, "clips": [clip] }] })).unwrap();
            let audio = crate::render(&p, crate::RenderSettings { tail_seconds: 0.1, ..Default::default() }).unwrap();
            assert!(audio.peak() > 0.01, "{} 应出声", l.name);
        }
    }

    #[test]
    fn user_loops_override_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("muser-loops-{}", std::process::id()));
        let mut l = builtin().into_iter().find(|l| l.name == "house_basic").unwrap();
        l.description = "我的版本".into();
        save_in(&dir, &l, false).unwrap();
        assert!(save_in(&dir, &l, false).unwrap_err().contains("已存在"));
        let list = list_in(&dir);
        let house: Vec<_> = list.iter().filter(|e| e.item.name == "house_basic").collect();
        assert_eq!((house.len(), house[0].source, house[0].item.description.as_str()), (1, "user", "我的版本"));
        assert_eq!(list.len(), BUILTIN.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn notes_to_grid_and_back() {
        let l = builtin().into_iter().find(|l| l.name == "funk").unwrap();
        let notes = l.grid.expand(l.beats()).unwrap();
        let g = grid_from_notes(&notes, l.beats(), 0.25).unwrap();
        let back = g.expand(l.beats()).unwrap();
        assert_eq!(back.iter().map(|n| (n.pitch, n.start.0)).collect::<Vec<_>>(), notes.iter().map(|n| (n.pitch, n.start.0)).collect::<Vec<_>>());
        let off = [muser_core::Note { pitch: 36, start: muser_core::Beats(0.1), length: muser_core::Beats(0.1), velocity: 0.8, accent: false }];
        assert!(grid_from_notes(&off, 1.0, 0.25).unwrap_err().contains("网格"));
    }
}
