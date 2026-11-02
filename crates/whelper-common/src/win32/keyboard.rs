//! 键盘状态查询与“掩码键”注入。
//!
//! 低级钩子（[`super::hook`]）只给出单个按键的 vk 与按下状态，组合键里的修饰键
//! 需要在这里额外查询；被吞掉的系统热键（`Win+D`、`Alt+Tab`）还要补一次
//! **掩码键**，否则 shell / 前台应用会把“只剩下修饰键”当成一次完整按键：
//!
//! - `Win+D`：只看到 Win 起落 → 弹出开始菜单；
//! - `Alt+Tab`：只看到 Alt 起落 → 激活前台窗口的菜单栏。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

use crate::win32::hook::INJECTED_FLAG;

pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
/// 通用 Alt（左右都算）。
pub const VK_MENU: u32 = 0x12;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;

/// PowerToys 同款“dummy key”：未文档化的 0xFF 虚拟键码。
const DUMMY_KEY: u16 = 0xFF;

/// 修饰键当前是否物理按下。
pub fn modifier_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// 注入一次 `0xFF` dummy key（down + up），并在 `dwExtraInfo` 里打上 [`INJECTED_FLAG`]。
///
/// 钩子链看到这个标记会直接放行，避免“注入 → 重入 → 再处理”的回环。
pub fn send_dummy_key() {
    // down + up 一起发：PowerToys 早期只发 up，后来因为 Slack 等应用的问题改成成对发。
    let inputs = [dummy_input(KEYBD_EVENT_FLAGS(0)), dummy_input(KEYEVENTF_KEYUP)];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn dummy_input(flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(DUMMY_KEY),
                // dummy 没有对应的物理键，MapVirtualKey 同样返回 0。
                wScan: 0,
                dwFlags: flags,
                time: 0,
                // 打上自家标记：钩子链据此跳过，避免把自己的注入再处理一遍。
                dwExtraInfo: INJECTED_FLAG,
            },
        },
    }
}
