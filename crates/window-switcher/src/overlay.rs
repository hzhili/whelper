//! 覆盖窗口：两种视图的绘制、布局过渡动画、鼠标交互与消息处理。
//!
//! 默认缩略图视图用深色半透明托盘单行显示应用图标；网格视图逐窗口显示 DWM 实时预览卡片。
//! 当前项以蓝色边框和整卡片高亮标识，两种视图共用键盘、鼠标与动画交互。
//!
//! - `WS_EX_NOACTIVATE` + `WS_EX_TOOLWINDOW` + `WS_EX_TOPMOST`：弹出时不抢焦点，
//!   提交前键盘焦点仍在原应用上；
//! - `WS_EX_LAYERED` + `surface::Surface`（`UpdateLayeredWindow`）：
//!   托盘使用逐像素 alpha，整窗 alpha 控制淡入淡出；
//! - 窗口画面由 `DwmRegisterThumbnail` 实时合成，并随布局动画同步位置与透明度；
//! - 预览项采用直角矩形边框与深色托盘；选中项的整张卡片铺设强调色底，预览下方标题区也同步高亮。
//!
//! 每帧几何都来自 [`crate::layout`] 的插值结果：选择移动与视图切换时平滑布局，缩略图同步跟随。

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use whelper_common::geometry::{Point, Rect};
use whelper_common::win32::string::{to_wide, wide_ptr};
use whelper_common::win32::{null_hwnd, window as common_window};
use whelper_common::window::WindowId;
use whelper_config::{self as config, DefaultView};
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateRectRgn, CreateRoundRectRgn,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, FillRect, GetStockObject,
    GetTextExtentPoint32W, InvalidateRect, RoundRect, SelectObject, SetBkMode, SetStretchBltMode,
    SetTextColor, SetWindowRgn, StretchBlt, ValidateRect, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    HALFTONE, HDC, HFONT, HGDIOBJ, NULL_PEN, SRCCOPY, STRETCH_BLT_MODE, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    IsWindow, KillTimer, LoadCursorW, PostMessageW, RegisterClassW, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, CREATESTRUCTW, GWLP_USERDATA, HMENU, HWND_TOPMOST, IDC_ARROW,
    SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_PAINT,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::anim::{lerp_u8, Anim};
use crate::layout::{self, Layout};
use crate::model::{Session, Step, ViewMode, WindowEntry};
use crate::preview;
use crate::shortcut::{MSG_CANCEL, MSG_COMMIT, MSG_MODE, MSG_START, MSG_STEP};
use crate::surface::Surface;
use crate::win32 as platform;

const CLASS_NAME: &str = "WHelperWindowSwitcher";
const TIMER_ANIM: usize = 1;
/// 图标线程 → 覆盖窗口：一批应用图标取到了（整批投递，避免逐个图标触发重绘）。
const MSG_ICONS: u32 = WM_APP + 41;

const FADE_IN_MS: u64 = 110;
const FADE_OUT_MS: u64 = 90;
const OPEN_SCALE_MS: u64 = 110;
const STEP_MS: u64 = 120;
const MODE_MS: u64 = 160;

/// 入场缩放：原生 Alt+Tab 也是“轻微放大 + 淡入”，不是位移。
const OPEN_SCALE: f32 = 0.96;
/// Windows 11 选中卡片外圈的边框宽度；卡片内部也会同步使用选中底色。
const SELECTION_BORDER_WIN11: i32 = 2;
/// 选中卡片的 DWM 预览保留一部分透明度，让蓝色选中底贯穿整个卡片。
const SELECTED_THUMBNAIL_ALPHA_PERCENT: u16 = 82;
/// Windows 10 Alt+Tab 托盘的不透明度。
const PANEL_ALPHA_WIN10: u8 = 230;
/// Windows 11 Alt+Tab 托盘的不透明度。
const PANEL_ALPHA_WIN11: u8 = 224;
/// Windows 11 Alt+Tab 圆角面板半径。
const PANEL_RADIUS_WIN11: i32 = 8;
/// 标题行里的应用图标边长（逻辑像素）。
const TILE_ICON: i32 = 16;
/// 占位缩略图里应用图标的边长（逻辑像素）。
const PLACEHOLDER_ICON: i32 = 40;
/// 在高分辨率绘制面上放大绘制，再用 HALFTONE 缩回屏幕尺寸。
const ICON_RENDER_SCALE: i32 = 4;

/// 图标线程与界面之间的“代次”：换会话即作废旧任务，避免把上一批图标塞进新会话。
static PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(0);

const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

/// 取色板：Windows 原生 Alt+Tab 的深色托盘和蓝色选中态。
#[derive(Clone, Copy)]
struct Palette {
    panel: COLORREF,
    preview_border: COLORREF,
    selected_card: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
}

const NATIVE_PALETTE: Palette = Palette {
    panel: rgb(32, 32, 32),
    preview_border: rgb(78, 78, 78),
    // 比单独的蓝色边框更醒目，同时保留预览内容与白色标题的对比度。
    selected_card: rgb(0, 86, 150),
    text: rgb(255, 255, 255),
    muted: rgb(190, 190, 190),
    accent: rgb(0, 120, 215),
};

fn palette() -> Palette {
    NATIVE_PALETTE
}

/// 透明度动画：`from → to`。真实透明度在每次绘制时按时间轴现算，
/// 因此即使 `WM_TIMER` 被预览消息挤后，画面也不会卡在“全透明”。
struct Fade {
    from: u8,
    to: u8,
    anim: Anim,
}

/// 图标是目标窗口借给我们的 `HICON`（不负责释放），整批送回主线程只触发一次重绘。
struct IconsPayload {
    generation: u64,
    icons: Vec<(String, preview::AppIcon)>,
}

/// 覆盖窗口及其全部状态。
pub struct Overlay {
    hwnd: HWND,
    scale: f32,
    font: HFONT,
    font_small: HFONT,
    /// 创建当前字体时使用的缩放：每帧 `ensure_fonts` 靠它判断是否需要重建。
    font_scale: f32,
    /// 当前系统原生 Alt+Tab 的调色板。
    pal: Palette,
    /// 用户上次选择的视图；新会话沿用。
    mode: ViewMode,
    session: Option<Session>,
    /// 目标布局（动画终点）。
    layout: Option<Layout>,
    /// 动画起点（打开/移动选择/切换视图时的当前画面）。
    from_layout: Option<Layout>,
    layout_anim: Option<Anim>,
    fade: Option<Fade>,
    alpha: u8,
    hover: Option<usize>,
    /// DWM 实时缩略图（句柄, 所属窗口）：会话期间注册，绘制时逐帧同步。
    thumbnails: Vec<(isize, WindowId)>,
    /// 注册失败的窗口（受保护内容等）：本会话内不再重试。
    thumb_failed: HashSet<WindowId>,
    icons: HashMap<String, preview::AppIcon>,
    generation: u64,
    timer_on: bool,
    /// 已提交/取消、正在淡出：期间忽略按键与鼠标。
    closing: bool,
    /// 本次会话是否抓取窗口预览（设置页开关，打开时读取）。
    previews_enabled: bool,
    /// 本次会话是否播放淡入 / 位移过渡（设置页开关，打开时读取）。
    animations: bool,
    /// 入场动画还没跨出第一帧：首次绘制可能很慢，动画时钟要从第一个 tick 起算。
    anim_deferred: bool,
}

impl Overlay {
    fn new() -> Self {
        Self {
            hwnd: null_hwnd(),
            scale: 1.0,
            font: HFONT(std::ptr::null_mut()),
            font_small: HFONT(std::ptr::null_mut()),
            font_scale: 0.0,
            pal: NATIVE_PALETTE,
            mode: ViewMode::Thumbnails,
            session: None,
            layout: None,
            from_layout: None,
            layout_anim: None,
            fade: None,
            alpha: 0,
            hover: None,
            thumbnails: Vec::new(),
            thumb_failed: HashSet::new(),
            icons: HashMap::new(),
            generation: 0,
            timer_on: false,
            closing: false,
            previews_enabled: true,
            animations: true,
            anim_deferred: false,
        }
    }

    /// 创建隐藏的覆盖窗口。`Box` 保证地址稳定：窗口的 `GWLP_USERDATA` 指向它。
    pub fn create() -> Option<Box<Self>> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let module = GetModuleHandleW(None).ok()?;
            let hinstance = HINSTANCE(module.0);
            let class_name = to_wide(CLASS_NAME);
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                hCursor: cursor,
                lpszClassName: wide_ptr(&class_name),
                ..Default::default()
            };
            // 重复注册只是“类已存在”，不影响后续 CreateWindowExW。
            let _ = RegisterClassW(&wc);

            let mut overlay = Box::new(Self::new());
            let context = &mut *overlay as *mut Self as *const c_void;
            let title = to_wide("窗口切换");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(
                    (WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED).0,
                ),
                wide_ptr(&class_name),
                wide_ptr(&title),
                WINDOW_STYLE(WS_POPUP.0),
                0,
                0,
                10,
                10,
                null_hwnd(),
                HMENU(std::ptr::null_mut()),
                hinstance,
                Some(context),
            )
            .ok()?;
            overlay.hwnd = hwnd;
            Some(overlay)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    // ---------- 会话生命周期 ----------

    fn start_session(&mut self, forward: bool) {
        self.reset_session();
        self.pal = palette();
        self.scale = platform::cursor_scale();
        unsafe { self.ensure_fonts() };

        // 每次打开都从设置页取一次偏好：默认视图 / 预览 / 动画即时生效。
        let settings = config::get();
        self.mode = match settings.switcher.default_view {
            DefaultView::Thumbnails => ViewMode::Thumbnails,
            DefaultView::Grid => ViewMode::Grid,
        };
        self.previews_enabled = settings.switcher.previews;
        self.animations = settings.switcher.animation;

        let windows = platform::windows(settings.switcher.cursor_monitor_only);
        let Some(mut session) = Session::new(windows, self.mode) else {
            if platform::debug_enabled() {
                eprintln!("[switcher] 没有可切换的窗口");
            }
            return;
        };
        // 索引 0 是当前前台窗口：第一次 Alt+Tab 应该预选下一个（Shift 时上一个）。
        session.apply(if forward { Step::Next } else { Step::Prev }, 1);
        let selected = session.selected();
        let len = session.len();
        let mode = session.mode();
        self.session = Some(session);
        self.closing = false;
        self.hover = None;

        let work = platform::cursor_work_area();
        let target = layout::compute(mode, len, selected, self.scale, work);
        // 入场：从 96% 轻微放大到 100%，同时淡入（与原生 Alt+Tab 一致，不做位移）。
        self.from_layout = Some(layout::scaled(&target, OPEN_SCALE));
        self.layout = Some(target.clone());
        let now = Instant::now();
        if self.animations {
            self.layout_anim = Some(Anim::new(now, Duration::from_millis(OPEN_SCALE_MS)));
            self.fade = Some(Fade {
                from: 0,
                to: 255,
                anim: Anim::new(now, Duration::from_millis(FADE_IN_MS)),
            });
            self.anim_deferred = true;
        } else {
            self.layout_anim = None;
            self.fade = None;
            self.anim_deferred = false;
            self.alpha = 255;
        }

        if platform::debug_enabled() {
            eprintln!(
                "[switcher] 打开会话：{len} 个候选，选中 {}（{mode:?}）",
                target.selected
            );
        }

        // 有动画时先以 alpha=0 摆好窗口与区域，避免第一帧闪一下未淡入的白块。
        self.set_alpha(if self.animations { 0 } else { 255 });
        unsafe {
            self.apply_geometry(&target);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.start_timer();
        self.start_icons();
        self.invalidate();
    }

    fn step(&mut self, step: Step) {
        {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let columns = self
                .layout
                .as_ref()
                .map(|layout| layout.columns)
                .unwrap_or(1);
            session.apply(step, columns);
            if platform::debug_enabled() {
                eprintln!(
                    "[switcher] 选择 {}/{}",
                    session.selected() + 1,
                    session.len()
                );
            }
        }
        self.relayout(Duration::from_millis(STEP_MS));
    }

    fn toggle_mode(&mut self) {
        {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let mode = session.toggle_mode();
            self.mode = mode;
            if platform::debug_enabled() {
                eprintln!("[switcher] 视图：{}", mode.label());
            }
        }
        self.relayout(Duration::from_millis(MODE_MS));
    }

    fn commit(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        let id = self
            .session
            .as_ref()
            .and_then(|session| session.selected_window())
            .map(|window| window.id);
        if let Some(id) = id {
            let activated = platform::activate(id);
            if platform::debug_enabled() {
                eprintln!(
                    "[switcher] 提交 {id}：激活{}",
                    if activated {
                        "成功"
                    } else {
                        "失败（可能被 UIPI 拒绝）"
                    }
                );
            }
        }
        self.fade_out();
    }

    fn cancel(&mut self) {
        if self.closing || self.session.is_none() {
            return;
        }
        self.closing = true;
        if platform::debug_enabled() {
            eprintln!("[switcher] 取消");
        }
        self.fade_out();
    }

    fn fade_out(&mut self) {
        if !self.animations {
            self.finish();
            return;
        }
        let now = Instant::now();
        self.fade = Some(Fade {
            from: self.alpha,
            to: 0,
            anim: Anim::new(now, Duration::from_millis(FADE_OUT_MS)),
        });
        self.start_timer();
    }

    /// 隐藏窗口并丢掉会话状态（不激活任何窗口）。
    fn finish(&mut self) {
        self.stop_timer();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.set_alpha(0);
        self.reset_session();
    }

    fn reset_session(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        // 图标线程按代次自行退出：已经取到的图标在下一轮循环开头丢弃。
        PREVIEW_GENERATION.store(self.generation, Ordering::SeqCst);
        self.session = None;
        self.layout = None;
        self.from_layout = None;
        self.layout_anim = None;
        self.hover = None;
        self.closing = false;
        unsafe { self.clear_thumbnails() };
        self.thumb_failed.clear();
        self.icons.clear();
    }

    /// 按新的选择/视图重算布局，并从“当前画面”平滑过渡到目标布局。
    fn relayout(&mut self, duration: Duration) {
        let Some((mode, len, selected)) = self
            .session
            .as_ref()
            .map(|session| (session.mode(), session.len(), session.selected()))
        else {
            return;
        };
        let previous = self.current_layout(Instant::now());
        let work = platform::cursor_work_area();
        let target = layout::compute(mode, len, selected, self.scale, work);
        self.from_layout = previous;
        self.hover = None;
        if self.animations {
            self.layout = Some(target);
            self.layout_anim = Some(Anim::new(Instant::now(), duration));
        } else {
            self.layout = Some(target.clone());
            self.layout_anim = None;
            unsafe { self.apply_geometry(&target) };
        }
        self.start_timer();
        self.invalidate();
    }

    /// 当前显示中的布局（动画中为插值结果）。
    fn current_layout(&self, now: Instant) -> Option<Layout> {
        let target = self.layout.as_ref()?;
        match (&self.from_layout, &self.layout_anim) {
            (Some(from), Some(anim)) => {
                let t = anim.eased(now);
                if t >= 1.0 {
                    Some(target.clone())
                } else {
                    Some(layout::lerp(from, target, t))
                }
            }
            _ => Some(target.clone()),
        }
    }

    /// 把窗口摆到插值后的面板矩形（窗口 = 面板），并同步窗口区域。
    unsafe fn apply_geometry(&self, layout: &Layout) {
        let panel = layout.panel;
        let width = panel.width().max(1);
        let height = panel.height().max(1);
        let _ = SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            panel.left,
            panel.top,
            width,
            height,
            SWP_NOACTIVATE,
        );
        let region = match platform::native_switcher_style() {
            platform::NativeSwitcherStyle::Windows10 => CreateRectRgn(0, 0, width + 1, height + 1),
            platform::NativeSwitcherStyle::Windows11 => {
                let radius = layout::scale(PANEL_RADIUS_WIN11, self.scale);
                CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2)
            }
        };
        let _ = SetWindowRgn(self.hwnd, region, true);
    }

    /// 后台按应用读取图标：优先取 Shell 大尺寸图标，失败时用 `WM_GETICON` 回退；
    /// 整批投递并按应用缓存，避免拖慢入场动画和重复读取同一应用图标。
    fn start_icons(&mut self) {
        let Some(apps) = self.session.as_ref().map(|session| {
            let mut seen = HashSet::new();
            session
                .windows()
                .iter()
                .filter(|window| seen.insert(window.app_id.clone()))
                .map(|window| (window.app_id.clone(), window.id))
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        let generation = self.generation;
        let hwnd_raw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut c_void);
            let mut icons = Vec::new();
            for (app_id, id) in &apps {
                if PREVIEW_GENERATION.load(Ordering::SeqCst) != generation {
                    break;
                }
                icons.push((app_id.clone(), preview::application_icon(app_id, *id)));
            }
            if !icons.is_empty() && PREVIEW_GENERATION.load(Ordering::SeqCst) == generation {
                let payload = Box::new(IconsPayload { generation, icons });
                let raw = Box::into_raw(payload);
                let posted =
                    unsafe { PostMessageW(hwnd, MSG_ICONS, WPARAM(0), LPARAM(raw as isize)) };
                if posted.is_err() {
                    drop(unsafe { Box::from_raw(raw) });
                }
            }
        });
    }

    fn on_icons(&mut self, lparam: LPARAM) {
        if lparam.0 == 0 {
            return;
        }
        let payload = unsafe { Box::from_raw(lparam.0 as *mut IconsPayload) };
        if payload.generation != self.generation || self.session.is_none() {
            return;
        }
        for (app_id, icon) in payload.icons {
            self.icons.insert(app_id, icon);
        }
        self.invalidate();
    }

    // ---------- DWM 实时缩略图 ----------

    /// 该窗口当前是否有可用的 DWM 缩略图（预览关闭时一律视为没有）。
    fn has_thumbnail(&self, id: WindowId) -> bool {
        self.previews_enabled && self.thumbnails.iter().any(|(_, window)| *window == id)
    }

    /// 让缩略图与当前布局、整窗透明度保持一致：注册新可见的窗口、注销滚出/
    /// 关闭的窗口，并逐帧同步位置（客户区坐标）与不透明度（跟随淡入淡出）。
    ///
    /// DWM 缩略图由合成器直接更新源窗口画面，不用抓屏；它画在绘制面之上且为直角矩形，
    /// 因此预览边框由 GDI 绘制在缩略图区域外围。
    unsafe fn sync_thumbnails(&mut self, layout: &Layout, origin: Point, alpha: u8) {
        let grid_mode = self
            .session
            .as_ref()
            .is_some_and(|session| session.mode() == ViewMode::Grid);
        if !self.previews_enabled || !grid_mode {
            self.clear_thumbnails();
            return;
        }
        let visible: Vec<(WindowId, RECT, bool)> = layout
            .cells
            .iter()
            .filter(|cell| layout.is_visible(cell.index))
            .filter_map(|cell| {
                let window = self.session.as_ref()?.windows().get(cell.index)?;
                Some((
                    window.id,
                    rel(cell.thumb, origin),
                    cell.index == layout.selected,
                ))
            })
            .collect();

        // 注销不再可见的；其余同步位置与不透明度（更新失败说明窗口已关闭）。
        let mut registered = std::mem::take(&mut self.thumbnails);
        for (thumbnail, id) in registered.drain(..) {
            let destination = visible
                .iter()
                .find(|(window, _, _)| *window == id)
                .map(|(_, rect, selected)| (*rect, *selected));
            match destination {
                Some((rect, selected))
                    if self.update_thumbnail(thumbnail, rect, thumbnail_alpha(alpha, selected)) =>
                {
                    self.thumbnails.push((thumbnail, id));
                }
                _ => {
                    let _ = DwmUnregisterThumbnail(thumbnail);
                }
            }
        }

        // 注册新出现的窗口；注册失败的记入 failed，本会话内不再重试。
        for (id, destination, selected) in &visible {
            if self.thumbnails.iter().any(|(_, window)| window == id)
                || self.thumb_failed.contains(id)
            {
                continue;
            }
            match DwmRegisterThumbnail(self.hwnd, common_window::hwnd(*id)) {
                Ok(thumbnail) => {
                    if self.update_thumbnail(
                        thumbnail,
                        *destination,
                        thumbnail_alpha(alpha, *selected),
                    ) {
                        self.thumbnails.push((thumbnail, *id));
                    } else {
                        let _ = DwmUnregisterThumbnail(thumbnail);
                        self.thumb_failed.insert(*id);
                    }
                }
                Err(_) => {
                    self.thumb_failed.insert(*id);
                }
            }
        }
    }

    unsafe fn update_thumbnail(&self, thumbnail: isize, destination: RECT, alpha: u8) -> bool {
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: destination,
            opacity: alpha,
            fVisible: BOOL(1),
            fSourceClientAreaOnly: BOOL(0),
            ..Default::default()
        };
        DwmUpdateThumbnailProperties(thumbnail, &properties).is_ok()
    }

    /// 注销全部缩略图（会话结束/预览关闭时调用）。
    unsafe fn clear_thumbnails(&mut self) {
        for (thumbnail, _) in self.thumbnails.drain(..) {
            let _ = DwmUnregisterThumbnail(thumbnail);
        }
    }

    // ---------- 绘制 ----------

    /// 按当前缩放准备字体；缩放没变时直接复用，避免每次 Alt+Tab 都重建 GDI 字体。
    unsafe fn ensure_fonts(&mut self) {
        if !self.font.0.is_null() && (self.font_scale - self.scale).abs() < 0.01 {
            return;
        }
        for font in [self.font, self.font_small] {
            if !font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
        self.font = HFONT(std::ptr::null_mut());
        self.font_small = HFONT(std::ptr::null_mut());

        // 与系统 Alt+Tab 一致：Segoe UI（中文由 GDI 的字体链接回退到雅黑）。
        let scale = self.scale.max(0.5);
        let face = to_wide("Segoe UI");
        self.font = CreateFontW(
            -((12.0 * scale).round() as i32),
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
            wide_ptr(&face),
        );
        self.font_small = CreateFontW(
            -((11.0 * scale).round() as i32),
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
            wide_ptr(&face),
        );
        self.font_scale = self.scale;
    }

    /// 每帧重画到 32bpp 绘制面，再整体交给分层窗口。
    ///
    /// 绘制深色托盘和当前视图；网格窗口画面由 DWM 实时缩略图合成，
    /// 每帧同步预览位置（跟随布局动画）与不透明度（跟随淡入淡出）。
    unsafe fn paint(&mut self) {
        let _ = ValidateRect(self.hwnd, None);
        let mut rc = RECT::default();
        let _ = GetClientRect(self.hwnd, &mut rc);
        let (width, height) = (rc.right, rc.bottom);
        if width <= 0 || height <= 0 {
            return;
        }
        let Some(mut surface) = Surface::new(width, height) else {
            return;
        };
        let mem = surface.dc();
        let _ = SetBkMode(mem, TRANSPARENT);
        self.ensure_fonts();

        let full = Rect::new(0, 0, width, height);
        // 透明度按时间轴现算：WM_TIMER 被消息挤后时，画面也不会卡在“全透明”。
        let alpha = self.effective_alpha(Instant::now());
        let layout_now = self.current_layout(Instant::now());

        // DWM 缩略图：注册/注销 + 位置与不透明度同步（先于绘制，保证本帧一致）。
        if let Some(layout) = &layout_now {
            let origin = Point::new(layout.panel.left, layout.panel.top);
            unsafe { self.sync_thumbnails(layout, origin, alpha) };
        } else {
            unsafe { self.clear_thumbnails() };
        }

        // 系统原生风格以一个深色托盘承载全部预览项，而非桌面上分散的浮卡。
        if let Some(layout) = &layout_now {
            let panel = RECT {
                left: 0,
                top: 0,
                right: layout.panel.width(),
                bottom: layout.panel.height(),
            };
            let brush = CreateSolidBrush(self.pal.panel);
            let _ = FillRect(mem, &panel, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
            let origin = Point::new(layout.panel.left, layout.panel.top);
            let rendered = layout.mode == ViewMode::Thumbnails
                && unsafe { self.paint_icon_strip_hq(mem, layout, origin) };
            if !rendered {
                unsafe { self.paint_cells(mem, layout, origin) };
            }
            // GDI 写入的 DIB alpha 为 0，必须在所有绘制完成后回填托盘透明度。
            let (panel_radius, panel_alpha) = match platform::native_switcher_style() {
                platform::NativeSwitcherStyle::Windows10 => (0, PANEL_ALPHA_WIN10),
                platform::NativeSwitcherStyle::Windows11 => (
                    layout::scale(PANEL_RADIUS_WIN11, self.scale),
                    PANEL_ALPHA_WIN11,
                ),
            };
            surface.set_alpha_round_rect(full, panel_radius, full, panel_alpha);
        }
        let _ = surface.apply(self.hwnd, alpha);
    }

    /// 把整条应用图标视图放大绘制，再用 HALFTONE 缩回目标尺寸，减少小图标锯齿。
    unsafe fn paint_icon_strip_hq(&self, hdc: HDC, layout: &Layout, origin: Point) -> bool {
        let width = layout.panel.width();
        let height = layout.panel.height();
        let scaled_width = width.saturating_mul(ICON_RENDER_SCALE);
        let scaled_height = height.saturating_mul(ICON_RENDER_SCALE);
        if width <= 0 || height <= 0 || scaled_width > 32_000 || scaled_height > 32_000 {
            return false;
        }

        let scaled_dc = unsafe { CreateCompatibleDC(hdc) };
        if scaled_dc.0.is_null() {
            return false;
        }
        let bitmap = unsafe { CreateCompatibleBitmap(hdc, scaled_width, scaled_height) };
        if bitmap.0.is_null() {
            let _ = unsafe { DeleteDC(scaled_dc) };
            return false;
        }
        let old_bitmap = unsafe { SelectObject(scaled_dc, HGDIOBJ(bitmap.0)) };
        let scaled_bounds = RECT {
            left: 0,
            top: 0,
            right: scaled_width,
            bottom: scaled_height,
        };
        let background = unsafe { CreateSolidBrush(self.pal.panel) };
        let _ = unsafe { FillRect(scaled_dc, &scaled_bounds, background) };
        let _ = unsafe { DeleteObject(HGDIOBJ(background.0)) };

        let Some(session) = self.session.as_ref() else {
            unsafe {
                let _ = SelectObject(scaled_dc, old_bitmap);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(scaled_dc);
            }
            return false;
        };
        let border = match platform::native_switcher_style() {
            platform::NativeSwitcherStyle::Windows10 => 1,
            platform::NativeSwitcherStyle::Windows11 => {
                layout::scale(SELECTION_BORDER_WIN11, self.scale)
            }
        } * ICON_RENDER_SCALE;
        let scale_rect = |rect: RECT| RECT {
            left: rect.left * ICON_RENDER_SCALE,
            top: rect.top * ICON_RENDER_SCALE,
            right: rect.right * ICON_RENDER_SCALE,
            bottom: rect.bottom * ICON_RENDER_SCALE,
        };

        for cell in &layout.cells {
            if !layout.is_visible(cell.index) {
                continue;
            }
            let Some(window) = session.candidate_window(cell.index) else {
                continue;
            };
            let frame = scale_rect(rel(cell.frame, origin));
            let selected = cell.index == layout.selected;
            round_fill(
                scaled_dc,
                inflate(frame, border),
                0,
                if selected {
                    self.pal.accent
                } else {
                    self.pal.preview_border
                },
            );
            round_fill(
                scaled_dc,
                frame,
                0,
                if selected {
                    self.pal.selected_card
                } else {
                    self.pal.panel
                },
            );
            if let Some(icon) = self
                .icons
                .get(&window.app_id)
                .filter(|icon| icon.raw() != 0)
            {
                let thumb = scale_rect(rel(cell.thumb, origin));
                preview::draw_icon(
                    scaled_dc,
                    icon,
                    Rect::new(thumb.left, thumb.top, thumb.right, thumb.bottom),
                );
            }
        }

        let old_mode = unsafe { SetStretchBltMode(hdc, HALFTONE) };
        let copied = unsafe {
            StretchBlt(
                hdc,
                0,
                0,
                width,
                height,
                scaled_dc,
                0,
                0,
                scaled_width,
                scaled_height,
                SRCCOPY,
            )
            .as_bool()
        };
        if old_mode != 0 {
            let _ = unsafe { SetStretchBltMode(hdc, STRETCH_BLT_MODE(old_mode)) };
        }
        unsafe {
            let _ = SelectObject(scaled_dc, old_bitmap);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(scaled_dc);
        }
        copied
    }

    /// 绘制单行应用图标，或绘制网格视图的窗口预览卡片。
    unsafe fn paint_cells(&self, hdc: HDC, layout: &Layout, origin: Point) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let border = match platform::native_switcher_style() {
            platform::NativeSwitcherStyle::Windows10 => 1,
            platform::NativeSwitcherStyle::Windows11 => {
                layout::scale(SELECTION_BORDER_WIN11, self.scale)
            }
        };

        for cell in &layout.cells {
            if !layout.is_visible(cell.index) {
                continue;
            }
            let Some(window) = session.candidate_window(cell.index) else {
                continue;
            };
            let selected = cell.index == layout.selected;
            let frame = rel(cell.frame, origin);
            round_fill(
                hdc,
                inflate(frame, border),
                0,
                if selected {
                    self.pal.accent
                } else {
                    self.pal.preview_border
                },
            );
            round_fill(
                hdc,
                frame,
                0,
                if selected {
                    self.pal.selected_card
                } else {
                    self.pal.panel
                },
            );

            if session.mode() == ViewMode::Thumbnails {
                if let Some(icon) = self
                    .icons
                    .get(&window.app_id)
                    .filter(|icon| icon.raw() != 0)
                {
                    let thumb = rel(cell.thumb, origin);
                    preview::draw_icon(
                        hdc,
                        icon,
                        Rect::new(thumb.left, thumb.top, thumb.right, thumb.bottom),
                    );
                }
                continue;
            }

            let thumb = rel(cell.thumb, origin);
            let title = rel(cell.title, origin);
            if !self.has_thumbnail(window.id) {
                self.paint_placeholder(hdc, thumb, &window.app_id);
            }
            self.paint_tile_label(hdc, title, window);
        }
    }

    /// 卡片底部的图标 + 标题行：图标与文字作为一个整体水平居中，超长省略。
    unsafe fn paint_tile_label(&self, hdc: HDC, rect: RECT, window: &WindowEntry) {
        let text = window.display_title();
        let icon = self
            .icons
            .get(&window.app_id)
            .filter(|icon| icon.raw() != 0);
        let icon_side = layout::scale(TILE_ICON, self.scale);
        let gap = layout::scale(6, self.scale);
        let reserved = if icon.is_some() { icon_side + gap } else { 0 };
        let max_text = (rect.right - rect.left - reserved).max(1);
        let text_w = text_width(hdc, self.font, text).min(max_text);
        let total = text_w + reserved;
        let mut left = rect.left + ((rect.right - rect.left) - total).max(0) / 2;

        if let Some(icon) = icon {
            let top = rect.top + ((rect.bottom - rect.top) - icon_side) / 2;
            preview::draw_icon(
                hdc,
                icon,
                Rect::new(left, top, left + icon_side, top + icon_side),
            );
            left += icon_side + gap;
        }
        draw_text(
            hdc,
            self.font,
            text,
            RECT {
                left,
                top: rect.top,
                right: left + text_w,
                bottom: rect.bottom,
            },
            self.pal.text,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }

    /// 缩略图注册不了（预览关闭 / 受保护窗口）时的卡片内容：居中的应用图标
    /// （没有图标时用一行灰字说明）。
    ///
    /// 标题行已经显示图标 + 标题，这里不再重复文字，避免一张卡片里出现两个图标。
    unsafe fn paint_placeholder(&self, hdc: HDC, thumb: RECT, app_id: &str) {
        let area_width = (thumb.right - thumb.left).max(1);
        let area_height = (thumb.bottom - thumb.top).max(1);
        let side = layout::scale(PLACEHOLDER_ICON, self.scale)
            .min(area_width)
            .min(area_height);

        if let Some(icon) = self.icons.get(app_id).filter(|icon| icon.raw() != 0) {
            if side > 0 {
                let at = Rect::new(
                    thumb.left + (area_width - side) / 2,
                    thumb.top + (area_height - side) / 2,
                    thumb.left + (area_width - side) / 2 + side,
                    thumb.top + (area_height - side) / 2 + side,
                );
                preview::draw_icon(hdc, icon, at);
            }
            return;
        }

        let label = if !self.previews_enabled {
            "预览已关闭"
        } else {
            "无预览"
        };
        draw_text(
            hdc,
            self.font_small,
            label,
            thumb,
            self.pal.muted,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }

    // ---------- 消息 ----------

    fn on_timer(&mut self) {
        let now = Instant::now();
        let mut animating = false;
        if self.anim_deferred {
            self.anim_deferred = false;
            // 首帧可能被首次绘制拖慢（尤其调试构建）：动画时钟从第一个 tick 起算，
            // 否则此时进度已经超过时长，入场动画会被直接跳过去。
            if let Some(fade) = self.fade.as_mut() {
                fade.anim = Anim::new(now, Duration::from_millis(FADE_IN_MS));
            }
            if let Some(anim) = self.layout_anim.as_mut() {
                *anim = Anim::new(now, Duration::from_millis(OPEN_SCALE_MS));
            }
        }

        // 先在只读借用里算出这一帧的透明度与是否结束，再写回窗口属性。
        let fade_state = self.fade.as_ref().map(|fade| {
            (
                lerp_u8(fade.from, fade.to, fade.anim.progress(now)),
                fade.anim.is_done(now),
            )
        });
        if let Some((alpha, done)) = fade_state {
            self.set_alpha(alpha);
            if done {
                self.fade = None;
            } else {
                animating = true;
            }
        }

        let layout_done = self.layout_anim.as_ref().map(|anim| anim.is_done(now));
        match layout_done {
            Some(true) => {
                self.layout_anim = None;
                if let Some(layout) = &self.layout {
                    unsafe { self.apply_geometry(layout) };
                }
            }
            Some(false) => {
                animating = true;
                if let Some(layout) = self.current_layout(now) {
                    unsafe { self.apply_geometry(&layout) };
                }
            }
            None => {}
        }

        if self.closing && self.fade.is_none() {
            self.finish();
            return;
        }
        if !animating && !self.closing {
            self.stop_timer();
        }
        self.invalidate();
    }

    fn on_mouse_move(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        let hover = layout.hit_test(Point::new(point.x, point.y));
        if hover != self.hover {
            self.hover = hover;
            self.invalidate();
        }
    }

    fn on_click(&mut self) {
        if self.closing {
            return;
        }
        let Some(index) = self
            .layout
            .as_ref()
            .and_then(|layout| layout.hit_test(cursor_point()))
        else {
            return;
        };
        if let Some(session) = self.session.as_mut() {
            session.select(index);
        }
        self.commit();
    }

    fn on_wheel(&mut self, wparam: isize) {
        if self.closing {
            return;
        }
        // 高字是带符号的滚轮增量。
        let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16;
        if delta > 0 {
            self.step(Step::Prev);
        } else if delta < 0 {
            self.step(Step::Next);
        }
    }

    fn on_dpi_changed(&mut self) {
        self.scale = platform::cursor_scale();
        unsafe { self.ensure_fonts() };
        if self.session.is_some() {
            self.relayout(Duration::from_millis(STEP_MS));
        }
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            MSG_START => {
                self.start_session(wparam.0 != 0);
                Some(LRESULT(0))
            }
            MSG_STEP => {
                if let Some(step) = Step::from_code(wparam.0 as isize) {
                    self.step(step);
                }
                Some(LRESULT(0))
            }
            MSG_MODE => {
                self.toggle_mode();
                Some(LRESULT(0))
            }
            MSG_COMMIT => {
                self.commit();
                Some(LRESULT(0))
            }
            MSG_CANCEL => {
                self.cancel();
                Some(LRESULT(0))
            }
            MSG_ICONS => {
                self.on_icons(lparam);
                Some(LRESULT(0))
            }
            WM_PAINT => {
                unsafe { self.paint() };
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_TIMER => {
                if wparam.0 == TIMER_ANIM {
                    self.on_timer();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.on_click();
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.cancel();
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                self.on_wheel(wparam.0 as isize);
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.on_dpi_changed();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    // ---------- 小工具 ----------

    /// 这一帧该用的整窗透明度：有淡入淡出时由时间轴现算，否则用静态值。
    fn effective_alpha(&self, now: Instant) -> u8 {
        match &self.fade {
            Some(fade) => lerp_u8(fade.from, fade.to, fade.anim.progress(now)),
            None => self.alpha,
        }
    }

    /// 透明度只在 `UpdateLayeredWindow` 提交时生效，所以改完要重画一帧。
    fn set_alpha(&mut self, alpha: u8) {
        if self.alpha == alpha {
            return;
        }
        self.alpha = alpha;
        self.invalidate();
    }

    fn start_timer(&mut self) {
        if self.timer_on {
            return;
        }
        self.timer_on = unsafe { SetTimer(self.hwnd, TIMER_ANIM, 16, None) != 0 };
    }

    fn stop_timer(&mut self) {
        if !self.timer_on {
            return;
        }
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_ANIM);
        }
        self.timer_on = false;
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // 让后台图标线程在下一轮循环开头退出，不必等它把剩下几十个窗口问完。
        PREVIEW_GENERATION.store(self.generation.wrapping_add(1), Ordering::SeqCst);
        self.stop_timer();
        unsafe { self.clear_thumbnails() };
        for font in [self.font, self.font_small] {
            if !font.0.is_null() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
            }
        }
        if !self.hwnd.0.is_null() && unsafe { IsWindow(self.hwnd).as_bool() } {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        let overlay = create.lpCreateParams as *mut Overlay;
        if !overlay.is_null() {
            (*overlay).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, overlay as isize);
        }
        return LRESULT(1);
    }

    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Overlay;
    if raw.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match (*raw).on_message(msg, wparam, lparam) {
        Some(result) => result,
        None => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 选中预览半透明叠加到底色上，其余候选保持 DWM 原始透明度。
fn thumbnail_alpha(alpha: u8, selected: bool) -> u8 {
    if selected {
        ((alpha as u16 * SELECTED_THUMBNAIL_ALPHA_PERCENT) / 100) as u8
    } else {
        alpha
    }
}

fn cursor_point() -> Point {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    Point::new(point.x, point.y)
}

// ---------- 绘图辅助 ----------

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

/// 四边同时外扩 `by` 像素（负值收缩）。
fn inflate(rect: RECT, by: i32) -> RECT {
    RECT {
        left: rect.left - by,
        top: rect.top - by,
        right: rect.right + by,
        bottom: rect.bottom + by,
    }
}

fn draw_text(
    hdc: HDC,
    font: HFONT,
    text: &str,
    rect: RECT,
    color: COLORREF,
    format: DRAW_TEXT_FORMAT,
) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return;
    }
    let mut rect = rect;
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        SetTextColor(hdc, color);
        let _ = DrawTextW(hdc, &mut wide, &mut rect, format);
        SelectObject(hdc, old);
    }
}

/// 屏幕坐标 → 客户区坐标（`origin` 是窗口左上角对应的屏幕坐标）。
fn rel(rect: Rect, origin: Point) -> RECT {
    RECT {
        left: rect.left - origin.x,
        top: rect.top - origin.y,
        right: rect.right - origin.x,
        bottom: rect.bottom - origin.y,
    }
}

/// 文字宽度（“图标 + 标题”整体居中时用）。
unsafe fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    SelectObject(hdc, old);
    size.cx
}
