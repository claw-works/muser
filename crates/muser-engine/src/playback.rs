//! 本机声卡播放：流式渲染的输出直接送进声卡，不先渲染整首。
//!
//! 播放线程跑 [`Renderer`]，把音频写进无锁的环形队列；声卡回调只从队列里取（不加锁、不分配、不阻塞）。
//! 改推子、静音（[`Player::set_mix`]）在播放线程里从下一段起生效，听到的延迟约等于队列长度（几十毫秒）。
//! 跳转像 DAW 一样直接从目标位置开始（[`Renderer::new_at`]）：那时还按着的音补发，效果器从空状态开始；
//! 音轨乐器已在 [`RenderCache`] 里时用缓存的输出。SFZ 音色在进程里只加载一次，重建不用重新读采样。
//!
//! 录音（[`Player::record_start`]）时临时换成双工流（同一个回调里拿到麦克风输入和声卡输出）：
//! 每一块输入对着的工程位置 = 这一块输出的工程位置 − 往返延迟（驱动报告的输入 + 输出延迟，再加用户的微调）。
//! 录完换回只输出的流，麦克风随之释放。[`Player::measure_latency`] 放咔嗒声、听回来，量出驱动没报告的那部分延迟。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use manguebyte::spsc;
use manguebyte::devices::{CallbackInfo, DuplexSettings, DuplexStream, InputDevice, OutputDevice, OutputStream, StreamSettings};
use manguebyte::prelude::AudioBuffer;
use muser_core::Project;

use crate::{RenderCache, RenderSettings, Renderer};

/// 队列长度（帧）：渲染线程最多领先声卡这么多。约 43 ms @ 48 kHz。
const QUEUE: usize = 2048;
/// 现场模式的队列（帧）：约 340 ms @ 48 kHz。改动本来就提前一秒半排好，队列长一些不影响生效时间，却能扛住几百毫秒的卡顿。
const LIVE_QUEUE: usize = 1 << 14;
/// 播放线程里一次渲染、换工程，或两次填队列之间超过这么久，记成慢事件（[`Player::slow_events`]）。
const SLOW_MS: f64 = 15.0;
/// 渲染线程每次推进的帧数。
const CHUNK: usize = 256;
/// 声卡回调最大块长。
const MAX_BLOCK: usize = 4096;
/// 录音输入的队列（帧）：约 1.4 秒 @ 48 kHz，收集线程每 10 ms 取一次。
const CAPTURE_QUEUE: usize = 1 << 16;

enum Cmd {
    Load(Project),
    Mix(Project),
    Play,
    Pause,
    Seek(usize),
    /// 前面的命令都处理完了（跳转的渲染器已建好）就回一声。
    Ack(mpsc::Sender<()>),
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
    /// 预备拍：还剩几帧、一共几帧、一拍几帧、一小节几拍。还剩 > 0 时回调放咔嗒声，放完开始播放。
    count_left: AtomicUsize,
    count_total: AtomicUsize,
    beat_frames: AtomicUsize,
    bar_beats: AtomicUsize,
    /// 录音中：播放时把输入送进收集队列。
    capturing: AtomicBool,
    /// 录到的第一个样本对着工程的第几帧（还没录到时为 i64::MIN）。
    first_frame: AtomicI64,
    /// 往返延迟之外再补的帧数（延迟微调）。
    nudge: AtomicI64,
    /// 驱动报告的往返延迟（帧，测延迟时记下）。
    round_trip: AtomicI64,
    /// 收集队列满了、丢掉输入的次数。
    dropped: AtomicUsize,
    /// 现场模式：正在听到的这一小段各节点的峰值（[`Player::levels`]）。
    levels: Mutex<Vec<(String, f32)>>,
    /// 现场模式：各节点约 1.5 秒的响度（[`Player::loudness`]）。
    loudness: Mutex<Vec<(String, Option<f64>)>>,
    /// 现场模式：最近约一秒里各节点占了多少（一个核的比例，[`Player::loads`]）。
    loads: Mutex<Vec<(String, f32)>>,
    /// 最近的慢事件：(在第几秒, 是什么, 毫秒)。
    slow: Mutex<std::collections::VecDeque<SlowEvent>>,
}

/// 播放线程里慢了的一下（掉帧的嫌疑）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SlowEvent {
    /// 当时听到第几秒。
    pub at_seconds: f64,
    pub what: &'static str,
    pub ms: f64,
}

impl Shared {
    fn slow(&self, what: &'static str, ms: f64, sr: f64) {
        if ms < SLOW_MS {
            return;
        }
        let at_seconds = self.position.load(Ordering::Acquire) as f64 / sr;
        let mut q = self.slow.lock().unwrap();
        q.push_back(SlowEvent { at_seconds, what, ms });
        while q.len() > 20 {
            q.pop_front();
        }
    }
}

/// 一遍录音：从工程第 `start` 帧开始的单声道样本。
pub struct Recording {
    pub start: usize,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// 收集跟不上丢过输入（这一遍可能有断口）。
    pub dropped: bool,
}

enum Stream {
    Out(#[allow(dead_code)] OutputStream),
    Duplex(#[allow(dead_code)] DuplexStream),
}

/// 正在录的一遍：从第几帧起、收集线程、让它停的开关。
struct Rec {
    from: usize,
    stop: Arc<AtomicBool>,
    collector: std::thread::JoinHandle<Vec<f32>>,
}

/// 声卡输出这一半（只输出的流和双工流共用）：从队列取音频，放预备拍。
struct Out {
    sh: Arc<Shared>,
    rx: Arc<Mutex<spsc::Consumer<[f32; 2]>>>,
    scratch: Vec<[f32; 2]>,
    sr: f32,
}

impl Out {
    /// 写这一块；在播放时返回这一块开头对着工程的第几帧。实时安全（锁只用 try_lock，只在换流时才有争用）。
    fn fill(&mut self, buf: &mut AudioBuffer<f32>) -> Option<usize> {
        let sh = &*self.sh;
        let Ok(mut rx) = self.rx.try_lock() else { return None };
        if sh.flush.load(Ordering::Acquire) {
            let n = rx.len();
            rx.skip(n);
            sh.flushed.store(true, Ordering::Release);
            return None;
        }
        let left = sh.count_left.load(Ordering::Acquire);
        if left > 0 {
            let n = buf.num_frames().min(left);
            self.click(buf, n, sh.count_total.load(Ordering::Relaxed) - left);
            sh.count_left.store(left - n, Ordering::Release);
            if left == n {
                sh.playing.store(true, Ordering::Release);
            }
            return None;
        }
        if !sh.playing.load(Ordering::Acquire) {
            return None;
        }
        let n = buf.num_frames().min(MAX_BLOCK);
        let got = rx.pop_slice(&mut self.scratch[..n]);
        let g = f32::from_bits(sh.monitor.load(Ordering::Relaxed));
        if buf.num_channels() == 1 {
            for (o, s) in buf.channel_mut(0).iter_mut().zip(&self.scratch[..got]) {
                *o = 0.5 * (s[0] + s[1]) * g;
            }
        } else {
            // 多于两个声道的设备：只用前两个。
            for c in 0..2 {
                for (o, s) in buf.channel_mut(c).iter_mut().zip(&self.scratch[..got]) {
                    *o = s[c] * g;
                }
            }
        }
        let start = sh.position.fetch_add(got, Ordering::AcqRel);
        if got < n && start + got < sh.total.load(Ordering::Acquire) && !sh.seeking.load(Ordering::Acquire) {
            sh.underruns.fetch_add(1, Ordering::Relaxed);
        }
        Some(start)
    }

    /// 预备拍的咔嗒声：每拍一下，小节头高一点。`at` 是这一块在预备拍里的第几帧。
    fn click(&self, buf: &mut AudioBuffer<f32>, n: usize, at: usize) {
        let (beat, bar) = (self.sh.beat_frames.load(Ordering::Relaxed).max(1), self.sh.bar_beats.load(Ordering::Relaxed).max(1));
        for ch in 0..buf.num_channels() {
            for (j, o) in buf.channel_mut(ch)[..n].iter_mut().enumerate() {
                *o = click_sample(at + j, beat, bar, self.sr);
            }
        }
    }
}

/// 第 `i` 帧的咔嗒声（每 `beat` 帧一下，每 `bar` 拍的第一下高八度）。
fn click_sample(i: usize, beat: usize, bar: usize, sr: f32) -> f32 {
    let within = (i % beat) as f32;
    if within > 0.08 * sr {
        return 0.0;
    }
    let hz = if (i / beat) % bar == 0 { 1320.0 } else { 880.0 };
    0.3 * (-within / (0.015 * sr)).exp() * (std::f32::consts::TAU * hz * within / sr).sin()
}

/// 收集线程：把输入队列取空存进一个 Vec，直到 `stop` 且队列空了。
fn collect(mut rx: spsc::Consumer<f32>, stop: Arc<AtomicBool>) -> std::io::Result<std::thread::JoinHandle<Vec<f32>>> {
    std::thread::Builder::new().name("muser-capture".into()).spawn(move || {
        let mut out = Vec::new();
        let mut buf = vec![0f32; 8192];
        loop {
            let n = rx.pop_slice(&mut buf);
            out.extend_from_slice(&buf[..n]);
            if n == 0 {
                if stop.load(Ordering::Acquire) {
                    return out;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    })
}

/// 驱动报告的往返延迟（帧）；不知道时为 0。
fn round_trip_frames(info: &CallbackInfo) -> i64 {
    info.round_trip_latency().map_or(0, |d| (d.as_secs_f64() * info.sample_rate).round() as i64)
}

/// 测延迟：`input` 是从放第一声前就开始录的输入（与输出逐帧对齐），`clicks` 是每一下放出去的帧。
/// 返回放出去到录回来的帧数（各下的中位数）；听到的少于一半时为 None。
fn click_delay(input: &[f32], clicks: &[usize], window: usize) -> Option<f64> {
    let first = clicks.first().copied().unwrap_or(0);
    let pre = &input[..first.saturating_sub(window / 10).min(input.len())];
    let floor = (pre.iter().map(|x| x * x).sum::<f32>() / pre.len().max(1) as f32).sqrt();
    let mut delays: Vec<f64> = clicks
        .iter()
        .filter_map(|&c| {
            let w = input.get(c..(c + window).min(input.len()))?;
            let peak = w.iter().fold(0f32, |m, x| m.max(x.abs()));
            if peak < (floor * 8.0).max(0.02) {
                return None;
            }
            w.iter().position(|x| x.abs() > peak * 0.3).map(|k| k as f64)
        })
        .collect();
    if delays.len() * 2 < clicks.len() {
        return None;
    }
    delays.sort_by(f64::total_cmp);
    Some(delays[delays.len() / 2])
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
    /// 声卡回调取音频的队列（换流时交给新的回调）。
    rx: Arc<Mutex<spsc::Consumer<[f32; 2]>>>,
    stream: Mutex<Option<Stream>>,
    rec: Mutex<Option<Rec>>,
}

impl Player {
    /// 打开系统默认输出设备。按设备的采样率渲染（`settings.sample_rate` 不用）。
    /// `cache` 里的乐器输出在跳转时复用（由调用方填充，例如浏览器模式的渲染线程）。
    pub fn start(settings: RenderSettings, cache: Arc<Mutex<RenderCache>>) -> Result<Player, String> {
        Self::open(settings, cache, false)
    }

    /// 现场模式（`muser live`）：第一次 [`Player::load`] 从头开始演奏（[`Renderer::live`]），之后每次 load
    /// 都是"往后加了东西"的工程，不停、不重建地换上（[`Renderer::reconcile`]）。不跳转。
    pub fn start_live(settings: RenderSettings) -> Result<Player, String> {
        Self::open(settings, Arc::new(Mutex::new(RenderCache::new())), true)
    }

    fn open(settings: RenderSettings, cache: Arc<Mutex<RenderCache>>, live: bool) -> Result<Player, String> {
        let device = OutputDevice::open_default().map_err(|e| format!("failed to open the audio device: {e}"))?;
        let sample_rate = device.sample_rate().round() as u32;
        let name = device.name().to_string();
        let shared = Arc::new(Shared { monitor: AtomicU32::new(1f32.to_bits()), first_frame: AtomicI64::new(i64::MIN), ..Default::default() });
        let (mut tx, rx) = spsc::channel::<[f32; 2]>(if live { LIVE_QUEUE } else { QUEUE });
        let rx = Arc::new(Mutex::new(rx));
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let settings = RenderSettings { sample_rate, ..settings };
        let sh = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("muser-playback".into())
            .spawn(move || run(settings, cache, sh, cmd_rx, &mut tx, live))
            .map_err(|e| format!("failed to start the playback thread: {e}"))?;
        let player = Player { cmd: cmd_tx, shared, sample_rate, device: name, rx, stream: Mutex::new(None), rec: Mutex::new(None) };
        player.output_only(&device)?;
        Ok(player)
    }

    fn out(&self) -> Out {
        Out { sh: Arc::clone(&self.shared), rx: Arc::clone(&self.rx), scratch: vec![[0f32; 2]; MAX_BLOCK], sr: self.sample_rate as f32 }
    }

    /// 换成只输出的流（启动时；录音、测延迟完以后）。
    fn output_only(&self, device: &OutputDevice) -> Result<(), String> {
        let mut slot = self.stream.lock().unwrap();
        *slot = None;
        let mut out = self.out();
        let stream = device
            .start(MAX_BLOCK, move |buf: &mut AudioBuffer<f32>| {
                out.fill(buf);
            })
            .map_err(|e| format!("failed to start audio output: {e}"))?;
        *slot = Some(Stream::Out(stream));
        Ok(())
    }

    fn restore_output(&self) {
        let r = OutputDevice::open_default().map_err(|e| e.to_string()).and_then(|d| self.output_only(&d));
        if let Err(e) = r {
            *self.shared.error.lock().unwrap() = Some(format!("failed to switch back to audio output: {e}"));
        }
    }

    /// 换成双工流：默认输入（麦克风）+ 默认输出，按播放的采样率。`f` 在声卡线程上拿到这一块的输入、输出、
    /// 时间信息和（播放时）这一块对着工程的第几帧；须实时安全。失败时换回只输出的流。
    fn duplex<F>(&self, mut f: F) -> Result<(), String>
    where
        F: FnMut(&AudioBuffer<f32>, &mut AudioBuffer<f32>, &CallbackInfo, Option<usize>) + Send + 'static,
    {
        let rate = StreamSettings::default().with_sample_rate(self.sample_rate);
        let input = InputDevice::open_default().map_err(|e| format!("failed to open the microphone: {e}"))?;
        let input = if input.sample_rate().round() as u32 == self.sample_rate { input } else { input.with_settings(rate).map_err(|e| format!("the microphone does not support {} Hz: {e}", self.sample_rate))? };
        let output = OutputDevice::open_default().map_err(|e| format!("failed to open the audio device: {e}"))?;
        let output = if output.sample_rate().round() as u32 == self.sample_rate { output } else { output.with_settings(rate).map_err(|e| format!("{e}"))? };
        let mut slot = self.stream.lock().unwrap();
        *slot = None;
        let mut out = self.out();
        let started = DuplexStream::start_timed(
            &input,
            &output,
            DuplexSettings::new(MAX_BLOCK),
            move |i: &AudioBuffer<f32>, o: &mut AudioBuffer<f32>, info: &CallbackInfo| {
                let pos = out.fill(o);
                f(i, o, info, pos);
            },
            |e| eprintln!("[play] audio device: {e}"),
        );
        match started {
            Ok(s) => {
                *slot = Some(Stream::Duplex(s));
                Ok(())
            }
            Err(e) => {
                drop(slot);
                self.restore_output();
                Err(format!("failed to start recording (microphone + audio device): {e}"))
            }
        }
    }

    /// 跳到第 `frame` 帧并等渲染器建好、队列填上（最多约 30 秒）。
    fn seek_and_wait(&self, frame: usize) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let _ = self.cmd.send(Cmd::Seek(frame));
        let _ = self.cmd.send(Cmd::Ack(tx));
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "the seek took too long to finish".to_string())?;
        for _ in 0..500 {
            if !self.shared.seeking.load(Ordering::Acquire) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// 开始录一遍：停下、跳到第 `from` 秒，放 `count_beats` 拍预备拍（每拍 `beat_seconds` 秒，一小节 `bar_beats` 拍），
    /// 然后边放伴奏边录，直到 [`Player::record_stop`]。`nudge_ms`：往返延迟之外再补的毫秒数（正数 = 录音往前挪）。
    pub fn record_start(&self, from: f64, count_beats: usize, beat_seconds: f64, bar_beats: usize, nudge_ms: f64) -> Result<(), String> {
        let mut rec = self.rec.lock().unwrap();
        if rec.is_some() {
            return Err("already recording".into());
        }
        let sr = f64::from(self.sample_rate);
        let from = (from.max(0.0) * sr) as usize;
        self.pause();
        self.seek_and_wait(from)?;
        let sh = &self.shared;
        let (mut cap_tx, cap_rx) = spsc::channel::<f32>(CAPTURE_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let collector = collect(cap_rx, Arc::clone(&stop)).map_err(|e| format!("failed to start the recording thread: {e}"))?;
        sh.first_frame.store(i64::MIN, Ordering::Release);
        sh.nudge.store((nudge_ms / 1000.0 * sr).round() as i64, Ordering::Release);
        sh.dropped.store(0, Ordering::Release);
        sh.capturing.store(true, Ordering::Release);
        let s2 = Arc::clone(sh);
        let started = self.duplex(move |input, _, info, pos| {
            // shortcut: 输出欠载时工程位置少走了几帧、输入照常进来，之后的录音会晚这么多；要精确时按欠载的帧数补齐。
            let (Some(pos), true) = (pos, s2.capturing.load(Ordering::Acquire)) else { return };
            let lat = round_trip_frames(info) + s2.nudge.load(Ordering::Relaxed);
            let _ = s2.first_frame.compare_exchange(i64::MIN, pos as i64 - lat, Ordering::AcqRel, Ordering::Relaxed);
            let x = input.channel(0);
            if cap_tx.push_slice(x) < x.len() {
                s2.dropped.fetch_add(1, Ordering::Relaxed);
            }
        });
        if let Err(e) = started {
            sh.capturing.store(false, Ordering::Release);
            stop.store(true, Ordering::Release);
            return Err(e);
        }
        let beat = (beat_seconds * sr).round().max(1.0) as usize;
        sh.beat_frames.store(beat, Ordering::Relaxed);
        sh.bar_beats.store(bar_beats.max(1), Ordering::Relaxed);
        sh.count_total.store(count_beats * beat, Ordering::Relaxed);
        if count_beats == 0 {
            self.play();
        } else {
            sh.count_left.store(count_beats * beat, Ordering::Release);
        }
        *rec = Some(Rec { from, stop, collector });
        Ok(())
    }

    /// 停止录音：停下播放、换回只输出的流（麦克风释放），返回从开始的位置起对齐好的这一遍。
    pub fn record_stop(&self) -> Result<Recording, String> {
        let r = self.rec.lock().unwrap().take().ok_or("not recording")?;
        let sh = &self.shared;
        sh.capturing.store(false, Ordering::Release);
        sh.count_left.store(0, Ordering::Release);
        self.pause();
        self.restore_output();
        r.stop.store(true, Ordering::Release);
        let mut samples = r.collector.join().map_err(|_| "recording thread error".to_string())?;
        let first = sh.first_frame.load(Ordering::Acquire);
        if first == i64::MIN {
            return Err("nothing was recorded (stopped before the backing started)".into());
        }
        // 从开始录的位置起：之前的样本（往返延迟那一小段）去掉；第一个样本已经晚于开始位置时就从它起。
        let skip = (r.from as i64 - first).max(0) as usize;
        samples.drain(..skip.min(samples.len()));
        Ok(Recording { start: (first.max(r.from as i64)) as usize, samples, sample_rate: self.sample_rate, dropped: sh.dropped.load(Ordering::Relaxed) > 0 })
    }

    /// 测延迟：停下，从声卡放 8 下咔嗒声、麦克风听回来（约 5 秒）。返回驱动报告的往返延迟之外还差几毫秒——
    /// 就是录音时该填的延迟微调。听不清时报错（音量开大、摘下耳机再试）。
    pub fn measure_latency(&self) -> Result<f64, String> {
        if self.rec.lock().unwrap().is_some() {
            return Err("recording in progress".into());
        }
        self.pause();
        let sr = self.sample_rate as usize;
        let (t0, gap) = (sr * 6 / 10, sr / 2);
        let clicks: Vec<usize> = (0..8).map(|k| t0 + k * gap).collect();
        let total = t0 + 8 * gap + sr * 4 / 10;
        let (mut cap_tx, cap_rx) = spsc::channel::<f32>(CAPTURE_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let collector = collect(cap_rx, Arc::clone(&stop)).map_err(|e| format!("failed to start the recording thread: {e}"))?;
        let sh = Arc::clone(&self.shared);
        sh.round_trip.store(i64::MIN, Ordering::Release);
        let marks = clicks.clone();
        let mut n = 0usize;
        let srf = sr as f32;
        self.duplex(move |input, output, info, _| {
            let _ = sh.round_trip.compare_exchange(i64::MIN, round_trip_frames(info), Ordering::AcqRel, Ordering::Relaxed);
            for ch in 0..output.num_channels() {
                for (j, o) in output.channel_mut(ch).iter_mut().enumerate() {
                    let i = n + j;
                    *o = marks.iter().find(|&&c| i >= c && i < c + gap).map_or(0.0, |&c| {
                        let w = (i - c) as f32;
                        if w < 0.02 * srf { 0.9 * (-w / (0.004 * srf)).exp() * (std::f32::consts::TAU * 2000.0 * w / srf).sin() } else { 0.0 }
                    });
                }
            }
            n += output.num_frames();
            let _ = cap_tx.push_slice(input.channel(0));
        })?;
        std::thread::sleep(Duration::from_secs_f64(total as f64 / sr as f64));
        self.restore_output();
        stop.store(true, Ordering::Release);
        let input = collector.join().map_err(|_| "recording thread error".to_string())?;
        let delay = click_delay(&input, &clicks, gap * 8 / 10).ok_or("could not hear the clicks: take off the headphones, turn the volume up, move closer to the microphone and measure again")?;
        let rt = self.shared.round_trip.load(Ordering::Acquire).max(0) as f64;
        Ok(((delay - rt) / sr as f64 * 1000.0).round())
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

    /// 最近的慢事件（最多 20 个，旧的在前）：一次渲染、换工程或播放线程被耽搁超过 15 毫秒。
    pub fn slow_events(&self) -> Vec<SlowEvent> {
        self.shared.slow.lock().unwrap().iter().cloned().collect()
    }

    /// 现场模式下各节点（轨名 / 总线名，主总线是空串）推子后约 1.5 秒的响度（RMS dBFS，没声音时 None）。
    /// 比听到的提前一点（渲染领先播放的那一小段）。
    pub fn loudness(&self) -> Vec<(String, Option<f64>)> {
        self.shared.loudness.lock().unwrap().clone()
    }

    /// 现场模式下最近约一秒各节点的负载（一个核的比例，1.0 = 一整个核），从高到低。
    pub fn loads(&self) -> Vec<(String, f32)> {
        self.shared.loads.lock().unwrap().clone()
    }

    /// 现场模式下正在听到的各节点峰值（线性）：轨名 / 总线名，主总线是空串。
    pub fn levels(&self) -> Vec<(String, f32)> {
        self.shared.levels.lock().unwrap().clone()
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
fn run(settings: RenderSettings, cache: Arc<Mutex<RenderCache>>, sh: Arc<Shared>, cmds: mpsc::Receiver<Cmd>, tx: &mut spsc::Producer<[f32; 2]>, live: bool) {
    let mut project: Option<Project> = None;
    let mut renderer: Option<Renderer> = None;
    // 负载：上次量的时刻与各节点用掉的时间。
    let mut last_cost: Option<(std::time::Instant, HashMap<String, Duration>)> = None;
    // 上次填完队列的时刻（播放中两次之间隔太久 = 播放线程被耽搁）。
    let mut last_fill: Option<std::time::Instant> = None;
    let sr = f64::from(settings.sample_rate);
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
            Some(Cmd::Load(p)) if live => {
                let t0 = std::time::Instant::now();
                let r = match renderer.as_mut() {
                    Some(r) => r.reconcile(&p).map_err(|e| format!("{e:#}")),
                    None => Renderer::live(&p, settings).map(|r| renderer = Some(r.with_meters())).map_err(|e| format!("{e:#}")),
                };
                sh.slow("swapping in the new project", t0.elapsed().as_secs_f64() * 1000.0, sr);
                match r {
                    Ok(()) => {
                        if let Some(r) = &renderer {
                            sh.total.store(r.total_frames(), Ordering::Release);
                        }
                        *sh.error.lock().unwrap() = None;
                    }
                    Err(e) => fail(e),
                }
                project = Some(p);
            }
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
            Some(Cmd::Ack(tx)) => {
                let _ = tx.send(());
            }
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
        if live && busy {
            if let Some(t) = last_fill {
                sh.slow("playback thread delayed", t.elapsed().as_secs_f64() * 1000.0, sr);
            }
        }
        while tx.free_len() >= CHUNK && r.position() < r.total_frames() {
            let t0 = std::time::Instant::now();
            let step = r.process(&mut block);
            if live {
                sh.slow("rendering a block", t0.elapsed().as_secs_f64() * 1000.0, sr);
            }
            match step {
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
        if live && wrote {
            if let Some(r) = &renderer {
                *sh.levels.lock().unwrap() = r.levels(sh.position.load(Ordering::Acquire));
                *sh.loudness.lock().unwrap() = r.loudness_now();
                let now = std::time::Instant::now();
                match &last_cost {
                    Some((t, _)) if now.duration_since(*t) < Duration::from_secs(1) => {}
                    _ => {
                        let costs: HashMap<String, Duration> = r.costs().into_iter().collect();
                        if let Some((t, prev)) = &last_cost {
                            let wall = now.duration_since(*t).as_secs_f32();
                            let mut loads: Vec<(String, f32)> = costs
                                .iter()
                                .map(|(n, c)| (n.clone(), c.saturating_sub(prev.get(n).copied().unwrap_or_default()).as_secs_f32() / wall))
                                .collect();
                            loads.sort_by(|a, b| b.1.total_cmp(&a.1));
                            *sh.loads.lock().unwrap() = loads;
                        }
                        last_cost = Some((now, costs));
                    }
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
        last_fill = busy.then(std::time::Instant::now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_delay_finds_the_echo() {
        // 8 下咔嗒声，每下晚 300 帧录回来，带一点噪声；有一下没录到也行。
        let (sr, gap) = (8000usize, 4000usize);
        let clicks: Vec<usize> = (0..8).map(|k| 4800 + k * gap).collect();
        let mut x: Vec<f32> = (0..40_000).map(|i| ((i * 7919) % 101) as f32 / 101.0 * 0.004 - 0.002).collect();
        for &c in &clicks[1..] {
            for k in 0..80 {
                x[c + 300 + k] += 0.5 * (-(k as f32) / 20.0).exp() * if k % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        assert_eq!(click_delay(&x, &clicks, gap * 8 / 10), Some(300.0));
        assert_eq!(click_delay(&vec![0.0; 40_000], &clicks, gap * 8 / 10), None, "没声音时测不出");
        let _ = sr;
    }

    #[test]
    fn count_in_clicks_accent_the_bar() {
        let sr = 48_000.0;
        let peak = |from: usize| (from..from + 2000).map(|i| click_sample(i, 24_000, 4, sr).abs()).fold(0f32, f32::max);
        assert!(peak(0) > 0.1 && peak(24_000) > 0.1);
        assert_eq!(click_sample(10_000, 24_000, 4, sr), 0.0, "两下之间是静的");
    }
}
