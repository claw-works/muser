//! 插件音色编辑会话（macOS）：插件自己的界面放在一个原生窗口里，插件同时接到声卡（能在插件的屏幕键盘上
//! 弹、能循环试听一段音符），状态一有变化就自动存盘。窗口关闭即结束（保存）。
//!
//! 由 `muser plugin edit` 运行；`muser view` 以子进程方式启动它，经 [`Cmd`] 驱动、经 `on_saved` 得知同步。

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::plugin::{read_state_file, write_state_file, Opened, StateHeader};

/// 会话命令。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    /// 循环试听：`notes` 为 `[音高, 起点拍, 时值拍, 力度]`，`beats` 拍一圈。
    Play { notes: Vec<(u8, f64, f64, f32)>, beats: f64, tempo: f64 },
    Stop,
    /// 存盘并关窗口。
    Done,
    /// 不存盘，关窗口。
    Cancel,
}

/// 会话怎么结束的。
pub enum Outcome {
    /// 存过盘（自动同步或结束时）。
    Saved(StateHeader),
    /// 没有任何改动，或放弃了。
    Unchanged,
}

/// 最后一次操作之后多久存盘。插件的状态没法逐字节比较（Kontakt 每次保存都写进新的随机 id），
/// 所以以"有人动过"为准：在窗口里松开鼠标、按键，或插件报告改了参数 / 状态。
const SYNC_AFTER: Duration = Duration::from_millis(1500);

/// 声卡上的循环试听（音频线程里排音符）。
#[cfg(feature = "devices")]
mod audition {
    use manguebyte::prelude::{AudioBuffer, Event, EventBuffer, EventKind, NoteRef, ProcessContext, Processor};
    use std::sync::mpsc;

    pub enum ToAudio {
        /// (帧, 音高, 力度；力度 < 0 为松键)，按帧排序；一圈 `len` 帧。
        Loop { events: Vec<(usize, u8, f32)>, len: usize },
        Stop,
    }

    pub fn start(
        mut proc: Box<dyn Processor<f32> + Send>,
        device: manguebyte::devices::OutputDevice,
        rx: mpsc::Receiver<ToAudio>,
        mut transport: manguebyte::prelude::Transport,
    ) -> Result<manguebyte::devices::OutputStream, String> {
        const MAX: usize = 4096;
        let sr = device.sample_rate();
        let mut scratch = AudioBuffer::<f32>::new(2, MAX);
        let mut queue = EventBuffer::with_capacity(512);
        let (mut events, mut len, mut pos) = (Vec::<(usize, u8, f32)>::new(), 0usize, 0usize);
        let mut held = [false; 128];
        device
            .start(MAX, move |buf: &mut AudioBuffer<f32>| {
                let n = buf.num_frames().min(MAX);
                queue.clear();
                let release = |queue: &mut EventBuffer, held: &mut [bool; 128]| {
                    for (k, h) in held.iter_mut().enumerate() {
                        if std::mem::take(h) {
                            let _ = queue.push(Event { offset: 0, kind: EventKind::NoteOff { note: NoteRef::new(0, k as u8), velocity: 0.0 } });
                        }
                    }
                };
                while let Ok(m) = rx.try_recv() {
                    release(&mut queue, &mut held);
                    (events, len, pos) = match m {
                        ToAudio::Loop { events, len } => (events, len, 0),
                        ToAudio::Stop => (Vec::new(), 0, 0),
                    };
                }
                if len > 0 {
                    // 这一块里到点的音符（跨圈时接着从头排）。
                    let mut at = 0;
                    while at < n {
                        let span = (len - pos).min(n - at);
                        for &(f, key, vel) in events.iter().filter(|e| e.0 >= pos && e.0 < pos + span) {
                            let offset = (at + f - pos) as u32;
                            let note = NoteRef::new(0, key);
                            let kind = if vel < 0.0 { EventKind::NoteOff { note, velocity: 0.0 } } else { EventKind::NoteOn { note, velocity: vel } };
                            held[usize::from(key & 0x7F)] = vel >= 0.0;
                            let _ = queue.push(Event { offset, kind });
                        }
                        at += span;
                        pos = (pos + span) % len;
                    }
                }
                scratch.set_num_frames(n);
                for c in 0..2 {
                    scratch.channel_mut(c).fill(0.0);
                }
                proc.process_with(&mut scratch, &ProcessContext::new(&queue, &transport));
                transport.advance(n, sr);
                for c in 0..buf.num_channels() {
                    buf.channel_mut(c)[..n].copy_from_slice(&scratch.channel(c.min(1))[..n]);
                }
            })
            .map_err(|e| format!("启动声卡输出失败: {e}"))
    }
}

/// 运行一次编辑会话，直到窗口关闭或收到 [`Cmd::Done`] / [`Cmd::Cancel`]。必须在主线程调用。
/// `on_saved` 在每次存盘（自动同步、结束）后调用。
pub fn run(
    spec: &str,
    params: &serde_json::Map<String, Value>,
    out: &Path,
    title: &str,
    cmds: mpsc::Receiver<Cmd>,
    mut on_saved: impl FnMut(),
) -> Result<Outcome, String> {
    use manguebyte::host::editor::{EditorRequest, EditorSize, ParentWindow};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().ok_or("插件界面须在主线程打开")?;
    let mut o = Opened::open_spec(spec)?;
    o.apply(params)?;

    // 声卡：插件按声卡的采样率激活，音频半边交给声卡线程。
    #[cfg(feature = "devices")]
    let device = manguebyte::devices::OutputDevice::open_default().map_err(|e| eprintln!("[muser] 打不开声卡，编辑时听不到声音: {e}")).ok();
    #[cfg(feature = "devices")]
    let sr = device.as_ref().map_or(48_000.0, |d| d.sample_rate());
    #[cfg(not(feature = "devices"))]
    let sr = 48_000.0;
    let mut proc = o.activate(sr, 4096)?;
    proc.prepare(&manguebyte::prelude::ProcessSpec { sample_rate: sr, max_block_size: 4096, num_channels: 2 });
    #[cfg(feature = "devices")]
    let (to_audio, stream) = {
        let (tx, rx) = mpsc::channel();
        let stream = match device {
            Some(d) => Some(audition::start(proc, d, rx, crate::transport::start())?),
            None => {
                drop(proc);
                None
            }
        };
        (tx, stream)
    };
    #[cfg(not(feature = "devices"))]
    let _proc = proc;

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
    // SAFETY: 主线程上新建窗口；关窗口后不释放（还要读它是否可见）。
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), rect, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(title));
    let view = window.contentView().ok_or("窗口没有内容视图")?;
    let parent = ParentWindow::cocoa(objc2::rc::Retained::as_ptr(&view) as usize).ok_or("无效的视图")?;
    let size = if let Some(p) = o.plugin.as_vst3_mut() {
        p.open_editor(parent, None).map_err(|e| format!("插件界面打不开: {e:?}"))?
    } else if let Some(p) = o.plugin.as_clap_mut() {
        p.open_editor(parent, None).map_err(|e| format!("插件界面打不开: {e:?}"))?
    } else {
        return Err("只支持 VST3 / CLAP 插件的界面".into());
    };
    let fit = |s: EditorSize| window.setContentSize(NSSize::new(f64::from(s.width), f64::from(s.height)));
    fit(size);
    window.center();
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    let header = || {
        let e = o.plugin.entry();
        StateHeader { plugin_id: e.id.clone(), plugin_name: e.name.clone(), version: e.version.clone(), format: format!("{:?}", e.format).to_lowercase() }
    };
    let header = header();
    let mut saved = false;
    // 最后一次改动的时间（存盘后清空）。
    let mut touched: Option<Instant> = None;
    // 刚载入时插件自己报的改动（载入音色、重建参数表）不算。
    let settled = Instant::now() + Duration::from_secs(3);
    let mut cancel = false;
    while window.isVisible() {
        // SAFETY: AppKit 的默认运行循环模式常量。
        let mode = unsafe { NSDefaultRunLoopMode };
        let until = NSDate::dateWithTimeIntervalSinceNow(0.01);
        while let Some(ev) = app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), mode, true) {
            use objc2_app_kit::NSEventType as T;
            if matches!(ev.r#type(), T::LeftMouseUp | T::RightMouseUp | T::OtherMouseUp | T::KeyUp) {
                touched = Some(Instant::now());
            }
            app.sendEvent(&ev);
        }
        // 插件界面的请求：改窗口大小、自己关掉。
        let mut requests = Vec::new();
        if let Some(p) = o.plugin.as_vst3_mut() {
            use manguebyte::host::vst3::HostEvent as E;
            requests.extend(p.pump().into_iter().filter_map(|e| match e {
                E::Editor(r) => Some(r),
                E::GestureEnd { .. } | E::StateDirty | E::ParamsRescanned { .. } => {
                    touched = (Instant::now() > settled).then(Instant::now).or(touched);
                    None
                }
                _ => None,
            }));
        } else if let Some(p) = o.plugin.as_clap_mut() {
            use manguebyte::host::clap::HostEvent as E;
            requests.extend(p.pump().into_iter().filter_map(|e| match e {
                E::Editor(r) => Some(r),
                E::GestureEnd { .. } | E::StateDirty | E::ParamsRescanned(_) => {
                    touched = (Instant::now() > settled).then(Instant::now).or(touched);
                    None
                }
                _ => None,
            }));
        }
        for r in requests {
            match r {
                EditorRequest::Resize(want) => {
                    let got = if let Some(p) = o.plugin.as_vst3_mut() {
                        p.set_editor_size(want).ok()
                    } else {
                        o.plugin.as_clap_mut().and_then(|p| p.set_editor_size(want).ok())
                    };
                    if let Some(s) = got {
                        fit(s);
                    }
                }
                EditorRequest::Closed => window.close(),
                _ => {}
            }
        }
        while let Ok(c) = cmds.try_recv() {
            match c {
                Cmd::Play { notes, beats, tempo } => {
                    #[cfg(feature = "devices")]
                    {
                        let frame = |b: f64| (b * 60.0 / tempo * sr).round() as usize;
                        let len = frame(beats).max(1);
                        let mut ev: Vec<(usize, u8, f32)> = Vec::new();
                        for (key, start, length, vel) in notes {
                            let (a, z) = (frame(start), frame(start + length).min(len - 1));
                            if a < len && z > a {
                                ev.push((a, key, vel.clamp(0.01, 1.0)));
                                ev.push((z, key, -1.0));
                            }
                        }
                        // 同一帧先松键再按键。
                        ev.sort_by(|x, y| x.0.cmp(&y.0).then((x.2 >= 0.0).cmp(&(y.2 >= 0.0))));
                        let _ = to_audio.send(audition::ToAudio::Loop { events: ev, len });
                    }
                    #[cfg(not(feature = "devices"))]
                    let _ = (notes, beats, tempo);
                }
                Cmd::Stop => {
                    #[cfg(feature = "devices")]
                    let _ = to_audio.send(audition::ToAudio::Stop);
                }
                Cmd::Done => window.close(),
                Cmd::Cancel => {
                    cancel = true;
                    window.close();
                }
            }
        }
        if !cancel && touched.is_some_and(|t| t.elapsed() >= SYNC_AFTER) {
            touched = None;
            if let Ok(data) = o.plugin.save_state() {
                write_state_file(out, &header, &data)?;
                saved = true;
                on_saved();
            }
        }
    }
    #[cfg(feature = "devices")]
    drop(stream);
    if !cancel {
        // 还没存的改动；换了插件（状态文件还不存在）时也要存一份初始音色。
        if touched.is_some() || (!saved && !out.is_file()) {
            let data = o.plugin.save_state().map_err(|e| format!("插件保存状态失败: {e:?}"))?;
            write_state_file(out, &header, &data)?;
            saved = true;
            on_saved();
        }
    }
    if let Some(p) = o.plugin.as_vst3_mut() {
        p.close_editor();
    } else if let Some(p) = o.plugin.as_clap_mut() {
        p.close_editor();
    }
    Ok(if saved && !cancel { Outcome::Saved(read_state_file(out)?.0) } else { Outcome::Unchanged })
}
