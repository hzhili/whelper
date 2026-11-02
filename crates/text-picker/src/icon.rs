//! 工具条图标：把矢量轮廓光栅化成覆盖率蒙版，再按主题色混合进 GDI 画布。
//!
//! 翻译图标使用内置的 24×24 SVG path（M/L/H/C/Z）；复制图标按同一 24×24
//! 网格用圆角矩形拼出（后页被前页遮住的部分先挖空）。
//!
//! 纯几何部分不依赖 Win32，可单元测试；只有最后的 `paint` 需要 HDC。

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{GetPixel, SetPixelV, HDC};

/// 「翻译」图标（24×24，fill 规则）。
pub const TRANSLATE_PATH: &str = "M2.99983 13.9999C3.55211 13.9999 3.99983 14.4476 3.99983 14.9999C3.99983 15.9669 4.00251 16.3345 4.04865 16.6258C4.31985 18.3371 5.66256 19.6798 7.37385 19.951C7.66517 19.9972 8.03277 19.9999 8.99983 19.9999H9.24983C9.80212 19.9999 10.2498 20.4476 10.2498 20.9999C10.2497 21.552 9.80204 21.9999 9.24983 21.9999H8.99983C8.1092 21.9999 7.54729 22.0026 7.06135 21.9256C4.49414 21.519 2.48073 19.5055 2.07404 16.9383C1.99709 16.4524 1.99982 15.8905 1.99982 14.9999C1.99982 14.4477 2.44764 14 2.99983 13.9999ZM15.6385 10.8309C16.1654 9.67519 17.8324 9.67259 18.3602 10.8309L18.41 10.9501L21.9393 20.6581C22.1279 21.177 21.8605 21.7505 21.3416 21.9393C20.8228 22.1278 20.2492 21.8604 20.0604 21.3417L19.0272 18.4999H14.9735L13.9393 21.3417C13.7504 21.8604 13.1769 22.128 12.658 21.9393C12.1393 21.7505 11.8719 21.1769 12.0604 20.6581L13.3231 17.1844L15.5897 10.951L15.6385 10.8309ZM15.7 16.4999H18.2996L16.9998 12.9256L15.7 16.4999ZM6.68342 1.55162C7.20727 1.37701 7.77331 1.6597 7.94807 2.18346L8.38655 3.49986H11.9998C12.5521 3.49986 12.9998 3.94758 12.9998 4.49986C12.9997 5.05204 12.552 5.49986 11.9998 5.49986H11.1912C10.717 6.70677 10.0039 8.16843 8.80745 9.52037C9.72142 10.2406 10.8311 10.7913 12.1639 11.0135C12.7086 11.1043 13.0768 11.6193 12.9862 12.1639C12.8953 12.7084 12.3803 13.0768 11.8358 12.9862C10.0063 12.6812 8.51969 11.8922 7.3387 10.9032C6.2746 11.7352 4.94963 12.4591 3.28693 12.9579C2.75815 13.1162 2.20053 12.8157 2.04182 12.287C1.88333 11.7581 2.18397 11.2006 2.71272 11.0419C4.01358 10.6516 5.06496 10.1019 5.92268 9.46568C5.9017 9.44011 5.87988 9.41516 5.8592 9.38951C4.80052 8.07615 4.11995 6.64312 3.7469 5.49986H2.99983C2.44772 5.49975 1.99995 5.05197 1.99982 4.49986C1.99982 3.94765 2.44764 3.49998 2.99983 3.49986H6.27912L6.05159 2.81627C5.877 2.29251 6.15983 1.72649 6.68342 1.55162ZM14.9998 1.99986C15.8905 1.99986 16.4524 1.99715 16.9383 2.07408C19.5057 2.48071 21.519 4.49403 21.9256 7.06139C22.0026 7.54732 21.9998 8.10923 21.9998 8.99986C21.9997 9.55204 21.552 9.99986 20.9998 9.99986C20.4477 9.99975 20 9.55197 19.9998 8.99986C19.9998 8.03286 19.9971 7.66519 19.951 7.37389C19.6799 5.66245 18.3372 4.31983 16.6258 4.04869C16.3345 4.00258 15.9669 3.99986 14.9998 3.99986C14.4477 3.99975 14 3.55197 13.9998 2.99986C13.9998 2.44765 14.4476 1.99998 14.9998 1.99986ZM5.87287 5.49986C6.19927 6.3039 6.69619 7.2336 7.39045 8.1024C8.12345 7.25376 8.63856 6.3485 9.02327 5.49986H5.87287Z";

type Poly = Vec<(f32, f32)>;

/// 覆盖率蒙版：`a[y * w + x]` 为 0..=255。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Mask {
    fn zeros(w: usize, h: usize) -> Self {
        Mask { w, h, a: vec![0; w * h] }
    }

    #[cfg(test)]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.a[y * self.w + x]
    }
}

/// 解析 SVG path 的 M / L / H / C / Z（本图标只用到绝对命令）。
fn flatten(d: &str) -> Vec<Poly> {
    let mut polys: Vec<Poly> = Vec::new();
    let mut current: Poly = Vec::new();
    let mut cmd = ' ';
    let mut nums: Vec<f32> = Vec::new();
    let bytes: Vec<char> = d.chars().collect();
    let mut i = 0usize;

    fn close(current: &mut Poly, polys: &mut Vec<Poly>) {
        if current.len() > 2 {
            polys.push(std::mem::take(current));
        } else {
            current.clear();
        }
    }

    fn flush(cmd: char, nums: &[f32], current: &mut Poly) {
        match cmd {
            'M' | 'L' => {
                for pair in nums.chunks_exact(2) {
                    current.push((pair[0], pair[1]));
                }
            }
            'H' => {
                if let Some(x) = nums.first() {
                    let y = current.last().map(|p| p.1).unwrap_or(0.0);
                    current.push((*x, y));
                }
            }
            'C' => {
                for six in nums.chunks_exact(6) {
                    let p0 = *current.last().unwrap_or(&(0.0, 0.0));
                    cubic(p0, (six[0], six[1]), (six[2], six[3]), (six[4], six[5]), current);
                }
            }
            _ => {}
        }
    }

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() {
            flush(cmd, &nums, &mut current);
            nums.clear();
            match c {
                'Z' | 'z' => {
                    close(&mut current, &mut polys);
                    cmd = ' ';
                }
                'M' => {
                    // 新的 M 总是开启新子路径。
                    close(&mut current, &mut polys);
                    cmd = 'M';
                }
                _ => cmd = c,
            }
            i += 1;
        } else if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                i += 1;
            }
            let text: String = bytes[start..i].iter().collect();
            nums.push(text.parse().unwrap_or(0.0));
        } else {
            i += 1;
        }
    }
    flush(cmd, &nums, &mut current);
    close(&mut current, &mut polys);
    polys
}

fn cubic(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), out: &mut Poly) {
    const STEPS: usize = 12;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let mt = 1.0 - t;
        let x = mt * mt * mt * p0.0 + 3.0 * mt * mt * t * p1.0 + 3.0 * mt * t * t * p2.0 + t * t * t * p3.0;
        let y = mt * mt * mt * p0.1 + 3.0 * mt * mt * t * p1.1 + 3.0 * mt * t * t * p2.1 + t * t * t * p3.1;
        out.push((x, y));
    }
}

fn rr_poly(x: f32, y: f32, w: f32, h: f32, r: f32) -> Poly {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    const SEG: usize = 6;
    let mut pts = Vec::with_capacity(4 * (SEG + 1));
    let corners = [
        (x + w - r, y + r, -std::f32::consts::FRAC_PI_2),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, std::f32::consts::FRAC_PI_2),
        (x + r, y + r, std::f32::consts::PI),
    ];
    for (cx, cy, start) in corners {
        for s in 0..=SEG {
            let a = start + std::f32::consts::FRAC_PI_2 * (s as f32 / SEG as f32);
            pts.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts
}

fn ring(x: f32, y: f32, w: f32, h: f32, r: f32, thickness: f32) -> Vec<Poly> {
    let outer = rr_poly(x, y, w, h, r);
    let mut inner = rr_poly(x + thickness, y + thickness, w - thickness * 2.0, h - thickness * 2.0, (r - thickness).max(0.5));
    inner.reverse();
    vec![outer, inner]
}

fn winding(polys: &[Poly], x: f32, y: f32) -> i32 {
    let mut w = 0;
    for poly in polys {
        let n = poly.len();
        for i in 0..n {
            let (x0, y0) = poly[i];
            let (x1, y1) = poly[(i + 1) % n];
            let cross = (x1 - x0) * (y - y0) - (y1 - y0) * (x - x0);
            if y0 <= y {
                if y1 > y && cross > 0.0 {
                    w += 1;
                }
            } else if y1 <= y && cross < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

/// 把若干闭合轮廓光栅化成 `px × px` 的抗锯齿覆盖率蒙版。
fn rasterize(polys: &[Poly], px: usize) -> Mask {
    assert!(px > 0);
    let mut mask = Mask::zeros(px, px);
    const S: usize = 4;
    for py in 0..px {
        for pxx in 0..px {
            let mut hit = 0u32;
            for sy in 0..S {
                for sx in 0..S {
                    let x = (pxx as f32 + (sx as f32 + 0.5) / S as f32) / px as f32 * 24.0;
                    let y = (py as f32 + (sy as f32 + 0.5) / S as f32) / px as f32 * 24.0;
                    if winding(polys, x, y) != 0 {
                        hit += 1;
                    }
                }
            }
            mask.a[py * px + pxx] = (hit * 255 / (S * S) as u32) as u8;
        }
    }
    mask
}

pub fn translate_mask(px: usize) -> Mask {
    rasterize(&flatten(TRANSLATE_PATH), px)
}

/// 复制图标：后页 + 前页两张蒙版；后页在前页覆盖处已挖空。
pub fn copy_masks(px: usize) -> (Mask, Mask) {
    let ring_t = 1.7;
    let back = ring(8.6, 2.4, 12.4, 15.0, 2.4, ring_t);
    let front = ring(3.0, 6.6, 12.4, 15.0, 2.4, ring_t);
    let front_fill = vec![rr_poly(3.0, 6.6, 12.4, 15.0, 2.4)];
    let mut back_mask = rasterize(&back, px);
    let fill_mask = rasterize(&front_fill, px);
    for i in 0..back_mask.a.len() {
        let keep = 255 - fill_mask.a[i] as u32;
        back_mask.a[i] = ((back_mask.a[i] as u32 * keep) / 255) as u8;
    }
    let front_mask = rasterize(&front, px);
    (back_mask, front_mask)
}

/// 把蒙版按 `color` 混合到 HDC 的 `(x, y)` 处。
///
/// 图标区域只有十几个像素见方，逐像素 `GetPixel`/`SetPixelV` 的开销可以忽略。
pub unsafe fn paint(hdc: HDC, x: i32, y: i32, mask: &Mask, color: COLORREF) {
    let cr = color.0 & 0xff;
    let cg = (color.0 >> 8) & 0xff;
    let cb = (color.0 >> 16) & 0xff;
    for row in 0..mask.h {
        for col in 0..mask.w {
            let a = mask.a[row * mask.w + col] as u32;
            if a == 0 {
                continue;
            }
            let px = x + col as i32;
            let py = y + row as i32;
            let bg = GetPixel(hdc, px, py);
            if bg.0 == u32::MAX {
                continue;
            }
            let br = bg.0 & 0xff;
            let bgc = (bg.0 >> 8) & 0xff;
            let bb = (bg.0 >> 16) & 0xff;
            let inv = 255 - a;
            let r = (cr * a + br * inv) / 255;
            let g = (cg * a + bgc * inv) / 255;
            let b = (cb * a + bb * inv) / 255;
            let _ = SetPixelV(hdc, px, py, COLORREF(r | (g << 8) | (b << 16)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_path_yields_all_six_subpaths() {
        let polys = flatten(TRANSLATE_PATH);
        assert_eq!(polys.len(), 6, "翻译图标由 6 段闭合轮廓组成");
        for p in &polys {
            assert!(p.len() >= 3);
            for (x, y) in p {
                assert!((0.0..=24.0).contains(x) && (0.0..=24.0).contains(y), "坐标应在 24×24 视图内：{x},{y}");
            }
        }
    }

    #[test]
    fn translate_mask_has_empty_border_and_ink_in_the_middle() {
        let m = translate_mask(16);
        assert_eq!((m.w, m.h), (16, 16));
        assert_eq!(m.at(0, 0), 0, "左上角必须是透明的");
        let ink: u32 = m.a.iter().map(|v| *v as u32).sum();
        assert!(ink > 2000, "图标应有可见笔画，实际覆盖率总和 {ink}");
        assert!(ink < 16 * 16 * 255 / 2, "图标不应糊成实心方块");
    }

    #[test]
    fn copy_back_page_is_punched_out_by_the_front_page() {
        let (back, front) = copy_masks(24);
        // 前页填充区的中心（约 9.2, 14.1）应完全被前页盖住 → 后页没有墨。
        assert_eq!(back.at(9, 14), 0, "后页被前页遮挡处应为空");
        assert!(front.at(9, 14) == 0, "前页中心是空心描边");
        // 前页描边本身有墨。
        assert!(front.at(9, 7) > 0);
        // 后页右侧露出的描边应有墨（x=20 落在 19.3..21 的笔画内）。
        assert!(back.at(20, 12) > 0);
    }

    #[test]
    fn masks_scale_without_losing_shape() {
        let small = translate_mask(16);
        let large = translate_mask(32);
        let small_ink: u32 = small.a.iter().map(|v| *v as u32).sum();
        let large_ink: u32 = large.a.iter().map(|v| *v as u32).sum();
        // 面积随边长平方增长（允许 30% 误差）。
        let ratio = large_ink as f32 / small_ink.max(1) as f32;
        assert!(ratio > 3.0 && ratio < 6.0, "覆盖率比例异常：{ratio}");
    }
}
