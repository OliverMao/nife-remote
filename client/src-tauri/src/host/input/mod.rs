//! 输入注入。每个会话一个 [`Injector`]，记录按下的键/鼠标键，会话结束时全部释放，避免粘键。
//!
//! 注入是同步的：会话不直接用 [`Injector`]，而是把输入交给自己的注入线程
//! （[`InjectorThread`]），`Injector` 在那个线程上创建、使用、释放。

mod keymap;
mod thread;
mod windows;

use nife_remote_proto::session::{DisplayInfo, InputEvent};

pub use thread::InjectorThread;

pub struct Injector {
    inner: windows::WinInjector,
}

impl Injector {
    pub fn new() -> Self {
        Self {
            inner: windows::WinInjector::default(),
        }
    }

    /// 处理一批输入事件。`display` 为当前画面对应的显示器（用于坐标换算）。
    #[allow(unused_variables)]
    pub fn handle(&mut self, events: &[InputEvent], display: &DisplayInfo) {
        self.inner.handle(events, display);
    }

    pub fn release_all(&mut self) {
        self.inner.release_all();
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::windows::Win32::Foundation::POINT;
    use ::windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

    fn cursor() -> (i32, i32) {
        let mut p = POINT::default();
        crate::host::dpi_aware(|| unsafe { GetCursorPos(&mut p).unwrap() });
        (p.x, p.y)
    }

    /// 把鼠标移到主显示器中心，然后恢复原位置。
    #[test]
    #[ignore]
    fn move_to_center() {
        let d = crate::host::capture::displays()
            .into_iter()
            .find(|d| d.primary)
            .unwrap();
        println!("display {d:?}");

        let before = cursor();
        let mut inj = Injector::new();
        inj.handle(&[InputEvent::MouseMove { x: 0.5, y: 0.5 }], &d);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = cursor();
        crate::host::dpi_aware(|| unsafe { SetCursorPos(before.0, before.1).unwrap() });
        println!("cursor after = {after:?}");
        let (ex, ey) = (d.x + d.width as i32 / 2, d.y + d.height as i32 / 2);
        assert!(
            (after.0 - ex).abs() <= 2 && (after.1 - ey).abs() <= 2,
            "期望 ({ex},{ey}) 实际 {after:?}"
        );
    }
}

