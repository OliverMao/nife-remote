//! 被控端的端到端身份和授权（协议见 `nife_remote_proto::handshake`）：本机的身份密钥、配对码、配对过的控制端。
//!
//! * 都保存在 config.toml 里（`[host]`、`[[paired]]`，见 [`crate::config`]），身份密钥和配对码加密保存；
//! * 没配对过的控制端：默认要配对码（config.toml 的 `access`），配对码显示在托盘菜单里，从不经过服务器；
//! * 连续 [`MAX_FAILURES`] 次未成功的配对尝试就暂停配对（`pairing_locked`，重启之后也一样），
//!   托盘里提示，本机用户换一个配对码才恢复：中间人（包括中转服务器）一共只能猜这么几次；
//! * 更换控制码时配对码也换一个、配对过的控制端全部忘掉（见 [`DesktopGate::reset`]）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nife_remote_link::host::{Access, Admitted, Gate};
use nife_remote_link::secure::{Identity, PairingAttempt, hex};
use nife_remote_proto::handshake::PublicKey;
use tracing::{info, warn};

use crate::config::{self, Paired};
use crate::profile;
use crate::status::StatusHub;

/// 未成功和在途的配对尝试合计上限
pub const MAX_FAILURES: u32 = 5;

pub struct DesktopGate {
    identity: Identity,
    access: Mutex<Access>,
    /// 界面里的"允许他人远程协助"：关掉之后不再接受任何控制端（正在控的由调用方断开）
    allowed: std::sync::atomic::AtomicBool,
    state: Mutex<State>,
    hub: StatusHub,
}

struct State {
    pairing: String,
    locked: bool,
    /// 未成功几次（配对成功、换配对码时清零；不保存：重启之后从零数，但暂停是保存的）
    failures: u32,
    pending: u32,
    generation: u64,
    paired: Vec<Paired>,
}

/// 新的配对码：[`nife_remote_proto::PAIRING_DIGITS`] 位随机数字
/// 界面里的验证方式 → 协议里的接入方式
pub fn access_from_verify(v: profile::Verify) -> Access {
    match v {
        profile::Verify::Pairing => Access::Pairing,
        profile::Verify::Approval => Access::Approval,
        profile::Verify::Open => Access::Open,
    }
}

fn new_code() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl DesktopGate {
    /// 读出 config.toml 里的身份、配对码、配对过的控制端；没有的生成并保存（写不进去的只在这次运行里有效）
    pub fn load(hub: StatusHub) -> Arc<Self> {
        let saved = config::host_security();
        let identity = match saved.key {
            Some(k) => Identity::from_private(k),
            None => {
                let id = Identity::generate();
                if let Err(e) = config::save_host_key(id.private()) {
                    warn!(
                        "can't save this computer's identity ({e:#}); controllers will see a new one after a restart"
                    );
                }
                info!(key = %hex(id.public()), "created this computer's end-to-end identity");
                id
            }
        };
        let pairing = match saved.pairing {
            Some(p) => p,
            None => {
                let p = new_code();
                if let Err(e) = config::save_pairing(&p, false) {
                    warn!("can't save the pairing code: {e:#}");
                }
                p
            }
        };
        // 界面里选过的验证方式优先（profile.json），没有就用 config.toml 的 access
        let access = access_from_verify(profile::get().verify);
        let allowed = profile::get().allow_assist;
        info!(?access, key = %hex(identity.public()), paired = saved.paired.len(), "host identity loaded");
        let gate = Arc::new(Self {
            identity,
            access: Mutex::new(access),
            allowed: AtomicBool::new(allowed),
            state: Mutex::new(State {
                pairing,
                locked: saved.pairing_locked,
                failures: 0,
                pending: 0,
                generation: 0,
                paired: saved.paired,
            }),
            hub,
        });
        gate.publish();
        gate
    }

    /// 界面里换验证方式（配对码 / 本机同意 / 不用验证）：立刻生效，配过对的不用重配
    pub fn set_access(&self, access: Access) {
        *self.access.lock().unwrap() = access;
        self.publish();
    }

    /// 界面里的"允许他人远程协助"
    pub fn set_allowed(&self, on: bool) {
        self.allowed.store(on, Ordering::Relaxed);
        self.publish();
    }

    pub fn allowed(&self) -> bool {
        self.allowed.load(Ordering::Relaxed)
    }

    /// 托盘里显示的：配对码、暂停了没有、配对过几个
    fn publish(&self) {
        let st = self.state.lock().unwrap();
        let pairing = (*self.access.lock().unwrap() == Access::Pairing).then(|| st.pairing.clone());
        let (locked, paired) = (st.locked, st.paired.len());
        drop(st);
        self.hub.send_if_modified(|s| {
            let changed = s.host.pairing != pairing
                || s.host.pairing_locked != locked
                || s.host.paired != paired;
            s.host.pairing = pairing;
            s.host.pairing_locked = locked;
            s.host.paired = paired;
            changed
        });
    }

    /// 本机用户换一个配对码（托盘菜单）：暂停的配对也随之恢复。配对过的控制端不受影响
    pub fn new_pairing_code(&self) {
        self.set_pairing_code(&new_code());
    }

    /// 换成本机用户自己填的配对码（控制台首页）：格式由调用方校验过。
    /// 和 [`Self::new_pairing_code`] 一样，暂停的配对随之恢复，配对过的控制端不受影响
    pub fn set_pairing_code(&self, code: &str) {
        let mut st = self.state.lock().unwrap();
        st.pairing = code.to_string();
        st.locked = false;
        st.failures = 0;
        st.pending = 0;
        st.generation += 1;
        if let Err(e) = config::save_pairing(&st.pairing, false) {
            warn!("can't save the pairing code: {e:#}");
        }
        drop(st);
        info!("pairing code changed");
        self.publish();
    }

    /// 忘掉所有配对过的控制端（托盘菜单）：它们下次要重新输入配对码
    pub fn forget_paired(&self) {
        let mut st = self.state.lock().unwrap();
        st.paired.clear();
        // 已经拿到旧授权、尚未提交的配对不能在清空之后重新加入。
        st.generation += 1;
        st.pending = 0;
        if let Err(e) = config::save_paired(&st.paired) {
            warn!("can't save the paired controllers: {e:#}");
        }
        drop(st);
        info!("forgot all paired controllers");
        self.publish();
    }

    /// 更换控制码：配对码也换一个，配对过的控制端全部忘掉
    pub fn reset(&self) {
        let mut st = self.state.lock().unwrap();
        st.paired.clear();
        st.pairing = new_code();
        st.locked = false;
        st.failures = 0;
        st.pending = 0;
        st.generation += 1;
        if let Err(e) = config::save_paired(&st.paired) {
            warn!("can't save the paired controllers: {e:#}");
        }
        if let Err(e) = config::save_pairing(&st.pairing, false) {
            warn!("can't save the pairing code: {e:#}");
        }
        drop(st);
        self.publish();
    }

    /// 会话已经通过认证并开始：只显示加密握手中确认过的名称。服务器的 Incoming 不发布到列表。
    pub fn admitted(&self, session_id: u64, ip: std::net::IpAddr, a: &Admitted) {
        self.hub.send_modify(|s| {
            s.host.controllers.push(crate::status::Controller {
                session_id,
                peer: nife_remote_proto::signal::Peer {
                    name: a.name.clone(),
                    ip,
                },
                safety: Some(a.safety.clone()),
            });
        });
    }
}

impl Gate for DesktopGate {
    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    fn access(&self) -> Access {
        *self.access.lock().unwrap()
    }

    fn begin_pairing(&self) -> Option<Box<dyn PairingAttempt + '_>> {
        if !self.allowed.load(Ordering::Relaxed) {
            return None;
        }
        let mut st = self.state.lock().unwrap();
        if st.locked || st.failures + st.pending >= MAX_FAILURES {
            return None;
        }
        st.pending += 1;
        Some(Box::new(Attempt {
            gate: self,
            code: st.pairing.clone(),
            generation: st.generation,
            completed: false,
        }))
    }

    fn is_known(&self, key: &PublicKey) -> bool {
        if !self.allowed.load(Ordering::Relaxed) {
            return false;
        }
        let k = hex(key);
        self.state.lock().unwrap().paired.iter().any(|p| p.key == k)
    }
}

struct Attempt<'a> {
    gate: &'a DesktopGate,
    code: String,
    generation: u64,
    completed: bool,
}

impl PairingAttempt for Attempt<'_> {
    fn code(&self) -> &str {
        &self.code
    }

    fn complete(mut self: Box<Self>, key: PublicKey, name: &str) -> bool {
        let mut st = self.gate.state.lock().unwrap();
        if st.generation != self.generation || st.locked {
            return false;
        }
        st.pending -= 1;
        self.completed = true;
        let k = hex(&key);
        st.paired.retain(|p| p.key != k);
        st.paired.push(Paired {
            key: k,
            name: name.chars().take(64).collect(),
            since: now_ms(),
        });
        let extra = st.paired.len().saturating_sub(config::MAX_PAIRED);
        st.paired.drain(..extra);
        st.failures = 0;
        if let Err(e) = config::save_paired(&st.paired) {
            warn!("can't save the paired controllers: {e:#}");
        }
        drop(st);
        self.gate.publish();
        true
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut st = self.gate.state.lock().unwrap();
        if st.generation != self.generation {
            return;
        }
        st.pending -= 1;
        st.failures += 1;
        warn!(failures = st.failures, "a pairing attempt did not complete");
        if st.failures >= MAX_FAILURES {
            st.locked = true;
            if let Err(e) = config::save_pairing(&st.pairing, true) {
                warn!("can't save the pairing state: {e:#}");
            }
        }
        drop(st);
        self.gate.publish();
    }
}


