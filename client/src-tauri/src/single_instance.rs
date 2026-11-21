//! 单实例：不产生任何文件。
//!
//! Windows 用命名互斥量（内核对象），进程退出时自动释放，不会留下锁文件。

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




