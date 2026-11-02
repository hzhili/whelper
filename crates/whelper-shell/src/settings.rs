//! 设置窗口：左侧分类 + 右侧即时生效的开关 / 分段选择 / 按钮。
//!
//! 全部自绘（GDI 双缓冲），而不是堆标准控件：
//!
//! - 与划词工具条、Alt+Tab 覆盖层共用同一套圆角、配色与 DPI 处理；
//! - 深色主题下不需要为原生控件做子类化；
//! - 绘制与点击共用 [`crate::plan::Plan`]，不会出现“看到的位置点不到”。
//!
//! 每次修改都通过 `whelper-config::update` 立即写盘，组件在自己的事件/定时器里读到新值；
//! 窗口本身只隐藏不销毁，从托盘再次打开是瞬时的。

use std::ffi::c_void;

use whelper_common::geometry::{Point, Rect};
use whelper_common::win32::string::{to_wide, wide_ptr};
use whelper_common::win32::{monitor, theme};
use whelper_config::{self as config, Config, Theme};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
    CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint,
    ExtCreatePen, FillRect, GetStockObject, InvalidateRect, Polyline, RestoreDC, RoundRect, SaveDC,
    ScreenToClient, SelectClipRgn, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, HBRUSH, HDC, HFONT, HGDIOBJ, LOGBRUSH, NULL_BRUSH, NULL_PEN, PAINTSTRUCT,
    PS_ENDCAP_ROUND, PS_GEOMETRIC, PS_JOIN_ROUND, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, VK_ESCAPE, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW,
    RegisterClassW, SendMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, HMENU, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND, MSG, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNORMAL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_SETCURSOR,
    WM_SETICON, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

/// `WM_MOUSELEAVE`（windows 0.58 把它放在 Controls 模块，这里直接写常量避免多开 feature）。
const WM_MOUSELEAVE: u32 = 0x02A3;

use crate::plan::{self, Action, Control, Page, Plan, RowLayout};
use crate::{autostart, icons};

const CLASS_NAME: &str = "WHelperSettings";
const TIMER_STATUS: usize = 1;
const STATUS_MS: u32 = 1800;

/// 客户区目标尺寸（逻辑像素）。
const CLIENT_W: i32 = 880;
const CLIENT_H: i32 = 640;

const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

#[derive(Clone, Copy)]
struct Palette {
    bg: COLORREF,
    sidebar: COLORREF,
    card: COLORREF,
    border: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
    accent_soft: COLORREF,
    hover: COLORREF,
    track: COLORREF,
    track_disabled: COLORREF,
    knob: COLORREF,
    segment_bg: COLORREF,
    error: COLORREF,
}

const LIGHT: Palette = Palette {
    bg: rgb(247, 248, 250),
    sidebar: rgb(255, 255, 255),
    card: rgb(255, 255, 255),
    border: rgb(226, 228, 232),
    text: rgb(32, 32, 32),
    muted: rgb(118, 121, 128),
    accent: rgb(47, 111, 235),
    accent_soft: rgb(232, 240, 254),
    hover: rgb(241, 244, 249),
    track: rgb(203, 207, 214),
    track_disabled: rgb(224, 226, 230),
    knob: rgb(255, 255, 255),
    segment_bg: rgb(240, 242, 246),
    error: rgb(206, 66, 66),
};

const DARK: Palette = Palette {
    bg: rgb(30, 31, 34),
    sidebar: rgb(24, 25, 28),
    card: rgb(42, 43, 47),
    border: rgb(62, 64, 70),
    text: rgb(232, 233, 236),
    muted: rgb(152, 156, 164),
    accent: rgb(86, 148, 255),
    accent_soft: rgb(34, 52, 82),
    hover: rgb(52, 54, 60),
    track: rgb(92, 96, 104),
    track_disabled: rgb(64, 66, 72),
    knob: rgb(245, 246, 248),
    segment_bg: rgb(33, 34, 38),
    error: rgb(255, 120, 120),
};

struct Fonts {
    brand: HFONT,
    title: HFONT,
    body: HFONT,
    bold: HFONT,
    small: HFONT,
}

impl Fonts {
    fn empty() -> Self {
        let null = HFONT(std::ptr::null_mut());
        Self { brand: null, title: null, body: null, bold: null, small: null }
    }

    unsafe fn destroy(&mut self) {
        for font in [self.brand, self.title, self.body, self.bold, self.small] {
            if !font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
        *self = Self::empty();
    }
}

struct Status {
    text: String,
    error: bool,
}

pub struct Settings {
    hwnd: HWND,
    scale: f32,
    font_scale: f32,
    fonts: Fonts,
    page: Page,
    hover: Option<Action>,
    tracking: bool,
    status: Option<Status>,
    timer_on: bool,
    autostart: bool,
}

impl Settings {
    /// 创建（隐藏的）设置窗口。`large` / `small` 用于标题栏与任务栏图标。
    pub fn create(large: Option<windows::Win32::UI::WindowsAndMessaging::HICON>, small: Option<windows::Win32::UI::WindowsAndMessaging::HICON>) -> Option<Box<Self>> {
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let hinstance = HINSTANCE(module.0);
            let class_name = to_wide(CLASS_NAME);
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                hCursor: cursor,
                hIcon: large.unwrap_or_default(),
                hbrBackground: HBRUSH(std::ptr::null_mut()),
                lpszClassName: wide_ptr(&class_name),
                ..Default::default()
            };
            // 重复注册只是“类已存在”。
            let _ = RegisterClassW(&wc);

            let mut settings = Box::new(Self {
                hwnd: whelper_common::win32::null_hwnd(),
                scale: 1.0,
                font_scale: 0.0,
                fonts: Fonts::empty(),
                page: Page::General,
                hover: None,
                tracking: false,
                status: None,
                timer_on: false,
                autostart: autostart::is_enabled(),
            });
            let context = &mut *settings as *mut Self as *const c_void;
            let title = to_wide("whelper 设置");
            let style = window_style();
            let (width, height, x, y) = window_geometry(style, 1.0)?;
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                wide_ptr(&class_name),
                wide_ptr(&title),
                style,
                x,
                y,
                width,
                height,
                whelper_common::win32::null_hwnd(),
                HMENU(std::ptr::null_mut()),
                hinstance,
                Some(context),
            )
            .ok()?;

            settings.hwnd = hwnd;
            settings.scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            if let Some(icon) = large {
                SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(icon.0 as isize));
            }
            if let Some(icon) = small {
                SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(icon.0 as isize));
            }
            settings.apply_titlebar();
            Some(settings)
        }
    }

    /// 从托盘 / 命令行打开：刷新状态、居中到光标所在显示器并激活。
    pub fn show(&mut self) {
        self.autostart = autostart::is_enabled();
        self.status = None;
        unsafe {
            self.scale = GetDpiForWindow(self.hwnd) as f32 / 96.0;
            self.ensure_fonts();
            self.apply_titlebar();
            let style = window_style();
            if let Some((width, height, x, y)) = window_geometry(style, self.scale) {
                let _ = SetWindowPos(
                    self.hwnd,
                    whelper_common::win32::null_hwnd(),
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let _ = ShowWindow(self.hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = InvalidateRect(self.hwnd, None, true);
        }
    }

    /// 托盘里改了开机自启后同步勾选状态。
    pub fn refresh(&mut self) {
        self.autostart = autostart::is_enabled();
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, true);
        }
    }

    fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    // ---------- 状态 ----------

    fn palette(&self) -> Palette {
        let dark = match config::get().general.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => !theme::apps_use_light_theme(),
        };
        if dark {
            DARK
        } else {
            LIGHT
        }
    }

    fn apply_titlebar(&self) {
        let dark = self.palette().bg == DARK.bg;
        let value = BOOL(if dark { 1 } else { 0 });
        let raw = &value as *const BOOL as *const c_void;
        let size = std::mem::size_of::<BOOL>() as u32;
        unsafe {
            let mut result = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                raw,
                size,
            );
            // 旧版 Windows 10 用 19；失败时再试一次。
            if result.is_err() {
                result = DwmSetWindowAttribute(
                    self.hwnd,
                    windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(19),
                    raw,
                    size,
                );
            }
            // Windows 11：直接指定标题栏底色与文字色，保证深浅两套主题下标题都清晰。
            // 旧系统不支持这两个属性，失败时由上面的 immersive dark 兜底。
            let caption = self.palette().sidebar;
            let text = self.palette().text;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(35),
                &caption as *const _ as *const c_void,
                4,
            );
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(36),
                &text as *const _ as *const c_void,
                4,
            );
            if config::get().general.debug {
                eprintln!("[settings] 深色标题栏 {dark}: {result:?}");
            }
            // 窗口已经创建后 DWM 不一定立刻重画标题栏，强制刷新一次非客户区。
            let _ = SetWindowPos(
                self.hwnd,
                whelper_common::win32::null_hwnd(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    fn saved(&mut self, text: &str) {
        self.status = Some(Status { text: text.to_string(), error: false });
        self.start_status_timer();
    }

    fn failed(&mut self, text: &str) {
        self.status = Some(Status { text: text.to_string(), error: true });
        self.start_status_timer();
    }

    fn start_status_timer(&mut self) {
        unsafe {
            if SetTimer(self.hwnd, TIMER_STATUS, STATUS_MS, None) != 0 {
                self.timer_on = true;
            }
        }
    }

    fn stop_status_timer(&mut self) {
        if self.timer_on {
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_STATUS);
            }
            self.timer_on = false;
        }
    }

    // ---------- 动作 ----------

    fn is_active(&self, action: Action, config: &Config) -> bool {
        match action {
            Action::ToggleAutostart => self.autostart,
            Action::TogglePicker => config.picker.enabled,
            Action::ToggleDoubleClick => config.picker.double_click,
            Action::SetDragThreshold(value) => config.picker.drag_threshold == value,
            Action::Theme(theme) => config.general.theme == theme,
            Action::ToggleDesktop => config.desktop.enabled,
            Action::ToggleSwitcher => config.switcher.enabled,
            Action::ToggleSwitcherCursorMonitorOnly => config.switcher.cursor_monitor_only,
            Action::SetDefaultView(view) => config.switcher.default_view == view,
            Action::TogglePreviews => config.switcher.previews,
            Action::ToggleAnimation => config.switcher.animation,
            Action::ToggleDebug => config.general.debug,
            _ => false,
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Nav(page) => {
                self.page = page;
                self.hover = None;
            }
            Action::ToggleAutostart => {
                let next = !self.autostart;
                if autostart::set_enabled(next) {
                    self.autostart = autostart::is_enabled();
                    self.saved(if self.autostart { "已开启开机自启" } else { "已关闭开机自启" });
                } else {
                    self.failed("无法写入启动项");
                }
            }
            Action::OpenConfigDir => self.open_config_dir(),
            Action::ResetDefaults => {
                match config::reset() {
                    Ok(()) => self.saved("已恢复默认设置"),
                    Err(error) => self.failed(&error),
                }
                self.autostart = autostart::is_enabled();
                self.apply_titlebar();
            }
            _ => {
                let result = config::update(|config| match action {
                    Action::TogglePicker => config.picker.enabled = !config.picker.enabled,
                    Action::ToggleDoubleClick => {
                        config.picker.double_click = !config.picker.double_click;
                    }
                    Action::SetDragThreshold(value) => config.picker.drag_threshold = value,
                    Action::Theme(theme) => config.general.theme = theme,
                    Action::ToggleDesktop => config.desktop.enabled = !config.desktop.enabled,
                    Action::ToggleSwitcher => config.switcher.enabled = !config.switcher.enabled,
                    Action::ToggleSwitcherCursorMonitorOnly => {
                        config.switcher.cursor_monitor_only = !config.switcher.cursor_monitor_only;
                    }
                    Action::SetDefaultView(view) => config.switcher.default_view = view,
                    Action::TogglePreviews => config.switcher.previews = !config.switcher.previews,
                    Action::ToggleAnimation => {
                        config.switcher.animation = !config.switcher.animation;
                    }
                    Action::ToggleDebug => config.general.debug = !config.general.debug,
                    _ => {}
                });
                match result {
                    Ok(()) => self.saved("已保存"),
                    Err(error) => self.failed(&error),
                }
                self.apply_titlebar();
            }
        }
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, true);
        }
    }

    fn open_config_dir(&mut self) {
        let path = config::config_path();
        // 先确保文件存在，打开目录时能看到它。
        let _ = config::save_to(&path, &config::get());
        let dir = path.parent().map(|dir| dir.display().to_string()).unwrap_or_default();
        let op = to_wide("open");
        let target = to_wide(&dir);
        unsafe {
            ShellExecuteW(
                self.hwnd,
                wide_ptr(&op),
                wide_ptr(&target),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    // ---------- 命中测试 ----------

    fn build_plan(&self) -> Plan {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        plan::plan(
            self.page,
            &config::get(),
            self.autostart,
            Rect::new(0, 0, rc.right, rc.bottom),
            self.scale,
        )
    }

    fn hit_test(&self, point: Point) -> Option<Action> {
        let plan = self.build_plan();
        for item in &plan.nav {
            if item.rect.contains(point) {
                return Some(Action::Nav(item.page));
            }
        }
        for (rect, action) in plan.rows.iter().flat_map(|row| {
            row.controls
                .iter()
                .filter(move |_| !row.disabled)
                .map(|control| (control.rect, control.action))
        }) {
            if rect.contains(point) {
                return Some(action);
            }
        }
        for button in &plan.about_buttons {
            if button.rect.contains(point) {
                return Some(button.action);
            }
        }
        None
    }

    /// 光标位置换算成客户区坐标（布局与命中都用客户区坐标）。
    fn cursor_point(&self) -> Point {
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        Point::new(point.x, point.y)
    }

    // ---------- 绘制 ----------

    unsafe fn ensure_fonts(&mut self) {
        if !self.fonts.body.0.is_null() && (self.font_scale - self.scale).abs() < 0.01 {
            return;
        }
        self.fonts.destroy();
        let scale = self.scale.max(0.75);
        self.fonts = Fonts {
            brand: create_font(17.0, 700, scale),
            title: create_font(19.0, 600, scale),
            body: create_font(13.0, 400, scale),
            bold: create_font(13.0, 600, scale),
            small: create_font(11.0, 400, scale),
        };
        self.font_scale = self.scale;
    }

    unsafe fn paint(&mut self) {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(self.hwnd, &mut ps);
        if hdc.0.is_null() {
            return;
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rc);
        let (width, height) = (rc.right, rc.bottom);
        if width <= 0 || height <= 0 {
            let _ = EndPaint(self.hwnd, &ps);
            return;
        }

        self.ensure_fonts();
        let palette = self.palette();
        let config = config::get();
        let plan = plan::plan(
            self.page,
            &config,
            self.autostart,
            Rect::new(0, 0, width, height),
            self.scale,
        );

        let mem = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let old_bitmap = SelectObject(mem, HGDIOBJ(bitmap.0));
        fill_rect(mem, rc, palette.bg);

        self.paint_sidebar(mem, &plan, &palette, height);
        self.paint_content(mem, &plan, &config, &palette, width);
        self.paint_status(mem, &palette, width, height);

        let _ = BitBlt(hdc, 0, 0, width, height, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bitmap);
        if !bitmap.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        let _ = DeleteDC(mem);
        let _ = EndPaint(self.hwnd, &ps);
    }

    unsafe fn paint_sidebar(&self, hdc: HDC, plan: &Plan, palette: &Palette, height: i32) {
        let sidebar = scaled(plan::SIDEBAR_W, self.scale);
        fill_rect(hdc, RECT { left: 0, top: 0, right: sidebar, bottom: height }, palette.sidebar);
        fill_rect(
            hdc,
            RECT { left: sidebar - 1, top: 0, right: sidebar, bottom: height },
            palette.border,
        );

        let logo = scaled(34, self.scale);
        let logo_rect = RECT {
            left: scaled(20, self.scale),
            top: scaled(24, self.scale),
            right: scaled(20, self.scale) + logo,
            bottom: scaled(24, self.scale) + logo,
        };
        draw_logo(hdc, logo_rect);

        let text_left = logo_rect.right + scaled(10, self.scale);
        draw_text(
            hdc,
            self.fonts.brand,
            "whelper",
            RECT {
                left: text_left,
                top: scaled(23, self.scale),
                right: sidebar - scaled(8, self.scale),
                bottom: scaled(23 + 24, self.scale),
            },
            palette.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        draw_text(
            hdc,
            self.fonts.small,
            "桌面助手",
            RECT {
                left: text_left,
                top: scaled(47, self.scale),
                right: sidebar - scaled(8, self.scale),
                bottom: scaled(47 + 18, self.scale),
            },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        for item in &plan.nav {
            let rect = to_rect(item.rect);
            let selected = item.page == self.page;
            let hovered = self.hover == Some(Action::Nav(item.page));
            if selected {
                round_fill(hdc, rect, scaled(8, self.scale), palette.accent_soft);
                fill_rect(
                    hdc,
                    RECT {
                        left: rect.left,
                        top: rect.top + scaled(8, self.scale),
                        right: rect.left + scaled(3, self.scale),
                        bottom: rect.bottom - scaled(8, self.scale),
                    },
                    palette.accent,
                );
            } else if hovered {
                round_fill(hdc, rect, scaled(8, self.scale), palette.hover);
            }
            draw_text(
                hdc,
                if selected { self.fonts.bold } else { self.fonts.body },
                item.page.label(),
                RECT {
                    left: rect.left + scaled(14, self.scale),
                    top: rect.top,
                    right: rect.right - scaled(8, self.scale),
                    bottom: rect.bottom,
                },
                if selected { palette.accent } else { palette.text },
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
        }

        draw_text(
            hdc,
            self.fonts.small,
            "设置即时保存",
            RECT {
                left: scaled(20, self.scale),
                top: height - scaled(34, self.scale),
                right: sidebar - scaled(12, self.scale),
                bottom: height - scaled(14, self.scale),
            },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
    }

    unsafe fn paint_content(
        &self,
        hdc: HDC,
        plan: &Plan,
        config: &Config,
        palette: &Palette,
        width: i32,
    ) {
        let left = scaled(plan::CONTENT_LEFT, self.scale);
        let right = width - scaled(plan::CONTENT_PAD, self.scale);

        draw_text(
            hdc,
            self.fonts.title,
            self.page.title(),
            RECT {
                left,
                top: scaled(32, self.scale),
                right,
                bottom: scaled(32 + 30, self.scale),
            },
            palette.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        if self.page == Page::About {
            self.paint_about(hdc, plan, palette, left, right);
            return;
        }

        draw_text(
            hdc,
            self.fonts.small,
            self.page.subtitle(),
            RECT {
                left,
                top: scaled(64, self.scale),
                right,
                bottom: scaled(84, self.scale),
            },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        for row in &plan.rows {
            self.paint_row(hdc, row, config, palette);
        }
    }

    unsafe fn paint_row(&self, hdc: HDC, row: &RowLayout, config: &Config, palette: &Palette) {
        let rect = to_rect(row.rect);
        if !row.disabled {
            round_fill(hdc, rect, scaled(10, self.scale), palette.card);
            round_stroke(hdc, rect, scaled(10, self.scale), 1, palette.border);
        }

        let text_left = rect.left + scaled(16, self.scale);
        let controls_left = row
            .controls
            .first()
            .map(|control| control.rect.left - scaled(16, self.scale))
            .unwrap_or(rect.right - scaled(16, self.scale));

        draw_text(
            hdc,
            self.fonts.bold,
            row.spec.title,
            RECT {
                left: text_left,
                top: rect.top + scaled(14, self.scale),
                right: controls_left,
                bottom: rect.top + scaled(38, self.scale),
            },
            if row.disabled { palette.muted } else { palette.text },
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        draw_text(
            hdc,
            self.fonts.small,
            row.spec.desc,
            RECT {
                left: text_left,
                top: rect.top + scaled(40, self.scale),
                right: controls_left,
                bottom: rect.top + scaled(62, self.scale),
            },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        match row.spec.control {
            Control::Switch(action) => {
                let Some(hit) = row.controls.first() else { return };
                self.paint_switch(
                    hdc,
                    to_rect(hit.rect),
                    self.is_active(action, config),
                    row.disabled,
                    self.hover == Some(action),
                    palette,
                );
            }
            Control::Segment(options) => {
                self.paint_segment(hdc, options, row, config, palette);
            }
            Control::Button(label, action) => {
                let Some(hit) = row.controls.first() else { return };
                self.paint_button(
                    hdc,
                    to_rect(hit.rect),
                    label,
                    false,
                    self.hover == Some(action),
                    palette,
                );
            }
            Control::Hint => {}
        }
    }

    unsafe fn paint_switch(
        &self,
        hdc: HDC,
        rect: RECT,
        on: bool,
        disabled: bool,
        hovered: bool,
        palette: &Palette,
    ) {
        let height = rect.bottom - rect.top;
        let track = if disabled {
            palette.track_disabled
        } else if on {
            palette.accent
        } else {
            palette.track
        };
        round_fill(hdc, rect, height / 2, track);
        if hovered && !disabled {
            round_stroke(hdc, rect, height / 2, 1, palette.accent);
        }

        let diameter = height - scaled(6, self.scale);
        let inset = scaled(3, self.scale);
        let left = if on { rect.right - diameter - inset } else { rect.left + inset };
        let brush = CreateSolidBrush(palette.knob);
        let old = SelectObject(hdc, HGDIOBJ(brush.0));
        let _ = Ellipse(hdc, left, rect.top + inset, left + diameter, rect.top + inset + diameter);
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }

    unsafe fn paint_segment(
        &self,
        hdc: HDC,
        options: &[plan::SegmentOption],
        row: &RowLayout,
        config: &Config,
        palette: &Palette,
    ) {
        let Some(first) = row.controls.first() else { return };
        let Some(last) = row.controls.last() else { return };
        let container = RECT {
            left: first.rect.left - scaled(3, self.scale),
            top: first.rect.top - scaled(3, self.scale),
            right: last.rect.right + scaled(3, self.scale),
            bottom: first.rect.bottom + scaled(3, self.scale),
        };
        round_fill(hdc, container, scaled(9, self.scale), palette.segment_bg);

        for (option, hit) in options.iter().zip(row.controls.iter()) {
            let rect = to_rect(hit.rect);
            let active = self.is_active(option.action, config);
            let hovered = self.hover == Some(option.action);
            if active {
                round_fill(hdc, rect, scaled(7, self.scale), palette.card);
                round_stroke(hdc, rect, scaled(7, self.scale), 1, palette.border);
            } else if hovered {
                round_fill(hdc, rect, scaled(7, self.scale), palette.hover);
            }
            draw_text(
                hdc,
                if active { self.fonts.bold } else { self.fonts.body },
                option.label,
                rect,
                if row.disabled {
                    palette.muted
                } else if active {
                    palette.accent
                } else {
                    palette.text
                },
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
        }
    }

    unsafe fn paint_button(
        &self,
        hdc: HDC,
        rect: RECT,
        label: &str,
        primary: bool,
        hovered: bool,
        palette: &Palette,
    ) {
        let (fill, border, text) = if primary {
            (palette.accent, palette.accent, COLORREF(0x00FF_FFFF))
        } else if hovered {
            (palette.hover, palette.border, palette.text)
        } else {
            (palette.card, palette.border, palette.text)
        };
        round_fill(hdc, rect, scaled(8, self.scale), fill);
        round_stroke(hdc, rect, scaled(8, self.scale), 1, border);
        draw_text(
            hdc,
            self.fonts.body,
            label,
            rect,
            text,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }

    unsafe fn paint_about(
        &self,
        hdc: HDC,
        plan: &Plan,
        palette: &Palette,
        left: i32,
        right: i32,
    ) {
        let logo = scaled(84, self.scale);
        let top = scaled(108, self.scale);
        draw_logo(hdc, RECT { left, top, right: left + logo, bottom: top + logo });

        draw_text(
            hdc,
            self.fonts.brand,
            "whelper",
            RECT {
                left: left + logo + scaled(18, self.scale),
                top: top + scaled(8, self.scale),
                right,
                bottom: top + scaled(36, self.scale),
            },
            palette.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        draw_text(
            hdc,
            self.fonts.small,
            &format!("版本 {} · 一个进程里的三个桌面小工具", env!("CARGO_PKG_VERSION")),
            RECT {
                left: left + logo + scaled(18, self.scale),
                top: top + scaled(40, self.scale),
                right,
                bottom: top + scaled(64, self.scale),
            },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        let lines = [
            "· 划词工具条：选中文字后复制 / 翻译，UIA 优先、不动剪贴板",
            "· 单屏显示桌面：Win+D 只影响光标所在显示器",
            "· Alt+Tab 切换器：缩略图 / 网格两种视图，带窗口预览",
        ];
        let mut y = top + logo + scaled(22, self.scale);
        for line in lines {
            draw_text(
                hdc,
                self.fonts.body,
                line,
                RECT { left, top: y, right, bottom: y + scaled(24, self.scale) },
                palette.text,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
            y += scaled(26, self.scale);
        }

        let config_path = config::config_path();
        draw_text(
            hdc,
            self.fonts.small,
            "配置文件",
            RECT { left, top: y + scaled(10, self.scale), right, bottom: y + scaled(30, self.scale) },
            palette.muted,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        draw_text(
            hdc,
            self.fonts.small,
            &config_path.display().to_string(),
            RECT {
                left,
                top: y + scaled(30, self.scale),
                right,
                bottom: y + scaled(50, self.scale),
            },
            palette.text,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        for (index, button) in plan.about_buttons.iter().enumerate() {
            let label = if index == 0 { "打开配置目录" } else { "恢复默认设置" };
            self.paint_button(
                hdc,
                to_rect(button.rect),
                label,
                index == 0,
                self.hover == Some(button.action),
                palette,
            );
        }
    }

    unsafe fn paint_status(&self, hdc: HDC, palette: &Palette, width: i32, height: i32) {
        let Some(status) = &self.status else { return };
        draw_text(
            hdc,
            self.fonts.small,
            &status.text,
            RECT {
                left: width - scaled(360, self.scale),
                top: height - scaled(34, self.scale),
                right: width - scaled(24, self.scale),
                bottom: height - scaled(12, self.scale),
            },
            if status.error { palette.error } else { palette.muted },
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }

    // ---------- 消息 ----------

    fn on_mouse_move(&mut self) {
        if !self.tracking {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            self.tracking = true;
        }
        let hover = self.hit_test(self.cursor_point());
        if hover != self.hover {
            self.hover = hover;
            unsafe {
                let _ = InvalidateRect(self.hwnd, None, true);
            }
        }
    }

    fn update_cursor(&self) {
        let hover = self.hit_test(self.cursor_point());
        let cursor = unsafe {
            if hover.is_some() {
                LoadCursorW(None, IDC_HAND)
            } else {
                LoadCursorW(None, IDC_ARROW)
            }
        };
        if let Ok(cursor) = cursor {
            unsafe {
                SetCursor(cursor);
            }
        }
    }

    unsafe fn on_message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_PAINT => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_MOUSEMOVE => {
                self.on_mouse_move();
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                if self.hover.is_some() {
                    self.hover = None;
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                // 用消息自带的客户区坐标，而不是全局光标：即使消息是转发/合成来的，
                // 点击位置也与鼠标按下时看到的一致。
                if let Some(action) = self.hit_test(message_point(lparam)) {
                    self.apply(action);
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                self.update_cursor();
                Some(LRESULT(1))
            }
            WM_KEYDOWN => {
                if wparam.0 as u16 == VK_ESCAPE.0 {
                    self.hide();
                }
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                self.hide();
                Some(LRESULT(0))
            }
            WM_TIMER => {
                if wparam.0 == TIMER_STATUS {
                    self.stop_status_timer();
                    self.status = None;
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.scale = GetDpiForWindow(self.hwnd) as f32 / 96.0;
                self.ensure_fonts();
                let suggested = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    self.hwnd,
                    whelper_common::win32::null_hwnd(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = InvalidateRect(self.hwnd, None, true);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        unsafe {
            self.fonts.destroy();
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ---------- 窗口与绘制辅助 ----------

fn window_style() -> WINDOW_STYLE {
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
}

/// 返回外框尺寸与居中位置（光标所在显示器的工作区）。
fn window_geometry(style: WINDOW_STYLE, scale: f32) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: scaled(CLIENT_W, scale),
            bottom: scaled(CLIENT_H, scale),
        };
        if AdjustWindowRectEx(&mut rect, style, BOOL(0), WINDOW_EX_STYLE(0)).is_err() {
            return None;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let work = monitor::work_area(Point::new(point.x, point.y))
            .unwrap_or(Rect::new(0, 0, 1280, 720));
        let x = work.left + (work.width() - width) / 2;
        let y = work.top + (work.height() - height) / 2;
        Some((width, height, x, y))
    }
}

fn scaled(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round() as i32
}

/// 鼠标消息 `lparam` 里的客户区坐标（有符号 16 位，支持负坐标）。
fn message_point(lparam: LPARAM) -> Point {
    let packed = lparam.0 as u32;
    let x = (packed & 0xFFFF) as u16 as i16 as i32;
    let y = (packed >> 16) as u16 as i16 as i32;
    Point::new(x, y)
}

fn to_rect(rect: Rect) -> RECT {
    RECT { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
}

unsafe fn create_font(size: f32, weight: i32, scale: f32) -> HFONT {
    let face = to_wide("Microsoft YaHei UI");
    CreateFontW(
        -((size * scale).round() as i32),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        0,
        0,
        CLEARTYPE_QUALITY.0 as u32,
        0,
        wide_ptr(&face),
    )
}

fn fill_rect(hdc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn round_fill(hdc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn round_stroke(hdc: HDC, rect: RECT, radius: i32, width: i32, color: COLORREF) {
    unsafe {
        let pen = CreatePen(PS_SOLID, width, color);
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

unsafe fn draw_text(
    hdc: HDC,
    font: HFONT,
    text: &str,
    rect: RECT,
    color: COLORREF,
    format: DRAW_TEXT_FORMAT,
) {
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    SetTextColor(hdc, color);
    SetBkMode(hdc, TRANSPARENT);
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rc = rect;
    DrawTextW(hdc, &mut wide, &mut rc, format);
    SelectObject(hdc, old);
}

/// 品牌标记：圆角方块渐变 + 白色 W（与 `tools/make-icon.ps1` 同一形状）。
unsafe fn draw_logo(hdc: HDC, rect: RECT) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 2 || height <= 2 {
        return;
    }
    let radius = (width as f32 * 0.22).round() as i32;
    let saved = SaveDC(hdc);
    let region = CreateRoundRectRgn(
        rect.left,
        rect.top,
        rect.right + 1,
        rect.bottom + 1,
        radius * 2,
        radius * 2,
    );
    SelectClipRgn(hdc, region);
    let _ = DeleteObject(HGDIOBJ(region.0));

    // 逐行填渐变，避免额外依赖 msimg32。
    let from = (47.0, 111.0, 235.0);
    let to = (124.0, 58.0, 237.0);
    for y in 0..height.max(1) {
        let t = if height <= 1 { 0.0 } else { y as f32 / (height - 1) as f32 };
        let r = from.0 + (to.0 - from.0) * t;
        let g = from.1 + (to.1 - from.1) * t;
        let b = from.2 + (to.2 - from.2) * t;
        fill_rect(
            hdc,
            RECT { left: rect.left, top: rect.top + y, right: rect.right, bottom: rect.top + y + 1 },
            rgb(r as u32, g as u32, b as u32),
        );
    }

    let stroke = ((width as f32 * 0.135).round() as u32).max(1);
    let log_brush = LOGBRUSH {
        lbStyle: windows::Win32::Graphics::Gdi::BS_SOLID,
        lbColor: COLORREF(0x00FF_FFFF),
        lbHatch: 0,
    };
    let pen = ExtCreatePen(PS_GEOMETRIC | PS_ENDCAP_ROUND | PS_JOIN_ROUND, stroke, &log_brush, None);

    let points: Vec<POINT> = [
        (0.165, 0.29),
        (0.335, 0.735),
        (0.5, 0.455),
        (0.665, 0.735),
        (0.835, 0.29),
    ]
    .iter()
    .map(|(x, y)| POINT {
        x: rect.left + (x * width as f32) as i32,
        y: rect.top + (y * height as f32) as i32,
    })
    .collect();

    let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let _ = Polyline(hdc, &points);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(pen.0));
    let _ = RestoreDC(hdc, saved);
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        let settings = create.lpCreateParams as *mut Settings;
        if !settings.is_null() {
            (*settings).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, settings as isize);
        }
        // 必须继续交给 DefWindowProc：窗口标题、图标等非客户区信息在它里面初始化。
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Settings;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    match (*raw).on_message(message, wparam, lparam) {
        Some(result) => result,
        None => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// 设置窗口的最小消息循环（只给 `--settings` 之类的独立调试用）。
pub fn run_standalone() -> i32 {
    let Some(mut settings) = Settings::create(icons::large(), icons::small()) else {
        eprintln!("无法创建设置窗口");
        return 1;
    };
    settings.show();
    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, whelper_common::win32::null_hwnd(), 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(x: i32, y: i32) -> LPARAM {
        LPARAM(((y as u32) << 16 | (x as u32 & 0xFFFF)) as isize)
    }

    #[test]
    fn mouse_message_point_decodes_signed_coordinates() {
        assert_eq!(message_point(pack(811, 153)), Point::new(811, 153));
        assert_eq!(message_point(pack(-5, -9)), Point::new(-5, -9));
    }
}
