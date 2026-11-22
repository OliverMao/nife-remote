//! 被控端发给控制端的消息队列（有界），和控制端的命令队列一样建在 [`crate::queue`] 上：
//!
//! * **走哪条通道由消息决定**（[`HostMsg::lane`]）：发的一方只管 [`Outbox::send`]，不用自己挑控制通道还是大块通道。
//!   大的消息（剪贴板、光标形状）在大块通道上排队，挡不住控制消息；
//! * **只关心最新值的状态不排队**：显示器列表、光标状态（控制通道）各占一个槽位、只留最新的一个，
//!   写任务先发它们，再按顺序发排着队的；
//! * 其它消息按顺序排队，有上限。满了怎么办由发的一方决定（[`SendError::Full`]）：控制消息排满了说明对方早就不收了，
//!   会话结束；剪贴板丢掉这一次；光标图像下一轮再发那时最新的。
//!
//! 显示器列表不能越过还排着队的 [`HostMsg::Hello`]（它带着开始时的显示器列表），不然新的会被旧的盖掉；
//! 反过来，Hello 排进队列时，槽位里更早的显示器列表就作废了。

use std::sync::Arc;

use nife_remote_proto::SessionLane;
use nife_remote_proto::framing;
use nife_remote_proto::handshake::SEAL_OVERHEAD;
use nife_remote_proto::session::HostMsg;
use bytes::Bytes;

use crate::queue::{Item, Limit, Mailbox};
use crate::transport::Frames;

/// 控制通道最多排这么多条。控制消息都很小，排满了说明对方早就不收了
const CONTROL_MESSAGES: usize = 32;
/// 大块通道最多排这么多条、这么多字节（一个剪贴板最多 1 MiB，一个光标形状几十 KB）。
/// 单条帧仍需满足所属通道的协议上限。
const BULK_MESSAGES: usize = 32;
const BULK_BYTES: usize = 4 * 1024 * 1024;

/// 队列里的两条通道和它们的上限
const CONTROL: usize = 0;
const BULK: usize = 1;
const LIMITS: [Limit; 2] = [
    Limit {
        messages: CONTROL_MESSAGES,
        bytes: usize::MAX,
    },
    Limit {
        messages: BULK_MESSAGES,
        bytes: BULK_BYTES,
    },
];
/// 控制通道上只留最新值的槽位（写任务按这个先后发）
const DISPLAYS: usize = 0;
const CURSOR: usize = 1;
const SLOTS: usize = 2;

/// 一条消息在队列里怎么放
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// 控制通道上的一个槽位：只留最新的一个
    Latest(usize),
    /// 按顺序排队。`before`：这个槽位里的状态它也带着，槽位要等它发出去之后才轮到（见模块说明）
    Queued { lane: usize, before: Option<usize> },
}

impl Place {
    fn of(msg: &HostMsg) -> Self {
        let lane = match msg.lane() {
            SessionLane::Bulk => BULK,
            // 画面不是消息，不走这里
            SessionLane::Control | SessionLane::Video => CONTROL,
        };
        let queued = |before| Place::Queued { lane, before };
        match msg {
            HostMsg::Displays { .. } => Place::Latest(DISPLAYS),
            HostMsg::Hello(_) => queued(Some(DISPLAYS)),
            HostMsg::Cursor { .. } => Place::Latest(CURSOR),
            HostMsg::AwaitingApproval
            | HostMsg::Notice(_)
            | HostMsg::Encoder(_)
            | HostMsg::CursorImage { .. }
            | HostMsg::Clipboard { .. }
            | HostMsg::Bye(_)
            | HostMsg::Direct(_)
            // 麦克风的状态很少变，跟在 Hello 后面按顺序发
            | HostMsg::Mic { .. } => queued(None),
        }
    }

    fn lane(self) -> usize {
        match self {
            Place::Latest(_) => CONTROL,
            Place::Queued { lane, .. } => lane,
        }
    }
}

/// 编码好的一条消息（带长度前缀，明文：写任务加密）。同一条消息发给几个会话时只编码一次（[`Outbox::encode`]）
#[derive(Debug, Clone)]
pub struct Encoded {
    bytes: Bytes,
    place: Place,
}

impl Encoded {
    /// 走哪条通道
    pub fn lane(&self) -> SessionLane {
        match self.place.lane() {
            BULK => SessionLane::Bulk,
            _ => SessionLane::Control,
        }
    }

    /// 帧：`u32 长度 | CBOR`
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }
}

/// 为什么没排进去
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// 这条通道排满了（网络跟不上，或者对方不收了）
    #[error("the session's send queue is full")]
    Full,
    /// 会话的写任务已经结束了
    #[error("the session's send queue is closed")]
    Closed,
    #[error("message exceeds its lane's frame budget")]
    TooLarge,
}

struct Shared {
    lanes: [Mailbox<()>; 2],
}

/// 最后一个 [`Outbox`] 丢掉时告诉写任务：排着的发完就结束
struct Alive(Arc<Shared>);

impl Drop for Alive {
    fn drop(&mut self) {
        for lane in &self.0.lanes {
            lane.finish();
        }
    }
}

/// 会话的消息发送端（可以复制，例如交给编码线程）。见模块说明
#[derive(Clone)]
pub struct Outbox {
    shared: Arc<Shared>,
    _alive: Arc<Alive>,
}

/// 一条通道的接收端，交给写任务（[`crate::transport::write_lane`]）。丢掉之后发送端收到 [`SendError::Closed`]
pub struct LaneReceiver {
    shared: Arc<Shared>,
    lane: usize,
}

/// 一个会话的消息队列：发送端，控制通道、大块通道各自的接收端
pub fn channel() -> (Outbox, LaneReceiver, LaneReceiver) {
    let shared = Arc::new(Shared {
        lanes: [Mailbox::new(SLOTS), Mailbox::new(0)],
    });
    let receiver = |lane| LaneReceiver {
        shared: shared.clone(),
        lane,
    };
    (
        Outbox {
            shared: shared.clone(),
            _alive: Arc::new(Alive(shared.clone())),
        },
        receiver(CONTROL),
        receiver(BULK),
    )
}

impl Outbox {
    /// 编码一条消息。同一条消息发给多个会话时只编码一次，每个会话 [`Outbox::send_encoded`]
    pub fn encode(msg: &HostMsg) -> Encoded {
        Encoded {
            // 写任务加密时在后面追加标签和计数，这里先留好位置
            bytes: Bytes::from(framing::encode_with_spare(msg, SEAL_OVERHEAD)),
            place: Place::of(msg),
        }
    }

    pub fn send(&self, msg: &HostMsg) -> Result<(), SendError> {
        self.send_encoded(Self::encode(msg))
    }

    /// 发送 [`Outbox::encode`] 编好的消息。只留最新值的状态（见模块说明）不会排满
    pub fn send_encoded(&self, msg: Encoded) -> Result<(), SendError> {
        if msg.bytes.len() - framing::PREFIX + SEAL_OVERHEAD > msg.lane().max_frame_size() {
            return Err(SendError::TooLarge);
        }
        let index = msg.place.lane();
        let lane = &self.shared.lanes[index];
        {
            let mut m = lane.lock();
            if m.closed {
                return Err(SendError::Closed);
            }
            match msg.place {
                Place::Latest(slot) => m.set(slot, msg.bytes),
                Place::Queued { before, .. } => {
                    if m.full(LIMITS[index], msg.bytes.len()) {
                        return Err(SendError::Full);
                    }
                    m.push_back(Item {
                        bytes: msg.bytes,
                        before,
                        meta: (),
                    });
                }
            }
        }
        lane.wake();
        Ok(())
    }
}

impl LaneReceiver {
    /// 下一帧要发的；发送端都不在了、排着的也发完了时为 `None`。可以放在 `select!` 里（取消不丢消息）
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.shared.lanes[self.lane].recv(|_| {}).await
    }

    /// 现在就能发的下一帧（没有就是 `None`）
    pub fn try_recv(&mut self) -> Option<Bytes> {
        self.shared.lanes[self.lane].try_recv()
    }
}

impl Frames for LaneReceiver {
    async fn next(&mut self) -> Option<Bytes> {
        self.recv().await
    }
}

impl Drop for LaneReceiver {
    fn drop(&mut self) {
        self.shared.lanes[self.lane].close();
    }
}


