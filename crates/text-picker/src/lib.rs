//! 划词取词器：全局钩子取词 + 悬浮工具条。
//!
//! 本 crate 只提供库能力，由仓库根的 `whelper` 可执行文件在**同一个进程**里加载：
//! - [`run_ui`]：创建工具条窗口并跑主消息循环（进程生命周期由它决定）；
//! - [`run_self_test`] / [`translate_blocking`]：命令行自检与翻译调试。

mod app;
mod capture;
mod clipboard;
mod hooks;
mod icon;
mod selftest;
mod toolbar;
mod translate;
mod uia;
mod winutil;

pub use app::run as run_ui;
pub use selftest::run as run_self_test;
pub use translate::translate_blocking;

/// 诊断用：对当前前台应用做一次取词，打印结果以及是否动过剪贴板。
///
/// UIA 命中时不会发送任何按键，`clipboard_changed` 应为 `false`；
/// 终端里用它可以在不动选区的前提下核对取词内容。
pub fn probe_capture() -> i32 {
    let before = clipboard::sequence_number();
    // 与常驻路径同一套决策：终端类窗口不自动复制。
    let allow_keyboard = !winutil::prefers_deferred_capture(winutil::foreground_window());
    let outcome = capture::capture_selection(
        std::time::Duration::ZERO,
        std::time::Duration::from_millis(600),
        allow_keyboard,
        winutil::cursor_position(),
    );
    let changed = clipboard::sequence_number() != before;
    match outcome {
        capture::Outcome::Text { text, bounds } => {
            println!("captured={text:?}");
            match bounds {
                Some(bounds) => println!(
                    "bounds=({}, {})-({}, {})",
                    bounds.left, bounds.top, bounds.right, bounds.bottom
                ),
                None => println!("bounds=<none>"),
            }
            println!("clipboard_changed={changed}");
            0
        }
        capture::Outcome::Deferred => {
            println!("deferred=true");
            0
        }
        capture::Outcome::None => {
            println!("captured=<none>");
            println!("clipboard_changed={changed}");
            1
        }
    }
}
