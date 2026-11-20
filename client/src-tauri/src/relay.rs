//! 中转服务器的地址和证书指纹：**由账号服务分配**，用户不用填。
//!
//! ```text
//! GET {auth}/relay → {"relay": {"host": "relay.example.com", "port": 4433, "fingerprint": "<hex 或空>"}}
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
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// 中转服务器的证书指纹（空 = 首次连接时信任并记下）
    #[serde(default)]
    pub fingerprint: String,
}

fn default_port() -> u16 {
    4433
}

#[derive(Deserialize)]
struct Envelope {
    relay: Option<Relay>,
}

impl Relay {
    /// `host:port`，写进 `config.toml` 的 `server`
    pub fn address(&self) -> String {
        format!("{}:{}", self.host.trim(), self.port)
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
    if relay.host.trim().is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_joins_host_and_port() {
        let relay = Relay { host: " relay.example.com ".into(), port: 4433, fingerprint: String::new() };
        assert_eq!(relay.address(), "relay.example.com:4433");
    }

    #[test]
    fn defaults_when_the_service_sends_only_a_host() {
        let relay: Relay = serde_json::from_str(r#"{"host":"relay.example.com"}"#).unwrap();
        assert_eq!(relay.port, 4433);
        assert!(relay.fingerprint.is_empty());
        assert_eq!(relay.address(), "relay.example.com:4433");
    }

    #[test]
    fn envelope_without_a_relay_is_empty() {
        let env: Envelope = serde_json::from_str(r#"{"relay":null}"#).unwrap();
        assert!(env.relay.is_none());
        let env: Envelope = serde_json::from_str(r#"{"relay":{"host":"h","port":4433,"fingerprint":"ab"}}"#).unwrap();
        assert_eq!(env.relay.unwrap().address(), "h:4433");
    }

    #[test]
    fn trailing_slashes_are_trimmed() {
        assert_eq!(trim("https://auth.example.com//"), "https://auth.example.com");
    }
}
