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
    get_text_with_sequence(|| true).and_then(|(text, _)| text)
}

/// 文本与版本必须在同一次 OpenClipboard 内读取；读取延迟渲染格式后再记录版本。
/// 等待剪贴板锁期间请求可能被取消，拿到锁后必须再次检查。
/// 外层 None 表示未取得快照；内层 None 表示非文本，仍可按版本还原。
pub fn get_text_with_sequence(is_current: impl Fn() -> bool) -> Option<(Option<String>, u32)> {
    if !open_with_retry() {
        return None;
    }
    if !is_current() {
        unsafe { let _ = CloseClipboard(); }
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
    let snapshot = Some((result, sequence_number()));
    unsafe {
        let _ = CloseClipboard();
    }
    snapshot
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
    sequence: Option<u32>,
}

impl Backup {
    pub fn take() -> Self {
        let mut entries = Vec::new();
        if !open_with_retry() {
            // 打开失败时不要还原，否则会把用户剪贴板清空。
            return Self { entries, sequence: None };
        }
        let sequence;
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
            sequence = sequence_number();
            let _ = CloseClipboard();
        }
        Self { entries, sequence: Some(sequence) }
    }

    /// 备份时的版本；失败时为 None，调用方不得继续模拟复制。
    pub fn sequence(&self) -> Option<u32> {
        self.sequence
    }

    /// 仅在剪贴板仍是本次取词读到的版本且请求未取消时还原。
    /// 校验必须放在 OpenClipboard 之后，避免等锁时用户的新复制被覆盖。
    pub fn restore_if_unchanged(&self, expected: u32, is_current: impl Fn() -> bool) -> bool {
        if self.sequence.is_none() || !is_current() || !open_with_retry() {
            return false;
        }
        let mut restored = false;
        unsafe {
            if sequence_number() == expected && is_current() && EmptyClipboard().is_ok() {
                restored = true;
                for (format, bytes) in &self.entries {
                    restored &= put_hglobal(*format, bytes);
                }
            }
            let _ = CloseClipboard();
        }
        restored
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
        let (_, captured_sequence) = get_text_with_sequence(|| true).unwrap();
        assert!(backup.restore_if_unchanged(captured_sequence, || true));
        assert_eq!(get_text().as_deref(), Some("划词工具条 测试 123"));

        // 取词读完后用户又复制了：必须保留新内容，即使文本恰好相同。
        assert!(set_text("用户复制的内容"));
        let stale_sequence = sequence_number();
        assert!(set_text("用户复制的内容"));
        assert_ne!(sequence_number(), stale_sequence);
        assert!(!backup.restore_if_unchanged(stale_sequence, || true));
        assert_eq!(get_text().as_deref(), Some("用户复制的内容"));

        // 用户的 C 已被钩子放行，但源应用还没写剪贴板：版本相同也不能还原。
        let unchanged = sequence_number();
        assert!(!backup.restore_if_unchanged(unchanged, || false));
        assert_eq!(sequence_number(), unchanged);

        // 获取剪贴板锁期间取消：必须在锁内重查，不能只在 OpenClipboard 前查。
        let checks = std::cell::Cell::new(0);
        assert!(!backup.restore_if_unchanged(unchanged, || {
            checks.set(checks.get() + 1);
            checks.get() == 1
        }));
        assert_eq!(checks.get(), 2);
        assert_eq!(sequence_number(), unchanged);
        assert!(get_text_with_sequence(|| false).is_none());

        // 备份失败时不能清空剪贴板。
        let invalid = Backup { entries: Vec::new(), sequence: None };
        assert!(!invalid.restore_if_unchanged(unchanged, || true));
        assert_eq!(sequence_number(), unchanged);

        // 没有文本的快照也要带版本，才能还原图形应用复制后留下的临时内容。
        assert!(open_with_retry());
        unsafe {
            assert!(EmptyClipboard().is_ok());
            let _ = CloseClipboard();
        }
        let (text, sequence) = get_text_with_sequence(|| true).unwrap();
        assert!(text.is_none());
        assert!(backup.restore_if_unchanged(sequence, || true));
        assert_eq!(get_text().as_deref(), Some("划词工具条 测试 123"));

        original.restore_if_unchanged(sequence_number(), || true);
    }
}
