//! 注入线程：会话把输入交给它就接着处理别的，不等注入完。
//!
//! 注入是同步的：注入一批输入可能要等系统处理完，每个会话都要等 10–20 ms（见 `x11.rs`），
//! 一长段文字要好几秒。在会话的任务里直接注入的话，这期间会话什么也处理不了：Pong 发不出去，画面的确认没人处理
//! （在途帧窗口停住，画面卡住），本机用户断开控制端也要等它输完。读任务还会被堵住，后面的输入读得晚了，
//! 被 `InputClock` 当成网络上耽搁的过时输入丢掉——超长的文字输到一半变成了"释放所有按键"。
//!
//! 所以每个会话一个线程，[`Injector`] 在这个线程上创建、使用、释放：
//!
//! * 输入按交来的顺序排队。排着的有上限（[`MAX_QUEUED`]）：排不下的整批丢掉，并在那个位置松开所有按键
//!   （丢掉的里面可能有松开按键），排着的都注入完了才接着收——不会丢一批收一批、文字中间一块一块地缺；
//! * 会话结束（[`InjectorThread`] 被丢掉）时排着的不再注入，松开所有按键后线程结束。很长的一段文字分成小段交给
//!   注入器（[`TEXT_PIECE`]），每段之间看一眼会话是不是结束了：本机用户断开控制端之后，最多再输入一小段。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use nife_remote_proto::session::{DisplayInfo, InputEvent};
use tracing::warn;

use super::Injector;

/// 排着还没注入完的输入最多占这么多字节（见 [`cost`]）。平时线程跟得上，排着的不到一批；
/// 只有文字输得慢的时候才会排起来
const MAX_QUEUED: usize = 4 << 20;

/// 一次交给注入器的文字最多这么多个字符。每个事件、每段文字之间都检查取消，
/// 包括前端已经分好的小段：不能把一整批小段重新合成一次不可取消的调用。
const TEXT_PIECE: usize = 512;

/// 注入器：各平台的 [`Injector`]（测试里是假的）
pub(super) trait Inject {
    fn handle(&mut self, events: &[InputEvent], display: &DisplayInfo);
    fn release_all(&mut self);
}

impl Inject for Injector {
    fn handle(&mut self, events: &[InputEvent], display: &DisplayInfo) {
        Injector::handle(self, events, display);
    }

    fn release_all(&mut self) {
        Injector::release_all(self);
    }
}

enum Job {
    Input {
        events: Vec<InputEvent>,
        display: DisplayInfo,
        /// 这一批的 [`cost`]
        cost: usize,
    },
    ReleaseAll,
}

/// 会话和注入线程都看得到的
struct Shared {
    /// 排着的（包括正在注入的那一批）占了多少字节
    queued: AtomicUsize,
    /// 会话结束了：排着的不再注入
    closed: AtomicBool,
}

/// 一个会话的注入线程（见模块说明）。丢掉时松开所有按键
pub struct InjectorThread {
    /// 线程起不来、或者意外结束了时为 `None`：之后的输入不再执行
    tx: Option<mpsc::Sender<Job>>,
    shared: Arc<Shared>,
    /// 排满了，正在丢输入（已经排了一个"松开所有按键"）
    dropping: bool,
}

impl InjectorThread {
    pub fn new() -> Self {
        Self::spawn(Injector::new)
    }

    /// `make` 在注入线程上调用：注入器不用能在线程之间传
    fn spawn<I: Inject>(make: impl FnOnce() -> I + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            queued: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        });
        let sh = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("nife-remote-inject".into())
            .spawn(move || run(make(), rx, &sh));
        if let Err(e) = &spawned {
            warn!("can't start the input thread, this controller's input will be ignored: {e}");
        }
        Self {
            tx: spawned.is_ok().then_some(tx),
            shared,
            dropping: false,
        }
    }

    /// 把一批输入排给注入线程，不等它注入。`display` 是现在的画面对应的显示器（坐标换算用）
    pub fn handle(&mut self, events: Vec<InputEvent>, display: &DisplayInfo) {
        if self.tx.is_none() {
            return;
        }
        let cost = cost(&events);
        let queued = self.shared.queued.load(Ordering::Acquire);
        // 排满过：排着的都注入完了才接着收
        let room = !self.dropping || queued == 0;
        if !room || queued.saturating_add(cost) > MAX_QUEUED {
            if !self.dropping {
                warn!(
                    queued,
                    "input arrives faster than it can be injected, dropping it until the backlog is gone"
                );
                self.dropping = true;
                // 丢掉的里面可能有松开按键：排在前面的都注入完之后，松开所有按键
                self.send(Job::ReleaseAll);
            }
            return;
        }
        self.dropping = false;
        self.shared.queued.fetch_add(cost, Ordering::AcqRel);
        self.send(Job::Input {
            events,
            display: display.clone(),
            cost,
        });
    }

    /// 排在前面的输入都注入完之后，松开所有按着的键
    pub fn release_all(&mut self) {
        self.send(Job::ReleaseAll);
    }

    /// 马上取消尚未注入的事件，并唤醒空闲线程。停止编码器之前就应调用。
    pub fn cancel(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.tx = None;
    }

    fn send(&mut self, job: Job) {
        if let Some(tx) = &self.tx
            && tx.send(job).is_err()
        {
            warn!("the input thread is gone, this controller's input will be ignored");
            self.tx = None;
        }
    }
}

impl Drop for InjectorThread {
    fn drop(&mut self) {
        // 排着的不再注入。发送端随之丢掉，线程醒来后松开所有按键、结束（不在这里等它：这里是会话的任务）
        self.shared.closed.store(true, Ordering::Release);
    }
}

/// 一批输入排着的时候大约占多少字节
fn cost(events: &[InputEvent]) -> usize {
    events
        .iter()
        .map(|e| {
            size_of::<InputEvent>()
                + match e {
                    InputEvent::Text { text } => text.len(),
                    InputEvent::Key { code, .. } => code.len(),
                    _ => 0,
                }
        })
        .sum()
}

/// 注入线程：按顺序注入，直到会话结束
fn run<I: Inject>(mut injector: I, jobs: mpsc::Receiver<Job>, shared: &Shared) {
    while let Ok(job) = jobs.recv() {
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        match job {
            Job::Input {
                events,
                display,
                cost,
            } => {
                inject(&mut injector, &events, &display, shared);
                shared.queued.fetch_sub(cost, Ordering::AcqRel);
            }
            Job::ReleaseAll => injector.release_all(),
        }
    }
    injector.release_all();
}

/// 注入一批输入。太长的文字（见 [`TEXT_PIECE`]）分成小段，会话结束了就不再往下输
fn inject<I: Inject>(
    injector: &mut I,
    events: &[InputEvent],
    display: &DisplayInfo,
    shared: &Shared,
) {
    for event in events {
        if shared.closed.load(Ordering::Acquire) {
            return;
        }
        let InputEvent::Text { text } = event else {
            injector.handle(std::slice::from_ref(event), display);
            continue;
        };
        let mut rest = text.as_str();
        while !rest.is_empty() {
            if shared.closed.load(Ordering::Acquire) {
                return;
            }
            let end = rest
                .char_indices()
                .nth(TEXT_PIECE)
                .map_or(rest.len(), |(at, _)| at);
            let (piece, tail) = rest.split_at(end);
            if piece.len() == text.len() {
                injector.handle(std::slice::from_ref(event), display);
            } else {
                injector.handle(&[InputEvent::Text { text: piece.into() }], display);
            }
            rest = tail;
        }
    }
}


