//! 控制台窗口全屏。
//!
//! 全屏时标签栏藏起来，工具栏悬浮在顶端，鼠标碰到屏幕顶端时出现（见界面的 SessionView），
//! 所以全屏期间网页必须能收到屏幕最顶端的鼠标移动。控制台是无边框窗口，Tauri 为了能拖动窗口边缘
//! 改大小，会拦下窗口边缘一圈的鼠标事件，全屏时也照拦不误：窗口上盖了一层透明子窗口
//! （`TAURI_DRAG_RESIZE_WINDOW`），挡住顶端的几个像素。窗口变为"不可调整大小"时 Tauri 会拿掉它，
//! 所以全屏期间关掉可调整大小。
//!
//! 顺序：先全屏再停用边缘，先恢复边缘再退出全屏。退出全屏时如果窗口回到最大化，
//! 要靠退出全屏引起的 WM_SIZE 让 Tauri 收起拖动区，所以要先恢复。

use tauri::WebviewWindow;

/// 窗口是不是全屏
pub fn is_on(window: &WebviewWindow) -> tauri::Result<bool> {
    window.is_fullscreen()
}

pub fn set(window: &WebviewWindow, on: bool) -> tauri::Result<()> {
    if on {
        window.set_fullscreen(true)?;
        pause_edge_resize(window, true)
    } else {
        pause_edge_resize(window, false)?;
        window.set_fullscreen(false)
    }
}

fn pause_edge_resize(window: &WebviewWindow, paused: bool) -> tauri::Result<()> {
    window.set_resizable(!paused)
}
