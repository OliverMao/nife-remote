//! 剪贴板文本双向同步（被控端与控制端共用）。
//!
//! * 检测到变化后才读取内容：用 `GetClipboardSequenceNumber`（不需要读取内容，几乎零开销）；
//! * 记录"双方已一致"的文本哈希，抑制回声（A 写入 B → B 检测到变化 → 又发回 A）。

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use nife_remote_proto::session::MAX_CLIPBOARD_TEXT;

/// 轮询间隔（被控端与控制端相同）
pub const POLL: Duration = nife_remote_link::controller::CLIPBOARD_POLL;

pub fn sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

/// 在剪贴板对象上执行操作。每次新建一个实例（Windows 上剪贴板不属于某个进程）。
fn with_clipboard<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, arboard::Error> {
    f(&mut arboard::Clipboard::new()?)
}

pub fn get_text() -> Result<String, arboard::Error> {
    with_clipboard(|c| c.get_text())
}

pub fn set_text(text: &str) -> Result<(), arboard::Error> {
    with_clipboard(|c| c.set_text(text))
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    // 0 保留为"未同步"
    h.finish() | 1
}

fn read_text() -> Result<Option<String>, arboard::Error> {
    for attempt in 0..3 {
        match get_text() {
            Ok(t) => return Ok(Some(t)),
            Err(arboard::Error::ContentNotAvailable) => return Ok(None),
            Err(e) if attempt == 2 => return Err(e),
            // 其它程序正占用剪贴板，稍后重试；失败不能被缓存成这个序号没有文本。
            Err(_) => std::thread::sleep(Duration::from_millis(40)),
        }
    }
    unreachable!()
}

fn write_text(t: &str) -> bool {
    for _ in 0..3 {
        if set_text(t).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

/// 整个进程只有一个写入任务。正在写一项时，其余更新只保留最新的一项；
/// 不让不同会话或连续更新创建无界的阻塞任务，也不让旧写入晚于新写入完成。
#[derive(Default)]
struct ClipboardWriter(Mutex<Writes>);

#[derive(Default)]
struct Writes {
    running: bool,
    pending: Option<Update>,
}

struct Update {
    text: String,
    synced: Weak<AtomicU64>,
}

impl ClipboardWriter {
    fn busy(&self) -> bool {
        self.0.lock().unwrap().running
    }

    /// true 表示调用方需要启动唯一的写任务。
    fn push(&self, text: String, synced: &Arc<AtomicU64>) -> bool {
        let mut state = self.0.lock().unwrap();
        state.pending = Some(Update {
            text,
            synced: Arc::downgrade(synced),
        });
        !std::mem::replace(&mut state.running, true)
    }

    fn drain(&self, mut write: impl FnMut(&str) -> bool) {
        loop {
            let update = {
                let mut state = self.0.lock().unwrap();
                match state.pending.take() {
                    Some(update) => update,
                    None => {
                        state.running = false;
                        return;
                    }
                }
            };
            // 会话已经结束的、尚未开始的写入不再执行。
            let Some(synced) = update.synced.upgrade() else {
                continue;
            };
            if write(&update.text) {
                synced.store(hash(&update.text), Ordering::Release);
            } else {
                tracing::debug!("writing the clipboard failed");
            }
        }
    }
}

fn writer() -> &'static Arc<ClipboardWriter> {
    static WRITER: OnceLock<Arc<ClipboardWriter>> = OnceLock::new();
    WRITER.get_or_init(Arc::default)
}

/// 读取也只允许一个后台任务。所有会话共享按剪贴板序号缓存的结果；
/// poll 只取已经完成的结果，慢的系统调用不能暂停会话或堆积阻塞任务。
#[derive(Default)]
struct ClipboardReader(Mutex<Reads>);

type ReadResult = Result<Option<Arc<str>>, ()>;

#[derive(Default)]
struct Reads {
    latest: Option<(u32, Option<Arc<str>>)>,
    pending: Option<(u32, tokio::sync::oneshot::Receiver<ReadResult>)>,
}

impl ClipboardReader {
    fn poll(
        &self,
        seq: u32,
        read: impl FnOnce() -> Result<Option<String>, arboard::Error> + Send + 'static,
    ) -> Option<Option<Arc<str>>> {
        let mut state = self.0.lock().unwrap();
        if let Some((at, rx)) = &mut state.pending {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    let at = *at;
                    state.latest = Some((at, text));
                    state.pending = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Ok(Err(())) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    state.pending = None
                }
            }
        }
        if let Some((at, text)) = &state.latest
            && *at == seq
        {
            return Some(text.clone());
        }
        if state.pending.is_none() {
            let (tx, rx) = tokio::sync::oneshot::channel();
            state.pending = Some((seq, rx));
            tokio::task::spawn_blocking(move || {
                let text = read()
                    .map(|text| {
                        text.filter(|t| !t.is_empty() && t.len() <= MAX_CLIPBOARD_TEXT)
                            .map(Arc::<str>::from)
                    })
                    .map_err(|_| ());
                let _ = tx.send(text);
            });
        }
        None
    }
}

fn reader() -> &'static ClipboardReader {
    static READER: OnceLock<ClipboardReader> = OnceLock::new();
    READER.get_or_init(ClipboardReader::default)
}

/// 单个会话的同步状态。
pub struct ClipSync {
    last_seq: u32,
    synced: Arc<AtomicU64>,
}

impl ClipSync {
    pub fn new() -> Self {
        // 从"当前状态"开始，只同步之后的变化
        Self {
            last_seq: sequence(),
            synced: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 本机剪贴板有新文本（且不是对端刚写入的）时返回它。
    pub fn poll(&mut self) -> Option<String> {
        // 写入过程中不读取中间状态，否则 A、B 连续更新时会把短暂的 A 当成本机变化回传。
        if writer().busy() {
            return None;
        }
        let seq = sequence();
        if seq == self.last_seq {
            return None;
        }
        let text = reader().poll(seq, read_text)?;
        if writer().busy() || sequence() != seq {
            return None;
        }
        self.last_seq = seq;
        let text = text?;
        let h = hash(&text);
        if self.synced.swap(h, Ordering::AcqRel) == h {
            return None;
        }
        Some(text.to_string())
    }

    /// 串行写入对端的最新文本。只有写入成功才更新同步哈希；写入期间 poll 不回传中间状态。
    pub fn apply(&self, text: String) {
        if text.len() > MAX_CLIPBOARD_TEXT {
            return;
        }
        let writer = writer();
        if !writer.busy() && self.synced.load(Ordering::Acquire) == hash(&text) {
            return;
        }
        if writer.push(text, &self.synced) {
            let writer = writer.clone();
            tokio::task::spawn_blocking(move || writer.drain(write_text));
        }
    }
}

/// 控制端会话用它同步本机剪贴板（见 `nife_remote_link::controller::Frontend::Desktop`）
impl nife_remote_link::controller::ClipboardSync for ClipSync {
    fn poll(&mut self) -> Option<String> {
        ClipSync::poll(self)
    }

    fn apply(&self, text: String) {
        ClipSync::apply(self, text)
    }
}
