//! 插件音色编辑会话（macOS、Windows）：插件自己的界面放在一个原生窗口里，插件同时接到声卡（能在插件的屏幕键盘上
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
    Play {
        notes: Vec<(u8, f64, f64, f32)>,
        beats: f64,
        tempo: f64,
    },
    Stop,
    /// 存盘并关窗口。
    Done,
    /// 不存盘，关窗口。
    Cancel,
    /// 改窗口标题（换了要载入的音色时）。
    Title {
        title: String,
    },
    /// 把窗口提到最前面（页面上又点了一次"打开"，窗口其实还开着、被挡在后面）。
    Front,
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
        Loop {
            events: Vec<(usize, u8, f32)>,
            len: usize,
        },
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
            .map_err(|e| format!("failed to start audio output: {e}"))
    }
}

/// 装插件界面的原生窗口：建窗口、处理系统事件、跟着插件改大小、改标题、关掉。各平台一份，会话的其余部分共用。
#[cfg(target_os = "macos")]
mod window {
    use manguebyte::host::editor::{EditorSize, ParentWindow};
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};
    use std::time::Instant;

    pub struct Window {
        app: Retained<NSApplication>,
        window: Retained<NSWindow>,
    }

    impl Window {
        pub fn open(title: &str) -> Result<(Self, ParentWindow), String> {
            let mtm = MainThreadMarker::new().ok_or("the plugin editor must be opened on the main thread")?;
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.finishLaunching();
            let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
            // SAFETY: 主线程上新建窗口；关窗口后不释放（还要读它是否可见）。
            let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), rect, style, NSBackingStoreType::Buffered, false) };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(&NSString::from_str(title));
            let view = window.contentView().ok_or("the window has no content view")?;
            let parent = ParentWindow::cocoa(Retained::as_ptr(&view) as usize).ok_or("invalid view")?;
            Ok((Self { app, window }, parent))
        }
        pub fn fit(&self, s: EditorSize) {
            self.window.setContentSize(NSSize::new(f64::from(s.width), f64::from(s.height)));
        }
        /// 第一次显示：居中、提到最前。
        pub fn show(&self) {
            self.window.center();
            self.front();
        }
        pub fn front(&self) {
            self.window.makeKeyAndOrderFront(None);
            #[allow(deprecated)]
            self.app.activateIgnoringOtherApps(true);
        }
        pub fn is_open(&self) -> bool {
            self.window.isVisible()
        }
        /// 处理一轮系统事件（最多等 10 ms）；人在窗口里松开鼠标、按键，记到 `touched`。
        pub fn pump(&self, touched: &mut Option<Instant>) {
            // SAFETY: AppKit 的默认运行循环模式常量。
            let mode = unsafe { NSDefaultRunLoopMode };
            let until = NSDate::dateWithTimeIntervalSinceNow(0.01);
            while let Some(ev) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), mode, true) {
                use objc2_app_kit::NSEventType as T;
                if matches!(ev.r#type(), T::LeftMouseUp | T::RightMouseUp | T::OtherMouseUp | T::KeyUp) {
                    *touched = Some(Instant::now());
                }
                self.app.sendEvent(&ev);
            }
        }
        pub fn set_title(&self, title: &str) {
            self.window.setTitle(&NSString::from_str(title));
        }
        /// 关掉（之后 `is_open` 为假）；插件界面在 [`destroy`](Self::destroy) 之前拆下来。
        pub fn close(&self) {
            self.window.close();
        }
        pub fn destroy(self) {}
    }
}

#[cfg(windows)]
mod window {
    use manguebyte::host::editor::{EditorSize, ParentWindow};
    use std::cell::Cell;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging as w;

    thread_local! {
        /// 窗口被人关掉（点了关闭按钮）：先藏起来，等插件界面拆下来再销毁窗口。
        static CLOSED: Cell<bool> = const { Cell::new(false) };
    }

    const STYLE: w::WINDOW_STYLE = w::WS_OVERLAPPED | w::WS_CAPTION | w::WS_SYSMENU | w::WS_MINIMIZEBOX;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == w::WM_CLOSE {
            CLOSED.with(|c| c.set(true));
            // SAFETY: 本线程建的窗口。
            unsafe { w::ShowWindow(hwnd, w::SW_HIDE) };
            return 0;
        }
        // SAFETY: 其余消息交给系统的默认处理。
        unsafe { w::DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub struct Window {
        hwnd: HWND,
        /// 这个线程的 OLE 是我们初始化的（结束时还原）。
        ole: bool,
    }

    impl Window {
        pub fn open(title: &str) -> Result<(Self, ParentWindow), String> {
            CLOSED.with(|c| c.set(false));
            // 插件界面（拖放、剪贴板）要求所在线程是 OLE 的单线程套间；已经初始化成别的套间就算了。
            // SAFETY: 这个线程上第一次、也是唯一一次初始化，成功了才在 destroy 里还原。
            let ole = unsafe { windows_sys::Win32::System::Ole::OleInitialize(std::ptr::null()) } >= 0;
            let class = wide("muser.plugin.editor");
            let title = wide(title);
            // SAFETY: 标准的 Win32 建窗口流程；字符串在调用期间都活着。重复注册同一个类会失败，不碍事。
            let hwnd = unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let wc = w::WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: instance,
                    hCursor: w::LoadCursorW(std::ptr::null_mut(), w::IDC_ARROW),
                    lpszClassName: class.as_ptr(),
                    ..Default::default()
                };
                w::RegisterClassW(&wc);
                w::CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    STYLE,
                    w::CW_USEDEFAULT,
                    w::CW_USEDEFAULT,
                    800,
                    600,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                )
            };
            let parent = ParentWindow::win32(hwnd as usize).ok_or("cannot create the plugin window")?;
            Ok((Self { hwnd, ole }, parent))
        }
        /// 窗口的客户区正好是插件界面的大小，放在屏幕中间。
        pub fn fit(&self, s: EditorSize) {
            let mut r = RECT { left: 0, top: 0, right: s.width as i32, bottom: s.height as i32 };
            // SAFETY: 本线程的窗口，`r` 活着。
            unsafe {
                w::AdjustWindowRectEx(&mut r, STYLE, 0, 0);
                let (cx, cy) = (r.right - r.left, r.bottom - r.top);
                let (sw, sh) = (w::GetSystemMetrics(w::SM_CXSCREEN), w::GetSystemMetrics(w::SM_CYSCREEN));
                w::SetWindowPos(self.hwnd, std::ptr::null_mut(), ((sw - cx) / 2).max(0), ((sh - cy) / 2).max(0), cx, cy, w::SWP_NOZORDER | w::SWP_NOACTIVATE);
            }
        }
        pub fn show(&self) {
            self.front();
        }
        pub fn front(&self) {
            // SAFETY: 本线程的窗口。
            unsafe {
                w::ShowWindow(self.hwnd, w::SW_SHOWNORMAL);
                w::SetForegroundWindow(self.hwnd);
            }
        }
        pub fn is_open(&self) -> bool {
            !CLOSED.with(Cell::get)
        }
        /// 处理一轮系统消息，再歇 10 ms；人在窗口里（包括插件界面）松开鼠标、按键，记到 `touched`。
        pub fn pump(&self, touched: &mut Option<Instant>) {
            // SAFETY: 标准的消息循环。
            unsafe {
                let mut msg: w::MSG = std::mem::zeroed();
                while w::PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, w::PM_REMOVE) != 0 {
                    if matches!(msg.message, w::WM_LBUTTONUP | w::WM_RBUTTONUP | w::WM_MBUTTONUP | w::WM_KEYUP) {
                        *touched = Some(Instant::now());
                    }
                    w::TranslateMessage(&msg);
                    w::DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        pub fn set_title(&self, title: &str) {
            let t = wide(title);
            // SAFETY: 本线程的窗口，字符串活着。
            unsafe { w::SetWindowTextW(self.hwnd, t.as_ptr()) };
        }
        pub fn close(&self) {
            CLOSED.with(|c| c.set(true));
            // SAFETY: 本线程的窗口。
            unsafe { w::ShowWindow(self.hwnd, w::SW_HIDE) };
        }
        /// 插件界面拆下来以后再销毁窗口。
        pub fn destroy(self) {
            // SAFETY: 本线程的窗口，之后不再用。
            unsafe { w::DestroyWindow(self.hwnd) };
            if self.ole {
                // SAFETY: 与 open 里成功的 OleInitialize 配对。
                unsafe { windows_sys::Win32::System::Ole::OleUninitialize() };
            }
        }
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
    use manguebyte::host::editor::EditorRequest;

    let mut o = Opened::open_spec(spec)?;
    o.apply(params)?;

    // 声卡：插件按声卡的采样率激活，音频半边交给声卡线程。
    #[cfg(feature = "devices")]
    let device = manguebyte::devices::OutputDevice::open_default()
        .map_err(|e| eprintln!("[muser] cannot open the audio device; you will hear nothing while editing: {e}"))
        .ok();
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

    let (window, parent) = window::Window::open(title)?;
    let size = if let Some(p) = o.plugin.as_vst3_mut() {
        p.open_editor(parent, None).map_err(|e| format!("cannot open the plugin editor: {e:?}"))?
    } else if let Some(p) = o.plugin.as_clap_mut() {
        p.open_editor(parent, None).map_err(|e| format!("cannot open the plugin editor: {e:?}"))?
    } else {
        return Err("only VST3 / CLAP plugin editors are supported".into());
    };
    window.fit(size);
    window.show();

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
    while window.is_open() {
        window.pump(&mut touched);
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
                        window.fit(s);
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
                Cmd::Title { title } => window.set_title(&title),
                Cmd::Front => window.front(),
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
            let data = o.plugin.save_state().map_err(|e| format!("the plugin failed to save its state: {e:?}"))?;
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
    window.destroy();
    Ok(if saved && !cancel { Outcome::Saved(read_state_file(out)?.0) } else { Outcome::Unchanged })
}
