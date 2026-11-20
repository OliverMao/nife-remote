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
        let Some(code) = normalize_code(code) else { return };
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

#[cfg(test)]
thread_local! {
    /// 测试时不碰真正的 profile.json
    static TEST_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn resolve() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Some(p) = TEST_PATH.with(|p| p.borrow().clone()) {
            return Some(p);
        }
        return None;
    }
    #[cfg(not(test))]
    {
        path()
    }
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
        let mut f = std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    info!("profile saved to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nife-remote-profile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE_NAME)
    }

    #[test]
    fn defaults_allow_assist_and_pairing() {
        let p = Profile::default();
        assert!(p.allow_assist);
        assert_eq!(p.verify, Verify::Pairing);
        assert!(p.favorites.is_empty());
        assert!(p.recent.is_empty());
    }

    #[test]
    fn missing_file_is_the_default_profile() {
        let path = temp_path("missing");
        assert_eq!(read_from(Some(&path)), Profile::default());
    }

    #[test]
    fn saves_and_reads_back() {
        let path = temp_path("roundtrip");
        let mut p = Profile::default();
        p.allow_assist = false;
        p.verify = Verify::Approval;
        p.favorites = vec!["482913".into()];
        p.push_recent("260778", "DESIGN-WS", 1_731_000_000);
        write_to(&path, &p).unwrap();
        assert_eq!(read_from(Some(&path)), p);
        // 写的是 JSON，人能看懂
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"allowAssist\": false"));
        assert!(text.contains("\"verify\": \"approval\""));
        // 没有留下临时文件
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn broken_json_falls_back_to_the_defaults() {
        let path = temp_path("broken");
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert_eq!(read_from(Some(&path)), Profile::default());
    }

    #[test]
    fn unknown_verify_is_pairing() {
        assert_eq!(Verify::parse("open"), Verify::Open);
        assert_eq!(Verify::parse(" APPROVAL "), Verify::Approval);
        assert_eq!(Verify::parse("whatever"), Verify::Pairing);
        let p: Profile = serde_json::from_str(r#"{"verify":"nonsense"}"#).unwrap_or_default();
        assert_eq!(p.verify, Verify::Pairing);
    }

    #[test]
    fn favorite_toggles_and_puts_the_new_one_first() {
        let mut p = Profile::default();
        assert!(p.toggle_favorite("482 913"));
        assert!(p.toggle_favorite("260778"));
        assert_eq!(p.favorites, vec!["260778", "482913"]);
        // 再点一次取消
        assert!(!p.toggle_favorite("260778"));
        assert_eq!(p.favorites, vec!["482913"]);
        // 不是 6 位数字的不记
        assert!(!p.toggle_favorite("12"));
        assert!(!p.toggle_favorite("abcdef"));
        assert_eq!(p.favorites, vec!["482913"]);
    }

    #[test]
    fn recent_keeps_the_newest_first_and_without_duplicates() {
        let mut p = Profile::default();
        p.push_recent("482913", "A", 1);
        p.push_recent("260778", "B", 2);
        p.push_recent("482913", "A2", 3);
        assert_eq!(p.recent.len(), 2);
        assert_eq!(p.recent[0].code, "482913");
        assert_eq!(p.recent[0].name, "A2");
        assert_eq!(p.recent[0].at, 3);
        p.drop_recent("482913");
        assert_eq!(p.recent.len(), 1);
        p.clear_recent();
        assert!(p.recent.is_empty());
    }

    #[test]
    fn recent_is_capped() {
        let mut p = Profile::default();
        for i in 0..(MAX_RECENT + 5) {
            p.push_recent(&format!("{}{:05}", 1 + i % 9, i), "", i as i64);
        }
        assert_eq!(p.recent.len(), MAX_RECENT);
    }

    #[test]
    fn bad_codes_are_rejected() {
        assert_eq!(normalize_code("260 778").as_deref(), Some("260778"));
        assert_eq!(normalize_code("012345"), None); // 控制码不以 0 开头
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("1234567"), None);
        assert_eq!(normalize_code(""), None);
    }
}
