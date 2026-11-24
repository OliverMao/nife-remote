//! 远程窗口：连上一台电脑之后，远程画面在**它自己的窗口**里，管理界面（控制台）只负责打开 / 切过去。
//!
//! * 一台电脑一个窗口（label = `session-<控制码>`）：再点同一台设备只是把它叫到前面，不会连第二次；
//! * 对方的多个显示器可以从标签**拖出来**单独成一个窗口（label = `session-<控制码>-d<显示器>`）：
//!   画面是一路流，两个窗口同时看就要两条会话，所以那个窗口是另外连的一次；
//! * 窗口跑的是同一个前端（index.html）：要连哪台、开始的画质、配对码、固定看哪个显示器通过初始化脚本
//!   `window.__NIFE_SESSION__` 带过去，会话就在窗口里建立（走 [`crate::console`] 的 WebSocket，
//!   一条连接就是一个控制端服务，关掉窗口这一路就断了）；
//! * 控制台左栏"会话中"要的那份列表在 [`Open`] 里维护：窗口打开 / 关掉 / 里面的会话状态变了都通知控制台。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use nife_remote_link::Problem;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::warn;

use crate::{AppState, CONSOLE, ephemeral, i18n};

/// 窗口 label 的前缀，后面是 6 位控制码
const PREFIX: &str = "session-";
/// 固定看某个显示器的窗口，label 后面再加 `-d<显示器>`
const DISPLAY: &str = "-d";
/// 开着的远程窗口变了（控制台左栏"会话中"听这个）
const CHANGED: &str = "sessions-changed";

/// 控制台左栏"会话中"的一行
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSession {
    pub code: String,
    /// 这个窗口固定看的显示器（被控端的显示器序号）；null = 由被控端决定（跟着它切）
    pub display: Option<u32>,
    /// 对方的设备名；还没连上时是空的
    pub name: String,
    /// 窗口里的会话状态（connecting / approving / live / reconnecting / waiting / closed，
    /// 见 client/src/lib/session.ts）
    pub state: String,
}

/// 开着的远程窗口（label → 它现在的样子）。窗口真的关了才移除
#[derive(Default)]
pub struct Open {
    rows: Mutex<BTreeMap<String, OpenSession>>,
    /// 正在建窗口的 label：连点两下不会建出两个窗口（Tauri 的 label 也不能重复）
    opening: Mutex<BTreeSet<String>>,
}

impl Open {
    pub fn list(&self) -> Vec<OpenSession> {
        self.rows.lock().unwrap().values().cloned().collect()
    }

    /// 登记一行（窗口刚建好时），返回有没有变化
    fn put(&self, label: &str, row: OpenSession) -> bool {
        let mut rows = self.rows.lock().unwrap();
        if rows.get(label) == Some(&row) {
            return false;
        }
        rows.insert(label.to_owned(), row);
        true
    }

    /// 更新已经登记的一行（窗口已经不在了就不加回来），返回有没有变化
    fn update(&self, label: &str, row: OpenSession) -> bool {
        let mut rows = self.rows.lock().unwrap();
        let Some(cur) = rows.get_mut(label) else {
            return false;
        };
        if *cur == row {
            return false;
        }
        *cur = row;
        true
    }

    fn remove(&self, label: &str) -> bool {
        self.rows.lock().unwrap().remove(label).is_some()
    }

    /// 开始建窗口：已经在建（连点了两下）返回 false
    fn begin(&self, label: &str) -> bool {
        self.opening.lock().unwrap().insert(label.to_owned())
    }

    fn end(&self, label: &str) {
        self.opening.lock().unwrap().remove(label);
    }
}

/// 在一台电脑自己的窗口里打开远程画面。已经开着就只是切过去。
///
/// `display`：固定看那个显示器（从显示器标签拖出来的窗口）；null = 由被控端决定。
pub fn open(
    app: &AppHandle,
    code: String,
    settings: serde_json::Value,
    pairing: Option<String>,
    display: Option<u32>,
) -> Result<(), Problem> {
    if !valid_code(&code) {
        return Err(Problem::InvalidCode);
    }
    let label = label(&code, display);
    if let Some(w) = app.get_webview_window(&label) {
        bring_to_front(&w);
        return Ok(());
    }
    // 连点了两下：前一下正在建窗口，等它建好（它会自己切到前面）
    if !app.state::<AppState>().open.begin(&label) {
        return Ok(());
    }
    let result = build(app, &label, &code, display, settings, pairing);
    app.state::<AppState>().open.end(&label);
    result
}

/// 打开一台电脑的**文件传输**窗口（`files-<控制码>`）：左边是本机目录，右边是被控端目录。
/// 一个控制码一个窗口，已经有了就切到前面
pub fn open_files(app: &AppHandle, code: &str, name: &str) -> Result<(), Problem> {
    if !valid_code(code) {
        return Err(Problem::InvalidCode);
    }
    let label = files_label(code);
    if let Some(w) = app.get_webview_window(&label) {
        bring_to_front(&w);
        return Ok(());
    }
    let payload = serde_json::json!({ "code": code, "name": name });
    let title = format!(
        "Nife Remote · {} · {}",
        fmt_code(code),
        tr!("文件传输", "File transfer")
    );
    let dir = ephemeral::new_dir();
    app.state::<AppState>()
        .webview_dirs
        .lock()
        .unwrap()
        .push(dir.clone());
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(title)
        .initialization_script(format!(
            "window.__NIFE_REMOTE_LANG__ = {:?};",
            i18n::current().code()
        ))
        .initialization_script(format!("window.__NIFE_FILES__ = {payload};"))
        .data_directory(dir)
        .inner_size(1120.0, 720.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .incognito(true)
        .disable_drag_drop_handler()
        // 界面是浅色的（见 styles.css），系统标题栏、窗口底色也要浅色，
        // 否则 index.html 里的 color-scheme: dark 会让标题栏变成黑的
        .theme(Some(tauri::Theme::Light))
        .background_color(tauri::window::Color(245, 246, 250, 255))
        // 和参考图一样是有系统边框的普通窗口（文件列表要拖选、滚动，不需要自绘标题栏）
        .build();
    match built {
        Ok(w) => {
            bring_to_front(&w);
            Ok(())
        }
        Err(e) => {
            warn!("can't open the files window: {e}");
            Err(Problem::failed(format!("can't open the window: {e}")))
        }
    }
}

/// 文件传输窗口的 label
fn files_label(code: &str) -> String {
    format!("files-{code}")
}

fn build(
    app: &AppHandle,
    label: &str,
    code: &str,
    display: Option<u32>,
    settings: serde_json::Value,
    pairing: Option<String>,
) -> Result<(), Problem> {
    // 窗口里读它（见 client/src/lib/session-window.ts）
    let payload = serde_json::json!({ "code": code, "display": display, "settings": settings, "pairing": pairing });
    let title = match display {
        Some(i) => format!("Nife Remote · {} · {}", fmt_code(code), i + 1),
        None => format!("Nife Remote · {}", fmt_code(code)),
    };
    // WebView2 需要一个数据目录，放在临时目录里，用完删除（见 ephemeral）
    let dir = ephemeral::new_dir();
    app.state::<AppState>()
        .webview_dirs
        .lock()
        .unwrap()
        .push(dir.clone());
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        // 中文 / 英文：首屏就用当前语言（之后的切换通过 lang-changed 事件同步）
        .initialization_script(format!(
            "window.__NIFE_REMOTE_LANG__ = {:?};",
            i18n::current().code()
        ))
        .initialization_script(format!("window.__NIFE_SESSION__ = {payload};"))
        .data_directory(dir)
        .inner_size(1280.0, 800.0)
        .min_inner_size(640.0, 400.0)
        .center()
        .incognito(true)
        .disable_drag_drop_handler()
        .background_color(tauri::window::Color(9, 11, 16, 255))
        .theme(Some(tauri::Theme::Dark))
        // 无边框：顶部那条工具栏就是窗口的标题栏（图标、对方名称、显示器标签、窗口按钮都在它上面，
        // 见 components/SessionView.tsx）
        .decorations(false)
        .build();
    let w = match built {
        Ok(w) => w,
        Err(e) => {
            warn!("can't open the session window: {e}");
            return Err(Problem::failed(format!("can't open the window: {e}")));
        }
    };
    // 登记到"会话中"：窗口里的会话连上之后再报名字和状态（见 report）
    let changed = app.state::<AppState>().open.put(
        label,
        OpenSession {
            code: code.to_owned(),
            display,
            name: String::new(),
            state: "connecting".to_owned(),
        },
    );
    if changed {
        notify(app);
    }
    bring_to_front(&w);
    Ok(())
}

/// 窗口里的会话变样了（连上、等对方同意、断了……）：更新左栏那一行
pub fn report(app: &AppHandle, code: &str, display: Option<u32>, name: String, state: String) {
    if !valid_code(code) {
        return;
    }
    let label = label(code, display);
    let changed = app.state::<AppState>().open.update(
        &label,
        OpenSession {
            code: code.to_owned(),
            display,
            name,
            state,
        },
    );
    if changed {
        notify(app);
    }
}

/// 远程窗口关掉了（会话跟着断开）：左栏那一行也撤掉
pub fn forget(app: &AppHandle, label: &str) {
    if app.state::<AppState>().open.remove(label) {
        notify(app);
    }
}

/// 切到已经开着的远程窗口
pub fn focus(app: &AppHandle, code: &str, display: Option<u32>) {
    if !valid_code(code) {
        return;
    }
    if let Some(w) = app.get_webview_window(&label(code, display)) {
        bring_to_front(&w);
    }
}

pub fn list(app: &AppHandle) -> Vec<OpenSession> {
    app.state::<AppState>().open.list()
}

/// 窗口 label 是远程窗口的话，取出它的控制码和固定看的显示器
pub fn parse_label(label: &str) -> Option<(&str, Option<u32>)> {
    let rest = label.strip_prefix(PREFIX)?;
    let (code, display) = match rest.find(DISPLAY) {
        Some(i) => (&rest[..i], Some(rest[i + DISPLAY.len()..].parse().ok()?)),
        None => (rest, None),
    };
    valid_code(code).then_some((code, display))
}

fn label(code: &str, display: Option<u32>) -> String {
    match display {
        Some(i) => format!("{PREFIX}{code}{DISPLAY}{i}"),
        None => format!("{PREFIX}{code}"),
    }
}

fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

/// 260778 → "260 778"（和界面里显示的一样）
fn fmt_code(code: &str) -> String {
    format!("{} {}", &code[..3], &code[3..])
}

fn bring_to_front(w: &WebviewWindow) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

/// 把最新的列表发给控制台
fn notify(app: &AppHandle) {
    let rows = list(app);
    let _ = app.emit_to(CONSOLE, CHANGED, rows);
}
