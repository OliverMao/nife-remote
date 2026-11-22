//! 帧率节流（采集线程按订阅者里最高的帧率，编码线程按自己会话的帧率）：按**画面的时间戳**排一条时间线，
//! 每处理一帧往后推一个周期；可以提前 1/4 个周期，按时到的画面不用等。
//!
//! 以前是"处理完这一帧，过 1/帧率 再处理下一帧"。采集本来就按这个帧率出画面时（60 Hz 的屏配 60 fps；
//! ），每帧多出一点处理和睡过头的时间，下一帧要等的一帧比一帧久，
//! 等满一帧就跳掉一帧再从头来：平均多出半帧的延迟，还每隔一阵顿一下。现在时间线跟着画面的时间戳走，
//! 和处理花了多久无关，不会累积。
//!
//! * 时间线落后了（画面静止了一阵、编码慢了）：从这一帧重新排，不会为了补上而连着处理好几帧；
//! * 画面比时间线早（采集的抖动，屏幕比设定的帧率快一点点，例如 60.001 Hz）：时间线每帧最多往前挪 1/64 个周期，
//!   跟得上这样的屏幕，不会一直落后它、越等越久；长期的帧率最多比设定的高 1/64。

use std::time::{Duration, Instant};

pub struct Pacer {
    period: Duration,
    /// 时间线上下一帧的位置
    next: Option<Instant>,
}

impl Pacer {
    pub fn new(fps: u8) -> Self {
        Self {
            period: period(fps),
            next: None,
        }
    }

    pub fn set_fps(&mut self, fps: u8) {
        self.period = period(fps);
    }

    /// 现在（`now`）还要等多久才能处理下一帧
    pub fn wait(&self, now: Instant) -> Duration {
        self.next
            .and_then(|next| next.checked_sub(self.period / 4))
            .map_or(Duration::ZERO, |earliest| {
                earliest.saturating_duration_since(now)
            })
    }

    /// 处理了时间戳为 `stamp` 的一帧（采集的时刻；重新编码的旧画面用现在）
    pub fn done(&mut self, stamp: Instant) {
        let base = match self.next {
            Some(next) => stamp.max(next.checked_sub(self.period / 64).unwrap_or(next)),
            None => stamp,
        };
        self.next = Some(base + self.period);
    }
}

fn period(fps: u8) -> Duration {
    Duration::from_secs(1) / fps.max(1) as u32
}


