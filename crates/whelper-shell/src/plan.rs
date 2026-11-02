//! 设置页的几何与动作表（纯逻辑，可单测）。
//!
//! 绘制与点击共用同一份 [`Plan`]：保证“看到的”和“点得到的”永远一致，
//! 也让新增一行配置只需要改 [`rows`] 里的静态表。

use whelper_common::geometry::Rect;
use whelper_config::{Config, DefaultView, Theme};

// ---------- 分类 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Picker,
    Desktop,
    Switcher,
    About,
}

impl Page {
    pub const ALL: [Self; 5] =
        [Self::General, Self::Picker, Self::Desktop, Self::Switcher, Self::About];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Picker => "划词工具条",
            Self::Desktop => "显示桌面",
            Self::Switcher => "窗口切换器",
            Self::About => "关于",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Picker => "划词工具条",
            Self::Desktop => "单屏显示桌面",
            Self::Switcher => "Alt+Tab 切换器",
            Self::About => "关于 whelper",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::General => "启动方式、主题与配置文件",
            Self::Picker => "选中文字后弹出的复制 / 翻译条",
            Self::Desktop => "只影响光标所在显示器的 Win+D",
            Self::Switcher => "应用图标、窗口预览与显示器筛选",
            Self::About => "一个进程里的三个桌面小工具",
        }
    }
}

// ---------- 动作 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nav(Page),
    ToggleAutostart,
    OpenConfigDir,
    TogglePicker,
    ToggleDoubleClick,
    SetDragThreshold(i32),
    Theme(Theme),
    ToggleDesktop,
    ToggleSwitcher,
    ToggleSwitcherCursorMonitorOnly,
    SetDefaultView(DefaultView),
    TogglePreviews,
    ToggleAnimation,
    ToggleDebug,
    ResetDefaults,
}

// ---------- 行定义 ----------

#[derive(Debug, Clone, Copy)]
pub struct SegmentOption {
    pub label: &'static str,
    pub action: Action,
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Switch(Action),
    Segment(&'static [SegmentOption]),
    Button(&'static str, Action),
    /// 只说明、不可点。
    Hint,
}

#[derive(Debug, Clone, Copy)]
pub struct RowSpec {
    pub title: &'static str,
    pub desc: &'static str,
    pub control: Control,
}

const THEME_OPTIONS: &[SegmentOption] = &[
    SegmentOption { label: "跟随系统", action: Action::Theme(Theme::System) },
    SegmentOption { label: "浅色", action: Action::Theme(Theme::Light) },
    SegmentOption { label: "深色", action: Action::Theme(Theme::Dark) },
];

const VIEW_OPTIONS: &[SegmentOption] = &[
    SegmentOption { label: "缩略图", action: Action::SetDefaultView(DefaultView::Thumbnails) },
    SegmentOption { label: "网格", action: Action::SetDefaultView(DefaultView::Grid) },
];

const DRAG_OPTIONS: &[SegmentOption] = &[
    SegmentOption { label: "2 px", action: Action::SetDragThreshold(2) },
    SegmentOption { label: "4 px", action: Action::SetDragThreshold(4) },
    SegmentOption { label: "8 px", action: Action::SetDragThreshold(8) },
];

const GENERAL_ROWS: &[RowSpec] = &[
    RowSpec {
        title: "开机自启",
        desc: "登录 Windows 后在后台启动（写入当前用户的启动项）",
        control: Control::Switch(Action::ToggleAutostart),
    },
    RowSpec {
        title: "界面主题",
        desc: "工具条、切换器与设置页统一使用",
        control: Control::Segment(THEME_OPTIONS),
    },
    RowSpec {
        title: "调试日志",
        desc: "在控制台打印各组件的运行细节（开发者用）",
        control: Control::Switch(Action::ToggleDebug),
    },
    RowSpec {
        title: "配置文件",
        desc: "config.ini · 也可以在退出 whelper 后手工编辑",
        control: Control::Button("打开目录", Action::OpenConfigDir),
    },
];

const PICKER_ROWS: &[RowSpec] = &[
    RowSpec {
        title: "启用划词工具条",
        desc: "选中文字后在光标附近弹出「复制 / 翻译」条",
        control: Control::Switch(Action::TogglePicker),
    },
    RowSpec {
        title: "双击 / 三击选词",
        desc: "不拖拽时也能通过双击选词触发",
        control: Control::Switch(Action::ToggleDoubleClick),
    },
    RowSpec {
        title: "拖拽触发距离",
        desc: "拖动超过该距离才认为在选字",
        control: Control::Segment(DRAG_OPTIONS),
    },
    RowSpec {
        title: "取词方式",
        desc: "优先 UI Automation 只读选区；不可用时回退 Ctrl+C 并还原剪贴板",
        control: Control::Hint,
    },
];

const DESKTOP_ROWS: &[RowSpec] = &[
    RowSpec {
        title: "启用单屏显示桌面",
        desc: "接管 Win+D，只最小化 / 还原光标所在显示器的窗口",
        control: Control::Switch(Action::ToggleDesktop),
    },
    RowSpec {
        title: "行为说明",
        desc: "再按一次 Win+D 按原 Z 序还原；期间窗口增减会按 Win+D 语义重新隐藏",
        control: Control::Hint,
    },
];

const SWITCHER_ROWS: &[RowSpec] = &[
    RowSpec {
        title: "启用 Alt+Tab 切换器",
        desc: "接管系统 Alt+Tab，松开 Alt 才真正切换",
        control: Control::Switch(Action::ToggleSwitcher),
    },
    RowSpec {
        title: "默认视图",
        desc: "每次打开切换器时使用的布局",
        control: Control::Segment(VIEW_OPTIONS),
    },
    RowSpec {
        title: "仅显示鼠标屏幕窗口",
        desc: "只列出鼠标所在显示器上的窗口，关闭时显示所有屏幕",
        control: Control::Switch(Action::ToggleSwitcherCursorMonitorOnly),
    },
    RowSpec {
        title: "窗口预览",
        desc: "抓取窗口画面作为缩略图；关闭后只显示应用图标",
        control: Control::Switch(Action::TogglePreviews),
    },
    RowSpec {
        title: "过渡动画",
        desc: "打开、移动选择与切换视图时的淡入和位移",
        control: Control::Switch(Action::ToggleAnimation),
    },
    RowSpec {
        title: "快捷键",
        desc: "Alt+Tab 循环 · Shift 反向 · ` 切换视图 · Esc 取消",
        control: Control::Hint,
    },
];

/// 某一页的配置行。
pub fn rows(page: Page) -> &'static [RowSpec] {
    match page {
        Page::General => GENERAL_ROWS,
        Page::Picker => PICKER_ROWS,
        Page::Desktop => DESKTOP_ROWS,
        Page::Switcher => SWITCHER_ROWS,
        Page::About => &[],
    }
}

// ---------- 几何 ----------

/// 侧栏宽度（逻辑像素）。
pub const SIDEBAR_W: i32 = 216;
/// 内容区左边距（侧栏之后）。
pub const CONTENT_LEFT: i32 = 246;
/// 内容区右边距。
pub const CONTENT_PAD: i32 = 30;
/// 第一行卡片的顶部。
pub const ROWS_TOP: i32 = 118;

const NAV_LEFT: i32 = 14;
const NAV_TOP: i32 = 132;
const NAV_H: i32 = 38;
const NAV_GAP: i32 = 4;
const ROW_H: i32 = 70;
const ROW_GAP: i32 = 8;
const ROW_PAD: i32 = 16;
const SWITCH_W: i32 = 46;
const SWITCH_H: i32 = 26;
const SEG_OPT_W: i32 = 92;
const SEG_H: i32 = 30;
const BUTTON_W: i32 = 112;
const BUTTON_H: i32 = 30;

fn scaled(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round() as i32
}

#[derive(Debug, Clone, Copy)]
pub struct NavItem {
    pub rect: Rect,
    pub page: Page,
}

#[derive(Debug, Clone, Copy)]
pub struct ControlHit {
    pub rect: Rect,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct RowLayout {
    pub rect: Rect,
    pub spec: &'static RowSpec,
    pub controls: Vec<ControlHit>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub nav: Vec<NavItem>,
    pub rows: Vec<RowLayout>,
    pub about_buttons: Vec<ControlHit>,
}

/// 生成某一页的导航、行与控件矩形。
pub fn plan(page: Page, config: &Config, autostart: bool, client: Rect, scale: f32) -> Plan {
    let mut plan = Plan::default();
    let width = client.width();
    let height = client.height();
    let right = width - scaled(CONTENT_PAD, scale);
    let left = scaled(CONTENT_LEFT, scale);

    for (index, item) in Page::ALL.iter().enumerate() {
        let top = scaled(NAV_TOP + index as i32 * (NAV_H + NAV_GAP), scale);
        plan.nav.push(NavItem {
            rect: Rect::new(
                scaled(NAV_LEFT, scale),
                top,
                scaled(SIDEBAR_W - NAV_LEFT, scale),
                top + scaled(NAV_H, scale),
            ),
            page: *item,
        });
    }

    if page == Page::About {
        let top = (height - scaled(100, scale)).max(scaled(ROWS_TOP, scale));
        plan.about_buttons.push(ControlHit {
            rect: Rect::new(left, top, left + scaled(BUTTON_W + 16, scale), top + scaled(BUTTON_H, scale)),
            action: Action::OpenConfigDir,
        });
        plan.about_buttons.push(ControlHit {
            rect: Rect::new(
                left + scaled(BUTTON_W + 28, scale),
                top,
                left + scaled(BUTTON_W * 2 + 44, scale),
                top + scaled(BUTTON_H, scale),
            ),
            action: Action::ResetDefaults,
        });
        return plan;
    }

    let mut top = scaled(ROWS_TOP, scale);
    for (index, spec) in rows(page).iter().enumerate() {
        let rect = Rect::new(left, top, right, top + scaled(ROW_H, scale));
        let controls = control_hits(spec, rect, right - scaled(ROW_PAD, scale), scale);
        plan.rows.push(RowLayout {
            rect,
            spec,
            controls,
            disabled: row_disabled(page, index, config, autostart),
        });
        top = rect.bottom + scaled(ROW_GAP, scale);
    }

    plan
}

fn control_hits(spec: &RowSpec, row: Rect, control_right: i32, scale: f32) -> Vec<ControlHit> {
    let center = row.top + row.height() / 2;
    match spec.control {
        Control::Switch(action) => {
            let w = scaled(SWITCH_W, scale);
            let h = scaled(SWITCH_H, scale);
            vec![ControlHit {
                rect: Rect::new(control_right - w, center - h / 2, control_right, center + h / 2),
                action,
            }]
        }
        Control::Segment(options) => {
            let gap = scaled(4, scale);
            let w = scaled(SEG_OPT_W, scale);
            let h = scaled(SEG_H, scale);
            let count = options.len() as i32;
            let left = control_right - count * w;
            options
                .iter()
                .enumerate()
                .map(|(index, option)| ControlHit {
                    rect: Rect::new(
                        left + index as i32 * w,
                        center - h / 2,
                        left + index as i32 * w + w - gap,
                        center + h / 2,
                    ),
                    action: option.action,
                })
                .collect()
        }
        Control::Button(_label, action) => {
            let w = scaled(BUTTON_W, scale);
            let h = scaled(BUTTON_H, scale);
            vec![ControlHit {
                rect: Rect::new(control_right - w, center - h / 2, control_right, center + h / 2),
                action,
            }]
        }
        Control::Hint => Vec::new(),
    }
}

/// 组件关闭时只把**子选项**置灰；第 0 行是总开关本身，必须始终可点，
/// 否则关掉以后就再也点不开（也正是这条规则最容易写错的地方）。
fn row_disabled(page: Page, index: usize, config: &Config, autostart: bool) -> bool {
    let _ = autostart;
    match page {
        Page::Picker => !config.picker.enabled && (1..=2).contains(&index),
        Page::Switcher => !config.switcher.enabled && (1..=4).contains(&index),
        Page::General | Page::Desktop | Page::About => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whelper_common::geometry::Point;

    /// 客户区是物理像素：缩放越大，窗口也越大。
    fn client(scale: f32) -> Rect {
        Rect::new(0, 0, (880.0 * scale) as i32, (640.0 * scale) as i32)
    }

    #[test]
    fn nav_has_five_clickable_pages() {
        let plan = plan(Page::General, &Config::default(), false, client(1.0), 1.0);
        assert_eq!(plan.nav.len(), 5);
        assert!(plan.nav.iter().all(|item| item.rect.width() > 0));
        for pair in plan.nav.windows(2) {
            assert!(pair[0].rect.top < pair[1].rect.top);
        }
    }

    #[test]
    fn general_rows_carry_expected_actions() {
        let plan = plan(Page::General, &Config::default(), false, client(1.0), 1.0);
        assert_eq!(plan.rows.len(), 4);
        assert_eq!(plan.rows[0].controls[0].action, Action::ToggleAutostart);
        assert_eq!(plan.rows[2].controls[0].action, Action::ToggleDebug);
        assert_eq!(plan.rows[3].controls[0].action, Action::OpenConfigDir);
        // 主题有三个分段，且都落在行内。
        assert_eq!(plan.rows[1].controls.len(), 3);
        for control in &plan.rows[1].controls {
            assert!(plan.rows[1]
                .rect
                .contains(Point::new(control.rect.left + 1, control.rect.top + 1)));
        }
    }

    #[test]
    fn picker_rows_disabled_when_picker_off() {
        let enabled = plan(Page::Picker, &Config::default(), false, client(1.0), 1.0);
        assert!(enabled.rows.iter().all(|row| !row.disabled));

        let mut config = Config::default();
        config.picker.enabled = false;
        let disabled = plan(Page::Picker, &config, false, client(1.0), 1.0);
        // 总开关必须保持可点，否则关掉以后再也开不回来。
        assert!(!disabled.rows[0].disabled);
        assert!(disabled.rows[1].disabled);
        assert!(disabled.rows[2].disabled);
        assert!(disabled.rows[3].controls.is_empty());
    }

    #[test]
    fn switcher_rows_disabled_when_switcher_off() {
        let mut config = Config::default();
        config.switcher.enabled = false;
        let plan = plan(Page::Switcher, &config, false, client(1.0), 1.0);
        assert!(!plan.rows[0].disabled);
        assert!(plan.rows[1].disabled);
        assert!(plan.rows[2].disabled);
        assert!(plan.rows[3].disabled);
        assert!(plan.rows[4].disabled);
        assert!(!plan.rows[5].disabled);
    }

    /// 回归：两个组件的总开关在关闭状态下必须仍然可点、动作正确。
    #[test]
    fn master_switches_stay_clickable_when_component_off() {
        let mut config = Config::default();
        config.picker.enabled = false;
        config.switcher.enabled = false;

        let picker = plan(Page::Picker, &config, false, client(1.0), 1.0);
        assert!(!picker.rows[0].disabled);
        assert_eq!(picker.rows[0].controls[0].action, Action::TogglePicker);

        let switcher = plan(Page::Switcher, &config, false, client(1.0), 1.0);
        assert!(!switcher.rows[0].disabled);
        assert_eq!(switcher.rows[0].controls[0].action, Action::ToggleSwitcher);
    }

    #[test]
    fn about_page_has_two_buttons() {
        let plan = plan(Page::About, &Config::default(), false, client(1.0), 1.0);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.about_buttons.len(), 2);
        assert_eq!(plan.about_buttons[0].action, Action::OpenConfigDir);
        assert_eq!(plan.about_buttons[1].action, Action::ResetDefaults);
        assert!(plan.about_buttons[1].rect.right < client(1.0).right);
    }

    #[test]
    fn controls_stay_inside_row_and_content() {
        for scale in [1.0, 1.25, 2.0] {
            let plan = plan(Page::Switcher, &Config::default(), false, client(scale), scale);
            for row in &plan.rows {
                assert!(row.rect.left >= scaled(CONTENT_LEFT, scale) - 1);
                for control in &row.controls {
                    assert!(control.rect.right <= row.rect.right);
                    assert!(control.rect.left >= row.rect.left);
                    assert!(control.rect.top >= row.rect.top);
                    assert!(control.rect.bottom <= row.rect.bottom);
                }
            }
        }
    }

    #[test]
    fn tiny_client_does_not_panic() {
        let plan = plan(Page::Picker, &Config::default(), false, Rect::new(0, 0, 80, 80), 2.0);
        assert_eq!(plan.rows.len(), 4);
        assert_eq!(plan.nav.len(), 5);
    }
}
