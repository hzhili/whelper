//! 平台抽象。
//!
//! 控制器只依赖 [`DesktopBackend`]，因此换平台（或写测试桩）不需要改决策逻辑。

#[cfg(test)]
pub mod memory;
#[cfg(windows)]
pub mod win32;

use whelper_common::geometry::{MonitorId, MonitorInfo};

use crate::window::{WindowId, WindowState};

pub trait DesktopBackend {
    /// 光标所在显示器。
    ///
    /// Win32：`GetCursorPos` + `MonitorFromPoint(MONITOR_DEFAULTTONEAREST)`。
    fn cursor_monitor(&self) -> Option<MonitorId>;

    /// Alt+Tab 会列出的顶层窗口，**按 Z 序自上而下**。
    ///
    /// Win32：`EnumWindows` 的自然顺序，过滤规则见 [`crate::window::classify`]。
    fn app_windows(&self) -> Vec<WindowId>;

    /// 窗口所属显示器。
    ///
    /// 最小化窗口按「还原矩形」归属，否则多屏下会因窗口被移到 (-32000, -32000)
    /// 而误判到别的显示器。
    fn window_monitor(&self, window: WindowId) -> Option<MonitorId>;

    /// 窗口当前状态（可见 / 最小化 / 不可见）。
    fn window_state(&self, window: WindowId) -> WindowState;

    /// 当前前台窗口；用于在最小化前保存并在还原后恢复原焦点。
    fn foreground_window(&self) -> Option<WindowId>;

    /// 显示并激活窗口，返回它是否成功成为前台窗口。
    fn activate(&self, window: WindowId) -> bool;

    /// 窗口标题，仅用于日志和预览输出。
    fn window_title(&self, window: WindowId) -> String;

    fn minimize(&self, window: WindowId);

    fn restore(&self, window: WindowId);

    fn monitor_info(&self, monitor: MonitorId) -> Option<MonitorInfo>;
}
