//! Win32 基础封装：只做“把系统 API 包成安全、可复用的小函数”，不含业务语义。

pub mod applink;
pub mod console;
pub mod dpi;
pub mod hook;
pub mod icon;
pub mod instance;
pub mod keyboard;
pub mod monitor;
pub mod string;
pub mod theme;
pub mod window;

use windows::Win32::Foundation::HWND;

/// 空窗口句柄（`windows` 0.58 的句柄是裸指针）。
pub fn null_hwnd() -> HWND {
    HWND(std::ptr::null_mut())
}
