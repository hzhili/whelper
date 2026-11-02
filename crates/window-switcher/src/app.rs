//! 一次性命令：只读报告、焦点切换自检、交互式演示。
//!
//! 由 `whelper` 的 CLI 转发（`--switcher-report` / `--switcher-self-test` /
//! `--switcher-demo`），也可以单独调试本 crate。

use std::time::Duration;

use whelper_common::win32::null_hwnd;
use whelper_common::win32::window as common_window;
use whelper_common::window::WindowId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, TranslateMessage, MSG,
};

use crate::layout;
use crate::model::{Session, ViewMode};
use crate::win32 as platform;

/// `--switcher-report`：枚举候选窗口、打印两种视图的布局尺寸。
///
/// 网格视图的窗口画面由 DWM 实时缩略图合成，报告不检查预览链路。
pub fn report() -> i32 {
    let scale = platform::cursor_scale();
    let work = platform::cursor_work_area();
    println!(
        "光标显示器工作区：{}×{} @ ({}, {})，缩放 {scale:.2}x",
        work.width(),
        work.height(),
        work.left,
        work.top
    );

    let settings = whelper_config::get();
    let default_view = settings.switcher.default_view.as_str();
    println!("设置中的默认视图：{default_view}");
    println!(
        "仅显示鼠标所在屏幕：{}",
        if settings.switcher.cursor_monitor_only {
            "是"
        } else {
            "否"
        }
    );

    let windows = platform::windows(settings.switcher.cursor_monitor_only);
    println!(
        "Alt+Tab 候选窗口 {} 个（与单屏显示桌面共用同一份枚举与 Z 序）：",
        windows.len()
    );
    if windows.is_empty() {
        return 0;
    }

    for window in &windows {
        println!(
            "  {:<10} {:<10} {}",
            format!("{}", window.id),
            if window.minimized {
                "已最小化"
            } else {
                "可见"
            },
            truncate(window.display_title(), 48)
        );
    }

    let app_count = Session::new(windows.clone(), ViewMode::Thumbnails)
        .expect("窗口列表非空")
        .len();
    let strip = layout::compute(
        ViewMode::Thumbnails,
        app_count,
        1.min(app_count - 1),
        scale,
        work,
    );
    let grid = layout::compute(
        ViewMode::Grid,
        windows.len(),
        1.min(windows.len() - 1),
        scale,
        work,
    );
    println!(
        "缩略图视图：单行应用图标，可见 {}/{} 个，面板 {}×{}，图标格 {}×{}",
        strip.visible.1 - strip.visible.0,
        app_count,
        strip.panel.width(),
        strip.panel.height(),
        strip.cells[0].frame.width(),
        strip.cells[0].frame.height()
    );
    println!(
        "网格视图：{} 列 × {} 行，面板 {}×{}，卡片 {}×{}",
        grid.columns,
        grid.cells.len().div_ceil(grid.columns),
        grid.panel.width(),
        grid.panel.height(),
        grid.cells[0].frame.width(),
        grid.cells[0].frame.height()
    );
    0
}

/// `--switcher-self-test`：真实“激活另一个窗口 → 校验前台 → 切回”，验证焦点切换链路。
pub fn self_test() -> i32 {
    let windows = platform::windows(false);
    if windows.len() < 2 {
        println!("SKIP 当前只有 {} 个可切换窗口", windows.len());
        return 0;
    }

    let foreground = unsafe { GetForegroundWindow() };
    let original = WindowId(foreground.0 as isize);
    let Some(target) = windows
        .iter()
        .map(|window| window.id)
        .find(|id| common_window::hwnd(*id) != foreground)
    else {
        println!("SKIP 除前台窗口外没有其它候选");
        return 0;
    };

    println!("前台 {original}，切换目标 {target}");
    if !platform::activate(target) {
        eprintln!("FAIL 无法激活 {target}（可能被 UIPI 拒绝）");
        return 1;
    }
    std::thread::sleep(Duration::from_millis(250));
    let now = unsafe { GetForegroundWindow() };
    if now != common_window::hwnd(target) {
        eprintln!("FAIL 前台窗口没有变成目标（实际 {:#x}）", now.0 as usize);
        return 1;
    }
    println!(
        "PASS 焦点已切换到 {}",
        truncate(&common_window::title(target), 40)
    );

    if !original.is_null() && common_window::activate(original) {
        println!("PASS 焦点已切回 {original}");
    } else {
        eprintln!("FAIL 无法切回原窗口 {original}");
        return 1;
    }
    println!("SELF-TEST PASS");
    0
}

/// `--switcher-demo`：接管 Alt+Tab 并跑独立消息循环，用于交互式验证。
pub fn demo() -> i32 {
    let Some(service) = crate::service::Service::start() else {
        eprintln!("FAIL 覆盖窗口或键盘钩子创建失败");
        return 1;
    };
    println!(
        "[whelper] Alt+Tab 切换器 demo：覆盖窗口 {:#x}",
        service.hwnd().0 as usize
    );
    println!("Alt+Tab 打开；Tab / Shift+Tab 选择，` 切换视图，Esc 取消，Ctrl+C 退出。");

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_hwnd(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}

/// 按字符截断（中文标题不会在半个字符处断开）。
fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
