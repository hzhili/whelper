//! 进程内的轻量窗口约定：组件之间**不互相依赖**，但要能互相唤起。
//!
//! 类名与消息号是“宿主约定”而不是业务逻辑：`whelper-shell` 用隐藏窗口接收托盘
//! 回调与打开设置请求，`text-picker` 用它拉起设置页、让托盘正常退出。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP, WM_CLOSE};

use super::string::{to_wide, wide_ptr};

/// `whelper-shell` 的隐藏外壳窗口类名。
pub const SHELL_CLASS: &str = "WHelperShell";
/// `text-picker` 的主窗口类名（隐藏消息窗口 / 工具条 / 翻译面板共用）。
pub const TOOLBAR_CLASS: &str = "TextPickerWnd";
/// 外壳窗口收到它就显示设置页。
pub const MSG_OPEN_SETTINGS: u32 = WM_APP + 60;

/// 按类名找本进程的顶层窗口（`FindWindow` 只查顶层，够用）。
pub fn find_by_class(class: &str) -> Option<HWND> {
    let wide = to_wide(class);
    unsafe { FindWindowW(wide_ptr(&wide), PCWSTR::null()) }.ok()
}

/// 给指定类名的窗口投递一条无参数消息。
pub fn post_to_class(class: &str, message: u32) -> bool {
    let Some(hwnd) = find_by_class(class) else {
        return false;
    };
    unsafe { PostMessageW(hwnd, message, WPARAM(0), LPARAM(0)).is_ok() }
}

/// 打开设置页（外壳不在时返回 `false`）。
pub fn open_settings() -> bool {
    post_to_class(SHELL_CLASS, MSG_OPEN_SETTINGS)
}

/// 让取词组件走正常退出路径（`WM_CLOSE` → 销毁窗口 → 摘钩子 → 退出消息循环）。
pub fn close_toolbar() -> bool {
    post_to_class(TOOLBAR_CLASS, WM_CLOSE)
}
