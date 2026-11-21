//! 本机标识（计算机名 / 操作系统），仅用于展示。计算机名未知时为空字符串。

use std::sync::OnceLock;

pub fn host_identity() -> (String, String) {
    static CACHE: OnceLock<(String, String)> = OnceLock::new();
    CACHE.get_or_init(|| (hostname(), os_name())).clone()
}

fn hostname() -> String {
    // 先看 COMPUTERNAME（Windows 的计算机名），再退回 HOSTNAME
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key)
            && !v.trim().is_empty()
        {
            return v;
        }
    }
    // 不知道名称：发空字符串，由显示的一方按自己的语言显示"未知设备"
    String::new()
}

fn os_name() -> String {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::{HSTRING, PCWSTR};

    fn reg(name: &str) -> Option<String> {
        let key = HSTRING::from(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        let name = HSTRING::from(name);
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        if ok.is_err() {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
    }

    let build: u32 = reg("CurrentBuild")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let product = reg("ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 的注册表 ProductName 仍然写着 Windows 10
    let product = if build >= 22000 {
        product.replace("Windows 10", "Windows 11")
    } else {
        product
    };
    match reg("DisplayVersion") {
        Some(v) => format!("{product} {v}"),
        None => product,
    }
}







