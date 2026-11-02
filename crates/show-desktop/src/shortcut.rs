//! 接管系统 `Win+D`：低级键盘钩子吞键 + 状态机 + dummy key 掩码。
//!
//! shell（explorer）持有 `Win+D` 这个系统热键，`RegisterHotKey` 只会返回“已被占用”；
//! 只有在 `WH_KEYBOARD_LL` 里对 D 返回非零值，才能在系统处理之前把它吃掉。
//! 钩子的安装/销毁样板在 `whelper-common::win32::hook`，这里只保留 Win+D 的语义。
//!
//! 吞掉 D 之后必须补一次掩码事件，否则 shell 会当成“只按了 Win”并弹出开始菜单；
//! 掩码键由 `whelper-common::win32::keyboard::send_dummy_key` 注入（Alt+Tab 切换器
//! 共用同一份实现）。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use whelper_common::win32::hook::{self, KeyEvent};
use whelper_common::win32::keyboard::{modifier_down, send_dummy_key, VK_LWIN, VK_RWIN};

/// 钩子 → 消息循环：执行一次单屏切换。
pub const MSG_TOGGLE: u32 = WM_APP + 1;

const VK_D: u32 = 0x44;

/// 触发时投递 [`MSG_TOGGLE`] 的目标窗口（同进程的服务消息窗口）。
static TARGET_WINDOW: AtomicIsize = AtomicIsize::new(0);
static WIN_HELD: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<WinDState> = Mutex::new(WinDState { d_held: false });

/// 键盘钩子里对 Win+D 的处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinDAction {
    /// 与 Win+D 无关，原样放行。
    Pass,
    /// 吞掉这次按键，并通知消息循环切换一次。
    Trigger,
    /// 吞掉这次按键，但不触发（长按的自动重复，以及配对的抬起事件）。
    Swallow,
}

/// Win+D 的按键状态机。
#[derive(Debug, Default)]
pub struct WinDState {
    /// 是否已经吞掉这次 Win+D 的按下（用于抑制自动重复）。
    d_held: bool,
}

impl WinDState {
    /// `win_held` 由调用方给出（物理 Win 键是否按下）。
    pub fn on_key(&mut self, vk: u32, is_down: bool, win_held: bool) -> WinDAction {
        if vk != VK_D {
            return WinDAction::Pass;
        }
        if is_down {
            if !win_held {
                // 单独按 D：正常输入。
                return WinDAction::Pass;
            }
            if self.d_held {
                return WinDAction::Swallow;
            }
            self.d_held = true;
            WinDAction::Trigger
        } else if self.d_held {
            // 按下被吞了，抬起也要吞，否则前台应用会收到孤立的 keyup。
            self.d_held = false;
            WinDAction::Swallow
        } else {
            WinDAction::Pass
        }
    }
}

/// 往共享的键盘钩子链上注册；`target` 是同进程服务窗口，触发时向它投递 [`MSG_TOGGLE`]。
pub fn install(target: HWND) -> bool {
    TARGET_WINDOW.store(target.0 as isize, Ordering::SeqCst);
    hook::add_handler(on_key)
}

pub fn uninstall() {
    hook::remove_handler(on_key);
    TARGET_WINDOW.store(0, Ordering::SeqCst);
    WIN_HELD.store(false, Ordering::SeqCst);
    *STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = WinDState::default();
}

/// 钩子回调：`true` 表示吞掉这次按键。
///
/// 注入的 `Win+D`（远程桌面、自动化）同样接管：本程序不会注入 `Win+D`，不存在回环。
fn on_key(event: &KeyEvent) -> bool {
    // 自己注入的 dummy key 会重入钩子链，直接放行。
    if hook::generated_by_us(event) {
        return false;
    }
    if !whelper_config::get().desktop.enabled {
        // 设置页关掉后不再接管 Win+D，并清掉半截按键状态。
        let mut state = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = WinDState::default();
        return false;
    }
    if matches!(event.vk, VK_LWIN | VK_RWIN) {
        WIN_HELD.store(event.is_down, Ordering::SeqCst);
    }
    if event.vk != VK_D {
        return false;
    }

    let action = {
        let mut state = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.on_key(event.vk, event.is_down, win_held())
    };

    match action {
        WinDAction::Pass => false,
        WinDAction::Trigger => {
            send_dummy_key();
            let raw = TARGET_WINDOW.load(Ordering::SeqCst);
            if raw != 0 {
                // 不直接在这里最小化：钩子回调必须尽快返回，真正的工作交给消息循环。
                unsafe {
                    let _ = PostMessageW(
                        HWND(raw as *mut c_void),
                        MSG_TOGGLE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            true
        }
        WinDAction::Swallow => true,
    }
}

fn win_held() -> bool {
    // 优先用钩子里跟踪的状态，`GetAsyncKeyState` 只兜底“钩子安装时 Win 已经按下”。
    WIN_HELD.load(Ordering::SeqCst) || modifier_down(VK_LWIN) || modifier_down(VK_RWIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_d_triggers_once_and_swallows_the_rest() {
        let mut state = WinDState::default();
        assert_eq!(state.on_key(VK_D, true, true), WinDAction::Trigger);
        // 长按的自动重复只吞不触发，避免桌面反复切换。
        assert_eq!(state.on_key(VK_D, true, true), WinDAction::Swallow);
        assert_eq!(state.on_key(VK_D, false, true), WinDAction::Swallow);
        // 松开后可以再次触发。
        assert_eq!(state.on_key(VK_D, true, true), WinDAction::Trigger);
    }

    #[test]
    fn plain_d_is_passed_through() {
        let mut state = WinDState::default();
        assert_eq!(state.on_key(VK_D, true, false), WinDAction::Pass);
        assert_eq!(state.on_key(VK_D, false, false), WinDAction::Pass);
    }

    #[test]
    fn orphan_keyup_is_passed_through() {
        let mut state = WinDState::default();
        assert_eq!(state.on_key(VK_D, false, true), WinDAction::Pass);
    }

    #[test]
    fn other_shortcuts_are_passed_through() {
        let mut state = WinDState::default();
        // Win+E 等其它组合不受影响。
        assert_eq!(state.on_key(0x45, true, true), WinDAction::Pass);
        assert_eq!(state.on_key(0x45, false, true), WinDAction::Pass);
    }
}
