//! 中转服务器的地址和证书指纹：**由账号服务分配**，用户不用填。
//!
//! ```text
//! GET {auth}/relay → {"relay": {"address": "relay.example.com:4433", "fingerprint": "<hex 或空>"}}
//! ```
//!
//! 启动时（以及改了账号服务地址之后）用这个结果写进 `config.toml` 的 `server` / `server_fp`，
//! 之后所有连接都按配置走。这样客户端界面上只有"账号服务地址"一个输入框；
//! 以后要做多线路、按账号分配不同中转、灰度换服务器，都只改账号服务，客户端不用动。
//!
//! 拉不到的时候（离线、账号服务挂了）**保留上一次成功拿到的值**：已经配好的客户端照样能连，
//! 只是没法换新的中转服务器。

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tracing::{info, warn};

use crate::config;

/// 请求超时
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Relay {
    /// `NIFE_REMOTE_AUTH_RELAY_HOST` 的原样内容：`host:port`，账号服务不拆、不补端口
    pub address: String,
    /// 中转服务器的证书指纹（空 = 首次连接时信任并记下）
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Deserialize)]
struct Envelope {
    relay: Option<Relay>,
}

impl Relay {
    /// 写进 `config.toml` 的 `server`：账号服务给什么就用什么
    pub fn address(&self) -> String {
        self.address.trim().to_string()
    }
}

/// 去掉末尾的斜杠
fn trim(url: &str) -> &str {
    url.trim().trim_end_matches('/')
}

/// 问账号服务要中转服务器的地址和指纹
pub async fn fetch(auth: &str) -> Result<Relay> {
    let url = format!("{}/relay", trim(auth));
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("building the HTTP client")?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("can't reach the account service at {url}"))?;
    if !response.status().is_success() {
        bail!("the account service answered {} for {url}", response.status());
    }
    let body: Envelope = response.json().await.context("unexpected reply from the account service")?;
    let Some(relay) = body.relay else {
        bail!("the account service hasn't assigned a relay server yet");
    };
    if relay.address.trim().is_empty() {
        bail!("the account service hasn't assigned a relay server yet");
    }
    Ok(relay)
}

/// 问一次，然后写进配置（`server` / `server_fp`）。失败时只记日志，保留原来的值
pub async fn resolve(auth: &str) -> Option<Relay> {
    match fetch(auth).await {
        Ok(relay) => {
            let address = relay.address();
            if let Err(e) = config::check_server(&address, &relay.fingerprint) {
                warn!("the account service returned an unusable relay ({address}): {e:#}");
                return None;
            }
            if config::save_server(&address, &relay.fingerprint).is_err() {
                warn!("can't save the relay assigned by the account service ({address})");
                return None;
            }
            info!(relay = %address, "relay server assigned by the account service");
            Some(relay)
        }
        Err(e) => {
            warn!("can't get the relay server from the account service: {e:#}");
            None
        }
    }
}


