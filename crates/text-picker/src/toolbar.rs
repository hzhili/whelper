//! 工具条的纯几何与状态逻辑（与 Win32 解耦，便于单元测试）。

use windows::Win32::Foundation::{POINT, RECT};

// 尺寸基线：
//   .container   { height:36px; padding:5px; border-radius:10px }
//   .btnArea     { height:26px; padding:2px 6px; border-radius:6px }
//   .actions     { gap:2px }
//   .btnArea img { width:20px;height:20px } / svg { width:1em;height:1em }

/// 工具条内边距（逻辑像素）。
pub const PAD: i32 = 5;
/// 单个按钮宽：6 + 16(图标) + 4(间距) + 28(两个 14px 汉字) + 6。
pub const BTN_W: i32 = 60;
/// 单个按钮高（逻辑像素）。
pub const BTN_H: i32 = 26;
/// 按钮间距。
pub const GAP: i32 = 2;
/// 按钮内的左右内边距。
pub const BTN_PAD_X: i32 = 6;
/// 图标边长。
pub const ICON: i32 = 16;
/// 图标与文字的间距。
pub const ICON_GAP: i32 = 4;
/// 卡片圆角。
pub const BAR_RADIUS: i32 = 10;
/// 按钮圆角。
pub const BTN_RADIUS: i32 = 6;
/// 卡片描边宽度。
pub const BORDER: i32 = 1;
/// 工具条与锚点（鼠标抬起位置）的垂直间距。
pub const ANCHOR_GAP: i32 = 8;
/// 与屏幕边缘的最小留白。
pub const SCREEN_MARGIN: i32 = 4;

/// 翻译结果面板的最小尺寸（逻辑像素）。
pub const PANEL_W: i32 = 360;
pub const PANEL_H: i32 = 120;
/// 翻译结果面板的最大尺寸（逻辑像素）。超过高度后使用滚动条。
pub const PANEL_MAX_W: i32 = 560;
pub const PANEL_MAX_H: i32 = 420;
/// 面板内容布局尺寸（逻辑像素）。
pub const PANEL_PAD: i32 = 14;
pub const PANEL_TITLE_H: i32 = 22;
pub const PANEL_LINE_H: i32 = 21;
pub const PANEL_FOOTER_H: i32 = 26;
pub const PANEL_DIVIDER_GAP: i32 = 8;
/// 自绘滚动条的轨道宽度、可点击区域宽度，以及正文预留间距。
pub const PANEL_SCROLLBAR_W: i32 = 6;
pub const PANEL_SCROLLBAR_HIT_W: i32 = 16;
pub const PANEL_SCROLLBAR_GAP: i32 = 6;

pub fn scale(v: i32, s: f32) -> i32 {
    (v as f32 * s).round().max(1.0) as i32
}

pub fn bar_size(s: f32) -> (i32, i32) {
    let w = PAD * 2 + BTN_W * 2 + GAP;
    let h = PAD * 2 + BTN_H;
    (scale(w, s), scale(h, s))
}

pub fn panel_size(s: f32) -> (i32, i32) {
    (scale(PANEL_W, s), scale(PANEL_H, s))
}

/// 按内容计算出的面板尺寸，统一夹在最小/最大尺寸之间。
/// `width`/`height` 已经是当前 DPI 下的像素值。
pub fn panel_size_for_content(width: i32, height: i32, s: f32) -> (i32, i32) {
    let (min_w, min_h) = panel_size(s);
    let max_w = scale(PANEL_MAX_W, s);
    let max_h = scale(PANEL_MAX_H, s);
    (width.max(min_w).min(max_w), height.max(min_h).min(max_h))
}

/// 面板正文区域。`scrollbar` 为 true 时给自绘滚动条预留宽度。
pub fn panel_body_rect(panel: RECT, s: f32, scrollbar: bool) -> RECT {
    let pad = scale(PANEL_PAD, s);
    let top = panel.top + pad + scale(PANEL_TITLE_H, s);
    let footer_y = panel.bottom - scale(PANEL_FOOTER_H, s);
    let bottom = footer_y - scale(PANEL_DIVIDER_GAP, s);
    let reserved = if scrollbar {
        scale(PANEL_SCROLLBAR_W + PANEL_SCROLLBAR_GAP, s)
    } else {
        0
    };
    RECT {
        left: panel.left + pad,
        top,
        right: (panel.right - pad - reserved).max(panel.left + pad + 1),
        bottom: bottom.max(top + 1),
    }
}

/// 面板滚动条的可点击区域（比视觉轨道略宽，便于鼠标操作）。
pub fn panel_scrollbar_rect(panel: RECT, s: f32) -> RECT {
    let pad = scale(PANEL_PAD, s);
    let width = scale(PANEL_SCROLLBAR_HIT_W, s);
    let top = panel.top + pad + scale(PANEL_TITLE_H, s);
    let bottom = panel.bottom - scale(PANEL_FOOTER_H + PANEL_DIVIDER_GAP, s);
    let right = panel.right - pad;
    RECT {
        left: (right - width).max(panel.left),
        top,
        right,
        bottom: bottom.max(top + 1),
    }
}

/// 面板滚动条的视觉轨道。
pub fn panel_scrollbar_track_rect(panel: RECT, s: f32) -> RECT {
    let hit = panel_scrollbar_rect(panel, s);
    let width = scale(PANEL_SCROLLBAR_W, s).min(hit.right - hit.left).max(1);
    let left = hit.left + (hit.right - hit.left - width) / 2;
    RECT {
        left,
        top: hit.top,
        right: left + width,
        bottom: hit.bottom,
    }
}

/// 根据正文总高度、可视高度和当前偏移量计算滚动条滑块。
pub fn panel_scrollbar_thumb_rect(
    panel: RECT,
    s: f32,
    content_height: i32,
    viewport_height: i32,
    offset: i32,
) -> RECT {
    let track = panel_scrollbar_track_rect(panel, s);
    let track_height = (track.bottom - track.top).max(1);
    let content_height = content_height.max(1);
    let viewport_height = viewport_height.max(1).min(content_height);
    let min_thumb = scale(20, s).min(track_height);
    let thumb_height =
        ((track_height as i64 * viewport_height as i64) / content_height as i64) as i32;
    let thumb_height = thumb_height.max(min_thumb).min(track_height);
    let max_offset = (content_height - viewport_height).max(0);
    let travel = track_height - thumb_height;
    let top_offset = if max_offset == 0 {
        0
    } else {
        (travel as i64 * offset.clamp(0, max_offset) as i64 / max_offset as i64) as i32
    };
    RECT {
        left: track.left,
        top: track.top + top_offset,
        right: track.right,
        bottom: track.top + top_offset + thumb_height,
    }
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo {
        lo
    } else {
        v.max(lo).min(hi)
    }
}

/// 以锚点为中心摆放一个 `w x h` 的窗口：优先放在锚点上方，空间不足则放下方，
/// 最后夹紧到显示器工作区内（选区上方优先 + 显示器夹紧）。
pub fn place(anchor: POINT, w: i32, h: i32, work: RECT, below_only: bool) -> RECT {
    let above_top = anchor.y - ANCHOR_GAP - h;
    let below_top = anchor.y + ANCHOR_GAP + 10;
    let fits_above = above_top >= work.top + SCREEN_MARGIN;
    let top = if below_only || !fits_above {
        below_top
    } else {
        above_top
    };
    let left = anchor.x - w / 2;
    let left = clamp(
        left,
        work.left + SCREEN_MARGIN,
        work.right - w - SCREEN_MARGIN,
    );
    let top = clamp(
        top,
        work.top + SCREEN_MARGIN,
        work.bottom - h - SCREEN_MARGIN,
    );
    RECT {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

pub fn bar_rect(anchor: POINT, s: f32, work: RECT) -> RECT {
    let (w, h) = bar_size(s);
    place(anchor, w, h, work, false)
}

pub fn panel_rect_for_size(anchor: POINT, size: (i32, i32), work: RECT) -> RECT {
    let max_w = (work.right - work.left - SCREEN_MARGIN * 2).max(1);
    let max_h = (work.bottom - work.top - SCREEN_MARGIN * 2).max(1);
    let w = size.0.max(1).min(max_w);
    let h = size.1.max(1).min(max_h);
    place(anchor, w, h, work, false)
}

/// 两个按钮的矩形：`[复制, 翻译]`。
pub fn button_rects(bar: RECT, s: f32) -> [RECT; 2] {
    let pad = scale(PAD, s);
    let bh = scale(BTN_H, s).min(bar.bottom - bar.top - pad * 2).max(1);
    let gap = scale(GAP, s);
    let top = bar.top + pad;
    // 从卡片实际宽度反推按钮宽，避免 125%/150% 缩放下 “缩放后求和” 与
    // “求和后缩放” 的 1px 差异把第二个按钮推出卡片。
    let avail = (bar.right - bar.left - pad * 2 - gap).max(2);
    let bw = avail / 2;
    let first = RECT {
        left: bar.left + pad,
        top,
        right: bar.left + pad + bw,
        bottom: top + bh,
    };
    let second_left = first.right + gap;
    let second = RECT {
        left: second_left,
        top,
        right: second_left + bw,
        bottom: top + bh,
    };
    [first, second]
}

/// 按钮内的图标矩形。
pub fn icon_rect(button: RECT, s: f32) -> RECT {
    let pad = scale(BTN_PAD_X, s);
    let size = scale(ICON, s);
    let top = button.top + (button.bottom - button.top - size) / 2;
    RECT {
        left: button.left + pad,
        top,
        right: button.left + pad + size,
        bottom: top + size,
    }
}

/// 按钮内文字的起始位置（左对齐、垂直居中）。
pub fn label_origin(button: RECT, s: f32) -> (i32, i32) {
    let icon = icon_rect(button, s);
    (icon.right + scale(ICON_GAP, s), button.top)
}

pub fn point_in(rect: RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

/// 工具条的交互模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 两个按钮：复制 / 翻译
    Bar,
    /// 翻译请求中
    Loading,
    /// 翻译结果
    Result,
    /// 翻译失败
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    #[test]
    fn toolbar_sits_above_the_anchor_when_there_is_room() {
        let anchor = POINT { x: 800, y: 600 };
        let r = bar_rect(anchor, 1.0, WORK);
        let (w, h) = bar_size(1.0);
        assert_eq!(r.right - r.left, w);
        assert_eq!(r.bottom - r.top, h);
        assert!(r.bottom <= anchor.y, "工具条底部应在锚点上方");
        assert_eq!(r.left, 800 - w / 2);
    }

    #[test]
    fn toolbar_flips_below_near_the_top_edge() {
        let anchor = POINT { x: 400, y: 10 };
        let r = bar_rect(anchor, 1.0, WORK);
        assert!(r.top > anchor.y, "上方放不下时应翻到下方");
    }

    #[test]
    fn toolbar_is_clamped_to_the_right_edge() {
        let anchor = POINT { x: 1918, y: 500 };
        let r = bar_rect(anchor, 1.0, WORK);
        assert_eq!(r.right, WORK.right - SCREEN_MARGIN);
        assert!(r.left >= WORK.left);
    }

    #[test]
    fn toolbar_is_clamped_to_a_secondary_monitor_offset() {
        let work = RECT {
            left: 1920,
            top: -200,
            right: 3840,
            bottom: 880,
        };
        let r = bar_rect(POINT { x: 1930, y: 0 }, 1.0, work);
        assert!(r.left >= work.left + SCREEN_MARGIN);
        assert!(r.right <= work.right - SCREEN_MARGIN);
    }

    #[test]
    fn buttons_are_hit_testable() {
        let bar = bar_rect(POINT { x: 800, y: 600 }, 1.0, WORK);
        let [copy, translate] = button_rects(bar, 1.0);
        assert!(point_in(
            copy,
            POINT {
                x: copy.left + 2,
                y: copy.top + 2
            }
        ));
        assert!(!point_in(
            copy,
            POINT {
                x: translate.left + 2,
                y: translate.top + 2
            }
        ));
        assert!(point_in(
            translate,
            POINT {
                x: translate.right - 1,
                y: translate.bottom - 1
            }
        ));
    }

    #[test]
    fn hi_dpi_geometry_scales() {
        let (w1, h1) = bar_size(1.0);
        let (w2, h2) = bar_size(1.5);
        assert_eq!(w2, (w1 as f32 * 1.5).round() as i32);
        assert_eq!(h2, (h1 as f32 * 1.5).round() as i32);
    }

    #[test]
    fn panel_is_wider_than_bar() {
        let (pw, _) = panel_size(1.0);
        let (bw, _) = bar_size(1.0);
        assert!(pw > bw);
    }

    #[test]
    fn panel_content_size_is_clamped_to_limits() {
        assert_eq!(panel_size_for_content(100, 80, 1.0), panel_size(1.0));
        assert_eq!(
            panel_size_for_content(PANEL_MAX_W + 100, PANEL_MAX_H + 100, 1.0),
            (PANEL_MAX_W, PANEL_MAX_H)
        );
    }

    #[test]
    fn scrollbar_thumb_reaches_both_ends() {
        let panel = RECT {
            left: 100,
            top: 100,
            right: 660,
            bottom: 520,
        };
        let top = panel_scrollbar_thumb_rect(panel, 1.0, 1000, 300, 0);
        let bottom = panel_scrollbar_thumb_rect(panel, 1.0, 1000, 300, 700);
        let track = panel_scrollbar_track_rect(panel, 1.0);
        assert_eq!(top.top, track.top);
        assert_eq!(bottom.bottom, track.bottom);
    }

    #[test]
    fn bar_matches_reference_metrics() {
        let (w, h) = bar_size(1.0);
        assert_eq!(h, 36, "工具条高 36px（5px padding + 26px 按钮）");
        assert_eq!(w, 132, "两个 60px 按钮 + 2px 间距 + 10px padding");
        let bar = RECT {
            left: 10,
            top: 20,
            right: 10 + w,
            bottom: 20 + h,
        };
        let [copy, _] = button_rects(bar, 1.0);
        assert_eq!(copy.top - bar.top, 5);
        assert_eq!(copy.bottom - copy.top, 26);
        assert_eq!(copy.right - copy.left, 60);
        let icon = icon_rect(copy, 1.0);
        assert_eq!((icon.right - icon.left, icon.bottom - icon.top), (16, 16));
        assert_eq!(icon.left - copy.left, 6);
        assert_eq!(label_origin(copy, 1.0).0 - icon.right, 4);
    }

    #[test]
    fn buttons_keep_the_same_gap_at_any_dpi() {
        for s in [1.0, 1.25, 1.5, 2.0] {
            let (w, _) = bar_size(s);
            let bar = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: scale(36, s),
            };
            let [copy, trans] = button_rects(bar, s);
            assert!(trans.left >= copy.right, "按钮不应重叠（scale {s}）");
            assert!(
                trans.right <= bar.right - scale(PAD, s),
                "按钮不应越界（scale {s}）"
            );
        }
    }
}
