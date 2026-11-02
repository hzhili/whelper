//! 顶层窗口的枚举与常用操作。
//!
//! [`alt_tab_windows`] 是 `show-desktop` 与 `window-switcher` **共用**的候选集合：
//! 两者都直接按 `EnumWindows` 的自然顺序（Z 序自上而下）使用它，
//! 因此候选集合与顺序天然一致。纯判定规则在 [`crate::window`]，这里只做 Win32 调用。

use std::ffi::c_void;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetForegroundWindow, GetLastActivePopup, GetWindowLongPtrW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PeekMessageW,
    SetForegroundWindow, ShowWindow, SwitchToThisWindow, GA_ROOTOWNER, GWL_EXSTYLE, GWL_STYLE, MSG,
    PM_NOREMOVE, SW_RESTORE,
};

use crate::window::{classify, state_from_style, WindowFlags, WindowId, WindowKind, WindowState};

/// `WindowId` → `HWND`。
pub fn hwnd(window: WindowId) -> HWND {
    HWND(window.0 as *mut c_void)
}

/// 按 Z 序（自上而下）返回参与 Alt+Tab 的应用窗口。
///
/// `EnumWindows` 只枚举顶层窗口；即便中途有窗口销毁，返回值也不影响已收集的清单。
pub fn alt_tab_windows() -> Vec<WindowId> {
    let mut windows: Vec<WindowId> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(&mut windows as *mut Vec<WindowId> as isize),
        );
    }
    windows
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<WindowId>);
    if classify(flags(hwnd)) == WindowKind::AppWindow {
        windows.push(WindowId(hwnd.0 as isize));
    }
    // 必须继续枚举：返回 FALSE 会中断整个遍历。
    TRUE
}

fn flags(hwnd: HWND) -> WindowFlags {
    unsafe {
        WindowFlags {
            style: GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
            ex_style: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32,
            cloaked: is_cloaked(hwnd),
            alt_tab: is_alt_tab_window(hwnd),
        }
    }
}

fn is_cloaked(hwnd: HWND) -> bool {
    // DWMWA_CLOAKED 是 4 字节 BOOL；上游用 `out bool`（同样 4 字节）。
    let mut cloaked: u32 = 0;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    result.is_ok() && cloaked != 0
}

/// 上游 `IsAltTabWindow`：以根所有者（`GetAncestor(hwnd, 3)`）为起点，
/// 取最后一个可见的活动弹出窗口，若就是自己则视为 Alt+Tab 窗口。
fn is_alt_tab_window(hwnd: HWND) -> bool {
    let root = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
    last_active_visible_popup(root) == hwnd
}

fn last_active_visible_popup(root: HWND) -> HWND {
    let null = HWND(std::ptr::null_mut());
    let mut walked = null;
    let mut current = root;
    while walked != current {
        walked = current;
        current = unsafe { GetLastActivePopup(walked) };
        if unsafe { IsWindowVisible(current).as_bool() } {
            return current;
        }
    }
    null
}

/// 窗口标题。空标题（只有图标的窗口）返回空串，由调用方决定显示什么。
pub fn title(window: WindowId) -> String {
    // 256 个 UTF-16 单元足够 UI 与日志使用；超长标题按截断处理，不做二次分配。
    let mut buffer = [0u16; 256];
    let length = unsafe { GetWindowTextW(hwnd(window), &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    let length = (length as usize).min(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

/// 窗口当前状态（可见 / 最小化 / 不可见）。
pub fn state(window: WindowId) -> WindowState {
    unsafe { state_from_style(GetWindowLongPtrW(hwnd(window), GWL_STYLE) as u32) }
}

/// 窗口是否最小化。
pub fn is_minimized(window: WindowId) -> bool {
    unsafe { IsIconic(hwnd(window)).as_bool() }
}

/// 把窗口带到前台并拿到焦点，返回最终是否真的成为前台窗口。
///
/// 三个必须处理的细节：
/// 1. **最小化窗口**：`SetForegroundWindow` 不会还原它，先 `ShowWindow(SW_RESTORE)`；
/// 2. **前台锁**：只有当进程拥有前台窗口或收到用户输入时才允许抢占前台，
///    因此按 `text-picker` 的做法临时 `AttachThreadInput` 到当前前台线程；
/// 3. **切换是异步的**：`SetForegroundWindow` 返回时前台窗口往往还没更新，
///    立即比较会误判失败，需要短暂轮询；仍失败再退到系统 Alt+Tab 自己用的
///    `SwitchToThisWindow`（未文档化，但能绕过前台锁）；
/// 4. **程序化切换会让前台锁重新生效**：连续第二次 `SetForegroundWindow` 会被拒绝，
///    因此重试前用 [`keyboard::send_dummy_key`] 注入一次无副作用的输入，重新取得前台资格。
///
/// 提权窗口受 UIPI 限制仍然可能失败，此时返回 `false`（调用方负责提示）。
pub fn activate(window: WindowId) -> bool {
    let target = hwnd(window);
    if !unsafe { IsWindow(target).as_bool() } {
        return false;
    }

    if is_minimized(window) {
        unsafe {
            let _ = ShowWindow(target, SW_RESTORE);
        }
    }

    if try_foreground(target, Duration::from_millis(80)) {
        return true;
    }
    super::keyboard::send_dummy_key();
    if try_foreground(target, Duration::from_millis(120)) {
        return true;
    }
    unsafe { SwitchToThisWindow(target, true) };
    wait_for_foreground(target, Duration::from_millis(120))
}

/// 附加到当前前台线程后调用 `SetForegroundWindow`，再短暂等待前台窗口真正切换。
fn try_foreground(target: HWND, timeout: Duration) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == target {
            let _ = SetFocus(target);
            return true;
        }

        // AttachThreadInput 要求调用线程已有消息队列；独立命令（自检/临时进程）的主线程
        // 可能从未创建过窗口，先 PeekMessage 一次把队列建起来。
        let mut message = MSG::default();
        let _ = PeekMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0, PM_NOREMOVE);

        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let current_thread = GetCurrentThreadId();
        let attached = AttachThreadInput(current_thread, foreground_thread, true).as_bool();
        let _ = SetForegroundWindow(target);
        if attached {
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
        let _ = SetFocus(target);
    }
    wait_for_foreground(target, timeout)
}

/// 轮询前台窗口，直到变成 `target` 或超时。
fn wait_for_foreground(target: HWND, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if unsafe { GetForegroundWindow() } == target {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
