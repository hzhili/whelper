//! 低级键盘钩子（`WH_KEYBOARD_LL`）的安装与分发。
//!
//! 全局钩子由**安装线程的消息泵**驱动：必须在跑 `GetMessage` 循环的线程上注册，
//! 并且该线程在进程生命周期内保持存活。
//!
//! 一个进程里可能同时有多个组件要处理键盘（取词判定、接管 Win+D），因此这里维护
//! 一条**处理器链**：首个处理器注册时安装钩子，最后一个注销时卸载；任一处理器返回
//! `true` 即吞掉该按键，链上后面的处理器不再被调用。
//!
//! 处理器**不在锁内**调用：处理器可能注入按键（例如吞掉 Win+D 后补一个掩码键），
//! 注入事件会重入本钩子，持锁调用会自锁。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
};

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;

/// `LLKHF_INJECTED`：由 `SendInput` / `keybd_event` 注入的事件。
const LLKHF_INJECTED: u32 = 0x10;

/// 我们在 `dwExtraInfo` 里给自家注入事件打的标记。
///
/// 对应 PowerToys 的 `KEYBOARDMANAGER_INJECTED_FLAG`：比 `LLKHF_INJECTED` 精确，
/// 只忽略“自己发的”，不会把别的程序（AutoHotkey 等）注入的按键也一起放过。
pub const INJECTED_FLAG: usize = 0x1;

/// 一次按键事件（只包含按下/抬起，不含其它键盘消息）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub vk: u32,
    pub is_down: bool,
    /// 任一注入方式（`SendInput` / `keybd_event`）产生的事件。
    pub injected: bool,
    /// `KBDLLHOOKSTRUCT.dwExtraInfo`：注入方用它标识自己的事件。
    pub extra_info: usize,
}

/// 这次事件是否由本进程注入（带了 [`INJECTED_FLAG`]）。
pub fn generated_by_us(event: &KeyEvent) -> bool {
    event.extra_info & INJECTED_FLAG != 0
}

/// 返回 `true` 表示吞掉这次按键：系统与前台应用都不会收到。
pub type KeyHandler = fn(&KeyEvent) -> bool;

static HANDLERS: Mutex<Vec<KeyHandler>> = Mutex::new(Vec::new());
static HOOK: AtomicIsize = AtomicIsize::new(0);

/// 处理器链的容量上限。组件数量很少，固定容量让钩子回调里不需要分配。
const MAX_HANDLERS: usize = 8;

fn debug_enabled() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("WHELPER_HOOK_DEBUG").is_some())
}

/// 注册处理器；首个处理器注册时安装全局钩子。返回钩子是否可用。
pub fn add_handler(handler: KeyHandler) -> bool {
    {
        let mut handlers = HANDLERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !handlers.iter().any(|registered| std::ptr::fn_addr_eq(*registered, handler)) {
            if handlers.len() < MAX_HANDLERS {
                handlers.push(handler);
            } else {
                eprintln!("[warn] 键盘钩子处理器已达上限 {MAX_HANDLERS}，忽略新注册");
            }
        }
    }
    if HOOK.load(Ordering::SeqCst) != 0 {
        return true;
    }
    install_hook()
}

/// 注销处理器；链为空时卸载钩子。
pub fn remove_handler(handler: KeyHandler) {
    let empty = {
        let mut handlers = HANDLERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        handlers.retain(|registered| !std::ptr::fn_addr_eq(*registered, handler));
        handlers.is_empty()
    };
    if empty {
        uninstall_hook();
    }
}

fn install_hook() -> bool {
    let module = unsafe { GetModuleHandleW(None) }.ok();
    let hinstance = module
        .map(|module| HINSTANCE(module.0))
        .unwrap_or(HINSTANCE(std::ptr::null_mut()));

    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hinstance, 0) } {
        Ok(hook) => {
            HOOK.store(hook.0 as isize, Ordering::SeqCst);
            true
        }
        Err(_) => false,
    }
}

fn uninstall_hook() {
    let raw = HOOK.swap(0, Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut c_void));
        }
    }
}

/// 把钩子回调的参数解成 [`KeyEvent`]；非按下/抬起消息返回 `None`。
pub fn decode(message: u32, vk: u32, flags: u32, extra_info: usize) -> Option<KeyEvent> {
    let is_down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    let is_up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
    (is_down || is_up).then_some(KeyEvent {
        vk,
        is_down,
        injected: flags & LLKHF_INJECTED != 0,
        extra_info,
    })
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    }

    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if let Some(event) = decode(wparam.0 as u32, info.vkCode, info.flags.0, info.dwExtraInfo) {
        // 先把处理器拷到栈上再调用：调用期间不持锁，允许处理器注入按键触发重入。
        let mut snapshot = [None; MAX_HANDLERS];
        let count = {
            let handlers = HANDLERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let count = handlers.len().min(MAX_HANDLERS);
            for (slot, handler) in snapshot.iter_mut().zip(handlers.iter()) {
                *slot = Some(*handler);
            }
            count
        };

        if debug_enabled() {
            eprintln!(
                "[hook] vk=0x{:x} down={} injected={} handlers={count}",
                event.vk, event.is_down, event.injected
            );
        }

        for handler in snapshot.iter().take(count).flatten() {
            if handler(&event) {
                return LRESULT(1);
            }
        }
    }

    CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_down_and_up_are_decoded() {
        let event = decode(WM_KEYDOWN, 0x44, 0, 0).unwrap();
        assert!(event.is_down);
        assert!(!event.injected);
        assert_eq!(event.vk, 0x44);

        let event = decode(WM_SYSKEYUP, 0x44, 0, 0).unwrap();
        assert!(!event.is_down);
    }

    #[test]
    fn injected_flag_is_reported() {
        assert!(decode(WM_KEYDOWN, 0x43, LLKHF_INJECTED, 0).unwrap().injected);
    }

    #[test]
    fn own_injection_is_recognised_by_extra_info() {
        // 自己注入的：带 INJECTED_FLAG。
        let event = decode(WM_KEYDOWN, 0xFF, LLKHF_INJECTED, INJECTED_FLAG).unwrap();
        assert!(event.injected);
        assert!(generated_by_us(&event));

        // 别的程序注入的：有 LLKHF_INJECTED，但没有我们的标记。
        let event = decode(WM_KEYDOWN, 0x44, LLKHF_INJECTED, 0).unwrap();
        assert!(event.injected);
        assert!(!generated_by_us(&event));
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(decode(0x0200, 0x44, 0, 0), None);
        assert_eq!(decode(0, 0, 0, 0), None);
    }
}
