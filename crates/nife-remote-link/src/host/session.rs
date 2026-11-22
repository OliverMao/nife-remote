//! 被控端的一个会话：协议层的事都在这里（[`serve`]），各平台只实现 [`Platform`]（采集、编码、注入输入、请本机用户同意）。
//!
//! 桌面版和 Android 以前各写一份会话循环，Pong、失联检测、踢人时发 Bye、画质设置、同意流程各有一份，
//! 慢慢就会不一样。现在它们只在这里：
//!
//! ```text
//! 端到端握手 ─┬─ 不用同意（配对过的、同意过的、谁都能连）→ start：Hello、画面…… ─ 控制端确认了 → confirmed：显示它
//!            └─ NeedsApproval ─ 控制端确认了 → AwaitingApproval，问本机用户（最多 60 秒）─┬─ 同意 → start
//!                                                                                   └─ 拒绝 / 没人理 → Bye
//! ```
//!
//! * 不用同意的会话不等控制端的确认就开始（省一个往返，编码器的初始化也和它重叠），确认了才显示它
//!   （[`Platform::confirmed`]）；[`CONFIRM_TIMEOUT`] 等不到确认就结束（见 `nife_remote_proto::handshake` 的"确认"）；
//! * 要同意的会话确认了才问本机用户：重放的握手消息不能弹出请求。同时最多有 [`MAX_PENDING_APPROVALS`] 个请求
//!   在等本机用户回答，再来的不问、直接回绝（[`ApprovalSlots`]）；
//! * 同意之前只回应 Ping、记下画质和声音设置（同意之后随 [`Start`] 一起生效），不发画面、不执行任何输入；
//! * 一直回应 Ping，[`PEER_TIMEOUT`] 收不到控制端的任何消息算失联。Ping 和画面确认（控制数据报）在读任务里
//!   就处理了（确认交给 [`Platform::acks`]），不等这里：这里可能正等着平台重建编码管线（见 [`super::HostStreams`]）；
//! * 本机用户断开所有控制端（[`Incoming::kick`]）时发 [`HostMsg::Bye`]，然后结束；
//! * 控制端的请求在会话的读任务里就换成了 [`Request`]（见 [`super::HostStreams`]）交给平台：画质设置做过范围校验，
//!   过时的输入已经换成了"释放所有按键"，对方版本新增的输入事件已经去掉了；
//! * 控制端麦克风的声音包和画面确认一样在读任务里就交给平台（[`Platform::mic`]），放不放进虚拟麦克风由平台决定
//!   （控制端打开了麦克风、[`Request::Mic`]，才放；会话开始之前的请求不会到平台）。
//!
//! 协议加了新的控制端消息，读任务里的 `match` 编译不过：在那里决定它是什么 [`Request`]（或者协议层自己处理），
//! 各平台跟着实现，不会一边做了、一边忘了。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use nife_remote_proto::session::{ByeReason, HostMsg, InputEvent, QualitySettings, Codec};
use anyhow::{Result, bail};
use bytes::Bytes;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::time::Instant;
use tracing::{info, warn};

use super::queue::Outbox;
use super::streams::{Admitted, ControllerReceiver, HostStreams, Writer};

/// 画面确认交给谁（[`Platform::acks`]）：桌面版是会话的 [`crate::flow::Flow`]，Android 是画面分发。
/// 在读任务里调用：要快，不能等什么
pub trait Acks: Send + Sync + 'static {
    /// 控制端收到了第 `seq` 帧（累积确认）；`arrived` 见 `ControllerDatagram::Ack`
    fn ack(&self, seq: u32, arrived: u64);

    /// 画面换了一条路（中转 ⇄ 直连，见 [`crate::direct`]）：时延的基准要从头量。
    /// `lost`：旧路（直连）断了，在途的帧不会确认了，不要再等它们（会话随即要一个关键帧）
    fn path_changed(&self, lost: bool) {
        let _ = lost;
    }
}

/// 控制端麦克风的声音交给谁（[`Platform::mic`]，见 `nife_remote_proto::session` 的"麦克风"）。在读任务里调用：要快，不能等什么
pub trait MicSink: Send + Sync + 'static {
    /// 一个声音包（`nife_remote_proto::media` 格式：头 + Opus 包）。数据报里的副本已经去掉了、丢了的补上了，
    /// 按采集先后一个一个来（见 `crate::audio`）
    fn packet(&self, packet: Vec<u8>);
}

impl Acks for crate::flow::Flow {
    fn ack(&self, seq: u32, arrived: u64) {
        crate::flow::Flow::ack(self, seq, Some(arrived));
    }

    fn path_changed(&self, lost: bool) {
        crate::flow::Flow::path_changed(self, lost);
    }
}
use super::{Admission, Gate, Incoming};

/// 控制端每秒都会发 Ping，这么久没有任何消息说明对方已失联
pub const PEER_TIMEOUT: Duration = Duration::from_secs(30);
/// 这么久等不到控制端的确认（见 `nife_remote_proto::handshake` 的"确认"）就结束会话
pub const CONFIRM_TIMEOUT: Duration = crate::secure::STEP_TIMEOUT;
/// 本机用户这么久没有回应连接请求就当作没人理会（[`ByeReason::Unanswered`]）
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
/// 多久检查一次是不是失联了
const WATCHDOG: Duration = Duration::from_secs(5);
/// 同时最多有这么多个连接请求在等本机用户回答，再来的不问、直接回绝（[`ByeReason::TooManyRequests`]）。
///
/// 知道控制码的人可以反复发起连接：没有这个上限的话，每个请求都弹出一次"是否允许"、各占着服务器上这台被控端的
/// 一个会话槽位直到 [`APPROVAL_TIMEOUT`]。回绝的会话马上结束，槽位随之归还。
/// 配对过的、同意过的控制端不用问，不受影响
pub const MAX_PENDING_APPROVALS: usize = 2;

/// 在等本机用户回答的连接请求的名额（[`MAX_PENDING_APPROVALS`]）：一个被控端服务的所有会话共用
/// （[`Incoming::approvals`]）
#[derive(Clone)]
pub struct ApprovalSlots(Arc<Semaphore>);

impl Default for ApprovalSlots {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(MAX_PENDING_APPROVALS)))
    }
}

impl ApprovalSlots {
    /// 占一个名额（丢掉时归还）；满了为 `None`
    fn take(&self) -> Option<OwnedSemaphorePermit> {
        self.0.clone().try_acquire_owned().ok()
    }
}

/// 控制端要被控端做的事（会话开始之后才有，见模块说明）
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// 一批输入（不会是空的）。过时的已经换成了 [`InputEvent::ReleaseAll`]，不认识的事件已经去掉了
    Input(Vec<InputEvent>),
    /// 画质设置变了（做过范围校验）
    Settings(QualitySettings),
    /// 要一个关键帧：控制端要的（例如解码器重置了）带着它收到的最后一帧的序号，在那之后发过关键帧就不用再编了
    /// （见 `nife_remote_proto::session` 的"关键帧"）；`None`：被控端自己要的（直连断了，断在上面的帧接收端等不到了），一定要编
    Keyframe(Option<u32>),
    /// 改看另一个显示器（序号没检查过）
    SelectDisplay(u8),
    /// 控制端复制了文本（不会超过 [`nife_remote_proto::session::MAX_CLIPBOARD_TEXT`]）
    Clipboard(String),
    /// 打开 / 关闭声音
    Audio(bool),
    /// 打开 / 关闭控制端的麦克风（见 [`MicSink`]）
    Mic(bool),
}

/// 会话怎么开始：握手里的画质和要不要声音，同意之前又收到的设置也算进来
#[derive(Debug, Clone, PartialEq)]
pub struct Start {
    /// 做过范围校验
    pub settings: QualitySettings,
    pub audio: bool,
    /// 控制端能解码的格式（空 = 旧版控制端，只有 H.264 一定能用）
    pub codecs: Vec<Codec>,
}

/// 交给平台的发送端（[`serve`] 的 `platform` 拿到它们）
pub struct Senders {
    /// 控制消息：Hello、提示、显示器列表、光标、剪贴板……（按消息挑通道，见 [`super::queue`]）
    pub out: Outbox,
    /// 画面包
    pub video: mpsc::Sender<Bytes>,
    /// 声音包
    pub audio: mpsc::Sender<Bytes>,
}

/// 一个会话里各平台不同的事。方法都在会话的任务里依次调用，不会同时进来两个。
///
/// 会话随时可能在某个 `await` 处被取消（与服务器的连接断了、更换控制码……）：一定要做的清理放在 `Drop` 里，
/// [`Platform::stop`] 只做正常结束时要等的事（例如等编码线程退出，写任务才能把 Bye 发完）
pub trait Platform: Send {
    /// 平台自己的事（定时检查、别处来的广播……），见 [`Platform::next_event`]
    type Event: Send;

    /// 请本机用户同意这次连接（握手时 [`Gate`] 要求的，[`Admission::NeedsApproval`]）。
    /// 回答从返回的通道来，通道关了算拒绝；最多等 [`APPROVAL_TIMEOUT`]。同时在等回答的请求最多
    /// [`MAX_PENDING_APPROVALS`] 个，再来的不会问到这里。
    /// 默认直接拒绝：给不会要求同意的 [`Gate`] 用
    fn ask(&mut self, admitted: &Admitted) -> oneshot::Receiver<bool> {
        let _ = admitted;
        warn!("this host can't ask its user to allow a connection, declining");
        oneshot::channel().1
    }

    /// 本机用户回答了连接请求（没人理会、会话先结束了的不调用）。同意了的话接着就是 [`Platform::start`]
    fn answered(&mut self, allowed: bool) {
        let _ = allowed;
    }

    /// 控制端确认了握手（见 `nife_remote_proto::handshake` 的"确认"）：是真的在连，这时才在托盘、列表里显示它。
    /// 不用同意的会话这时已经开始了（[`Platform::start`] 在确认之前：重放的握手消息看不到画面，只是让本机白编码一会儿）；
    /// 要同意的会话接着就是 [`Platform::ask`]
    fn confirmed(&mut self) {}

    /// 画面确认交给谁。会话一开始取一次；确认在读任务里就交给它，不经过 [`Platform::request`]
    fn acks(&self) -> Arc<dyn Acks>;

    /// 控制端麦克风的声音交给谁。会话一开始取一次，和画面确认一样在读任务里就交给它。
    /// 默认没有（收到的丢掉）：这个平台不接麦克风
    fn mic(&self) -> Option<Arc<dyn MicSink>> {
        None
    }

    /// 会话开始（不用同意，或者本机用户同意了）：发 [`HostMsg::Hello`] 和开始时的提示，开始采集
    fn start(&mut self, start: Start) -> impl Future<Output = Result<()>> + Send;

    /// 执行控制端的请求。返回错误时会话结束
    fn request(&mut self, req: Request) -> impl Future<Output = Result<()>> + Send;

    /// 等平台自己的下一件事（会话开始之后才等）。会和控制端的消息一起放在 `select!` 里：
    /// 必须能随时取消、不丢东西（等定时器、等通道可以；真正做事放在 [`Platform::event`] 里）
    fn next_event(&mut self) -> impl Future<Output = Self::Event> + Send;

    /// 处理 [`Platform::next_event`] 等到的事。返回错误时会话结束
    fn event(&mut self, event: Self::Event) -> impl Future<Output = Result<()>> + Send;

    /// 视频确认超过期限，控制心跳仍正常也应结束会话，避免永久冻结画面。
    fn video_stalled(&self) -> bool {
        false
    }

    /// 会话正常结束：停掉还拿着发送端的东西（编码线程、声音订阅），写任务才能把排着的（例如 Bye）发完
    fn stop(self) -> impl Future<Output = ()> + Send
    where
        Self: Sized;
}

/// 一个会话，从端到端握手到结束。
///
/// * `gate`：让不让这个控制端进来（拒绝时返回错误，还没有调用 `platform`）；
/// * `video_queue` / `audio_queue`：画面、声音的发送队列有多长（见 [`HostStreams::open`]）；
/// * `platform`：握手通过之后建起这个会话的平台部分（控制端还没确认：确认了才调用 [`Platform::confirmed`]）。
pub async fn serve<P, F>(
    inc: Incoming,
    gate: &dyn Gate,
    video_queue: usize,
    audio_queue: usize,
    platform: F,
) -> Result<()>
where
    P: Platform,
    F: FnOnce(&Admitted, Senders) -> P,
{
    let session_id = inc.session_id;
    let mut kick = inc.kick.clone();
    let (mut io, admitted) = match HostStreams::open(&inc, video_queue, audio_queue, gate).await {
        Ok(r) => r,
        Err(e) => {
            warn!(session_id, "controller not admitted: {e:#}");
            return Err(e);
        }
    };
    if admitted.admission == Admission::Known {
        info!(session_id, "a known controller connected");
    }
    let out = io.out.clone();
    let started = io.started();
    let mut platform = platform(
        &admitted,
        Senders {
            out: io.out.clone(),
            video: io.video.clone(),
            audio: io.audio.clone(),
        },
    );
    let result = drive(
        session_id,
        &admitted,
        &mut platform,
        &out,
        &mut io.incoming,
        &mut kick,
        &mut io.writer,
        &inc.approvals,
        &|| started.notify_one(),
    )
    .await;
    // 先停掉还拿着发送端的东西，写任务才能把排着的（例如 Bye）发完
    platform.stop().await;
    drop(out);
    io.close(session_id, result).await
}

/// 会话处在哪一步
enum Phase {
    /// 要请本机用户同意，在等控制端的确认（确认之前不问）
    Confirming,
    /// 在等本机用户回答
    Asking {
        answer: oneshot::Receiver<bool>,
        deadline: Instant,
        /// 占着的名额（[`ApprovalSlots`]）：不再等了就归还
        _slot: OwnedSemaphorePermit,
    },
    /// 开始了
    Open,
}

enum Answer {
    Allowed,
    Declined,
    Unanswered,
}

/// 等本机用户回答（会话开始之后一直等下去）
async fn answer(phase: &mut Phase) -> Answer {
    match phase {
        Phase::Asking {
            answer, deadline, ..
        } => match tokio::time::timeout_at(*deadline, answer).await {
            Ok(Ok(true)) => Answer::Allowed,
            Ok(_) => Answer::Declined,
            Err(_) => Answer::Unanswered,
        },
        Phase::Confirming | Phase::Open => std::future::pending().await,
    }
}

/// 会话的主循环（见模块说明）。正常结束（控制端断开、发了 Bye）时为 `Ok`。
/// `started`：会话开始了（发了 Hello）之后调用：这时才开始建直连（见 [`crate::direct`]），
/// 不把本机的地址告诉还没被同意的控制端
#[allow(clippy::too_many_arguments)]
async fn drive<P: Platform>(
    session_id: u64,
    admitted: &Admitted,
    platform: &mut P,
    out: &Outbox,
    incoming: &mut ControllerReceiver,
    kick: &mut watch::Receiver<ByeReason>,
    writer: &mut Writer,
    approvals: &ApprovalSlots,
    started: &(dyn Fn() + Sync),
) -> Result<()> {
    // 本机用户可能在端到端握手期间已经点了断开，不能先开始采集再处理这个通知。
    if kick.has_changed().unwrap_or(false) {
        out.send(&HostMsg::Bye(*kick.borrow_and_update()))?;
        return Ok(());
    }
    incoming.set_acks(platform.acks());
    if let Some(mic) = platform.mic() {
        incoming.set_mic(mic);
    }
    let mut start = Start {
        settings: admitted.settings,
        audio: admitted.audio,
        codecs: admitted.codecs.clone(),
    };
    // 不用同意的：不等确认就开始（见模块说明）
    let mut phase = if admitted.admission == Admission::NeedsApproval {
        Phase::Confirming
    } else {
        // 同意之前还可能收到画质/声音设置（见下面的 Request 分支），所以留一份
        platform.start(start.clone()).await?;
        info!(session_id, "session started");
        started();
        Phase::Open
    };
    let mut confirm = incoming.confirmation();
    let mut confirmed = false;
    let confirm_by = Instant::now() + CONFIRM_TIMEOUT;
    let mut watchdog = tokio::time::interval(WATCHDOG);
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // 被控端服务还在（踢人的发送端还在）
    let mut kickable = true;

    loop {
        if kick.has_changed().unwrap_or(false) {
            out.send(&HostMsg::Bye(*kick.borrow_and_update()))?;
            return Ok(());
        }
        let open = matches!(phase, Phase::Open);
        tokio::select! {
            c = &mut confirm, if !confirmed => {
                // 读任务没等到确认就结束了：会话的结果是读任务的错误（见 HostStreams::close）
                if c.is_err() {
                    return Ok(());
                }
                confirmed = true;
                info!(session_id, "the controller confirmed the handshake");
                platform.confirmed();
                if !matches!(phase, Phase::Confirming) {
                    continue;
                }
                // 在等本机用户回答的请求已经够多了：这一个不问（见 MAX_PENDING_APPROVALS）
                let Some(slot) = approvals.take() else {
                    info!(
                        session_id,
                        "too many connection requests are waiting for the local user, turning this one away"
                    );
                    out.send(&HostMsg::Bye(ByeReason::TooManyRequests))?;
                    return Ok(());
                };
                let answer = platform.ask(admitted);
                info!(session_id, safety = %admitted.safety, "asking the local user to allow the connection");
                out.send(&HostMsg::AwaitingApproval)?;
                phase = Phase::Asking {
                    answer,
                    deadline: Instant::now() + APPROVAL_TIMEOUT,
                    _slot: slot,
                };
            }
            () = tokio::time::sleep_until(confirm_by), if !confirmed => {
                bail!("the controller didn't confirm the handshake");
            }
            req = incoming.recv() => {
                let Some(req) = req else { return Ok(()) };
                match req {
                    req if open => platform.request(req).await?,
                    // 同意之前：记下画质和声音设置，同意之后一起生效；别的（输入、剪贴板……）都不执行
                    Request::Settings(s) => start.settings = s,
                    Request::Audio(a) => start.audio = a,
                    _ => {}
                }
            }
            a = answer(&mut phase) => {
                if let Answer::Allowed | Answer::Declined = a {
                    platform.answered(matches!(a, Answer::Allowed));
                }
                let reason = match a {
                    Answer::Allowed => {
                        info!(session_id, "the local user allowed the connection");
                        phase = Phase::Open;
                        // 同意之前还可能收到画质/声音设置（见下面的 Request 分支），所以留一份
        platform.start(start.clone()).await?;
                        info!(session_id, "session started");
                        started();
                        continue;
                    }
                    Answer::Declined => {
                        info!(session_id, "the local user declined the connection");
                        ByeReason::Declined
                    }
                    Answer::Unanswered => {
                        info!(session_id, "nobody answered the connection request");
                        ByeReason::Unanswered
                    }
                };
                out.send(&HostMsg::Bye(reason))?;
                return Ok(());
            }
            changed = kick.changed(), if kickable => {
                // 发送端不在了：被控端服务正在结束，会话随之被取消
                if changed.is_err() {
                    kickable = false;
                    continue;
                }
                let reason = *kick.borrow_and_update();
                info!(session_id, ?reason, "the local user disconnected the controller");
                out.send(&HostMsg::Bye(reason))?;
                return Ok(());
            }
            event = platform.next_event(), if open => platform.event(event).await?,
            e = writer.failed() => return Err(e),
            _ = watchdog.tick() => {
                if open && platform.video_stalled() {
                    bail!("the controller stopped acknowledging video");
                }
                if incoming.silent_for() > PEER_TIMEOUT {
                    bail!("the controller stopped responding");
                }
            }
        }
    }
}


