//! 本机声卡播放：流式渲染的输出直接送进声卡，不先渲染整首。
//!
//! 播放线程跑 [`Renderer`]，把音频写进无锁的环形队列；声卡回调只从队列里取（不加锁、不分配、不阻塞）。
//! 改推子、静音（[`Player::set_mix`]）在播放线程里从下一段起生效，听到的延迟约等于队列长度（几十毫秒）。
//! 跳转像 DAW 一样直接从目标位置开始（[`Renderer::new_at`]）：那时还按着的音补发，效果器从空状态开始；
//! 音轨乐器已在 [`RenderCache`] 里时用缓存的输出。SFZ 音色在进程里只加载一次，重建不用重新读采样。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use manguebyte::spsc;
use manguebyte::devices::{OutputDevice, OutputStream};
use manguebyte::prelude::AudioBuffer;
use muser_core::Project;

use crate::{RenderCache, RenderSettings, Renderer};

/// 队列长度（帧）：渲染线程最多领先声卡这么多。约 43 ms @ 48 kHz。
const QUEUE: usize = 2048;
/// 渲染线程每次推进的帧数。
const CHUNK: usize = 256;

enum Cmd {
    Load(Project),
    Mix(Project),
    Play,
    Pause,
    Seek(usize),
}

/// 播放线程与声卡回调共享的状态。
#[derive(Default)]
struct Shared {
    playing: AtomicBool,
    /// 已经送进声卡的帧位置（工程时间）。
    position: AtomicUsize,
    total: AtomicUsize,
    /// 为真时回调丢弃队列里的内容（跳转、换工程），丢完把 `flushed` 置真。
    flush: AtomicBool,
    flushed: AtomicBool,
    /// 播放中队列被取空的次数（渲染跟不上实时）。
    underruns: AtomicUsize,
    /// 监听音量（线性，f32 的位）。
    monitor: AtomicU32,
    /// 正在快进（跳转后还没到目标位置）。
    seeking: AtomicBool,
    error: Mutex<Option<String>>,
}

/// 播放状态快照。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlayerStatus {
    pub playing: bool,
    pub seeking: bool,
    /// 秒。
    pub position: f64,
    pub duration: f64,
    pub underruns: usize,
    pub sample_rate: u32,
    pub device: String,
    pub error: Option<String>,
}

/// 本机声卡播放器。丢弃时停止播放。
pub struct Player {
    cmd: mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
    sample_rate: u32,
    device: String,
    _stream: OutputStream,
}

impl Player {
    /// 打开系统默认输出设备。按设备的采样率渲染（`settings.sample_rate` 不用）。
    /// `cache` 里的乐器输出在跳转时复用（由调用方填充，例如浏览器模式的渲染线程）。
    pub fn start(settings: RenderSettings, cache: Arc<Mutex<RenderCache>>) -> Result<Player, String> {
        let device = OutputDevice::open_default().map_err(|e| format!("打开声卡失败: {e}"))?;
        let sample_rate = device.sample_rate().round() as u32;
        let name = device.name().to_string();
        let shared = Arc::new(Shared { monitor: AtomicU32::new(1f32.to_bits()), ..Default::default() });
        let (mut tx, mut rx) = spsc::channel::<[f32; 2]>(QUEUE);
        let max_block = 4096;
        let mut scratch = vec![[0f32; 2]; max_block];
        let sh = Arc::clone(&shared);
        let stream = device
            .start(max_block, move |buf: &mut AudioBuffer<f32>| {
                if sh.flush.load(Ordering::Acquire) {
                    rx.skip(rx.len());
                    sh.flushed.store(true, Ordering::Release);
                    return;
                }
                if !sh.playing.load(Ordering::Acquire) {
                    return;
                }
                let n = buf.num_frames().min(max_block);
                let got = rx.pop_slice(&mut scratch[..n]);
                let g = f32::from_bits(sh.monitor.load(Ordering::Relaxed));
                if buf.num_channels() == 1 {
                    for (o, s) in buf.channel_mut(0).iter_mut().zip(&scratch[..got]) {
                        *o = 0.5 * (s[0] + s[1]) * g;
                    }
                } else {
                    // 多于两个声道的设备：只用前两个。
                    for c in 0..2 {
                        for (o, s) in buf.channel_mut(c).iter_mut().zip(&scratch[..got]) {
                            *o = s[c] * g;
                        }
                    }
                }
                let pos = sh.position.fetch_add(got, Ordering::AcqRel) + got;
                if got < n && pos < sh.total.load(Ordering::Acquire) && !sh.seeking.load(Ordering::Acquire) {
                    sh.underruns.fetch_add(1, Ordering::Relaxed);
                }
            })
            .map_err(|e| format!("启动声卡输出失败: {e}"))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let settings = RenderSettings { sample_rate, ..settings };
        let sh = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("muser-playback".into())
            .spawn(move || run(settings, cache, sh, cmd_rx, &mut tx))
            .map_err(|e| format!("启动播放线程失败: {e}"))?;
        Ok(Player { cmd: cmd_tx, shared, sample_rate, device: name, _stream: stream })
    }

    /// 换上（新的）工程，保持当前播放位置。
    pub fn load(&self, project: Project) {
        let _ = self.cmd.send(Cmd::Load(project));
    }

    /// 播放中改推子与静音（结构须与 [`Player::load`] 的工程相同；结构变了请重新 load）。
    pub fn set_mix(&self, project: Project) {
        let _ = self.cmd.send(Cmd::Mix(project));
    }

    pub fn play(&self) {
        let _ = self.cmd.send(Cmd::Play);
    }

    pub fn pause(&self) {
        let _ = self.cmd.send(Cmd::Pause);
    }

    /// 跳到第 `seconds` 秒。
    pub fn seek(&self, seconds: f64) {
        let _ = self.cmd.send(Cmd::Seek((seconds.max(0.0) * f64::from(self.sample_rate)) as usize));
    }

    /// 监听音量（dB），只影响本机输出，不进工程。
    pub fn set_monitor_db(&self, db: f64) {
        let g = 10f64.powf(db.clamp(-80.0, 12.0) / 20.0) as f32;
        self.shared.monitor.store(g.to_bits(), Ordering::Relaxed);
    }

    pub fn status(&self) -> PlayerStatus {
        let sr = f64::from(self.sample_rate);
        let sh = &self.shared;
        PlayerStatus {
            playing: sh.playing.load(Ordering::Acquire),
            seeking: sh.seeking.load(Ordering::Acquire),
            position: sh.position.load(Ordering::Acquire) as f64 / sr,
            duration: sh.total.load(Ordering::Acquire) as f64 / sr,
            underruns: sh.underruns.load(Ordering::Relaxed),
            sample_rate: self.sample_rate,
            device: self.device.clone(),
            error: sh.error.lock().unwrap().clone(),
        }
    }
}

/// 让回调丢掉队列里还没播的内容。声卡不在跑时（回调不来）最多等 200 ms。
fn flush(sh: &Shared) {
    sh.flushed.store(false, Ordering::Release);
    sh.flush.store(true, Ordering::Release);
    for _ in 0..200 {
        if sh.flushed.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    sh.flush.store(false, Ordering::Release);
}

/// 播放线程。
fn run(settings: RenderSettings, cache: Arc<Mutex<RenderCache>>, sh: Arc<Shared>, cmds: mpsc::Receiver<Cmd>, tx: &mut spsc::Producer<[f32; 2]>) {
    let mut project: Option<Project> = None;
    let mut renderer: Option<Renderer> = None;
    let mut block = AudioBuffer::<f32>::new(2, CHUNK);
    let mut frames = vec![[0f32; 2]; CHUNK];
    let fail = |e: String| {
        eprintln!("[play] {e}");
        *sh.error.lock().unwrap() = Some(e);
    };
    // 从 `at` 建渲染器（跳转追踪，不从头算）；期间回调输出静音。
    let rebuild = |p: &Project, at: usize| -> Result<Renderer, String> {
        sh.seeking.store(true, Ordering::Release);
        flush(&sh);
        let result = {
            let c = cache.lock().unwrap();
            Renderer::new_at(p, settings, Some(&c), true, at).map_err(|e| format!("{e:#}"))
        };
        match &result {
            // 队列填满后才清除 seeking（见下面的填充），刚跳转时的空队列不算欠载。
            Ok(r) => {
                sh.total.store(r.total_frames(), Ordering::Release);
                sh.position.store(r.position(), Ordering::Release);
            }
            Err(_) => sh.seeking.store(false, Ordering::Release),
        }
        result
    };
    loop {
        // 播放中不等命令；暂停或没有工程时阻塞等。
        let busy = sh.playing.load(Ordering::Acquire) && renderer.as_ref().is_some_and(|r| r.position() < r.total_frames());
        let cmd = if busy {
            match cmds.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        } else {
            match cmds.recv_timeout(Duration::from_millis(50)) {
                Ok(c) => Some(c),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };
        match cmd {
            Some(Cmd::Load(p)) => {
                let at = sh.position.load(Ordering::Acquire);
                match rebuild(&p, at) {
                    Ok(r) => {
                        renderer = Some(r);
                        *sh.error.lock().unwrap() = None;
                    }
                    Err(e) => fail(e),
                }
                project = Some(p);
            }
            Some(Cmd::Mix(p)) => {
                if let Some(r) = renderer.as_mut() {
                    if let Err(e) = r.set_mix(&p) {
                        fail(format!("{e:#}"));
                    }
                }
                project = Some(p);
            }
            Some(Cmd::Play) => {
                // 播到结尾后再播：从头开始。
                if let (Some(r), Some(p)) = (&renderer, &project) {
                    if r.position() >= r.total_frames() && sh.position.load(Ordering::Acquire) >= r.total_frames() {
                        sh.position.store(0, Ordering::Release);
                        match rebuild(p, 0) {
                            Ok(r) => renderer = Some(r),
                            Err(e) => fail(e),
                        }
                    }
                }
                sh.playing.store(true, Ordering::Release);
            }
            Some(Cmd::Pause) => sh.playing.store(false, Ordering::Release),
            Some(Cmd::Seek(at)) => {
                if let Some(p) = &project {
                    sh.position.store(at, Ordering::Release);
                    match rebuild(p, at) {
                        Ok(r) => renderer = Some(r),
                        Err(e) => fail(e),
                    }
                }
            }
            None => {}
        }
        // 把队列填满。
        let Some(r) = renderer.as_mut() else { continue };
        let mut wrote = false;
        while tx.free_len() >= CHUNK && r.position() < r.total_frames() {
            match r.process(&mut block) {
                Ok(0) => break,
                Ok(n) => {
                    for (i, f) in frames[..n].iter_mut().enumerate() {
                        *f = [block.channel(0)[i], block.channel(1)[i]];
                    }
                    tx.push_slice(&frames[..n]);
                    wrote = true;
                }
                Err(e) => {
                    fail(format!("{e:#}"));
                    renderer = None;
                    break;
                }
            }
        }
        if tx.free_len() < CHUNK || renderer.as_ref().is_none_or(|r| r.position() >= r.total_frames()) {
            sh.seeking.store(false, Ordering::Release);
        }
        // 播完了：停在结尾。
        if sh.position.load(Ordering::Acquire) >= sh.total.load(Ordering::Acquire) {
            sh.playing.store(false, Ordering::Release);
        }
        if busy && !wrote {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
