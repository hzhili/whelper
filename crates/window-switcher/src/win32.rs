//! 平台辅助：候选窗口、光标显示器的工作区与 DPI、调试开关。
//!
//! 只做“取数据/转发”，界面与决策分别在 `overlay` 与 `model`/`layout`。

use std::ffi::c_void;
use std::sync::OnceLock;

use whelper_common::geometry::{Point, Rect};
use whelper_common::win32::{monitor, window as common_window};
use whelper_common::window::WindowId;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, POINT};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowThreadProcessId};

use crate::model::WindowEntry;

/// 当前系统 Alt+Tab 的主要视觉版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSwitcherStyle {
    Windows10,
    Windows11,
}

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(version: *mut OsVersionInfoW) -> i32;
}

static NATIVE_SWITCHER_STYLE: OnceLock<NativeSwitcherStyle> = OnceLock::new();

/// 使用 `RtlGetVersion` 读取未受应用兼容性清单虚报影响的系统版本。
pub fn native_switcher_style() -> NativeSwitcherStyle {
    *NATIVE_SWITCHER_STYLE.get_or_init(detect_native_switcher_style)
}

fn detect_native_switcher_style() -> NativeSwitcherStyle {
    let mut version = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    let status = unsafe { RtlGetVersion(&mut version) };
    if status == 0 && version.major >= 10 && version.build >= 22_000 {
        NativeSwitcherStyle::Windows11
    } else {
        NativeSwitcherStyle::Windows10
    }
}

/// 枚举 Alt+Tab 候选窗口。
///
/// 候选集合与 Z 序来自 [`whelper_common::win32::window::alt_tab_windows`]，
/// 与 `show-desktop` 是同一份实现，两个功能看到的窗口顺序天然一致。

pub fn windows(cursor_monitor_only: bool) -> Vec<WindowEntry> {
    let cursor_monitor = cursor_monitor_only.then(monitor::cursor_monitor).flatten();
    common_window::alt_tab_windows()
        .into_iter()
        .filter(|window| {
            cursor_monitor.is_none_or(|monitor_id| {
                monitor::monitor_of_window(common_window::hwnd(*window)) == Some(monitor_id)
            })
        })
        .map(|id| WindowEntry {
            id,
            app_id: application_id(id),
            title: common_window::title(id),
            minimized: common_window::is_minimized(id),
        })
        .collect()
}

/// 以可执行文件路径作为应用分组键，让同一程序的多个顶层窗口合并显示。
/// 路径查询失败时退回进程 ID，避免把未知应用错误合并到一起。
fn application_id(window: WindowId) -> String {
    let hwnd = common_window::hwnd(window);
    let mut process_id = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == 0 {
        return format!("window:{window}");
    }

    let Ok(process) =
        (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return format!("pid:{process_id}");
    };
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    if result.is_ok() && length > 0 {
        String::from_utf16_lossy(&path[..length as usize]).to_lowercase()
    } else {
        format!("pid:{process_id}")
    }
}

/// 光标所在显示器的工作区（已排除任务栏）。
///
/// 失败时退化为主屏尺寸：宁可位置不准，也不要因为取不到显示器而完全不弹窗。
pub fn cursor_work_area() -> Rect {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    monitor::work_area(Point::new(point.x, point.y)).unwrap_or_else(|| Rect::new(0, 0, 1920, 1080))
}

/// 光标所在显示器的缩放比例（96 DPI = 1.0）。
///
/// 用 `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` 而不是 `GetDpiForWindow`：
/// 覆盖窗口还没移到目标显示器上，按窗口取 DPI 拿到的是**上一块屏**的值。
pub fn cursor_scale() -> f32 {
    let Some(id) = monitor::cursor_monitor() else {
        return 1.0;
    };
    let hmonitor = HMONITOR(id.0 as *mut c_void);
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    let ok = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if ok.is_err() || dpi_x == 0 {
        return 1.0;
    }
    dpi_x as f32 / 96.0
}

/// 激活窗口（最小化先还原、绕过前台锁），返回是否真的成为前台窗口。
pub fn activate(window: WindowId) -> bool {
    common_window::activate(window)
}

/// `SWITCHER_DEBUG=1` 或设置页的“调试日志”：打印会话生命周期与预览抓取失败原因。
pub fn debug_enabled() -> bool {
    static ENV: OnceLock<bool> = OnceLock::new();
    *ENV.get_or_init(|| std::env::var_os("SWITCHER_DEBUG").is_some())
        || whelper_config::get().general.debug
}
