//! 常驻形态：在当前进程的主线程上创建消息窗口，并往共享键盘钩子链上注册 Win+D。
//!
//! 这里**不跑消息循环**：窗口由宿主（`whelper`）与取词组件共用的主循环 `DispatchMessage`
//! 驱动。触发路径是「钩子吞键 → `PostMessage(MSG_TOGGLE)` → 本窗口的 wndproc → 切换」，
//! 钩子回调里不做任何窗口操作。

use std::ffi::c_void;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
    SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, HMENU, HWND_MESSAGE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_NCCREATE, WNDCLASSW, WS_POPUP,
};
use whelper_common::win32::string::{to_wide, wide_ptr};

use crate::backend::win32::Win32Backend;
use crate::shortcut;
use crate::ShowDesktop;

const CLASS_NAME: &str = "WHelperShowDesktop";
const WINDOW_TITLE: &str = "whelper show-desktop";

/// 常驻服务句柄：持有控制器与消息窗口，析构即注销钩子、销毁窗口。
pub struct Service {
    hwnd: HWND,
    /// 控制器地址稳定（Box），wndproc 通过 `GWLP_USERDATA` 取回；这里只负责生命周期。
    #[allow(dead_code)]
    controller: Box<ShowDesktop<Win32Backend>>,
}

impl Service {
    /// 创建消息窗口并接管 Win+D。返回 `None` 表示窗口或钩子创建失败。
    pub fn start() -> Option<Self> {
        let mut controller = Box::new(ShowDesktop::new(Win32Backend::new()));
        let context = &mut *controller as *mut ShowDesktop<Win32Backend> as *const c_void;

        let hwnd = create_message_window(context)?;
        if !shortcut::install(hwnd) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }

        Some(Self { hwnd, controller })
    }

    /// 消息窗口句柄，仅用于诊断输出。
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // 先摘钩子，避免销毁窗口后仍有 MSG_TOGGLE 投递进来。
        shortcut::uninstall();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
        // `controller` 在字段析构时释放。
    }
}

fn create_message_window(context: *const c_void) -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let hinstance = HINSTANCE(module.0);
        let class = to_wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: wide_ptr(&class),
            ..Default::default()
        };
        // 重复注册只是「类已存在」，不影响后续 CreateWindowExW。
        let _ = RegisterClassW(&wc);

        let title = to_wide(WINDOW_TITLE);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            wide_ptr(&class),
            wide_ptr(&title),
            WINDOW_STYLE(WS_POPUP.0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            HMENU(std::ptr::null_mut()),
            hinstance,
            Some(context),
        )
        .ok()
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return LRESULT(1);
    }

    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ShowDesktop<Win32Backend>;
    if !raw.is_null() && message == shortcut::MSG_TOGGLE {
        toggle(&mut *raw);
        return LRESULT(0);
    }

    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn toggle(controller: &mut ShowDesktop<Win32Backend>) {
    match controller.toggle_at_cursor() {
        Ok(report) => {
            let debug = std::env::var_os("SHOW_DESKTOP_DEBUG").is_some()
                || whelper_config::get().general.debug;
            if debug {
                eprintln!(
                    "[desktop] {} {} 个窗口（候选 {} 个）",
                    crate::app::action_label(report.action),
                    report.affected,
                    report.considered
                );
            }
        }
        Err(error) => eprintln!("[warn] 显示桌面失败: {error}"),
    }
}
