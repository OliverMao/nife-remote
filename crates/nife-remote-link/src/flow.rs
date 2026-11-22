//! 被控端的在途控制和码率控制（桌面版和 Android 共用）。帧序号属于会话，不属于某个编码器或显示器：
//! 重建编码管线时接着用同一个 [`Flow`]。
//!
//! 端到端的拥塞控制只能在这里做：中转把 QUIC 连接拆成了两段，两段的拥塞控制各管各的。
//!
//! 控制端每收到一帧都确认（累积确认，几帧连着到时只确认最后一帧），确认里带着这一帧**开始到达**控制端的时刻
//! （控制端的时钟，见 `ControllerDatagram::Ack`）。从确认里知道三件事：
//!
//! * **往返时延**：发出到收到确认，含界面那一段（IPC、8 ms 的确认合并）。基础 RTT 是最近 [`MIN_RTT_WINDOW`]
//!   里的最小值（单调队列实现的滑动窗口最小值）；
//! * **排队时延**（[`FlowState::queue_delay`]）：到达时刻减去本端的发送时刻，是单向时延加上两边时钟之差（常数），
//!   再减去它最近 [`MIN_RTT_WINDOW`] 里的最小值，就是这一帧在路上多排了多久的队（本端 QUIC 的发送缓冲、中转、
//!   瓶颈链路的缓冲区）。不含界面那一段、确认回来的路，也不含这一帧自己要传多久（用的是开始到达的时刻）；
//! * **带宽**：两次确认之间控制端收到了多少字节。只有这一帧发出时前面的还没传完（它排的队比两次到达的间隔还长），
//!   这段时间链路才一直在忙，算出来的才是带宽；不然只是编码器出了多少。
//!
//! **码率**（[`FlowState::bitrate_kbps`]，编码器每帧取一次）随每次确认调整：
//!
//! * 最近 [`STANDING`] 次确认排的队都超过 [`QUEUE_HIGH_MS`]（取最小值，偶尔一帧慢不算），或者排着队、在途字节又已经
//!   挡住了要发的帧（在途字节把队压在门槛以下，码率却比带宽高，只好跳帧）：降到排队期间带宽的 85%，
//!   至少降 10%（量不出带宽时降 20%）。降了之后等一个往返加上排着的队再看：正在消下去就接着等；
//!   降了两次还是不见少，说明不是我们发多了（换了路由、别的下载占着路由器的缓冲区），以现在的单向时延为新的基准；
//! * 不排队（低于 [`QUEUE_LOW_MS`]），编码器也确实用到了估计值的一半以上：每秒约加 15%，接近上次拥塞时的带宽时
//!   每秒约 4%，不超过画质设置里的码率。画面静止、编码器用不了那么多时不加：没有证据说明网络撑得住；
//! * 估计值变化 5% 以上才交给编码器，不用每帧都重新配置编码器。
//!
//! 关掉自适应码率时编码器一直用画质设置里的码率，估计照样做：在途字节还是按估计值限，网络跟不上时少发几帧，
//! 不让延迟越积越多。
//!
//! **在途字节**（已发送、未确认）不超过 码率 × (基础 RTT + [`QUEUE_ALLOWANCE_MS`])：码率估高了、带宽突然掉了，
//! 网络里最多也只积压这么久的数据；编码器等着，等到了就编最新的画面。没有在途的帧时总能发一帧，关键帧再大也不会卡住。
//! 另外最多 [`MAX_WINDOW`] 帧、[`MAX_INFLIGHT_BYTES`] 字节：中转往控制端的连接上所有会话共用一个发送窗口，
//! 画面占满了它，同一连接上的 Bye 也写不进去。
//!
//! **换路**（中转 ⇄ 直连，见 `crate::direct`，[`FlowState::path_changed`]）：新的路时延不一样，基础 RTT、
//! 最小单向时延从头量（不然换到慢一点的路会被当成排队、白白降码率），换路之前发出的帧确认回来时不算样本
//! （它们走的是旧路）；码率的估计接着用。
//! 直连断了换回中转时，断在直连上的帧永远不会确认了：不再算在途，编码器马上能发下一帧（关键帧）。
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use nife_remote_proto::session::QualitySettings;

/// 在途帧数的上限（Android 的画面队列、控制端记到达时刻的环形缓冲都按它定大小）
pub const MAX_WINDOW: u32 = 32;
/// 在途字节的上限，与码率、RTT 无关
pub const MAX_INFLIGHT_BYTES: usize = 4 * 1024 * 1024;
/// 控制心跳正常也不能永久占着未确认的视频；超过期限由会话关闭并重连。
pub const ACK_TIMEOUT: Duration = Duration::from_secs(20);
/// 基础 RTT、最小单向时延取这么长时间里的最小样本
pub const MIN_RTT_WINDOW: Duration = Duration::from_secs(10);
/// 在途字节按 基础 RTT 加上这么久 算：网络里最多排这么久的队（毫秒）
pub const QUEUE_ALLOWANCE_MS: f64 = 50.0;
/// 排队超过这么久算拥塞（毫秒）。比 [`QUEUE_ALLOWANCE_MS`] 小：在途字节卡住之前先降码率
pub const QUEUE_HIGH_MS: f64 = 30.0;
/// 排队少于这么久算通畅（毫秒）
pub const QUEUE_LOW_MS: f64 = 10.0;
/// 看最近这么多次确认排的队
pub const STANDING: usize = 3;
/// 还没有 RTT 样本时按这么多算（毫秒）
const DEFAULT_RTT_MS: f64 = 100.0;
/// 在途字节至少允许这么多
const MIN_WINDOW_BYTES: usize = 16 * 1024;
/// 编码器用了多少码率看最近这么久（控制端的时钟，微秒）；中间空了这么久（画面静止）就从头算
const RATE_SPAN_US: u64 = 1_000_000;
/// 至少这么长（微秒）才算得出编码器用了多少码率
const MIN_RATE_SPAN_US: u64 = 100_000;

/// `kbps` 的码率、`fps` 的帧率下平均一帧多少字节
pub fn frame_bytes(kbps: u32, fps: u8) -> usize {
    kbps as usize * 125 / fps.max(1) as usize
}

/// 最近 [`MIN_RTT_WINDOW`] 里的最小值：样本按时间排列、值严格递增，队头就是窗口里的最小值。
/// 窗口里只有一个样本时即使过了期也留着（画面静止时不发帧，没有新样本）
#[derive(Debug, Default)]
struct WindowMin(VecDeque<(f64, Instant)>);

impl WindowMin {
    fn push(&mut self, v: f64, now: Instant) {
        while self.0.back().is_some_and(|&(x, _)| x >= v) {
            self.0.pop_back();
        }
        self.0.push_back((v, now));
        while self.0.len() > 1
            && self
                .0
                .front()
                .is_some_and(|&(_, t)| now.duration_since(t) > MIN_RTT_WINDOW)
        {
            self.0.pop_front();
        }
    }

    fn get(&self) -> Option<f64> {
        self.0.front().map(|&(v, _)| v)
    }
}

/// 发出去、还没确认的一帧
#[derive(Debug, Clone, Copy)]
struct Sent {
    seq: u32,
    at: Instant,
    bytes: usize,
    /// 在它之前一共发了多少字节
    before: u64,
}

/// 一次带着到达时刻的确认
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// 这一帧开始到达控制端的时刻（控制端的时钟，微秒）
    arrived: u64,
    /// 在它之前一共发了多少字节：流是有序的，到这个时刻控制端已经收到了这么多
    before: u64,
    /// 它排了多久的队（毫秒）
    queue: f64,
}

#[derive(Debug)]
pub struct FlowState {
    sent: u32,
    acked: u32,
    pending: VecDeque<Sent>,
    pending_bytes: usize,
    /// 一共发了多少字节
    total: u64,
    srtt_ms: f64,
    min_rtt: WindowMin,
    /// 把发送时刻换成毫秒的起点（和控制端的到达时刻相减）
    epoch: Instant,
    /// 单向时延（到达时刻 − 发送时刻，含两边时钟之差，毫秒）的最小值
    min_owd: WindowMin,
    /// 最近的确认（[`RATE_SPAN_US`] 之内）
    samples: VecDeque<Sample>,
    rate: Rate,
    /// 编码器用估计的码率（不然用画质设置里的）
    adaptive: bool,
    /// 上次调码率以来，在途字节挡住过要发的帧（[`FlowState::note_blocked`]）
    blocked: bool,
    /// 换了路（[`FlowState::path_changed`]）：这一帧和之前发的走的是旧路，确认时不算时延样本
    old_path: Option<u32>,
}

impl Default for FlowState {
    fn default() -> Self {
        Self {
            sent: 0,
            acked: 0,
            pending: VecDeque::new(),
            pending_bytes: 0,
            total: 0,
            srtt_ms: 0.0,
            min_rtt: WindowMin::default(),
            epoch: Instant::now(),
            min_owd: WindowMin::default(),
            samples: VecDeque::new(),
            rate: Rate::new(QualitySettings::default().bitrate_kbps),
            adaptive: true,
            blocked: false,
            old_path: None,
        }
    }
}

impl FlowState {
    pub fn on_sent(&mut self, seq: u32, bytes: usize) {
        self.on_sent_at(seq, bytes, Instant::now());
    }

    fn on_sent_at(&mut self, seq: u32, bytes: usize, now: Instant) {
        // Android shares a sequence across subscribers, so skipped numbers are
        // not in-flight frames. Never use sent - acked as the queue length.
        if self.pending.is_empty() {
            self.acked = seq.wrapping_sub(1);
        }
        self.sent = seq;
        self.pending.push_back(Sent {
            seq,
            at: now,
            bytes,
            before: self.total,
        });
        self.pending_bytes += bytes;
        self.total += bytes as u64;
    }

    /// 控制端确认到了第 `seq` 帧（累积）。`arrived`：这一帧开始到达控制端的时刻（控制端的时钟，微秒）
    pub fn on_ack(&mut self, seq: u32, arrived: Option<u64>) {
        self.on_ack_at(seq, arrived, Instant::now());
    }

    fn on_ack_at(&mut self, seq: u32, arrived: Option<u64>, now: Instant) {
        if seq.wrapping_sub(self.acked) as i32 <= 0 || seq.wrapping_sub(self.sent) as i32 > 0 {
            return;
        }
        self.acked = seq;
        let mut acked = None;
        while let Some(&f) = self.pending.front() {
            if f.seq.wrapping_sub(seq) as i32 > 0 {
                break;
            }
            self.pending.pop_front();
            self.pending_bytes -= f.bytes;
            if f.seq == seq {
                acked = Some(f);
            }
        }
        let Some(f) = acked else { return };
        // 旧路上发的帧：时延是旧路的，不能拿来当新路的基准
        if let Some(last) = self.old_path {
            if f.seq.wrapping_sub(last) as i32 <= 0 {
                return;
            }
            self.old_path = None;
        }
        let rtt = now.saturating_duration_since(f.at).as_secs_f64() * 1000.0;
        self.srtt_ms = if self.srtt_ms == 0.0 {
            rtt
        } else {
            self.srtt_ms * 0.875 + rtt * 0.125
        };
        self.min_rtt.push(rtt, now);
        if let Some(arrived) = arrived {
            self.on_arrival(f, arrived, now);
        }
    }

    fn on_arrival(&mut self, f: Sent, arrived: u64, now: Instant) {
        let sent = f.at.saturating_duration_since(self.epoch).as_secs_f64() * 1000.0;
        let owd = arrived as f64 / 1000.0 - sent;
        self.min_owd.push(owd, now);
        let queue = owd - self.min_owd.get().unwrap_or(owd);
        // 中间空了很久（画面静止）：之前的样本和现在接不上
        if self
            .samples
            .back()
            .is_some_and(|s| arrived.saturating_sub(s.arrived) > RATE_SPAN_US)
        {
            self.samples.clear();
        }
        self.samples.push_back(Sample {
            arrived,
            before: f.before,
            queue,
        });
        while self
            .samples
            .front()
            .is_some_and(|s| arrived.saturating_sub(s.arrived) > RATE_SPAN_US)
        {
            self.samples.pop_front();
        }
        let n = self.samples.len();
        if n < STANDING {
            return;
        }
        let standing = self
            .samples
            .range(n - STANDING..)
            .map(|s| s.queue)
            .fold(f64::INFINITY, f64::min);
        let signals = Signals {
            standing,
            busy: self.busy_kbps(),
            used: self.used_kbps(),
            srtt: self.srtt_ms,
            blocked: std::mem::take(&mut self.blocked),
        };
        if self.rate.update(now, signals) == Verdict::Rebase {
            self.min_owd = WindowMin::default();
            self.min_owd.push(owd, now);
            self.samples.clear();
        }
    }

    /// 链路一直在忙的那几段（这一帧排的队比和上一次到达的间隔还长）里的交付速率：就是带宽（kbps）
    fn busy_kbps(&self) -> Option<f64> {
        let n = self.samples.len();
        let recent = self.samples.range(n.saturating_sub(STANDING + 1)..);
        let (mut bytes, mut us) = (0u64, 0u64);
        for (a, b) in recent.clone().zip(recent.skip(1)) {
            let gap = b.arrived.saturating_sub(a.arrived);
            if b.queue * 1000.0 >= gap as f64 {
                bytes += b.before - a.before;
                us += gap;
            }
        }
        (us > 0 && bytes > 0).then(|| bytes as f64 * 8000.0 / us as f64)
    }

    /// 最近控制端收到画面的速率（kbps）：编码器实际用了多少码率
    fn used_kbps(&self) -> Option<f64> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        let us = last.arrived.saturating_sub(first.arrived);
        (us >= MIN_RATE_SPAN_US).then(|| (last.before - first.before) as f64 * 8000.0 / us as f64)
    }

    /// 最近确认的一帧在网络上比最快的时候多排了多久（毫秒，见模块说明）；控制端没报到达时刻时为 `None`
    pub fn queue_delay(&self) -> Option<f64> {
        self.samples.back().map(|s| s.queue)
    }

    /// 画质设置里的码率（上限）、要不要自适应。码率变了就从这个码率重新开始估计
    pub fn set_target(&mut self, kbps: u32, adaptive: bool) {
        if kbps != self.rate.target {
            self.rate = Rate::new(kbps);
        }
        self.adaptive = adaptive;
    }

    /// 编码器现在该用的码率（kbps，见模块说明）；不自适应时就是画质设置里的码率
    pub fn bitrate_kbps(&self) -> u32 {
        if self.adaptive {
            self.rate.applied
        } else {
            self.rate.target
        }
    }

    /// 平均一帧多少字节（按编码器现在的码率）
    pub fn frame_bytes(&self, fps: u8) -> usize {
        frame_bytes(self.bitrate_kbps(), fps)
    }

    pub fn inflight(&self) -> u32 {
        self.pending.len() as u32
    }

    pub fn ack_stalled(&self) -> bool {
        self.pending
            .front()
            .is_some_and(|f| f.at.elapsed() > ACK_TIMEOUT)
    }

    /// 在途字节的上限：码率 × (基础 RTT + [`QUEUE_ALLOWANCE_MS`])
    pub fn window_bytes(&self) -> usize {
        let rtt = self.min_rtt.get().unwrap_or(DEFAULT_RTT_MS);
        // kbps × ms / 8 = 字节
        let bytes = self.rate.estimate * (rtt + QUEUE_ALLOWANCE_MS) / 8.0;
        (bytes as usize).clamp(MIN_WINDOW_BYTES, MAX_INFLIGHT_BYTES)
    }

    /// 在途字节挡住了要发的帧（编码器得等、这个会话跳过一帧）：码率控制把它算作拥塞的迹象（见模块说明）
    pub fn note_blocked(&mut self) {
        self.blocked = true;
    }

    /// 能不能再发 `bytes` 字节的一帧。没有在途的帧时总能发
    pub fn can_send(&self, bytes: usize) -> bool {
        self.pending.is_empty()
            || (self.pending.len() < MAX_WINDOW as usize
                && self.pending_bytes + bytes <= self.window_bytes())
    }

    /// (平滑的 RTT, 基础 RTT)，毫秒；还没有样本时是 0
    pub fn rtt(&self) -> (f64, f64) {
        (self.srtt_ms, self.min_rtt.get().unwrap_or(0.0))
    }

    /// 画面换了一条路（见模块说明的"换路"）。`lost`：旧路断了，在途的帧不会确认了
    pub fn path_changed(&mut self, lost: bool) {
        self.min_rtt = WindowMin::default();
        self.min_owd = WindowMin::default();
        self.samples.clear();
        // 还在旧路上的帧确认回来时不算时延样本（旧路快的话，新路会被当成一直在排队）
        self.old_path = Some(self.sent);
        if lost {
            self.pending.clear();
            self.pending_bytes = 0;
            self.acked = self.sent;
        }
    }
}

/// 码率控制收到的信号（见模块说明）
#[derive(Debug, Clone, Copy)]
struct Signals {
    /// 最近 [`STANDING`] 次确认里排队最少的（毫秒）
    standing: f64,
    /// 排着队时量到的带宽（kbps）
    busy: Option<f64>,
    /// 编码器最近实际用了多少码率（kbps）
    used: Option<f64>,
    /// 平滑的 RTT（毫秒）
    srtt: f64,
    /// 在途字节挡住过要发的帧
    blocked: bool,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Keep,
    /// 排队降不下来：以现在的单向时延为新的基准
    Rebase,
}

#[derive(Debug)]
struct Rate {
    /// 画质设置里的码率（上限）
    target: u32,
    /// 估计值（kbps）
    estimate: f64,
    /// 交给编码器的（kbps）
    applied: u32,
    /// 上次往上加的时刻（一直不排队时才有）
    raised: Option<Instant>,
    /// 降了之后，这之前不再动
    hold: Option<Instant>,
    /// 这一轮排队（超过 [`QUEUE_HIGH_MS`] 到降回 [`QUEUE_LOW_MS`] 以下）里降了几次、上次看时排了多久
    episode: Option<(u32, f64)>,
    /// 上次拥塞时量到的带宽（kbps）：估计值接近它时慢慢加
    capacity: Option<f64>,
}

impl Rate {
    fn new(target: u32) -> Self {
        Self {
            target,
            estimate: target as f64,
            applied: target,
            raised: None,
            hold: None,
            episode: None,
            capacity: None,
        }
    }

    /// 最低降到这么多
    fn floor(&self) -> f64 {
        (self.target / 6).max(300).min(self.target) as f64
    }

    fn update(&mut self, now: Instant, s: Signals) -> Verdict {
        let held = self.hold.is_some_and(|t| now < t);
        // 降了之后多久再看：一个往返加上排着的队
        let wait = Duration::from_secs_f64(((s.srtt + s.standing) / 1000.0).clamp(0.1, 1.0));
        if s.standing > QUEUE_HIGH_MS || (s.blocked && s.standing > QUEUE_LOW_MS) {
            self.raised = None;
            if held {
                return Verdict::Keep;
            }
            match self.episode {
                // 降过之后正在消下去：再等等
                Some((times, last)) if s.standing < last * 0.8 => {
                    self.episode = Some((times, s.standing));
                    self.hold = Some(now + wait);
                    return Verdict::Keep;
                }
                // 降了两次还是不见少：不是我们发多了
                Some((times, last)) if times >= 2 && s.standing <= last * 1.1 => {
                    self.episode = None;
                    self.hold = None;
                    return Verdict::Rebase;
                }
                _ => {}
            }
            let next = match s.busy {
                Some(busy) => (busy * 0.85).min(self.estimate * 0.9),
                None => self.estimate * 0.8,
            };
            self.estimate = next.max(self.floor());
            if s.busy.is_some() {
                self.capacity = s.busy;
            }
            let times = self.episode.map_or(0, |(t, _)| t);
            self.episode = Some((times + 1, s.standing));
            self.hold = Some(now + wait);
        } else if s.standing < QUEUE_LOW_MS {
            self.episode = None;
            if held {
                self.raised = None;
                return Verdict::Keep;
            }
            let dt = self.raised.map_or(0.0, |t| {
                now.saturating_duration_since(t).as_secs_f64().min(0.2)
            });
            self.raised = Some(now);
            if s.used.is_some_and(|u| u >= self.estimate * 0.5) {
                let near = self.capacity.is_some_and(|c| self.estimate >= c * 0.9);
                let per_sec = if near { 0.04 } else { 0.15 };
                self.estimate =
                    (self.estimate * (1.0 + per_sec * dt) + 100.0 * dt).min(self.target as f64);
                // 超过上次拥塞时的带宽不少了：网络变好了
                if self.capacity.is_some_and(|c| self.estimate > c * 1.2) {
                    self.capacity = None;
                }
            }
        } else {
            // 排着一点队：不加也不降
            self.raised = None;
        }
        self.apply();
        Verdict::Keep
    }

    /// 估计值变化 5% 以上（往下的都是）才交给编码器
    fn apply(&mut self) {
        let e = self.estimate.round() as u32;
        if e < self.applied || e >= self.applied + self.applied / 20 || e == self.target {
            self.applied = e;
        }
    }
}

#[derive(Default)]
pub struct Flow {
    state: Mutex<FlowState>,
    cv: Condvar,
}

impl Flow {
    /// Assign and record the next session-wide frame before exposing it to the receiver.
    pub fn sent(&self, bytes: usize) -> u32 {
        let mut state = self.state.lock().unwrap();
        let seq = state.sent.wrapping_add(1);
        state.on_sent(seq, bytes);
        seq
    }
    /// A frame assigned a sequence but cancelled before enqueueing needs no ACK.
    pub fn discard(&self, seq: u32) {
        let mut state = self.state.lock().unwrap();
        if let Some(index) = state.pending.iter().position(|f| f.seq == seq) {
            let f = state.pending.remove(index).unwrap();
            state.pending_bytes -= f.bytes;
            state.total -= f.bytes as u64;
        }
        drop(state);
        self.cv.notify_all();
    }
    pub fn ack(&self, seq: u32, arrived: Option<u64>) {
        self.state.lock().unwrap().on_ack(seq, arrived);
        self.cv.notify_all();
    }
    pub fn wake(&self) {
        self.cv.notify_all();
    }
    /// 编码之前：等到在途的少到能再发平均大小的一帧（按现在的码率和 `fps` 估计）
    pub fn wait_window(&self, fps: u8, stop: &AtomicBool, timeout: Duration) -> bool {
        self.wait_until(stop, timeout, |s| s.can_send(s.frame_bytes(fps)))
    }
    /// 编码后知道真实大小，再等字节预算；等待期间必须保留这一帧，否则后续增量帧失去参考。
    pub fn wait_room(&self, bytes: usize, stop: &AtomicBool, timeout: Duration) -> bool {
        self.wait_until(stop, timeout, |s| s.can_send(bytes))
    }
    fn wait_until(
        &self,
        stop: &AtomicBool,
        timeout: Duration,
        ready: impl Fn(&FlowState) -> bool,
    ) -> bool {
        let mut state = self.state.lock().unwrap();
        if !ready(&state) {
            state.note_blocked();
        }
        let (state, _) = self
            .cv
            .wait_timeout_while(state, timeout, |s| {
                !ready(s) && !stop.load(Ordering::Acquire)
            })
            .unwrap();
        !stop.load(Ordering::Acquire) && ready(&state)
    }
    pub fn ack_stalled(&self) -> bool {
        self.state.lock().unwrap().ack_stalled()
    }
    /// 见 [`FlowState::path_changed`]
    pub fn path_changed(&self, lost: bool) {
        self.state.lock().unwrap().path_changed(lost);
        self.cv.notify_all();
    }
    /// 见 [`FlowState::set_target`]
    pub fn set_target(&self, kbps: u32, adaptive: bool) {
        self.state.lock().unwrap().set_target(kbps, adaptive);
        self.cv.notify_all();
    }
    /// 见 [`FlowState::bitrate_kbps`]
    pub fn bitrate_kbps(&self) -> u32 {
        self.state.lock().unwrap().bitrate_kbps()
    }
}


