//! 长度前缀帧：`u32 LE 长度 | 负载`。
//!
//! 负载是什么：
//!
//! * 信令消息（包括版本握手，见 [`crate::signal`]）、端到端握手、会话的控制和大块消息：CBOR（[`encode`] / [`decode`]）；
//! * 画面通道：媒体包（见 [`crate::media`]），原样读出来交给界面。
//!
//! 会话通道上的负载在端到端握手之后都是密文（见 `nife_remote_link::secure`），解密之后才是上面的格式。
//!
//! 这里只有编解码，不碰 IO，也不依赖任何异步运行时：从流里读写帧在 `nife_remote_link::wire`（它用的就是这里的
//! [`payload_len`] / [`decode`]）。手里已经有一段字节的（网页里的 wasm、fuzz）用 [`split`] / [`take_msg`]：
//! 和 `nife_remote_link::wire::read_msg` 走的是同一段解码代码。
//!
//! # 兼容规则
//!
//! 消息是 minicbor 派生的 CBOR：每个枚举变体、每个字段都有一个**显式的编号**（`#[n(…)]`），线上只有编号，
//! 没有名字，也和声明的先后无关。编号一旦发布就不能改、不能复用。
//!
//! * **不认识的消息跳过，不断开**：一条消息**本身**（顶层的枚举变体）不认识，[`decode`] 返回 [`Decoded::Unknown`]，
//!   读的一方跳过它（帧有长度，跳过不会错位）。认识这条消息、但是里面有解不出来的东西（包括没标明可以跳过的地方
//!   出现了不认识的变体）是协议错误，照样断开：那是 bug，不能悄悄吞掉——例如被控端的 `Hello` 里多了一个新的枚举值，
//!   旧的控制端要是把整条 `Hello` 跳过，会话就卡在那里，也没有任何报错；
//! * **加字段**：只能加 `Option` 字段（或者带 `#[cbor(default)]` 的）。旧版本不认识新字段，跳过；
//!   新版本读旧版本的消息，缺的字段是 `None`。必填的字段要永远存在；
//! * **枚举加值**：只有编号的枚举都是开放枚举（`open_enum!`，见 [`crate::cbor`]），不认识的值是 `Unknown(原值)`，
//!   放在哪里都不会出错，由收的一方决定怎么对待；
//! * **带字段的枚举加变体**：作为一整条消息时，旧版本跳过整条消息；作为字段时写成 `Option<枚举>` 并且用
//!   [`crate::cbor::opt`]（不认识的是 `None`）；在列表里用 [`crate::cbor::open_vec`]，只跳过不认识的元素。
//!   只有这三个地方能跳过，别的地方不能放带字段的枚举（为什么见 [`crate::cbor`]）；
//! * **对方可能不认识的新消息**：只在对方的能力位（[`crate::Features`]）表明它认识时才发。
//!
//! 只有改不下去的时候（例如要删掉必填字段）才加版本号（[`crate::SIGNAL_VERSION`] / [`crate::SESSION_VERSION`]）。

use minicbor::{Decode, Encode};

use crate::cbor;

/// 长度前缀的字节数（`u32 LE`）
pub const PREFIX: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("encoding error: {0}")]
    Decode(#[from] minicbor::decode::Error),
    #[error("message too large: {0} bytes")]
    TooLarge(usize),
}

/// 把一条 CBOR 消息编码成带长度前缀的帧。
pub fn encode<T: Encode<()>>(msg: &T) -> Vec<u8> {
    encode_with_spare(msg, 0)
}

/// 同 [`encode`]，多留 `spare` 字节的容量：加密时在后面追加标签和计数（[`crate::handshake::SEAL_OVERHEAD`]）不用重新分配
pub fn encode_with_spare<T: Encode<()>>(msg: &T, spare: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + spare);
    buf.extend_from_slice(&[0u8; 4]);
    minicbor::encode(msg, &mut buf).expect("writing to a Vec can't fail");
    let len = buf.len() - 4;
    buf[..4].copy_from_slice(&(len as u32).to_le_bytes());
    buf
}

/// 一条消息的负载（不带长度前缀，CBOR）
pub fn to_vec<T: Encode<()>>(msg: &T) -> Vec<u8> {
    minicbor::to_vec(msg).expect("writing to a Vec can't fail")
}

/// 解码的结果
#[derive(Debug)]
pub enum Decoded<T> {
    Msg(T),
    /// 对方版本新增的消息（顶层的变体不认识）：跳过。带着变体的编号（日志用）
    Unknown(i64),
}

/// 解码一条消息的负载（不带长度前缀）。消息本身不认识的返回 [`Decoded::Unknown`]，
/// 其它解不出来的（包括里面没标明可以跳过的地方有不认识的变体）是错误（见模块说明）
pub fn decode<'b, T: Decode<'b, ()>>(payload: &'b [u8]) -> Result<Decoded<T>, FrameError> {
    match minicbor::decode(payload) {
        Ok(m) => Ok(Decoded::Msg(m)),
        Err(e) if cbor::unknown_here(&e, payload) => Ok(Decoded::Unknown(
            cbor::variant_of(payload).unwrap_or_default(),
        )),
        Err(e) => Err(cbor::strict(e).into()),
    }
}

/// 解码一条一定认识的消息（握手这类没有"跳过"可言的）
pub fn decode_known<'b, T: Decode<'b, ()>>(payload: &'b [u8]) -> Result<T, FrameError> {
    minicbor::decode(payload).map_err(|e| cbor::strict(e).into())
}

/// 一帧：长度前缀 + `payload`
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PREFIX + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// 长度前缀说负载有多长；超过 `max` 是错误（对方说多长就分配多长的话，一个前缀就能让这边分配 4 GiB）
pub fn payload_len(prefix: [u8; PREFIX], max: usize) -> Result<usize, FrameError> {
    let len = u32::from_le_bytes(prefix) as usize;
    if len > max {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

/// [`split`] 切下来的：`(负载, 剩下的字节)`；还不够一帧时为 `None`
pub type Split<'b> = Option<(&'b [u8], &'b [u8])>;

/// 从 `buf` 开头切下一帧：`(负载, 剩下的字节)`。还不够一帧（要等更多数据）时返回 `Ok(None)`
pub fn split(buf: &[u8], max: usize) -> Result<Split<'_>, FrameError> {
    let Some((prefix, rest)) = buf.split_first_chunk::<PREFIX>() else {
        return Ok(None);
    };
    let len = payload_len(*prefix, max)?;
    Ok(rest.split_at_checked(len))
}

/// 从 `buf` 开头取下一条消息，不认识的跳过（见模块说明），`buf` 前移到这条消息后面。
/// 剩下的不够一帧时返回 `Ok(None)`（`buf` 停在那一帧开头）
pub fn take_msg<'b, T: Decode<'b, ()>>(
    buf: &mut &'b [u8],
    max: usize,
) -> Result<Option<T>, FrameError> {
    loop {
        let Some((payload, rest)) = split(buf, max)? else {
            return Ok(None);
        };
        *buf = rest;
        match decode(payload)? {
            Decoded::Msg(m) => return Ok(Some(m)),
            Decoded::Unknown(variant) => {
                tracing::debug!(
                    variant,
                    len = payload.len(),
                    "skipping a message from a newer version"
                );
            }
        }
    }
}


