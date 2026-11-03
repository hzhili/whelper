//! `--self-test`：在真实桌面上验证"模拟 Ctrl+C → 读剪贴板 → 还原剪贴板"。
//!
//! 创建一个含 EDIT 控件的窗口并全选文本，然后走与生产完全相同的取词链路。
//! 需要交互式桌面会话；无桌面（服务/会话 0）时会失败并明确报告。

use std::sync::atomic::{AtomicBool, Ordering};
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
use crate::hooks;
use crate::winutil;

const EDIT_ID: usize = 1001;
const EM_SETSEL: u32 = 0x00B1;
const USER_COPY: &str = "SELF-TEST-用户的新复制不能被旧备份覆盖";
static SIMULATE_USER_COPY: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn test_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == 0x0102 && wparam.0 == 3 && SIMULATE_USER_COPY.load(Ordering::SeqCst) {
        // 真实 Ctrl+C 已到达目标窗口、后台还在等剪贴板：在这个确定的时点
        // 模拟用户输入使请求取消，随后应用写入用户复制结果，复现旧版覆盖竞态。
        capture::cancel_pending();
        clipboard::set_text(USER_COPY);
        return LRESULT(0);
    }
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

        if winutil::foreground_window() != hwnd {
            eprintln!("FAIL 自测窗口未获得前台焦点（请在交互式桌面运行）");
            let _ = DestroyWindow(hwnd);
            return 1;
        }
        println!("[self-test] 已全选文本：{SAMPLE}");

        // 保存测试前的剪贴板，测试完成后不留下哨兵内容。
        let original = clipboard::Backup::take();
        if original.sequence().is_none() {
            eprintln!("FAIL 无法备份测试前的剪贴板");
            let _ = DestroyWindow(hwnd);
            return 1;
        }
        let hooks_installed = hooks::install(hwnd);
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

        let request = capture::Request::new(hwnd);
        let worker = std::thread::spawn(move || {
            capture::capture_selection_for(request, Duration::from_millis(120), Duration::from_millis(500), true, anchor)
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

        if !hooks_installed {
            eprintln!("FAIL 无法安装自测键盘/鼠标钩子");
            failures += 1;
        }

        // EDIT 的 UIA 往往成功，因此上面的测试不一定走过剪贴板兜底。
        // 强制调用生产兜底路径，验证正常还原、复制期间取消和无复制结果超时。
        for (interrupted, no_copy) in [(false, false), (true, false), (false, true)] {
            SIMULATE_USER_COPY.store(interrupted, Ordering::SeqCst);
            let _ = SetFocus(if interrupted || no_copy { hwnd } else { edit });
            if !clipboard::set_text(SENTINEL) {
                eprintln!("FAIL 无法写入测试哨兵");
                failures += 1;
                break;
            }
            let before_sequence = clipboard::sequence_number();
            let request = capture::Request::new(hwnd);
            let worker = std::thread::spawn(move || {
                capture::capture_via_clipboard(&request, Duration::from_millis(500))
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !worker.is_finished() {
                pump(10);
            }
            let result = worker.join().ok();
            let content = clipboard::get_text();
            let ok = if no_copy {
                matches!(result, Some(capture::Outcome::None))
                    && clipboard::sequence_number() == before_sequence
            } else if interrupted {
                matches!(result, Some(capture::Outcome::None))
                    && content.as_deref() == Some(USER_COPY)
            } else {
                matches!(result, Some(capture::Outcome::Text { text, .. }) if text == SAMPLE)
                    && content.as_deref() == Some(SENTINEL)
            };
            let label = if no_copy {
                "复制超时不改写剪贴板"
            } else if interrupted {
                "取消取词后保留用户的新复制"
            } else {
                "Ctrl+C 兜底取词并按版本还原"
            };
            if ok {
                println!("PASS {label}");
            } else {
                eprintln!("FAIL {label}：clipboard={content:?}");
                failures += 1;
            }
        }
        SIMULATE_USER_COPY.store(false, Ordering::SeqCst);
        hooks::uninstall();
        if let Some((text, sequence)) = clipboard::get_text_with_sequence(|| true) {
            if matches!(text.as_deref(), Some(SENTINEL) | Some(USER_COPY)) {
                original.restore_if_unchanged(sequence, || true);
            }
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
