//! 系统深浅色主题查询。
//!
//! 多个组件的弹窗都要跟随系统主题（划词工具条、Alt+Tab 切换器），
//! 否则在深色桌面上一块白卡片会非常刺眼。

use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::win32::string::{to_wide, wide_ptr};

/// 应用是否使用浅色主题。
///
/// 读取 `HKCU\...\Themes\Personalize\AppsUseLightTheme`；读不到时按浅色处理
/// （与 Windows 默认值一致）。
pub fn apps_use_light_theme() -> bool {
    let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let value = to_wide("AppsUseLightTheme");
    let mut data: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide_ptr(&subkey),
            wide_ptr(&value),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    rc.0 == 0 && data != 0
}
