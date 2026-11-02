//! 图标资源：多尺寸 `assets/whelper.ico` 直接编进程序。
//!
//! 托盘用 16px（`SM_CXSMICON`），窗口/任务栏用 32px（`SM_CXICON`）；
//! 资源里没有匹配尺寸时退回系统默认应用图标，不会出现“有托盘但没图标”。

use whelper_common::win32::icon;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, LoadIconW, HICON, IDI_APPLICATION, SM_CXICON, SM_CXSMICON,
};

/// 生成的图标（`tools/make-icon.ps1`）。
const ICO: &[u8] = include_bytes!("../assets/whelper.ico");

/// 窗口 / 任务栏图标。
pub fn large() -> Option<HICON> {
    load(SM_CXICON)
}

/// 托盘 / 标题栏小图标。
pub fn small() -> Option<HICON> {
    load(SM_CXSMICON)
}

fn load(metric: windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX) -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(metric) }.max(16);
    icon::from_ico(ICO, size).or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION).ok() })
}

/// `--icon-report`：确认嵌入的图标能被系统解析。
pub fn report() -> i32 {
    let sizes = icon::sizes(ICO);
    println!(
        "whelper.ico：{} 字节，尺寸 {:?}",
        ICO.len(),
        sizes.iter().map(|size| format!("{size}px")).collect::<Vec<_>>()
    );

    let large = large();
    let small = small();
    println!(
        "系统图标尺寸：大 {} / 小 {}",
        unsafe { GetSystemMetrics(SM_CXICON) },
        unsafe { GetSystemMetrics(SM_CXSMICON) }
    );
    println!(
        "加载结果：大 {} / 小 {}",
        if large.is_some() { "OK" } else { "FAIL" },
        if small.is_some() { "OK" } else { "FAIL" }
    );

    if let Some(icon) = large {
        icon::destroy(icon);
    }
    if let Some(icon) = small {
        icon::destroy(icon);
    }

    if sizes.is_empty() || large.is_none() {
        1
    } else {
        0
    }
}
