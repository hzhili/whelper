//! 窗口身份、状态与 Alt+Tab 等价分类。
//!
//! 实现已下沉到 [`whelper_common::window`]：Alt+Tab 切换器（`window-switcher`）需要
//! 与「显示桌面」完全相同的候选集合与判定顺序，放在共享底座里可以避免两份规则漂移。
//! 这里保留同名模块与全部导出，`crate::window::*` 的既有路径不变。

pub use whelper_common::window::*;
