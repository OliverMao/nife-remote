//! 控制端的麦克风（协议见 `nife_remote_proto::session` 的"麦克风"）：把控制端发来的声音放进本机的虚拟麦克风，
//! 本机的程序（会议、语音输入）选它做录音设备，就像用着一个本地麦克风。
//!
//! Windows 用 Steam 的虚拟声卡 Steam Streaming Microphone（装了 Steam、用过一次远程同乐就有，见 [`wasapi`]）。
//! Nife Remote 不带、也不安装这个驱动，只用已经装好的设备：没有它时控制端看到的是 `MicState::NoDevice`。
//! 其它平台还不支持（`MicState::Unsupported`）。
//!
//! * 同一时刻只给一个会话（[`open`]）：别的会话这时看到的是 `Busy`，空出来之后告诉它们（[`owner_changed`]）；
//! * 控制端打开麦克风时才打开设备、开始播放线程，关掉或者会话结束时（丢掉 [`MicLease`]）停止：不一直占着设备；
//! * 声音包在会话的读任务里交给 [`MicInput`]（不能等），经有界队列交给播放线程：解码 Opus（丢了的包用 Opus 的
//!   丢包补偿盖过去），抖动缓冲（[`Playout`]）攒够 [`TARGET_MS`] 再开始放，换成设备的格式（[`Format`]）写进去。
//!   没有声音的时候写静音；网络卡住、积压太多时丢掉旧的，延迟不会越积越多。

pub(crate) mod wasapi;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use nife_remote_link::host::MicSink;
use nife_remote_proto::media::{self, AUDIO_FRAME_MS, AUDIO_SAMPLE_RATE, AudioHeader};
use nife_remote_proto::session::MicState;
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// 每包的采样数（48 kHz 单声道 20 ms = 960）
const FRAME: usize = (AUDIO_SAMPLE_RATE / 1000 * AUDIO_FRAME_MS) as usize;
/// 抖动缓冲：攒够这么多再开始放（网络偶尔晚到一两包盖得住）。语音输入、开会都不在乎这点延迟
pub const TARGET_MS: usize = 60;
const TARGET: usize = AUDIO_SAMPLE_RATE as usize / 1000 * TARGET_MS;
/// 积压超过这么多（网络卡了一阵之后一下子到了很多）就丢掉旧的，只留 [`TARGET_MS`]
const MAX: usize = AUDIO_SAMPLE_RATE as usize / 1000 * 200;
/// 连着丢了不超过这么多包时用 Opus 的丢包补偿补上；丢得更多就不补了（补出来的也只是噪音）
const MAX_CONCEAL: u32 = 3;
/// 读任务交给播放线程的包最多排这么多个（约 640 ms），播放线程跟不上（几乎不会）就丢
const QUEUE: usize = 32;
/// 打不开设备时隔这么久再试
const REOPEN: Duration = Duration::from_secs(1);
/// 设备里最多先写这么多：够播放线程两次醒来之间用（Windows 的计时器可能要 15.6 ms 才叫醒它）。
/// 不能把设备的缓冲填满：那会把抖动缓冲里的一下子取光，包稍晚一点就断粮（这台电脑上设备的缓冲是 80 ms）
const AHEAD_MS: usize = 40;

/// 告诉控制端的状态：状态和给排查问题用的技术信息（英文）
pub type Status = (MicState, Option<String>);

// ───────────────────────────── 会话的入口 ─────────────────────────────

/// 读任务交过来的声音包放进这里（[`MicSink`]）：会话一开始就有，打开了麦克风（有 [`MicLease`]）才交给播放线程，
/// 否则丢掉
#[derive(Default)]
pub struct MicInput {
    tx: Mutex<Option<SyncSender<Vec<u8>>>>,
}

impl MicSink for MicInput {
    fn packet(&self, packet: Vec<u8>) {
        if let Some(tx) = &*self.tx.lock().unwrap() {
            let _ = tx.try_send(packet);
        }
    }
}

/// 现在是哪个会话在用麦克风
static OWNER: LazyLock<watch::Sender<Option<u64>>> = LazyLock::new(|| watch::Sender::new(None));
/// 上一个播放线程：新的播放线程等它放开设备再开始（独占模式下设备同时只能打开一次）
static LAST: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// 用麦克风的会话变了（别的会话看到的 `Busy` 可能变成能用了）
pub fn owner_changed() -> watch::Receiver<Option<u64>> {
    OWNER.subscribe()
}

/// 会话 `session_id` 现在能不能用麦克风（不打开设备）：会话开始时、空出来之后告诉控制端。
/// 要列举声音设备，放在阻塞线程里
pub async fn probe(session_id: u64) -> MicState {
    if OWNER.borrow().is_some_and(|o| o != session_id) {
        return MicState::Busy;
    }
    {
        tokio::task::spawn_blocking(wasapi::probe)
            .await
            .unwrap_or(MicState::Failed)
    }
}

/// 控制端打开了麦克风：占用麦克风，开始播放线程。别的会话占着时为 `Err(Busy)`，这个平台不支持时为
/// `Err(Unsupported)`。打开设备在播放线程里做，结果从 [`MicLease::changed`] 来
pub fn open(session_id: u64, input: &Arc<MicInput>) -> Result<MicLease, MicState> {
    if !cfg!(windows) {
        return Err(MicState::Unsupported);
    }
    let mut mine = false;
    OWNER.send_if_modified(|o| match *o {
        None => {
            *o = Some(session_id);
            mine = true;
            true
        }
        Some(other) => {
            mine = other == session_id;
            false
        }
    });
    if !mine {
        return Err(MicState::Busy);
    }
    let (tx, rx) = sync_channel(QUEUE);
    *input.tx.lock().unwrap() = Some(tx);
    let stop = Arc::new(AtomicBool::new(false));
    let (status_tx, status) = watch::channel((MicState::Ready, None));
    let mut last = LAST.lock().unwrap();
    let previous = last.take();
    let thread_stop = stop.clone();
    let spawned = std::thread::Builder::new()
        .name("nife-remote-mic".into())
        .spawn(move || {
            if let Some(p) = previous {
                let _ = p.join();
            }
            render(rx, &thread_stop, &status_tx, wasapi::open);
        });
    match spawned {
        Ok(t) => *last = Some(t),
        Err(e) => warn!("can't start the microphone thread: {e}"),
    }
    info!(session_id, "the controller's microphone is on");
    Ok(MicLease {
        session_id,
        input: input.clone(),
        stop,
        status,
    })
}

/// 一个会话在用麦克风（[`open`]）。丢掉时停止播放、放开设备，别的会话就能用了
pub struct MicLease {
    session_id: u64,
    input: Arc<MicInput>,
    stop: Arc<AtomicBool>,
    status: watch::Receiver<Status>,
}

impl MicLease {
    /// 现在的状态
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// 等到状态有变化（打开了设备、打不开、设备没了……），返回新的状态（取消安全）
    pub async fn changed(&mut self) -> Status {
        // 播放线程退出了（不应该发生）：不会再变
        if self.status.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        self.status.borrow_and_update().clone()
    }
}

impl Drop for MicLease {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        *self.input.tx.lock().unwrap() = None;
        let id = self.session_id;
        OWNER.send_if_modified(|o| {
            let mine = *o == Some(id);
            if mine {
                *o = None;
            }
            mine
        });
        info!(session_id = id, "the controller's microphone is off");
    }
}

// ───────────────────────────── 播放 ─────────────────────────────

/// 采样的格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sample {
    F32,
    I16,
    /// 3 字节紧挨着
    I24,
    /// 32 位整数（包括 32 位容器里的 24 位：有效位在高位，按 32 位写就对了）
    I32,
}

impl Sample {
    fn size(self) -> usize {
        match self {
            Sample::I16 => 2,
            Sample::I24 => 3,
            Sample::F32 | Sample::I32 => 4,
        }
    }
}

/// 写进设备的格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Format {
    pub sample: Sample,
    pub channels: usize,
    pub rate: u32,
    /// 一帧多少字节（可能比 声道数 × 采样大小 多出几个填充字节）
    pub block_align: usize,
}

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

impl Format {
    /// 解析 `WAVEFORMATEX` / `WAVEFORMATEXTENSIBLE` 的字节（Windows 设备属性里的设备格式就是这个）
    pub fn parse(b: &[u8]) -> Result<Self> {
        let u16_at = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
        let u32_at = |i: usize| {
            b.get(i..i + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        };
        let (Some(tag), Some(channels), Some(rate), Some(block_align), Some(bits)) =
            (u16_at(0), u16_at(2), u32_at(4), u16_at(12), u16_at(14))
        else {
            bail!("the wave format is too short ({} bytes)", b.len());
        };
        let tag = match tag {
            // 扩展格式：真正的格式是 SubFormat（一个 GUID，前 4 个字节就是 WAVE_FORMAT_*）
            WAVE_FORMAT_EXTENSIBLE => match u32_at(24) {
                Some(sub) => sub as u16,
                None => bail!("the extensible wave format is too short"),
            },
            t => t,
        };
        let sample = match (tag, bits) {
            (WAVE_FORMAT_IEEE_FLOAT, 32) => Sample::F32,
            (WAVE_FORMAT_PCM, 16) => Sample::I16,
            (WAVE_FORMAT_PCM, 24) => Sample::I24,
            (WAVE_FORMAT_PCM, 32) => Sample::I32,
            _ => bail!("unsupported wave format (tag {tag:#x}, {bits} bits)"),
        };
        let (channels, block_align) = (channels as usize, block_align as usize);
        if channels == 0 || rate == 0 || block_align < channels * sample.size() {
            bail!("invalid wave format ({channels} channels, {rate} Hz, block {block_align})");
        }
        Ok(Self {
            sample,
            channels,
            rate,
            block_align,
        })
    }

    /// 把一帧（`frame.len() == block_align`）写成 `v`（每个声道一样，填充字节为 0）
    fn write(&self, frame: &mut [u8], v: f32) {
        let v = v.clamp(-1.0, 1.0);
        frame.fill(0);
        let size = self.sample.size();
        for c in 0..self.channels {
            let s = &mut frame[c * size..(c + 1) * size];
            match self.sample {
                Sample::F32 => s.copy_from_slice(&v.to_le_bytes()),
                Sample::I16 => s.copy_from_slice(&((v * 32767.0) as i16).to_le_bytes()),
                Sample::I24 => s.copy_from_slice(&((v * 8_388_607.0) as i32).to_le_bytes()[..3]),
                Sample::I32 => {
                    s.copy_from_slice(&((v as f64 * 2_147_483_647.0) as i32).to_le_bytes())
                }
            }
        }
    }
}

/// 收到的声音：解码、补上丢了的包、抖动缓冲（48 kHz 单声道）
pub(crate) struct Playout {
    decoder: opus::Decoder,
    queue: VecDeque<f32>,
    /// 解码过的最后一包的序号
    last: Option<u32>,
    /// 在攒数据（刚开始，或者断粮了）：攒够 [`TARGET`] 之前放静音
    buffering: bool,
    pcm: Vec<f32>,
}

impl Playout {
    pub fn new() -> Result<Self> {
        Ok(Self {
            decoder: opus::Decoder::new(AUDIO_SAMPLE_RATE, opus::Channels::Mono)?,
            queue: VecDeque::with_capacity(MAX + FRAME * 4),
            last: None,
            buffering: true,
            // 一个 Opus 包最长 120 ms
            pcm: vec![0.0; AUDIO_SAMPLE_RATE as usize / 1000 * 120],
        })
    }

    /// 一个声音包（头 + Opus 包，已经按先后排好、去掉了副本）。前面丢了几包的话先用丢包补偿补上
    pub fn push(&mut self, packet: &[u8]) {
        let (Some(h), Some(data)) = (AudioHeader::decode(packet), media::payload(packet)) else {
            return;
        };
        // 停顿之后的第一包（控制端刚打开麦克风）前面没有丢什么
        if let Some(last) = self.last
            && !h.start
        {
            let lost = h.seq.wrapping_sub(last).wrapping_sub(1);
            if (1..=MAX_CONCEAL).contains(&lost) {
                for _ in 0..lost {
                    self.decode(&[]);
                }
            }
        }
        self.decode(data);
        self.last = Some(h.seq);
        if self.queue.len() > MAX {
            let extra = self.queue.len() - TARGET;
            self.queue.drain(..extra);
            debug!(
                dropped_ms = extra / 48,
                "the microphone fell behind, skipping ahead"
            );
        }
    }

    /// 解一包（空的：丢包补偿，补一包的长度）
    fn decode(&mut self, data: &[u8]) {
        let out = if data.is_empty() {
            &mut self.pcm[..FRAME]
        } else {
            &mut self.pcm[..]
        };
        match self.decoder.decode_float(data, out, false) {
            Ok(n) => self.queue.extend(&self.pcm[..n]),
            Err(e) => debug!("can't decode a microphone packet: {e}"),
        }
    }

    /// 下一个采样（48 kHz）：攒够之前、断粮的时候是静音
    pub fn next(&mut self) -> f32 {
        if self.buffering {
            if self.queue.len() < TARGET {
                return 0.0;
            }
            self.buffering = false;
        }
        match self.queue.pop_front() {
            Some(v) => v,
            None => {
                debug!("the microphone ran dry, buffering again");
                self.buffering = true;
                0.0
            }
        }
    }
}

/// 48 kHz → 设备的采样率（线性插值：说话的声音够用了），一个一个地往外取
pub(crate) struct Resampler {
    /// 每个输出采样在输入上走多远
    step: f64,
    /// 下一个输出在 `prev` 和 `cur` 之间的位置（≥ 1 时要再取一个输入）
    pos: f64,
    prev: f32,
    cur: f32,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        Self {
            step: AUDIO_SAMPLE_RATE as f64 / rate as f64,
            pos: 1.0,
            prev: 0.0,
            cur: 0.0,
        }
    }

    pub fn next(&mut self, input: &mut impl FnMut() -> f32) -> f32 {
        while self.pos >= 1.0 {
            self.prev = self.cur;
            self.cur = input();
            self.pos -= 1.0;
        }
        let v = self.prev + (self.cur - self.prev) * self.pos as f32;
        self.pos += self.step;
        v
    }
}

/// 虚拟麦克风的播放端（各平台实现）
pub(crate) trait Output {
    /// 写进去的格式
    fn format(&self) -> Format;
    /// 设备里还排着多少帧没放、一共放得下多少帧
    fn buffered(&mut self) -> Result<(usize, usize)>;
    /// 写 `frames` 帧：`fill` 按 [`Output::format`] 填好这些字节
    fn write(&mut self, frames: usize, fill: &mut dyn FnMut(&mut [u8])) -> Result<()>;
}

/// 打开播放端：`Ok(None)` 是没有虚拟麦克风
pub(crate) type Opener = fn() -> Result<Option<Box<dyn Output>>>;

/// 播放线程：收包、打开设备（打不开隔一会儿再试）、往设备里写，直到 `stop` 或者会话放开了麦克风
pub(crate) fn render(
    rx: Receiver<Vec<u8>>,
    stop: &AtomicBool,
    status: &watch::Sender<Status>,
    open: Opener,
) {
    let set = |s: Status| {
        status.send_if_modified(|now| {
            let changed = *now != s;
            *now = s;
            changed
        });
    };
    let mut playout = match Playout::new() {
        Ok(p) => p,
        Err(e) => {
            warn!("can't create the Opus decoder: {e:#}");
            set((MicState::Failed, Some(format!("Opus: {e}"))));
            return;
        }
    };
    super::capture::raise_priority();
    let mut out: Option<(Box<dyn Output>, Resampler)> = None;
    let mut retry = Instant::now();
    let mut failures = 0u32;
    while !stop.load(Ordering::Acquire) {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(p) => {
                playout.push(&p);
                while let Ok(p) = rx.try_recv() {
                    playout.push(&p);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // 会话放开了麦克风
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if out.is_none() && Instant::now() >= retry {
            match open() {
                Ok(Some(o)) => {
                    let fmt = o.format();
                    info!(?fmt, "the virtual microphone is open");
                    failures = 0;
                    set((MicState::Ready, None));
                    out = Some((o, Resampler::new(fmt.rate)));
                }
                Ok(None) => {
                    set((MicState::NoDevice, None));
                    retry = Instant::now() + REOPEN;
                }
                Err(e) => {
                    failures += 1;
                    if failures == 1 || failures.is_multiple_of(30) {
                        warn!("can't open the virtual microphone: {e:#}");
                    }
                    set((MicState::Failed, Some(super::capture::wire_detail(&e))));
                    retry = Instant::now() + REOPEN;
                }
            }
        }
        if let Some((o, resampler)) = &mut out
            && let Err(e) = feed(o.as_mut(), resampler, &mut playout)
        {
            // 设备没了、被拔掉了（驱动卸载）：重新找
            info!("the virtual microphone stopped, reopening: {e:#}");
            out = None;
            retry = Instant::now() + Duration::from_millis(200);
        }
    }
}

/// 把设备里排着的补到 [`AHEAD_MS`]（没有声音的时候是静音）
fn feed(o: &mut dyn Output, resampler: &mut Resampler, playout: &mut Playout) -> Result<()> {
    let fmt = o.format();
    let (queued, size) = o.buffered()?;
    let ahead = (fmt.rate as usize * AHEAD_MS / 1000).min(size);
    if queued >= ahead {
        return Ok(());
    }
    let frames = ahead - queued;
    o.write(frames, &mut |buf| {
        for frame in buf.chunks_exact_mut(fmt.block_align) {
            let v = resampler.next(&mut || playout.next());
            fmt.write(frame, v);
        }
    })
}
