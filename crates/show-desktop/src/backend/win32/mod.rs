//! `windows` crate 上的真实后端。
//!
//! 无状态：每次调用直接落到 Win32，记忆由 [`crate::controller::ShowDesktop`] 负责。
//! 显示器归属与标题读取等通用能力来自 `whelper-common`，这里只保留窗口样式与显隐操作。

mod enumeration;
mod ops;

use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SW_MINIMIZE, SW_RESTORE};
use whelper_common::geometry::{MonitorId, MonitorInfo};
use whelper_common::win32::{monitor, window as common_window};

use crate::backend::DesktopBackend;
use crate::window::{state_from_style, WindowId, WindowState};

#[derive(Debug, Default, Clone, Copy)]
pub struct Win32Backend;

impl Win32Backend {
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopBackend for Win32Backend {
    fn cursor_monitor(&self) -> Option<MonitorId> {
        monitor::cursor_monitor()
    }

    fn app_windows(&self) -> Vec<WindowId> {
        enumeration::app_windows()
    }

    fn window_monitor(&self, window: WindowId) -> Option<MonitorId> {
        monitor::monitor_of_window(ops::hwnd(window))
    }

    fn window_state(&self, window: WindowId) -> WindowState {
        state_from_style(ops::style(window))
    }

    fn foreground_window(&self) -> Option<WindowId> {
        let foreground = unsafe { GetForegroundWindow() };
        (!foreground.0.is_null()).then_some(WindowId(foreground.0 as isize))
    }

    fn activate(&self, window: WindowId) -> bool {
        common_window::activate(window)
    }

    fn window_title(&self, window: WindowId) -> String {
        ops::title(window)
    }

    fn minimize(&self, window: WindowId) {
        // 上游 SetMinimizeWindow 用 SW_MINIMIZE，且只对可见窗口调用（控制器已过滤）。
        ops::show(window, SW_MINIMIZE);
    }

    fn restore(&self, window: WindowId) {
        // 上游 SetRestoreWindow 用 SW_RESTORE。
        ops::show(window, SW_RESTORE);
    }

    fn monitor_info(&self, monitor: MonitorId) -> Option<MonitorInfo> {
        monitor::info(monitor)
    }
}
