//! 显示器归属与工作区查询。
//!
//! 两个组件都需要“这个点/这个窗口属于哪块屏”：
//! 取词工具条要按锚点夹紧工作区，单屏显示桌面要按光标挑显示器。

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowPlacement, SW_MINIMIZE, SW_SHOWMINIMIZED, SW_SHOWMINNOACTIVE,
    WINDOWPLACEMENT,
};

use crate::geometry::{MonitorId, MonitorInfo, Point, Rect};

/// `MONITORINFOF_PRIMARY`：windows 0.58 的 Gdi 模块没有导出这个常量。
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// 光标所在显示器。
pub fn cursor_monitor() -> Option<MonitorId> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    monitor_of_point(Point::new(point.x, point.y))
}

pub fn monitor_of_point(point: Point) -> Option<MonitorId> {
    let raw = POINT { x: point.x, y: point.y };
    from_hmonitor(unsafe { MonitorFromPoint(raw, MONITOR_DEFAULTTONEAREST) })
}

/// 窗口所属显示器。
///
/// 最小化窗口的可见矩形在 (-32000, -32000)，直接 `MonitorFromWindow` 会把它算到
/// 最靠近这个坐标的屏幕上；这里改用 `WINDOWPLACEMENT.rcNormalPosition` 归属。
pub fn monitor_of_window(hwnd: HWND) -> Option<MonitorId> {
    if let Some(rect) = minimized_normal_rect(hwnd) {
        return from_hmonitor(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) });
    }
    from_hmonitor(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
}

/// 某点所在显示器的工作区（已排除任务栏）。
pub fn work_area(point: Point) -> Option<Rect> {
    info(monitor_of_point(point)?).map(|info| info.work)
}

pub fn info(monitor: MonitorId) -> Option<MonitorInfo> {
    let hmonitor = HMONITOR(monitor.0 as *mut c_void);
    if hmonitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmonitor, &mut info).as_bool() } {
        return None;
    }
    Some(MonitorInfo {
        id: monitor,
        bounds: to_rect(info.rcMonitor),
        work: to_rect(info.rcWork),
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

fn minimized_normal_rect(hwnd: HWND) -> Option<RECT> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &mut placement).ok()? };

    let minimized = placement.showCmd == SW_SHOWMINIMIZED.0 as u32
        || placement.showCmd == SW_MINIMIZE.0 as u32
        || placement.showCmd == SW_SHOWMINNOACTIVE.0 as u32;
    minimized.then_some(placement.rcNormalPosition)
}

fn from_hmonitor(monitor: HMONITOR) -> Option<MonitorId> {
    if monitor.is_invalid() {
        None
    } else {
        Some(MonitorId(monitor.0 as usize))
    }
}

fn to_rect(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right, rect.bottom)
}
