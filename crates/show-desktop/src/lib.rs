//! 只对「光标所在的那一个显示器」执行显示桌面（Windows `Win+D` 的单屏版）。
//!
//! 这是 [ShowDesktopOneMonitor](https://github.com/ruzrobert/ShowDesktopOneMonitor)
//! （C# / WinForms，核心取自 FrigoTab）的 Rust 重写。行为与上游一致：按下热键最小化
//! 当前显示器上的窗口并记住它们的状态，再按一次按原 Z 序还原；期间如果有窗口被打开
//! 或关闭，则重新最小化（与 `Win+D` 的“状态变了就重新隐藏”一致）。
//!
//! # 分层
//!
//! ```text
//! controller            编排：枚举 → 决策 → 执行 → 记忆
//! ├── plan              纯逻辑：这次该最小化还是还原、按什么顺序
//! ├── store             纯数据：每个显示器上次记住的窗口清单
//! ├── window            纯数据与纯函数：窗口身份、状态、Alt+Tab 分类
//! └── backend           平台抽象（trait），Win32 实现见 `backend::win32`
//! ```
//!
//! 显示器/矩形类型来自 `whelper-common::geometry`（与 `text-picker` 共用）。
//!
//! 除 `backend::win32` 外全部不依赖 Win32，可跨平台编译和单测；接入方只要实现
//! [`backend::DesktopBackend`] 就能把同一套决策逻辑复用到别的窗口系统或测试桩上。
//!
//! # 示例
//!
//! ```no_run
//! use show_desktop::backend::win32::Win32Backend;
//! use show_desktop::ShowDesktop;
//!
//! let mut show_desktop = ShowDesktop::new(Win32Backend::new());
//! let report = show_desktop.toggle_at_cursor()?;
//! println!("{:?} {} 个窗口", report.action, report.affected);
//! # Ok::<(), show_desktop::Error>(())
//! ```

#[cfg(windows)]
pub mod app;
pub mod backend;
pub mod controller;
pub mod plan;
#[cfg(windows)]
pub mod service;
pub mod store;
pub mod window;

#[cfg(windows)]
mod shortcut;

pub use controller::{Error, Preview, ShowDesktop, ToggleAction, ToggleReport, WindowDescriptor};
pub use whelper_common::geometry::{MonitorId, MonitorInfo, Point, Rect};
pub use window::{WindowEntry, WindowId, WindowKind, WindowState};
