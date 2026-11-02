//! 选区取词：UIA 优先，`Ctrl+C` 兜底。
//!
//! - UIA 只读，不碰键盘与剪贴板，因此在终端里不会取消选区；
//! - 终端把 `Ctrl+C` 定义成“复制并取消选区”，所以对终端类窗口**不自动复制**，
//!   改为 `Deferred`（工具条先出现，用户点按钮时再取词）。

use std::time::{Duration, Instant};

use whelper_common::geometry::Rect;
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    VIRTUAL_KEY,
};

use crate::{clipboard, uia, winutil};

/// 取词结果的最大长度：超过视为"不是一次划词"，不弹条。
pub const MAX_CAPTURE_CHARS: usize = 20_000;

const VK_CONTROL: i32 = 0x11;
const VK_C: u16 = 0x43;

/// 一次取词的结果。
pub enum Outcome {
    /// 取到了文本；`bounds` 是 UIA 给出的选区外接矩形（Ctrl+C 路径没有）。
    Text { text: String, bounds: Option<Rect> },
    /// 目标窗口不能安全地模拟 Ctrl+C（终端复制会取消选区），等用户点按钮时再取。
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
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_ctrl_c() {
    unsafe {
        let ctrl_down = GetAsyncKeyState(VK_CONTROL) < 0;
        let inputs: Vec<INPUT> = if ctrl_down {
            // 用户正按着 Ctrl（例如 Ctrl+A 触发）：只补一个 C，避免破坏按键状态。
            vec![key(VK_C, false), key(VK_C, true)]
        } else {
            vec![
                key(VK_CONTROL as u16, false),
                key(VK_C, false),
                key(VK_C, true),
                key(VK_CONTROL as u16, true),
            ]
        };
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// 同步取词。调用线程会被阻塞最多 `timeout`，不要放在钩子回调里。
///
/// `settle` 用于等待源应用处理完鼠标/按键抬起；`allow_keyboard == false` 时
/// 不会发送 Ctrl+C（终端类窗口），UIA 拿不到就返回 [`Outcome::Deferred`]。
///
/// `anchor` 必须是**拖选时的鼠标抬起点**，而不是调用时的光标位置：
/// 延迟取词（点工具条按钮）时鼠标已经移到工具条上，用当前光标会被
/// UIA 的“焦点/命中元素跨进程”规则误判成拒绝。
pub fn capture_selection(
    settle: Duration,
    timeout: Duration,
    allow_keyboard: bool,
    anchor: POINT,
) -> Outcome {
    if !settle.is_zero() {
        std::thread::sleep(settle);
    }

    // 1) UIA：只读选区，终端不会因此清空选中状态。
    match uia::probe(winutil::foreground_window(), anchor) {
        uia::Probe::Selection(selection) if is_plausible_selection(&selection.text) => {
            return Outcome::Text {
                text: selection.text,
                bounds: selection.bounds,
            };
        }
        // 密码框、焦点与命中元素跨进程：直接不弹，也不要回退复制。
        uia::Probe::Rejected => return Outcome::None,
        _ => {}
    }

    // 2) 终端类窗口不能走复制：Ctrl+C 会“复制并取消选区”。
    if !allow_keyboard {
        return Outcome::Deferred;
    }

    // 3) 其它应用：沿用 Ctrl+C + 剪贴板备份/还原。
    let backup = clipboard::Backup::take();
    let before = clipboard::sequence_number();

    send_ctrl_c();

    let start = Instant::now();
    let mut captured = None;
    while start.elapsed() < timeout {
        unsafe { Sleep(10) };
        if clipboard::sequence_number() != before {
            if let Some(text) = clipboard::get_text() {
                captured = Some(text);
            }
            break;
        }
    }

    // 还原用户剪贴板：避免 Ctrl+C 泄留。
    backup.restore();

    match captured {
        Some(text) if is_plausible_selection(&text) => Outcome::Text { text, bounds: None },
        _ => Outcome::None,
    }
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
}
