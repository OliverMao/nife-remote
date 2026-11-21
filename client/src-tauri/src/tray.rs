//! 系统托盘：图标 + 右键菜单（本机连接码、服务器状态、正在控制本机的控制端、开机自动启动 /
//! 开机自动启动、语言、退出）。本机连接码在控制台首页也看得到。

use std::sync::OnceLock;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

use crate::i18n::{self, Lang};
use crate::status::{AppStatus, ServiceStatus};
use nife_remote_link::{Problem, ServerState};

const TRAY_ID: &str = "nife-remote";
static ICON_PNG: &[u8] = include_bytes!("../icons/tray.png");

fn icon_online() -> Image<'static> {
    static IMG: OnceLock<Image<'static>> = OnceLock::new();
    IMG.get_or_init(|| Image::from_bytes(ICON_PNG).expect("tray icon").to_owned())
        .clone()
}

/// 未连接时使用灰色图标（运行时由彩色图标去色生成，不需要额外资源文件）。
fn icon_offline() -> Image<'static> {
    static IMG: OnceLock<Image<'static>> = OnceLock::new();
    IMG.get_or_init(|| {
        let src = icon_online();
        let mut rgba = src.rgba().to_vec();
        for px in rgba.as_chunks_mut::<4>().0 {
            let l = (px[0] as u32 * 30 + px[1] as u32 * 59 + px[2] as u32 * 11) / 100;
            let l = (l * 3 / 4 + 40).min(255) as u8;
            px[0] = l;
            px[1] = l;
            px[2] = l;
            px[3] = (px[3] as u32 * 85 / 100) as u8;
        }
        Image::new_owned(rgba, src.width(), src.height())
    })
    .clone()
}

/// 能否显示托盘图标（Windows 上总是可以）。
pub fn available() -> bool {
    true
}

pub fn create(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let st = AppStatus::default();
    let builder = TrayIconBuilder::with_id(TRAY_ID);
    builder
        .icon(icon_offline())
        .tooltip("Nife Remote")
        .menu(&build_menu(app, &st)?)
        // 左键在控制端模式下打开控制台，右键弹出菜单
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| crate::on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::on_tray_left_click(tray.app_handle());
            }
        })
        .build(app)
}


fn label(app: &AppHandle, text: impl AsRef<str>) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::new(app, text, false, None::<&str>)
}

fn server_line(server: &ServerState) -> String {
    match server {
        ServerState::Connecting => tr!("服务器：连接中…", "Server: connecting…"),
        ServerState::Connected => tr!("服务器：已连接 ●", "Server: connected ●"),
        // 只显示原因的种类；详细的技术信息在日志里
        ServerState::Disconnected { reason } => server_problem(reason),
    }
}

/// 托盘菜单：只放"现在什么状态"和常用的几个动作，换控制码、换配对码、语言这些低频的收进「更多」。
/// 控制端的连接状态、正在控制本机的设备明细都在控制台里看。
fn build_menu(app: &AppHandle, st: &AppStatus) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    match st.service {
        Some(svc) => service_items(app, &menu, st, svc)?,
        None => {
            // 被控端一直都在（这台电脑随时能被别人连），下面是它的状态
            host_items(app, &menu, st)?;
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            menu.append(&item(app, "console", tr!("打开控制台", "Open console"))?)?;
        }
    }
    menu.append(&more_menu(app, st)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "quit", tr!("退出", "Quit"))?)?;
    Ok(menu)
}

/// 「更多」里的低频操作：开机自动启动、换控制码 / 配对码、忘掉配对过的控制端、语言。
fn more_menu(app: &AppHandle, st: &AppStatus) -> tauri::Result<Submenu<Wry>> {
    let mut entries: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    if let Some(svc) = st.service {
        entries.push(Box::new(CheckMenuItem::with_id(
            app,
            "autostart",
            tr!("开机自动启动", "Start with Windows"),
            svc.connected,
            svc.autostart,
            None::<&str>,
        )?));
    }
    if matches!(st.host.server, ServerState::Connected) {
        if st.host.code.is_some() {
            entries.push(Box::new(item(
                app,
                "new_code",
                tr!(
                    "更换控制码（旧码失效，配对也作废）",
                    "Change control code (old code and pairings stop working)"
                ),
            )?));
        }
        if st.host.pairing.is_some() {
            entries.push(Box::new(item(
                app,
                "new_pairing",
                tr!("更换配对码", "Change pairing code"),
            )?));
        }
        if st.host.paired > 0 {
            let n = st.host.paired;
            entries.push(Box::new(item(
                app,
                "forget_paired",
                tr!("忘掉已配对的 {n} 个控制端", "Forget {n} paired controllers"),
            )?));
        }
    }
    if !entries.is_empty() {
        entries.push(Box::new(PredefinedMenuItem::separator(app)?));
    }
    entries.push(Box::new(language_menu(app)?));
    let refs: Vec<&dyn IsMenuItem<Wry>> = entries.iter().map(|e| e.as_ref()).collect();
    Submenu::with_items(app, tr!("更多", "More"), true, &refs)
}

/// Windows 安装版：被控端在系统服务里一直运行，控制台随时可以打开
fn service_items(
    app: &AppHandle,
    menu: &Menu<Wry>,
    st: &AppStatus,
    svc: ServiceStatus,
) -> tauri::Result<()> {
    if svc.connected {
        host_items(app, menu, st)?;
    } else if !svc.running {
        menu.append(&label(
            app,
            tr!("被控端服务：未运行", "Host service: not running"),
        )?)?;
        menu.append(&item(
            app,
            "start_service",
            tr!("启动被控端服务", "Start the host service"),
        )?)?;
    } else if svc.elsewhere {
        menu.append(&label(
            app,
            tr!(
                "被控端在屏幕前的另一个用户那里运行",
                "The host runs for the other user at the screen"
            ),
        )?)?;
    } else {
        menu.append(&label(
            app,
            tr!("被控端服务：正在启动…", "Host service: starting…"),
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "console", tr!("打开控制台", "Open console"))?)?;
    Ok(())
}


/// 被控端：服务器状态、控制码 / 配对码（点一下复制）、正在被控制时一行提示（点一下断开）
fn host_items(app: &AppHandle, menu: &Menu<Wry>, st: &AppStatus) -> tauri::Result<()> {
    menu.append(&label(app, server_line(&st.host.server))?)?;
    if let (ServerState::Connected, Some(code)) = (&st.host.server, st.host.code) {
        let code = nife_remote_proto::format_code(code);
        menu.append(&item(
            app,
            "copy_code",
            tr!("控制码 {code}（点击复制）", "Control code {code} (click to copy)"),
        )?)?;
        match &st.host.pairing {
            // 配对码：第一次连接本机时要输，点一下复制
            Some(p) if !st.host.pairing_locked => {
                let p = format!("{} {}", &p[..3], &p[3..]);
                menu.append(&item(
                    app,
                    "copy_pairing",
                    tr!("配对码 {p}（点击复制）", "Pairing code {p} (click to copy)"),
                )?)?;
            }
            // 输错太多次暂停了：点一下换一个
            Some(_) => menu.append(&item(
                app,
                "new_pairing",
                tr!("⚠ 配对已暂停，点击换一个配对码", "⚠ Pairing paused, click for a new code"),
            )?)?,
            None => {}
        }
    }
    if !st.host.controllers.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&item(
            app,
            "kick_all",
            tr!(
                "⚠ {}（点击断开）",
                "⚠ {} (click to disconnect)",
                controlled_by(st.host.controllers.len())
            ),
        )?)?;
    }
    Ok(())
}

fn item(app: &AppHandle, id: &str, text: String) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}

/// 语言子菜单。标题中英并列，看不懂当前语言的人也能找到；选项各用自己的语言书写。
fn language_menu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let cur = i18n::current();
    let title = if cur == Lang::Zh {
        "语言 / Language"
    } else {
        "Language / 语言"
    };
    let zh = CheckMenuItem::with_id(
        app,
        "lang_zh",
        "简体中文",
        true,
        cur == Lang::Zh,
        None::<&str>,
    )?;
    let en = CheckMenuItem::with_id(
        app,
        "lang_en",
        "English",
        true,
        cur == Lang::En,
        None::<&str>,
    )?;
    Submenu::with_items(app, title, true, &[&zh, &en])
}

fn server_problem(p: &Problem) -> String {
    match p {
        Problem::NoServer => tr!(
            "服务器：还没有填写地址（控制台里改）",
            "Server: no address set yet (change it in the console)"
        ),
        Problem::ServerUnreachable { .. } => tr!("服务器：无法连接", "Server: can't connect"),
        Problem::CertMismatch => tr!("服务器：证书指纹不匹配", "Server: certificate mismatch"),
        Problem::Incompatible { server, client } if server > client => {
            tr!(
                "服务器：版本不兼容，请更新 Nife Remote",
                "Server: incompatible, please update Nife Remote"
            )
        }
        Problem::Incompatible { .. } => tr!(
            "服务器：版本不兼容，请更新服务器",
            "Server: incompatible, please update the server"
        ),
        _ => tr!("服务器：连接已断开", "Server: disconnected"),
    }
}

fn controlled_by(n: usize) -> String {
    match n {
        1 => tr!("正在被 1 个控制端控制", "Being controlled by 1 controller"),
        _ => tr!(
            "正在被 {n} 个控制端控制",
            "Being controlled by {n} controllers"
        ),
    }
}

pub fn refresh(app: &AppHandle, st: &AppStatus) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = build_menu(app, st) {
        let _ = tray.set_menu(Some(menu));
    }
    // Windows 安装版：没连上服务里的被控端时，被控端的状态没有意义
    let host_known = st.service.is_none_or(|s| s.connected);
    // 任意一半连上了服务器就亮：能被别人连，或者能去连别人
    let online = (host_known && st.host.server == ServerState::Connected)
        || st.controller == ServerState::Connected;
    let _ = tray.set_icon(Some(if online {
        icon_online()
    } else {
        icon_offline()
    }));
    let code_line = |code: u32| {
        let code = nife_remote_proto::format_code(code);
        tr!("控制码 {code}", "Control code {code}")
    };
    let server = if host_known {
        &st.host.server
    } else {
        &st.controller
    };
    let code = if host_known { st.host.code } else { None };
    let controllers = st.host.controllers.len();
    // 悬停提示先讲本机能不能被连（控制码），再说服务器的情况
    let tip = match (host_known, server, code) {
        (false, ..) => match st.service {
            Some(s) if s.running => tr!("Nife Remote · 被控端", "Nife Remote · Host"),
            _ => tr!(
                "Nife Remote · 被控端服务未运行",
                "Nife Remote · Host service not running"
            ),
        },
        (true, ServerState::Connected, Some(code)) if controllers > 0 => {
            format!(
                "Nife Remote · {}\n{}",
                controlled_by(controllers),
                code_line(code)
            )
        }
        (true, ServerState::Connected, Some(code)) => {
            format!("Nife Remote · {}\n{}", tr!("被控端", "Host"), code_line(code))
        }
        (true, ServerState::Connected, None) => {
            tr!("Nife Remote · 被控端", "Nife Remote · Host")
        }
        (true, ServerState::Connecting, _) => tr!(
            "Nife Remote · 正在连接服务器…",
            "Nife Remote · Connecting to server…"
        ),
        (true, ServerState::Disconnected { .. }, _) => tr!(
            "Nife Remote · 未连接服务器",
            "Nife Remote · Not connected to server"
        ),
    };
    let _ = tray.set_tooltip(Some(tip));
}
