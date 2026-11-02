//! `--self-test`：在真实桌面上验证"模拟 Ctrl+C → 读剪贴板 → 还原剪贴板"。
//!
//! 创建一个含 EDIT 控件的窗口并全选文本，然后走与生产完全相同的取词链路。
//! 需要交互式桌面会话；无桌面（服务/会话 0）时会失败并明确报告。

use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetWindowRect, PeekMessageW,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage,
    ES_AUTOHSCROLL, ES_LEFT, ES_MULTILINE, HMENU, MSG, PM_REMOVE, SW_SHOW, WINDOW_STYLE, WNDCLASSW,
    WS_BORDER, WS_CHILD, WS_EX_OVERLAPPEDWINDOW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::capture;
use crate::clipboard;
use crate::winutil;

const EDIT_ID: usize = 1001;
const EM_SETSEL: u32 = 0x00B1;

unsafe extern "system" fn test_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn pump(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < deadline {
            while PeekMessageW(&mut msg, winutil::null_hwnd(), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn run() -> i32 {
    const SAMPLE: &str = "text-picker self-test：Hello, whelper!";
    const SENTINEL: &str = "SENTINEL-剪贴板-必须被还原";

    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(_) => {
                eprintln!("FAIL 无法获取模块句柄");
                return 2;
            }
        };
        let hinstance = HINSTANCE(module.0);
        let class = winutil::to_wide("TextPickerSelfTest");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(test_wndproc),
            hInstance: hinstance,
            lpszClassName: winutil::wide_ptr(&class),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let title = winutil::to_wide("text-picker self-test");
        let hwnd = match CreateWindowExW(
            WS_EX_OVERLAPPEDWINDOW,
            winutil::wide_ptr(&class),
            winutil::wide_ptr(&title),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            120,
            120,
            460,
            220,
            winutil::null_hwnd(),
            HMENU(std::ptr::null_mut()),
            hinstance,
            None,
        ) {
            Ok(h) => h,
            Err(_) => {
                eprintln!("FAIL 无法创建测试窗口（是否没有交互式桌面？）");
                return 2;
            }
        };

        let edit_class = winutil::to_wide("EDIT");
        let edit = match CreateWindowExW(
            Default::default(),
            winutil::wide_ptr(&edit_class),
            PCWSTR::null(),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | (ES_LEFT | ES_MULTILINE | ES_AUTOHSCROLL) as u32),
            12,
            12,
            420,
            150,
            hwnd,
            HMENU(EDIT_ID as *mut std::ffi::c_void),
            hinstance,
            None,
        ) {
            Ok(h) => h,
            Err(_) => {
                eprintln!("FAIL 无法创建 EDIT 控件");
                let _ = DestroyWindow(hwnd);
                return 2;
            }
        };

        let sample = winutil::to_wide(SAMPLE);
        let _ = SetWindowTextW(edit, winutil::wide_ptr(&sample));
        SendMessageW(edit, EM_SETSEL, WPARAM(0), LPARAM(-1));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        winutil::force_foreground(hwnd);
        let _ = SetFocus(edit);
        pump(300);

        println!("[self-test] 已全选文本：{SAMPLE}");

        // 备份哨兵内容，验证取词后会还原用户剪贴板。
        clipboard::set_text(SENTINEL);
        let before = clipboard::get_text();

        // 取词锚点必须落在被选中的控件上：UIA 用它做“焦点/命中元素同进程”校验。
        let mut edit_rect = RECT::default();
        let _ = GetWindowRect(edit, &mut edit_rect);
        let anchor = POINT {
            x: (edit_rect.left + edit_rect.right) / 2,
            y: (edit_rect.top + edit_rect.bottom) / 2,
        };

        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            capture::capture_selection(Duration::ZERO, Duration::from_millis(500), true, anchor)
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !worker.is_finished() {
            pump(30);
        }
        let captured = worker.join().ok().and_then(|outcome| match outcome {
            capture::Outcome::Text { text, .. } => Some(text),
            _ => None,
        });

        let after = clipboard::get_text();
        let mut failures = 0;

        match captured.as_deref() {
            Some(text) if text == SAMPLE => {
                println!("PASS 取词成功：{text}");
            }
            Some(text) => {
                eprintln!("FAIL 取词内容不匹配：得到 {text:?}，期望 {SAMPLE:?}");
                failures += 1;
            }
            None => {
                eprintln!("FAIL 未取到文本（桌面焦点/剪贴板访问失败）");
                failures += 1;
            }
        }

        if before.as_deref() == Some(SENTINEL) && after.as_deref() == Some(SENTINEL) {
            println!("PASS 剪贴板已恢复到取词前的内容");
        } else {
            eprintln!("FAIL 剪贴板未还原：before={before:?} after={after:?}");
            failures += 1;
        }

        let _ = DestroyWindow(hwnd);
        pump(50);

        if failures == 0 {
            println!("SELF-TEST PASS");
            0
        } else {
            println!("SELF-TEST FAIL ({failures})");
            1
        }
    }
}
