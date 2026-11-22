//! 合并跨通道的光标状态（控制通道）与图像（大块通道），换成交给界面的光标包（`nife_remote_proto::media::cursor_packet`）：
//! 三种界面不需要各写一份乱序处理。
use nife_remote_proto::media::{CursorSlot, CursorState};
use nife_remote_proto::session::{CURSOR_SLOTS, CursorRef, CursorShape, CursorVisibility};

struct Image {
    generation: u64,
    shape: CursorShape,
    delivered: bool,
}

pub(super) struct CursorReceiver {
    visibility: CursorVisibility,
    wanted: Option<CursorRef>,
    slots: Vec<Option<Image>>,
}

impl Default for CursorReceiver {
    fn default() -> Self {
        Self {
            visibility: CursorVisibility::Shown,
            wanted: None,
            slots: (0..CURSOR_SLOTS).map(|_| None).collect(),
        }
    }
}

impl CursorReceiver {
    pub fn update(
        &mut self,
        visibility: CursorVisibility,
        shape: Option<CursorRef>,
    ) -> CursorState {
        self.visibility = visibility;
        self.wanted = shape.filter(|r| r.slot < CURSOR_SLOTS);
        self.current()
    }

    pub fn image(&mut self, reference: CursorRef, image: CursorShape) -> Option<CursorState> {
        let slot = self.slots.get_mut(reference.slot as usize)?;
        // 先验证尺寸，不让坏图像进入前端缓存。
        let pixels = usize::from(image.width) * usize::from(image.height);
        if pixels == 0 || pixels.checked_mul(4)? != image.rgba.len() {
            return None;
        }
        if slot
            .as_ref()
            .is_some_and(|s| s.generation >= reference.generation)
        {
            return None;
        }
        *slot = Some(Image {
            generation: reference.generation,
            shape: image,
            delivered: false,
        });
        // 图像本身不改变可见性或选择；晚到的旧图像只能入缓存。
        (self.wanted == Some(reference)).then(|| self.current())
    }

    fn current(&mut self) -> CursorState {
        let shape = self.wanted.and_then(|r| {
            let image = self.slots.get_mut(r.slot as usize)?.as_mut()?;
            if image.generation != r.generation {
                return None;
            }
            let data = (!image.delivered).then(|| image.shape.clone());
            image.delivered = true;
            Some(CursorSlot {
                slot: r.slot,
                image: data,
            })
        });
        CursorState {
            visibility: self.visibility,
            shape,
        }
    }
}


