//! 一条通道从几条路（中转上的流、直连上的流）收到的帧，按计数排好再解密、交出去（见 `nife_remote_proto` 的"直连"）。
//! 只有中转一条路的通道（大块通道）也用它：解密、按通道的上限读帧，和别的通道是同一段代码。
//!
//! 每个密封单元末尾的计数是明文（[`crate::secure::counter_of`]），每条路自己是有序的，发送端的计数一个接一个，
//! 所以接收端看计数就知道该交哪一帧、哪一帧是重复的：
//!
//! * **控制、大块通道**（[`Policy::Complete`]）：中转上是完整的一份，直连上是同样的帧（发送端来不及时可能少几帧）。
//!   计数正好是下一个的交出去，已经交出去了的（另一条路先到了）丢掉；直连上来早了的先放着，等中转把中间的补上。
//!   中转上的帧必须正好是下一个、或者已经交出去了的：和只有中转时一样，中转删不掉、调换不了一条消息
//!   （直连已经送到了的除外，那对接收端没有区别）；
//! * **画面**（[`Policy::Video`]）：每一帧只走一条路，被控端换路时新路上的帧可能先到，要等旧路上的。
//!   直连突然断了，断在上面的帧再也等不到：**关键帧不用等前面的帧**（被控端换路时编一个），
//!   另外等了 [`VIDEO_WAIT`] 还等不到也不等了，交出去的帧标上 [`Frame::gap`]，控制端据此要一个关键帧。
//!
//! 每条路一个读任务，把读到的帧放进同一个信箱。对方的流（中转上的那条、每次直连通了之后直连上的那条）从这条通道的
//! 信箱（[`crate::lanes::Inbox`]）来。

use std::collections::BTreeMap;
use std::time::Duration;

use nife_remote_proto::SessionLane;
use nife_remote_proto::media::{Kind, VideoHeader};
use anyhow::{Context, Result, bail};
use quinn::RecvStream;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::debug;

use crate::clock::now_us;
use crate::lanes::{LaneStreams, Via};
use crate::secure::{Opener, counter_of};
use crate::wire::read_packet_timed;

/// 画面：等前面的帧最多等这么久（一直等不到说明断在直连上了），之后从手里最早的一帧接着交
pub const VIDEO_WAIT: Duration = Duration::from_secs(1);
/// 画面：最多先放着这么多帧、这么多字节（被控端在途的帧不超过 `flow::MAX_WINDOW` 帧、`flow::MAX_INFLIGHT_BYTES`），
/// 再多就不等了
const MAX_HELD_FRAMES: usize = 2 * crate::flow::MAX_WINDOW as usize;
const MAX_HELD_VIDEO_BYTES: usize = 8 * crate::flow::MAX_INFLIGHT_BYTES;
/// 控制、大块通道：直连上来早了的帧最多先放着这么多字节，再多就丢掉（中转上还有一份）
const MAX_HELD_BYTES: usize = 8 * 1024 * 1024;

/// 怎么合并（见模块说明）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// 控制、大块通道：中转上是完整的一份
    Complete,
    /// 画面：每一帧只走一条路
    Video,
}

/// 交出去的一帧（已经解密）
#[derive(Debug)]
pub struct Frame {
    pub data: Vec<u8>,
    /// 开始从网络上到达的时刻（控制端的时钟，见 [`crate::wire::read_packet_timed`]）
    pub arrived: u64,
    /// 从直连来的
    pub direct: bool,
    /// 画面：前面有帧没等到（断在了直连上），解码器要从关键帧重新开始
    pub gap: bool,
}

struct Held {
    /// [`Policy::Complete`]：密文（按顺序解密）；[`Policy::Video`]：明文（收到就解开了，要看是不是关键帧）
    data: Vec<u8>,
    arrived: u64,
    direct: bool,
    key: bool,
}

/// 合并的状态（不碰 IO，测试直接用）
struct Core {
    policy: Policy,
    opener: Opener,
    /// 下一个要交出去的计数
    next: u64,
    held: BTreeMap<u64, Held>,
    held_bytes: usize,
    /// 画面：从什么时候开始等 `next`（手里有后面的帧）
    waiting: Option<Instant>,
}

impl Core {
    fn new(policy: Policy, opener: Opener) -> Self {
        Self {
            policy,
            next: opener.expected(),
            opener,
            held: BTreeMap::new(),
            held_bytes: 0,
            waiting: None,
        }
    }

    /// 收到一帧（密封单元）
    fn push(&mut self, direct: bool, mut data: Vec<u8>, arrived: u64) -> Result<()> {
        let c = counter_of(&data).context("a session frame is too short")?;
        if c < self.next || self.held.contains_key(&c) {
            // 另一条路先送到了
            return Ok(());
        }
        let key = match self.policy {
            Policy::Complete => {
                if !direct && c != self.next {
                    bail!("a session frame was dropped or reordered");
                }
                // 直连上来早了：先放着，放不下就算了（中转上还有一份）
                if c != self.next && self.held_bytes + data.len() > MAX_HELD_BYTES {
                    return Ok(());
                }
                false
            }
            Policy::Video => {
                if !self.opener.is_fresh(c) {
                    return Ok(());
                }
                self.opener.open(&mut data)?;
                anyhow::ensure!(
                    Kind::of(&data) == Some(Kind::Video),
                    "unexpected packet on the video stream"
                );
                VideoHeader::decode_any(&data).is_some_and(|h| h.key)
            }
        };
        self.held_bytes += data.len();
        self.held.insert(
            c,
            Held {
                data,
                arrived,
                direct,
                key,
            },
        );
        Ok(())
    }

    /// 现在能交出去的下一帧
    fn pop(&mut self, now: Instant) -> Result<Option<Frame>> {
        if let Some(h) = self.take(self.next) {
            return self.deliver(h, false).map(Some);
        }
        if self.policy == Policy::Complete || self.held.is_empty() {
            self.waiting = None;
            return Ok(None);
        }
        // 画面：关键帧不用等前面的帧
        if let Some(k) = self.held.iter().find(|(_, h)| h.key).map(|(c, _)| *c) {
            debug!(from = self.next, to = k, "video: skipping to a keyframe");
            return self.skip_to(k, false).map(Some);
        }
        let since = *self.waiting.get_or_insert(now);
        if now >= since + VIDEO_WAIT
            || self.held.len() >= MAX_HELD_FRAMES
            || self.held_bytes > MAX_HELD_VIDEO_BYTES
        {
            let first = *self.held.keys().next().expect("not empty");
            debug!(
                from = self.next,
                to = first,
                "video: gave up waiting for frames"
            );
            return self.skip_to(first, true).map(Some);
        }
        Ok(None)
    }

    /// 画面：等到这个时候还等不到前面的帧就不等了
    fn deadline(&self) -> Option<Instant> {
        self.waiting.map(|t| t + VIDEO_WAIT)
    }

    fn take(&mut self, c: u64) -> Option<Held> {
        let h = self.held.remove(&c)?;
        self.held_bytes -= h.data.len();
        Some(h)
    }

    /// 前面的帧不等了：从 `c` 接着交
    fn skip_to(&mut self, c: u64, gap: bool) -> Result<Frame> {
        while let Some((&older, _)) = self.held.first_key_value().filter(|(k, _)| **k < c) {
            self.take(older);
        }
        self.next = c;
        let h = self.take(c).expect("held");
        self.deliver(h, gap)
    }

    fn deliver(&mut self, mut h: Held, gap: bool) -> Result<Frame> {
        if self.policy == Policy::Complete {
            self.opener.open(&mut h.data)?;
        }
        self.next += 1;
        self.waiting = None;
        Ok(Frame {
            data: h.data,
            arrived: h.arrived,
            direct: h.direct,
            gap,
        })
    }
}

enum Event {
    Frame {
        direct: bool,
        data: Vec<u8>,
        arrived: u64,
    },
    End {
        direct: bool,
        error: Option<anyhow::Error>,
    },
}

/// 中转上的那条流
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relay {
    /// 对方还没开过来
    Waiting,
    Open,
    Ended,
}

/// 一条通道的接收端：几条路上的帧合并成一串（见模块说明）。读任务随它一起结束
pub struct Merge {
    lane: SessionLane,
    core: Core,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    streams: LaneStreams,
    streams_open: bool,
    relay: Relay,
    /// 还开着的直连的流
    direct: usize,
    readers: JoinSet<()>,
}

impl Merge {
    /// `streams`：对方在这条通道上开的流（[`crate::lanes::Inbox`] 里这条通道的信箱）
    pub fn new(lane: SessionLane, policy: Policy, opener: Opener, streams: LaneStreams) -> Self {
        let capacity = (4 * 1024 * 1024 / lane.max_frame_size()).clamp(2, 16);
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            lane,
            core: Core::new(policy, opener),
            tx,
            rx,
            streams,
            streams_open: true,
            relay: Relay::Waiting,
            direct: 0,
            readers: JoinSet::new(),
        }
    }

    /// 加一条已经拿到的流（握手时在控制通道上读过的那条）
    pub fn add(&mut self, recv: RecvStream, via: Via) -> Result<()> {
        let direct = via == Via::Direct;
        if !direct {
            anyhow::ensure!(
                self.relay == Relay::Waiting,
                "the relay opened {:?} twice",
                self.lane
            );
            self.relay = Relay::Open;
        }
        self.read(recv, direct);
        Ok(())
    }

    fn read(&mut self, mut recv: RecvStream, direct: bool) {
        let tx = self.tx.clone();
        let max = self.lane.max_frame_size();
        if direct {
            self.direct += 1;
        }
        self.readers.spawn(async move {
            loop {
                let ev = match read_packet_timed(&mut recv, max, now_us).await {
                    Ok(Some((data, arrived))) => Event::Frame {
                        direct,
                        data,
                        arrived,
                    },
                    Ok(None) => Event::End {
                        direct,
                        error: None,
                    },
                    Err(e) => Event::End {
                        direct,
                        error: Some(e.into()),
                    },
                };
                let end = matches!(ev, Event::End { .. });
                if tx.send(ev).await.is_err() || end {
                    return;
                }
            }
        });
    }

    /// 下一帧（已经解密）。中转上的流结束了就是这条通道结束了（`None`，画面要等直连上的也都结束）；
    /// 中转上的流出错、解不开都是错误，直连上的流断了不算。可以放在 `select!` 里（取消不丢帧）
    pub async fn next(&mut self) -> Result<Option<Frame>> {
        loop {
            if let Some(f) = self.core.pop(Instant::now())? {
                return Ok(Some(f));
            }
            let relay_done = match self.relay {
                Relay::Ended => true,
                // 会话结束了，对方再也开不过来了
                Relay::Waiting => !self.streams_open,
                Relay::Open => false,
            };
            let ended = match self.core.policy {
                // 中转上的是完整的一份：它结束了，手里剩下的也补不齐了
                Policy::Complete => relay_done,
                Policy::Video => relay_done && self.direct == 0 && self.core.held.is_empty(),
            };
            if ended {
                return Ok(None);
            }
            let deadline = self.core.deadline();
            tokio::select! {
                biased;
                ev = self.rx.recv() => match ev.expect("we keep a sender") {
                    Event::Frame { direct, data, arrived } => self.core.push(direct, data, arrived)?,
                    Event::End { direct: false, error } => {
                        if let Some(e) = error {
                            return Err(e);
                        }
                        self.relay = Relay::Ended;
                    }
                    Event::End { direct: true, error } => {
                        self.direct -= 1;
                        if let Some(e) = error {
                            debug!(lane = ?self.lane, "a direct stream ended: {e:#}");
                        }
                    }
                },
                s = self.streams.recv(), if self.streams_open => match s {
                    Some((recv, via)) => self.add(recv, via)?,
                    None => self.streams_open = false,
                },
                () = sleep_until(deadline) => {}
            }
        }
    }
}

async fn sleep_until(t: Option<Instant>) {
    match t {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}


