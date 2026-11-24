//! 同账号两台设备之间的文件传输：走**账号服务的 WebSocket** 转发，不经过远程会话。
//!
//! 为什么走账号服务（而不是复用会话的那条 QUIC 链路）：
//!
//! * 两端本来就各有一条到账号服务的常连接（见 [`crate::presence`]），不用再建会话、不用配对码；
//! * 服务只转发 `data`（隧道号 + 原样 JSON），协议怎么改都不用动服务端；
//! * 一台设备只要"在线"就能收文件，不需要有人正在看它的画面。
//!
//! 代价：只有**同一个账号**里的设备之间能传（控制码属于别的账号就不行）——界面上会说明。
//!
//! 两边的角色：发起方（[`Role::Client`]）发请求，应答方（[`Role::Server`]）用 [`crate::host::files`]
//! 读写自己的磁盘。请求 / 回答都是一条 JSON：
//!
//! ```json
//! {"req": 3, "op": "list", "path": "D:\\"}      → {"req": 3, "ok": {...}}
//! {"req": 4, "op": "read", "id": 1, "offset": 0, "len": 524288}
//!                                              → {"req": 4, "ok": {"data": "<base64>", "eof": false}}
//! {"req": 5, "op": "write", "id": 1, "offset": 0, "data": "<base64>"} → {"req": 5, "ok": null}
//! ```
//!
//! 回答只有 [`Role::Client`] 这一侧才要，直接交给界面（`fs-peer` 事件），由那边对号。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::host::files;

/// 界面收到的一条对端回答（或隧道断了）
const EVENT: &str = "fs-peer";

/// 等服务端回答 `fs-open` 最多这么久（旧版服务端不会回答）
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// 一条隧道的角色
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// 发起方：发请求、收回答
    Client,
    /// 应答方：按请求读写本机磁盘
    Server,
}

struct Tunnel {
    role: Role,
    /// 应答方打开着的文件
    jobs: Arc<Mutex<files::Jobs>>,
}

struct Registry {
    /// 往账号服务发的通道（连上时才有）
    out: Option<mpsc::Sender<Value>>,
    next_use: u32,
    /// 等 `fs-opened` / `fs-error` 的 `use` 号
    opens: HashMap<u32, oneshot::Sender<Result<u32, String>>>,
    /// 隧道号 → 隧道（两端各有一份，号是服务给的）
    tunnels: HashMap<u32, Tunnel>,
}

fn reg() -> &'static Mutex<Registry> {
    static REG: OnceLock<Mutex<Registry>> = OnceLock::new();
    REG.get_or_init(|| {
        Mutex::new(Registry {
            out: None,
            next_use: 1,
            opens: HashMap::new(),
            tunnels: HashMap::new(),
        })
    })
}

/// 连上账号服务了：换上新的发送通道（旧隧道随之作废，服务那边会通知对面）
pub fn connect(out: mpsc::Sender<Value>) {
    let mut r = reg().lock().unwrap();
    r.out = Some(out);
    r.opens.clear();
    r.tunnels.clear();
}

/// 和账号服务的连接断了：命令都发不出去了
pub fn disconnect() {
    let mut r = reg().lock().unwrap();
    r.out = None;
    for (_, tx) in r.opens.drain() {
        let _ = tx.send(Err("账号服务的连接断开了".into()));
    }
    r.tunnels.clear();
}

/// 服务推来的一条消息。返回 `true` = 这条是文件传输的（别的地方不用管）
pub fn handle(app: &AppHandle, value: &Value) -> bool {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return false;
    };
    match kind {
        "fs-opened" => {
            let use_id = num(value, "use") as u32;
            let tid = num(value, "id") as u32;
            if let Some(tx) = reg().lock().unwrap().opens.remove(&use_id) {
                let _ = tx.send(Ok(tid));
            }
        }
        "fs-error" => {
            let use_id = num(value, "use") as u32;
            let code = value.get("code").and_then(Value::as_str).unwrap_or("failed");
            if let Some(tx) = reg().lock().unwrap().opens.remove(&use_id) {
                let _ = tx.send(Err(reason_text(code)));
            }
        }
        "fs-call" => {
            // 对面要读写这台电脑：登记一条应答隧道，之后按请求干活
            let tid = num(value, "id") as u32;
            reg().lock().unwrap().tunnels.insert(
                tid,
                Tunnel {
                    role: Role::Server,
                    jobs: Arc::new(Mutex::new(files::Jobs::default())),
                },
            );
            let from = value.get("from").and_then(Value::as_str).unwrap_or("");
            debug!(tid, from, "serving a file transfer to another device");
        }
        "fs-data" => {
            let tid = num(value, "id") as u32;
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            let (role, jobs) = {
                let r = reg().lock().unwrap();
                match r.tunnels.get(&tid) {
                    Some(t) => (t.role, t.jobs.clone()),
                    None => return true, // 隧道已经没了：丢掉
                }
            };
            match role {
                // 应答方：读 / 写磁盘（可能慢），放到阻塞线程池，别挡住连接本身
                Role::Server => {
                    let out = reg().lock().unwrap().out.clone();
                    let data = data.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let reply = answer(&jobs, &data);
                        if let Some(out) = out {
                            let _ = out.try_send(json!({"type": "fs-send", "id": tid, "data": reply}));
                        }
                    });
                }
                // 发起方：交给界面对号
                Role::Client => {
                    let _ = app.emit(EVENT, json!({"tid": tid, "data": data}));
                }
            }
        }
        "fs-gone" => {
            let tid = num(value, "id") as u32;
            let reason = value.get("reason").and_then(Value::as_str).unwrap_or("closed");
            reg().lock().unwrap().tunnels.remove(&tid);
            let _ = app.emit(EVENT, json!({"tid": tid, "gone": reason}));
        }
        _ => return false,
    }
    true
}

fn num(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// 服务给的错误码 → 给用户看的句子（前端会再翻一次，这里只给个兜底）
fn reason_text(code: &str) -> String {
    match code {
        "peerOffline" => "对方不在线，或者不在这个账号里".into(),
        other => other.to_owned(),
    }
}

/// 开一条隧道：`code` 是对方控制码，返回隧道号
pub async fn open(code: &str) -> Result<u32, String> {
    let (use_id, rx, out) = {
        let mut r = reg().lock().unwrap();
        let Some(out) = r.out.clone() else {
            return Err("还没连上账号服务".into());
        };
        r.next_use += 1;
        let use_id = r.next_use;
        let (tx, rx) = oneshot::channel();
        r.opens.insert(use_id, tx);
        (use_id, rx, out)
    };
    if out
        .send(json!({"type": "fs-open", "to": code, "use": use_id}))
        .await
        .is_err()
    {
        return Err("账号服务的连接断开了".into());
    }
    // 服务端还是旧版本时不认识 `fs-open`，一个字都不会回：不能一直等下去
    let reply = tokio::time::timeout(OPEN_TIMEOUT, rx).await;
    match reply {
        Err(_) => Err("账号服务没有回应：请确认它已经更新到支持文件传输的版本".into()),
        Ok(Ok(Ok(tid))) => {
            let mut r = reg().lock().unwrap();
            r.tunnels.insert(
                tid,
                Tunnel {
                    role: Role::Client,
                    jobs: Arc::new(Mutex::new(files::Jobs::default())),
                },
            );
            Ok(tid)
        }
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) => Err("账号服务的连接断开了".into()),
    }
}

/// 发一条请求（`data` 原样交给对面）
pub fn send(tid: u32, data: Value) -> Result<(), String> {
    let r = reg().lock().unwrap();
    let out = r.out.clone().ok_or("还没连上账号服务")?;
    out.try_send(json!({"type": "fs-send", "id": tid, "data": data}))
        .map_err(|_| "账号服务的发送队列满了".to_owned())
}

/// 关掉隧道（对面会收到 `fs-gone`）
pub fn close(tid: u32) {
    let (out, tunnel) = {
        let mut r = reg().lock().unwrap();
        (r.out.clone(), r.tunnels.remove(&tid))
    };
    if let Some(t) = tunnel {
        // 应答方：关掉所有打开着的文件（写完的要落盘）
        if t.role == Role::Server {
            let mut jobs = t.jobs.lock().unwrap();
            for id in jobs.ids() {
                jobs.close(id);
            }
        }
    }
    if let Some(out) = out {
        let _ = out.try_send(json!({"type": "fs-close", "id": tid}));
    }
}

/// 应答方：按一条请求干活，返回回答
fn answer(jobs: &Arc<Mutex<files::Jobs>>, data: &Value) -> Value {
    let req = data.get("req").cloned().unwrap_or(Value::from(0));
    let op = data.get("op").and_then(Value::as_str).unwrap_or("");
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let offset = num(data, "offset");
    let id = num(data, "id") as u32;

    // 每一项都是 `Result<Value, String>`：失败也回一条，界面才知道哪一步不行
    let result: Result<Value, String> = match op {
        "roots" => {
            let (places, home) = files::roots();
            Ok(json!({"places": places, "home": home}))
        }
        "list" => match files::list(&text("path")) {
            Ok((path, parent, entries)) => {
                Ok(json!({"path": path, "parent": parent, "entries": entries, "error": null}))
            }
            Err(e) => Ok(json!({"path": text("path"), "parent": null, "entries": [], "error": e})),
        },
        "exists" => Ok(json!(files::exists(&text("path")))),
        "open" => {
            let write = data.get("write").and_then(Value::as_bool).unwrap_or(false);
            jobs.lock()
                .unwrap()
                .open(id, &text("path"), write)
                .map(Value::from)
        }
        "read" => {
            let len = num(data, "len").min(512 * 1024) as u32;
            jobs.lock().unwrap().read(id, offset, len).map(|(bytes, eof)| {
                json!({
                    "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                    "eof": eof,
                })
            })
        }
        "write" => base64::engine::general_purpose::STANDARD
            .decode(text("data").as_bytes())
            .map_err(|e| e.to_string())
            .and_then(|bytes| jobs.lock().unwrap().write(id, offset, &bytes))
            .map(|()| Value::Null),
        "close" => {
            jobs.lock().unwrap().close(id);
            Ok(Value::Null)
        }
        other => Err(format!("unknown operation {other}")),
    };
    match result {
        Ok(ok) => json!({"req": req, "ok": ok}),
        Err(err) => {
            warn!(op, "file transfer request failed: {err}");
            json!({"req": req, "err": err})
        }
    }
}
