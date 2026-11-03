//! 选区取词：UIA 优先，`Ctrl+C` 兜底。
//!
//! - UIA 只读，不碰键盘与剪贴板，因此在终端里不会取消选区；
//! - 终端或用户仍按着快捷键时不模拟复制，改为点工具条按钮后延迟取词；
//! - 真实输入、新请求或前台变化会取消旧请求，取消后不得再复制或还原剪贴板。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use whelper_common::geometry::Rect;
use whelper_common::win32::hook::KeyEvent;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    VIRTUAL_KEY,
};

use crate::{clipboard, uia, winutil};

/// 取词结果的最大长度：超过视为"不是一次划词"，不弹条。
pub const MAX_CAPTURE_CHARS: usize = 20_000;

const VK_CONTROL: u16 = 0x11;
const VK_C: u16 = 0x43;
static GENERATION: AtomicU64 = AtomicU64::new(0);
// 从备份到还原是一笔事务；不能让两次后台取词互相备份/恢复临时内容。
static CLIPBOARD_CAPTURE: Mutex<()> = Mutex::new(());
// 完整匹配专用标记；保留最低位以兼容其它组件对自身注入事件的过滤。
const COPY_INPUT_MARKER: usize = 0x5748_4351;
static COPY_INJECTION: Mutex<Option<CopyInjection>> = Mutex::new(None);
static COPY_HOOK_READY: AtomicBool = AtomicBool::new(false);

pub fn set_keyboard_hook_ready(ready: bool) {
    COPY_HOOK_READY.store(ready, Ordering::SeqCst);
    if !ready {
        cancel_pending();
    }
}

struct CopyInjection {
    request: Request,
    allowed: Option<bool>,
    remaining: u32,
}

impl CopyInjection {
    fn suppress(&mut self, is_current: bool) -> bool {
        // SendInput 的同批事件不会与真实输入交错：只在第一键决定整批放行，
        // 不能放行 Ctrl 按下后又吞掉其抬起，造成修饰键卡住。
        !*self.allowed.get_or_insert(is_current)
    }
}

/// 在输入真正进入钩子链时提交复制，而不是信任工作线程注入前的旧检查。
/// 已取消的自家整批事件可以丢弃，用户按键始终不在此处拦截。
pub fn filter_copy_input(event: &KeyEvent) -> Option<bool> {
    if !event.injected || event.extra_info != COPY_INPUT_MARKER {
        return None;
    }
    let mut pending = COPY_INJECTION.lock().unwrap_or_else(|e| e.into_inner());
    let Some(injection) = pending.as_mut() else {
        return Some(true);
    };
    let current = injection.request.is_current();
    let suppress = injection.suppress(current);
    injection.remaining -= 1;
    if injection.remaining == 0 {
        *pending = None;
    }
    Some(suppress)
}

/// 钩子必须同步取消，不可仅 PostMessage 后等待 UI 线程丢弃结果。
pub fn cancel_pending() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// 在派发线程前记录上下文，不能等后台 UIA/settle 结束后才开始监听取消。
#[derive(Clone, Copy)]
pub struct Request {
    generation: u64,
    target: isize,
    clipboard_sequence: u32,
}

impl Request {
    pub fn new(target: HWND) -> Self {
        Self {
            generation: GENERATION.load(Ordering::SeqCst),
            target: target.0 as isize,
            clipboard_sequence: clipboard::sequence_number(),
        }
    }

    pub fn is_current(&self) -> bool {
        self.generation == GENERATION.load(Ordering::SeqCst)
            && self.target != 0
            && winutil::foreground_window().0 as isize == self.target
    }
}

/// 一次取词的结果。
pub enum Outcome {
    /// 取到了文本；`bounds` 是 UIA 给出的选区外接矩形（Ctrl+C 路径没有）。
    Text { text: String, bounds: Option<Rect> },
    /// 不能安全地模拟 Ctrl+C，等用户点按钮时再取。
    Deferred,
    None,
}

/// 过滤掉空白、零宽字符、纯控制字符等"看起来没有内容"的选区。
pub fn is_plausible_selection(text: &str) -> bool {
    if text.chars().count() > MAX_CAPTURE_CHARS {
        return false;
    }
    text.chars().any(|ch| {
        let c = ch as u32;
        !(ch.is_whitespace()
            || ch.is_control()
            || c == 0x200B
            || c == 0x200C
            || c == 0x200D
            || c == 0xFEFF
            || c == 0x00AD)
    })
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYBD_EVENT_FLAGS(0x0002) } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: COPY_INPUT_MARKER,
            },
        },
    }
}

fn copy_keys_are_down(mut is_down: impl FnMut(i32) -> bool) -> bool {
    // SendInput 不会重置已有键盘状态。不要借用用户按住的 Ctrl，或把 Ctrl+C
    // 与用户的 Shift/Alt/Win/C 混合；短暂按下再抬起由钩子的 generation 捕获。
    [VK_CONTROL as i32, 0x10, 0x12, 0x5B, 0x5C, VK_C as i32]
        .into_iter()
        .any(&mut is_down)
}

fn send_ctrl_c(request: &Request) -> bool {
    {
        let mut pending = COPY_INJECTION.lock().unwrap_or_else(|e| e.into_inner());
        if pending.is_some() || !COPY_HOOK_READY.load(Ordering::SeqCst) || !request.is_current() {
            return false;
        }
        *pending = Some(CopyInjection { request: *request, allowed: None, remaining: 4 });
    }
    let inputs = [
        key(VK_CONTROL, false),
        key(VK_C, false),
        key(VK_C, true),
        key(VK_CONTROL, true),
    ];
    // 不能持锁调用 SendInput：它会同步回调安装钩子的 UI 线程。
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        let mut pending = COPY_INJECTION.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(injection) = pending.as_mut() {
            injection.remaining = injection.remaining.saturating_sub(inputs.len() as u32 - sent);
            if injection.remaining == 0 {
                *pending = None;
            }
        }
        return false;
    }
    true
}

/// 同步取词。UIA 调用本身可能阻塞，不要放在钩子回调里。
/// `timeout` 限制等待复制结果的时长；`settle` 等待源应用处理完选区操作。
pub fn capture_selection(
    settle: Duration,
    timeout: Duration,
    allow_keyboard: bool,
    anchor: POINT,
) -> Outcome {
    capture_selection_for(Request::new(winutil::foreground_window()), settle, timeout, allow_keyboard, anchor)
}

/// `request` 在派发任务时创建；`anchor` 必须保留拖选时的鼠标抬起点，不能用
/// 点击工具条后的光标位置，否则 UIA 的跨进程校验会拒绝取词。
pub fn capture_selection_for(
    request: Request,
    settle: Duration,
    timeout: Duration,
    allow_keyboard: bool,
    anchor: POINT,
) -> Outcome {
    if !settle.is_zero() {
        std::thread::sleep(settle);
    }
    if !request.is_current() {
        return Outcome::None;
    }

    // 1) UIA：只读选区。UIA 耗时期间用户可能已按 Ctrl+C 或切走窗口。
    let probe = uia::probe(HWND(request.target as *mut _), anchor);
    if !request.is_current() {
        return Outcome::None;
    }
    match probe {
        uia::Probe::Selection(selection) if is_plausible_selection(&selection.text) => {
            return Outcome::Text {
                text: selection.text,
                bounds: selection.bounds,
            };
        }
        uia::Probe::Rejected => return Outcome::None,
        _ => {}
    }

    if !allow_keyboard || copy_keys_are_down(|vk| unsafe { GetAsyncKeyState(vk) < 0 }) {
        return Outcome::Deferred;
    }

    capture_via_clipboard(&request, timeout)
}

pub(crate) fn capture_via_clipboard(request: &Request, timeout: Duration) -> Outcome {
    let Ok(_transaction) = CLIPBOARD_CAPTURE.try_lock() else {
        // 不排队执行过时的复制；正在结束的旧请求可能仍持有剪贴板。
        return Outcome::Deferred;
    };
    if !request.is_current() || clipboard::sequence_number() != request.clipboard_sequence {
        return Outcome::None;
    }
    let backup = clipboard::Backup::take();
    let Some(before) = backup.sequence() else {
        return Outcome::None;
    };
    if !request.is_current() || clipboard::sequence_number() != before {
        return Outcome::None;
    }
    // 备份可能等待剪贴板锁，注入前必须再检查一次按键状态。
    if copy_keys_are_down(|vk| unsafe { GetAsyncKeyState(vk) < 0 }) {
        return Outcome::Deferred;
    }
    if !send_ctrl_c(request) {
        return Outcome::None;
    }

    let start = Instant::now();
    while start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(10));
        if !request.is_current() {
            // 用户复制优先：即使内容与本次取词完全相同也不能恢复旧备份。
            return Outcome::None;
        }
        if clipboard::sequence_number() != before {
            let captured = clipboard::get_text_with_sequence(|| request.is_current());
            let Some((text, sequence)) = captured else {
                return Outcome::None;
            };
            if !request.is_current() {
                return Outcome::None;
            }
            // 只恢复本次实际读到的版本，不覆盖随后发生的新复制。
            backup.restore_if_unchanged(sequence, || request.is_current());
            return match text {
                Some(text) if request.is_current() && is_plausible_selection(&text) => {
                    Outcome::Text { text, bounds: None }
                }
                _ => Outcome::None,
            };
        }
    }
    // 没观察到复制结果（超时/UIPI/无选区）时没有可安全还原的版本。
    Outcome::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_and_invisible_selections() {
        assert!(!is_plausible_selection(""));
        assert!(!is_plausible_selection("   \n\t\r\n"));
        assert!(!is_plausible_selection("\u{200b}\u{200c}\u{feff}"));
        assert!(is_plausible_selection("a"));
        assert!(is_plausible_selection("  划词  "));
        assert!(is_plausible_selection("한국어 텍스트"));
    }

    #[test]
    fn rejects_absurdly_long_selections() {
        let huge = "a".repeat(MAX_CAPTURE_CHARS + 1);
        assert!(!is_plausible_selection(&huge));
    }

    #[test]
    fn held_copy_keys_and_modifiers_prevent_injection() {
        for held in [0x11, 0x10, 0x12, 0x5B, 0x5C, 0x43] {
            assert!(copy_keys_are_down(|vk| vk == held));
        }
        assert!(!copy_keys_are_down(|_| false));
        assert!(!copy_keys_are_down(|vk| vk == 0x41));
    }

    #[test]
    fn cancelled_request_stops_before_uia_or_clipboard_fallback() {
        let request = Request::new(winutil::foreground_window());
        cancel_pending();
        assert_ne!(request.generation, GENERATION.load(Ordering::SeqCst));
        assert!(!request.is_current());
        assert!(matches!(
            capture_selection_for(request, Duration::ZERO, Duration::ZERO, true, POINT::default()),
            Outcome::None
        ));
    }

    #[test]
    fn cancellation_before_first_injected_key_rejects_the_entire_batch() {
        let mut injection = CopyInjection {
            request: Request::new(winutil::null_hwnd()),
            allowed: None,
            remaining: 4,
        };
        assert!(injection.suppress(false));
        // 取消后即使前台恢复也不能放行后半批按键。
        assert!(injection.suppress(true));
        assert!(injection.suppress(true));
        assert!(injection.suppress(true));
    }

    #[test]
    fn accepted_batch_keeps_keyups_and_external_markers_are_not_ours() {
        let mut injection = CopyInjection {
            request: Request::new(winutil::null_hwnd()),
            allowed: None,
            remaining: 4,
        };
        assert!(!injection.suppress(true));
        assert!(!injection.suppress(false));
        for extra_info in [0, 1, 3, 0xFFFF] {
            assert!(filter_copy_input(&KeyEvent {
                vk: VK_C as u32, is_down: true, injected: true, extra_info,
            }).is_none());
        }
    }

    #[test]
    fn copy_injection_is_identifiable_by_the_hook() {
        unsafe {
            for input in [key(VK_CONTROL, false), key(VK_C, true)] {
                assert_eq!(input.Anonymous.ki.dwExtraInfo, COPY_INPUT_MARKER);
            }
        }
    }
}
