//! 纯几何：显示器标识、点与矩形。
//!
//! 不依赖 Win32，方便在任意平台单测；`text-picker` 与 `show-desktop` 共用同一套类型，
//! 避免各自维护一份“屏幕矩形”。

use std::fmt;

/// 显示器句柄（`HMONITOR`）的稳定包装。
///
/// 用句柄而不是“第几块屏”作为键，显示器增减时不会让记忆/布局错位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId(pub usize);

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// 屏幕矩形（物理像素，左上/右下）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.left
            && point.x < self.right
            && point.y >= self.top
            && point.y < self.bottom
    }
}

/// 显示器信息，用于对外报告（定位 UI、日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// 整块屏幕的物理矩形。
    pub bounds: Rect,
    /// 扣除任务栏后的工作区。
    pub work: Rect,
    pub primary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_uses_half_open_interval() {
        let rect = Rect::new(0, 0, 1920, 1080);
        assert!(rect.contains(Point::new(0, 0)));
        assert!(rect.contains(Point::new(1919, 1079)));
        assert!(!rect.contains(Point::new(1920, 1079)));
        assert!(!rect.contains(Point::new(1919, 1080)));
        assert!(!rect.contains(Point::new(-1, 0)));
    }

    #[test]
    fn handles_negative_origin_for_left_or_top_monitors() {
        let rect = Rect::new(-1920, -200, 0, 880);
        assert_eq!(rect.width(), 1920);
        assert_eq!(rect.height(), 1080);
        assert!(rect.contains(Point::new(-1920, -200)));
        assert!(!rect.contains(Point::new(0, -200)));
    }

    #[test]
    fn degenerate_rect_has_no_area() {
        let rect = Rect::new(100, 100, 100, 200);
        assert_eq!(rect.width(), 0);
        assert_eq!(rect.height(), 100);
    }
}
