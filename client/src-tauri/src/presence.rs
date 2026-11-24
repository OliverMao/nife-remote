//! 设备的常连接：一条 WebSocket 连到账号服务，报到（控制码、名字、系统），收到同账号的设备列表。
//!
//! 为什么不用 HTTP 心跳（见 `server/auth/app/presence.py`）：设备一上线 / 下线，别的设备马上看到；
//! 控制码属于哪个账号也在这条连接上定下来。界面上"我的设备"显示的名字、系统、在线状态都来自这里的推送。
//!
//! 生命周期跟着登录和本机控制码走：
//!
//! * 没登录 → 什么都不做，过几秒再看一眼；
//! * 登录了 → 连上，拿到控制码就报一次（`hello`），控制码变了（换了码、被控端重启）再报一次；
//! * 连接断了（网络、服务重启）→ 退避重连；登录凭证换了（改密码）→ 断开重连；
//! * 退出时连接关掉，服务那边马上把它标成离线。

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::accounts;
use crate::status::{AppStatus, StatusHub};

/// 界面收到的设备列表（`account-devices` 事件）：服务端推什么就转什么，见 server/auth 的说明
const EVENT: &str = "account-devices";
/// 断线之后等多久再连（指数退避，封顶）
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// 保活：这么久没有别的事就发一个 ping（服务回 pong）
const PING_EVERY: Duration = Duration::from_secs(30);

/// 启动设备连接（托盘启动时调用一次，之后自己重连）
pub fn start(app: AppHandle, hub: StatusHub) {
    tauri::async_runtime::spawn(async move {
        let mut host = hub.subscribe();
        let mut backoff = RETRY_MIN;
        loop {
            let Some(token) = accounts::token() else {
                // 没登录：等它登录（登录之后最迟几秒就连上）
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };
            let base = accounts::base_url();
            if base.is_empty() {
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
            match serve(&app, &base, &token, &mut host).await {
                // 正常结束（登录凭证换了、退出）：马上重连一次
                Ok(()) => backoff = RETRY_MIN,
                Err(e) => {
                    warn!("can't keep the device connection: {e:#}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RETRY_MAX);
                    continue;
                }
            }
        }
    });
}

/// 连上并一直服务到断开。`Ok` = 该重连（凭证变了），`Err` = 出错了（退避后再试）
async fn serve(
    app: &AppHandle,
    base: &str,
    token: &str,
    host: &mut tokio::sync::watch::Receiver<AppStatus>,
) -> anyhow::Result<()> {
    let url = socket_url(base, token);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;
    info!(%base, "connected to the account service");
    // 文件传输也走这条连接：别处（Tauri 命令、应答方）往这个通道里塞消息
    let (out, mut outgoing) = mpsc::channel::<Value>(32);
    crate::peer::connect(out);
    // 报到的控制码：没见过 / 变了就再报一次
    let mut told: Option<u32> = None;
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // 连上就先报一次（控制码已经有了的话）。借用要在这里结束（`watch::Ref` 不是 Send）
    let known = host.borrow().host.code;
    if let Some(code) = known {
        hello(&mut ws, code).await?;
        told = Some(code);
    }

    let result = loop {
        tokio::select! {
            // 文件传输的消息、控制台命令都从这里出去
            Some(payload) = outgoing.recv() => {
                ws.send(Message::Text(payload.to_string().into())).await?;
            }
            message = ws.next() => match message {
                Some(Ok(Message::Text(text))) => on_text(app, &text),
                Some(Ok(Message::Close(_))) | None => {
                    debug!("the account service closed the device connection");
                    break Ok(());
                }
                Some(Err(e)) => break Err(e.into()),
                _ => {}
            },
            _ = host.changed() => {
                let code = host.borrow_and_update().host.code;
                if code != told {
                    if let Some(code) = code {
                        hello(&mut ws, code).await?;
                    }
                    told = code;
                }
            }
            _ = ping.tick() => {
                ws.send(Message::Text(json!({"type":"ping"}).to_string().into())).await?;
                // 登录凭证换了（改密码之后服务会作废旧凭证）：断开重连
                if accounts::token().as_deref() != Some(token) {
                    break Ok(());
                }
            }
        }
    };
    // 这条连接上的文件传输隧道全断了：让界面别再等
    crate::peer::disconnect();
    result
}

/// 报到：控制码、本机名字、系统
async fn hello<S>(ws: &mut S, code: u32) -> anyhow::Result<()>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let (name, os) = crate::sysinfo::host_identity();
    let payload = json!({
        "type": "hello",
        "code": format!("{code:06}"),
        "name": name,
        "os": os,
    });
    ws.send(Message::Text(payload.to_string().into())).await?;
    info!(code, "reported this device to the account service");
    Ok(())
}

/// 服务推来的消息：设备列表交给界面，别的不理
fn on_text(app: &AppHandle, text: &str) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    // 文件传输的消息（fs-*）交给 peer 模块，别的不理
    if crate::peer::handle(app, &value) {
        return;
    }
    match value.get("type").and_then(Value::as_str) {
        Some("devices") => {
            if let Err(e) = app.emit(EVENT, &value) {
                debug!("can't send the device list to the window: {e}");
            }
        }
        Some("error") => warn!("the account service refused this device: {value}"),
        _ => {}
    }
}

/// `https://host/path` → `wss://host/path/auth/ws?token=…`
fn socket_url(base: &str, token: &str) -> String {
    let base = base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{base}")
    };
    format!("{ws}/auth/ws?token={token}")
}
