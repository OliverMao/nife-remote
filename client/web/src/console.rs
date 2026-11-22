//! 浏览器控制台的 WebSocket（`/ws`）。每个连接就是一个控制台（一个浏览器标签页），
//! 有自己的控制端服务和到中转服务器的连接，标签页关掉时它的会话全部断开。
//!
//! 协议和桌面版一样，见 `nife_remote_link::console`；这里只管接受连接（检查 Origin）、把 axum 的 WebSocket 接上去。
//! 剪贴板由浏览器自己同步（`nife_remote_link::controller::Frontend::Browser`）。

use std::net::SocketAddr;
use std::sync::Arc;

use nife_remote_link::console::{self, MAX_MESSAGE, Setup, WsMessage};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt, future};
use serde_json::json;
use tracing::{info, warn};

use crate::http::App;

pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers) {
        warn!(%peer, origin = ?headers.get(ORIGIN), "rejected a WebSocket opened by another website");
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| run(socket, app, peer))
}

/// 防止别的网站在用户的浏览器里偷偷连上来（浏览器总会带上 Origin）。
/// 经过反向代理时，代理需要保留 Host，或者设置 X-Forwarded-Host
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        // 不是浏览器
        return true;
    };
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|o| o.split_once("://"))
        .map(|(_, host)| host)
    else {
        return false;
    };
    [HOST.as_str(), "x-forwarded-host"]
        .into_iter()
        .filter_map(|h| headers.get(h)?.to_str().ok())
        .filter_map(|v| v.split(',').next())
        .any(|h| h.trim().eq_ignore_ascii_case(origin))
}

async fn run(socket: WebSocket, app: Arc<App>, peer: SocketAddr) {
    info!(%peer, "console opened");
    let socket = socket
        .with(|m| future::ready(Ok::<_, axum::Error>(to_axum(m))))
        .map(|m| m.map(from_axum));
    let setup = Setup {
        link: app.link.clone(),
        identity: app.identity.clone(),
        // 界面语言由浏览器自己决定
        info: json!({
            "version": env!("CARGO_PKG_VERSION"),
            "server": app.link.server,
            "configured": true,
            "pinned": app.link.pinned(),
            // 被控端看到的控制端名称
            "hostname": app.link.name,
        }),
        report: None,
        clipboard: None,
    };
    match console::serve(socket, setup).await {
        Ok(()) => info!(%peer, "console closed"),
        Err(e) => info!(%peer, "console closed: {e:#}"),
    }
}

fn from_axum(m: Message) -> WsMessage {
    match m {
        Message::Text(t) => WsMessage::Text(t.as_str().to_owned()),
        Message::Binary(b) => WsMessage::Binary(b),
        Message::Ping(_) | Message::Pong(_) => WsMessage::Ping,
        Message::Close(_) => WsMessage::Close,
    }
}

fn to_axum(m: WsMessage) -> Message {
    match m {
        WsMessage::Text(t) => Message::Text(t.into()),
        WsMessage::Binary(b) => Message::Binary(b),
        WsMessage::Ping => Message::Ping(Default::default()),
        WsMessage::Close => Message::Close(None),
    }
}


