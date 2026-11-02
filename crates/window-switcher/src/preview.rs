//! 应用图标的高分辨率取用与绘制。
//!
//! 优先从 Windows Shell 的 Jumbo / ExtraLarge 图标列表取得应用图标，再回退到窗口图标。
//! HICON 句柄由 [`AppIcon`] 复制并持有，生命周期由本模块管理。

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use whelper_common::geometry::Rect;
use whelper_common::win32::window as common_window;
use whelper_common::window::WindowId;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{HBRUSH, HDC};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, DestroyIcon, DrawIconEx, GetClassLongPtrW, IsWindow, LoadIconW, SendMessageTimeoutW,
    DI_NORMAL, GCLP_HICON, GCLP_HICONSM, HICON, ICON_BIG, ICON_SMALL, IDI_APPLICATION,
    SMTO_ABORTIFHUNG, WM_GETICON,
};

const SHIL_LARGE: i32 = 0;
const SHIL_EXTRALARGE: i32 = 2;
const SHIL_JUMBO: i32 = 4;

/// 独立持有的应用图标句柄。
pub struct AppIcon(isize);

impl AppIcon {
    pub fn raw(&self) -> isize {
        self.0
    }
}

impl Drop for AppIcon {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                let _ = DestroyIcon(HICON(self.0 as *mut c_void));
            }
        }
    }
}

/// 从 Shell 图标列表取得大尺寸应用图标；失败时复制窗口图标，再回退系统默认图标。
pub fn application_icon(executable: &str, window: WindowId) -> AppIcon {
    let is_frame_host = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ApplicationFrameHost.exe"));

    // UWP 的顶层窗口由 ApplicationFrameHost 承载，Shell 给到的是宿主图标，优先询问窗口。
    let icon = if is_frame_host {
        copied_window_icon(window).or_else(|| shell_icon(executable))
    } else {
        shell_icon(executable).or_else(|| copied_window_icon(window))
    }
    .unwrap_or_else(fallback_icon);

    AppIcon(icon.0 as isize)
}

fn shell_icon(executable: &str) -> Option<HICON> {
    if executable.is_empty() || !Path::new(executable).exists() {
        return None;
    }
    let path: Vec<u16> = executable.encode_utf16().chain(Some(0)).collect();
    let mut info = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(path.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        )
    };
    if result == 0 {
        return None;
    }

    for size in [SHIL_JUMBO, SHIL_EXTRALARGE, SHIL_LARGE] {
        let Ok(list) = (unsafe { SHGetImageList::<IImageList>(size) }) else {
            continue;
        };
        if let Ok(icon) = unsafe { list.GetIcon(info.iIcon, 1) } {
            if !icon.0.is_null() {
                return Some(icon);
            }
        }
    }
    None
}

fn copied_window_icon(window: WindowId) -> Option<HICON> {
    let hwnd = common_window::hwnd(window);
    if !unsafe { IsWindow(hwnd).as_bool() } {
        return None;
    }

    unsafe {
        if let Some(icon) = copy_message_icon(hwnd, ICON_BIG as usize) {
            return Some(icon);
        }
        for index in [GCLP_HICON, GCLP_HICONSM] {
            let icon = GetClassLongPtrW(hwnd, index);
            if icon != 0 {
                if let Ok(icon) = CopyIcon(HICON(icon as *mut c_void)) {
                    return Some(icon);
                }
            }
        }
        copy_message_icon(hwnd, ICON_SMALL as usize)
    }
}

unsafe fn copy_message_icon(hwnd: windows::Win32::Foundation::HWND, kind: usize) -> Option<HICON> {
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(kind),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            50,
            Some(&mut result),
        )
    };
    if sent.0 == 0 || result == 0 {
        return None;
    }
    unsafe { CopyIcon(HICON(result as *mut c_void)).ok() }
}

fn fallback_icon() -> HICON {
    unsafe {
        LoadIconW(None, IDI_APPLICATION)
            .ok()
            .and_then(|icon| CopyIcon(icon).ok())
            .unwrap_or_default()
    }
}

/// 把独立持有的图标画进矩形（居中，保持正方形）。
pub fn draw_icon(hdc: HDC, icon: &AppIcon, at: Rect) {
    if icon.0 == 0 {
        return;
    }
    let side = at.width().min(at.height()).max(1);
    let x = at.left + (at.width() - side) / 2;
    let y = at.top + (at.height() - side) / 2;
    unsafe {
        let _ = DrawIconEx(
            hdc,
            x,
            y,
            HICON(icon.0 as *mut c_void),
            side,
            side,
            0,
            HBRUSH(std::ptr::null_mut()),
            DI_NORMAL,
        );
    }
}
