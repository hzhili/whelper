//! Alt+Tab 等价的顶层窗口枚举。
//!
//! 枚举与过滤规则（cloaked → `WS_DISABLED` → 无 `WS_VISIBLE` → `WS_EX_NOACTIVATE` →
//! `WS_EX_APPWINDOW` → `WS_EX_TOOLWINDOW` → `IsAltTabWindow` 启发式）在
//! `whelper-common::win32::window`，与 Alt+Tab 切换器共用同一份实现。

use whelper_common::win32::window;

use crate::window::WindowId;

/// 按 Z 序（自上而下）返回参与 Alt+Tab 的应用窗口。
pub(super) fn app_windows() -> Vec<WindowId> {
    window::alt_tab_windows()
}
