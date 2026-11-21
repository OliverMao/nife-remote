//! 界面语言：简体中文 / 英文。
//!
//! * 默认跟随系统界面语言：系统是中文就用中文，其它语言（或识别不出来）一律用英文；
//! * 托盘菜单和控制台都可以切换。**不写进配置文件**，只在本次运行期间有效；
//! * 托盘和窗口标题按当前语言生成（用 [`tr!`]）；控制台的文字在前端（`src/lib/i18n.ts`）；
//! * 发给界面的问题与提示都是类型（见 [`crate::problem`]），显示时才按语言变成文字；日志始终是英文。

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Lang {
    Zh = 1,
    En = 2,
}

impl Lang {
    /// 前端使用的代码（`"zh"` / `"en"`）
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }
}

/// 0 = 尚未确定（第一次使用时读取系统语言）
static CURRENT: AtomicU8 = AtomicU8::new(0);

fn decode(v: u8) -> Option<Lang> {
    match v {
        1 => Some(Lang::Zh),
        2 => Some(Lang::En),
        _ => None,
    }
}

pub fn current() -> Lang {
    if let Some(l) = decode(CURRENT.load(Ordering::Relaxed)) {
        return l;
    }
    let l = system();
    // 多个线程同时第一次调用时，以先写入的为准（用户已经切换过就不会被覆盖）
    match CURRENT.compare_exchange(0, l as u8, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => l,
        Err(v) => decode(v).unwrap_or(l),
    }
}

/// 切换界面语言，返回是否有变化。
pub fn set(lang: Lang) -> bool {
    CURRENT.swap(lang as u8, Ordering::Relaxed) != lang as u8
}

pub fn is_zh() -> bool {
    current() == Lang::Zh
}

/// 系统界面语言。只看首选语言：不是中文就用英文。
pub fn system() -> Lang {
    // 用户的"显示语言"（不是"区域格式"）；主语言 ID 0x04 = LANG_CHINESE（简体、繁体都算中文）
    const LANG_CHINESE: u16 = 0x04;
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if id & 0x3ff == LANG_CHINESE {
        Lang::Zh
    } else {
        Lang::En
    }
}



/// 与 gettext 相同的规则：`LC_ALL` > `LC_MESSAGES` > `LANG` 决定区域设置；
/// 区域设置不是 C/POSIX 时，`LANGUAGE`（冒号分隔的优先列表）优先，取第一个。
/// 按当前界面语言二选一并格式化，写法同 `format!`：`tr!("显示器 {n}", "Display {n}")`。
macro_rules! tr {
    ($zh:literal, $en:literal $(, $($args:tt)*)?) => {
        if $crate::i18n::is_zh() { format!($zh $(, $($args)*)?) } else { format!($en $(, $($args)*)?) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch() {
        let before = current();
        set(Lang::En);
        assert_eq!(tr!("中文 {}", "English {}", 1), "English 1");
        let n = 2;
        assert!(!set(Lang::En));
        assert!(set(Lang::Zh));
        assert_eq!(tr!("中文 {n}", "English {n}"), "中文 2");
        set(before);
    }
}
