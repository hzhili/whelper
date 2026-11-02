//! whelper 的桌面外壳：托盘图标与菜单、设置页入口、开机自启。
//!
//! 在宿主主线程创建一个**隐藏的顶层窗口**：
//!
//! - 托盘图标的回调落在这里（左键打开设置、右键弹菜单）；
//! - `whelper --settings` 通过 [`whelper_common::win32::applink`] 往这个窗口投递
//!   [`applink::MSG_OPEN_SETTINGS`]，已常驻的实例会把设置页拉到前台；
//! - 设置窗口按需创建、关闭时只隐藏，从托盘再次打开是瞬时的。
//!
//! 与其它组件一样，本模块**不跑消息循环**：窗口由宿主的主循环 `DispatchMessage` 驱动。

mod autostart;
mod icons;
mod plan;
mod settings;

use std::ffi::c_void;
use std::mem::size_of;

use whelper_common::win32::string::{to_wide, wide_ptr};
use whelper_common::win32::{applink, icon, null_hwnd};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    SetCurrentProcessExplicitAppUserModelID, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP,
    NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, TrackPopupMenu, CREATESTRUCTW,
    GWLP_USERDATA, HICON, HMENU, IDC_ARROW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCCREATE, WM_NULL,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

pub use settings::run_standalone;

const CLASS_NAME: &str = applink::SHELL_CLASS;
const WINDOW_TITLE: &str = "whelper";
const TRAY_ID: u32 = 1;
/// 托盘图标回调消息（`lparam` 低 16 位是鼠标消息）。
const TRAY_CALLBACK: u32 = WM_APP + 61;
const TIMER_TRAY_RETRY: usize = 1;
const TRAY_RETRY_MS: u32 = 2000;
const MAX_TRAY_RETRIES: u8 = 5;

const MENU_OPEN: usize = 1;
const MENU_AUTOSTART: usize = 2;
const MENU_QUIT: usize = 3;

/// 常驻外壳：托盘图标 + 设置窗口的所有者。
pub struct Shell {
    hwnd: HWND,
    settings: Option<Box<settings::Settings>>,
    icon_large: Option<HICON>,
    icon_small: Option<HICON>,
    tray_added: bool,
    tray_retries: u8,
}

impl Shell {
    /// 创建隐藏外壳窗口并挂上托盘图标。返回 `None` 表示窗口创建失败。
    pub fn start() -> Option<Box<Self>> {
        unsafe {
            let _ = SetCurrentProcessExplicitAppUserModelID(w!("whelper.desktop"));
        }

        let mut shell = Box::new(Self {
            hwnd: null_hwnd(),
            settings: None,
            icon_large: icons::large(),
            icon_small: icons::small(),
            tray_added: false,
            tray_retries: 0,
        });
        let context = &mut *shell as *mut Self as *const c_void;
        shell.hwnd = create_window(context)?;
        shell.add_tray();
        Some(shell)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 打开（或拉到前台）设置页。
    pub fn open_settings(&mut self) {
        if self.settings.is_none() {
            self.settings = settings::Settings::create(self.icon_large, self.icon_small);
            if self.settings.is_none() {
                eprintln!("[warn] 设置窗口创建失败");
                return;
            }
        }
        if let Some(settings) = self.settings.as_deref_mut() {
            settings.show();
        }
    }

    fn add_tray(&mut self) -> bool {
        let Some(tray_icon) = self.icon_small.or(self.icon_large) else {
            return false;
        };
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK,
            hIcon: tray_icon,
            ..Default::default()
        };
        set_tip(&mut data.szTip, "whelper · 划词 / 显示桌面 / Alt+Tab");

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        if whelper_config::get().general.debug {
            eprintln!("[shell] 托盘图标：{}", if added { "已添加" } else { "添加失败，稍后重试" });
        }
        self.tray_added = added;
        if !added && self.tray_retries < MAX_TRAY_RETRIES {
            // 开机自启时 explorer 可能还没准备好接收托盘图标，稍后重试。
            unsafe {
                let _ = SetTimer(self.hwnd, TIMER_TRAY_RETRY, TRAY_RETRY_MS, None);
            }
        }
        added
    }

    fn remove_tray(&mut self) {
        if !self.tray_added {
            return;
        }
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        self.tray_added = false;
    }

    fn on_tray(&mut self, lparam: LPARAM) {
        let event = lparam.0 as u32 & 0xFFFF;
        match event {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => self.open_settings(),
            WM_RBUTTONUP | WM_CONTEXTMENU => self.show_menu(),
            _ => {}
        }
    }

    /// 退出：优先让工具条窗口走 `WM_CLOSE`（正常摘钩子），否则直接结束消息循环。
    fn quit(&self) {
        if !applink::close_toolbar() {
            unsafe {
                PostQuitMessage(0);
            }
        }
    }

    fn show_menu(&mut self) {
        let autostart = autostart::is_enabled();
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("打开设置"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let check = if autostart { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(menu, MF_STRING | check, MENU_AUTOSTART, w!("开机自启"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("退出 whelper"));

            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            // 菜单要在“前台”窗口上弹出，否则点空白处不会收起。
            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                0,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);

            match command.0 as usize {
                MENU_OPEN => self.open_settings(),
                MENU_AUTOSTART => {
                    let enabled = !autostart;
                    if autostart::set_enabled(enabled) {
                        if let Some(settings) = self.settings.as_deref_mut() {
                            settings.refresh();
                        }
                    }
                }
                MENU_QUIT => self.quit(),
                _ => {}
            }
        }
    }

    unsafe fn on_message(
        &mut self,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match message {
            applink::MSG_OPEN_SETTINGS => {
                self.open_settings();
                Some(LRESULT(0))
            }
            TRAY_CALLBACK => {
                self.on_tray(lparam);
                Some(LRESULT(0))
            }
            WM_TIMER => {
                if wparam.0 == TIMER_TRAY_RETRY {
                    let _ = KillTimer(self.hwnd, TIMER_TRAY_RETRY);
                    if !self.tray_added {
                        self.tray_retries += 1;
                        self.add_tray();
                    }
                }
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                let _ = DestroyWindow(self.hwnd);
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.remove_tray();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.remove_tray();
        // Settings::drop 负责销毁设置窗口与字体。
        self.settings = None;
        if let Some(icon) = self.icon_large.take() {
            icon::destroy(icon);
        }
        if let Some(icon) = self.icon_small.take() {
            icon::destroy(icon);
        }
        if !self.hwnd.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 给已常驻的实例发“打开设置”请求；没有实例时返回 `false`。
pub fn open_settings_in_running_instance() -> bool {
    applink::open_settings()
}

/// `--icon-report`：确认嵌入的图标能被系统解析。
pub fn icon_report() -> i32 {
    icons::report()
}

fn set_tip(tip: &mut [u16; 128], text: &str) {
    // 默认全 0（已经以 NUL 结尾），按字符覆盖即可。
    for (slot, unit) in tip.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
    }
}

fn create_window(context: *const c_void) -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let hinstance = HINSTANCE(module.0);
        let class = to_wide(CLASS_NAME);
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hCursor: cursor,
            lpszClassName: wide_ptr(&class),
            ..Default::default()
        };
        // 重复注册只是“类已存在”。
        let _ = RegisterClassW(&wc);

        let title = to_wide(WINDOW_TITLE);
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            wide_ptr(&class),
            wide_ptr(&title),
            WINDOW_STYLE(WS_POPUP.0),
            0,
            0,
            0,
            0,
            null_hwnd(),
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
        let shell = create.lpCreateParams as *mut Shell;
        if !shell.is_null() {
            (*shell).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, shell as isize);
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Shell;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    match (*raw).on_message(message, wparam, lparam) {
        Some(result) => result,
        None => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
