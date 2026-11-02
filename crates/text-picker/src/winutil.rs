//! text-picker 自己的 Win32 辅助。
//!
//! 跨组件共用的能力（宽字符串、显示器归属/工作区、单实例、键盘钩子）已下沉到
//! `whelper-common`；这里只保留取词与工具条特有的东西，并重新导出通用项，
//! 让本 crate 的调用方仍然写 `winutil::to_wide(...)`，不必关心实现搬到了哪里。

use whelper_common::geometry::Point;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
    SetForegroundWindow,
};

pub use whelper_common::win32::null_hwnd;
pub use whelper_common::win32::string::{to_wide, wide_ptr};
/// 深浅色主题查询已下沉到 `whelper-common`（Alt+Tab 切换器同样跟随系统主题）。
pub use whelper_common::win32::theme::apps_use_light_theme;

/// 终端窗口类：复制即“取消选区”，不能自动模拟 Ctrl+C。
const TERMINAL_CLASSES: &[&str] = &[
    // conhost：cmd / powershell 的经典控制台
    "consolewindowclass",
    // Windows Terminal
    "cascadia_hosting_window_class",
];

/// IDE 宿主进程：终端是内嵌控件，窗口类与编辑器相同，只能按进程判断。
const IDE_HOSTS: &[&str] = &[
    "code.exe",
    "code - insiders.exe",
    "codebuddy cn.exe",
    "codebuddy.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "goland64.exe",
    "clion64.exe",
    "datagrip64.exe",
    "rider64.exe",
    "studio64.exe",
    "devenv.exe",
    "sublime_text.exe",
];

/// 当前光标位置（屏幕坐标）。
pub fn cursor_position() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

/// 该窗口是否属于“复制会取消选区”的终端类目标。
pub fn prefers_deferred_capture(hwnd: HWND) -> bool {
    is_deferred_target(&window_class(hwnd), process_name(hwnd).as_deref())
}

/// 分类逻辑独立出来，便于单测（`window_class` / `process_name` 需要真实窗口）。
fn is_deferred_target(class: &str, process: Option<&str>) -> bool {
    let class = class.to_ascii_lowercase();
    if TERMINAL_CLASSES.contains(&class.as_str()) {
        return true;
    }
    process.is_some_and(|name| IDE_HOSTS.contains(&name.to_ascii_lowercase().as_str()))
}

/// 窗口类名（失败返回空串）。
pub fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

/// 窗口所属进程的可执行文件名（小写，含扩展名）。
pub fn process_name(hwnd: HWND) -> Option<String> {
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 {
        return None;
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size)
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    if queried.is_err() {
        return None;
    }

    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    path.rsplit(['\\', '/'])
        .next()
        .map(|name| name.to_ascii_lowercase())
}

/// 取得某点所在显示器的工作区（已排除任务栏）。失败时退化为主屏。
pub fn work_area(pt: POINT) -> RECT {
    match whelper_common::win32::monitor::work_area(Point::new(pt.x, pt.y)) {
        Some(rect) => RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        None => RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
    }
}

/// 当前前台窗口。
pub fn foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

/// 把目标窗口强行带到前台并获得焦点（用于 `--self-test` 与演示）。
pub fn force_foreground(hwnd: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        if fg == hwnd {
            let _ = SetFocus(hwnd);
            return;
        }
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let my_tid = GetCurrentThreadId();
        let attached = AttachThreadInput(my_tid, fg_tid, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
    }
}

/// 判断 HWND 是否仍然有效。
pub fn is_window(hwnd: HWND) -> bool {
    unsafe { IsWindow(hwnd).as_bool() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_classes_are_deferred() {
        assert!(is_deferred_target("ConsoleWindowClass", None));
        assert!(is_deferred_target("CASCADIA_HOSTING_WINDOW_CLASS", Some("WindowsTerminal.exe")));
        assert!(!is_deferred_target("Notepad", Some("notepad.exe")));
        assert!(!is_deferred_target("", None));
    }

    #[test]
    fn ide_hosts_are_deferred_case_insensitively() {
        assert!(is_deferred_target("Chrome_WidgetWin_1", Some("CodeBuddy CN.exe")));
        assert!(is_deferred_target("SunAwtFrame", Some("idea64.exe")));
        assert!(!is_deferred_target("Chrome_WidgetWin_1", Some("chrome.exe")));
    }
}
