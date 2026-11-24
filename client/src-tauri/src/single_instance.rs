//! 单实例：不产生任何文件。
//!
//! Windows 用命名互斥量（内核对象），进程退出时自动释放，不会留下锁文件。

/// 已经有一个在运行（托盘里）：弹一句说明再退出。
///
/// 不做这个的话，双击图标会"什么都不发生"（看起来像闪退）—— 单实例的互斥量让新进程直接返回，
/// 用户看不到任何提示，就以为程序坏了。
pub fn show_already_running() {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let text = HSTRING::from(tr!(
        "Nife Remote 已经在运行：看屏幕右下角的托盘图标。",
        "Nife Remote is already running: look for its icon in the notification area."
    ));
    unsafe {
        MessageBoxW(
            None,
            &text,
            &HSTRING::from("Nife Remote"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub struct Guard {
    _handle: windows::Win32::Foundation::HANDLE,
}

pub fn acquire() -> Option<Guard> {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\Nife Remote.SingleInstance")).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return None;
        }
        Some(Guard { _handle: handle })
    }
}
