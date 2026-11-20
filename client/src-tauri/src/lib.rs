//! Nife Remote 客户端：同一个二进制既是被控端也是控制端。
//!
//! * 启动后默认是**被控端**：只有托盘图标，没有任何窗口；
//! * 托盘菜单可切换为**控制端**：打开现代化控制台，可同时控制多台电脑；
//! * Windows 需要安装：被控端作为系统服务运行（见 `winsvc`），托盘只负责显示和操作；
//!   调试构建直接运行时被控端在本进程里运行。macOS / Linux 的被控端一直在本进程里运行；
//! * macOS 的登录时自动启动是系统的登录项（见 [`login_item`]），第一次从"应用程序"文件夹运行时默认打开；
//! * 配置和数据只有一个 `config.toml`（见 [`config`]）：服务器设置、本机的控制码、控制台的"我的设备"
//!   （见 [`devices`]）。Windows 在安装目录，macOS 在 `~/Library/Application Support/app.nife-remote.client/`，
//!   Linux 在 `~/.config/nife-remote/`；被删掉了会自动重新生成。其余运行时产生的文件退出时删除（见 [`ephemeral`]）；
//! * 界面支持中文 / 英文，默认跟随系统语言（见 [`i18n`]）。

// 最先声明：其它模块都要用到 `tr!`
#[macro_use]
mod i18n;
mod accounts;
mod clipboard;
mod config;
mod console;
mod devices;
#[cfg(test)]
mod e2e;
mod ephemeral;
mod fullscreen;
pub mod host;
#[cfg(target_os = "macos")]
mod login_item;
mod profile;
mod relay;
mod single_instance;
pub mod status;
mod sysinfo;
mod tray;
#[cfg(target_os = "macos")]
mod uninstall;
#[cfg(windows)]
mod winsvc;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use nife_remote_link::secure::Identity;
use nife_remote_link::{Problem, ServerState};
use serde::Serialize;
use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tracing::{info, warn};

use host::{HostCmd, HostService};
use i18n::Lang;
use status::{HostStatus, StatusHub};

const CONSOLE: &str = "main";

pub struct AppState {
    hub: StatusHub,
    services: Mutex<Services>,
    webview_dirs: Mutex<Vec<PathBuf>>,
    /// Windows 安装版：被控端在系统服务里运行（托盘只负责显示和操作），切换模式不影响它
    service_mode: bool,
    /// 控制台连过来的 WebSocket 地址（第一次要的时候开始监听，见 [`console`]）
    console: tokio::sync::OnceCell<String>,
}

#[derive(Default)]
struct Services {
    host: Option<Hosting>,
    /// 控制端模式：控制台的身份（端到端加密）。控制端服务每个控制台连接一个，在 [`console`] 里
    controller: Option<Identity>,
}

/// 被控端在哪里运行
enum Hosting {
    /// 本进程里
    InProcess(HostService),
    /// Windows 安装版：系统服务里，托盘经命名管道和它通信
    #[cfg(windows)]
    Service(winsvc::ipc::AgentLink),
}

/// 托盘菜单里对配对的操作
#[derive(Debug, Clone)]
enum PairingCmd {
    /// 换一个配对码（暂停了的配对随之恢复）
    NewCode,
    /// 换成本机用户填的配对码
    Set(String),
    /// 忘掉所有配对过的控制端
    Forget,
}

impl Hosting {
    fn send(&self, cmd: HostCmd) {
        match self {
            Hosting::InProcess(h) => h.send(cmd),
            #[cfg(windows)]
            Hosting::Service(link) => link.send(match cmd {
                HostCmd::KickAll => winsvc::ipc::ToAgent::KickAll,
                HostCmd::Kick { session_id } => winsvc::ipc::ToAgent::Kick { session_id },
                HostCmd::NewCode => winsvc::ipc::ToAgent::NewCode,
            }),
        }
    }

    /// 界面上改了"验证方式 / 允许他人远程协助"：本进程里的被控端直接改，服务里的经管道通知
    fn access(&self, verify: profile::Verify, allow: bool) {
        match self {
            Hosting::InProcess(h) => {
                let gate = h.gate().clone();
                std::thread::spawn(move || {
                    gate.set_access(crate::host::gate::access_from_verify(verify));
                    gate.set_allowed(allow);
                });
            }
            #[cfg(windows)]
            Hosting::Service(link) => link.send(winsvc::ipc::ToAgent::SetAccess {
                verify: verify.as_str().to_string(),
                allow,
            }),
        }
    }

    fn pairing(&self, cmd: PairingCmd) {
        match self {
            Hosting::InProcess(h) => {
                let gate = h.gate().clone();
                // 要写配置文件，不在主线程上做
                std::thread::spawn(move || match cmd {
                    PairingCmd::NewCode => gate.new_pairing_code(),
                    PairingCmd::Set(code) => gate.set_pairing_code(&code),
                    PairingCmd::Forget => gate.forget_paired(),
                });
            }
            #[cfg(windows)]
            Hosting::Service(link) => link.send(match cmd {
                PairingCmd::NewCode => winsvc::ipc::ToAgent::NewPairingCode,
                PairingCmd::Set(code) => winsvc::ipc::ToAgent::SetPairingCode { code },
                PairingCmd::Forget => winsvc::ipc::ToAgent::ForgetPaired,
            }),
        }
    }
}

// ───────────────────────────── 后台服务 ─────────────────────────────

/// 启动被控端和控制端（不涉及窗口）。两者同时在跑：被控端让别人连本机，控制端让控制台去连别人。
/// 被控端在系统服务里运行时（Windows 安装版）只准备控制台的身份。
fn start_services(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut sv = state.services.lock().unwrap();
    if sv.host.is_none() {
        hub_reset_host(state.hub.clone(), state.service_mode);
        sv.host = Some(if state.service_mode {
            start_service_link(app, &state.hub)
        } else {
            // 被控端在本进程里：启动就开，关窗口不影响
            Hosting::InProcess(HostService::start(state.hub.clone()))
        });
        #[cfg(target_os = "macos")]
        host::macos::request_permissions();
    }
    // 控制台的身份（端到端加密）：第一次用时生成，记进 config.toml。
    // 控制端服务在控制台连上来时启动（见 console）
    if sv.controller.is_none() {
        let (identity, new_key) = config::controller_identity();
        if let Some(key) = new_key {
            save_controller_key(&sv, key);
        }
        sv.controller = Some(identity);
    }
}

/// 被控端另开头时（Windows 安装版：服务里的那个不归我们管）把它的状态清干净
fn hub_reset_host(hub: StatusHub, service_mode: bool) {
    hub.send_modify(|s| {
        s.host = HostStatus::default();
        s.controller = ServerState::Connecting;
        if !service_mode {
            s.service = None;
        }
    });
}

/// 记下控制台新生成的身份。Windows 安装版：配置文件由服务里的被控端维护，经命名管道交给它写
#[cfg_attr(not(windows), allow(unused_variables))]
fn save_controller_key(sv: &Services, key: String) {
    #[cfg(windows)]
    if let Some(Hosting::Service(link)) = &sv.host {
        link.send(winsvc::ipc::ToAgent::SaveControllerKey { key });
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = config::save_controller_key(&key) {
            warn!("can't save the console's identity: {e:#}");
        }
    });
}

/// macOS：是否在程序坞里显示图标（也决定能不能用 Command+Tab 切换到控制台）。
/// 只有托盘图标时是"辅助程序"，打开控制台时变成普通程序。
#[cfg(target_os = "macos")]
fn show_in_dock(app: &AppHandle, visible: bool) {
    let policy = if visible {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    let _ = app.set_activation_policy(policy);
}

fn open_console(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    show_in_dock(app, true);
    if let Some(w) = app.get_webview_window(CONSOLE) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let app = app.clone();
    // Windows 上在主线程同步创建 WebView2 可能死锁，放到异步任务中创建
    tauri::async_runtime::spawn(async move {
        let builder =
            WebviewWindowBuilder::new(&app, CONSOLE, WebviewUrl::App("index.html".into()));
        // WebView2 / WebKitGTK 需要一个数据目录（放在临时目录里，用完删除，见 ephemeral）；
        // macOS 的 WKWebView 没有这个概念，隐私模式下不保存网页数据（WebKit 自己在系统临时目录里建的目录见 ephemeral）
        #[cfg(not(target_os = "macos"))]
        let builder = {
            let dir = ephemeral::new_dir();
            app.state::<AppState>()
                .webview_dirs
                .lock()
                .unwrap()
                .push(dir.clone());
            builder.data_directory(dir)
        };
        let builder = builder
            .title(console_title())
            // 首屏就用当前语言（之后的切换通过 lang-changed 事件通知）
            .initialization_script(format!(
                "window.__NIFE_REMOTE_LANG__ = {:?};",
                i18n::current().code()
            ))
            .inner_size(1360.0, 860.0)
            .min_inner_size(820.0, 560.0)
            .center()
            .incognito(true)
            .disable_drag_drop_handler()
            .background_color(tauri::window::Color(9, 11, 16, 255))
            .theme(Some(tauri::Theme::Dark));
        // 无边框：标题栏就是控制台顶部的标签栏（拖动、双击最大化、窗口按钮都由界面处理）。
        // macOS 保留系统的红绿灯按钮，叠在标签栏左侧（垂直居中于 40px 高的标签栏）
        #[cfg(target_os = "macos")]
        let builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(16.0, 22.0));
        #[cfg(not(target_os = "macos"))]
        let builder = builder.decorations(false);
        let built = builder.build();
        match built {
            Ok(w) => {
                let _ = w.set_focus();
            }
            Err(e) => warn!("can't open the console: {e}"),
        }
    });
}

fn console_title() -> String {
    tr!("Nife Remote 控制台", "Nife Remote Console")
}

/// 切换界面语言（托盘菜单或控制台）。
fn set_lang(app: &AppHandle, lang: Lang) {
    if i18n::set(lang) {
        info!(?lang, "switching UI language");
    }
    // 总是通知状态循环刷新托盘菜单：勾选项被点击时系统会自动翻转勾选状态，需要重建菜单恢复
    app.state::<AppState>().hub.send_modify(|_| {});
}

fn on_console_destroyed(app: &AppHandle) {
    // 关掉控制台只断开自己的会话（控制端），被控端照常跑，别人随时可以连本机
    #[cfg(target_os = "macos")]
    {
        // 全屏中被关掉时菜单栏和程序坞还藏着
        fullscreen::reset(app);
        show_in_dock(app, false);
    }
    let dirs: Vec<PathBuf> =
        std::mem::take(&mut *app.state::<AppState>().webview_dirs.lock().unwrap());
    for d in dirs {
        ephemeral::remove_in_background(d);
    }
}

pub(crate) fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "console" => open_console(app),
        "lang_zh" => set_lang(app, Lang::Zh),
        "lang_en" => set_lang(app, Lang::En),
        "kick_all" | "new_code" => {
            let cmd = if id == "kick_all" {
                HostCmd::KickAll
            } else {
                HostCmd::NewCode
            };
            if let Some(h) = &app.state::<AppState>().services.lock().unwrap().host {
                h.send(cmd);
            }
        }
        "new_pairing" | "forget_paired" => {
            let cmd = if id == "new_pairing" {
                PairingCmd::NewCode
            } else {
                PairingCmd::Forget
            };
            if let Some(h) = &app.state::<AppState>().services.lock().unwrap().host {
                h.pairing(cmd);
            }
        }
        #[cfg(windows)]
        "autostart" => {
            let state = app.state::<AppState>();
            let current = state.hub.borrow().service.map(|s| s.autostart);
            if let (Some(current), Some(Hosting::Service(link))) =
                (current, &state.services.lock().unwrap().host)
            {
                link.send(winsvc::ipc::ToAgent::SetAutostart { on: !current });
            }
            // 勾选项被点击时系统会自动翻转勾选状态：重建菜单，以被控端确认后的设置为准
            state.hub.send_modify(|_| {});
        }
        #[cfg(windows)]
        "start_service" => {
            std::thread::spawn(|| {
                if let Err(e) = winsvc::start() {
                    warn!("can't start the Nife Remote service: {e:#}");
                }
            });
        }
        #[cfg(target_os = "macos")]
        "autostart" => {
            let hub = app.state::<AppState>().hub.clone();
            // 等用户允许的也算"打开"：再点一下是去系统设置里允许（菜单里就是这么写的）
            let on = hub.borrow().login_item != Some(login_item::State::On);
            // 会经 XPC 问系统，不在主线程上等
            std::thread::spawn(move || match login_item::set(on, true) {
                Ok(s) => {
                    info!(state = ?s, "open at login changed");
                    record_login_item(s);
                    hub.send_modify(|st| st.login_item = Some(s));
                }
                Err(e) => {
                    warn!("{e}");
                    hub.send_modify(|_| {});
                }
            });
            // 勾选项被点击时系统会自动翻转勾选状态：先按原来的状态重建菜单，切换完了再刷新
            app.state::<AppState>().hub.send_modify(|_| {});
        }
        #[cfg(target_os = "macos")]
        "uninstall" => {
            // 对话框是模态的：等这次菜单事件处理完，再在主线程上弹出来
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || uninstall_app(&handle));
        }
        #[cfg(target_os = "macos")]
        "perm_screen" => {
            host::macos::open_privacy_settings(host::macos::PrivacyPane::ScreenRecording)
        }
        #[cfg(target_os = "macos")]
        "perm_input" => host::macos::open_privacy_settings(host::macos::PrivacyPane::Accessibility),
        "copy_code" => {
            if let Some(code) = app.state::<AppState>().hub.borrow().host.code {
                match clipboard::set_text(&format!("{code:06}")) {
                    Ok(()) => info!("control code copied"),
                    Err(e) => warn!("copying failed: {e}"),
                }
            }
        }
        "copy_pairing" => {
            let pairing = app.state::<AppState>().hub.borrow().host.pairing.clone();
            if let Some(p) = pairing {
                match clipboard::set_text(&p) {
                    Ok(()) => info!("pairing code copied"),
                    Err(e) => warn!("copying failed: {e}"),
                }
            }
        }
        "quit" => {
            let state = app.state::<AppState>();
            // Windows 安装版：退出 = 被控端也停止（和直接运行时一样，退出后本机不能再被控制）
            #[cfg(windows)]
            if state.service_mode
                && let Err(e) = winsvc::stop()
            {
                warn!("can't stop the Nife Remote service: {e:#}");
            }
            let mut sv = state.services.lock().unwrap();
            sv.host = None;
            sv.controller = None;
            drop(sv);
            app.exit(0);
        }
        _ => {}
    }
}

pub(crate) fn on_tray_left_click(app: &AppHandle) {
    open_console(app);
}

// ───────────────────────────── 前端命令 ─────────────────────────────

/// 控制台的 `appInfo`（经 WebSocket，见 [`console`]）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    /// 账号服务地址（用户唯一要填的；中转服务器由它分配）
    auth: String,
    server: String,
    /// 配置过中转服务器没有（没有的话 `server` 只是占位的 127.0.0.1:4433，不连接也不重试）
    configured: bool,
    pinned: bool,
    hostname: String,
    lang: Lang,
    /// 配置文件（也保存着我的设备）的路径，控制台的菜单里显示
    config_path: Option<String>,
}

fn app_info() -> AppInfo {
    AppInfo {
        version: config::VERSION,
        auth: config::settings().auth.clone(),
        server: config::settings().server.clone(),
        configured: config::server_configured(),
        pinned: config::pinned(),
        hostname: sysinfo::host_identity().0,
        lang: i18n::current(),
        config_path: config::path().map(|p| p.display().to_string()),
    }
}

/// 我的设备（配置文件里本账户的）
#[tauri::command]
async fn devices_load() -> Vec<devices::Device> {
    tauri::async_runtime::spawn_blocking(devices::load)
        .await
        .unwrap_or_default()
}

/// 保存我的设备（控制台传来整个列表）
#[tauri::command]
#[cfg_attr(not(windows), allow(unused_variables))]
async fn devices_save(
    state: tauri::State<'_, AppState>,
    devices: Vec<devices::Device>,
) -> Result<(), Problem> {
    let list = tauri::async_runtime::spawn_blocking(move || devices::for_file(devices))
        .await
        .map_err(Problem::failed)?
        .map_err(|e| Problem::failed(format!("{e:#}")))?;
    // Windows 安装版：配置文件在安装目录里，由服务里的被控端维护（托盘没有写权限），经命名管道交给它写
    #[cfg(windows)]
    if state.service_mode {
        let connected = state.hub.borrow().service.is_some_and(|s| s.connected);
        let sv = state.services.lock().unwrap();
        return match &sv.host {
            Some(Hosting::Service(link)) if connected => {
                link.send(winsvc::ipc::ToAgent::SaveDevices { devices: list });
                Ok(())
            }
            _ => Err(Problem::failed(
                "the Nife Remote service isn't reachable from this session",
            )),
        };
    }
    tauri::async_runtime::spawn_blocking(move || config::save_devices(&list))
        .await
        .map_err(Problem::failed)?
        .map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 保存控制台里填的中转服务器地址和证书指纹（`fp` 为空 = 首次连接时信任）。
/// 改完要重启程序才生效（见 [`config::settings`]）。
#[tauri::command]
#[cfg_attr(not(windows), allow(unused_variables))]
async fn server_save(
    state: tauri::State<'_, AppState>,
    server: String,
    fp: String,
) -> Result<(), Problem> {
    // 先校验（服务在另一个进程里，错误拿不回来）
    config::check_server(&server, &fp).map_err(|e| Problem::failed(format!("{e:#}")))?;
    // Windows 安装版：配置文件在安装目录里，由服务里的被控端维护（托盘没有写权限），经命名管道交给它写
    #[cfg(windows)]
    if state.service_mode {
        let connected = state.hub.borrow().service.is_some_and(|s| s.connected);
        // 锁不能跨 await（future 要 Send），发完就撒手
        {
            let sv = state.services.lock().unwrap();
            match &sv.host {
                Some(Hosting::Service(link)) if connected => {
                    link.send(winsvc::ipc::ToAgent::SetServer { server, fp });
                }
                _ => {
                    return Err(Problem::failed(
                        "the Nife Remote service isn't reachable from this session",
                    ));
                }
            }
        }
        // 服务那边是异步写的（没有回执），马上重启会把还没写的丢掉：等它写进 config.toml 再返回
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || config::save_server(&server, &fp))
        .await
        .map_err(Problem::failed)?
        .map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 断开某一路控制端（被控中页列表里那一行）
#[tauri::command]
fn kick_session(state: tauri::State<'_, AppState>, session_id: u64) -> Result<(), Problem> {
    let sv = state.services.lock().unwrap();
    let Some(host) = &sv.host else {
        return Err(Problem::failed("the host isn't running"));
    };
    host.send(HostCmd::Kick { session_id });
    Ok(())
}

/// 现在登录的账号（本机记着的登录状态；没登录返回 null）
#[tauri::command]
fn account_get() -> Option<accounts::AccountInfo> {
    accounts::current()
}

/// 打开界面时刷新一下账号信息（凭证过期就变成没登录）
#[tauri::command]
async fn account_refresh() -> Option<accounts::AccountInfo> {
    accounts::refresh().await
}

/// 注册一个账号（注册完直接登录）
#[tauri::command]
async fn account_register(name: String, password: String) -> Result<accounts::AccountInfo, Problem> {
    accounts::register(&name, &password).await.map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 登录
#[tauri::command]
async fn account_login(name: String, password: String) -> Result<accounts::AccountInfo, Problem> {
    accounts::login(&name, &password).await.map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 退出登录
#[tauri::command]
async fn account_logout() -> Result<(), Problem> {
    accounts::logout().await.map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 改密码（旧密码、新密码）
#[tauri::command]
async fn account_change_password(old_password: String, new_password: String) -> Result<accounts::AccountInfo, Problem> {
    accounts::change_password(&old_password, &new_password)
        .await
        .map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 账号下的设备列表（"本账号设备"）
#[tauri::command]
async fn account_devices() -> Result<Vec<String>, Problem> {
    accounts::devices().await.map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 把设备列表存进账号
#[tauri::command]
async fn account_save_devices(codes: Vec<String>) -> Result<(), Problem> {
    accounts::save_devices(&codes).await.map_err(|e| Problem::failed(format!("{e:#}")))
}

/// 账号服务的地址（界面里显示 / 排查用）
#[tauri::command]
fn account_server() -> String {
    accounts::base_url()
}

/// 界面自己的偏好和记录（profile.json）：允许协助、验证方式、收藏、最近连接
#[tauri::command]
fn profile_get() -> profile::Profile {
    profile::get()
}

/// 允许他人远程协助：关掉之后正在控的全部断开，也不再接受新的
#[tauri::command]
fn profile_set_allow(state: tauri::State<'_, AppState>, allow: bool) -> Result<profile::Profile, Problem> {
    let (p, ()) = profile::update(|p| {
        let changed = p.allow_assist != allow;
        p.allow_assist = allow;
        (changed, ())
    })
    .map_err(|e| Problem::failed(format!("{e:#}")))?;
    if let Some(host) = &state.services.lock().unwrap().host {
        host.access(p.verify, p.allow_assist);
        if !allow {
            host.send(HostCmd::KickAll);
        }
    }
    Ok(p)
}

/// 验证方式：pairing（配对码）/ approval（本机同意）/ open（不用验证）
#[tauri::command]
fn profile_set_verify(state: tauri::State<'_, AppState>, verify: String) -> Result<profile::Profile, Problem> {
    let verify = profile::Verify::parse(&verify);
    let (p, ()) = profile::update(|p| {
        let changed = p.verify != verify;
        p.verify = verify;
        (changed, ())
    })
    .map_err(|e| Problem::failed(format!("{e:#}")))?;
    if let Some(host) = &state.services.lock().unwrap().host {
        host.access(p.verify, p.allow_assist);
    }
    Ok(p)
}

/// 收藏 / 取消收藏一台设备（按设备ID）
#[tauri::command]
fn favorite_toggle(code: String) -> Result<profile::Profile, Problem> {
    let (p, _) = profile::update(|p| (true, p.toggle_favorite(&code)))
        .map_err(|e| Problem::failed(format!("{e:#}")))?;
    Ok(p)
}

/// 记一条最近连接（连上之后由界面调用）
#[tauri::command]
fn recent_push(code: String, name: String) -> Result<profile::Profile, Problem> {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (p, ()) = profile::update(|p| {
        let before = p.recent.clone();
        p.push_recent(&code, &name, at);
        (p.recent != before, ())
    })
    .map_err(|e| Problem::failed(format!("{e:#}")))?;
    Ok(p)
}

/// 最近连接里删掉一条 / 全部清掉
#[tauri::command]
fn recent_drop(code: Option<String>) -> Result<profile::Profile, Problem> {
    let (p, ()) = profile::update(|p| {
        let before = p.recent.clone();
        match &code {
            Some(c) => p.drop_recent(c),
            None => p.clear_recent(),
        }
        (p.recent != before, ())
    })
    .map_err(|e| Problem::failed(format!("{e:#}")))?;
    Ok(p)
}

/// 账号服务地址（界面里唯一要填的地址；中转服务器由它分配）
#[tauri::command]
fn auth_get() -> String {
    config::settings().auth.clone()
}

/// 保存账号服务地址：马上问一次中转服务器，写进 config.toml，然后重启让连接用上新的
#[tauri::command]
async fn auth_save(auth: String) -> Result<(), Problem> {
    let auth = config::check_auth(&auth).map_err(|e| Problem::failed(format!("{e:#}")))?;
    // 先确认这个地址能用（拿得到中转服务器），再落盘：填错了能当场看到错误
    let relay = relay::fetch(&auth).await.map_err(|e| Problem::failed(format!("{e:#}")))?;
    let address = relay.address();
    config::check_server(&address, &relay.fingerprint).map_err(|e| Problem::failed(format!("{e:#}")))?;
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        config::save_auth(&auth)?;
        config::save_server(&address, &relay.fingerprint)?;
        Ok(())
    })
    .await
    .map_err(Problem::failed)?
    .map_err(|e| Problem::failed(format!("{e:#}")))?;
    Ok(())
}

/// 断开所有正在控制本机的控制端（左栏被控卡点开放大后那一页的按钮）
#[tauri::command]
fn kick_all(state: tauri::State<'_, AppState>) -> Result<(), Problem> {
    let sv = state.services.lock().unwrap();
    let Some(host) = &sv.host else {
        return Err(Problem::failed("the host isn't running"));
    };
    host.send(HostCmd::KickAll);
    Ok(())
}

/// 首页里填的配对码（6 位数字，本机生成的，可以自己改）：写进 `config.toml` 并马上生效。
/// Windows 安装版：配置文件在安装目录里，由服务里的被控端维护（托盘没有写权限），经命名管道交给它
#[tauri::command]
async fn pairing_save(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<(), Problem> {
    config::check_pairing(&code).map_err(|e| Problem::failed(format!("{e:#}")))?;
    let code = code.trim().to_string();
    {
        let sv = state.services.lock().unwrap();
        let Some(host) = &sv.host else {
            return Err(Problem::failed("the host isn't running"));
        };
        host.pairing(PairingCmd::Set(code));
    }
    Ok(())
}

/// 重启程序：服务器地址和指纹在启动时读一次（见 [`config::settings`]），改完靠这个生效
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// 控制台切换界面语言
#[tauri::command]
fn set_language(app: AppHandle, lang: Lang) {
    set_lang(&app, lang);
}

/// 控制台和控制端服务之间的 WebSocket 地址：会话、画面、声音都走它，不走 Tauri 的 IPC（见 [`console`]）
#[tauri::command]
async fn console_url(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<String, Problem> {
    state
        .console
        .get_or_try_init(|| console::listen(app.clone()))
        .await
        .cloned()
        .map_err(Problem::failed)
}

/// 读取本机剪贴板文本（用于"粘贴到远端"）
#[tauri::command]
fn clipboard_text() -> Result<String, Problem> {
    clipboard::get_text().map_err(|_| Problem::NoClipboardText)
}

/// 控制台窗口进入 / 退出全屏（各系统要额外处理的地方见 [`fullscreen`]）
#[tauri::command]
fn set_fullscreen(window: tauri::WebviewWindow, on: bool) -> Result<(), Problem> {
    fullscreen::set(&window, on).map_err(Problem::failed)
}

/// 控制台窗口是不是全屏（包括 macOS 上自己铺满屏幕的全屏，见 [`fullscreen`]）
#[tauri::command]
fn window_fullscreen(window: tauri::WebviewWindow) -> Result<bool, Problem> {
    fullscreen::is_on(&window).map_err(Problem::failed)
}

/// 本机连接码等被控端状态（控制台首页显示自己的连接码，见 `host-status` 事件）
#[tauri::command]
fn host_status(state: tauri::State<'_, AppState>) -> HostStatus {
    state.hub.borrow().host.clone()
}

// ───────────────────────────── 入口 ─────────────────────────────

pub fn run() {
    // Windows：安装包和服务管理器用的命令行（--install、--service、--agent……）
    #[cfg(windows)]
    if let Some(code) = winsvc::cli() {
        std::process::exit(code);
    }
    init_logging();
    let Some(_instance) = single_instance::acquire() else {
        // 已经有一个实例在运行（托盘里），直接退出
        return;
    };
    // Windows：安装好的 Nife Remote 由系统服务运行被控端；发布版必须安装，调试构建直接运行时被控端在本进程里运行
    #[cfg(windows)]
    let service_mode = winsvc::installed_here();
    #[cfg(all(windows, not(debug_assertions)))]
    if !service_mode {
        winsvc::show_not_installed();
        return;
    }
    #[cfg(not(windows))]
    let service_mode = false;
    // 配置文件由本进程维护（Windows 安装版由服务里的被控端维护，见 winsvc::agent）：
    // 不存在时按默认配置生成，运行中被删掉了也会重新生成
    if !service_mode {
        if let Err(e) = config::ensure_file() {
            warn!("can't create the configuration file: {e:#}");
        }
        config::keep_file();
    }
    // 配了账号服务的话：启动时先问它"中转服务器在哪"，写进 config.toml 再启动各种服务。
    // 拉不到就保留上一次的值（离线也能连已经配好的服务器）。
    let auth = config::settings().auth.clone();
    if !auth.is_empty() && !service_mode {
        let _ = tauri::async_runtime::block_on(async { relay::resolve(&auth).await });
    }
    enable_dpi_awareness();
    // 先清理上次残留的目录，再创建本次的（同一个前缀）
    ephemeral::sweep_stale();
    #[cfg(target_os = "linux")]
    ephemeral::isolate_caches();

    let hub = status::new_hub();
    let app = tauri::Builder::default()
        .manage(AppState {
            hub: hub.clone(),
            services: Mutex::default(),
            webview_dirs: Mutex::default(),
            service_mode,
            console: Default::default(),
        })
        // 控制台的麦克风（把本机的麦克风发给对方，见 client/src/lib/mic.ts）：用户在连接里打开了才会用到，
        // 直接允许，不再弹 WebView 自己的询问（控制台的数据目录是临时的，不然每次打开控制台都要问一遍）
        .on_permission_request(|_, kind| match kind {
            PermissionKind::Microphone => PermissionResponse::Allow,
            _ => PermissionResponse::Default,
        })
        .invoke_handler(tauri::generate_handler![
            console_url,
            set_language,
            clipboard_text,
            set_fullscreen,
            window_fullscreen,
            host_status,
            devices_load,
            devices_save,
            server_save,
            pairing_save,
            auth_get,
            auth_save,
            kick_all,
            kick_session,
            account_get,
            account_refresh,
            account_server,
            account_change_password,
            account_devices,
            account_save_devices,
            account_register,
            account_login,
            account_logout,
            profile_get,
            profile_set_allow,
            profile_set_verify,
            favorite_toggle,
            recent_push,
            recent_drop,
            restart_app,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            if tray::available() {
                tray::create(&handle)?;
            }
            #[cfg(target_os = "linux")]
            exit_on_signals(handle.clone());
            // 两个角色同时开：被控端（本机可被连）+ 控制台的控制端身份（去连别人）
            start_services(&handle);
            #[cfg(target_os = "macos")]
            {
                watch_permissions(hub.clone());
                login_item_startup(hub.clone());
            }
            // 启动就打开控制台（首页显示本机连接码）；关掉后只剩托盘图标
            open_console(&handle);

            // 状态（或界面语言）变化 → 刷新托盘 / 通知控制台
            let mut rx = hub.subscribe();
            tauri::async_runtime::spawn(async move {
                let mut lang = None;
                loop {
                    let st = rx.borrow_and_update().clone();
                    tray::refresh(&handle, &st);
                    // 控制台首页的本机连接码、正在控制本机的控制端
                    let _ = handle.emit_to(CONSOLE, "host-status", &st.host);
                    let cur = i18n::current();
                    if lang.replace(cur) != Some(cur) {
                        if let Some(w) = handle.get_webview_window(CONSOLE) {
                            let _ = w.set_title(&console_title());
                        }
                        let _ = handle.emit_to(CONSOLE, "lang-changed", cur);
                    }
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == CONSOLE && matches!(event, WindowEvent::Destroyed) {
                on_console_destroyed(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("can't initialize Tauri");
    // macOS：启动就打开控制台，所以程序坞里要有图标（注册为普通程序）。关掉窗口后 [`on_console_destroyed`]
    // 再改回“辅助程序”（只剩菜单栏图标）；打包的 .app 在 Info.plist 里声明了 LSUIElement
    #[cfg(target_os = "macos")]
    show_in_dock(&app.handle().clone(), true);

    app.run(|app, event| match event {
        // 托盘程序：关闭所有窗口时不退出
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        RunEvent::Exit => {
            let state = app.state::<AppState>();
            {
                let mut sv = state.services.lock().unwrap();
                sv.host = None;
                sv.controller = None;
            }
            if let Some(w) = app.get_webview_window(CONSOLE) {
                let _ = w.destroy();
            }
            let dirs: Vec<PathBuf> = std::mem::take(&mut *state.webview_dirs.lock().unwrap());
            for d in dirs {
                ephemeral::remove_with_retry(&d, Duration::from_secs(3));
            }
            // Linux：托盘图标、GStreamer 缓存所在的目录（内存盘 XDG_RUNTIME_DIR 里），退出时删掉
            #[cfg(target_os = "linux")]
            ephemeral::remove_with_retry(&ephemeral::process_dir(), Duration::from_secs(1));
        }
        _ => {}
    });
}

/// Windows 安装版：被控端在系统服务里运行。打开 Nife Remote 时服务没有运行就启动它；服务停止后托盘也退出。
#[cfg(windows)]
fn start_service_link(app: &AppHandle, hub: &StatusHub) -> Hosting {
    hub.send_modify(|s| {
        s.service = Some(status::ServiceStatus {
            connected: false,
            running: false,
            elsewhere: false,
            autostart: config::settings().autostart,
        })
    });
    std::thread::spawn(|| {
        if let Err(e) = winsvc::start() {
            warn!("can't start the Nife Remote service: {e:#}");
        }
    });
    let app = app.clone();
    Hosting::Service(winsvc::ipc::AgentLink::start(hub.clone(), move || {
        app.exit(0)
    }))
}

#[cfg(not(windows))]
fn start_service_link(_app: &AppHandle, _hub: &StatusHub) -> Hosting {
    unreachable!("the host only runs as a service on Windows")
}

/// macOS：用户在系统设置里授予 / 收回权限、打开 / 关掉登录时自动启动时刷新托盘菜单
/// （菜单里有未授权的提示、登录时自动启动的勾选）。
#[cfg(target_os = "macos")]
fn watch_permissions(hub: StatusHub) {
    tauri::async_runtime::spawn(async move {
        let mut last = host::macos::permissions();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let now = host::macos::permissions();
            if now != last {
                info!(
                    screen = now.screen,
                    input = now.input,
                    "macOS permissions changed"
                );
                last = now;
                hub.send_modify(|_| {});
            }
            // 问系统要几毫秒到几十毫秒（XPC），不占用异步运行时的线程
            let Ok(item) = tokio::task::spawn_blocking(login_item::state).await else {
                continue;
            };
            let before = hub.borrow().login_item;
            // 还没查过（启动时由 login_item_startup 查）或者没变：不动
            if before.is_none() || item == before {
                continue;
            }
            info!(state = ?item, "open at login changed in System Settings");
            if let Some(s) = item {
                record_login_item(s);
            }
            hub.send_modify(|st| st.login_item = item);
        }
    });
}

/// macOS：启动时查一下登录时自动启动的状态。第一次从"应用程序"文件夹运行时（config.toml 里还没有 `autostart`）
/// 和 Windows 一样默认打开，只做这一次：之后用户在菜单或者系统设置里关掉了就一直关着（见 [`login_item`]）。
#[cfg(target_os = "macos")]
fn login_item_startup(hub: StatusHub) {
    use login_item::State;
    std::thread::spawn(move || {
        let mut state = login_item::state();
        if config::autostart_default_pending() {
            match state {
                Some(State::Off) => match login_item::set(true, false) {
                    Ok(s) => {
                        info!(state = ?s, "open at login: on by default (first run from the Applications folder)");
                        record_login_item(s);
                        state = Some(s);
                    }
                    // 没记下来：下次启动再试
                    Err(e) => warn!("{e}"),
                },
                // 已经登记过（例如删掉重装）：照现在的记下来，不改
                Some(s @ (State::On | State::NeedsApproval)) => record_login_item(s),
                // 不是 App、还不在"应用程序"文件夹里：放进去之后第一次运行时再做
                Some(State::NotInstalled) | None => {}
            }
        }
        hub.send_modify(|st| st.login_item = state);
    });
}

/// macOS：菜单里的"卸载 Nife Remote…"。确认之后停掉被控端和控制端、配置文件不再写入，再按顺序清理（见 [`uninstall`]），
/// 最后告诉用户结果并退出
#[cfg(target_os = "macos")]
fn uninstall_app(app: &AppHandle) {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    if !uninstall::confirm(mtm) {
        return;
    }
    info!("uninstalling");
    let state = app.state::<AppState>();
    {
        // 正在控制本机的连接断开，控制台的会话也断开
        let mut sv = state.services.lock().unwrap();
        sv.host = None;
        sv.controller = None;
    }
    if let Some(w) = app.get_webview_window(CONSOLE) {
        let _ = w.destroy();
    }
    // 还没完全停下的被控端不能再把配置文件写回去，删掉也不再重新生成
    config::freeze();
    // 菜单栏图标要在删偏好设置之前拿掉（见 tray::remove）
    tray::remove(app);
    let plan = uninstall::Plan {
        bundle: login_item::bundle(),
        bundle_id: config::BUNDLE_ID.into(),
        exe_name: std::env::current_exe()
            .ok()
            .and_then(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "nife-remote".into()),
        config_dir: config::path().and_then(|p| Some(p.parent()?.to_path_buf())),
    };
    let problems = uninstall::run(&plan);
    uninstall::done(mtm, &problems);
    app.exit(0);
}

/// macOS：把登录时自动启动的状态记进 config.toml（只是记录，以系统设置为准）
#[cfg(target_os = "macos")]
fn record_login_item(s: login_item::State) {
    let on = match s {
        login_item::State::On => true,
        login_item::State::Off | login_item::State::NeedsApproval => false,
        login_item::State::NotInstalled => return,
    };
    if let Err(e) = config::save_autostart(on) {
        warn!("can't save the open at login setting: {e:#}");
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("NIFE_REMOTE_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,quinn=warn,quinn_proto=warn"));
    // 只输出到 stderr，绝不写日志文件
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Linux：收到 SIGTERM（注销/关机）、SIGINT（终端里按 Ctrl+C）、SIGHUP 时正常退出，照样清理临时目录。
///
/// 信号处理函数里只做一件事：往管道里写一个字节（异步信号安全），由单独的线程读到后通知事件循环退出。
#[cfg(target_os = "linux")]
fn exit_on_signals(app: AppHandle) {
    use std::sync::atomic::{AtomicI32, Ordering};
    static PIPE: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn on_signal(_: libc::c_int) {
        let fd = PIPE.load(Ordering::Relaxed);
        if fd >= 0 {
            unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
        }
    }

    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return;
    }
    PIPE.store(fds[1], Ordering::Relaxed);
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
    std::thread::spawn(move || {
        let mut b = [0u8; 1];
        loop {
            match unsafe { libc::read(fds[0], b.as_mut_ptr().cast(), 1) } {
                1 => break,
                -1 if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {
                    continue;
                }
                _ => return,
            }
        }
        info!("received an exit signal");
        app.exit(0);
    });
}

pub(crate) fn enable_dpi_awareness() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        // 清单中已声明；这里再设置一次以防万一（已设置时会返回错误，忽略即可）
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
