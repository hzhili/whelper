//! 单个窗口上的 Win32 操作：样式读取、最小化 / 还原、标题。
//!
//! 标题读取与 `Alt+Tab` 候选枚举一样属于共享能力，直接转给 `whelper-common`；
//! 这里只保留「显示桌面」语义下的显隐操作。

use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, ShowWindow, GWL_STYLE, SHOW_WINDOW_CMD};
use whelper_common::win32::window as common_window;

use crate::window::WindowId;

/// `WindowId` → `HWND`。只在本模块与 `monitor` 内部使用。
pub(super) fn hwnd(window: WindowId) -> HWND {
    HWND(window.0 as *mut c_void)
}

pub(super) fn style(window: WindowId) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd(window), GWL_STYLE) as u32 }
}

pub(super) fn show(window: WindowId, command: SHOW_WINDOW_CMD) {
    // 上游同样忽略 ShowWindow 的返回值：窗口可能已退出，失败无需处理。
    unsafe {
        let _ = ShowWindow(hwnd(window), command);
    }
}

pub(super) fn title(window: WindowId) -> String {
    common_window::title(window)
}
