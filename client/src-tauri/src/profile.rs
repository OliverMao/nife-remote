//! 界面自己的偏好和记录，存在 `profile.json`（和 `config.toml` 同一个目录，一个简单的本地 JSON 文件）：
//!
//! ```json
//! {
//!   "allowAssist": true,
//!   "verify": "pairing",
//!   "favorites": ["482913"],
//!   "recent": [{ "code": "482913", "name": "DESIGN-WS", "at": 1731000000 }]
//! }
//! ```
//!
//! * `allowAssist`：允许他人远程协助。关掉时正在控制本机的控制端会被断开，也不接受新的（见 [`crate::host::gate`]）；
//! * `verify`：验证方式——`pairing` 用配对码（默认）、`approval` 要本机同意、`open` 不用验证；
//! * `favorites` / `recent`：远程协助页里收藏的设备ID 和最近连接（控制码是钥匙，所以按设备ID 记就够了）。
//!
//! 这个文件是界面用的，被控端（系统服务）不读它：安装版的界面进程和服务进程共用同一个目录，
//! 服务里的被控端通过命名管道拿到"允许协助 / 验证方式"的改动（见 [`crate::winsvc::ipc`]）。
//!
//! 写文件是原子的：先写 `profile.json.tmp` 再改名，断电不会留下半个文件。读坏了就当默认值，不阻塞启动。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// 收藏最多记多少台
pub const MAX_FAVORITES: usize = 100;
/// 最近连接最多记多少条
pub const MAX_RECENT: usize = 20;
/// 名称长度上限（字符）
const MAX_NAME: usize = 80;

const FILE_NAME: &str = "profile.json";

/// 验证方式（不配对过的控制端怎么才能连上本机）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Verify {
    /// 用配对码（配对之后记住它）
    #[default]
    Pairing,
    /// 每次都要本机点同意
    Approval,
    /// 知道控制码就能连
    Open,
}

impl Verify {
    pub fn as_str(self) -> &'static str {
        match self {
            Verify::Pairing => "pairing",
            Verify::Approval => "approval",
            Verify::Open => "open",
        }
    }

    /// 界面传上来的字符串；不认识的就是 `pairing`（最安全的那一档）
    pub fn parse(s: &str) -> Verify {
        match s.trim().to_ascii_lowercase().as_str() {
            "approval" => Verify::Approval,
            "open" => Verify::Open,
            _ => Verify::Pairing,
        }
    }
}

/// 一条最近连接
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    /// 对方的设备ID（6 位数字）
    pub code: String,
    /// 那台电脑的名字（连上时才知道，可能是空的）
    #[serde(default)]
    pub name: String,
    /// 最后一次连接的时间（Unix 秒）
    #[serde(default)]
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    /// 允许他人远程协助
    pub allow_assist: bool,
    pub verify: Verify,
    pub favorites: Vec<String>,
    pub recent: Vec<Recent>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            allow_assist: true,
            verify: Verify::Pairing,
            favorites: Vec::new(),
            recent: Vec::new(),
        }
    }
}

impl Profile {
    /// 收藏 / 取消收藏，返回现在是不是收藏着
    pub fn toggle_favorite(&mut self, code: &str) -> bool {
        let Some(code) = normalize_code(code) else {
            return false;
        };
        if let Some(i) = self.favorites.iter().position(|c| *c == code) {
            self.favorites.remove(i);
            return false;
        }
        self.favorites.insert(0, code);
        self.favorites.truncate(MAX_FAVORITES);
        true
    }

    /// 记一条最近连接：同一台只留最新的一条，排在最前面
    pub fn push_recent(&mut self, code: &str, name: &str, at: i64) {
        let Some(code) = normalize_code(code) else {
            return;
        };
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        self.recent.retain(|r| r.code != code);
        self.recent.insert(0, Recent { code, name, at });
        self.recent.truncate(MAX_RECENT);
    }

    pub fn drop_recent(&mut self, code: &str) {
        self.recent.retain(|r| r.code != code);
    }

    pub fn clear_recent(&mut self) {
        self.recent.clear();
    }
}

/// 6 位数字的设备ID（"260 778" 这种带空格的也认），别的都不认
fn normalize_code(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 6 || digits.starts_with('0') {
        return None;
    }
    Some(digits)
}

/// `profile.json` 的路径：和 config.toml 在同一个目录里（见 [`crate::config::path`]）
pub fn path() -> Option<PathBuf> {
    Some(crate::config::path()?.with_file_name(FILE_NAME))
}

fn resolve() -> Option<PathBuf> {
    path()
}

/// 进程里读到的这份（之后的修改都走 [`update`]）
fn cache() -> &'static Mutex<Profile> {
    static CACHE: OnceLock<Mutex<Profile>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(read_from(resolve().as_deref())))
}

/// 现在的偏好（克隆一份给界面）
pub fn get() -> Profile {
    cache().lock().unwrap().clone()
}

/// 改一下再存：`f` 返回 `true` 才写盘
pub fn update<R>(f: impl FnOnce(&mut Profile) -> (bool, R)) -> Result<(Profile, R)> {
    let mut guard = cache().lock().unwrap();
    let mut next = guard.clone();
    let (changed, out) = f(&mut next);
    if changed && next != *guard {
        if let Some(path) = resolve() {
            if let Err(e) = write_to(&path, &next) {
                warn!("can't save the profile: {e:#}");
                return Err(e);
            }
        }
        *guard = next;
    }
    Ok((guard.clone(), out))
}

fn read_from(path: Option<&Path>) -> Profile {
    let Some(path) = path else {
        return Profile::default();
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Profile::default(),
        Err(e) => {
            warn!("{}: {e}", path.display());
            return Profile::default();
        }
    };
    // 个别键不对（比如手改坏了）不要整份丢掉：能救多少救多少
    match serde_json::from_str::<Profile>(&text) {
        Ok(p) => {
            let mut p = p;
            p.favorites.truncate(MAX_FAVORITES);
            p.recent.truncate(MAX_RECENT);
            p
        }
        Err(e) => {
            warn!("{}: {e}; using the defaults", path.display());
            Profile::default()
        }
    }
}

/// 原子写：先写临时文件再改名
fn write_to(path: &Path, profile: &Profile) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(profile).context("serializing the profile")?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    info!("profile saved to {}", path.display());
    Ok(())
}
