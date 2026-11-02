//! 应用图标单行与窗口预览网格的几何、命中测试和布局插值。
//!
//! 全部使用**屏幕坐标**（与 `whelper_common::geometry::Rect` 一致），覆盖窗口绘制时
//! 减去 `panel` 的原点即可。不依赖 Win32，可跨平台单测。
//!
//! ```text
//! 缩略图（Thumbnails）                 网格（Grid）
//! ┌───────────────────────────┐       ┌───────────────────────────┐
//! │  ◉   ◉   ◉   ◉   ◉         │       │ ▢ ▢ ▢ ▢                   │
//! └───────────────────────────┘       │ ▢ ▢ ▢ ▢                   │
//!                                     └───────────────────────────┘
//! ```
//!
//! 缩略图视图用应用图标单行排列；网格视图用“窗口预览 + 标题行”展示各个窗口。
//!
//! 两种模式共用同一份 [`Cell`] 结构，因此 [`lerp`] 可以在模式切换时把每个预览项从
//! 旧位置平滑移到新位置（候选数量不变）。

use whelper_common::geometry::{Point, Rect};

use crate::model::ViewMode;

// 尺寸基线（逻辑像素，按 DPI 缩放）：图标条单行展示应用；网格项为预览图 + 标题行。

/// 缩略图宽/高（16:10）。
pub const THUMB_W: i32 = 224;
pub const THUMB_H: i32 = 132;
/// 卡片内标题行高（图标 + 文字）。
pub const TITLE_H: i32 = 26;
/// 面板内边距。
pub const PAD: i32 = 20;
/// 卡片间距。
pub const GAP: i32 = 12;
/// 面板距工作区边缘的最小留白。
pub const EDGE_MARGIN: i32 = 16;
/// 图标条和网格面板最多占工作区宽度的比例（百分比）。
const PANEL_MAX_WIDTH_PCT: i32 = 96;
/// 图标项尺寸。
const ICON_CELL: i32 = 56;
const APP_ICON: i32 = 48;
const ICON_PAD: i32 = 12;
const ICON_GAP: i32 = 8;
/// 多行布局最多占工作区高度的比例。
const LAYOUT_MAX_HEIGHT_PCT: i32 = 82;

/// 按 DPI 缩放一个逻辑像素值。
pub fn scale(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round().max(1.0) as i32
}

/// 一个当前视图候选：可以是应用图标，也可以是窗口预览图区和标题区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub index: usize,
    pub frame: Rect,
    pub thumb: Rect,
    pub title: Rect,
}

/// 一次布局的结果。`cells` 与当前模式下的候选一一对应（包括滚出可见区域的项目），
/// 这样候选在视图切换时有起点与终点可插值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub mode: ViewMode,
    pub panel: Rect,
    pub cells: Vec<Cell>,
    pub selected: usize,
    /// 当前布局列数；`Row*` 步长按此值在相邻行间移动。
    pub columns: usize,
    /// 可见区间 `[start, end)`；屏幕空间不足时隐藏未显示的图标或预览项。
    pub visible: (usize, usize),
}

impl Layout {
    /// 项目是否在可见区间内（滚出托盘的行仍存在，但不绘制、不响应鼠标）。
    pub fn is_visible(&self, index: usize) -> bool {
        index >= self.visible.0 && index < self.visible.1
    }

    /// 屏幕坐标 → 当前视图候选下标。
    pub fn hit_test(&self, point: Point) -> Option<usize> {
        if !self.panel.contains(point) {
            return None;
        }
        self.cells
            .iter()
            .find(|cell| self.is_visible(cell.index) && cell.frame.contains(point))
            .map(|cell| cell.index)
    }

    /// 选中项的高亮外框。
    pub fn highlight(&self) -> Option<Rect> {
        self.cells
            .get(self.selected)
            .map(|cell| inflate(cell.frame, 2))
    }
}

/// 计算指定视图的布局。`len == 0` 时返回空面板（调用方不应在空列表上弹窗）。
pub fn compute(mode: ViewMode, len: usize, selected: usize, scale: f32, work: Rect) -> Layout {
    if len == 0 {
        return Layout {
            mode,
            panel: Rect::default(),
            cells: Vec::new(),
            selected: 0,
            columns: 1,
            visible: (0, 0),
        };
    }
    let selected = selected.min(len - 1);
    match mode {
        ViewMode::Thumbnails => strip(len, selected, scale, work),
        ViewMode::Grid => grid(len, selected, scale, work),
    }
}

/// 缩略图模式：一行显示应用图标，候选过多时水平滚动并保持选择可见。
fn strip(len: usize, selected: usize, s: f32, work: Rect) -> Layout {
    let selected = selected.min(len - 1);
    let edge = scale(EDGE_MARGIN, s);
    let bound_w = (work.width() - edge * 2).max(1);
    let bound_h = (work.height() - edge * 2).max(1);
    let pad = scale(ICON_PAD, s).min((bound_w.min(bound_h) - 1) / 2);
    let gap = scale(ICON_GAP, s).min((bound_w - 1).max(0));
    let cell = scale(ICON_CELL, s).min((bound_w - pad * 2).max(1));
    let cell = cell.min((bound_h - pad * 2).max(1));
    let icon = scale(APP_ICON, s).min(cell);
    let max_w = (bound_w * PANEL_MAX_WIDTH_PCT / 100)
        .max(cell + pad * 2)
        .min(bound_w);
    let inner_w = (max_w - pad * 2).max(cell);
    let max_visible = (((inner_w + gap) / (cell + gap)).max(1)) as usize;
    let visible_count = len.min(max_visible);
    let first = selected
        .saturating_sub(visible_count / 2)
        .min(len.saturating_sub(visible_count));
    let end = first + visible_count;
    let panel_w = pad * 2 + visible_count as i32 * cell + (visible_count as i32 - 1) * gap;
    let panel_h = pad * 2 + cell;
    let left = work.left + (work.width() - panel_w) / 2;
    let top = work.top + (work.height() - panel_h) / 2;
    let panel = Rect::new(left, top, left + panel_w, top + panel_h);
    let cells = (0..len)
        .map(|index| {
            let x = panel.left + pad + (index as i32 - first as i32) * (cell + gap);
            let y = panel.top + pad;
            let frame = Rect::new(x, y, x + cell, y + cell);
            let icon_left = x + (cell - icon) / 2;
            let icon_top = y + (cell - icon) / 2;
            let thumb = Rect::new(icon_left, icon_top, icon_left + icon, icon_top + icon);
            Cell {
                index,
                frame,
                thumb,
                title: Rect::new(x, y + cell, x + cell, y + cell),
            }
        })
        .collect();
    Layout {
        mode: ViewMode::Thumbnails,
        panel,
        cells,
        selected,
        columns: 1,
        visible: (first, end),
    }
}

/// 网格模式：每个窗口显示一张较大的实时预览卡片。
fn grid(len: usize, selected: usize, s: f32, work: Rect) -> Layout {
    grid_layout(
        ViewMode::Grid,
        len,
        selected,
        s,
        work,
        PANEL_MAX_WIDTH_PCT,
        scale(THUMB_W, s),
    )
}

/// 为网格视图挑选预览面积较大的行列布局；超出容量时按行滚动并保持选择可见。
fn grid_layout(
    mode: ViewMode,
    len: usize,
    selected: usize,
    s: f32,
    work: Rect,
    max_width_pct: i32,
    max_thumb_w: i32,
) -> Layout {
    let pad = scale(PAD, s);
    let gap = scale(GAP, s);
    let title_h = scale(TITLE_H, s);
    let edge = scale(EDGE_MARGIN, s);
    let bound_w = (work.width() - edge * 2).max(1);
    let bound_h = (work.height() - edge * 2).max(1);

    let max_w = (bound_w * max_width_pct / 100)
        .max(scale(120, s))
        .min(bound_w);
    let max_h = (bound_h * LAYOUT_MAX_HEIGHT_PCT / 100)
        .max(scale(100, s))
        .min(bound_h);
    let inner_w = (max_w - pad * 2).max(1);
    let inner_h = (max_h - pad * 2).max(1);
    let preferred_thumb_w = max_thumb_w;
    let preferred_thumb_h = scale(THUMB_H, s);
    let min_thumb_w = scale(64, s);
    let min_thumb_h = scale(40, s);
    let max_rows = (((inner_h + gap) / (min_thumb_h + title_h + gap)).max(1) as usize).min(6);
    let max_columns = len.min(8);
    let preferred_columns = ((len as f64).sqrt().round() as usize).clamp(1, max_columns);

    let mut best: Option<(usize, usize, i32, i32, i64)> = None;
    for columns in 1..=max_columns {
        let rows = len.div_ceil(columns).min(max_rows);
        let avail_w = inner_w - gap * (columns as i32 - 1);
        let avail_h = inner_h - gap * (rows as i32 - 1);
        if avail_w <= 0 || avail_h <= 0 {
            continue;
        }
        let cell_w = avail_w / columns as i32;
        let cell_h = avail_h / rows as i32;
        let thumb_h_limit = cell_h - title_h;
        if cell_w < min_thumb_w || thumb_h_limit < min_thumb_h {
            continue;
        }
        let thumb_w = cell_w.min(preferred_thumb_w).min(thumb_h_limit * 16 / 10);
        let thumb_h = (thumb_w * 10 / 16)
            .min(thumb_h_limit)
            .min(preferred_thumb_h);
        let shown = len.min(columns * rows);
        let score = thumb_w as i64 * thumb_h as i64 * shown as i64;
        let preferred_penalty =
            (columns as isize - preferred_columns as isize).unsigned_abs() as i64;
        let score = score * 100 - preferred_penalty;
        if best.is_none_or(|(_, _, _, _, best_score)| score > best_score) {
            best = Some((columns, rows, thumb_w, thumb_h, score));
        }
    }

    let (columns, visible_rows, thumb_w, thumb_h) = best
        .map(|(columns, rows, thumb_w, thumb_h, _)| (columns, rows, thumb_w, thumb_h))
        .unwrap_or_else(|| (1, 1, inner_w.max(1), (inner_h - title_h).max(1)));
    let total_rows = len.div_ceil(columns);
    let visible_rows = visible_rows.min(total_rows);
    let selected_row = selected / columns;
    let first_row = selected_row
        .saturating_sub(visible_rows / 2)
        .min(total_rows.saturating_sub(visible_rows));
    let visible_start = first_row * columns;
    let visible_end = (visible_start + columns * visible_rows).min(len);
    let rows = visible_rows;
    let cell_w = thumb_w;
    let cell_h = thumb_h + title_h;
    let panel_w = pad * 2 + columns as i32 * cell_w + gap * (columns as i32 - 1);
    let panel_h = pad * 2 + rows as i32 * cell_h + gap * (rows as i32 - 1);
    let panel_left = work.left + (work.width() - panel_w) / 2;
    let panel_top = work.top + (work.height() - panel_h) / 2;
    let panel = Rect::new(
        panel_left,
        panel_top,
        panel_left + panel_w,
        panel_top + panel_h,
    );
    let inner = panel.width() - pad * 2;

    let cells = (0..len)
        .map(|index| {
            let row = index / columns;
            let column = index % columns;
            // 最后一行不满时居中，避免固定左对齐导致的锯齿。
            let row_count = columns.min(len - row * columns);
            let row_w = row_count as i32 * cell_w + gap * (row_count as i32 - 1);
            let row_left = panel.left + pad + (inner - row_w) / 2;
            let x = row_left + column as i32 * (cell_w + gap);
            let y = panel.top + pad + (row as i32 - first_row as i32) * (cell_h + gap);
            let frame = Rect::new(x, y, x + cell_w, y + cell_h);
            let thumb = Rect::new(x, y, x + thumb_w, y + thumb_h);
            let title = Rect::new(x, thumb.bottom, x + cell_w, frame.bottom);
            Cell {
                index,
                frame,
                thumb,
                title,
            }
        })
        .collect();

    Layout {
        mode,
        panel,
        cells,
        selected,
        columns,
        visible: (visible_start, visible_end),
    }
}

/// 两个布局之间插值（模式切换 / 选择移动的过渡）。卡片数量不同则直接取目标布局。
///
/// 只插值**几何**：`mode` / `selected` / `columns` / `visible` 立即取目标值。
/// 这样过渡期间高亮已经落在新卡片上，而卡片本身从旧位置飞过去；
/// `t = 0` 时画面停在旧几何，`t = 1` 时与 `to` 完全相等。
pub fn lerp(from: &Layout, to: &Layout, t: f32) -> Layout {
    if from.cells.len() != to.cells.len() {
        return to.clone();
    }
    let t = t.clamp(0.0, 1.0);
    Layout {
        mode: to.mode,
        panel: lerp_rect(from.panel, to.panel, t),
        cells: from
            .cells
            .iter()
            .zip(to.cells.iter())
            .map(|(a, b)| Cell {
                index: b.index,
                frame: lerp_rect(a.frame, b.frame, t),
                thumb: lerp_rect(a.thumb, b.thumb, t),
                title: lerp_rect(a.title, b.title, t),
            })
            .collect(),
        selected: to.selected,
        columns: to.columns,
        visible: to.visible,
    }
}

/// 围绕面板中心整体缩放（入场时的“轻微放大”，与原生 Alt+Tab 一致）。
pub fn scaled(layout: &Layout, factor: f32) -> Layout {
    let factor = factor.max(0.1);
    let panel = layout.panel;
    let cx = (panel.left + panel.right) as f32 / 2.0;
    let cy = (panel.top + panel.bottom) as f32 / 2.0;
    let scale_rect = |rect: Rect| {
        Rect::new(
            (cx + (rect.left as f32 - cx) * factor).round() as i32,
            (cy + (rect.top as f32 - cy) * factor).round() as i32,
            (cx + (rect.right as f32 - cx) * factor).round() as i32,
            (cy + (rect.bottom as f32 - cy) * factor).round() as i32,
        )
    };
    Layout {
        mode: layout.mode,
        panel: scale_rect(panel),
        cells: layout
            .cells
            .iter()
            .map(|cell| Cell {
                index: cell.index,
                frame: scale_rect(cell.frame),
                thumb: scale_rect(cell.thumb),
                title: scale_rect(cell.title),
            })
            .collect(),
        selected: layout.selected,
        columns: layout.columns,
        visible: layout.visible,
    }
}

/// 把源尺寸按比例放进目标矩形（居中 + 留白），用于缩略图不拉伸变形。
pub fn fit(src_w: i32, src_h: i32, dst: Rect) -> Rect {
    if src_w <= 0 || src_h <= 0 || dst.width() <= 0 || dst.height() <= 0 {
        return dst;
    }
    let ratio = (dst.width() as f32 / src_w as f32).min(dst.height() as f32 / src_h as f32);
    let width = ((src_w as f32 * ratio).round() as i32).clamp(1, dst.width());
    let height = ((src_h as f32 * ratio).round() as i32).clamp(1, dst.height());
    let left = dst.left + (dst.width() - width) / 2;
    let top = dst.top + (dst.height() - height) / 2;
    Rect::new(left, top, left + width, top + height)
}

fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    let f = |a: i32, b: i32| a + ((b - a) as f32 * t).round() as i32;
    Rect::new(
        f(from.left, to.left),
        f(from.top, to.top),
        f(from.right, to.right),
        f(from.bottom, to.bottom),
    )
}

fn inflate(rect: Rect, by: i32) -> Rect {
    Rect::new(
        rect.left - by,
        rect.top - by,
        rect.right + by,
        rect.bottom + by,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0, 0, 1920, 1040);

    fn inside(outer: Rect, inner: Rect) -> bool {
        inner.left >= outer.left
            && inner.top >= outer.top
            && inner.right <= outer.right
            && inner.bottom <= outer.bottom
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    fn entries(len: usize) -> Vec<usize> {
        (0..len).collect()
    }

    #[test]
    fn strip_keeps_single_row_app_icons_inside_the_panel() {
        let layout = compute(ViewMode::Thumbnails, 12, 0, 1.0, WORK);
        assert_eq!(layout.cells.len(), 12);
        assert_eq!(layout.visible, (0, 12));
        assert_eq!(layout.columns, 1);
        for cell in &layout.cells {
            assert!(inside(layout.panel, cell.frame), "图标项应在面板内");
            assert!(inside(cell.frame, cell.thumb), "应用图标应在图标项内");
            assert_eq!(cell.title.height(), 0, "应用图标视图不显示标题");
        }
        assert!(inside(WORK, layout.panel));
    }

    #[test]
    fn strip_is_centred_and_keeps_icons_on_one_row() {
        let layout = compute(ViewMode::Thumbnails, 3, 0, 1.0, WORK);
        assert_eq!(
            layout.panel.top,
            WORK.top + (WORK.height() - layout.panel.height()) / 2
        );
        assert_eq!(
            layout.panel.left,
            WORK.left + (WORK.width() - layout.panel.width()) / 2
        );
        assert_eq!(layout.columns, 1);
        assert_eq!(layout.cells[0].frame.top, layout.cells[2].frame.top);
    }

    #[test]
    fn icon_strip_keeps_selection_visible_when_scrolling() {
        let layout = compute(ViewMode::Thumbnails, 100, 0, 1.0, WORK);
        let visible = layout.visible.1 - layout.visible.0;
        assert!(visible < 100, "大量应用图标应横向滚动");
        assert!(layout.is_visible(0));

        let last = compute(ViewMode::Thumbnails, 100, 99, 1.0, WORK);
        assert!(last.is_visible(99), "选中的应用图标必须可见");
        assert!(last.visible.0 > 0, "选择移动到后续项时图标条应滚动");
    }

    #[test]
    fn strip_displays_all_apps_on_one_row_when_they_fit() {
        let layout = compute(ViewMode::Thumbnails, 12, 3, 1.0, WORK);
        assert_eq!(layout.visible, (0, 12));
        assert_eq!(layout.columns, 1);
        assert!(layout
            .cells
            .iter()
            .all(|cell| cell.frame.top == layout.cells[0].frame.top));
    }

    #[test]
    fn selection_is_clamped_to_the_window_list() {
        let layout = compute(ViewMode::Grid, 4, 99, 1.0, WORK);
        assert_eq!(layout.selected, 3);
    }

    #[test]
    fn grid_cards_do_not_overlap_and_stay_inside_the_panel() {
        for len in [1usize, 2, 3, 4, 5, 8, 13, 21] {
            let layout = compute(ViewMode::Grid, len, 0, 1.0, WORK);
            assert_eq!(layout.cells.len(), len);
            assert_eq!(layout.visible, (0, len));
            assert!(layout.columns >= 1);
            assert!(
                inside(WORK, layout.panel),
                "{len} 个窗口：面板越界 {:?}",
                layout.panel
            );
            for cell in &layout.cells {
                assert!(inside(layout.panel, cell.frame), "{len} 个窗口：卡片越界");
                assert!(inside(cell.frame, cell.thumb), "{len} 个窗口：缩略图越界");
                assert!(inside(cell.frame, cell.title), "{len} 个窗口：标题越界");
            }
            for (i, a) in layout.cells.iter().enumerate() {
                for b in layout.cells.iter().skip(i + 1) {
                    assert!(!overlap(a.frame, b.frame), "{len} 个窗口：卡片重叠");
                }
            }
        }
    }

    #[test]
    fn grid_last_row_is_centred() {
        let layout = compute(ViewMode::Grid, 10, 0, 1.0, WORK);
        let columns = layout.columns;
        let last_row_start = (layout.cells.len() - 1) / columns * columns;
        let first_row_left = layout.cells[0].frame.left;
        let last_row_left = layout.cells[last_row_start].frame.left;
        assert!(last_row_left > first_row_left, "不满一行时最后一行应居中");
    }

    #[test]
    fn hit_test_finds_cards_and_rejects_gaps() {
        let layout = compute(ViewMode::Grid, 4, 0, 1.0, WORK);
        let cell = layout.cells[2];
        let centre = Point::new(
            cell.frame.left + cell.frame.width() / 2,
            cell.frame.top + cell.frame.height() / 2,
        );
        assert_eq!(layout.hit_test(centre), Some(2));

        // 面板外与卡片间的空隙都不命中。
        assert_eq!(
            layout.hit_test(Point::new(WORK.right - 1, WORK.bottom - 1)),
            None
        );
        let gap_point = Point::new(cell.frame.right + 2, cell.frame.top + 2);
        assert_eq!(layout.hit_test(gap_point), None);
    }

    #[test]
    fn rows_outside_the_visible_range_are_not_hit_testable() {
        let layout = compute(ViewMode::Thumbnails, 100, 99, 1.0, WORK);
        let hidden = layout.cells[0];
        assert!(!layout.is_visible(0));
        assert_eq!(
            layout.hit_test(Point::new(hidden.frame.left + 1, hidden.frame.top + 1)),
            None
        );
    }

    #[test]
    fn lerp_hits_both_endpoints() {
        let from = compute(ViewMode::Grid, 30, 0, 1.0, WORK);
        let to = compute(ViewMode::Thumbnails, 30, 3, 1.0, WORK);

        // 起点：几何停在旧布局，但选择/视图已经切到目标值。
        let start = lerp(&from, &to, 0.0);
        assert_eq!(start.panel, from.panel);
        assert_eq!(start.cells, from.cells);
        assert_eq!(start.selected, to.selected);
        assert_eq!(start.mode, to.mode);

        // 终点：与目标布局完全一致。
        assert_eq!(lerp(&from, &to, 1.0), to);

        let middle = lerp(&from, &to, 0.5);
        assert_eq!(middle.selected, 3);
        assert_eq!(middle.mode, ViewMode::Thumbnails);
        assert!(middle.panel.width() > 0);
        assert!(middle.panel.height() > 0);
    }

    #[test]
    fn lerp_with_different_card_counts_snaps() {
        let from = compute(ViewMode::Grid, 4, 0, 1.0, WORK);
        let to = compute(ViewMode::Thumbnails, 6, 3, 1.0, WORK);
        assert_eq!(lerp(&from, &to, 0.3), to);
    }

    #[test]
    fn scaled_shrinks_around_the_panel_centre() {
        let layout = compute(ViewMode::Grid, 6, 0, 1.0, WORK);
        let small = scaled(&layout, 0.5);
        let panel_centre = (
            (layout.panel.left + layout.panel.right) / 2,
            (layout.panel.top + layout.panel.bottom) / 2,
        );
        let small_centre = (
            (small.panel.left + small.panel.right) / 2,
            (small.panel.top + small.panel.bottom) / 2,
        );
        assert!((panel_centre.0 - small_centre.0).abs() <= 1);
        assert!((panel_centre.1 - small_centre.1).abs() <= 1);
        assert!(small.panel.width() < layout.panel.width());
        assert!(small.cells[0].thumb.width() < layout.cells[0].thumb.width());
    }

    #[test]
    fn fit_preserves_aspect_ratio_and_centres() {
        let dst = Rect::new(10, 20, 210, 220);
        let fitted = fit(400, 200, dst);
        assert_eq!(fitted.width(), 200);
        assert_eq!(fitted.height(), 100);
        assert_eq!(fitted.top, 20 + (200 - 100) / 2);

        // 竖图：高度撑满，左右留白。
        let tall = fit(100, 400, dst);
        assert_eq!(tall.height(), 200);
        assert_eq!(tall.width(), 50);
        assert_eq!(tall.left, 10 + (200 - 50) / 2);

        // 非法输入原样返回，避免除零。
        assert_eq!(fit(0, 100, dst), dst);
    }

    #[test]
    fn highlight_wraps_the_selected_card() {
        let layout = compute(ViewMode::Grid, 5, 2, 1.0, WORK);
        let highlight = layout.highlight().expect("有选中卡片");
        assert!(highlight.contains(Point::new(
            layout.cells[2].frame.left,
            layout.cells[2].frame.top
        )));
        assert!(highlight.width() > layout.cells[2].frame.width());
    }

    #[test]
    fn hi_dpi_scales_preview_sizes() {
        let normal = compute(ViewMode::Thumbnails, 3, 0, 1.0, WORK);
        let scaled = compute(ViewMode::Thumbnails, 3, 0, 1.5, Rect::new(0, 0, 2880, 1560));
        assert!(scaled.cells[0].thumb.width() > normal.cells[0].thumb.width());
        assert!(scaled.cells[0].thumb.height() > normal.cells[0].thumb.height());
        assert!(inside(Rect::new(0, 0, 2880, 1560), scaled.panel));
    }

    #[test]
    fn tiny_work_area_does_not_panic() {
        for len in [1usize, 7, 30] {
            let work = Rect::new(0, 0, 320, 240);
            for mode in [ViewMode::Thumbnails, ViewMode::Grid] {
                let layout = compute(mode, len, 0, 2.0, work);
                assert_eq!(layout.cells.len(), len);
                assert!(layout.panel.width() > 0 && layout.panel.height() > 0);
            }
        }
        assert_eq!(entries(3).len(), 3);
    }
}
