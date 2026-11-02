//! 一次性命令：只读预览、单次切换、往返自检。
//!
//! 常驻形态在 [`crate::service`]：同一进程里创建消息窗口，由宿主的消息循环驱动。

use crate::backend::win32::Win32Backend;
use crate::{ShowDesktop, ToggleAction, WindowState};

/// `--desktop-report`：只读预览，不改变任何窗口状态。
pub fn report() -> i32 {
    let controller = ShowDesktop::new(Win32Backend::new());
    let preview = match controller.preview_at_cursor() {
        Ok(preview) => preview,
        Err(error) => {
            eprintln!("无法预览: {error}");
            return 1;
        }
    };

    match preview.monitor_info {
        Some(info) => println!(
            "显示器 {}：{}×{} @ ({}, {}){}",
            preview.monitor,
            info.bounds.width(),
            info.bounds.height(),
            info.bounds.left,
            info.bounds.top,
            if info.primary { "（主屏）" } else { "" }
        ),
        None => println!("显示器 {}", preview.monitor),
    }
    println!(
        "候选窗口 {} 个（Alt+Tab 等价枚举，按 Z 序自上而下）：",
        preview.windows.len()
    );
    for window in &preview.windows {
        println!(
            "  {:<10} {} {}",
            state_label(window.state),
            window.id,
            window.title
        );
    }
    println!(
        "下一次按下将{} {} 个窗口",
        action_label(preview.action),
        preview.affected
    );
    0
}

/// `--toggle-desktop`：立即切换一次并打印结果（进程退出即丢记忆）。
pub fn toggle_once() -> i32 {
    let mut controller = ShowDesktop::new(Win32Backend::new());
    match controller.toggle_at_cursor() {
        Ok(report) => {
            println!(
                "{} {} 个窗口（候选 {} 个）",
                action_label(report.action),
                report.affected,
                report.considered
            );
            0
        }
        Err(error) => {
            eprintln!("显示桌面失败: {error}");
            1
        }
    }
}

/// `--desktop-self-test`：最小化 → 等待动画完成 → 再按一次还原。
///
/// 必须在同一个进程里连按两次，才能命中「记忆清单」的还原分支；结束后桌面回到原状。
pub fn self_test() -> i32 {
    let mut controller = ShowDesktop::new(Win32Backend::new());

    let first = match controller.toggle_at_cursor() {
        Ok(report) => report,
        Err(error) => {
            eprintln!("FAIL 第一次切换失败: {error}");
            return 1;
        }
    };
    if first.affected == 0 {
        println!("SKIP 当前显示器没有可最小化的窗口");
        return 0;
    }
    println!("PASS 最小化 {} 个窗口", first.affected);

    // 等最小化动画结束，避免还原请求被窗口管理器合并。
    std::thread::sleep(std::time::Duration::from_millis(700));

    let second = match controller.toggle_at_cursor() {
        Ok(report) => report,
        Err(error) => {
            eprintln!("FAIL 第二次切换失败: {error}");
            return 1;
        }
    };
    if second.action != ToggleAction::Restore {
        eprintln!("FAIL 第二次按下没有走还原分支（期间可能有窗口被打开/关闭）");
        return 1;
    }
    if second.affected != first.affected {
        eprintln!(
            "FAIL 还原数量不一致：期望 {}，实际 {}",
            first.affected, second.affected
        );
        return 1;
    }
    println!("PASS 还原 {} 个窗口", second.affected);
    println!("SELF-TEST PASS");
    0
}

pub(crate) fn action_label(action: ToggleAction) -> &'static str {
    match action {
        ToggleAction::Minimize => "最小化",
        ToggleAction::Restore => "还原",
    }
}

fn state_label(state: WindowState) -> &'static str {
    match state {
        WindowState::Visible => "可见",
        WindowState::Minimized => "已最小化",
        WindowState::Disabled => "隐藏",
    }
}
