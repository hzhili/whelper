//! 常驻形态：创建覆盖窗口并注册 Alt+Tab 处理器，供宿主嵌入。
//!
//! 与 `show-desktop::service` 一样，本模块**不跑消息循环**：窗口由宿主
//! （`whelper` 与取词组件共用的主循环）驱动，触发路径是
//! 「钩子吞键 → `PostMessage` → 覆盖窗口 wndproc → 会话操作」。

use windows::Win32::Foundation::HWND;

use crate::overlay::Overlay;
use crate::shortcut;

/// 常驻服务句柄：持有覆盖窗口，析构即注销钩子、销毁窗口。
pub struct Service {
    overlay: Box<Overlay>,
}

impl Service {
    /// 创建覆盖窗口并接管 Alt+Tab。返回 `None` 表示窗口或键盘钩子创建失败。
    pub fn start() -> Option<Self> {
        let overlay = Overlay::create()?;
        if !shortcut::install(overlay.hwnd()) {
            return None;
        }
        Some(Self { overlay })
    }

    /// 覆盖窗口句柄，仅用于诊断输出。
    pub fn hwnd(&self) -> HWND {
        self.overlay.hwnd()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // 先摘钩子，避免窗口销毁后仍有消息投递进来；窗口本身由 `Overlay::Drop` 销毁。
        shortcut::uninstall();
    }
}
