//! 32 位 DIB 绘制面：给覆盖窗口做**逐像素 alpha**（`UpdateLayeredWindow`）。
//!
//! GDI 绘制只写 RGB、不写 alpha，这里的约定是：
//!
//! 1. 整面清零 → 初始全透明；
//! 2. GDI 绘制托盘、预览边框、文字等 RGB 内容；
//! 3. 用 [`Surface::set_alpha_round_rect`] 为托盘区域回填逐像素 alpha，可选直角或圆角；
//! 4. DWM 实时缩略图由系统合成在绘制面之上，不占这里的 alpha。
//!
//! 圆角区域按给定半径逐像素判定，与托盘的窗口区域保持一致。

use std::ffi::c_void;
use std::mem::size_of;

use whelper_common::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HANDLE, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// 一块由上到下的 32bpp 绘制面（内存里的 u32 是 `0xAARRGGBB`）。
pub struct Surface {
    width: i32,
    height: i32,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Surface {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.0.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    // 负高度 = 自上而下，和屏幕坐标一致。
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                HANDLE(std::ptr::null_mut()),
                0,
            ) {
                Ok(bitmap) => bitmap,
                Err(_) => {
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
            if bitmap.0.is_null() || bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self {
                width,
                height,
                dc,
                bitmap,
                old,
                bits: bits as *mut u32,
            })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    /// 把矩形内的 alpha 设为 `alpha`；指定圆角半径为 0 时绘制直角矩形。
    pub fn set_alpha_round_rect(&mut self, rect: Rect, radius: i32, clip: Rect, alpha: u8) {
        let left = rect.left.max(clip.left).max(0);
        let top = rect.top.max(clip.top).max(0);
        let right = rect.right.min(clip.right).min(self.width);
        let bottom = rect.bottom.min(clip.bottom).min(self.height);
        if left >= right || top >= bottom {
            return;
        }
        let radius = radius.min(rect.width().min(rect.height()) / 2).max(0);
        let value = (alpha as u32) << 24;
        // 圆角只影响上下两条半径带；中间整行都在形状内，直接写，省掉逐像素的圆角判定。
        let band_top = rect.top + radius;
        let band_bottom = rect.bottom - radius;
        for y in top..bottom {
            let row_fully_inside = y >= band_top && y < band_bottom;
            if row_fully_inside {
                for x in left..right {
                    let index = (y * self.width + x) as usize;
                    unsafe {
                        let pixel = self.bits.add(index);
                        *pixel = (*pixel & 0x00FF_FFFF) | value;
                    }
                }
            } else {
                for x in left..right {
                    if inside_round_rect(rect, radius, x, y) {
                        let index = (y * self.width + x) as usize;
                        unsafe {
                            let pixel = self.bits.add(index);
                            *pixel = (*pixel & 0x00FF_FFFF) | value;
                        }
                    }
                }
            }
        }
    }

    /// 提交到分层窗口：`alpha` 是整窗透明度（淡入淡出），与逐像素 alpha 相乘。
    pub fn apply(&self, hwnd: HWND, alpha: u8) -> bool {
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                HDC(std::ptr::null_mut()),
                None,
                Some(&size),
                self.dc,
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .is_ok()
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.old.0.is_null() {
                SelectObject(self.dc, self.old);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

/// 判定像素中心是否落在圆角矩形内（与 GDI `RoundRect` 的椭圆角一致，不做抗锯齿）。
fn inside_round_rect(rect: Rect, radius: i32, x: i32, y: i32) -> bool {
    if x < rect.left || y < rect.top || x >= rect.right || y >= rect.bottom {
        return false;
    }
    // 半径超过半边时圆角会互相吃掉，按几何上限夹紧（与调用方保持一致）。
    let radius = radius.min(rect.width().min(rect.height()) / 2).max(0) as f32;
    if radius <= 0.0 {
        return true;
    }
    let px = x as f32 + 0.5;
    let py = y as f32 + 0.5;
    let low_x = rect.left as f32 + radius;
    let high_x = rect.right as f32 - radius;
    let low_y = rect.top as f32 + radius;
    let high_y = rect.bottom as f32 - radius;
    let center_x = if low_x <= high_x {
        px.clamp(low_x, high_x)
    } else {
        (low_x + high_x) / 2.0
    };
    let center_y = if low_y <= high_y {
        py.clamp(low_y, high_y)
    } else {
        (low_y + high_y) / 2.0
    };
    let dx = px - center_x;
    let dy = py - center_y;
    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_rect_keeps_corners_transparent() {
        let rect = Rect::new(0, 0, 40, 40);
        assert!(inside_round_rect(rect, 12, 20, 20));
        assert!(inside_round_rect(rect, 12, 0, 20));
        assert!(!inside_round_rect(rect, 12, 0, 0));
        assert!(!inside_round_rect(rect, 12, 39, 39));
        assert!(inside_round_rect(Rect::new(0, 0, 40, 40), 0, 0, 0));
        assert!(!inside_round_rect(rect, 12, 40, 20));
    }

    #[test]
    fn round_rect_handles_radius_larger_than_half() {
        let rect = Rect::new(0, 0, 10, 6);
        assert!(inside_round_rect(rect, 8, 5, 3));
        assert!(!inside_round_rect(rect, 8, 0, 0));
    }
}
