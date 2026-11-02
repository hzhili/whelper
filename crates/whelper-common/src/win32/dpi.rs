//! 进程级 DPI 感知。
//!
//! 必须在**创建任何窗口之前**设置；各组件虽然也会自己调用一次（单独调试时），
//! 但组合根需要更早调用：托盘、设置页可能先于工具条窗口创建。

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// 开启 Per-Monitor V2（已经设置过时调用失败，忽略即可）。
pub fn enable_per_monitor_v2() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
