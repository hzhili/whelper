//! whelper 的通用底座。
//!
//! 只放**跨组件共用**的东西，不实现任何业务功能：
//!
//! | 模块 | 内容 | 谁在用 |
//! | --- | --- | --- |
//! | [`geometry`] | 显示器/矩形/点的纯数据类型（不依赖 Win32，可跨平台单测） | `text-picker`、`show-desktop`、`window-switcher` |
//! | [`window`] | 窗口身份/状态 + Alt+Tab 等价分类的纯判定 | `show-desktop`、`window-switcher` |
//! | [`win32::string`] | UTF-16 转换与 `PCWSTR` | 三个组件 |
//! | [`win32::instance`] | 进程级单实例守卫 | 各组件的常驻入口 |
//! | [`win32::monitor`] | 光标/窗口/点的显示器归属、工作区 | `text-picker`（工具条定位）、`show-desktop`（单屏判定） |
//! | [`win32::hook`] | `WH_KEYBOARD_LL` 的安装与事件分发 | `text-picker`（取词触发）、`show-desktop`（接管 Win+D）、`window-switcher`（Alt+Tab） |
//! | [`win32::keyboard`] | 修饰键查询与掩码键注入 | 吞系统热键的组件 |
//! | [`win32::window`] | Alt+Tab 等价顶层窗口枚举、标题、激活 | `show-desktop`、`window-switcher` |
//! | [`win32::theme`] | 系统深浅色主题 | 弹窗类组件 |
//! | [`win32::icon`] | 从嵌入的 `.ico` 字节创建 `HICON` | 托盘 / 任务栏 / 设置页 |
//! | [`win32::applink`] | 进程内窗口类名与消息约定（打开设置、正常退出） | `text-picker`、`whelper-shell` |
//! | [`win32::console`] | 隐藏控制台窗口 | `whelper`（开机自启） |
//! | [`win32::dpi`] | 进程级 DPI 感知 | `whelper`（创建任何窗口之前） |
//!
//! 组件特有的能力（取词、绘制、窗口状态机、热键语义）留在各组件内部，不要往这里加。

pub mod geometry;
pub mod window;

#[cfg(windows)]
pub mod win32;
