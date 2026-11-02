//! 单测专用后端：用内存里的假窗口替代 Win32。
//!
//! 顺序即 Z 序（与 `EnumWindows` 一致），最小化/还原只改状态并记录调用顺序。

use std::cell::RefCell;

use whelper_common::geometry::{MonitorId, MonitorInfo, Rect};

use crate::backend::DesktopBackend;
use crate::window::{WindowId, WindowState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Minimize(WindowId),
    Restore(WindowId),
}

#[derive(Debug, Clone)]
struct FakeWindow {
    id: WindowId,
    monitor: MonitorId,
    state: WindowState,
    title: String,
}

#[derive(Debug)]
pub struct MemoryBackend {
    cursor: MonitorId,
    windows: RefCell<Vec<FakeWindow>>,
    calls: RefCell<Vec<Call>>,
}

impl MemoryBackend {
    pub fn new(cursor: MonitorId) -> Self {
        Self { cursor, windows: RefCell::new(Vec::new()), calls: RefCell::new(Vec::new()) }
    }

    /// 追加一个窗口；调用顺序即 Z 序（先加的在最上层）。
    pub fn add_window(
        &mut self,
        id: isize,
        monitor: MonitorId,
        state: WindowState,
        title: &str,
    ) -> &mut Self {
        self.windows.borrow_mut().push(FakeWindow {
            id: WindowId(id),
            monitor,
            state,
            title: title.to_string(),
        });
        self
    }

    pub fn remove_window(&mut self, id: WindowId) {
        self.windows.borrow_mut().retain(|window| window.id != id);
    }

    pub fn set_state(&mut self, id: WindowId, state: WindowState) {
        if let Some(window) = self.windows.borrow_mut().iter_mut().find(|w| w.id == id) {
            window.state = state;
        }
    }

    pub fn set_cursor(&mut self, monitor: MonitorId) {
        self.cursor = monitor;
    }

    pub fn state_of(&self, id: WindowId) -> Option<WindowState> {
        self.windows.borrow().iter().find(|w| w.id == id).map(|w| w.state)
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    pub fn clear_calls(&self) {
        self.calls.borrow_mut().clear();
    }

    fn update_state(&self, id: WindowId, state: WindowState) {
        if let Some(window) = self.windows.borrow_mut().iter_mut().find(|w| w.id == id) {
            window.state = state;
        }
    }
}

impl DesktopBackend for MemoryBackend {
    fn cursor_monitor(&self) -> Option<MonitorId> {
        Some(self.cursor)
    }

    fn app_windows(&self) -> Vec<WindowId> {
        self.windows.borrow().iter().map(|w| w.id).collect()
    }

    fn window_monitor(&self, window: WindowId) -> Option<MonitorId> {
        self.windows.borrow().iter().find(|w| w.id == window).map(|w| w.monitor)
    }

    fn window_state(&self, window: WindowId) -> WindowState {
        self.state_of(window).unwrap_or(WindowState::Disabled)
    }

    fn window_title(&self, window: WindowId) -> String {
        self.windows
            .borrow()
            .iter()
            .find(|w| w.id == window)
            .map(|w| w.title.clone())
            .unwrap_or_default()
    }

    fn minimize(&self, window: WindowId) {
        self.update_state(window, WindowState::Minimized);
        self.calls.borrow_mut().push(Call::Minimize(window));
    }

    fn restore(&self, window: WindowId) {
        self.update_state(window, WindowState::Visible);
        self.calls.borrow_mut().push(Call::Restore(window));
    }

    fn monitor_info(&self, monitor: MonitorId) -> Option<MonitorInfo> {
        Some(MonitorInfo {
            id: monitor,
            bounds: Rect::new(0, 0, 1920, 1080),
            work: Rect::new(0, 0, 1920, 1040),
            primary: monitor == MonitorId(1),
        })
    }
}
