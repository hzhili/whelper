//! 控制台窗口控制。
//!
//! 从注册表 `Run` 项开机自启时，控制台子系统的程序会先弹出一个黑框；
//! 在 `--autostart` 启动路径上尽早调用 [`hide_console`] 把它藏起来。
//!
//! 只 `ShowWindow(SW_HIDE)` 在 Windows Terminal 的 `PseudoConsoleWindow` 上无效，
//! 因此再 `FreeConsole` 彻底脱开控制台（自启模式不需要 stdout）。

use windows::Win32::System::Console::{FreeConsole, GetConsoleWindow};
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

/// 隐藏并脱离当前进程的控制台窗口（没有控制台时什么也不做）。
pub fn hide_console() {
    unsafe {
        let hwnd = GetConsoleWindow();
        if !hwnd.0.is_null() {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        // 关掉控制台/伪控制台窗口；之后 stdout/stderr 写入会被系统丢弃。
        let _ = FreeConsole();
    }
}
