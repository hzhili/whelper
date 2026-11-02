//! 接管 Alt+Tab：低级键盘钩子吞键 + 会话状态机 + 掩码键。
//!
//! 系统 Alt+Tab 由 shell 直接处理，`RegisterHotKey` 拦不到，只能在 `WH_KEYBOARD_LL`
//! 里对 Tab 返回非零值把它吃掉（钩子样板在 `whelper-common::win32::hook`）。
//!
//! 为什么**只吞 Tab、放行 Alt**：
//!
//! - 前台应用在 Tab 之前已经收到了 Alt 按下，吞掉配对的 Alt 抬起会让它以为 Alt 一直按着；
//! - 但如果它只看到 Alt 起落，就会激活菜单栏，因此在第一次吞 Tab 时注入一次掩码键
//!   （[`send_dummy_key`]，与 `Win+D` 防开始菜单是同一份实现）。
//!
//! 状态机（[`KeyState`]）是纯逻辑：修饰键状态由调用方查询后传入，可单测。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use whelper_common::win32::hook::{self, KeyEvent};
use whelper_common::win32::keyboard::{modifier_down, send_dummy_key, VK_SHIFT};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::model::Step;

/// 钩子 → 覆盖窗口：打开一次会话。wparam = 1 向前 / 0 向后。
pub const MSG_START: u32 = WM_APP + 30;
/// 钩子 → 覆盖窗口：移动选择。wparam = [`Step::code`]。
pub const MSG_STEP: u32 = WM_APP + 31;
/// 钩子 → 覆盖窗口：松开 Alt，提交当前选择。
pub const MSG_COMMIT: u32 = WM_APP + 32;
/// 钩子 → 覆盖窗口：Esc，取消会话。
pub const MSG_CANCEL: u32 = WM_APP + 33;
/// 钩子 → 覆盖窗口：切换缩略图 / 网格视图。
pub const MSG_MODE: u32 = WM_APP + 34;

const VK_TAB: u32 = 0x09;
const VK_ESCAPE: u32 = 0x1B;
const VK_HOME: u32 = 0x24;
const VK_END: u32 = 0x23;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
/// `Alt+`` `（反引号）：切换两种视图（窗口切换器的经典手势）。
const VK_OEM_3: u32 = 0xC0;

/// 通用 Alt 与左右 Alt。
const VK_MENU: u32 = 0x12;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;

/// 键盘钩子里对一次按键的处理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// 与本功能无关，原样放行。
    Pass,
    /// 吞掉这次按键，但不通知界面（配对的抬起、自动重复）。
    Swallow,
    /// 吞掉按键（Alt 抬起除外），并向覆盖窗口投递 `message`。
    Notify(u32, isize),
}

/// Alt+Tab 的按键状态机。
#[derive(Debug, Default)]
pub struct KeyState {
    /// 左 Alt 是否按下。左右键独立追踪，避免一侧先抬起时误结束另一侧的 Alt 会话。
    left_alt: bool,
    /// 右 Alt 是否按下。
    right_alt: bool,
    /// 钩子报告通用 `VK_MENU` 时的 Alt 状态（与左右 Alt 状态合并判断）。
    generic_alt: bool,
    /// 会话是否打开：从第一次吞 Tab 到全部 Alt 抬起 / Esc。
    session: bool,
    /// Tab 按下被吞过，配对的抬起也要吞。
    tab_swallowed: bool,
}

impl KeyState {
    pub fn session_active(&self) -> bool {
        self.session
    }

    /// `shift` 由调用方查询（钩子事件里没有修饰键状态）。
    pub fn on_key(&mut self, vk: u32, is_down: bool, shift: bool) -> KeyAction {
        match vk {
            VK_MENU | VK_LMENU | VK_RMENU => self.on_alt(vk, is_down),
            VK_TAB => self.on_tab(is_down, shift),
            _ => self.on_session_key(vk, is_down),
        }
    }

    fn alt_down(&self) -> bool {
        self.left_alt || self.right_alt || self.generic_alt
    }

    fn on_alt(&mut self, vk: u32, is_down: bool) -> KeyAction {
        let was_down = self.alt_down();
        match vk {
            VK_LMENU => self.left_alt = is_down,
            VK_RMENU => self.right_alt = is_down,
            VK_MENU => self.generic_alt = is_down,
            _ => unreachable!("on_alt 仅处理 Alt 虚拟键"),
        }
        if is_down || !was_down || self.alt_down() || !self.session {
            return KeyAction::Pass;
        }
        self.session = false;
        // 只有最后一个 Alt 抬起才提交；抬起本身必须放行。
        KeyAction::Notify(MSG_COMMIT, 0)
    }

    fn on_tab(&mut self, is_down: bool, shift: bool) -> KeyAction {
        if !is_down {
            // 按下被吞了，抬起也要吞，否则前台应用会收到孤立的 keyup。
            if self.tab_swallowed {
                self.tab_swallowed = false;
                return KeyAction::Swallow;
            }
            return KeyAction::Pass;
        }
        if !self.alt_down() {
            // 单独按 Tab：正常输入 / 焦点切换。
            return KeyAction::Pass;
        }
        self.tab_swallowed = true;
        if self.session {
            return KeyAction::Notify(MSG_STEP, step_code(shift));
        }
        self.session = true;
        // 第一次 Alt+Tab：打开覆盖层，同时按方向预选到下一个/上一个窗口。
        KeyAction::Notify(MSG_START, if shift { 0 } else { 1 })
    }

    fn on_session_key(&mut self, vk: u32, is_down: bool) -> KeyAction {
        if !self.session || !is_down {
            return KeyAction::Pass;
        }
        let step = match vk {
            VK_ESCAPE => {
                self.session = false;
                return KeyAction::Notify(MSG_CANCEL, 0);
            }
            VK_OEM_3 => return KeyAction::Notify(MSG_MODE, 0),
            VK_LEFT => Step::Prev,
            VK_RIGHT => Step::Next,
            // 图标条只有一行，行移动退化为前后一步；网格按当前列数跨行。
            VK_UP | VK_PRIOR => Step::RowPrev,
            VK_DOWN | VK_NEXT => Step::RowNext,
            VK_HOME => Step::First,
            VK_END => Step::Last,
            _ => return KeyAction::Pass,
        };
        KeyAction::Notify(MSG_STEP, step.code())
    }
}

fn step_code(shift: bool) -> isize {
    if shift {
        Step::Prev.code()
    } else {
        Step::Next.code()
    }
}

/// 触发时投递消息的目标窗口（覆盖窗口）。
static TARGET_WINDOW: AtomicIsize = AtomicIsize::new(0);
static STATE: Mutex<KeyState> = Mutex::new(KeyState {
    left_alt: false,
    right_alt: false,
    generic_alt: false,
    session: false,
    tab_swallowed: false,
});

/// 往共享键盘钩子链上注册；`target` 是同进程的覆盖窗口。
pub fn install(target: HWND) -> bool {
    TARGET_WINDOW.store(target.0 as isize, Ordering::SeqCst);
    hook::add_handler(on_key)
}

pub fn uninstall() {
    hook::remove_handler(on_key);
    TARGET_WINDOW.store(0, Ordering::SeqCst);
    reset();
}

/// 清空状态机（会话被界面强制结束、组件卸载时调用）。
pub fn reset() {
    *STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = KeyState::default();
}

fn on_key(event: &KeyEvent) -> bool {
    // 自己注入的掩码键会重入钩子链，直接放行。
    if hook::generated_by_us(event) {
        return false;
    }

    if !whelper_config::get().switcher.enabled {
        // 设置页关掉切换器后不再接管 Alt+Tab；如果切换器正开着，先让它取消淡出。
        let mut state = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.session {
            state.session = false;
            state.tab_swallowed = false;
            drop(state);
            post(MSG_CANCEL, 0);
        }
        return false;
    }

    let shift = modifier_down(VK_SHIFT);
    let action = {
        let mut state = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.on_key(event.vk, event.is_down, shift)
    };

    match action {
        KeyAction::Pass => false,
        KeyAction::Swallow => true,
        KeyAction::Notify(message, code) => {
            if message == MSG_START {
                // 吞掉 Tab 后 Alt 期间只剩修饰键：补掩码键，避免前台应用在 Alt 抬起时
                // 激活菜单栏。必须在钩子回调里发，赶在 Alt 抬起之前。
                send_dummy_key();
            }
            post(message, code);
            // Alt 抬起必须放行：前台应用已经收到过 Alt 按下。
            message != MSG_COMMIT
        }
    }
}

fn post(message: u32, code: isize) {
    let raw = TARGET_WINDOW.load(Ordering::SeqCst);
    if raw == 0 {
        return;
    }
    if crate::win32::debug_enabled() {
        eprintln!("[switcher] msg={message} code={code}");
    }
    unsafe {
        let _ = PostMessageW(
            HWND(raw as *mut c_void),
            message,
            WPARAM(code as usize),
            LPARAM(0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;

    fn down(state: &mut KeyState, vk: u32) -> KeyAction {
        state.on_key(vk, true, false)
    }

    fn up(state: &mut KeyState, vk: u32) -> KeyAction {
        state.on_key(vk, false, false)
    }

    #[test]
    fn tab_without_alt_is_passed_through() {
        let mut state = KeyState::default();
        assert_eq!(down(&mut state, VK_TAB), KeyAction::Pass);
        assert_eq!(up(&mut state, VK_TAB), KeyAction::Pass);
        assert!(!state.session_active());
    }

    #[test]
    fn alt_tab_opens_a_session_and_swallows_tab() {
        let mut state = KeyState::default();
        assert_eq!(down(&mut state, VK_MENU), KeyAction::Pass);
        assert_eq!(down(&mut state, VK_TAB), KeyAction::Notify(MSG_START, 1));
        assert!(state.session_active());
    }

    #[test]
    fn repeated_tab_steps_instead_of_restarting() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(
            down(&mut state, VK_TAB),
            KeyAction::Notify(MSG_STEP, Step::Next.code())
        );
        assert_eq!(
            down(&mut state, VK_TAB),
            KeyAction::Notify(MSG_STEP, Step::Next.code())
        );
    }

    #[test]
    fn shift_alt_tab_starts_backwards_and_steps_backwards() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        assert_eq!(
            state.on_key(VK_TAB, true, true),
            KeyAction::Notify(MSG_START, 0)
        );
        assert_eq!(
            state.on_key(VK_TAB, true, true),
            KeyAction::Notify(MSG_STEP, Step::Prev.code())
        );
    }

    #[test]
    fn tab_keyup_is_swallowed_only_when_the_keydown_was() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(up(&mut state, VK_TAB), KeyAction::Swallow);

        // 单独按 Tab：抬起照常放行。
        let mut state = KeyState::default();
        assert_eq!(up(&mut state, VK_TAB), KeyAction::Pass);
    }

    #[test]
    fn releasing_alt_commits_once() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(up(&mut state, VK_MENU), KeyAction::Notify(MSG_COMMIT, 0));
        assert!(!state.session_active());
        assert_eq!(up(&mut state, VK_MENU), KeyAction::Pass);
    }

    #[test]
    fn releasing_one_alt_does_not_commit_while_the_other_is_held() {
        let mut state = KeyState::default();
        down(&mut state, VK_LMENU);
        down(&mut state, VK_RMENU);
        down(&mut state, VK_TAB);

        assert_eq!(up(&mut state, VK_LMENU), KeyAction::Pass);
        assert!(state.session_active());
        assert_eq!(
            down(&mut state, VK_TAB),
            KeyAction::Notify(MSG_STEP, Step::Next.code())
        );
        assert_eq!(up(&mut state, VK_RMENU), KeyAction::Notify(MSG_COMMIT, 0));
        assert!(!state.session_active());
    }

    #[test]
    fn generic_alt_and_side_specific_alt_are_tracked_independently() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_LMENU);
        down(&mut state, VK_TAB);

        assert_eq!(up(&mut state, VK_MENU), KeyAction::Pass);
        assert!(state.session_active());
        assert_eq!(up(&mut state, VK_LMENU), KeyAction::Notify(MSG_COMMIT, 0));
    }

    #[test]
    fn escape_cancels_and_releasing_alt_does_not_commit() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(
            down(&mut state, VK_ESCAPE),
            KeyAction::Notify(MSG_CANCEL, 0)
        );
        assert!(!state.session_active());
        assert_eq!(up(&mut state, VK_MENU), KeyAction::Pass);
        // 取消后 Tab 抬起仍然要吞（按下已经吞了）。
        assert_eq!(up(&mut state, VK_TAB), KeyAction::Swallow);
    }

    #[test]
    fn arrow_keys_move_only_during_a_session() {
        let mut state = KeyState::default();
        assert_eq!(down(&mut state, VK_RIGHT), KeyAction::Pass);

        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(
            down(&mut state, VK_RIGHT),
            KeyAction::Notify(MSG_STEP, Step::Next.code())
        );
        assert_eq!(
            down(&mut state, VK_LEFT),
            KeyAction::Notify(MSG_STEP, Step::Prev.code())
        );
        assert_eq!(
            down(&mut state, VK_DOWN),
            KeyAction::Notify(MSG_STEP, Step::RowNext.code())
        );
        assert_eq!(
            down(&mut state, VK_UP),
            KeyAction::Notify(MSG_STEP, Step::RowPrev.code())
        );
        assert_eq!(
            down(&mut state, VK_HOME),
            KeyAction::Notify(MSG_STEP, Step::First.code())
        );
        assert_eq!(
            down(&mut state, VK_END),
            KeyAction::Notify(MSG_STEP, Step::Last.code())
        );
        // 抬起不拦截：按下已经被吞，孤立的抬起没有副作用。
        assert_eq!(up(&mut state, VK_RIGHT), KeyAction::Pass);
    }

    #[test]
    fn backtick_toggles_the_view_only_during_a_session() {
        let mut state = KeyState::default();
        assert_eq!(down(&mut state, VK_OEM_3), KeyAction::Pass);

        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(down(&mut state, VK_OEM_3), KeyAction::Notify(MSG_MODE, 0));
    }

    #[test]
    fn unrelated_keys_are_passed_through_during_a_session() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        assert_eq!(down(&mut state, VK_A), KeyAction::Pass);
        assert_eq!(up(&mut state, VK_A), KeyAction::Pass);
    }

    #[test]
    fn a_new_session_can_start_after_the_previous_one() {
        let mut state = KeyState::default();
        down(&mut state, VK_MENU);
        down(&mut state, VK_TAB);
        up(&mut state, VK_MENU);
        // Alt 重新按下后可以再次触发。
        down(&mut state, VK_MENU);
        assert_eq!(down(&mut state, VK_TAB), KeyAction::Notify(MSG_START, 1));
    }
}
