//! 声音数据报的冗余（格式见 `nife_remote_proto::media` 的"声音数据报"）：发的一方每个数据报带上之前几包的副本
//! （[`Bundler`]），收的一方拆开、按采集时刻只留没收过的（[`Unbundler`]）。丢了的包这样就补回来了，不用等重传；
//! 乱序到的数据报也一样：晚到的那包在后一个数据报里已经有了。被控端的系统声音（被控端发、控制端收）和
//! 控制端的麦克风（控制端发、被控端收）都这样。

use std::collections::VecDeque;

use nife_remote_proto::framing;
use nife_remote_proto::handshake::SEAL_OVERHEAD;
use nife_remote_proto::media::{self, AudioHeader};
use bytes::Bytes;

/// 每个声音数据报最多带上之前这么多包的副本（放得下的话）：连着丢了这么多个数据报也补得回来。
/// 一包 Opus 约 240 字节，一个数据报三包约 0.8 KB，每秒 50 个约 40 KB（320 kbps），和画面比可以忽略
pub(crate) const REDUNDANCY: usize = 2;

/// 发的一方：拼声音数据报的明文（一个会话一个）
#[derive(Default)]
pub(crate) struct Bundler {
    /// 最近发出去的包（不带长度前缀），从新到旧
    recent: VecDeque<Bytes>,
}

impl Bundler {
    /// 这一包（[`media::frame`] 编好的，带长度前缀）连同之前几包的副本：要加密发出去的数据报明文。
    /// 单独一包都放不进一个数据报（[`media::MAX_AUDIO_PACKET`]）时为 `None`，这一包丢掉
    pub fn bundle(&mut self, framed: Bytes) -> Option<Vec<u8>> {
        self.bundle_packet(framed.slice(framing::PREFIX.min(framed.len())..))
    }

    /// 同 [`Bundler::bundle`]，这一包不带长度前缀（控制端的麦克风：界面打好的包）
    pub fn bundle_packet(&mut self, packet: Bytes) -> Option<Vec<u8>> {
        // 静音之后的第一包：之前的那几包是上一段声音的，补过去就是在新的声音前面放一段过时的
        if AudioHeader::decode(&packet).is_none_or(|h| h.start) {
            self.recent.clear();
        }
        let plain = media::audio_datagram(&packet, self.recent.iter().map(|p| &p[..]))?;
        self.recent.push_front(packet);
        self.recent.truncate(REDUNDANCY);
        Some(plain)
    }
}

/// 收的一方：拆开声音数据报的明文（一个会话一个）
#[derive(Default)]
pub(crate) struct Unbundler {
    order: AudioOrder,
}

impl Unbundler {
    /// 解密后的数据报明文里没收过的声音包，按采集先后放进 `out`（每个多留了加密开销的容量：
    /// 和原地解密的画面包一样，交给界面时可以在原缓冲里加东西，见 `console::ConsoleUi`）。
    /// 返回其中有几包是补回来的（之前的数据报丢了、或者还没到）。格式不对的整个丢掉
    pub fn unbundle(&mut self, plain: &[u8], out: &mut VecDeque<Vec<u8>>) -> usize {
        let Some(packets) = media::audio_packets(plain) else {
            tracing::debug!(len = plain.len(), "dropped a malformed audio datagram");
            return 0;
        };
        let newest = packets.len() - 1;
        let mut recovered = 0;
        for (i, p) in packets.into_iter().enumerate() {
            // 不是声音包的（以后的版本可能放别的）、副本、迟到的都不要
            if !self.order.accept(p) {
                continue;
            }
            if i < newest {
                recovered += 1;
            }
            let mut packet = Vec::with_capacity(p.len() + SEAL_OVERHEAD);
            packet.extend_from_slice(p);
            out.push_back(packet);
        }
        recovered
    }
}

/// 数据报可能乱序：采集时刻在一个会话内不倒退，即使采集器重启、序号从头算也一样。
/// `start` 只描述静音段边界，不能让迟到的首包重置顺序。相同采集时刻用序号消歧（允许绕回）。
#[derive(Default)]
struct AudioOrder {
    last: Option<AudioHeader>,
}

impl AudioOrder {
    fn accept(&mut self, packet: &[u8]) -> bool {
        let Some(h) = AudioHeader::decode(packet) else {
            return false;
        };
        if let Some(last) = self.last
            && (h.capture_us < last.capture_us
                || (h.capture_us == last.capture_us && h.seq.wrapping_sub(last.seq) as i32 <= 0))
        {
            return false;
        }
        self.last = Some(h);
        true
    }
}


