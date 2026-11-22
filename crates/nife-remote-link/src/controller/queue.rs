//! 控制端发给被控端的命令队列（有界），建在 [`crate::queue`] 上（和被控端的 `Outbox` 一样）。
//!
//! * 只关心最新值的画质设置不排队：它占一个槽位，只留最新的一个，写任务**先发它**，再按顺序发排着队的
//!   输入和其它命令。Ping、画面确认不在这里：它们是控制数据报（见 `super::session::Feedback`），不排在输入后面；
//! * 连续的绝对移动只留最后一个，连续的相对移动加起来；移动只和紧挨着的移动合并，
//!   不会越过点击、按键（松开按键不会被合并掉）；
//! * 排着的输入太旧了（网络卡了一阵子，[`INPUT_MAX_AGE`]），或者队列满了：丢掉排着的输入、
//!   补发一个"释放所有按键"（丢掉的里面可能有松开按键），会话照常继续——不在网络恢复时把一串过时的远程操作补发出去，
//!   也不因为网络卡了一下就断开重连。真正失联由会话的超时处理。
//!   丢掉输入之后还是放不下（例如排满了不能丢的命令）才算会话失败（[`Sender::failed`]）。
//!
//! 这里只管得了还在队列里的输入：断网时往 QUIC 里写不会阻塞，写出去的输入进了 QUIC 的发送缓冲，
//! 网络恢复后照样发出去。所以输入还带着排进队列的时刻（[`ControllerMsg::Input`] 的 `t`），
//! 被控端丢掉晚到太久的（见 `nife_remote_proto::session` 的"过时的输入"）。
use crate::Problem;
use crate::clock::now_us;
use crate::queue::{Item, Limit, Mail, Mailbox};
use crate::transport::Frames;
use nife_remote_proto::{
    framing,
    session::{ControllerMsg, INPUT_MAX_AGE, InputEvent},
};
use bytes::Bytes;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing::warn;

const LIMIT: Limit = Limit {
    messages: 128,
    bytes: 2 * 1024 * 1024,
};

/// 排队的东西是什么
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// 其它输入（点击、按键、文字……）
    Input,
    /// 只有绝对移动：只留最后一个
    Move,
    /// 只有相对移动：加起来
    Delta,
    /// 不是输入的命令（切换显示器、请求关键帧、剪贴板……）：不会过时，不丢
    Command,
}

impl Kind {
    fn is_input(self) -> bool {
        self != Kind::Command
    }
}

/// 排着的一条命令的来历（合并、丢掉过时的输入时用）
struct Meta {
    kind: Kind,
    /// 相对移动的总量（合并时用）
    delta: (i32, i32),
    at: Instant,
    /// 输入里的 `t`（合并相对移动时用最早的那一个）
    t: u64,
}

/// 只留最新值的槽位：画质设置
const SETTINGS: usize = 0;
const SLOTS: usize = 1;

struct Shared {
    mail: Mailbox<Meta>,
    /// 排满了不能丢的命令：会话失败
    failed: AtomicBool,
}

#[derive(Clone)]
pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);

pub(super) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        mail: Mailbox::new(SLOTS),
        failed: AtomicBool::new(false),
    });
    (Sender(shared.clone()), Receiver(shared))
}

/// 编码（明文，带长度前缀）；写任务加密时在后面追加认证标签，这里先留好位置
fn encode(msg: &ControllerMsg) -> Result<Bytes, Problem> {
    let bytes = framing::encode_with_spare(msg, nife_remote_proto::handshake::SEAL_OVERHEAD);
    if msg.lane() != nife_remote_proto::SessionLane::Control
        || bytes.len() - framing::PREFIX + nife_remote_proto::handshake::SEAL_OVERHEAD
            > nife_remote_proto::MAX_CONTROL_MSG
    {
        return Err(Problem::failed(
            "controller message exceeds its lane budget",
        ));
    }
    Ok(Bytes::from(bytes))
}

/// 一批输入，`t` 是现在（界面的输入都从这里进来）
pub(super) fn input(events: Vec<InputEvent>) -> ControllerMsg {
    ControllerMsg::Input {
        t: now_us(),
        events,
    }
}

fn release_msg(t: u64) -> ControllerMsg {
    ControllerMsg::Input {
        t,
        events: vec![InputEvent::ReleaseAll],
    }
}

fn delta_msg(t: u64, (dx, dy): (i32, i32)) -> ControllerMsg {
    ControllerMsg::Input {
        t,
        events: vec![InputEvent::MouseDelta { dx, dy }],
    }
}

/// 丢掉排着的输入，补一个"释放所有按键"排在最前面。返回丢掉了几条（没有可丢的就什么也不做）
fn drop_input(m: &mut Mail<Meta>) -> usize {
    let dropped = m.retain(|i| !i.meta.kind.is_input());
    if dropped > 0 {
        // 现在的时刻：被控端不会把它当成过时的输入
        let t = now_us();
        let release = encode(&release_msg(t)).expect("ReleaseAll always encodes");
        m.push_front(
            release,
            Meta {
                kind: Kind::Input,
                delta: (0, 0),
                at: Instant::now(),
                t,
            },
        );
    }
    dropped
}

// CBOR 的固定开销按上限估计，给消息外层和 AEAD 标签留空间。
const INPUT_BUDGET: usize = nife_remote_proto::MAX_CONTROL_MSG - 128;
fn input_cost(event: &InputEvent) -> usize {
    64 + match event {
        InputEvent::Text { text } => text.len(),
        InputEvent::Key { code, .. } => code.len(),
        _ => 0,
    }
}

impl Sender {
    pub fn send(&self, msg: ControllerMsg) -> Result<(), Problem> {
        let ControllerMsg::Input { t, mut events } = msg else {
            return self.send_one(msg);
        };
        InputEvent::coalesce(&mut events);
        if events.is_empty() {
            return Ok(());
        }
        if events.iter().map(input_cost).sum::<usize>() <= INPUT_BUDGET {
            return self.send_one(ControllerMsg::Input { t, events });
        }
        let mut batch = Vec::new();
        let mut size = 0;
        let mut push = |event: InputEvent| -> Result<(), Problem> {
            let cost = input_cost(&event);
            if cost > INPUT_BUDGET {
                return Err(Problem::failed(
                    "input event exceeds the control lane budget",
                ));
            }
            if size + cost > INPUT_BUDGET {
                self.send_one(ControllerMsg::Input {
                    t,
                    events: std::mem::take(&mut batch),
                })?;
                size = 0;
            }
            size += cost;
            batch.push(event);
            Ok(())
        };
        for event in events {
            match event {
                InputEvent::Text { text } if text.len() + 64 > INPUT_BUDGET => {
                    let mut rest = text.as_str();
                    while !rest.is_empty() {
                        let mut end = rest.len().min(INPUT_BUDGET - 64);
                        while !rest.is_char_boundary(end) {
                            end -= 1;
                        }
                        push(InputEvent::Text {
                            text: rest[..end].to_owned(),
                        })?;
                        rest = &rest[end..];
                    }
                }
                event => push(event)?,
            }
        }
        if !batch.is_empty() {
            self.send_one(ControllerMsg::Input { t, events: batch })?;
        }
        Ok(())
    }

    /// 队列失败了（排满了不能丢的命令）
    pub fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Relaxed)
    }

    fn send_one(&self, msg: ControllerMsg) -> Result<(), Problem> {
        let slot = matches!(msg, ControllerMsg::Settings(_)).then_some(SETTINGS);
        let (kind, delta, t) = match &msg {
            ControllerMsg::Input { t, events } => match events[..] {
                [InputEvent::MouseMove { .. }] => (Kind::Move, (0, 0), *t),
                [InputEvent::MouseDelta { dx, dy }] => (Kind::Delta, (dx, dy), *t),
                _ => (Kind::Input, (0, 0), *t),
            },
            _ => (Kind::Command, (0, 0), 0),
        };
        let mut bytes = encode(&msg)?;
        let mail = &self.0.mail;
        let mut m = mail.lock();
        if m.closed {
            return Err(Problem::SessionEnded);
        }
        if let Some(slot) = slot {
            m.set(slot, bytes);
            drop(m);
            mail.wake();
            return Ok(());
        }
        let mut meta = Meta {
            kind,
            delta,
            at: Instant::now(),
            t,
        };
        // 移动只和紧挨着的移动合并（队尾只会是输入或命令，确认这类状态不在队列里）
        if matches!(kind, Kind::Move | Kind::Delta) && m.back().is_some_and(|i| i.meta.kind == kind)
        {
            let old = m.pop_back().expect("checked above").meta;
            if kind == Kind::Delta {
                meta.delta = (
                    old.delta.0.saturating_add(delta.0),
                    old.delta.1.saturating_add(delta.1),
                );
                // 合并之后的移动从最早的那一部分算起，过时了一起丢（这里和被控端都是）
                meta.at = old.at;
                meta.t = old.t;
                bytes = encode(&delta_msg(meta.t, meta.delta))?;
            }
        }
        if m.full(LIMIT, bytes.len()) {
            // 网络卡住了：排着的输入已经没有意义，丢掉它们再放这一条
            let dropped = drop_input(&mut m);
            warn!(
                dropped,
                "controller command queue full, dropped the queued input"
            );
            if m.full(LIMIT, bytes.len()) {
                self.0.failed.store(true, Ordering::Relaxed);
                m.close();
                drop(m);
                mail.wake();
                return Err(Problem::failed("controller command queue overflow"));
            }
        }
        m.push_back(Item {
            bytes,
            before: None,
            meta,
        });
        drop(m);
        mail.wake();
        Ok(())
    }
}

impl Receiver {
    /// 写任务要发的下一帧；队列失败了、会话结束了时为 `None`
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.0
            .mail
            .recv(|m| {
                // 最早的输入已经过时：排着的输入全都丢掉（后面的也是在网络卡住时攒下的）
                if m.items()
                    .find(|i| i.meta.kind.is_input())
                    .is_some_and(|i| i.meta.at.elapsed() > INPUT_MAX_AGE)
                {
                    let dropped = drop_input(m);
                    warn!(
                        dropped,
                        "dropped stale input queued for more than {INPUT_MAX_AGE:?}"
                    );
                }
            })
            .await
    }
}

impl Frames for Receiver {
    async fn next(&mut self) -> Option<Bytes> {
        self.recv().await
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.0.mail.close();
    }
}


