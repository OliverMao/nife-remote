//! 运行时产生的文件：用完就删，不留在系统里。
//!
//! Nife Remote 要保存的东西（设置、控制码、我的设备）都只在 config.toml 一个文件里（见 [`crate::config`]）；
//! 除此之外不在磁盘上留下任何东西。被控端模式完全不创建 WebView，因此不会产生其它文件。
//! 控制端界面需要 WebView（WebView2），WebView 需要一个用户数据目录；
//! 这里把它放到临时目录中的随机子目录，并开启隐私模式（不持久化 Cookie/缓存），在窗口关闭和程序退出时彻底删除。
//! 万一进程被强杀，下次启动时也会先清理残留目录。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

const PREFIX: &str = "nife-remote-webview-";

/// 控制台 WebView 的数据目录。
pub fn new_dir() -> PathBuf {
    std::env::temp_dir().join(format!("{PREFIX}{:016x}", rand::random::<u64>()))
}

/// 清理之前异常退出残留的目录（单实例锁保证不会误删正在使用的目录）。
pub fn sweep_stale() {
    sweep_cache();
}

/// 清理缓存目录，返回删掉的字节数（界面里"清理本地缓存"用）。
/// 正在用的那个目录删不掉（WebView 还开着它），下次启动时由 [`sweep_stale`] 收尾。
pub fn sweep_cache() -> u64 {
    let mut freed = 0;
    let places = vec![(std::env::temp_dir(), PREFIX)];
    for (dir, prefix) in places {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with(prefix) {
                let path = entry.path();
                let size = dir_size(&path);
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => {
                        freed += size;
                        debug!("removed leftover directory {}", path.display());
                    }
                    Err(e) => debug!("can't remove leftover directory {}: {e}", path.display()),
                }
            }
        }
    }
    freed
}

/// 目录里所有文件的大小（删之前算，报给界面）
fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// 删除目录；WebView 进程退出需要一点时间，所以会重试直到超时。
pub fn remove_with_retry(path: &PathBuf, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(e) => {
                if Instant::now() >= deadline {
                    warn!("can't remove temporary directory {}: {e}", path.display());
                    return false;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }
}

/// 后台删除（不阻塞调用方）。
pub fn remove_in_background(path: PathBuf) {
    std::thread::spawn(move || {
        remove_with_retry(&path, Duration::from_secs(30));
    });
}
