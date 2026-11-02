//! 无副作用的选区读取：UI Automation `TextPattern`。
//!
//! 取词路径：
//! 鼠标抬起点 → `ElementFromPoint` → `TextPattern.GetSelection()` →
//! `GetText()` + `GetBoundingRectangles()`。
//!
//! 矩形有两个用途：判断选区是否有效，以及给工具条定位（选区上方居中）。
//! 另外补两道过滤：密码框不取、焦点元素与命中元素不同进程时不取
//! （典型是 IME 候选窗抢走焦点）。

use std::ffi::c_void;

use whelper_common::geometry::Rect;
use windows::Win32::Foundation::{HWND, POINT, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextRange, IUIAutomationTreeWalker, UIA_IsPasswordPropertyId,
    UIA_ProcessIdPropertyId, UIA_TextPatternId,
};

/// 最多向上找几层祖先：焦点元素常常是终端内部的 textarea，文本模式挂在祖先上。
const MAX_ANCESTORS: usize = 8;

/// 一次 UIA 探测的结果。
pub enum Probe {
    /// 读到了非空选区。
    Selection(Selection),
    /// 应用暴露了文本模式，但当前没有选中内容；或压根没有文本模式。
    Empty,
    /// 拒绝：密码框、焦点与命中元素跨进程。**不要**再回退 Ctrl+C。
    Rejected,
}

/// 一次 UIA 取词的结果。
pub struct Selection {
    pub text: String,
    /// 选区在屏幕坐标下的外接矩形；UIA 没给出有效矩形时为 `None`。
    pub bounds: Option<Rect>,
}

/// 读取锚点附近的当前选区。
pub fn probe(foreground: HWND, anchor: POINT) -> Probe {
    let Some(_com) = Com::new() else {
        return Probe::Empty;
    };
    let Ok(automation) = (unsafe {
        CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }) else {
        return Probe::Empty;
    };

    let focused = unsafe { automation.GetFocusedElement() }.ok();
    let hit = unsafe { automation.ElementFromPoint(anchor) }.ok();

    // 焦点元素与命中元素不属于同一进程时不取（避免弹到 IME 候选窗上）。
    if let (Some(focused), Some(hit)) = (focused.as_ref(), hit.as_ref()) {
        let focused_pid = process_id(focused);
        let hit_pid = process_id(hit);
        if focused_pid != 0 && hit_pid != 0 && focused_pid != hit_pid {
            return Probe::Rejected;
        }
    }

    // 优先用鼠标抬起点命中的元素，失败再退回焦点元素/前台窗口。
    let mut candidates = Vec::new();
    if let Some(element) = hit {
        candidates.push(element);
    }
    if let Some(element) = focused {
        candidates.push(element);
    }
    if let Ok(element) = unsafe { automation.ElementFromHandle(foreground) } {
        candidates.push(element);
    }

    let Ok(walker) = (unsafe { automation.ControlViewWalker() }) else {
        return Probe::Empty;
    };
    candidates
        .iter()
        .map(|element| probe_from(&walker, element))
        .find(|probe| !matches!(probe, Probe::Empty))
        .unwrap_or(Probe::Empty)
}

fn probe_from(walker: &IUIAutomationTreeWalker, element: &IUIAutomationElement) -> Probe {
    let mut node = element.clone();
    for _ in 0..MAX_ANCESTORS {
        if let Ok(pattern) =
            unsafe { node.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
        {
            if is_password(&node) {
                return Probe::Rejected;
            }
            // 有 TextPattern 就以它为准：即使没有选中内容，也不必再往上找。
            return selection_text(&pattern);
        }
        match unsafe { walker.GetParentElement(&node) } {
            Ok(parent) => node = parent,
            Err(_) => break,
        }
    }
    Probe::Empty
}

fn selection_text(pattern: &IUIAutomationTextPattern) -> Probe {
    let Ok(ranges) = (unsafe { pattern.GetSelection() }) else {
        return Probe::Empty;
    };
    let Ok(count) = (unsafe { ranges.Length() }) else {
        return Probe::Empty;
    };

    let mut text = String::new();
    let mut bounds: Option<Rect> = None;
    for index in 0..count {
        let Ok(range) = (unsafe { ranges.GetElement(index) }) else {
            continue;
        };
        if let Ok(value) = unsafe { range.GetText(-1) } {
            text.push_str(&value.to_string());
        }
        if let Some(rect) = range_bounds(&range) {
            bounds = Some(match bounds {
                Some(current) => union(current, rect),
                None => rect,
            });
        }
    }

    if text.trim().is_empty() {
        Probe::Empty
    } else {
        Probe::Selection(Selection { text, bounds })
    }
}

/// `GetBoundingRectangles()` 返回 SAFEARRAY&lt;f64&gt;，每 4 个一组：left, top, width, height。
fn range_bounds(range: &IUIAutomationTextRange) -> Option<Rect> {
    let array = unsafe { range.GetBoundingRectangles() }.ok()?;
    if array.is_null() {
        return None;
    }

    unsafe {
        let lower = SafeArrayGetLBound(array, 1).ok()?;
        let upper = SafeArrayGetUBound(array, 1).ok()?;
        let count = (upper - lower + 1).max(0) as usize;
        if count < 4 {
            return None;
        }

        let mut data: *mut c_void = std::ptr::null_mut();
        if SafeArrayAccessData(array, &mut data).is_err() {
            return None;
        }
        let values = std::slice::from_raw_parts(data as *const f64, count);

        let mut bounds: Option<Rect> = None;
        let mut index = 0;
        while index + 4 <= values.len() {
            let chunk = &values[index..index + 4];
            index += 4;
            let rect = Rect::new(
                chunk[0] as i32,
                chunk[1] as i32,
                (chunk[0] + chunk[2]) as i32,
                (chunk[1] + chunk[3]) as i32,
            );
            if rect.width() <= 0 || rect.height() <= 0 {
                continue;
            }
            bounds = Some(match bounds {
                Some(current) => union(current, rect),
                None => rect,
            });
        }

        let _ = SafeArrayUnaccessData(array);
        bounds
    }
}

pub(crate) fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.left.min(b.left),
        a.top.min(b.top),
        a.right.max(b.right),
        a.bottom.max(b.bottom),
    )
}

fn process_id(element: &IUIAutomationElement) -> u32 {
    unsafe { element.GetCurrentPropertyValue(UIA_ProcessIdPropertyId) }
        .ok()
        .and_then(|value| i32::try_from(&value).ok())
        .unwrap_or(0) as u32
}

fn is_password(element: &IUIAutomationElement) -> bool {
    unsafe { element.GetCurrentPropertyValue(UIA_IsPasswordPropertyId) }
        .ok()
        .and_then(|value| bool::try_from(&value).ok())
        .unwrap_or(false)
}

/// COM 初始化守卫：取词跑在临时工作线程上，每个线程各自初始化/反初始化。
struct Com {
    uninit: bool,
}

impl Com {
    fn new() -> Option<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // 线程已在别的套间里：UIA 仍可创建，但退出时不能 CoUninitialize。
            return Some(Self { uninit: false });
        }
        if hr.is_err() {
            return None;
        }
        Some(Self { uninit: true })
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.uninit {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_covers_both_line_rects() {
        // 多行选区：两行矩形合并成一个外接矩形。
        let first = Rect::new(10, 100, 210, 120);
        let second = Rect::new(10, 120, 160, 140);
        let merged = union(first, second);
        assert_eq!(merged, Rect::new(10, 100, 210, 140));
        assert_eq!(merged.width(), 200);
        assert_eq!(merged.height(), 40);
    }
}
