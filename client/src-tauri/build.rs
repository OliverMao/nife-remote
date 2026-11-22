use std::path::Path;

/// 账号服务地址的默认值：环境变量和 `.env` 都没有时用它（正式服务）
const DEFAULT_AUTH: &str = "https://nife-remote-auth.naig.us";

fn main() {
    // Media Foundation 延迟加载：Windows N/KN 版（未装媒体功能包）没有 mfplat.dll，
    // 延迟加载后程序仍能启动，并自动回退到软件编码。
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:mfplat.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }
    // 账号服务地址：环境变量 > `.env` > 默认值。编译时定下来，写进程序（`env!("NIFE_REMOTE_AUTH")`），
    // 运行时不再读环境变量、界面上也看不到、改不了。
    let auth = account_service();
    println!("cargo:rustc-env=NIFE_REMOTE_AUTH={auth}");
    println!("cargo:rerun-if-env-changed=NIFE_REMOTE_AUTH");
    tauri_build::build()
}

/// 账号服务地址：环境变量优先，其次是 `client/.env` / `client/src-tauri/.env` 里的 `NIFE_REMOTE_AUTH`
fn account_service() -> String {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let mut from_env_file = None;
    for rel in ["../.env", ".env"] {
        let path = Path::new(&manifest).join(rel);
        println!("cargo:rerun-if-changed={}", path.display());
        if from_env_file.is_none() {
            from_env_file = read_env(&path, "NIFE_REMOTE_AUTH");
        }
    }
    let value = std::env::var("NIFE_REMOTE_AUTH")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or(from_env_file)
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_AUTH.to_string());
    value
}

/// 从 `.env` 里读一项（`KEY=value`，允许 `export ` 前缀和引号）
fn read_env(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim().trim_matches('"').trim_matches('\'').trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}
