//! 配置。
//!
//! * 中转服务器地址和证书指纹：**只有一个来源**——账号服务的 `GET /relay`（见 [`crate::relay`]）。账号服务的地址
//!   在**构建时**用 `NIFE_REMOTE_AUTH` 写进程序（见 [`crate::accounts`]），运行时不读环境变量、不读配置文件，
//!   界面上也改不了。装好后发现中转服务器变了（迁移、换机器）由账号服务通知，客户端自己跟上。
//!
//!   不用账号服务、直连中转服务器的老办法（`NIFE_REMOTE_SERVER` / `NIFE_REMOTE_SERVER_FP`）只作为构建期默认值保留，
//!   写在这里的地址是"出厂值"，客户端不会因为 `config.toml` 里写了别的而改连别处。
//!
//! * **配置文件 `config.toml`**（见 [`Settings`]）：不是给用户改的配置文件，是程序自己的状态（服务器分配的控制码
//!   （固定不变，见 `nife_remote_proto::signal` 的"固定控制码"）、控制台的"我的设备"（见 [`crate::devices`]），
//!   还有开机自动启动。
//!   所有数据都只在这一个文件里。位置（见 [`path`]）：程序所在目录（安装目录）。文件由系统服务里的被控端维护
//!   （见 [`crate::winsvc`]），托盘进程没有写权限，控制台保存设备时经命名管道交给它写。
//! * 文件由 Nife Remote 维护，手动修改的内容和注释都会保留；改了设置要重新启动 Nife Remote 才生效。
//! * 文件不存在时（第一次运行、被删掉了）按默认配置重新生成：启动时检查一次，运行中每隔几秒检查一次（[`keep_file`]），
//!   控制码、开机自动启动、我的设备、记下的服务器指纹从删掉之前的内容接着用，不会丢。
//! * 中转服务器的证书指纹同样来自账号服务（`[relay] fingerprint`），客户端不自己记：账号服务没给指纹时，
//!   本次运行信任第一次连上的那个证书（不落盘）。
//! * 端到端加密（见 `nife_remote_proto::handshake`）：本机作为被控端的身份密钥和配对码（`[host]`）、配对过的控制端
//!   （`[[paired]]`）、控制台的身份密钥（`[controller]`，Windows 上每个账户一把），也都在这个文件里，
//!   密钥和配对码加密保存（见 [`secret`]）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use nife_remote_link::LinkConfig;
use nife_remote_link::net::Trust;
use nife_remote_link::secure::{Access, Identity};
use nife_remote_proto::signal::Reclaim;
use toml_edit::{DocumentMut, Item};
use tracing::{error, info, warn};

/// 编译进程序的中转服务器地址 `host:port`（UDP），`config.toml` 里没有设置时使用。
pub const DEFAULT_SERVER: &str = match option_env!("NIFE_REMOTE_SERVER") {
    Some(v) => v,
    None => "127.0.0.1:4433",
};

/// 编译进程序的服务器证书 SHA-256 指纹（64 位十六进制，允许用 `:` 或空白分隔），`config.toml` 里没有设置时使用。
///
/// * 非空：严格校验，指纹不符直接拒绝连接（防中间人）。
/// * 为空：本次运行信任第一次连上的那个证书（不记到磁盘上，下次连上账号服务再拿新的）。
pub const DEFAULT_SERVER_FP: &str = match option_env!("NIFE_REMOTE_SERVER_FP") {
    Some(v) => v,
    None => "",
};

// 指纹写错了在编译时就报错，而不是等到运行时连不上服务器（规则与 nife_remote_link::net::parse_fingerprint 相同）
const _: () = assert!(
    fingerprint_is_valid(DEFAULT_SERVER_FP),
    "NIFE_REMOTE_SERVER_FP must be the 64 hex digit fingerprint printed in the server log (`:` and spaces are allowed)"
);

const fn fingerprint_is_valid(s: &str) -> bool {
    let b = s.as_bytes();
    let (mut i, mut digits) = (0, 0);
    while i < b.len() {
        if b[i].is_ascii_hexdigit() {
            digits += 1;
        } else if !(b[i] == b':' || b[i].is_ascii_whitespace()) {
            return false;
        }
        i += 1;
    }
    digits == 0 || digits == 64
}

/// 产品版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 配置文件名
pub const FILE_NAME: &str = "config.toml";

/// 卸载中（见 [`freeze`]）：不再写配置文件，删掉了也不再重新生成
static FROZEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 运行中多久检查一次配置文件还在不在
const KEEP_CHECK: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(3)
};

/// 进程启动时读到的配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 中转服务器地址（`[relay] address`）：账号服务分配的，不是用户配置
    pub server: String,
    /// 中转服务器证书指纹（`[relay] fingerprint`，账号服务给的），空字符串表示本次运行信任第一次连上的证书
    pub server_fp: String,
    /// 开机自动启动（`autostart`，安装版）：开机后被控端服务自动运行，登录之前也能被控制
    pub autostart: bool,
    /// 没配对过的控制端怎么办（`access`）：要配对码（默认），或者知道控制码就能连
    pub access: Access,
    /// 服务器分配的控制码和加密过的密钥（`[code]`），用 [`Settings::code`] 解开
    saved_code: Option<(i64, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: DEFAULT_SERVER.to_string(),
            server_fp: DEFAULT_SERVER_FP.to_string(),
            autostart: true,
            access: Access::Pairing,
            saved_code: None,
        }
    }
}

impl Settings {
    /// 保存的控制码和取回它用的密钥。密钥只有保存它的账户（安装版是服务里的被控端，SYSTEM）解得开，
    /// 解不开（例如配置文件是从别的电脑拷来的）时返回 `None`：让服务器重新分配一个
    pub fn code(&self) -> Option<Reclaim> {
        let (code, secret) = self.saved_code.as_ref()?;
        match (u32::try_from(*code), secret::open(secret)) {
            (Ok(code), Ok(secret)) => Some(Reclaim { code, secret }),
            (_, Err(e)) => {
                warn!("can't use the saved control code ({e:#}), getting a new one");
                None
            }
            _ => {
                warn!("invalid saved control code {code}, getting a new one");
                None
            }
        }
    }
}

/// 配置文件的路径：程序所在目录（安装目录）下的 `config.toml`，被控端（系统服务）和所有账户共用。
///
/// 找不到程序目录时（极少见）返回 `None`：这次运行不保存任何东西
pub fn path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(FILE_NAME))
}

/// 配置文件现在的内容（不存在、读不了时为 `None`）
pub fn read_text() -> Option<String> {
    std::fs::read_to_string(path()?).ok()
}

/// 控制码会不会保存下来（有配置文件）：保存的话，注册时请服务器一直为它保留（`Register::keep`）
pub fn persistent() -> bool {
    path().is_some()
}

/// 进程启动时读到的配置（之后的修改，例如更换控制码，由各自的模块记在内存里）。
pub fn settings() -> &'static Settings {
    static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
        let Some(path) = path() else {
            return Settings::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text).unwrap_or_else(|e| {
                error!("{}: {e:#}; using the defaults", path.display());
                Settings::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                error!("can't read {}: {e}; using the defaults", path.display());
                Settings::default()
            }
        }
    });
    &SETTINGS
}

fn parse(text: &str) -> Result<Settings> {
    let doc: DocumentMut = text.parse()?;
    let mut s = Settings::default();
    if let Some(v) = doc.get("server").and_then(|v| v.as_str())
        && !v.trim().is_empty()
    {
        s.server = v.trim().to_string();
    }
    if let Some(t) = doc.get("relay").and_then(|v| v.as_table_like()) {
        if let Some(v) = t.get("address").and_then(|v| v.as_str())
            && !v.trim().is_empty()
        {
            s.server = v.trim().to_string();
        }
        if let Some(v) = t.get("fingerprint").and_then(|v| v.as_str()) {
            s.server_fp = v.to_string();
        }
    }
    // 旧版本写在顶层的这两个（那时是用户配置）：读出来用，下次写文件时搬进 [relay]
    if doc.get("relay").is_none() {
        if let Some(v) = doc.get("server_fp").and_then(|v| v.as_str()) {
            s.server_fp = v.to_string();
        }
    }
    if let Some(v) = doc.get("autostart").and_then(|v| v.as_bool()) {
        s.autostart = v;
    }
    // 只有写明了 "open" 才不要配对码：写错了的按更安全的来
    if let Some(v) = doc.get("access").and_then(|v| v.as_str()) {
        s.access = match v.trim() {
            "open" => Access::Open,
            "pairing" => Access::Pairing,
            other => {
                warn!("config.toml: unknown access {other:?}, using \"pairing\"");
                Access::Pairing
            }
        };
    }
    if let Some(t) = doc.get("code").and_then(|v| v.as_table_like()) {
        let code = t.get("code").and_then(|v| v.as_integer());
        let secret = t.get("secret").and_then(|v| v.as_str());
        if let (Some(code), Some(secret)) = (code, secret) {
            s.saved_code = Some((code, secret.to_string()));
        }
    }
    Ok(s)
}

/// 连接中转服务器用的配置：服务器地址与指纹，加上本机名称（被控端和控制端共用）
pub fn link() -> Arc<LinkConfig> {
    static LINK: LazyLock<Arc<LinkConfig>> = LazyLock::new(|| {
        let s = settings();
        let (name, os) = crate::sysinfo::host_identity();
        let configured = server_configured();
        let link = LinkConfig::new(s.server.clone(), &s.server_fp, name.clone(), os.clone())
            .unwrap_or_else(|e| {
                // 指纹写错了：不能因此就不校验证书。用一个不可能对得上的指纹，连接时报“证书指纹不匹配”
                error!("config.toml: invalid server_fp ({e:#}), refusing to connect");
                LinkConfig {
                    server: s.server.clone(),
                    trust: Trust::Pinned([0; 32]),
                    name,
                    os,
                    configured,
                }
            });
        // 没配置过服务器（用的是占位的 127.0.0.1:4433）：不连接、不重试
        Arc::new(if configured {
            link
        } else {
            link.unconfigured()
        })
    });
    LINK.clone()
}

/// 配置过中转服务器没有：`config.toml` 里写了 `server`，或者构建时用 `NIFE_REMOTE_SERVER` 给了默认值。
/// 都没给的话用的只是占位值（`127.0.0.1:4433`），不连接也不重试（见 `LinkConfig::configured`）
pub fn server_configured() -> bool {
    static CONFIGURED: LazyLock<bool> = LazyLock::new(|| {
        if option_env!("NIFE_REMOTE_SERVER").is_some() {
            return true;
        }
        let Some(path) = path() else {
            return false;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        text.parse::<DocumentMut>()
            .ok()
            .and_then(|doc| {
                let relay = doc
                    .get("relay")
                    .and_then(|t| t.as_table_like())
                    .and_then(|t| t.get("address"))
                    .and_then(|v| v.as_str())
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                let server = doc
                    .get("server")
                    .and_then(|v| v.as_str())
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                Some(relay || server)
            })
            .unwrap_or(false)
    });
    *CONFIGURED
}

// ───────────────────────────── 写配置 ─────────────────────────────

/// 新建配置文件时的内容（按系统语言写注释）
fn template() -> String {
    template_windows()
}

/// Windows 的模板：文件在安装目录里，由系统服务维护
fn template_windows() -> String {
    let access = access_comment();
    tr!(
        r#"# Nife Remote 的状态文件（不是配置文件：服务器地址在打包时写进程序、由账号服务分配，这里没有可以改的连接设置）。
# 由 Nife Remote 自己维护（控制码、开机自动启动、控制台的"我的设备"），关机前不用管它。
# 不小心删掉了也没关系：Nife Remote 会重新生成，控制码和我的设备不会丢。

{access}

# 开机自动启动：开机后被控端服务自动运行（登录之前也能被控制），登录后显示托盘图标。托盘菜单里也可以切换
autostart = true

# 控制码：第一次连上服务器时由服务器分配，之后一直使用同一个（重启、掉线、升级都不变）。
# 托盘菜单"更换控制码"会换一个新的；删掉 code 和 secret 则下次启动时重新分配。
# secret 是取回这个控制码的凭据，用 Windows DPAPI 加密过，只有这台电脑上的 Nife Remote 能解开
[code]
"#,
        r#"# Nife Remote's state file (not a configuration file: the account service address is compiled in and the relay is assigned by it,
# so there is no connection setting to edit here). Nife Remote maintains it (control code, autostart, the console's "My devices").
# If it gets deleted by accident, Nife Remote recreates it, keeping the control code and your devices.

{access}

# Start with Windows: the host service runs after boot (the computer can be controlled before anyone signs in),
# and the tray icon shows up after signing in. Can also be changed in the tray menu
autostart = true

# Control code: assigned by the server the first time, then always the same (across restarts, disconnects and upgrades).
# "Change control code" in the tray menu gets a new one; delete code and secret to get a new one at the next start.
# secret proves to the server that this computer owns the code. It's encrypted with Windows DPAPI, only Nife Remote on this computer can read it
[code]
"#
    )
}

/// 模板里 `access` 的说明
fn access_comment() -> String {
    tr!(
        "# 没配对过的控制端能不能连本机。会话是端到端加密的，中转服务器看不到画面和键盘输入，但它知道所有控制码：\n#   \"pairing\"：第一次连接时要输入托盘菜单里的配对码（配对码不经过服务器），配对之后记住它（默认）；\n#   \"open\"：知道控制码就能连。服务器也知道控制码，所以这等于信任中转服务器\n# access = \"pairing\"",
        "# Whether controllers that haven't paired with this computer may connect. Sessions are end-to-end encrypted, so the relay server can't see the screen or keystrokes, but it does know every control code:\n#   \"pairing\": the first connection needs the pairing code shown in the tray menu (it never goes through the server); paired controllers are remembered (default);\n#   \"open\": the control code alone is enough. The server knows it too, so this means trusting the relay server\n# access = \"pairing\""
    )
}

/// 写在第一台设备前面的说明
fn devices_comment() -> String {
    let how = tr!(
        "code 用你的 Windows 账户加密过，别的账户、别的电脑解不开；手动添加时直接写 6 位数字。",
        "code is encrypted with your Windows account; other accounts and computers can't read it. To add one by hand, just write the 6 digits."
    );
    tr!(
        "# 我的设备：控制台连接过的电脑，连上时自动保存（\"新建连接\"页里的\"我的设备\"，最多 50 台）。\n# {how}\n",
        "# My devices: computers the console has connected to, saved automatically (\"My devices\" on the New connection page, up to 50).\n# {how}\n"
    )
}

/// 本进程最近一次读到 / 写入的配置文件内容。文件被删掉时从这里接着用控制码、开机自动启动、我的设备（见 [`regenerate`]）。
/// 同时也是读改写配置文件的锁：被控端保存控制码、控制台保存设备可能同时发生
static LAST: Mutex<Option<String>> = Mutex::new(None);

fn last() -> MutexGuard<'static, Option<String>> {
    LAST.lock().unwrap_or_else(|e| e.into_inner())
}

/// 按模板（默认配置）生成配置文件的内容。`before` 是文件被删掉之前的内容：服务器等设置回到默认，
/// 状态接着用——开机自动启动（Windows）、本机的控制码、我的设备。只搬值，模板里的说明和顺序不动
fn regenerate(before: Option<&str>) -> Result<DocumentMut> {
    let mut doc: DocumentMut = template().parse()?;
    let Some(old) = before.and_then(|t| t.parse::<DocumentMut>().ok()) else {
        return Ok(doc);
    };
    if let Some(on) = old.get("autostart").and_then(Item::as_bool) {
        put_autostart(&mut doc, on);
    }
    if let (Some(from), Some(to)) = (
        old.get("code").and_then(Item::as_table_like),
        doc.get_mut("code").and_then(Item::as_table_like_mut),
    ) {
        for (k, v) in from.iter() {
            to.insert(k, v.clone());
        }
    }
    // 端到端加密的身份、配对码、配对过的控制端：丢了的话控制端会认为本机"身份变了"，配对过的也要重新配对
    for key in ["host", "controller"] {
        if let Some(item) = old.get(key).filter(|i| i.is_table()) {
            doc[key] = item.clone();
        }
    }
    if let Some(item) = old.get("paired").filter(|i| i.is_array_of_tables()) {
        doc["paired"] = item.clone();
    }
    // 设备写在最后，前面有自己的说明
    if let Some(item) = old.get("device").filter(|i| i.is_array_of_tables()) {
        doc["device"] = item.clone();
    }
    Ok(doc)
}

/// 修改配置文件（不存在时按默认配置重新生成）。没有配置文件时什么也不做
fn edit(f: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    match path() {
        Some(path) => edit_at(&path, f),
        None => Ok(()),
    }
}

fn edit_at(path: &Path, f: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let mut last = last();
    if FROZEN.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(());
    }
    let mut doc = match std::fs::read_to_string(path) {
        // 用户写坏了的文件不覆盖：这次就不保存了
        Ok(text) => text.parse::<DocumentMut>().map_err(|e| {
            anyhow::anyhow!("{} isn't valid TOML, leaving it alone: {e}", path.display())
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => regenerate(last.as_deref())?,
        Err(e) => return Err(e.into()),
    };
    migrate(&mut doc);
    f(&mut doc)?;
    let text = doc.to_string();
    write(path, &text)?;
    *last = Some(text);
    Ok(())
}

/// 先写临时文件再改名：写到一半断电也不会把配置文件弄坏。目录不存在时创建（整个目录被删掉了）。
fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 配置文件不存在时按默认配置新建，存在时记下它的内容（见 [`keep_file`]）。
/// 维护配置文件的进程启动时调用（Windows 安装版是服务里的被控端，其它情况是托盘进程），安装时也调用
pub fn ensure_file() -> Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            if text.parse::<DocumentMut>().is_ok() {
                *last() = Some(text);
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            edit(|_| Ok(()))?;
            info!("created {}", path.display());
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// 运行中配置文件被删掉了（文件或者整个目录）：几秒之内按默认配置重新生成，控制码、开机自动启动、
/// 我的设备从删掉之前的内容接着用。只在维护配置文件的进程里调用（见 [`ensure_file`]）
pub fn keep_file() {
    let Some(path) = path() else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("nife-remote-config".into())
        .spawn(move || {
            let mut seen = None;
            loop {
                std::thread::sleep(KEEP_CHECK);

                if FROZEN.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                match std::fs::metadata(&path) {
                    Ok(m) => {
                        // 文件变了（包括手动修改）：记下新的内容，删掉之后接着用的是最新的
                        let modified = m.modified().ok();
                        if modified != seen {
                            seen = modified;
                            let mut last = last();
                            if let Ok(text) = std::fs::read_to_string(&path)
                                && text.parse::<DocumentMut>().is_ok()
                            {
                                *last = Some(text);
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        seen = None;
                        match edit_at(&path, |_| Ok(())) {
                            Ok(()) => info!("{} was deleted, recreated it", path.display()),
                            Err(e) => warn!("can't recreate {}: {e:#}", path.display()),
                        }
                    }
                    Err(_) => {}
                }
            }
        });
    if let Err(e) = spawned {
        warn!("can't watch the configuration file: {e}");
    }
}

/// 保存服务器分配的控制码；`None` 表示放弃了原来的控制码（下次启动时重新分配）
pub fn save_code(code: Option<Reclaim>) -> Result<()> {
    edit(|doc| {
        let table = doc
            .entry("code")
            .or_insert(toml_edit::table())
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("`code` in config.toml must be a table"))?;
        match code {
            Some(r) => {
                table["code"] = toml_edit::value(i64::from(r.code));
                table["secret"] = toml_edit::value(secret::seal(&r.secret)?);
            }
            None => {
                table.remove("code");
                table.remove("secret");
            }
        }
        Ok(())
    })
}

/// 保存开机自动启动的设置
pub fn save_autostart(on: bool) -> Result<()> {
    edit(|doc| {
        put_autostart(doc, on);
        Ok(())
    })
}

/// 检查账号服务给回来的中转服务器设置（不改文件）：
/// 地址必须是 `host:port`，指纹为空（本次运行信任第一次连上的证书）或者 64 位十六进制。
pub fn check_server(server: &str, fp: &str) -> Result<()> {
    let server = server.trim();
    anyhow::ensure!(!server.is_empty(), "the server address is empty");
    anyhow::ensure!(
        matches!(server.rsplit_once(':'), Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok()),
        "the server address must be host:port, e.g. relay.example.com:4433"
    );
    anyhow::ensure!(
        fingerprint_is_valid(fp.trim()),
        "the fingerprint must be 64 hex digits (or empty)"
    );
    Ok(())
}

/// 检查首页里填的配对码：6 位数字（本机生成的，可以自己改，见 [`crate::host::gate::DesktopGate::set_pairing_code`]）
pub fn check_pairing(code: &str) -> Result<()> {
    let code = code.trim();
    anyhow::ensure!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
        "the pairing code must be 6 digits"
    );
    Ok(())
}

/// 写在 `[relay]` 前面的说明（程序自己的状态，不是要用户改的配置）
fn relay_comment() -> String {
    tr!(
        "# 账号服务分配的中转服务器（客户端自己写的：改了也没用，下次连上账号服务就换回来）
",
        "# The relay server assigned by the account service (written by the client; edits are overwritten at the next sign-in)
"
    )
}

/// 把账号服务分配的中转服务器地址和指纹写进配置文档（`[relay]`，程序自己的状态，不是用户配置）；
/// 顺手删掉旧版本写在顶层的 `server` / `server_fp` / `auth` 和 `[server_trust]`（指纹不再记在本地）
fn put_server(doc: &mut DocumentMut, server: &str, fp: &str) {
    let item = doc.entry("relay").or_insert_with(|| {
        let mut t = toml_edit::Table::new();
        t.decor_mut().set_prefix(format!(
            "
{}",
            relay_comment()
        ));
        Item::Table(t)
    });
    if !item.is_table() {
        *item = Item::Table(toml_edit::Table::new());
    }
    item["address"] = toml_edit::value(server);
    item["fingerprint"] = toml_edit::value(fp);
    doc.remove("server");
    doc.remove("server_fp");
    doc.remove("auth");
    doc.remove("server_trust");
}

/// 旧版本把中转服务器和账号服务地址写成顶层配置（`server` / `server_fp` / `auth`）：搬进 `[relay]` 并删掉。
/// 用户改过的那份不再有作用——地址只认账号服务给的
fn migrate(doc: &mut DocumentMut) {
    let has_old = ["server", "server_fp", "auth"]
        .iter()
        .any(|k| doc.get(*k).is_some());
    if !has_old {
        return;
    }
    let address = doc
        .get("server")
        .and_then(Item::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let fp = doc
        .get("server_fp")
        .and_then(Item::as_str)
        .unwrap_or("")
        .to_string();
    if !address.is_empty() && doc.get("relay").is_none() {
        put_server(doc, &address, &fp);
        // put_server 已经把旧的键删掉了
        return;
    }
    for k in ["server", "server_fp", "auth"] {
        doc.remove(k);
    }
}

/// 保存账号服务分配的中转服务器地址和证书指纹（`fp` 为空表示首次连接时信任）。
///
/// 服务器地址在启动时读一次（见 [`settings`]），所以保存后要重启 Nife Remote 才生效。
pub fn save_server(server: &str, fp: &str) -> Result<()> {
    check_server(server, fp)?;
    let server = server.trim().to_string();
    let fp = fp.trim().to_string();
    edit(move |doc| {
        put_server(doc, &server, &fp);
        Ok(())
    })
}

/// 写入 `autostart`。已经有了就只改值；没有时加上一段说明，放在文件开头的说明后面
/// （直接插入的话 toml_edit 会把它排在整个文件的第一行，开头的说明前面）
fn put_autostart(doc: &mut DocumentMut, on: bool) {
    if let Some(item) = doc.get_mut("autostart") {
        *item = toml_edit::value(on);
        return;
    }
    let note = autostart_note();
    // 文件开头的说明属于排在最前面的那一项：已经有别的值（例如用户写了 server）时是那个值，
    // 否则是第一个表（模板里的 [code]），这时在最后一个空行处分开，前半段（开头的说明）移到 autostart 前面
    let has_values = doc.iter().any(|(_, item)| item.is_value());
    let mut prefix = format!("\n{note}");
    if !has_values
        && let Some((_, item)) = doc.iter_mut().find(|(_, item)| item.is_table())
        && let Some(table) = item.as_table_mut()
    {
        let old = table
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string();
        let (head, tail) = old
            .rfind("\n\n")
            .map_or((old.as_str(), ""), |i| old.split_at(i + 1));
        prefix = format!("{head}\n{note}");
        table
            .decor_mut()
            .set_prefix(format!("\n{}", tail.trim_start_matches('\n')));
    }
    doc.insert("autostart", toml_edit::value(on));
    if let Some(mut key) = doc.as_table_mut().key_mut("autostart") {
        key.leaf_decor_mut().set_prefix(prefix);
    }
}

/// `autostart` 前面的说明
fn autostart_note() -> String {
    tr!(
        "# 开机自动启动：开机后被控端服务自动运行（登录之前也能被控制），登录后显示托盘图标。托盘菜单里也可以切换\n",
        "# Start with Windows: the host service runs after boot (the computer can be controlled before anyone signs in),\n# and the tray icon shows up after signing in. Can also be changed in the tray menu\n"
    )
}

/// 保存控制台的"我的设备"：整个列表换成 `list`（[`crate::devices::for_file`] 整理过的，`code` 已经加密）
pub fn save_devices(list: &[crate::devices::Device]) -> Result<()> {
    edit(|doc| {
        put_devices(doc, list);
        Ok(())
    })
}

fn put_devices(doc: &mut DocumentMut, list: &[crate::devices::Device]) {
    let mut arr = toml_edit::ArrayOfTables::new();
    for t in list
        .iter()
        .take(crate::devices::MAX_IN_FILE)
        .filter_map(crate::devices::to_table)
    {
        arr.push(t);
    }
    if let Some(first) = arr.get_mut(0) {
        first
            .decor_mut()
            .set_prefix(format!("\n{}", devices_comment()));
    }
    doc.remove("device");
    if !arr.is_empty() {
        doc.insert("device", Item::ArrayOfTables(arr));
    }
}

// ───────────────────────────── 端到端加密的身份和配对 ─────────────────────────────

/// 配对过的一个控制端（`[[paired]]`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    /// 公钥（64 位十六进制）
    pub key: String,
    /// 它配对时报的名称
    pub name: String,
    /// 配对的时间（Unix 毫秒）
    pub since: i64,
}

/// 本机作为被控端的端到端状态（`[host]` 和 `[[paired]]`）
#[derive(Debug, Default)]
pub struct HostSecurity {
    /// 身份密钥（私钥）；没有、解不开时为 `None`
    pub key: Option<[u8; 32]>,
    /// 配对码
    pub pairing: Option<String>,
    /// 有人连续输错配对码，暂停了配对（本机用户换一个配对码才恢复）
    pub pairing_locked: bool,
    pub paired: Vec<Paired>,
}

/// 写在 `[host]` 前面的说明
fn host_comment() -> String {
    tr!(
        "# 本机作为被控端的端到端加密身份（key）和配对码（pairing），由 Nife Remote 维护，都是加密保存的。\n# 删掉 key 会生成新的身份：连过本机的控制端会提示本机的身份变了\n",
        "# This computer's end-to-end encryption identity as a host (key) and its pairing code (pairing). Maintained by Nife Remote, stored encrypted.\n# Deleting key creates a new identity: controllers that connected before will warn that this computer's identity changed\n"
    )
}

/// 写在第一个 `[[paired]]` 前面的说明
fn paired_comment() -> String {
    tr!(
        "# 配对过的控制端（它们的公钥）：不用配对码就能连本机。删掉一项就是忘掉那个控制端\n",
        "# Paired controllers (their public keys): they can connect without the pairing code. Delete an entry to forget that controller\n"
    )
}

/// 本机作为被控端的端到端状态（每次重新读文件）。密钥解不开（例如配置文件是从别的电脑拷来的）当作没有
pub fn host_security() -> HostSecurity {
    let Some(doc) = read_text().and_then(|t| t.parse::<DocumentMut>().ok()) else {
        return HostSecurity::default();
    };
    host_security_in(&doc)
}

fn host_security_in(doc: &DocumentMut) -> HostSecurity {
    let mut s = HostSecurity::default();
    if let Some(t) = doc.get("host").and_then(Item::as_table_like) {
        let open = |k: &str, kind| {
            let text = t.get(k)?.as_str()?;
            match secret::open_bytes(text, kind) {
                Ok(b) => Some(b),
                Err(e) => {
                    warn!("config.toml: can't read [host] {k} ({e:#})");
                    None
                }
            }
        };
        s.key = open("key", secret::Kind::HostKey).and_then(|b| <[u8; 32]>::try_from(b).ok());
        s.pairing = open("pairing", secret::Kind::Pairing)
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|p| nife_remote_proto::parse_pairing(&p));
        s.pairing_locked = t
            .get("pairing_locked")
            .and_then(Item::as_bool)
            .unwrap_or(false);
    }
    if let Some(arr) = doc.get("paired").and_then(Item::as_array_of_tables) {
        s.paired = arr
            .iter()
            .filter_map(|t| {
                let key = t.get("key")?.as_str()?.trim().to_ascii_lowercase();
                nife_remote_link::secure::unhex32(&key)?;
                Some(Paired {
                    key,
                    name: t
                        .get("name")
                        .and_then(Item::as_str)
                        .unwrap_or_default()
                        .chars()
                        .take(64)
                        .collect(),
                    since: match t.get("since") {
                        Some(Item::Value(toml_edit::Value::Datetime(d))) => {
                            crate::devices::to_millis(d.value()).unwrap_or(0)
                        }
                        _ => 0,
                    },
                })
            })
            .collect();
    }
    s
}

/// 保存本机作为被控端的身份密钥
pub fn save_host_key(key: &[u8; 32]) -> Result<()> {
    let sealed = secret::seal_bytes(key, secret::Kind::HostKey)?;
    edit(|doc| {
        host_table(doc)?["key"] = toml_edit::value(sealed);
        Ok(())
    })
}

/// 保存配对码和它有没有暂停
pub fn save_pairing(code: &str, locked: bool) -> Result<()> {
    let sealed = secret::seal_bytes(code.as_bytes(), secret::Kind::Pairing)?;
    edit(|doc| {
        let t = host_table(doc)?;
        t["pairing"] = toml_edit::value(sealed);
        t["pairing_locked"] = toml_edit::value(locked);
        Ok(())
    })
}

/// 保存配对过的控制端（整个换掉）
pub fn save_paired(list: &[Paired]) -> Result<()> {
    edit(|doc| {
        put_paired(doc, list);
        Ok(())
    })
}

fn put_paired(doc: &mut DocumentMut, list: &[Paired]) {
    let mut arr = toml_edit::ArrayOfTables::new();
    for p in list.iter().take(MAX_PAIRED) {
        let mut t = toml_edit::Table::new();
        t["key"] = toml_edit::value(&p.key);
        t["name"] = toml_edit::value(&p.name);
        if let Some(dt) = crate::devices::from_millis(p.since) {
            t["since"] = toml_edit::value(dt);
        }
        arr.push(t);
    }
    if let Some(first) = arr.get_mut(0) {
        first
            .decor_mut()
            .set_prefix(format!("\n{}", paired_comment()));
    }
    doc.remove("paired");
    if !arr.is_empty() {
        doc.insert("paired", Item::ArrayOfTables(arr));
    }
}

/// 最多记住这么多个配对过的控制端（再多就忘掉最早的）
pub const MAX_PAIRED: usize = 64;

fn host_table(doc: &mut DocumentMut) -> Result<&mut toml_edit::Table> {
    let item = doc.entry("host").or_insert_with(|| {
        let mut t = toml_edit::Table::new();
        t.decor_mut().set_prefix(format!("\n{}", host_comment()));
        Item::Table(t)
    });
    item.as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("`host` in config.toml must be a table"))
}

/// 控制台的身份（端到端握手里的静态密钥）：`[controller]` 里这个账户解得开的那一把。
///
/// Windows 上配置文件是所有账户共用的，每个账户一把，用各自的 DPAPI 加密（和我的设备里的控制码一样）。
/// 没有就生成一把，返回要保存的那一项（加密过的）：调用方自己写进去（[`save_controller_key`]），
/// Windows 安装版经命名管道交给服务里的被控端写
pub fn controller_identity() -> (Identity, Option<String>) {
    let doc = read_text().and_then(|t| t.parse::<DocumentMut>().ok());
    let found = doc.as_ref().and_then(|doc| {
        doc.get("controller")?
            .get("keys")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str())
            .find_map(|s| secret::open_bytes(s, secret::Kind::ControllerKey).ok())
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
    });
    if let Some(k) = found {
        return (Identity::from_private(k), None);
    }
    let id = Identity::generate();
    let sealed = match secret::seal_bytes(id.private(), secret::Kind::ControllerKey) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!("can't encrypt the console's identity ({e:#}), it's only kept until restart");
            None
        }
    };
    (id, sealed)
}

/// `[controller]` 里最多这么多把（Windows 上每个账户一把）
const MAX_CONTROLLER_KEYS: usize = 32;

/// 记下控制台的身份（[`controller_identity`] 返回的那一项）。别的账户的原样保留
pub fn save_controller_key(sealed: &str) -> Result<()> {
    let sealed = sealed.trim();
    anyhow::ensure!(
        !sealed.is_empty() && sealed.len() <= 1024 && !sealed.contains(['\n', '\r']),
        "invalid controller key"
    );
    edit(|doc| {
        let item = doc.entry("controller").or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.decor_mut()
                .set_prefix(format!("\n{}", controller_comment()));
            Item::Table(t)
        });
        let t = item
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("`controller` in config.toml must be a table"))?;
        let mut keys = t
            .get("keys")
            .and_then(Item::as_array)
            .cloned()
            .unwrap_or_default();
        if !keys.iter().any(|v| v.as_str() == Some(sealed)) {
            keys.push(sealed);
        }
        while keys.len() > MAX_CONTROLLER_KEYS {
            keys.remove(0);
        }
        t["keys"] = toml_edit::value(keys);
        Ok(())
    })
}

/// 写在 `[controller]` 前面的说明
fn controller_comment() -> String {
    tr!(
        "# 控制台的端到端加密身份（加密保存，Windows 上每个账户一把）。配对过的电脑靠它认出这个控制台；删掉了就要重新配对\n",
        "# The console's end-to-end encryption identity (stored encrypted, one per account on Windows). Paired computers recognize this console by it; delete it and you'll have to pair again\n"
    )
}

/// 加密保存的东西：取回控制码用的密钥、"我的设备"里的控制码、端到端加密的身份密钥和配对码。
///
/// config.toml 在安装目录里，本机所有账户都能读，所以用 DPAPI 加密（只有同一个账户能解开：
/// 被控端的密钥是 SYSTEM，设备的控制码是保存它的用户），配置文件被别人读到、拷走也没用。
pub(crate) mod secret {
    use anyhow::{Result, bail};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    const ENTROPY: &[u8] = b"nife-remote control code secret v1";
    const DEVICE_ENTROPY: &[u8] = b"nife-remote saved device code v1";

    /// 端到端加密用到的密文：各用各的 DPAPI 熵，不能互相冒充
    #[derive(Debug, Clone, Copy)]
    pub enum Kind {
        /// 本机作为被控端的身份密钥（Windows：SYSTEM 的 DPAPI）
        HostKey,
        /// 配对码（同上）
        Pairing,
        /// 控制台的身份密钥（Windows：用户的 DPAPI）
        ControllerKey,
    }

    fn entropy(kind: Kind) -> &'static [u8] {
        match kind {
            Kind::HostKey => b"nife-remote host identity v1",
            Kind::Pairing => b"nife-remote pairing code v1",
            Kind::ControllerKey => b"nife-remote console identity v1",
        }
    }

    /// 加密保存一段字节（DPAPI）
    pub fn seal_bytes(data: &[u8], kind: Kind) -> Result<String> {
        Ok(STANDARD.encode(dpapi(data, entropy(kind), true)?))
    }

    pub fn open_bytes(text: &str, kind: Kind) -> Result<Vec<u8>> {
        let raw = STANDARD.decode(text.trim())?;
        let raw = dpapi(&raw, entropy(kind), false)?;
        Ok(raw)
    }

    pub fn seal(secret: &[u8; 16]) -> Result<String> {
        Ok(STANDARD.encode(dpapi(secret, ENTROPY, true)?))
    }

    pub fn open(text: &str) -> Result<[u8; 16]> {
        let raw = STANDARD.decode(text.trim())?;
        let raw = dpapi(&raw, ENTROPY, false)?;
        match <[u8; 16]>::try_from(raw.as_slice()) {
            Ok(s) => Ok(s),
            Err(_) => bail!("the secret has the wrong length"),
        }
    }

    /// "我的设备"里的控制码写进文件之前（DPAPI 加密）
    pub fn seal_device_code(code: &str) -> Result<String> {
        Ok(STANDARD.encode(dpapi(code.as_bytes(), DEVICE_ENTROPY, true)?))
    }

    /// 解开加密过的控制码（6 位数字的不用解，调用方直接用）
    pub fn open_device_code(text: &str) -> Result<String> {
        {
            let raw = dpapi(&STANDARD.decode(text.trim())?, DEVICE_ENTROPY, false)?;
            Ok(String::from_utf8(raw)?)
        }
    }

    fn dpapi(data: &[u8], entropy: &[u8], protect: bool) -> Result<Vec<u8>> {
        use anyhow::Context;
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        };

        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let entropy = CRYPT_INTEGER_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    windows::core::w!("Nife Remote"),
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
                .context("CryptProtectData")?;
            } else {
                CryptUnprotectData(
                    &input,
                    None,
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
                .context("CryptUnprotectData (was config.toml copied from another computer?)")?;
            }
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(Some(HLOCAL(out.pbData.cast())));
            Ok(bytes)
        }
    }
}
