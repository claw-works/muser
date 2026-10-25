//! muser MCP server —— 把引擎能力暴露为 AI agent 可调用的 tools。
//!
//! 以 stdio 运行，维护内存中的工程会话（`project_id → Project`）。agent 通过
//! 工具创建/编辑/渲染工程，无需直接读写磁盘 JSON，也无需用 shell。
//!
//! 工具与 CLI 同源：都只是 `muser-core` + `muser-engine` 的薄封装，行为一致。

use std::collections::HashMap;
use std::sync::Mutex;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

use muser_core::{Beats, Clip, Note, Project, Track};
use muser_engine::{render_to_wav, RenderSettings};

/// 服务器状态：内存中的工程会话表。
#[derive(Default)]
struct Muser {
    sessions: Mutex<HashMap<String, Project>>,
    next_id: Mutex<u64>,
}

// —— 各工具的入参结构（派生 JsonSchema 以生成 MCP inputSchema）——

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CreateProjectParams {
    /// 工程名（可选）。
    #[serde(default)]
    name: Option<String>,
    /// 速度（每分钟四分音符数），必填且 > 0。
    tempo: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ProjectIdParams {
    /// 工程句柄 id（由 create_project 返回）。
    project_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SetProjectParams {
    project_id: String,
    /// 完整的工程 JSON（与 muser 工程文件格式一致）。
    project: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SetTempoParams {
    project_id: String,
    /// 新的速度，> 0。
    tempo: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AddTrackParams {
    project_id: String,
    /// 轨名（可选）。
    #[serde(default)]
    name: Option<String>,
    /// 语义乐器名（见 list_instruments），如 "drums.808" / "synth.poly"。
    instrument: String,
    /// 轨增益 dB（可选，默认 0）。
    #[serde(default)]
    gain_db: Option<f64>,
    /// 声像 -1~1（可选，默认 0）。
    #[serde(default)]
    pan: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AddClipParams {
    project_id: String,
    /// 目标音轨下标（从 0 起）。
    track_index: usize,
    /// 片段起点（beats，从工程开头算）。
    start: f64,
    /// 片段长度（beats），> 0。
    length: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct NoteSpec {
    /// MIDI 音高 0-127（60 = 中央C；鼓组用 GM 鼓键）。
    pitch: u8,
    /// 音符起点（beats，相对所在 clip 起点）。
    start: f64,
    /// 音符时值（beats），> 0。
    length: f64,
    /// 力度 0-1（可选，默认 0.8）。
    #[serde(default)]
    velocity: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AddNotesParams {
    project_id: String,
    track_index: usize,
    clip_index: usize,
    /// 要加入的音符列表。
    notes: Vec<NoteSpec>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct RenderParams {
    project_id: String,
    /// 输出 WAV 文件路径。
    output_path: String,
    /// 采样率（可选，默认 48000）。
    #[serde(default)]
    sample_rate: Option<u32>,
    /// 结尾余响秒数（可选，默认 2.0）。
    #[serde(default)]
    tail: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct RemoveTrackParams {
    project_id: String,
    /// 要删除的音轨下标。
    track_index: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct UpdateTrackParams {
    project_id: String,
    track_index: usize,
    /// 新增益 dB（可选）。
    #[serde(default)]
    gain_db: Option<f64>,
    /// 新声像 -1~1（可选）。
    #[serde(default)]
    pan: Option<f64>,
    /// 是否静音（可选）。
    #[serde(default)]
    muted: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ClearClipParams {
    project_id: String,
    track_index: usize,
    clip_index: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct NameParams {
    /// 名称，如 "reverb" / "guitar.nylon" / "poly.moog_bass"。
    name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ListInstrumentsParams {
    /// 只列某个类别（可选）：drums / synth / guitar / bass / organ / mallet / strings / wind / choir / poly。
    #[serde(default)]
    family: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct LoadProjectParams {
    /// 磁盘上的工程 JSON 路径。
    path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SaveProjectParams {
    project_id: String,
    /// 落盘路径。
    path: String,
}

/// 工具级错误 → 用户可见的 CallToolResult::error。
fn user_err(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

/// 工具级成功文本。
fn ok_text(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(msg.into())])
}

#[tool_router]
impl Muser {
    fn new() -> Self {
        Self::default()
    }

    /// 取下一个工程 id。
    fn alloc_id(&self) -> String {
        let mut n = self.next_id.lock().unwrap();
        *n += 1;
        format!("proj-{n}")
    }

    /// 对某工程做一次可变操作的辅助：找不到工程则返回错误文本。
    fn with_project<F>(&self, id: &str, f: F) -> Result<CallToolResult, McpError>
    where
        F: FnOnce(&mut Project) -> Result<CallToolResult, CallToolResult>,
    {
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get_mut(id) {
            Some(p) => Ok(f(p).unwrap_or_else(|e| e)),
            None => Ok(user_err(format!("找不到工程 '{id}'"))),
        }
    }

    #[tool(description = "新建一个空工程，返回工程句柄 id。之后用该 id 编辑/渲染。")]
    fn create_project(
        &self,
        Parameters(p): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        if !(p.tempo > 0.0) {
            return Ok(user_err("tempo 必须为正数"));
        }
        let id = self.alloc_id();
        let project = Project::new(p.name.unwrap_or_else(|| id.clone()), p.tempo);
        self.sessions.lock().unwrap().insert(id.clone(), project);
        Ok(ok_text(format!("已创建工程，project_id = {id}")))
    }

    #[tool(description = "取出工程的完整 JSON。")]
    fn get_project(
        &self,
        Parameters(p): Parameters<ProjectIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let sessions = self.sessions.lock().unwrap();
        match sessions.get(&p.project_id) {
            Some(proj) => match proj.to_json() {
                Ok(json) => Ok(ok_text(json)),
                Err(e) => Ok(user_err(format!("序列化失败: {e}"))),
            },
            None => Ok(user_err(format!("找不到工程 '{}'", p.project_id))),
        }
    }

    #[tool(description = "用一份完整的工程 JSON 整体替换当前工程（适合一次性提交完整构思）。")]
    fn set_project(
        &self,
        Parameters(p): Parameters<SetProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let project: Project = match serde_json::from_value(p.project) {
            Ok(proj) => proj,
            Err(e) => return Ok(user_err(format!("工程 JSON 解析失败: {e}"))),
        };
        if let Err(e) = muser_engine::validate(&project) {
            return Ok(user_err(e.to_string()));
        }
        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.contains_key(&p.project_id) {
            return Ok(user_err(format!("找不到工程 '{}'", p.project_id)));
        }
        sessions.insert(p.project_id.clone(), project);
        Ok(ok_text(format!("已替换工程 '{}'", p.project_id)))
    }

    #[tool(description = "校验工程是否合法（tempo/clip/乐器等）。")]
    fn validate_project(
        &self,
        Parameters(p): Parameters<ProjectIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let sessions = self.sessions.lock().unwrap();
        match sessions.get(&p.project_id) {
            Some(proj) => match muser_engine::validate(proj) {
                Ok(()) => Ok(ok_text(format!(
                    "OK: tempo {}, {} 轨, 时长 {:.2} 拍",
                    proj.tempo,
                    proj.tracks.len(),
                    proj.duration_beats().0
                ))),
                Err(e) => Ok(user_err(e.to_string())),
            },
            None => Ok(user_err(format!("找不到工程 '{}'", p.project_id))),
        }
    }

    #[tool(description = "设置工程速度 (tempo)。")]
    fn set_tempo(
        &self,
        Parameters(p): Parameters<SetTempoParams>,
    ) -> Result<CallToolResult, McpError> {
        if !(p.tempo > 0.0) {
            return Ok(user_err("tempo 必须为正数"));
        }
        self.with_project(&p.project_id.clone(), |proj| {
            proj.tempo = p.tempo;
            Ok(ok_text(format!("tempo = {}", p.tempo)))
        })
    }

    #[tool(description = "新增一条音轨，返回其下标 (track_index)。")]
    fn add_track(
        &self,
        Parameters(p): Parameters<AddTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = p.project_id.clone();
        self.with_project(&id, |proj| {
            let mut track = Track::new(p.name.unwrap_or_default(), p.instrument);
            if let Some(g) = p.gain_db {
                track.gain_db = g;
            }
            if let Some(pan) = p.pan {
                track.pan = pan;
            }
            proj.tracks.push(track);
            Ok(ok_text(format!("track_index = {}", proj.tracks.len() - 1)))
        })
    }

    #[tool(description = "在某音轨上新增一个片段 (clip)，返回其下标 (clip_index)。")]
    fn add_clip(
        &self,
        Parameters(p): Parameters<AddClipParams>,
    ) -> Result<CallToolResult, McpError> {
        if !(p.length > 0.0) {
            return Ok(user_err("clip length 必须为正"));
        }
        let id = p.project_id.clone();
        self.with_project(&id, |proj| {
            let track = proj
                .tracks
                .get_mut(p.track_index)
                .ok_or_else(|| user_err(format!("track_index {} 越界", p.track_index)))?;
            track.clips.push(Clip::new(Beats(p.start), Beats(p.length)));
            Ok(ok_text(format!("clip_index = {}", track.clips.len() - 1)))
        })
    }

    #[tool(description = "向某片段加入一批音符。")]
    fn add_notes(
        &self,
        Parameters(p): Parameters<AddNotesParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = p.project_id.clone();
        let (ti, ci) = (p.track_index, p.clip_index);
        let notes = p.notes;
        self.with_project(&id, |proj| {
            let track = proj
                .tracks
                .get_mut(ti)
                .ok_or_else(|| user_err(format!("track_index {ti} 越界")))?;
            let clip = track
                .clips
                .get_mut(ci)
                .ok_or_else(|| user_err(format!("clip_index {ci} 越界")))?;
            let count = notes.len();
            for n in notes {
                let mut note = Note::new(n.pitch, Beats(n.start), Beats(n.length));
                if let Some(v) = n.velocity {
                    note.velocity = v;
                }
                clip.notes.push(note);
            }
            Ok(ok_text(format!("已加入 {count} 个音符")))
        })
    }

    #[tool(description = "把工程离线渲染为 WAV 文件，返回时长与峰值。")]
    fn render(
        &self,
        Parameters(p): Parameters<RenderParams>,
    ) -> Result<CallToolResult, McpError> {
        let sessions = self.sessions.lock().unwrap();
        let proj = match sessions.get(&p.project_id) {
            Some(proj) => proj,
            None => return Ok(user_err(format!("找不到工程 '{}'", p.project_id))),
        };
        let settings = RenderSettings {
            sample_rate: p.sample_rate.unwrap_or(48_000),
            tail_seconds: p.tail.unwrap_or(2.0),
            ..RenderSettings::default()
        };
        match render_to_wav(proj, &p.output_path, settings) {
            Ok((seconds, peak_db)) => Ok(ok_text(format!(
                "已渲染 → {}: {:.2}s, 峰值 {:.1} dBFS",
                p.output_path, seconds, peak_db
            ))),
            Err(e) => Ok(user_err(format!("渲染失败: {e}"))),
        }
    }

    #[tool(description = "删除一条音轨（后续音轨下标前移）。")]
    fn remove_track(
        &self,
        Parameters(p): Parameters<RemoveTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = p.project_id.clone();
        let ti = p.track_index;
        self.with_project(&id, |proj| {
            if ti >= proj.tracks.len() {
                return Err(user_err(format!("track_index {ti} 越界")));
            }
            proj.tracks.remove(ti);
            Ok(ok_text(format!("已删除音轨 {ti}，现有 {} 轨", proj.tracks.len())))
        })
    }

    #[tool(description = "更新音轨属性（增益/声像/静音），只改提供的字段。")]
    fn update_track(
        &self,
        Parameters(p): Parameters<UpdateTrackParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = p.project_id.clone();
        let ti = p.track_index;
        self.with_project(&id, |proj| {
            let track = proj
                .tracks
                .get_mut(ti)
                .ok_or_else(|| user_err(format!("track_index {ti} 越界")))?;
            if let Some(g) = p.gain_db {
                track.gain_db = g;
            }
            if let Some(pan) = p.pan {
                track.pan = pan;
            }
            if let Some(m) = p.muted {
                track.muted = m;
            }
            Ok(ok_text(format!(
                "已更新音轨 {ti}: gain={}dB pan={} muted={}",
                track.gain_db, track.pan, track.muted
            )))
        })
    }

    #[tool(description = "清空某片段内的所有音符（保留片段本身）。")]
    fn clear_clip(
        &self,
        Parameters(p): Parameters<ClearClipParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = p.project_id.clone();
        let (ti, ci) = (p.track_index, p.clip_index);
        self.with_project(&id, |proj| {
            let track = proj
                .tracks
                .get_mut(ti)
                .ok_or_else(|| user_err(format!("track_index {ti} 越界")))?;
            let clip = track
                .clips
                .get_mut(ci)
                .ok_or_else(|| user_err(format!("clip_index {ci} 越界")))?;
            let n = clip.notes.len();
            clip.notes.clear();
            Ok(ok_text(format!("已清空片段，移除 {n} 个音符")))
        })
    }

    #[tool(description = "从磁盘读取一个工程 JSON 到会话，返回新的 project_id。")]
    fn load_project(
        &self,
        Parameters(p): Parameters<LoadProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let text = match std::fs::read_to_string(&p.path) {
            Ok(t) => t,
            Err(e) => return Ok(user_err(format!("读取 '{}' 失败: {e}", p.path))),
        };
        let project = match Project::from_json(&text) {
            Ok(proj) => proj,
            Err(e) => return Ok(user_err(format!("解析工程 JSON 失败: {e}"))),
        };
        if let Err(e) = muser_engine::validate(&project) {
            return Ok(user_err(e.to_string()));
        }
        let id = self.alloc_id();
        self.sessions.lock().unwrap().insert(id.clone(), project);
        Ok(ok_text(format!("已载入，project_id = {id}")))
    }

    #[tool(description = "把会话中的工程写到磁盘 JSON 文件。")]
    fn save_project(
        &self,
        Parameters(p): Parameters<SaveProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let sessions = self.sessions.lock().unwrap();
        let proj = match sessions.get(&p.project_id) {
            Some(proj) => proj,
            None => return Ok(user_err(format!("找不到工程 '{}'", p.project_id))),
        };
        let json = match proj.to_json() {
            Ok(j) => j,
            Err(e) => return Ok(user_err(format!("序列化失败: {e}"))),
        };
        match std::fs::write(&p.path, json) {
            Ok(()) => Ok(ok_text(format!("已保存 → {}", p.path))),
            Err(e) => Ok(user_err(format!("写入 '{}' 失败: {e}", p.path))),
        }
    }

    #[tool(description = "列出可用乐器（名称、类别、音高约定、推荐音域、说明）。可按 family 过滤：drums / synth / guitar / bass / organ / mallet / strings / wind / choir / poly。另可用 plugin:/path/to.vst3 加载第三方 VSTi。")]
    fn list_instruments(
        &self,
        Parameters(p): Parameters<ListInstrumentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let list: Vec<_> = muser_engine::instrument_catalog()
            .into_iter()
            .filter(|i| match &p.family {
                Some(f) => i.family == *f || i.family.starts_with(&format!("{f}/")),
                None => true,
            })
            .collect();
        json_result(&list)
    }

    #[tool(description = "查看某个乐器的详情：类别、音高约定(gm_drums 用 GM 鼓键 / pitched 用 MIDI 音高)、推荐音域、复音数、音色说明。")]
    fn describe_instrument(
        &self,
        Parameters(p): Parameters<NameParams>,
    ) -> Result<CallToolResult, McpError> {
        match muser_engine::describe_instrument(&p.name) {
            Some(info) => json_result(&info),
            None => Ok(user_err(format!(
                "未知乐器 '{}'，用 list_instruments 查看全部",
                p.name
            ))),
        }
    }

    #[tool(description = "列出所有效果器及一句话说明。效果器用于 track.effects（轨级）或 master_effects（主总线），按数组顺序串联。")]
    fn list_effects(&self) -> Result<CallToolResult, McpError> {
        let list: Vec<_> = muser_core::EFFECT_SPECS
            .iter()
            .map(|s| serde_json::json!({ "name": s.name, "summary": s.summary }))
            .collect();
        json_result(&list)
    }

    #[tool(description = "查看某个效果器的完整说明：放置位置、每个参数的范围/默认值/单位/听感、可直接使用的 JSON 示例。")]
    fn describe_effect(
        &self,
        Parameters(p): Parameters<NameParams>,
    ) -> Result<CallToolResult, McpError> {
        match muser_core::effect_spec(&p.name) {
            Some(spec) => json_result(spec),
            None => Ok(user_err(format!(
                "未知效果器 '{}'，用 list_effects 查看全部",
                p.name
            ))),
        }
    }
}

/// 把任意可序列化的值作为 JSON 文本返回。
fn json_result<T: serde::Serialize + ?Sized>(value: &T) -> Result<CallToolResult, McpError> {
    match serde_json::to_string_pretty(value) {
        Ok(s) => Ok(ok_text(s)),
        Err(e) => Ok(user_err(format!("序列化失败: {e}"))),
    }
}

#[tool_handler(
    name = "muser",
    version = "0.1.0",
    instructions = "muser：面向 AI 的无头编曲引擎。用 create_project 建工程得到 project_id，\
再用 add_track / add_clip / add_notes 增量编排（或 set_project 一次性提交完整 JSON），\
最后用 render 离线渲染为 WAV。时间单位是拍(beats)：1.0=四分音符。\
选乐器前先用 list_instruments / describe_instrument 查看（drums.* 用 GM 鼓键：36=底鼓,38=军鼓,42=闭镲；\
其余用 MIDI 音高：60=中央C）。加效果器前用 list_effects / describe_effect 查看参数范围与示例。\
validate_project 会指出越界参数与未知乐器的具体位置。"
)]
impl ServerHandler for Muser {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 以 stdio 运行 MCP server。
    let service = Muser::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
