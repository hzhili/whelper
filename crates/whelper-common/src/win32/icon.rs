//! 从嵌入的 `.ico` 字节创建 `HICON`。
//!
//! 图标资源（`crates/whelper-shell/assets/whelper.ico`）用 `include_bytes!` 编进程序，
//! 不依赖安装目录里是否存在文件；这里只做 ICO 目录解析 + `CreateIconFromResourceEx`，
//! 避免为了一个托盘图标往磁盘写临时文件。

use windows::Win32::Foundation::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR,
};

/// ICO 目录里的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    width: i32,
    offset: usize,
    length: usize,
}

/// 从多尺寸 ICO 里挑接近 `size` 的一项，创建对应的 `HICON`。
pub fn from_ico(bytes: &[u8], size: i32) -> Option<HICON> {
    let entry = pick_entry(bytes, size)?;
    let data = bytes.get(entry.offset..entry.offset + entry.length)?;
    // dwVer = 0x00030000（Vista+，支持 PNG 压缩的图标项）。
    unsafe {
        CreateIconFromResourceEx(
            data,
            BOOL(1),
            0x0003_0000,
            entry.width,
            entry.width,
            LR_DEFAULTCOLOR,
        )
        .ok()
    }
}

/// 选最接近目标尺寸的一项；同样接近时优先更大的（细节更多）。
fn pick_entry(bytes: &[u8], size: i32) -> Option<Entry> {
    if bytes.len() < 6 {
        return None;
    }
    if u16::from_le_bytes([bytes[2], bytes[3]]) != 1 {
        return None;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut best: Option<(i32, Entry)> = None;

    for index in 0..count {
        let base = 6 + index * 16;
        let Some(raw) = bytes.get(base..base + 16) else {
            break;
        };
        // 256 及以上的宽度在目录里写作 0。
        let width = if raw[0] == 0 { 256 } else { raw[0] as i32 };
        let length = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
        let offset = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as usize;
        let in_bounds = length > 0
            && offset
                .checked_add(length)
                .is_some_and(|end| end <= bytes.len());
        if !in_bounds {
            continue;
        }

        let entry = Entry { width, offset, length };
        let distance = (width - size).abs();
        let better = match best {
            None => true,
            Some((best_distance, best_entry)) => {
                distance < best_distance
                    || (distance == best_distance && entry.width > best_entry.width)
            }
        };
        if better {
            best = Some((distance, entry));
        }
    }

    best.map(|(_, entry)| entry)
}

/// 列出 ICO 里包含的图片尺寸（诊断用）。
pub fn sizes(bytes: &[u8]) -> Vec<i32> {
    if bytes.len() < 6 || u16::from_le_bytes([bytes[2], bytes[3]]) != 1 {
        return Vec::new();
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    (0..count)
        .filter_map(|index| bytes.get(6 + index * 16).map(|width| if *width == 0 { 256 } else { *width as i32 }))
        .collect()
}

/// 销毁 [`from_ico`] 创建的图标。
pub fn destroy(icon: HICON) {
    if !icon.0.is_null() {
        unsafe {
            let _ = DestroyIcon(icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个只有目录、没有真实图像数据的 ICO（只验证选项目标，不建图标）。
    fn ico(entries: &[(u8, usize)]) -> Vec<u8> {
        let mut bytes = vec![0u8, 0, 1, 0];
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let offset = 6 + entries.len() * 16;
        let mut data_start = offset;
        for (width, length) in entries {
            bytes.extend_from_slice(&[
                *width,
                *width,
                0,
                0,
                1,
                0,
                32,
                0,
                *length as u8,
                0,
                0,
                0,
                data_start as u8,
                0,
                0,
                0,
            ]);
            data_start += length;
        }
        bytes.resize(data_start, 0);
        bytes
    }

    #[test]
    fn picks_the_closest_size() {
        let bytes = ico(&[(16, 4), (32, 4), (48, 4)]);
        assert_eq!(pick_entry(&bytes, 16).unwrap().width, 16);
        assert_eq!(pick_entry(&bytes, 24).unwrap().width, 32);
        assert_eq!(pick_entry(&bytes, 40).unwrap().width, 48);
        assert_eq!(pick_entry(&bytes, 300).unwrap().width, 48);
    }

    #[test]
    fn rejects_broken_directories() {
        assert!(pick_entry(&[], 16).is_none());
        assert!(pick_entry(&[0, 0, 2, 0, 0, 0], 16).is_none());
        // 声明了两项，但目录被截断。
        let mut truncated = vec![0u8, 0, 1, 0, 2, 0];
        truncated.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0]);
        assert!(pick_entry(&truncated, 16).is_none());
    }

    #[test]
    fn skips_out_of_bounds_data() {
        let mut bytes = ico(&[(16, 4)]);
        // 把长度改得比文件还长。
        bytes[14] = 0xFF;
        assert!(pick_entry(&bytes, 16).is_none());
    }

    #[test]
    fn lists_sizes() {
        let bytes = ico(&[(16, 4), (32, 4), (0, 4)]);
        assert_eq!(sizes(&bytes), vec![16, 32, 256]);
        assert!(sizes(&[0, 0, 2, 0, 0, 0]).is_empty());
    }
}
