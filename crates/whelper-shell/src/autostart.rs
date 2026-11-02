//! 开机自启：当前用户的 `Run` 启动项（不需要管理员权限）。
//!
//! 值名固定为 `whelper`，数据是带引号的 exe 路径 + `--autostart`。
//! 读取时要求与当前 exe 完全一致：程序被挪走后，设置页会显示为“未开启”，
//! 重新打开会覆盖成新路径，避免留下指向旧位置的僵尸启动项。

use std::path::Path;

use whelper_common::win32::string::{to_wide, wide_ptr};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS,
    REG_SZ, REG_VALUE_TYPE,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "whelper";

/// 自启命令行：路径加引号（允许空格），并带上无控制台的 `--autostart` 模式。
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

fn expected_command() -> String {
    std::env::current_exe()
        .map(|exe| command_line(&exe))
        .unwrap_or_default()
}

/// 启动项是否存在且指向当前 exe。
pub fn is_enabled() -> bool {
    let expected = expected_command();
    !expected.is_empty() && read_value().is_some_and(|value| value.trim() == expected)
}

/// 写入 / 删除启动项，返回是否成功。
pub fn set_enabled(enabled: bool) -> bool {
    if enabled {
        write_value(&expected_command())
    } else {
        delete_value()
    }
}

fn open_run_key(access: REG_SAM_FLAGS) -> Option<HKEY> {
    let subkey = to_wide(RUN_KEY);
    let mut key = HKEY::default();
    let code = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide_ptr(&subkey), 0, access, &mut key) };
    (code == ERROR_SUCCESS).then_some(key)
}

fn read_value() -> Option<String> {
    let key = open_run_key(KEY_READ)?;
    let name = to_wide(VALUE_NAME);

    let mut size = 0u32;
    let code = unsafe { RegQueryValueExW(key, wide_ptr(&name), None, None, None, Some(&mut size)) };
    if code != ERROR_SUCCESS || size == 0 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }

    let mut buffer = vec![0u8; size as usize + 2];
    let mut kind = REG_VALUE_TYPE::default();
    let mut written = buffer.len() as u32;
    let code = unsafe {
        RegQueryValueExW(
            key,
            wide_ptr(&name),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut written),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if code != ERROR_SUCCESS || kind != REG_SZ {
        return None;
    }

    let mut units: Vec<u16> = Vec::with_capacity(written as usize / 2);
    let mut index = 0;
    while index + 1 < written as usize {
        units.push(u16::from_le_bytes([buffer[index], buffer[index + 1]]));
        index += 2;
    }
    let end = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

fn write_value(command: &str) -> bool {
    let subkey = to_wide(RUN_KEY);
    let mut key = HKEY::default();
    let code = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide_ptr(&subkey),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if code != ERROR_SUCCESS {
        return false;
    }

    let name = to_wide(VALUE_NAME);
    let mut data: Vec<u16> = command.encode_utf16().collect();
    data.push(0);
    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<u16>())
    };
    let code = unsafe { RegSetValueExW(key, wide_ptr(&name), 0, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    code == ERROR_SUCCESS
}

fn delete_value() -> bool {
    let Some(key) = open_run_key(KEY_SET_VALUE) else {
        // 键都不存在，等于已经是关闭状态。
        return true;
    };
    let name = to_wide(VALUE_NAME);
    let code: WIN32_ERROR = unsafe { RegDeleteValueW(key, wide_ptr(&name)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    code == ERROR_SUCCESS || code == ERROR_FILE_NOT_FOUND
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_the_path() {
        let command = command_line(Path::new("C:\\Program Files\\whelper\\whelper.exe"));
        assert_eq!(command, "\"C:\\Program Files\\whelper\\whelper.exe\" --autostart");
    }

    #[test]
    fn command_keeps_unicode_paths() {
        let command = command_line(Path::new("D:\\工具\\whelper.exe"));
        assert!(command.starts_with("\"D:\\工具\\whelper.exe\""));
        assert!(command.ends_with("--autostart"));
    }
}
