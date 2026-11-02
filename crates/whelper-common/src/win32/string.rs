//! UTF-16 转换：`windows` crate 的宽字符 API 只接受 `PCWSTR`。

use windows::core::PCWSTR;

/// 转成以 NUL 结尾的 UTF-16 序列，供 `PCWSTR` 使用。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_ptr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}
