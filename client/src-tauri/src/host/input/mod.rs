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
