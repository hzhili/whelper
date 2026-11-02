//! 窗口身份、状态与 Alt+Tab 等价分类（纯逻辑，不依赖 Win32）。
//!
//! `show-desktop`（单屏显示桌面）与 `window-switcher`（Alt+Tab 切换器）必须对
//! “哪些顶层窗口算应用窗口”给出**完全一致**的答案，否则同一个桌面会出现两套候选集合、
//! 两套 Z 序。因此这里放判定规则，Win32 调用在 [`crate::win32::window`]。
//!
//! 分类规则对应 ShowDesktopOneMonitor 上游 `FrigoTab/WindowFinder.cs`：先把顶层窗口
//! 分成 Hidden / AppWindow / ToolWindow，只有 AppWindow 参与切换。
//!
//! 样式位在这里重新声明而不直接引用 `windows` crate，是为了让分类逻辑与状态判定
//! 能在没有 Win32 的环境下单测（数值与 `Win32_UI_WindowsAndMessaging` 一致）。

use std::fmt;

/// 窗口句柄（`HWND`）的包装。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub isize);

impl WindowId {
    pub const NULL: Self = Self(0);

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0 as usize)
    }
}

/// 窗口在“显示桌面 / 切换”语义下的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    /// 可见且未最小化：显示桌面时会被最小化，也是切换器缩略图的来源。
    Visible,
    /// 已最小化。
    Minimized,
    /// 无 `WS_VISIBLE` 或带 `WS_DISABLED`。
    Disabled,
}

/// 记忆清单里的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowEntry {
    pub id: WindowId,
    pub state: WindowState,
}

pub const WS_DISABLED: u32 = 0x0800_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_MINIMIZE: u32 = 0x2000_0000;

pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// 由 `GWL_STYLE` 推导状态；判定顺序与上游 `DesktopWindowID` 构造函数一致
/// （先看最小化，再看可见）。
pub fn state_from_style(style: u32) -> WindowState {
    if style & WS_MINIMIZE != 0 {
        WindowState::Minimized
    } else if style & WS_VISIBLE != 0 {
        WindowState::Visible
    } else {
        WindowState::Disabled
    }
}

/// 窗口类别，对应上游 `WindowFinder.WindowType`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// 不参与切换（cloaked、不可见、`WS_EX_NOACTIVATE`、非 Alt+Tab 窗口）。
    Hidden,
    /// Alt+Tab 会列出的应用窗口。
    AppWindow,
    /// 工具窗口（上游单独收集，但“显示桌面 / 切换”都不使用）。
    ToolWindow,
}

/// 分类所需的窗口属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFlags {
    /// `GetWindowLongPtrW(GWL_STYLE)`
    pub style: u32,
    /// `GetWindowLongPtrW(GWL_EXSTYLE)`
    pub ex_style: u32,
    /// `DwmGetWindowAttribute(DWMWA_CLOAKED)`：UWP 挂起窗口等“隐身但可见”的窗口。
    pub cloaked: bool,
    /// `IsAltTabWindow`：`GetLastActiveVisiblePopup(GetAncestor(hwnd, GA_ROOTOWNER)) == hwnd`。
    ///
    /// 上游在 `GetWindowType` 的最后一个分支才计算它；这里要求调用方提前算好，
    /// 代价是每个顶层窗口多几次只读调用（一次按键一次枚举，可忽略）。
    pub alt_tab: bool,
}

/// 与上游 `WindowFinder.GetWindowType` 的判定顺序逐条对应。
pub fn classify(flags: WindowFlags) -> WindowKind {
    if flags.cloaked {
        return WindowKind::Hidden;
    }
    if flags.style & WS_DISABLED != 0 {
        return WindowKind::Hidden;
    }
    if flags.style & WS_VISIBLE == 0 {
        return WindowKind::Hidden;
    }
    let ex = flags.ex_style;
    if ex & WS_EX_NOACTIVATE != 0 {
        return WindowKind::Hidden;
    }
    if ex & WS_EX_APPWINDOW != 0 {
        return WindowKind::AppWindow;
    }
    if ex & WS_EX_TOOLWINDOW != 0 {
        return WindowKind::ToolWindow;
    }
    if flags.alt_tab {
        WindowKind::AppWindow
    } else {
        WindowKind::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(style: u32, ex_style: u32) -> WindowFlags {
        WindowFlags { style, ex_style, cloaked: false, alt_tab: true }
    }

    #[test]
    fn minimized_wins_over_visible() {
        assert_eq!(state_from_style(WS_MINIMIZE | WS_VISIBLE), WindowState::Minimized);
        assert_eq!(state_from_style(WS_VISIBLE), WindowState::Visible);
        assert_eq!(state_from_style(0), WindowState::Disabled);
        assert_eq!(state_from_style(WS_DISABLED | WS_VISIBLE), WindowState::Visible);
    }

    #[test]
    fn cloaked_window_is_hidden() {
        let mut f = flags(WS_VISIBLE, 0);
        f.cloaked = true;
        assert_eq!(classify(f), WindowKind::Hidden);
    }

    #[test]
    fn disabled_or_invisible_is_hidden() {
        assert_eq!(classify(flags(WS_DISABLED, 0)), WindowKind::Hidden);
        assert_eq!(classify(flags(0, 0)), WindowKind::Hidden);
    }

    #[test]
    fn noactivate_is_hidden_even_with_appwindow_flag() {
        let f = flags(WS_VISIBLE, WS_EX_NOACTIVATE | WS_EX_APPWINDOW);
        assert_eq!(classify(f), WindowKind::Hidden);
    }

    #[test]
    fn appwindow_and_toolwindow_flags_win_over_heuristic() {
        assert_eq!(classify(flags(WS_VISIBLE, WS_EX_APPWINDOW)), WindowKind::AppWindow);
        assert_eq!(classify(flags(WS_VISIBLE, WS_EX_TOOLWINDOW)), WindowKind::ToolWindow);
        // 上游先判 APPWINDOW：两个标志同时存在时算应用窗口。
        assert_eq!(
            classify(flags(WS_VISIBLE, WS_EX_APPWINDOW | WS_EX_TOOLWINDOW)),
            WindowKind::AppWindow
        );
    }

    #[test]
    fn plain_window_falls_back_to_alt_tab_heuristic() {
        let mut f = flags(WS_VISIBLE, 0);
        assert_eq!(classify(f), WindowKind::AppWindow);
        f.alt_tab = false;
        assert_eq!(classify(f), WindowKind::Hidden);
    }

    #[test]
    fn null_window_id_is_zero() {
        assert!(WindowId::NULL.is_null());
        assert!(!WindowId(42).is_null());
    }
}
