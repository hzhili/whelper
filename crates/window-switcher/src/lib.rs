//! Alt+Tab 窗口切换器：缩略图 / 网格两种视图，带实时预览与焦点切换。
//!
//! 交互模型参考 [sigoden/window-switcher](https://github.com/sigoden/window-switcher)
//! 的“按住 Alt 连续按键循环、松开才真正切换”，界面部分按本项目的覆盖窗口/GDI 约定重写：
//!
//! ```text
//! Alt 按下（放行）
//!   └─ Tab 按下 ──► 吞掉 + 注入掩码键 + 弹出覆盖层（第 1 次）
//!        Tab 抬起 ──► 吞掉（配对的抬起不能漏给前台应用）
//!        Tab 再按 ──► 选择前移/后移，覆盖层保持
//!        Alt+`    ──► 缩略图 / 网格视图切换（带布局插值过渡）
//!      Alt 抬起 ──► 提交：激活当前选择，覆盖层淡出
//!      Esc      ──► 取消：覆盖层淡出，不切换
//! ```
//!
//! # 分层
//!
//! ```text
//! model          纯状态：候选窗口、选择、视图模式、导航步长
//! layout         纯几何：单行应用图标 / 窗口预览网格 / 命中测试 / 布局插值
//! anim           纯时间轴：进度与缓动
//! shortcut       Alt+Tab 键盘状态机（纯判定）+ 共享键盘钩子处理器
//! win32          平台能力：窗口枚举、图标、DPI、工作区
//! surface        32 位 DIB 绘制面：逐像素透明（`UpdateLayeredWindow`）
//! overlay        覆盖窗口：应用图标/卡片绘制、DWM 缩略图同步、动画、鼠标交互、消息
//! service        常驻形态：创建覆盖窗口 + 注册键盘钩子，供宿主嵌入
//! app            一次性命令：report / self_test / demo
//! ```
//!
//! 除 `win32` / `overlay` / `service` / `shortcut` 的钩子部分外全部不依赖 Win32，
//! 可跨平台单测；窗口候选集合与 `show-desktop` 共用
//! [`whelper_common::win32::window::alt_tab_windows`]，不存在两套过滤规则。
//!
//! # 示例
//!
//! ```no_run
//! // 宿主（whelper）在主线程、进入自己的消息循环之前启动
//! let service = window_switcher::service::Service::start();
//! // ... 宿主消息循环（GetMessage/DispatchMessage）
//! drop(service);
//! ```

pub mod anim;
pub mod layout;
pub mod model;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod preview;
#[cfg(windows)]
pub mod service;
#[cfg(windows)]
pub mod shortcut;
#[cfg(windows)]
mod surface;
#[cfg(windows)]
pub mod win32;

pub use model::{Session, Step, ViewMode, WindowEntry};
pub use whelper_common::window::WindowId;
