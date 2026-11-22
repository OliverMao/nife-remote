//! 控制端缓存了哪些光标形状（见 [`nife_remote_proto::session::CURSOR_SLOTS`]）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{Outbox, SendError};
use nife_remote_proto::session::{CURSOR_SLOTS, CursorRef, CursorShape, CursorVisibility, HostMsg};

/// 槽位里是哪个形状
#[derive(Clone, Copy)]
struct Entry {
    /// 形状内容的哈希
    hash: u64,
    /// 最近一次用到的时刻（换掉最久没用的那个）
    used: u64,
    /// 这个槽位的第几代图像（槽位复用时变大，见 [`CursorRef`]）
    generation: u64,
}

/// 发给一个控制端的光标形状缓存：控制端按槽位存形状（[`CURSOR_SLOTS`] 个），这里记着每个槽位里是什么。
/// 只在一个会话里有效，会话里换显示器、重建编码管线时接着用。
///
/// 槽位满了换掉最久没用到的那个。用形状内容的 64 位哈希认形状，几十个形状里撞上的概率可以忽略
pub struct CursorCache {
    slots: Vec<Option<Entry>>,
    clock: u64,
    /// 控制端现在用的形状（发出去了图像的）
    current: Option<CursorRef>,
}

impl Default for CursorCache {
    fn default() -> Self {
        Self {
            slots: vec![None; CURSOR_SLOTS as usize],
            clock: 0,
            current: None,
        }
    }
}

impl CursorCache {
    /// 发送光标的最新状态：控制端还没有的形状先把图像排进大块通道，可见性和形状引用走控制通道（只留最新值）。
    /// 大块通道排满了（返回 `Err(Full)`）时可见性照样发出去，形状保持上一个，图像下一轮再试
    pub fn send(
        &mut self,
        out: &Outbox,
        visibility: CursorVisibility,
        shape: Option<&CursorShape>,
    ) -> Result<(), SendError> {
        let mut result = Ok(());
        if let Some(shape) = shape {
            let (reference, new) = self.place(shape);
            if new {
                result = out.send(&HostMsg::CursorImage {
                    reference,
                    image: shape.clone(),
                });
                if result.is_err() {
                    // 控制端没收到它：下次还得带图像
                    self.slots[reference.slot as usize] = None;
                }
            }
            if result.is_ok() {
                self.current = Some(reference);
            }
        }
        out.send(&HostMsg::Cursor {
            visibility,
            shape: self.current,
        })?;
        result
    }

    /// 光标换成了 `shape`：它在哪个槽位，控制端是不是还没有它（要发图像）
    fn place(&mut self, shape: &CursorShape) -> (CursorRef, bool) {
        let mut h = DefaultHasher::new();
        shape.hash(&mut h);
        let hash = h.finish();
        self.clock += 1;
        let reference = |slot: usize, e: &Entry| CursorRef {
            slot: slot as u8,
            generation: e.generation,
        };
        if let Some(i) = self
            .slots
            .iter()
            .position(|s| s.is_some_and(|e| e.hash == hash))
        {
            let e = self.slots[i].as_mut().expect("found above");
            e.used = self.clock;
            return (reference(i, e), false);
        }
        let i = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                (0..self.slots.len())
                    .min_by_key(|&i| self.slots[i].map_or(0, |e| e.used))
                    .unwrap_or(0)
            });
        let e = Entry {
            hash,
            used: self.clock,
            generation: self.clock,
        };
        self.slots[i] = Some(e);
        (reference(i, &e), true)
    }
}


