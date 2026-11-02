//! 剪贴板读写与"取词前备份 / 取词后还原"。
//!
//! 取词会临时占用剪贴板：取词前按格式备份，取词后恢复用户内容。
//! 这里不做 OLE `IDataObject` 快照：目标应用写剪贴板时会 `EmptyClipboard`，
//! 之前的 OLE 对象会失效。改为按格式把 HGLOBAL 数据复制到本进程内，还原时再写回，
//! 可覆盖文本、HTML、RTF、文件列表（HDROP）、自定义格式等绝大多数场景。

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::Sleep;

use crate::winutil;

const CF_UNICODETEXT: u32 = 13;
const MAX_CLIPBOARD_CHARS: usize = 4 * 1024 * 1024;
/// 不是 HGLOBAL 的格式，无法用 GlobalLock 复制，直接跳过。
const NON_HGLOBAL_FORMATS: [u32; 6] = [2, 3, 9, 14, 0x80, 0x8E];

fn open_with_retry() -> bool {
    for _ in 0..20 {
        if unsafe { OpenClipboard(winutil::null_hwnd()) }.is_ok() {
            return true;
        }
        unsafe { Sleep(10) };
    }
    false
}

/// 读取剪贴板文本（`CF_UNICODETEXT`）。
pub fn get_text() -> Option<String> {
    if !open_with_retry() {
        return None;
    }
    let result = unsafe {
        match GetClipboardData(CF_UNICODETEXT) {
            Ok(handle) => {
                let hglobal = HGLOBAL(handle.0);
                let ptr = GlobalLock(hglobal) as *const u16;
                if ptr.is_null() {
                    None
                } else {
                    let mut len = 0usize;
                    while len < MAX_CLIPBOARD_CHARS && *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                    let _ = GlobalUnlock(hglobal);
                    Some(text)
                }
            }
            Err(_) => None,
        }
    };
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// 把文本写入剪贴板（`复制` 按钮使用）。
pub fn set_text(text: &str) -> bool {
    if !open_with_retry() {
        return false;
    }
    let ok = unsafe {
        EmptyClipboard().is_ok() && put_hglobal(CF_UNICODETEXT, &utf16_bytes(text))
    };
    unsafe {
        let _ = CloseClipboard();
    }
    ok
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for u in units {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes
}

unsafe fn put_hglobal(format: u32, bytes: &[u8]) -> bool {
    let Ok(hglobal) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else {
        return false;
    };
    let ptr = GlobalLock(hglobal) as *mut u8;
    if ptr.is_null() {
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    let _ = GlobalUnlock(hglobal);
    // 成功后所有权归系统。
    SetClipboardData(format, HANDLE(hglobal.0)).is_ok()
}

pub fn sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// 取词前的剪贴板备份（仅 HGLOBAL 格式）。
pub struct Backup {
    entries: Vec<(u32, Vec<u8>)>,
    valid: bool,
}

impl Backup {
    pub fn take() -> Self {
        let mut entries = Vec::new();
        if !open_with_retry() {
            // 打开失败时不要还原，否则会把用户剪贴板清空。
            return Self { entries, valid: false };
        }
        unsafe {
            let mut format = EnumClipboardFormats(0);
            while format != 0 {
                if !NON_HGLOBAL_FORMATS.contains(&format) {
                    if let Ok(handle) = GetClipboardData(format) {
                        let hglobal = HGLOBAL(handle.0);
                        let size = GlobalSize(hglobal);
                        if size > 0 {
                            let ptr = GlobalLock(hglobal) as *const u8;
                            if !ptr.is_null() {
                                let data = std::slice::from_raw_parts(ptr, size).to_vec();
                                entries.push((format, data));
                                let _ = GlobalUnlock(hglobal);
                            }
                        }
                    }
                }
                format = EnumClipboardFormats(format);
            }
            let _ = CloseClipboard();
        }
        Self { entries, valid: true }
    }

    /// 把备份写回剪贴板；原本为空则清空。
    pub fn restore(&self) {
        if !self.valid || !open_with_retry() {
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
            for (format, bytes) in &self.entries {
                put_hglobal(*format, bytes);
            }
            let _ = CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 覆盖「写入 → 读取」与「备份 → 覆盖 → 还原」两条路径。
    /// 测试结束会把用户原来的剪贴板内容写回。
    #[test]
    fn clipboard_roundtrip_and_backup_restore() {
        let original = Backup::take();

        assert!(set_text("划词工具条 测试 123"));
        assert_eq!(get_text().as_deref(), Some("划词工具条 测试 123"));

        let backup = Backup::take();
        assert!(set_text("取词时临时写入的内容"));
        assert_eq!(get_text().as_deref(), Some("取词时临时写入的内容"));
        backup.restore();
        assert_eq!(get_text().as_deref(), Some("划词工具条 测试 123"));

        original.restore();
    }
}
