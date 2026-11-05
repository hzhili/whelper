//! 主窗口与工具条 UI。
//!
//! 一个窗口承担三种角色：隐藏的消息窗口、两按钮工具条、翻译结果面板。
//! 全部使用 `WS_EX_NOACTIVATE`，保证弹出时不会抢走原应用的焦点，
//! 这样取词用的 Ctrl+C 仍然发给原来的应用。

use std::ffi::c_void;
use std::time::{Duration, Instant};

use whelper_common::geometry::Rect;
use whelper_common::win32::applink;
use whelper_config::{self as config, Theme};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect, GetDC,
    GetStockObject, GetTextExtentPoint32W, IntersectClipRect, InvalidateRect, ReleaseDC, RestoreDC,
    RoundRect, SaveDC, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TextOutW, UpdateWindow,
    CLEARTYPE_QUALITY, DEFAULT_CHARSET, HBRUSH, HDC, HFONT, HGDIOBJ, NULL_PEN, PAINTSTRUCT,
    SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer,
    LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage, CREATESTRUCTW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HMENU, HWND_TOPMOST, IDC_ARROW,
    MF_SEPARATOR, MF_STRING, MSG, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE,
    SW_SHOWNORMAL, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCREATE, WM_NULL, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture;
use crate::clipboard;
use crate::hooks;
use crate::icon::{self, Mask};
use crate::toolbar::{self, Mode};
use crate::translate;
use crate::winutil;

const CLASS_NAME: &str = applink::TOOLBAR_CLASS;
const TIMER_FOREGROUND: usize = 1;
const MSG_DEMO: u32 = WM_APP + 20;
/// 工具条右键菜单项：打开设置 / 退出。
const MENU_OPEN_SETTINGS: usize = 1;
const MENU_QUIT: usize = 2;
/// 刚弹出时忽略失焦的宽限期（避免弹出瞬间因焦点切换而立刻消失）。
const SHOW_GRACE_MS: u64 = 400;
const CAPTURE_TIMEOUT_MS: u64 = 350;
const WHEEL_DELTA: i32 = 120;

struct ScrollDrag {
    grab_offset: i32,
}

const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

/// 工具条取色板：浅色/深色两套，数值取自视觉稿。
#[derive(Clone, Copy)]
struct Palette {
    card: COLORREF,
    border: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    hover: COLORREF,
    divider: COLORREF,
    error: COLORREF,
}

const LIGHT: Palette = Palette {
    card: rgb(255, 255, 255),
    border: rgb(222, 224, 227),
    text: rgb(38, 38, 38),
    muted: rgb(115, 115, 115),
    hover: rgb(240, 240, 240),
    divider: rgb(238, 239, 242),
    error: rgb(214, 69, 69),
};

const DARK: Palette = Palette {
    card: rgb(41, 41, 41),
    border: rgb(95, 95, 95),
    text: rgb(217, 217, 217),
    muted: rgb(153, 153, 153),
    hover: rgb(56, 56, 56),
    divider: rgb(70, 70, 70),
    error: rgb(255, 107, 107),
};

/// 设置页的主题优先；`TEXT_PICKER_THEME=light|dark` 仍可强制覆盖（调试用）。
fn palette() -> Palette {
    if let Ok(v) = std::env::var("TEXT_PICKER_THEME") {
        if v.eq_ignore_ascii_case("dark") {
            return DARK;
        }
        if v.eq_ignore_ascii_case("light") {
            return LIGHT;
        }
    }
    let dark = match config::get().general.theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => !winutil::apps_use_light_theme(),
    };
    if dark {
        DARK
    } else {
        LIGHT
    }
}

fn debug_enabled() -> bool {
    std::env::var_os("TEXT_PICKER_DEBUG").is_some() || config::get().general.debug
}

/// 按当前 DPI 缓存的图标蒙版（只在 DPI 变化时重算）。
struct IconCache {
    size: usize,
    copy_back: Mask,
    copy_front: Mask,
    translate: Mask,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureAction {
    ShowBar,
    Copy,
    Translate,
}

struct CapturePayload {
    text: String,
    /// 终端类窗口：先弹条、不复制，等用户点按钮再取词（避免 Ctrl+C 取消选区）。
    deferred: bool,
    /// UIA 给出的选区外接矩形；有值时用它定位工具条（上方居中）。
    bounds: Option<Rect>,
    anchor: POINT,
    target: HWND,
    generation: u64,
    request: capture::Request,
    action: CaptureAction,
}

/// 工具条定位点：优先用选区矩形的「上边中点」，UIA 没给矩形时退回鼠标位置。
fn anchor_from_bounds(point: POINT, bounds: Option<Rect>) -> POINT {
    match bounds {
        Some(bounds) if bounds.width() > 0 && bounds.height() > 0 => POINT {
            x: bounds.left + bounds.width() / 2,
            y: bounds.top,
        },
        _ => point,
    }
}

struct TranslatePayload {
    result: Result<String, String>,
    generation: u64,
}

pub struct App {
    hwnd: HWND,
    scale: f32,
    font: HFONT,
    font_small: HFONT,
    pal: Palette,
    icons: Option<IconCache>,
    mode: Mode,
    visible: bool,
    hover: Option<usize>,
    /// 当前选区文本
    text: String,
    /// 选区文本尚未取到（终端类窗口）：点按钮时才真正取词
    deferred: bool,
    /// 工具条定位点：UIA 有选区矩形时是矩形上边中点，否则是鼠标抬起位置
    anchor: POINT,
    /// 取词锚点：始终保留最初的鼠标抬起位置（延迟取词时鼠标已移到工具条上）
    capture_anchor: POINT,
    /// 选区的来源窗口
    target: HWND,
    shown_at: Instant,
    rect: RECT,
    translation: String,
    error: String,
    panel_lines: Vec<String>,
    panel_content_height: i32,
    panel_viewport_height: i32,
    panel_scroll_y: i32,
    wheel_remainder: i32,
    panel_size: (i32, i32),
    panel_has_scrollbar: bool,
    scroll_drag: Option<ScrollDrag>,
    generation: u64,
    timer_on: bool,
    quitting: bool,
    /// 上次看到的配置版本：变化时刷新调色板（设置页改主题立即生效）。
    config_revision: u64,
}

impl App {
    fn new() -> Self {
        Self {
            hwnd: winutil::null_hwnd(),
            scale: 1.0,
            font: HFONT(std::ptr::null_mut()),
            font_small: HFONT(std::ptr::null_mut()),
            pal: LIGHT,
            icons: None,
            mode: Mode::Bar,
            visible: false,
            hover: None,
            text: String::new(),
            deferred: false,
            anchor: POINT { x: 0, y: 0 },
            capture_anchor: POINT { x: 0, y: 0 },
            target: winutil::null_hwnd(),
            shown_at: Instant::now(),
            rect: RECT::default(),
            translation: String::new(),
            error: String::new(),
            panel_lines: Vec::new(),
            panel_content_height: 0,
            panel_viewport_height: 0,
            panel_scroll_y: 0,
            wheel_remainder: 0,
            panel_size: (0, 0),
            panel_has_scrollbar: false,
            scroll_drag: None,
            generation: 0,
            timer_on: false,
            quitting: false,
            config_revision: 0,
        }
    }

    unsafe fn ensure_fonts(&mut self) {
        if !self.font.0.is_null() {
            return;
        }
        let dpi = GetDpiForWindow(self.hwnd);
        self.scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
        // 字号 14px / 行高 22px。
        let height = -((14.0 * self.scale).round() as i32);
        let small = -((12.0 * self.scale).round() as i32);
        let face = winutil::to_wide("Microsoft YaHei UI");
        self.font = CreateFontW(
            height,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            0,
            0,
            CLEARTYPE_QUALITY.0 as u32,
            0,
            winutil::wide_ptr(&face),
        );
        self.font_small = CreateFontW(
            small,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            0,
            0,
            CLEARTYPE_QUALITY.0 as u32,
            0,
            winutil::wide_ptr(&face),
        );
    }

    unsafe fn rebuild_fonts(&mut self) {
        for f in [self.font, self.font_small] {
            if !f.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(f.0));
            }
        }
        self.font = HFONT(std::ptr::null_mut());
        self.font_small = HFONT(std::ptr::null_mut());
        self.ensure_fonts();
    }

    fn cancel_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        capture::cancel_pending();
    }

    fn hide(&mut self, reason: &str) {
        if self.quitting {
            return;
        }
        if debug_enabled() {
            eprintln!("[hide] {reason} (visible={})", self.visible);
        }
        self.cancel_pending();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = ReleaseCapture();
        }
        hooks::set_toolbar_rect(None);
        self.visible = false;
        self.deferred = false;
        self.mode = Mode::Bar;
        self.hover = None;
        self.translation.clear();
        self.error.clear();
        self.panel_lines.clear();
        self.panel_content_height = 0;
        self.panel_viewport_height = 0;
        self.panel_scroll_y = 0;
        self.wheel_remainder = 0;
        self.panel_size = (0, 0);
        self.panel_has_scrollbar = false;
        self.scroll_drag = None;
    }

    unsafe fn apply_region(&self) {
        let mut rc = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rc);
        let radius = toolbar::scale(toolbar::BAR_RADIUS, self.scale);
        let region = CreateRoundRectRgn(0, 0, rc.right + 1, rc.bottom + 1, radius, radius);
        SetWindowRgn(self.hwnd, region, true);
    }

    fn refresh_palette(&mut self) {
        self.pal = palette();
    }

    /// 图标蒙版只与像素尺寸有关，按尺寸缓存。
    fn ensure_icons(&mut self) {
        let size = toolbar::scale(toolbar::ICON, self.scale).max(8) as usize;
        if self.icons.as_ref().map(|c| c.size) != Some(size) {
            let (copy_back, copy_front) = icon::copy_masks(size);
            self.icons = Some(IconCache {
                size,
                copy_back,
                copy_front,
                translate: icon::translate_mask(size),
            });
        }
    }

    unsafe fn place_window(&mut self, rect: RECT) {
        self.rect = rect;
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let _ = SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        self.apply_region();
        hooks::set_toolbar_rect(Some(rect));
        self.visible = true;
        let _ = InvalidateRect(self.hwnd, None, true);
    }

    unsafe fn show_bar(&mut self) {
        if debug_enabled() {
            eprintln!("[show] anchor=({}, {})", self.anchor.x, self.anchor.y);
        }
        self.ensure_fonts();
        self.refresh_palette();
        self.mode = Mode::Bar;
        self.hover = None;
        self.translation.clear();
        self.error.clear();
        self.shown_at = Instant::now();
        let work = winutil::work_area(self.anchor);
        let rect = toolbar::bar_rect(self.anchor, self.scale, work);
        self.place_window(rect);
        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(self.hwnd);
    }

    fn panel_body(&self) -> &str {
        match self.mode {
            Mode::Loading => "正在翻译…",
            Mode::Result => &self.translation,
            Mode::Error => &self.error,
            Mode::Bar => "",
        }
    }

    /// 根据当前正文重新计算面板宽高、换行结果和滚动范围。
    unsafe fn reflow_panel(&mut self) {
        self.ensure_fonts();
        let body = self.panel_body().to_string();
        let work = winutil::work_area(self.anchor);
        let pad = toolbar::scale(toolbar::PANEL_PAD, self.scale);
        let min_size = toolbar::panel_size(self.scale);
        let screen_w = (work.right - work.left - toolbar::SCREEN_MARGIN * 2).max(1);
        let screen_h = (work.bottom - work.top - toolbar::SCREEN_MARGIN * 2).max(1);
        let max_w = toolbar::scale(toolbar::PANEL_MAX_W, self.scale)
            .min(screen_w)
            .max(1);
        let max_h = toolbar::scale(toolbar::PANEL_MAX_H, self.scale)
            .min(screen_h)
            .max(1);
        let min_w = min_size.0.min(max_w);
        let min_h = min_size.1.min(max_h);

        let dc = GetDC(self.hwnd);
        if dc.0.is_null() {
            return;
        }
        let natural_width = body
            .split('\n')
            .map(|line| measure(dc, self.font, line))
            .max()
            .unwrap_or(0);
        let mut width = (natural_width + pad * 2).max(min_w).min(max_w);
        let mut has_scrollbar = false;
        let mut lines = Vec::new();
        let mut content_height = 0;
        for _ in 0..2 {
            let body_width = (width
                - pad * 2
                - if has_scrollbar {
                    toolbar::scale(
                        toolbar::PANEL_SCROLLBAR_W + toolbar::PANEL_SCROLLBAR_GAP,
                        self.scale,
                    )
                } else {
                    0
                })
            .max(1);
            lines = wrap_text(dc, self.font, &body, body_width, usize::MAX);
            if lines.is_empty() {
                lines.push(String::new());
            }
            content_height = toolbar::scale(toolbar::PANEL_LINE_H, self.scale)
                * lines.len().min(i32::MAX as usize) as i32;
            let needed_height = pad
                + toolbar::scale(toolbar::PANEL_TITLE_H, self.scale)
                + content_height
                + toolbar::scale(
                    toolbar::PANEL_DIVIDER_GAP + toolbar::PANEL_FOOTER_H,
                    self.scale,
                );
            let next_scrollbar = needed_height > max_h;
            if next_scrollbar == has_scrollbar {
                break;
            }
            has_scrollbar = next_scrollbar;
        }
        let _ = ReleaseDC(self.hwnd, dc);

        // `height` may have been calculated before the second wrapping pass; keep
        // it consistent with the final line count.
        let needed_height = pad
            + toolbar::scale(toolbar::PANEL_TITLE_H, self.scale)
            + content_height
            + toolbar::scale(
                toolbar::PANEL_DIVIDER_GAP + toolbar::PANEL_FOOTER_H,
                self.scale,
            );
        let mut height = needed_height.max(min_h).min(max_h);
        let size = toolbar::panel_size_for_content(width, height, self.scale);
        width = size.0.min(max_w).max(1);
        height = size.1.min(max_h).max(1);
        self.panel_lines = lines;
        self.panel_content_height = content_height;
        self.panel_size = (width, height);
        self.panel_has_scrollbar = has_scrollbar;
        let client = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let body_rect = toolbar::panel_body_rect(client, self.scale, has_scrollbar);
        self.panel_viewport_height = (body_rect.bottom - body_rect.top).max(1);
        self.panel_scroll_y = self.max_scroll().min(self.panel_scroll_y.max(0));
    }

    fn max_scroll(&self) -> i32 {
        (self.panel_content_height - self.panel_viewport_height).max(0)
    }

    fn scroll_to(&mut self, offset: i32) {
        let next = offset.clamp(0, self.max_scroll());
        if next != self.panel_scroll_y {
            self.panel_scroll_y = next;
            unsafe {
                let _ = InvalidateRect(self.hwnd, None, true);
            }
        }
    }

    fn scroll_by(&mut self, delta: i32) {
        self.scroll_to(self.panel_scroll_y.saturating_add(delta));
    }

    unsafe fn switch_to_panel(&mut self) {
        self.reflow_panel();
        let work = winutil::work_area(self.anchor);
        let rect = toolbar::panel_rect_for_size(self.anchor, self.panel_size, work);
        self.place_window(rect);
        let _ = UpdateWindow(self.hwnd);
    }

    unsafe fn show_demo(&mut self) {
        let work = winutil::work_area(POINT { x: 0, y: 0 });
        self.text = "whelper 划词工具条（demo）".to_string();
        self.anchor = POINT {
            x: (work.left + work.right) / 2,
            y: (work.top + work.bottom) / 2,
        };
        self.target = self.hwnd;
        self.show_bar();
    }

    fn start_capture(&mut self, point: POINT, settle_ms: u64) {
        self.start_capture_action(point, settle_ms, CaptureAction::ShowBar);
    }

    fn start_capture_action(&mut self, point: POINT, settle_ms: u64, action: CaptureAction) {
        self.cancel_pending();
        self.capture_anchor = point;
        let generation = self.generation;
        let hwnd_raw = self.hwnd.0 as isize;
        let target = if action == CaptureAction::ShowBar {
            winutil::foreground_window()
        } else {
            self.target
        };
        let target_raw = target.0 as isize;
        // 自动触发不复制终端；显式点击按钮才允许延迟取词。
        let allow_keyboard =
            action != CaptureAction::ShowBar || !winutil::prefers_deferred_capture(target);
        let request = capture::Request::new(target);
        std::thread::spawn(move || {
            let outcome = capture::capture_selection_for(
                request,
                Duration::from_millis(settle_ms),
                Duration::from_millis(CAPTURE_TIMEOUT_MS),
                allow_keyboard,
                point,
            );
            let (text, deferred, bounds) = match outcome {
                capture::Outcome::Text { text, bounds } => (text, false, bounds),
                capture::Outcome::Deferred => (String::new(), true, None),
                capture::Outcome::None if action == CaptureAction::ShowBar => return,
                capture::Outcome::None => (String::new(), false, None),
            };
            let hwnd = HWND(hwnd_raw as *mut c_void);
            let target = HWND(target_raw as *mut c_void);
            let payload = Box::new(CapturePayload {
                text,
                deferred,
                bounds,
                anchor: point,
                target,
                generation,
                request,
                action,
            });
            unsafe {
                let _ = PostMessageW(
                    hwnd,
                    hooks::MSG_CAPTURE_DONE,
                    WPARAM(0),
                    LPARAM(Box::into_raw(payload) as isize),
                );
            }
        });
    }

    /// 延迟取词也必须在后台执行：UI 线程同时驱动低级键盘钩子，
    /// 同步 UIA/剪贴板等待会阻塞 Ctrl+C 放行和取消通知。
    fn resolve_deferred(&mut self, action: CaptureAction) -> bool {
        if !self.deferred {
            return false;
        }
        self.start_capture_action(self.capture_anchor, 0, action);
        true
    }

    fn start_translate(&mut self) {
        if self.resolve_deferred(CaptureAction::Translate) {
            return;
        }
        if self.text.trim().is_empty() {
            self.hide("translate-empty");
            return;
        }
        self.mode = Mode::Loading;
        self.hover = None;
        self.error.clear();
        self.translation.clear();
        self.panel_scroll_y = 0;
        self.wheel_remainder = 0;
        unsafe {
            self.switch_to_panel();
        }
        let generation = self.generation;
        let hwnd_raw = self.hwnd.0 as isize;
        let text = self.text.clone();
        std::thread::spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut c_void);
            let result = translate::translate_blocking(&text);
            let payload = Box::new(TranslatePayload { result, generation });
            unsafe {
                let _ = PostMessageW(
                    hwnd,
                    hooks::MSG_TRANSLATE_DONE,
                    WPARAM(0),
                    LPARAM(Box::into_raw(payload) as isize),
                );
            }
        });
    }

    fn copy_selection(&mut self) {
        if self.resolve_deferred(CaptureAction::Copy) {
            return;
        }
        if self.text.trim().is_empty() {
            self.hide("copy-empty");
            return;
        }
        if !clipboard::set_text(&self.text) {
            self.error = "复制失败：剪贴板被占用".to_string();
            return;
        }
        self.hide("copy-done");
    }

    fn open_translation_in_browser(&mut self) {
        let (source, target) = translate::language_pair(&self.text);
        let url = translate::browser_url(&self.text, source, target);
        let op = winutil::to_wide("open");
        let url_w = winutil::to_wide(&url);
        unsafe {
            ShellExecuteW(
                winutil::null_hwnd(),
                winutil::wide_ptr(&op),
                winutil::wide_ptr(&url_w),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
        self.hide("open-browser");
    }

    fn button_at(&self, pt: POINT) -> Option<usize> {
        if self.mode != Mode::Bar {
            return None;
        }
        let [copy, trans] = toolbar::button_rects(self.rect, self.scale);
        if toolbar::point_in(copy, pt) {
            Some(0)
        } else if toolbar::point_in(trans, pt) {
            Some(1)
        } else {
            None
        }
    }

    fn on_mouse_up(&mut self) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        match self.mode {
            Mode::Bar => match self.button_at(pt) {
                Some(0) => self.copy_selection(),
                Some(1) => self.start_translate(),
                _ => self.hide("click-elsewhere"),
            },
            Mode::Loading => {}
            Mode::Result | Mode::Error => {
                let was_dragging = self.scroll_drag.take().is_some();
                unsafe {
                    let _ = ReleaseCapture();
                }
                if was_dragging {
                    return;
                }
                if toolbar::point_in(self.panel_footer_rect(), pt) {
                    self.open_translation_in_browser();
                } else if toolbar::point_in(
                    toolbar::panel_scrollbar_rect(self.rect, self.scale),
                    pt,
                ) {
                    self.scroll_to(self.scroll_from_thumb_y(pt.y));
                } else {
                    self.hide("panel-click-elsewhere");
                }
            }
        }
    }

    fn panel_footer_rect(&self) -> RECT {
        let footer_y = self.rect.bottom - toolbar::scale(toolbar::PANEL_FOOTER_H, self.scale);
        RECT {
            left: self.rect.left,
            top: footer_y,
            right: self.rect.right,
            bottom: self.rect.bottom,
        }
    }

    fn scroll_from_thumb_y(&self, y: i32) -> i32 {
        let track = toolbar::panel_scrollbar_track_rect(self.rect, self.scale);
        let thumb = toolbar::panel_scrollbar_thumb_rect(
            self.rect,
            self.scale,
            self.panel_content_height,
            self.panel_viewport_height,
            self.panel_scroll_y,
        );
        let travel = (track.bottom - track.top - (thumb.bottom - thumb.top)).max(1);
        let position = (y - track.top - (thumb.bottom - thumb.top) / 2).clamp(0, travel);
        let max_scroll = self.max_scroll();
        if max_scroll == 0 {
            0
        } else {
            (position as i64 * max_scroll as i64 / travel as i64) as i32
        }
    }

    fn on_mouse_move(&mut self) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        if let Some(drag) = self.scroll_drag.as_ref() {
            let track = toolbar::panel_scrollbar_track_rect(self.rect, self.scale);
            let thumb = toolbar::panel_scrollbar_thumb_rect(
                self.rect,
                self.scale,
                self.panel_content_height,
                self.panel_viewport_height,
                self.panel_scroll_y,
            );
            let travel = (track.bottom - track.top - (thumb.bottom - thumb.top)).max(1);
            let position = (pt.y - track.top - drag.grab_offset).clamp(0, travel);
            let max_scroll = self.max_scroll();
            if max_scroll > 0 {
                self.scroll_to((position as i64 * max_scroll as i64 / travel as i64) as i32);
            }
            return;
        }
        if self.mode != Mode::Bar {
            return;
        }
        let hover = self.button_at(pt);
        if hover != self.hover {
            self.hover = hover;
            unsafe {
                let _ = InvalidateRect(self.hwnd, None, true);
            }
        }
    }

    fn on_mouse_down(&mut self) {
        if !matches!(self.mode, Mode::Result | Mode::Error) {
            return;
        }
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        let thumb = toolbar::panel_scrollbar_thumb_rect(
            self.rect,
            self.scale,
            self.panel_content_height,
            self.panel_viewport_height,
            self.panel_scroll_y,
        );
        if toolbar::point_in(thumb, pt) {
            self.scroll_drag = Some(ScrollDrag {
                grab_offset: pt.y - thumb.top,
            });
            unsafe {
                let _ = SetCapture(self.hwnd);
            }
        } else if toolbar::point_in(toolbar::panel_scrollbar_rect(self.rect, self.scale), pt) {
            self.scroll_to(self.scroll_from_thumb_y(pt.y));
        }
    }

    fn on_wheel(&mut self, delta: i32, pt: POINT) {
        if !self.visible {
            return;
        }
        if !matches!(self.mode, Mode::Result | Mode::Error) || !toolbar::point_in(self.rect, pt) {
            self.hide("wheel");
            return;
        }
        if self.max_scroll() == 0 {
            self.hide("wheel-no-scroll");
            return;
        }
        self.wheel_remainder = self.wheel_remainder.saturating_add(delta);
        let notches = self.wheel_remainder / WHEEL_DELTA;
        self.wheel_remainder %= WHEEL_DELTA;
        if notches != 0 {
            self.scroll_by(-notches * toolbar::scale(toolbar::PANEL_LINE_H * 3, self.scale));
        }
    }

    unsafe fn paint(&mut self) {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(self.hwnd, &mut ps);
        if hdc.0.is_null() {
            return;
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        if w <= 0 || h <= 0 {
            let _ = EndPaint(self.hwnd, &ps);
            return;
        }

        // 双缓冲，避免悬停重绘闪烁。
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, HGDIOBJ(bmp.0));
        // 卡片 = 1px 描边 + 内层填充（窗口区域已经把外圈裁成 10px 圆角）。
        fill_rect(mem, rc, self.pal.border);
        let bw = toolbar::scale(toolbar::BORDER, self.scale);
        let inner = RECT {
            left: bw,
            top: bw,
            right: w - bw,
            bottom: h - bw,
        };
        let inner_radius = (toolbar::scale(toolbar::BAR_RADIUS, self.scale) - bw).max(2);
        round_fill(mem, inner, inner_radius, self.pal.card);
        let _ = SetBkMode(mem, TRANSPARENT);

        self.ensure_fonts();
        match self.mode {
            Mode::Bar => self.paint_bar(mem),
            Mode::Loading => {
                let muted = self.pal.muted;
                self.paint_panel(mem, rc, "翻译", muted);
            }
            Mode::Result => {
                self.paint_panel(mem, rc, "翻译", self.pal.text);
            }
            Mode::Error => {
                self.paint_panel(mem, rc, "翻译失败", self.pal.error);
            }
        }

        let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        if !bmp.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(mem);
        let _ = EndPaint(self.hwnd, &ps);
    }

    unsafe fn paint_bar(&mut self, hdc: HDC) {
        let radius = toolbar::scale(toolbar::BTN_RADIUS, self.scale);
        let bar = self.rect;
        let [copy, trans] = toolbar::button_rects(bar, self.scale);
        let offset = |r: RECT| RECT {
            left: r.left - bar.left,
            top: r.top - bar.top,
            right: r.right - bar.left,
            bottom: r.bottom - bar.top,
        };
        self.ensure_icons();
        let pal = self.pal;
        let cache = self.icons.as_ref().expect("图标缓存已填充");
        for (idx, btn) in [copy, trans].iter().enumerate() {
            let btn_client = offset(*btn);
            // 按钮默认透明，仅 hover 时铺一层 6% 黑（暗色下为提亮）。
            if self.hover == Some(idx) {
                round_fill(hdc, btn_client, radius, pal.hover);
            }
            let irc = offset(toolbar::icon_rect(*btn, self.scale));
            match idx {
                0 => {
                    icon::paint(hdc, irc.left, irc.top, &cache.copy_back, pal.text);
                    icon::paint(hdc, irc.left, irc.top, &cache.copy_front, pal.text);
                }
                _ => icon::paint(hdc, irc.left, irc.top, &cache.translate, pal.text),
            }
            let label = if idx == 0 { "复制" } else { "翻译" };
            let (lx, _) = toolbar::label_origin(*btn, self.scale);
            draw_left_vcenter(hdc, self.font, label, lx - bar.left, btn_client, pal.text);
        }
    }

    unsafe fn paint_panel(&self, hdc: HDC, rc: RECT, title: &str, body_color: COLORREF) {
        let pad = toolbar::scale(toolbar::PANEL_PAD, self.scale);
        let title_y = pad;
        draw_left(hdc, self.font_small, title, pad, title_y, self.pal.muted);

        let body_rect = toolbar::panel_body_rect(rc, self.scale, self.panel_has_scrollbar);
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(
            hdc,
            body_rect.left,
            body_rect.top,
            body_rect.right,
            body_rect.bottom,
        );
        let line_h = toolbar::scale(toolbar::PANEL_LINE_H, self.scale);
        let mut y = body_rect.top - self.panel_scroll_y;
        for line in &self.panel_lines {
            draw_left(hdc, self.font, line, body_rect.left, y, body_color);
            y += line_h;
        }
        if saved != 0 {
            let _ = RestoreDC(hdc, saved);
        }

        if self.panel_has_scrollbar {
            let track = toolbar::panel_scrollbar_track_rect(rc, self.scale);
            let thumb = toolbar::panel_scrollbar_thumb_rect(
                rc,
                self.scale,
                self.panel_content_height,
                self.panel_viewport_height,
                self.panel_scroll_y,
            );
            fill_rect(hdc, track, self.pal.divider);
            round_fill(
                hdc,
                thumb,
                toolbar::scale(toolbar::PANEL_SCROLLBAR_W / 2, self.scale).max(1),
                self.pal.muted,
            );
        }

        // 底部提示与分隔线（1px divider）。
        let footer_y = rc.bottom - toolbar::scale(toolbar::PANEL_FOOTER_H, self.scale);
        let divider = RECT {
            left: pad,
            top: footer_y - toolbar::scale(toolbar::PANEL_DIVIDER_GAP, self.scale),
            right: rc.right - pad,
            bottom: footer_y - toolbar::scale(toolbar::PANEL_DIVIDER_GAP, self.scale)
                + toolbar::scale(1, self.scale).max(1),
        };
        fill_rect(hdc, divider, self.pal.divider);
        draw_left(
            hdc,
            self.font_small,
            "点击下方用浏览器打开 · Esc / 点击外部关闭",
            pad,
            footer_y,
            self.pal.muted,
        );
    }

    fn on_timer(&mut self) {
        // 设置页改了主题：立即刷新调色板（工具条可见时重绘）。
        let revision = config::revision();
        if self.config_revision != revision {
            self.config_revision = revision;
            self.refresh_palette();
            if self.visible {
                unsafe {
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
            }
        }

        // 关掉划词后，鼠标/键盘钩子仍挂着但会直接放行；这里负责收起还开着的工具条。
        if !config::get().picker.enabled {
            if self.visible {
                self.hide("picker-disabled");
            }
            return;
        }

        if !self.visible {
            return;
        }
        if self.shown_at.elapsed().as_millis() < SHOW_GRACE_MS as u128 {
            return;
        }
        let fg = winutil::foreground_window();
        let target = self.target;
        // 工具条自身永不激活，因此 fg 仍应是选区的来源窗口；
        // 若来源窗口已经销毁（关标签/关应用），也一并收起。
        let monitor_self = target == self.hwnd || target.0.is_null();
        if !monitor_self && (fg != target || !winutil::is_window(target)) {
            self.hide("foreground-changed");
        }
    }

    /// 工具条右键菜单：打开设置 / 退出（与托盘菜单同一入口）。
    fn show_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            // `--demo` 等没有托盘外壳的场景不显示“设置”。
            if applink::find_by_class(applink::SHELL_CLASS).is_some() {
                let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_SETTINGS, w!("设置…"));
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("退出 whelper"));

            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            // 工具条是 NOACTIVATE 的：仍要把“前台所有者”设成自己，点空白处菜单才会收起。
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
                MENU_OPEN_SETTINGS => {
                    self.hide("open-settings");
                    applink::open_settings();
                }
                MENU_QUIT => {
                    let _ = DestroyWindow(self.hwnd);
                }
                _ => {}
            }
        }
    }

    unsafe fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_MOUSEMOVE => {
                self.on_mouse_move();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                self.on_mouse_down();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.on_mouse_up();
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.show_menu();
                Some(LRESULT(0))
            }
            hooks::MSG_MOUSE_DOWN => {
                self.cancel_pending();
                self.hide("mouse-down");
                Some(LRESULT(0))
            }
            hooks::MSG_MOUSE_UP => {
                let trigger = wparam.0 != 0;
                let point = hooks::unpack_point(lparam);
                if trigger {
                    self.hide("new-selection");
                    self.start_capture(point, 0);
                }
                Some(LRESULT(0))
            }
            hooks::MSG_WHEEL => {
                let point = hooks::unpack_point(lparam);
                self.on_wheel(wparam.0 as i32, point);
                Some(LRESULT(0))
            }
            hooks::MSG_KEY_DOWN => {
                self.cancel_pending();
                if self.visible {
                    self.hide("key-down");
                }
                Some(LRESULT(0))
            }
            hooks::MSG_KEY_CANCEL => {
                self.cancel_pending();
                if self.visible {
                    self.hide("escape");
                }
                Some(LRESULT(0))
            }
            hooks::MSG_KEY_SELECT => {
                self.hide("select-shortcut");
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                // 等应用真正把选区应用完（Ctrl+A / Shift+方向键后的重绘）。
                self.start_capture(pt, 120);
                Some(LRESULT(0))
            }
            hooks::MSG_CAPTURE_DONE => {
                let payload = Box::from_raw(lparam.0 as *mut CapturePayload);
                let stale = payload.generation != self.generation || !payload.request.is_current();
                let fg_ok = payload.target.0.is_null()
                    || winutil::foreground_window() == payload.target
                    || !winutil::is_window(payload.target);
                if stale || !fg_ok {
                    return Some(LRESULT(0));
                }
                self.text = payload.text;
                self.deferred = payload.deferred && payload.action == CaptureAction::ShowBar;
                self.anchor = anchor_from_bounds(payload.anchor, payload.bounds);
                self.target = payload.target;
                match payload.action {
                    CaptureAction::ShowBar => self.show_bar(),
                    CaptureAction::Copy => self.copy_selection(),
                    CaptureAction::Translate => self.start_translate(),
                }
                Some(LRESULT(0))
            }
            hooks::MSG_TRANSLATE_DONE => {
                let payload = Box::from_raw(lparam.0 as *mut TranslatePayload);
                if payload.generation != self.generation {
                    return Some(LRESULT(0));
                }
                match payload.result {
                    Ok(text) => {
                        self.translation = text;
                        self.mode = Mode::Result;
                    }
                    Err(err) => {
                        self.error = err;
                        self.mode = Mode::Error;
                    }
                }
                self.panel_scroll_y = 0;
                self.wheel_remainder = 0;
                self.switch_to_panel();
                Some(LRESULT(0))
            }
            MSG_DEMO => {
                self.show_demo();
                Some(LRESULT(0))
            }
            WM_TIMER => {
                if wparam.0 == TIMER_FOREGROUND {
                    self.on_timer();
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.rebuild_fonts();
                if self.visible {
                    if self.mode == Mode::Bar {
                        let work = winutil::work_area(self.anchor);
                        let rect = toolbar::bar_rect(self.anchor, self.scale, work);
                        self.place_window(rect);
                    } else {
                        self.switch_to_panel();
                    }
                }
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                let _ = DestroyWindow(self.hwnd);
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                hooks::uninstall();
                hooks::reset_state();
                if self.timer_on {
                    let _ = KillTimer(self.hwnd, TIMER_FOREGROUND);
                    self.timer_on = false;
                }
                PostQuitMessage(0);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        let app = create.lpCreateParams as *mut App;
        if !app.is_null() {
            (*app).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
        }
        return LRESULT(1);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if raw.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match (*raw).on_message(msg, wparam, lparam) {
        Some(result) => result,
        None => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run(demo: bool) -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(_) => return 1,
        };
        let hinstance = HINSTANCE(module.0);
        let class_name = winutil::to_wide(CLASS_NAME);
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hCursor: cursor,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: winutil::wide_ptr(&class_name),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return 1;
        }

        let app = Box::new(App::new());
        let app_ptr = Box::into_raw(app);
        let title = winutil::to_wide("划词工具条");
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE((WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE).0),
            winutil::wide_ptr(&class_name),
            winutil::wide_ptr(&title),
            WINDOW_STYLE(WS_POPUP.0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            10,
            10,
            winutil::null_hwnd(),
            HMENU(std::ptr::null_mut()),
            hinstance,
            Some(app_ptr as *const c_void),
        ) {
            Ok(h) => h,
            Err(_) => {
                drop(Box::from_raw(app_ptr));
                return 1;
            }
        };
        (*app_ptr).timer_on = SetTimer(hwnd, TIMER_FOREGROUND, 150, None) != 0;
        // 演示模式不装钩子：它只是 UI 预览，装了反而会被真实鼠标/滚轮事件立刻收起。
        if !demo && !hooks::install(hwnd) {
            eprintln!("[warn] 全局钩子安装失败，划词触发不可用");
        }
        if demo {
            let _ = PostMessageW(hwnd, MSG_DEMO, WPARAM(0), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, winutil::null_hwnd(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // 窗口已销毁，App 在这里释放。
        drop(Box::from_raw(app_ptr));
        0
    }
}

// ---------- 绘图辅助 ----------

fn fill_rect(hdc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush: HBRUSH = CreateSolidBrush(color);
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn round_fill(hdc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

unsafe fn measure(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = windows::Win32::Foundation::SIZE::default();
    let ok = GetTextExtentPoint32W(hdc, &wide, &mut size).as_bool();
    SelectObject(hdc, old);
    if ok {
        size.cx
    } else {
        0
    }
}

fn draw_left(hdc: HDC, font: HFONT, text: &str, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        SetTextColor(hdc, color);
        let wide: Vec<u16> = text.encode_utf16().collect();
        if !wide.is_empty() {
            let _ = TextOutW(hdc, x, y, &wide);
        }
        SelectObject(hdc, old);
    }
}

/// 左对齐、在 `rect` 内垂直居中绘制（按钮文字与图标基线对齐）。
fn draw_left_vcenter(hdc: HDC, font: HFONT, text: &str, x: i32, rect: RECT, color: COLORREF) {
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let wide: Vec<u16> = text.encode_utf16().collect();
        if wide.is_empty() {
            SelectObject(hdc, old);
            return;
        }
        let mut size = windows::Win32::Foundation::SIZE::default();
        if GetTextExtentPoint32W(hdc, &wide, &mut size).as_bool() {
            let y = rect.top + (rect.bottom - rect.top - size.cy) / 2;
            SetTextColor(hdc, color);
            let _ = TextOutW(hdc, x, y, &wide);
        }
        SelectObject(hdc, old);
    }
}

fn wrap_text(hdc: HDC, font: HFONT, text: &str, max_width: i32, max_lines: usize) -> Vec<String> {
    unsafe {
        let chars: Vec<char> = text.chars().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut i = 0usize;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\n' {
                lines.push(std::mem::take(&mut current));
                i += 1;
                if lines.len() == max_lines {
                    return ellipsis(lines, i < chars.len());
                }
                continue;
            }
            let mut probe = current.clone();
            probe.push(ch);
            if measure(hdc, font, &probe) > max_width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                if lines.len() == max_lines {
                    return ellipsis(lines, true);
                }
                current.push(ch);
            } else {
                current = probe;
            }
            i += 1;
        }
        if !current.is_empty() && lines.len() < max_lines {
            lines.push(current);
        }
        lines
    }
}

fn ellipsis(mut lines: Vec<String>, truncated: bool) -> Vec<String> {
    if truncated {
        if let Some(last) = lines.last_mut() {
            if !last.ends_with('…') {
                last.push('…');
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_uses_selection_top_center() {
        let pointer = POINT { x: 111, y: 222 };
        let bounds = Rect::new(86, 127, 167, 146);
        let anchor = anchor_from_bounds(pointer, Some(bounds));
        // 上边中点：多行选区也能把工具条居中放在选区上方。
        assert_eq!((anchor.x, anchor.y), (126, 127));
    }

    #[test]
    fn anchor_falls_back_to_pointer_without_bounds() {
        let pointer = POINT { x: 111, y: 222 };
        let anchor = anchor_from_bounds(pointer, None);
        assert_eq!((anchor.x, anchor.y), (111, 222));

        let degenerate = Rect::new(10, 10, 10, 10);
        let anchor = anchor_from_bounds(pointer, Some(degenerate));
        assert_eq!((anchor.x, anchor.y), (111, 222));
    }
}
