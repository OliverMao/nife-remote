//! 控制端的输入晚到了多久（见 `nife_remote_proto::session` 的"过时的输入"）。

use std::time::{Duration, Instant};

use nife_remote_proto::session::INPUT_MAX_AGE;
use tracing::warn;

/// 判断控制端的输入晚到了多久（见 `nife_remote_proto::session` 的"过时的输入"）。
///
/// 两边的时钟不同步，只能比"比平时晚了多少"：`收到的时刻 − t` 是两边时钟之差加上这一路的耽搁，
/// 记下它的最小值当作基准（时钟之差 + 最快的那一次），超出基准的部分就是这条消息额外耽搁了多久——
/// 在控制端的队列里、在 QUIC 的发送缓冲里（断网时往里写不会阻塞）、在服务器上。Ping 和输入都带着 `t`，都算样本。
///
/// * 基准每秒最多上调 1 ms（[`InputClock::DRIFT`]），跟得上两边时钟的漂移（通常不到 0.01%），
///   断网再久，恢复后涌来的旧消息也远远超出基准；
/// * 连续 [`InputClock::RESYNC`] 收到的全是晚到的消息，说明这一路本身变慢了（例如换了网络），而不是积压：
///   以现在为准重新取基准，不能一直丢下去。
pub struct InputClock {
    epoch: Instant,
    /// 基准（微秒）和取它的时刻
    base: Option<(i64, Instant)>,
    /// 从什么时候起收到的全是晚到的消息
    late_since: Option<Instant>,
}

impl Default for InputClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            base: None,
            late_since: None,
        }
    }
}

impl InputClock {
    /// 基准每过这么多微秒最多上调 1 微秒
    const DRIFT: i64 = 1000;
    const RESYNC: Duration = Duration::from_secs(5);

    /// 记下一条带着控制端时刻 `t` 的消息（Ping、输入），`now` 是收到它的时刻。返回它是不是过时了（只对输入有意义）
    pub fn stale(&mut self, t: u64, now: Instant) -> bool {
        let offset = now.saturating_duration_since(self.epoch).as_micros() as i64 - t as i64;
        let base = match self.base {
            Some((b, at)) => {
                (b + now.saturating_duration_since(at).as_micros() as i64 / Self::DRIFT).min(offset)
            }
            None => offset,
        };
        let late = Duration::from_micros((offset - base) as u64) > INPUT_MAX_AGE;
        let since = match (late, self.late_since) {
            (false, _) => None,
            (true, since) => Some(since.unwrap_or(now)),
        };
        if since.is_some_and(|s| now.saturating_duration_since(s) > Self::RESYNC) {
            warn!(
                "input has been arriving late for {:?}, the network path got slower: resynchronizing",
                Self::RESYNC
            );
            self.base = Some((offset, now));
            self.late_since = None;
            return false;
        }
        self.base = Some((base, now));
        self.late_since = since;
        late
    }
}


