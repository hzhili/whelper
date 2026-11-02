//! 单实例守卫：多个常驻实例会各自安装全局钩子，必须互斥。

use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

use super::string::{to_wide, wide_ptr};

/// 进程级单实例互斥体。
///
/// `acquire` 返回 `None` 说明同名互斥体已存在（已有实例在跑）；守卫析构时释放。
pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    pub fn acquire(name: &str) -> Option<Self> {
        let name = to_wide(name);
        match unsafe { CreateMutexW(None, true, wide_ptr(&name)) } {
            Ok(handle) => {
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    None
                } else {
                    Some(Self { handle })
                }
            }
            Err(_) => None,
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
