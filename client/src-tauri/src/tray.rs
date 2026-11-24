//! 系统托盘：图标 + 右键菜单。菜单只有两项 —— 显示主界面、退出；连接码、配对码、服务器状态、
//! 正在控制本机的设备都在主界面里看。左键单击直接打开主界面。

use std::sync::OnceLock;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

use crate::status::AppStatus;
use nife_remote_link::ServerState;

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
    let builder = TrayIconBuilder::with_id(TRAY_ID);
    builder
        .icon(icon_offline())
        .tooltip("Nife Remote")
        .menu(&build_menu(app)?)
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

fn item(app: &AppHandle, id: &str, text: String) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}

/// 托盘菜单：只有两项 —— 打开主界面、退出。状态、控制码、配对码都在主界面里看。
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&item(app, "console", tr!("显示主界面", "Show window"))?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "quit", tr!("退出", "Quit"))?)?;
    Ok(menu)
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
    // 菜单是固定的两项，不用跟着状态重建
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
            format!(
                "Nife Remote · {}\n{}",
                tr!("被控端", "Host"),
                code_line(code)
            )
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
