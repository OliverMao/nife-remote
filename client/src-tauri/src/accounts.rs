//! 账号：登录、注册、退出、账号下的设备列表。**账号在中转服务器旁边的账号服务里**（`server/auth`，FastAPI + JSON 文件），
//! 这个文件只是客户端：HTTP 调用 + 把登录凭证存在本机 `session.json`（和 `config.toml` 同一个目录）。
//!
//! ```text
//! POST {auth}/auth/register  {name, password} → {token, user}
//! POST {auth}/auth/login     {name, password} → {token, user}
//! POST {auth}/auth/logout    Bearer <token>
//! GET  {auth}/auth/me        Bearer <token> → {user}
//! GET  {auth}/auth/devices   Bearer <token> → {devices}
//! PUT  {auth}/auth/devices   Bearer <token> {devices}
//! ```
//!
//! 账号服务的地址：`config.toml` 的 `auth` → 环境变量 `NIFE_REMOTE_AUTH` → 编译进去的默认值，都没有就是
//! `http://127.0.0.1:8080`（本机调试 / 没配置账号服务时不至于直接崩，只是登录会报连不上）。
//!
//! 密码从不落盘：只有服务端知道盐和摘要，本机只保存登录凭证（`session.json`，权限 600）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// 账号服务地址（编译期默认值，可以用 `NIFE_REMOTE_AUTH` 覆盖）
const DEFAULT_AUTH: Option<&str> = option_env!("NIFE_REMOTE_AUTH");

const FILE_NAME: &str = "session.json";
/// 请求超时（账号服务很快，超过就是连不上）
const TIMEOUT: Duration = Duration::from_secs(10);

/// 给界面看的账号信息（不含密码，也没有盐和摘要——那些只在服务端）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub devices: Vec<String>,
}

/// 本机保存的登录状态
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Session {
    /// 登录凭证（服务端可以作废）
    token: Option<String>,
    /// 上次拿到的账号信息（离线时界面还能显示"已登录 xxx"）
    user: Option<AccountInfo>,
}

#[derive(Deserialize)]
struct AuthResponse {
    token: String,
    user: AccountInfo,
}

#[derive(Deserialize)]
struct UserResponse {
    user: AccountInfo,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: String,
}

// ───────────────────────── 地址 ─────────────────────────

/// 账号服务的地址（末尾不带 `/`）
pub fn base_url() -> String {
    // 界面里填的（config.toml 的 auth）优先：登录页 / 设置面板里改的就是它
    let saved = crate::config::auth();
    if !saved.trim().is_empty() {
        return trim(saved);
    }
    if let Some(url) = std::env::var("NIFE_REMOTE_AUTH").ok().filter(|s| !s.trim().is_empty()) {
        return trim(url);
    }
    match DEFAULT_AUTH {
        Some(url) if !url.trim().is_empty() => trim(url.to_string()),
        _ => "http://127.0.0.1:8080".to_string(),
    }
}

fn trim(mut url: String) -> String {
    while url.ends_with('/') {
        url.pop();
    }
    url
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("building the HTTP client")
}

// ───────────────────────── 接口 ─────────────────────────

/// 注册（成功后直接登录）
pub async fn register(name: &str, password: &str) -> Result<AccountInfo> {
    let r = client()?
        .post(format!("{}/auth/register", base_url()))
        .json(&serde_json::json!({ "name": name, "password": password }))
        .send()
        .await
        .context("can't reach the account service")?;
    let body = check(r).await?;
    let auth: AuthResponse = serde_json::from_value(body).context("unexpected reply from the account service")?;
    save_session(Session { token: Some(auth.token), user: Some(auth.user.clone()) })?;
    info!(account = %auth.user.name, "registered on the account service");
    Ok(auth.user)
}

/// 登录
pub async fn login(name: &str, password: &str) -> Result<AccountInfo> {
    let r = client()?
        .post(format!("{}/auth/login", base_url()))
        .json(&serde_json::json!({ "name": name, "password": password }))
        .send()
        .await
        .context("can't reach the account service")?;
    let body = check(r).await?;
    let auth: AuthResponse = serde_json::from_value(body).context("unexpected reply from the account service")?;
    save_session(Session { token: Some(auth.token), user: Some(auth.user.clone()) })?;
    info!(account = %auth.user.name, "logged in");
    Ok(auth.user)
}

/// 退出登录：让服务端作废凭证，然后清掉本机的
pub async fn logout() -> Result<()> {
    let session = load_session();
    if let Some(token) = session.token.clone() {
        // 连不上也要退出（本机凭证先删掉）
        let sent = client().and_then(|c| {
            Ok(c.post(format!("{}/auth/logout", base_url()))
                .bearer_auth(token)
                .send())
        });
        match sent {
            Ok(req) => {
                if let Err(e) = req.await {
                    warn!("can't tell the account service about the logout: {e}");
                }
            }
            Err(e) => warn!("can't build the logout request: {e:#}"),
        }
    }
    save_session(Session::default())
}

/// 现在登录的账号（本机记着的；没有就是 `None`）
pub fn current() -> Option<AccountInfo> {
    load_session().user
}

/// 打开界面时刷新一下账号信息（凭证还能用就用服务端的，过期了就当成没登录）
pub async fn refresh() -> Option<AccountInfo> {
    let session = load_session();
    let token = session.token.clone()?;
    let r = client().ok()?.get(format!("{}/auth/me", base_url())).bearer_auth(token).send().await.ok()?;
    if !r.status().is_success() {
        // 凭证过期 / 被作废：清掉本机的登录状态
        if r.status().as_u16() == 401 {
            let _ = save_session(Session::default());
        }
        return session.user;
    }
    let body: UserResponse = r.json().await.ok()?;
    let _ = save_session(Session { token: session.token, user: Some(body.user.clone()) });
    Some(body.user)
}

/// 账号下的设备列表（"本账号设备"）
pub async fn devices() -> Result<Vec<String>> {
    let session = load_session();
    let Some(token) = session.token else {
        bail!("not signed in");
    };
    let r = client()?
        .get(format!("{}/auth/devices", base_url()))
        .bearer_auth(token)
        .send()
        .await
        .context("can't reach the account service")?;
    let body = check(r).await?;
    let list = body.get("devices").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// 把设备列表存到账号里
pub async fn save_devices(codes: &[String]) -> Result<()> {
    let session = load_session();
    let Some(token) = session.token else {
        bail!("not signed in");
    };
    let r = client()?
        .put(format!("{}/auth/devices", base_url()))
        .bearer_auth(token)
        .json(&serde_json::json!({ "devices": codes }))
        .send()
        .await
        .context("can't reach the account service")?;
    check(r).await?;
    Ok(())
}

/// 改密码：要验旧的，成功后服务端作废所有旧凭证并给这一台发新的
pub async fn change_password(old_password: &str, new_password: &str) -> Result<AccountInfo> {
    let session = load_session();
    let Some(token) = session.token else {
        bail!("not signed in");
    };
    let r = client()?
        .post(format!("{}/auth/password", base_url()))
        .bearer_auth(token)
        .json(&serde_json::json!({ "old_password": old_password, "new_password": new_password }))
        .send()
        .await
        .context("can't reach the account service")?;
    let body = check(r).await?;
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if token.is_empty() {
        bail!("the account service didn't return a new login token");
    }
    let user = session.user.unwrap_or(AccountInfo {
        id: String::new(),
        name: String::new(),
        created_at: 0,
        devices: Vec::new(),
    });
    save_session(Session { token: Some(token), user: Some(user.clone()) })?;
    info!(account = %user.name, "password changed");
    Ok(user)
}

/// 服务端的错误信息直接给界面看
async fn check(r: reqwest::Response) -> Result<serde_json::Value> {
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str(&text).context("unexpected reply from the account service");
    }
    let message = serde_json::from_str::<ErrorBody>(&text)
        .map(|e| e.error.message)
        .ok()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("the account service answered {status}"));
    bail!("{message}")
}

// ───────────────────────── 本机保存的登录状态 ─────────────────────────

#[cfg_attr(test, allow(dead_code))]
fn path() -> Option<PathBuf> {
    Some(crate::config::path()?.with_file_name(FILE_NAME))
}

#[cfg(test)]
thread_local! {
    static TEST_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn resolve() -> Option<PathBuf> {
    #[cfg(test)]
    {
        return TEST_PATH.with(|p| p.borrow().clone());
    }
    #[cfg(not(test))]
    {
        path()
    }
}

fn cache() -> &'static Mutex<Session> {
    static CACHE: std::sync::OnceLock<Mutex<Session>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(read_from(resolve().as_deref())))
}

fn load_session() -> Session {
    cache().lock().unwrap().clone()
}

fn save_session(next: Session) -> Result<()> {
    let mut guard = cache().lock().unwrap();
    if *guard != next {
        if let Some(path) = resolve() {
            write_to(&path, &next)?;
        }
        *guard = next;
    }
    Ok(())
}

fn read_from(path: Option<&Path>) -> Session {
    let Some(path) = path else {
        return Session::default();
    };
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            warn!("{}: {e}; treating it as signed out", path.display());
            Session::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Session::default(),
        Err(e) => {
            warn!("{}: {e}", path.display());
            Session::default()
        }
    }
}

fn write_to(path: &Path, session: &Session) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(session).context("serializing the session")?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        let mut f = opts.open(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nife-remote-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE_NAME)
    }

    #[test]
    fn base_url_is_trimmed() {
        assert!(!base_url().ends_with('/'));
    }

    #[test]
    fn session_roundtrip() {
        let path = temp_path("roundtrip");
        let user = AccountInfo {
            id: "u1".into(),
            name: "mao".into(),
            created_at: 7,
            devices: vec!["260778".into()],
        };
        let session = Session { token: Some("t0ken".into()), user: Some(user.clone()) };
        write_to(&path, &session).unwrap();
        assert_eq!(read_from(Some(&path)), session);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"token\": \"t0ken\""));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_or_broken_file_means_signed_out() {
        assert_eq!(read_from(None), Session::default());
        let path = temp_path("broken");
        std::fs::write(&path, b"{ nope").unwrap();
        assert_eq!(read_from(Some(&path)), Session::default());
    }

    #[test]
    fn error_body_is_read() {
        let body: ErrorBody = serde_json::from_str(r#"{"error":{"code":"wrong_password","message":"wrong password"}}"#).unwrap();
        assert_eq!(body.error.message, "wrong password");
    }
}
