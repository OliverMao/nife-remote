//! 端到端加密（协议见 `nife_remote_proto::handshake`）：设备的身份密钥、握手、之后每一帧的加解密。
//!
//! * [`Identity`]：一台设备的长期 X25519 密钥（Noise 的静态密钥），由各端自己保存；
//! * [`initiate`] / [`Initiator::finish`]：控制端开始握手（第一条消息放进发起会话的请求里），之后在控制通道上做完
//!   （最后是控制端的确认）；[`respond`]：被控端回答（发出决定就返回，确认由会话的读任务解开）。
//!   两边都得到 [`Secured`] 和双方协商好的 [`Negotiated`]；
//! * [`Secured`]：握手一次性派生的通道密钥：每条通道每个方向一个 [`Sealer`] / [`Opener`]，声音数据报、
//!   控制数据报各一对。
//!
//! 加解密都是原地做的：发送的帧预留了标签和计数的位置（`nife_remote_proto::framing::encode_with_spare`、
//! `nife_remote_proto::media::frame`），收到的帧解密之后截掉尾巴，画面包照样原样交给界面，不多拷贝。

use std::future::Future;
use std::time::Duration;

use crate::wire::{read_frame, write_frame};
use nife_remote_proto::framing::{self, PREFIX};
use nife_remote_proto::session::Codec;
use nife_remote_proto::handshake::{
    self, Confirm, ControllerIntro, HandshakeError, HandshakeFinish, HandshakeInit, HandshakeReply,
    HostVerdict, MAX_MSG, NONCE_LEN, PAKE_CONTROLLER, PAKE_HOST, Pattern, PublicKey, Refusal,
    SEAL_OVERHEAD, TAG_LEN,
};
use nife_remote_proto::session::QualitySettings;
use nife_remote_proto::{Features, Role, SESSION_MIN, SESSION_VERSION, SessionLane, datagram};
use anyhow::{Context, Result, anyhow, bail};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::hkdf;
use spake2::{Ed25519Group, Identity as PakeId, Password, Spake2};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info};

pub use nife_remote_proto::handshake::Negotiated;

use crate::Problem;

/// 握手的每一步最多等这么久（被控端的用户同意不在握手里，在之后的会话里）
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

// ───────────────────────────── 身份 ─────────────────────────────

/// 一台设备的长期 X25519 密钥对（Noise 的静态密钥）。只保存私钥（32 字节），公钥由它算出
#[derive(Clone)]
pub struct Identity {
    private: [u8; 32],
    public: PublicKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public", &hex(&self.public))
            .finish()
    }
}

impl Identity {
    pub fn generate() -> Self {
        Self::from_private(rand::random())
    }

    pub fn from_private(private: [u8; 32]) -> Self {
        let public = curve25519_dalek::MontgomeryPoint::mul_base_clamped(private).to_bytes();
        Self { private, public }
    }

    /// 保存用的私钥（调用方负责加密保存，和取回控制码的密钥一样）
    pub fn private(&self) -> &[u8; 32] {
        &self.private
    }

    pub fn public(&self) -> &PublicKey {
        &self.public
    }
}

impl Drop for Identity {
    fn drop(&mut self) {
        self.private = [0; 32];
    }
}

/// 公钥写成 64 位小写十六进制（保存、显示、传给界面）
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// 解析 [`hex`] 写出的公钥 / 私钥
pub fn unhex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

// ───────────────────────────── 每一帧 ─────────────────────────────

fn nonce(counter: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_le_bytes());
    Nonce::assume_unique_for_key(n)
}

fn aead_key(raw: &[u8; 32]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, raw).expect("32-byte key"))
}

/// 一条通道一个方向的发送端。每个密封单元是 `密文 | 标签 | 计数`，计数就是 nonce，从 0 开始递增
/// （见 `nife_remote_proto::handshake` 的"之后的每一帧"）
pub struct Sealer {
    key: LessSafeKey,
    counter: u64,
}

impl Sealer {
    /// 把一帧（`u32 长度 | 明文`，`framing::encode_with_spare` / `media::frame` 编好的，留了 [`SEAL_OVERHEAD`]
    /// 的容量）原地加密成 `u32 长度 | 密文 | 标签 | 计数`
    pub fn seal_frame(&mut self, frame: &mut Vec<u8>) -> Result<()> {
        anyhow::ensure!(frame.len() >= PREFIX, "not a frame");
        self.seal_at(frame, PREFIX)?;
        let len = (frame.len() - PREFIX) as u32;
        frame[..PREFIX].copy_from_slice(&len.to_le_bytes());
        Ok(())
    }

    /// 加密一段负载（不带长度前缀），返回密封单元 `密文 | 标签 | 计数`
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(plaintext.len() + SEAL_OVERHEAD);
        out.extend_from_slice(plaintext);
        self.seal_at(&mut out, 0)?;
        Ok(out)
    }

    /// 数据报：`会话 ID | 种类 | 密文 | 标签 | 计数`（见 `nife_remote_proto::datagram`）
    pub fn seal_datagram(&mut self, session_id: u64, kind: u8, packet: &[u8]) -> Result<Vec<u8>> {
        let mut d = Vec::with_capacity(datagram::HEADER + packet.len() + SEAL_OVERHEAD);
        d.extend_from_slice(&session_id.to_le_bytes());
        d.push(kind);
        d.extend_from_slice(packet);
        self.seal_at(&mut d, datagram::HEADER)?;
        Ok(d)
    }

    /// 把 `buf[start..]` 原地加密，后面追加标签和计数
    fn seal_at(&mut self, buf: &mut Vec<u8>, start: usize) -> Result<()> {
        let c = self.counter;
        self.counter = c.checked_add(1).context("nonce counter exhausted")?;
        let tag = self
            .key
            .seal_in_place_separate_tag(nonce(c), Aad::empty(), &mut buf[start..])
            .map_err(|_| anyhow!("encryption failed"))?;
        buf.extend_from_slice(tag.as_ref());
        buf.extend_from_slice(&c.to_le_bytes());
        Ok(())
    }
}

/// 密封单元（`密文 | 标签 | 计数`）末尾的计数，不用解密就看得到：几条路上来的帧按它排好再解密
/// （见 `crate::merge`）。短得不像一个密封单元时为 `None`
pub fn counter_of(sealed: &[u8]) -> Option<u64> {
    let at = sealed.len().checked_sub(NONCE_LEN)?;
    (sealed.len() >= SEAL_OVERHEAD).then(|| u64::from_le_bytes(sealed[at..].try_into().unwrap()))
}

/// 一条通道一个方向的接收端
pub struct Opener {
    key: LessSafeKey,
    order: Order,
}

/// 接收端认哪些计数（见 `nife_remote_proto::handshake` 的"之后的每一帧"）
enum Order {
    /// 控制、大块通道：必须是下一个，丢掉、重排、重放的消息都不认
    Next(u64),
    /// 画面、声音、探测：没收到过就行（滑动窗口挡掉重放）
    Window(ReplayWindow),
}

impl Opener {
    /// 下一个要的计数（控制、大块通道）；画面、声音是收到过的最大计数 + 1
    pub(crate) fn expected(&self) -> u64 {
        match &self.order {
            Order::Next(n) => *n,
            Order::Window(w) => w.top,
        }
    }

    /// 这个计数还没收到过（画面、声音的滑动窗口；控制、大块通道必须正好是下一个）
    pub(crate) fn is_fresh(&self, c: u64) -> bool {
        match &self.order {
            Order::Next(n) => c == *n,
            Order::Window(w) => w.fresh(c),
        }
    }

    /// 原地解开一个密封单元（`密文 | 标签 | 计数`，流上一帧去掉长度前缀的部分），之后 `buf` 就是明文。
    /// 解不开（被改过、不是这条通道的、计数不对）是错误：控制、大块、画面通道上会话不能再继续；
    /// 数据报丢掉就是了
    pub fn open(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        let at = buf
            .len()
            .checked_sub(NONCE_LEN + TAG_LEN)
            .context("a session frame is too short")?
            + TAG_LEN;
        let c = u64::from_le_bytes(buf[at..].try_into().expect("8 bytes"));
        anyhow::ensure!(
            self.is_fresh(c),
            "a session frame was dropped, reordered or replayed"
        );
        buf.truncate(at);
        let len = self
            .key
            .open_in_place(nonce(c), Aad::empty(), buf)
            .map_err(|_| anyhow!("a session frame failed authentication"))?
            .len();
        buf.truncate(len);
        // 认证通过之后才记下：伪造的单元不能把计数推走
        match &mut self.order {
            Order::Next(next) => *next = c.saturating_add(1),
            Order::Window(w) => w.accept(c),
        }
        Ok(())
    }
}

/// 最近 128 个计数里收过哪些（RFC 4303 的做法）
#[derive(Default)]
struct ReplayWindow {
    /// 收到过的最大计数 + 1（0 表示还没收到过）
    top: u64,
    /// 第 i 位：top - 1 - i 收到过
    seen: u128,
}

impl ReplayWindow {
    fn fresh(&self, c: u64) -> bool {
        if c >= self.top {
            return true;
        }
        let back = self.top - 1 - c;
        back < 128 && self.seen & (1 << back) == 0
    }

    fn accept(&mut self, c: u64) {
        if c >= self.top {
            let shift = c - self.top + 1;
            self.seen = if shift >= 128 { 0 } else { self.seen << shift };
            self.seen |= 1;
            self.top = c.saturating_add(1);
        } else {
            self.seen |= 1 << (self.top - 1 - c);
        }
    }
}

// ───────────────────────────── 会话的密钥 ─────────────────────────────

/// 一条通道（或者一种数据报）一个方向一对加密端。没有 Clone，也没有重新从计数 0 创建加密端的公开接口：
/// 每对密钥只能移交一次
pub struct LaneKeys {
    pub sealer: Sealer,
    pub opener: Opener,
}

/// 握手之后移交给各条通道的密钥；控制通道的在 [`Initiated`] / [`Responded`] 里（握手已经用掉了它的第一个计数）。
pub struct Secured {
    /// 对方的公钥
    pub peer: PublicKey,
    /// 安全码（两端一样，见 `nife_remote_proto::handshake::safety_code`）
    pub safety: String,
    pub video: LaneKeys,
    pub bulk: LaneKeys,
    /// 声音数据报。乱序可以、重放不行
    pub audio: LaneKeys,
    /// 控制数据报（Ping、Pong、画面确认、探测和回声，`nife_remote_proto::datagram::CONTROL`）。乱序可以、重放不行：
    /// 直连通着时同一个数据报两条路各发一份，后到的那份就是重放
    pub datagrams: LaneKeys,
}

/// 仅握手内部持有的原始材料，消费后派生一次全部通道密钥。
struct KeyMaterial {
    /// 本端是哪一边（控制端是 Noise 的发起方）
    role: Role,
    /// 发起方 → 响应方、响应方 → 发起方
    i2r: [u8; 32],
    r2i: [u8; 32],
    /// 握手的哈希（派生用的 salt）
    h: [u8; 32],
    peer: PublicKey,
    safety: String,
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        self.i2r = [0; 32];
        self.r2i = [0; 32];
    }
}

/// 每条通道、每种数据报的密钥用不同的标签派生：从别的通道搬过来的帧解不开
fn label(lane: SessionLane) -> &'static [u8] {
    match lane {
        SessionLane::Control => b"nife-remote control",
        SessionLane::Video => b"nife-remote video",
        SessionLane::Bulk => b"nife-remote bulk",
    }
}

impl KeyMaterial {
    fn new(role: Role, hs: &mut snow::HandshakeState) -> Result<Self> {
        let peer: PublicKey = hs
            .get_remote_static()
            .context("the peer has no static key")?
            .try_into()
            .context("bad static key")?;
        let h: [u8; 32] = hs
            .get_handshake_hash()
            .try_into()
            .context("unexpected handshake hash length")?;
        let (i2r, r2i) = hs.dangerously_get_raw_split();
        let mut seed = [0u8; 32];
        derive(&h, &h, b"nife-remote safety code", &mut seed);
        Ok(Self {
            role,
            i2r,
            r2i,
            h,
            peer,
            safety: handshake::safety_code(&seed),
        })
    }

    /// 全部通道的密钥，和控制通道的
    fn split(self) -> (Secured, LaneKeys) {
        let window = || Order::Window(ReplayWindow::default());
        (
            Secured {
                peer: self.peer,
                safety: self.safety.clone(),
                video: self.keys(label(SessionLane::Video), window()),
                bulk: self.keys(label(SessionLane::Bulk), Order::Next(0)),
                audio: self.keys(b"nife-remote audio", window()),
                datagrams: self.keys(b"nife-remote control datagram", window()),
            },
            self.keys(label(SessionLane::Control), Order::Next(0)),
        )
    }

    fn key(&self, sending: bool, label: &[u8]) -> LessSafeKey {
        // 控制端发送 = 发起方 → 响应方
        let i2r = sending == (self.role == Role::Controller);
        let ikm = if i2r { &self.i2r } else { &self.r2i };
        let mut out = [0u8; 32];
        derive(&self.h, ikm, label, &mut out);
        aead_key(&out)
    }

    fn keys(&self, label: &[u8], order: Order) -> LaneKeys {
        LaneKeys {
            sealer: Sealer {
                key: self.key(true, label),
                counter: 0,
            },
            opener: Opener {
                key: self.key(false, label),
                order,
            },
        }
    }
}

/// 测试用：一条通道一个方向的一对加密端（发送端、接收端），不用握手。`ordered`：控制、大块通道那样必须一个接一个


/// HKDF-SHA256：`salt` 提取、`info` 扩展出 32 字节
fn derive(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8; 32]) {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    prk.expand(&[info], hkdf::HKDF_SHA256)
        .and_then(|okm| okm.fill(out))
        .expect("32 bytes is a valid HKDF-SHA256 output length");
}

// ───────────────────────────── 握手 ─────────────────────────────

/// 认识的模式的 Noise 参数（不认识的模式在这之前就回答了 [`HandshakeError::Unsupported`]）
fn noise(pattern: Pattern) -> Result<snow::params::NoiseParams> {
    let name = pattern
        .noise()
        .with_context(|| format!("unsupported handshake pattern {pattern:?}"))?;
    Ok(name.parse().expect("valid noise pattern"))
}

async fn read_step<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> Result<()> {
    let got = tokio::time::timeout(STEP_TIMEOUT, read_frame(r, MAX_MSG, buf))
        .await
        .context("the end-to-end handshake timed out")??;
    anyhow::ensure!(got, "the session closed during the end-to-end handshake");
    Ok(())
}

async fn send_step<W: AsyncWrite + Unpin, T: nife_remote_proto::minicbor::Encode<()>>(
    w: &mut W,
    msg: &T,
) -> Result<()> {
    tokio::time::timeout(STEP_TIMEOUT, write_frame(w, &framing::to_vec(msg)))
        .await
        .context("the end-to-end handshake writer timed out")??;
    Ok(())
}

/// Noise 消息的缓冲区（Noise 规定一条消息最多 65535 字节）
fn noise_buf() -> Vec<u8> {
    vec![0u8; 65535]
}

/// 控制端握手的结果
pub struct Initiated {
    pub secured: Secured,
    /// 控制通道上被控端 → 控制端的接收端（已经解开了被控端的决定，计数接着往下数）
    pub opener: Opener,
    /// 控制通道上控制端 → 被控端的发送端（已经发出了确认，计数接着往下数）
    pub sealer: Sealer,
    /// 选定的会话版本、双方都支持的能力位（和被控端算出来的一样）
    pub negotiated: Negotiated,
}

/// 控制端怎么证明自己、怎么认被控端
pub struct InitiatorParams<'a> {
    /// 控制码（配对时算进 SPAKE2 的密码）
    pub code: u32,
    pub me: &'a Identity,
    /// 之前记下的被控端公钥（"我的设备"）：只认它
    pub host_key: Option<PublicKey>,
    /// 配对码（被控端要求配对、或者对方的身份变了时用户输入的）。带着它就用配对码认对方，
    /// 对方的公钥可以和 `host_key` 不一样（重新配对之后记下新的，见 `nife_remote_proto::handshake` 的"身份"）
    pub pairing: Option<&'a str>,
    pub intro: &'a ControllerIntro,
}

/// 控制端发出了第一条消息、在等被控端回答的握手（见 [`initiate`]）
pub struct Initiator {
    hs: snow::HandshakeState,
    pattern: Pattern,
    pake: Option<Spake2<Ed25519Group>>,
    /// 介绍（CBOR），`Noise_XX` 时放在第 3 条里
    intro: Vec<u8>,
    features: Features,
}

/// 控制端：开始握手，返回第一条消息（[`HandshakeInit`] 的 CBOR，放进 `ClientMsg::Connect::init`）。
/// 接着在控制通道上用 [`Initiator::finish`] 做完
pub fn initiate(p: InitiatorParams<'_>) -> Result<(Initiator, Vec<u8>)> {
    let pattern = match (p.pairing, p.host_key) {
        (Some(_), _) => Pattern::Pairing,
        (None, Some(_)) => Pattern::Known,
        (None, None) => Pattern::First,
    };
    let pake = p.pairing.map(|code| {
        Spake2::<Ed25519Group>::start_a(
            &Password::new(handshake::pake_password(p.code, code)),
            &PakeId::new(PAKE_CONTROLLER),
            &PakeId::new(PAKE_HOST),
        )
    });
    let mut init = HandshakeInit {
        pattern,
        pake: pake.as_ref().map(|(_, msg)| msg.clone()),
        noise: Vec::new(),
    };
    let prologue = handshake::prologue(&init);
    let mut builder = snow::Builder::new(noise(pattern)?)
        .local_private_key(p.me.private())?
        .prologue(&prologue)?;
    if let (Pattern::Known, Some(k)) = (pattern, &p.host_key) {
        builder = builder.remote_public_key(k)?;
    }
    let mut hs = builder.build_initiator()?;
    let intro = framing::to_vec(p.intro);
    let mut msg = noise_buf();
    // IK：介绍放在第 1 条里（加密、认证过）；XX：第 1 条什么都不能带
    let first: &[u8] = if pattern == Pattern::Known {
        &intro
    } else {
        &[]
    };
    let n = hs.write_message(first, &mut msg)?;
    init.noise = msg[..n].to_vec();
    let init = framing::to_vec(&init);
    anyhow::ensure!(init.len() <= MAX_MSG, "the handshake message is too large");
    Ok((
        Initiator {
            hs,
            pattern,
            pake: pake.map(|(s, _)| s),
            intro,
            features: p.intro.features,
        },
        init,
    ))
}

impl Initiator {
    /// 读被控端的回答、做完握手（见 `nife_remote_proto::handshake`）。`recv` / `send`：控制通道收、发的两半。
    /// 失败时的错误是 [`Problem`]（对方身份变了、要配对码、配对码不对、版本不兼容……）或者普通的网络错误
    pub async fn finish<R, W>(self, recv: &mut R, send: &mut W) -> Result<Initiated>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let Self {
            mut hs,
            pattern,
            pake,
            intro,
            features,
        } = self;
        let mut buf = Vec::new();
        read_step(recv, &mut buf).await?;
        let reply: HandshakeReply = framing::decode_known(&buf)?;
        if let Some(e) = reply.error {
            return Err(handshake_problem(e, pattern, reply.patterns.as_deref()).into());
        }
        if let Some(spake) = pake {
            let theirs = reply
                .pake
                .as_deref()
                .context("the host didn't answer the pairing")?;
            let psk = spake
                .finish(theirs)
                .map_err(|_| anyhow!("bad pairing message from the host"))?;
            hs.set_psk(3, &psk)?;
        }
        let msg2 = reply
            .noise
            .as_deref()
            .context("the host sent no handshake message")?;
        let mut payload = noise_buf();
        hs.read_message(msg2, &mut payload)
            .map_err(|_| -> anyhow::Error {
                // IK：被控端用的不是我们记着的密钥（换过身份，或者是别人冒充的）
                if pattern == Pattern::Known {
                    Problem::HostKeyChanged.into()
                } else {
                    anyhow!("the host's handshake message failed authentication")
                }
            })?;

        let verdict = if pattern == Pattern::Known {
            reply.verdict
        } else {
            // First：之前不认识它；Pairing：配对码认的是它，换过身份也没关系
            let mut msg = noise_buf();
            let n = hs.write_message(&intro, &mut msg)?;
            send_step(
                send,
                &HandshakeFinish {
                    noise: msg[..n].to_vec(),
                },
            )
            .await?;
            read_step(recv, &mut buf).await?;
            let reply: HandshakeReply = framing::decode_known(&buf)?;
            if let Some(e) = reply.error {
                return Err(handshake_problem(e, pattern, reply.patterns.as_deref()).into());
            }
            reply.verdict
        };
        anyhow::ensure!(hs.is_handshake_finished(), "the handshake didn't finish");
        let (
            secured,
            LaneKeys {
                mut opener,
                mut sealer,
            },
        ) = KeyMaterial::new(Role::Controller, &mut hs)?.split();
        let mut verdict = verdict.context("the host sent no verdict")?;
        opener.open(&mut verdict)?;
        let verdict: HostVerdict = framing::decode_known(&verdict)?;
        let version = match (verdict.version, verdict.refused) {
            (Some(v), _) => v,
            (None, Some(Refusal::PairingRequired)) => return Err(Problem::PairingRequired.into()),
            (None, Some(Refusal::Incompatible)) => {
                return Err(incompatible(verdict.min, verdict.max).into());
            }
            (None, r) => {
                return Err(
                    Problem::failed(format!("the host refused the connection ({r:?})")).into(),
                );
            }
        };
        anyhow::ensure!(
            (SESSION_MIN..=SESSION_VERSION).contains(&version),
            "the host chose session version {version}, which we can't speak"
        );
        let mut negotiated = Negotiated::new(version, verdict.features);
        negotiated.features = negotiated.features.common(features);
        // 确认（见 nife_remote_proto::handshake 的"确认"）：被控端解开它才算本机进来了，重放的握手消息走不到这一步
        let mut confirm = framing::encode_with_spare(&Confirm::default(), SEAL_OVERHEAD);
        sealer.seal_frame(&mut confirm)?;
        tokio::time::timeout(STEP_TIMEOUT, send.write_all(&confirm))
            .await
            .context("the handshake confirmation writer timed out")??;
        debug!(?pattern, ?negotiated, safety = %secured.safety, "end-to-end handshake done");
        Ok(Initiated {
            negotiated,
            secured,
            opener,
            sealer,
        })
    }
}

/// 会话版本不兼容：`min..=max` 是对方能说的
fn incompatible(min: u16, max: u16) -> Problem {
    if max < SESSION_MIN {
        // 对方太旧
        Problem::PeerIncompatible {
            peer: max,
            ours: SESSION_MIN,
        }
    } else {
        Problem::PeerIncompatible {
            peer: min,
            ours: SESSION_VERSION,
        }
    }
}

/// 被控端回答的握手错误。`theirs`：被控端会的模式（[`HandshakeError::Unsupported`] 时有）
fn handshake_problem(e: HandshakeError, pattern: Pattern, theirs: Option<&[Pattern]>) -> Problem {
    match e {
        HandshakeError::Unsupported => {
            // 模式的编号越大越新。对方会的都比这次用的旧：对方该更新了；
            // 否则（对方会更新的模式、或者不再支持这个旧的）是本机该更新了
            let ours = pattern.code();
            let newest = Pattern::newest(theirs.unwrap_or_default());
            let narrow = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
            if ours > newest {
                Problem::PeerIncompatible {
                    peer: narrow(newest),
                    ours: narrow(ours),
                }
            } else {
                Problem::PeerIncompatible {
                    peer: narrow(newest.max(ours + 1)),
                    ours: narrow(ours),
                }
            }
        }
        HandshakeError::UnknownKey if pattern == Pattern::Known => Problem::HostKeyChanged,
        HandshakeError::PairingFailed => Problem::PairingFailed,
        HandshakeError::PairingUnavailable => Problem::PairingUnavailable,
        e => Problem::failed(format!("end-to-end handshake failed: {e:?}")),
    }
}

/// 被控端怎么对待没配对过的控制端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// 知道控制码就能连（等于信任中转服务器：服务器知道所有控制码）
    Open,
    /// 请本机用户同意（同意之后在这次运行里记住它）
    Approval,
    /// 要配对码（配对之后记住它）
    Pairing,
}

/// 一个已占名额的配对尝试。实现必须把失败和在途尝试一起计入本机预算；
/// 丢弃（包括超时、取消、格式错误）算一次未成功尝试。换配对码后旧尝试不能提交。
pub trait PairingAttempt: Send {
    fn code(&self) -> &str;
    /// 在本机同一把锁下检查代际并记住控制端，成功才返回 true。
    fn complete(self: Box<Self>, key: PublicKey, name: &str) -> bool;
}

/// 被控端：自己是谁，认得哪些控制端，配对尝试的本机预算（各平台自己保存）。
pub trait Gate: Send + Sync + 'static {
    fn identity(&self) -> Identity;
    fn access(&self) -> Access;
    /// 原子地占一个配对名额；None 表示不接受配对或预算已用完。
    fn begin_pairing(&self) -> Option<Box<dyn PairingAttempt + '_>> {
        None
    }
    /// 这个控制端配对过（或者本机用户这次运行里同意过）
    fn is_known(&self, key: &PublicKey) -> bool;
}

/// 被控端为什么让这个控制端进来
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// 刚刚用配对码配对成功
    Paired,
    /// 配对过的 / 本机用户同意过的
    Known,
    /// 被控端允许任何知道控制码的人连
    Open,
    /// 还要请本机用户同意（会话里先发 `AwaitingApproval`）
    NeedsApproval,
}

/// 被控端握手的结果。控制端还没确认（见 `nife_remote_proto::handshake` 的"确认"）：`opener` 解开的第一帧必须是
/// [`Confirm`]，在那之前不能显示"谁在连"、不能请本机用户同意
pub struct Responded {
    pub secured: Secured,
    /// 控制通道上被控端 → 控制端的发送端（已经发出了决定，计数接着往下数）
    pub sealer: Sealer,
    /// 控制通道上控制端 → 被控端的接收端（下一帧是确认，计数 0）
    pub opener: Opener,
    pub intro: ControllerIntro,
    pub admission: Admission,
    /// 选定的会话版本、双方都支持的能力位（和控制端算出来的一样）
    pub negotiated: Negotiated,
}

pub struct ResponderParams<'a> {
    /// 本机的控制码（配对时算进 SPAKE2 的密码）
    pub code: u32,
    pub gate: &'a dyn Gate,
}

/// 被控端：回答控制端的握手（`init` 是 `ServerMsg::Incoming::init`，见 `nife_remote_proto::handshake`）。
/// 回答写进 `send`（本端的控制通道）；控制端之后的消息从 `recv` 来（控制端的控制通道，要等它开过来：
/// 用到时才等）。让不让进来由 [`Gate`] 决定；拒绝时已经把原因告诉了控制端，返回错误。
///
/// 发出决定就返回，不等控制端的确认（见 [`Responded`]）。用到了 `recv` 的话（`Noise_XX` 要读第 3 条）连同它等到的流
/// 一起返回，`Noise_IK` 用不着它，是 `None`
pub async fn respond<W, R, F>(
    init: &[u8],
    send: &mut W,
    recv: F,
    p: ResponderParams<'_>,
) -> Result<(Responded, Option<R>)>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
    F: Future<Output = Result<R>>,
{
    let init: HandshakeInit = match framing::decode_known(init) {
        Ok(i) if init.len() <= MAX_MSG => i,
        Ok(_) => {
            fail(send, HandshakeError::Malformed).await;
            bail!("the controller's first handshake message is too large");
        }
        Err(e) => {
            fail(send, HandshakeError::Malformed).await;
            return Err(e.into());
        }
    };
    let pattern = init.pattern;
    if !pattern.is_known() {
        // 以后的模式：告诉控制端本机会哪些，它据此判断该更新哪一边
        let _ = send_step(
            send,
            &HandshakeReply {
                error: Some(HandshakeError::Unsupported),
                patterns: Some(Pattern::KNOWN.to_vec()),
                ..Default::default()
            },
        )
        .await;
        bail!("unsupported end-to-end handshake pattern {pattern:?}");
    }
    let me = p.gate.identity();
    let mut reply = HandshakeReply::default();
    let mut psk = None;
    let mut pairing = None;
    if pattern == Pattern::Pairing {
        let Some(attempt) = p.gate.begin_pairing() else {
            fail(send, HandshakeError::PairingUnavailable).await;
            bail!("a controller tried to pair, but pairing is off");
        };
        let password = handshake::pake_password(p.code, attempt.code());
        pairing = Some(attempt);
        let (spake, msg_b) = Spake2::<Ed25519Group>::start_b(
            &Password::new(password),
            &PakeId::new(PAKE_CONTROLLER),
            &PakeId::new(PAKE_HOST),
        );
        let theirs = init.pake.as_deref().unwrap_or_default();
        match spake.finish(theirs) {
            Ok(k) => psk = Some(k),
            Err(_) => {
                fail(send, HandshakeError::Malformed).await;
                bail!("bad pairing message");
            }
        }
        reply.pake = Some(msg_b);
    }
    let prologue = handshake::prologue(&init);
    let mut hs = snow::Builder::new(noise(pattern)?)
        .local_private_key(me.private())?
        .prologue(&prologue)?
        .build_responder()?;
    if let Some(psk) = &psk {
        hs.set_psk(3, psk)?;
    }
    let mut payload = noise_buf();
    let n = match hs.read_message(&init.noise, &mut payload) {
        Ok(n) => n,
        Err(_) => {
            // IK：控制端记着的不是我们的公钥（我们换过身份，或者它连的根本不是我们）
            let e = if pattern == Pattern::Known {
                HandshakeError::UnknownKey
            } else {
                HandshakeError::Malformed
            };
            fail(send, e).await;
            bail!("the controller's first handshake message failed authentication ({e:?})");
        }
    };
    let mut msg = noise_buf();
    let m = hs.write_message(&[], &mut msg)?;
    reply.noise = Some(msg[..m].to_vec());

    let (intro_bytes, stream) = if pattern == Pattern::Known {
        (payload[..n].to_vec(), None)
    } else {
        send_step(send, &reply).await?;
        reply = HandshakeReply::default();
        let mut r = recv.await?;
        let mut buf = Vec::new();
        read_step(&mut r, &mut buf).await?;
        let finish: HandshakeFinish = framing::decode_known(&buf)?;
        match hs.read_message(&finish.noise, &mut payload) {
            Ok(n) => (payload[..n].to_vec(), Some(r)),
            Err(_) if pattern == Pattern::Pairing => {
                drop(pairing.take());
                fail(send, HandshakeError::PairingFailed).await;
                bail!("a controller entered a wrong pairing code");
            }
            Err(_) => {
                fail(send, HandshakeError::Malformed).await;
                bail!("the controller's last handshake message failed authentication");
            }
        }
    };
    anyhow::ensure!(hs.is_handshake_finished(), "the handshake didn't finish");
    let intro: ControllerIntro = match framing::decode_known(&intro_bytes) {
        Ok(i) => i,
        Err(e) => {
            fail(send, HandshakeError::Malformed).await;
            return Err(e.into());
        }
    };
    let (secured, LaneKeys { opener, mut sealer }) = KeyMaterial::new(Role::Host, &mut hs)?.split();

    // 决定：版本，然后是让不让进来
    let version = intro.max.min(SESSION_VERSION);
    let compatible = version >= intro.min.max(SESSION_MIN);
    let negotiated = Negotiated::new(version, intro.features);
    let admission = if pattern == Pattern::Pairing {
        Some(Admission::Paired)
    } else if p.gate.is_known(&secured.peer) {
        Some(Admission::Known)
    } else {
        match p.gate.access() {
            Access::Open => Some(Admission::Open),
            Access::Approval => Some(Admission::NeedsApproval),
            Access::Pairing => None,
        }
    };
    let refused = if !compatible {
        Some(Refusal::Incompatible)
    } else if admission.is_none() {
        Some(Refusal::PairingRequired)
    } else {
        None
    };
    // 配对：解开了第 3 条，配对码就是对的、这次握手也是新鲜的（第 3 条离不开本端这次的临时密钥）：现在就记住它。
    // 配对码可能在这期间换掉了：本机在自己的锁里检查，换掉了就不认（见 nife_remote_proto::handshake 的"配对"）
    if let (None, Some(attempt)) = (refused, pairing) {
        if !attempt.complete(secured.peer, &intro.name) {
            fail(send, HandshakeError::PairingUnavailable).await;
            bail!("pairing was revoked during the handshake");
        }
        info!(name = %intro.name, key = %hex(&secured.peer), "a controller paired with this host");
    }
    let verdict = HostVerdict {
        version: refused.is_none().then_some(version),
        features: nife_remote_proto::features::session::SUPPORTED,
        refused,
        min: SESSION_MIN,
        max: SESSION_VERSION,
    };
    reply.verdict = Some(sealer.seal(&framing::to_vec(&verdict))?);
    send_step(send, &reply).await?;
    if let Some(r) = refused {
        bail!("refused the controller: {r:?}");
    }
    Ok((
        Responded {
            negotiated,
            secured,
            sealer,
            opener,
            intro,
            admission: admission.expect("checked above"),
        },
        stream,
    ))
}

/// 告诉控制端握手失败了（明文，见 `HandshakeReply::error`）。发不出去也没关系
async fn fail<W: AsyncWrite + Unpin>(send: &mut W, e: HandshakeError) {
    let _ = send_step(
        send,
        &HandshakeReply {
            error: Some(e),
            ..Default::default()
        },
    )
    .await;
}

/// 本端的介绍（控制端）：名称、系统，开始时的画质、要不要声音，以及本机能解码的格式
/// （`codecs`：被控端据此在"双方都支持"的格式里挑、回退，见 [`ControllerIntro::codecs`]）
pub fn controller_intro(
    name: &str,
    os: &str,
    settings: QualitySettings,
    audio: bool,
    codecs: Vec<Codec>,
) -> ControllerIntro {
    ControllerIntro {
        min: SESSION_MIN,
        max: SESSION_VERSION,
        features: nife_remote_proto::features::session::SUPPORTED,
        name: name.to_string(),
        os: os.to_string(),
        settings: settings.sanitized(),
        audio,
        codecs,
    }
}


