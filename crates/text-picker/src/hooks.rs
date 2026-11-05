//! 全局低级鼠标/键盘钩子与触发判定。
//!
//! 钩子回调只做"是否可能产生选区"的判定，然后把消息投递给主窗口，
//! 取词与 UI 一律在主线程/工作线程完成，避免阻塞输入链。
//!
//! 键盘钩子的安装与事件解码由 `whelper-common::win32::hook` 负责（同一进程里的
//! `show-desktop` 也往这条链上注册），本模块只实现“取词触发”的判定。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::Mutex;

use whelper_common::win32::hook::{self as keyboard_hook, KeyEvent};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetDoubleClickTime};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetSystemMetrics, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
    MSLLHOOKSTRUCT, SM_CXDOUBLECLK, SM_CYDOUBLECLK, WH_MOUSE_LL, WM_APP,
};

use whelper_config as config;

use crate::capture;

// 自定义消息（钩子 → 主窗口）
pub const MSG_MOUSE_DOWN: u32 = WM_APP + 1;
pub const MSG_MOUSE_UP: u32 = WM_APP + 2;
pub const MSG_WHEEL: u32 = WM_APP + 3;
pub const MSG_KEY_DOWN: u32 = WM_APP + 4;
pub const MSG_KEY_CANCEL: u32 = WM_APP + 5;
pub const MSG_KEY_SELECT: u32 = WM_APP + 6;
pub const MSG_CAPTURE_DONE: u32 = WM_APP + 7;
pub const MSG_TRANSLATE_DONE: u32 = WM_APP + 8;

// 原始鼠标消息
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MOUSEWHEEL: u32 = 0x020A;

const VK_ESCAPE: u32 = 0x1B;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;

/// 拖拽触发阈值（像素）：小于它视为单击。
pub const DRAG_TRIGGER_PX: i32 = 4;

static HOOK_WND: AtomicIsize = AtomicIsize::new(0);
static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
static TOOLBAR_VISIBLE: AtomicBool = AtomicBool::new(false);
static TOOLBAR_L: AtomicI32 = AtomicI32::new(0);
static TOOLBAR_T: AtomicI32 = AtomicI32::new(0);
static TOOLBAR_R: AtomicI32 = AtomicI32::new(0);
static TOOLBAR_B: AtomicI32 = AtomicI32::new(0);
// ---------- 触发判定（纯逻辑，可单测） ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    pub should_capture: bool,
    pub click_count: u32,
}

/// 判定"这次鼠标操作是否产生了选区"：
/// 拖拽超过阈值，或双击/三击选词。
#[derive(Debug, Clone)]
pub struct TriggerDetector {
    dragging: bool,
    drag_start: POINT,
    max_distance: i32,
    last_click_ms: u32,
    click_count: u32,
    last_click: POINT,
    double_click_ms: u32,
    double_click_cx: i32,
    double_click_cy: i32,
    drag_threshold: i32,
    /// 设置页开关：双击 / 三击是否算作选区触发。
    multi_click: bool,
}

impl TriggerDetector {
    pub fn new(
        double_click_ms: u32,
        double_click_cx: i32,
        double_click_cy: i32,
        drag_threshold: i32,
    ) -> Self {
        Self {
            dragging: false,
            drag_start: POINT { x: 0, y: 0 },
            max_distance: 0,
            last_click_ms: 0,
            click_count: 0,
            last_click: POINT { x: 0, y: 0 },
            double_click_ms,
            double_click_cx,
            double_click_cy,
            drag_threshold,
            multi_click: true,
        }
    }

    /// 应用设置页里的触发参数（每次按下鼠标时刷新，改配置立即生效）。
    pub fn set_options(&mut self, drag_threshold: i32, multi_click: bool) {
        self.drag_threshold = drag_threshold.max(1);
        self.multi_click = multi_click;
    }

    pub fn system_default() -> Self {
        unsafe {
            Self::new(
                GetDoubleClickTime(),
                GetSystemMetrics(SM_CXDOUBLECLK),
                GetSystemMetrics(SM_CYDOUBLECLK),
                DRAG_TRIGGER_PX,
            )
        }
    }

    /// 记一次按下，返回累计连击次数。
    pub fn button_down(&mut self, pt: POINT, now_ms: u32) -> u32 {
        let same_place = (pt.x - self.last_click.x).abs() <= self.double_click_cx / 2
            && (pt.y - self.last_click.y).abs() <= self.double_click_cy / 2;
        let within_time = now_ms.wrapping_sub(self.last_click_ms) <= self.double_click_ms;
        self.click_count = if within_time && same_place {
            (self.click_count + 1).min(3)
        } else {
            1
        };
        self.last_click_ms = now_ms;
        self.last_click = pt;
        self.dragging = true;
        self.drag_start = pt;
        self.max_distance = 0;
        self.click_count
    }

    pub fn mouse_move(&mut self, pt: POINT) {
        if !self.dragging {
            return;
        }
        let distance = (pt.x - self.drag_start.x)
            .abs()
            .max((pt.y - self.drag_start.y).abs());
        if distance > self.max_distance {
            self.max_distance = distance;
        }
    }

    /// 抬起鼠标。未记录到按下（例如钩子中途安装）时返回 `None`。
    pub fn button_up(&mut self) -> Option<Trigger> {
        if !self.dragging {
            return None;
        }
        self.dragging = false;
        let dragged = self.max_distance >= self.drag_threshold;
        let multi_click = self.multi_click && self.click_count >= 2;
        Some(Trigger {
            should_capture: dragged || multi_click,
            click_count: self.click_count,
        })
    }

    pub fn reset(&mut self) {
        self.dragging = false;
        self.max_distance = 0;
        self.click_count = 0;
    }
}

static DETECTOR: Mutex<Option<TriggerDetector>> = Mutex::new(None);

// ---------- 钩子生命周期 ----------

pub fn install(hwnd: HWND) -> bool {
    HOOK_WND.store(hwnd.0 as isize, Ordering::SeqCst);
    if let Ok(mut guard) = DETECTOR.lock() {
        *guard = Some(TriggerDetector::system_default());
    }
    // 鼠标钩子只服务取词，留在本模块；键盘钩子用 `whelper-common` 的通用实现。
    let mouse_ok = unsafe {
        let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok();
        let hinstance = match module {
            Some(m) => HINSTANCE(m.0),
            None => HINSTANCE(std::ptr::null_mut()),
        };
        match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinstance, 0) {
            Ok(hook) => {
                MOUSE_HOOK.store(hook.0 as isize, Ordering::SeqCst);
                true
            }
            Err(_) => false,
        }
    };
    let installed = mouse_ok && keyboard_hook::add_handler(on_key_event);
    capture::set_keyboard_hook_ready(installed);
    installed
}

pub fn uninstall() {
    capture::set_keyboard_hook_ready(false);
    unsafe {
        let mouse = MOUSE_HOOK.swap(0, Ordering::SeqCst);
        if mouse != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(mouse as *mut c_void));
        }
    }
    keyboard_hook::remove_handler(on_key_event);
    HOOK_WND.store(0, Ordering::SeqCst);
    if let Ok(mut guard) = DETECTOR.lock() {
        *guard = None;
    }
}

/// 主窗口在显示/隐藏工具条时同步矩形，供钩子判断"点击是否落在条上"。
pub fn set_toolbar_rect(rect: Option<RECT>) {
    match rect {
        Some(r) => {
            TOOLBAR_L.store(r.left, Ordering::SeqCst);
            TOOLBAR_T.store(r.top, Ordering::SeqCst);
            TOOLBAR_R.store(r.right, Ordering::SeqCst);
            TOOLBAR_B.store(r.bottom, Ordering::SeqCst);
            TOOLBAR_VISIBLE.store(true, Ordering::SeqCst);
        }
        None => TOOLBAR_VISIBLE.store(false, Ordering::SeqCst),
    }
}

fn inside_toolbar(pt: POINT) -> bool {
    TOOLBAR_VISIBLE.load(Ordering::SeqCst)
        && pt.x >= TOOLBAR_L.load(Ordering::SeqCst)
        && pt.x < TOOLBAR_R.load(Ordering::SeqCst)
        && pt.y >= TOOLBAR_T.load(Ordering::SeqCst)
        && pt.y < TOOLBAR_B.load(Ordering::SeqCst)
}

pub fn pack_point(pt: POINT) -> LPARAM {
    let x = (pt.x as u32 & 0xFFFF) as isize;
    let y = (pt.y as u32 & 0xFFFF) as isize;
    LPARAM((y << 16) | x)
}

pub fn unpack_point(lparam: LPARAM) -> POINT {
    let raw = lparam.0 as u32;
    POINT {
        x: (raw & 0xFFFF) as u16 as i16 as i32,
        y: ((raw >> 16) & 0xFFFF) as u16 as i16 as i32,
    }
}

fn post(msg: u32, wparam: usize, lparam: isize) {
    let raw = HOOK_WND.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(
                HWND(raw as *mut c_void),
                msg,
                WPARAM(wparam),
                LPARAM(lparam),
            );
        }
    }
}

fn modifier_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5 | 0x5D)
}

fn is_arrow(vk: u32) -> bool {
    matches!(vk, 0x25..=0x28)
}

fn now_ms() -> u32 {
    unsafe { GetTickCount64() as u32 }
}

pub fn wheel_delta(mouse_data: u32) -> i32 {
    ((mouse_data >> 16) as u16 as i16) as i32
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    }
    // 设置页关闭取词后，钩子仍然挂着（卸载要重启组件），这里直接放行。
    if !config::get().picker.enabled {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    }
    let msg = wparam.0 as u32;
    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    // 忽略自己用 SendInput 注入的事件（取词时的 Ctrl+C、复制点击等）。
    if info.flags & 0x0003 != 0 {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    }
    let pt = info.pt;
    if (matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) && !inside_toolbar(pt))
        || (msg == WM_MOUSEWHEEL && !inside_toolbar(pt))
    {
        capture::cancel_pending();
    }
    match msg {
        WM_LBUTTONDOWN => {
            if inside_toolbar(pt) {
                return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
            }
            let picker = config::get().picker;
            if let Ok(mut guard) = DETECTOR.try_lock() {
                if let Some(detector) = guard.as_mut() {
                    detector.set_options(picker.drag_threshold, picker.double_click);
                    let count = detector.button_down(pt, now_ms());
                    drop(guard);
                    post(MSG_MOUSE_DOWN, count as usize, pack_point(pt).0);
                }
            }
        }
        WM_MOUSEMOVE => {
            if let Ok(mut guard) = DETECTOR.try_lock() {
                if let Some(detector) = guard.as_mut() {
                    detector.mouse_move(pt);
                }
            }
        }
        WM_LBUTTONUP => {
            let trigger = match DETECTOR.try_lock() {
                Ok(mut guard) => guard.as_mut().and_then(|detector| detector.button_up()),
                Err(_) => None,
            };
            if let Some(trigger) = trigger {
                post(
                    MSG_MOUSE_UP,
                    trigger.should_capture as usize,
                    pack_point(pt).0,
                );
            }
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if !inside_toolbar(pt) {
                post(MSG_MOUSE_DOWN, 0, pack_point(pt).0);
            }
        }
        WM_MOUSEWHEEL => {
            // `mouseData` 的高 16 位是有符号滚轮增量（通常为 ±120）。
            let delta = wheel_delta(info.mouseData);
            post(MSG_WHEEL, delta as usize, pack_point(pt).0);
        }
        _ => {}
    }
    CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam)
}

/// 键盘钩子的取词判定（钩子安装与事件解码在 `whelper-common`）。
///
/// 与 `show-desktop` 不同，这里**从不吞用户按键**：`Ctrl+A` / `Shift+方向键` 等仍要送给前台应用。
/// 唯一可丢弃的是已取消请求生成的自家整批 Ctrl+C。
/// 返回 `true` 的语义只有“吞掉该按键”。
fn on_key_event(event: &KeyEvent) -> bool {
    if let Some(suppress) = capture::filter_copy_input(event) {
        return suppress;
    }
    if event.is_down {
        // 必须在放行按键前同步通知后台：PostMessage 只取消 UI 结果太迟。
        // 修饰键和其它程序注入的复制也算用户输入，不能被旧备份覆盖。
        capture::cancel_pending();
    }
    if event.injected {
        // 其它程序注入的事件可以取消取词，但不触发新取词。
        return false;
    }
    if !config::get().picker.enabled {
        return false;
    }
    let vk = event.vk;
    if event.is_down {
        if vk == VK_ESCAPE {
            post(MSG_KEY_CANCEL, 0, 0);
        } else if !is_modifier(vk) {
            // 任何输入都会改变/取消选区：先收起旧工具条。
            post(MSG_KEY_DOWN, vk as usize, 0);
        }
    } else {
        let ctrl = modifier_down(VK_CONTROL);
        let shift = modifier_down(VK_SHIFT);
        let ctrl_a = ctrl && vk == 0x41; // 'A'
        let shift_arrow = shift && is_arrow(vk);
        if ctrl_a || shift_arrow {
            post(MSG_KEY_SELECT, vk as usize, 0);
        } else if vk == VK_ESCAPE {
            post(MSG_KEY_CANCEL, 0, 0);
        }
    }
    false
}

/// 供 `--self-test`/调试使用：重装钩子前清理内部状态。
pub fn reset_state() {
    if let Ok(mut guard) = DETECTOR.lock() {
        if let Some(detector) = guard.as_mut() {
            detector.reset();
        }
    }
    set_toolbar_rect(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> TriggerDetector {
        TriggerDetector::new(500, 8, 8, DRAG_TRIGGER_PX)
    }

    fn pt(x: i32, y: i32) -> POINT {
        POINT { x, y }
    }

    #[test]
    fn plain_click_does_not_trigger() {
        let mut d = detector();
        assert_eq!(d.button_down(pt(100, 100), 1000), 1);
        d.mouse_move(pt(101, 101));
        let trigger = d.button_up().unwrap();
        assert!(!trigger.should_capture);
        assert_eq!(trigger.click_count, 1);
    }

    #[test]
    fn drag_beyond_threshold_triggers() {
        let mut d = detector();
        d.button_down(pt(100, 100), 1000);
        d.mouse_move(pt(120, 104));
        let trigger = d.button_up().unwrap();
        assert!(trigger.should_capture);
    }

    #[test]
    fn double_click_triggers_even_without_drag() {
        let mut d = detector();
        assert_eq!(d.button_down(pt(10, 10), 1000), 1);
        assert!(!d.button_up().unwrap().should_capture);
        assert_eq!(d.button_down(pt(10, 10), 1200), 2);
        let trigger = d.button_up().unwrap();
        assert!(trigger.should_capture);
        assert_eq!(trigger.click_count, 2);
    }

    #[test]
    fn slow_second_click_is_not_a_double_click() {
        let mut d = detector();
        d.button_down(pt(10, 10), 1000);
        d.button_up();
        assert_eq!(d.button_down(pt(10, 10), 3000), 1);
        assert!(!d.button_up().unwrap().should_capture);
    }

    #[test]
    fn distant_second_click_is_not_a_double_click() {
        let mut d = detector();
        d.button_down(pt(10, 10), 1000);
        d.button_up();
        assert_eq!(d.button_down(pt(200, 200), 1100), 1);
        assert!(!d.button_up().unwrap().should_capture);
    }

    #[test]
    fn button_up_without_down_is_ignored() {
        let mut d = detector();
        assert!(d.button_up().is_none());
    }

    #[test]
    fn multi_click_can_be_disabled() {
        let mut d = detector();
        d.set_options(DRAG_TRIGGER_PX, false);
        d.button_down(pt(10, 10), 1000);
        assert!(!d.button_up().unwrap().should_capture);
        d.button_down(pt(10, 10), 1100);
        assert!(!d.button_up().unwrap().should_capture);
    }

    #[test]
    fn drag_threshold_follows_settings() {
        let mut d = detector();
        d.set_options(20, true);
        d.button_down(pt(0, 0), 0);
        d.mouse_move(pt(10, 0));
        assert!(!d.button_up().unwrap().should_capture);
        d.button_down(pt(0, 0), 10);
        d.mouse_move(pt(24, 0));
        assert!(d.button_up().unwrap().should_capture);
    }

    #[test]
    fn copy_shortcut_is_never_swallowed() {
        for vk in [0x11, 0xA2, 0xA3, 0x43] {
            for is_down in [true, false] {
                for (injected, extra_info) in
                    [(false, 0), (true, 0), (true, keyboard_hook::INJECTED_FLAG)]
                {
                    assert!(!on_key_event(&KeyEvent {
                        vk,
                        is_down,
                        injected,
                        extra_info
                    }));
                }
            }
        }
    }

    #[test]
    fn point_packing_roundtrips_negative_coordinates() {
        let p = pt(-1200, -40);
        assert_eq!(unpack_point(pack_point(p)), p);
        let p = pt(3000, 1500);
        assert_eq!(unpack_point(pack_point(p)), p);
    }

    #[test]
    fn wheel_delta_decodes_signed_high_word() {
        assert_eq!(wheel_delta(120 << 16), 120);
        assert_eq!(wheel_delta((-120i32 as u32) << 16), -120);
    }
}
