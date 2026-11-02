# whelper（可执行文件 / 组合根）

`whelper` 是整个仓库**唯一的可执行文件**：它把 `text-picker`、`show-desktop`、`window-switcher`、
`whelper-shell` 与 `whelper-config` 链接进同一个进程，提供统一的命令行、单实例保护与生命周期管理。
它本身**不实现任何业务逻辑**。

## 为什么是“组合根”

- 需求是“一个 `cargo run` 只产出一个 exe、所有组件跑在同一个进程里”，因此必须有一个 crate 承担 bin 角色；
- 组件保持纯库：`text-picker` 不依赖任何桌面组件，`show-desktop` 与 `window-switcher` 也互不依赖；
- 组件之间唯一的共性放在 `whelper-common`，配置放在 `whelper-config`，`whelper` 只负责“按顺序启动、按顺序收尾”。

```toml
# crates/whelper/Cargo.toml（节选）
[package]
name = "whelper"

[dependencies]
show-desktop = { path = "../show-desktop" }
text-picker = { path = "../text-picker" }
window-switcher = { path = "../window-switcher" }
whelper-shell = { path = "../whelper-shell" }
whelper-config = { path = "../whelper-config" }
whelper-common = { path = "../whelper-common" }
```

## 启动流程

常驻模式（无参数）的顺序是固定的：

```rust
// crates/whelper/src/main.rs
// 配置先于任何组件：设置页、钩子、绘制都会读它。
whelper_config::install(whelper_config::load());

// 一个进程一个实例；拿到 None 时按参数决定“已运行”提示或转发打开设置。
let _guard = SingleInstance::acquire("Local\\WHelper_SingleInstance");

// 先启动桌面服务：创建 message-only 窗口，把 Win+D 处理器挂到键盘钩子链上
let desktop = show_desktop::service::Service::start();

// 再启动切窗服务：创建覆盖窗口，把 Alt+Tab 处理器排在 Win+D 之后
let switcher = window_switcher::service::Service::start();

// 托盘外壳：隐藏窗口 + 托盘图标；设置页按需创建
let mut shell = whelper_shell::Shell::start();

// 最后进入工具条窗口的主消息循环；它也负责派发上面三个服务的消息
let code = text_picker::run_ui(demo);

// 先撤托盘与设置页，再销毁两个服务，最后退出
drop(shell);
drop(switcher);
drop(desktop);
```

1. **单实例**：所有模块共用同一个互斥体，避免钩子/托盘被重复安装。
2. **`--settings` 的两种路径**：拿到互斥体就正常启动并打开设置页；拿不到就通过
   `whelper-shell::open_settings_in_running_instance()` 向已有实例投递 `MSG_OPEN_SETTINGS`，然后以 0 退出。
3. **`--autostart`**：先 `hide_console()`（`ShowWindow` + `FreeConsole`）再常驻，登录时不弹黑框；
   启动日志也一并静音。
4. **退出**：工具条窗口收到 `WM_CLOSE`（托盘“退出”或工具条右键菜单）→ 销毁窗口 → 摘钩子 →
   `PostQuitMessage`；`run_ui` 返回后按 shell → switcher → desktop 的顺序 `drop`。

## 命令行

一次性命令**不参与单实例互斥**，常驻进程运行时也能用：

| 参数 | 说明 | 归属组件 |
| --- | --- | --- |
| *(无参数)* | 常驻：工具条 + 单屏显示桌面 + Alt+Tab 切换器 + 托盘 | 全部 |
| `--settings` | 打开设置页；已有实例时转发到前台 | whelper-shell |
| `--autostart` | 开机自启入口：隐藏控制台后常驻 | whelper |
| `--demo` | 只演示工具条 UI（不装钩子、不建托盘） | text-picker |
| `--self-test` | 取词 + 剪贴板还原自检 | text-picker |
| `--capture-probe` | 只读取词诊断：选区文本、选区矩形、是否动过剪贴板 | text-picker |
| `--translate <text>` | 直接调用翻译接口 | text-picker |
| `--desktop-report` | 只读预览：光标显示器的候选窗口与下一步动作 | show-desktop |
| `--toggle-desktop` | 立即切换一次（进程退出即丢记忆） | show-desktop |
| `--desktop-self-test` | 最小化→还原往返自检 | show-desktop |
| `--switcher-report` | 只读：Alt+Tab 候选窗口、预览抓取、两种视图尺寸 | window-switcher |
| `--switcher-self-test` | 激活另一个窗口→校验前台→切回 | window-switcher |
| `--switcher-demo` | 接管 Alt+Tab 并跑独立消息循环（需先退出常驻实例） | window-switcher |
| `--icon-report` | 检查嵌入的图标资源与系统图标尺寸 | whelper-shell |
| `--config-report` | 打印配置文件路径与当前生效值 | whelper-config |

调试环境变量：`TEXT_PICKER_DEBUG`、`TEXT_PICKER_THEME`、`SHOW_DESKTOP_DEBUG`、`SWITCHER_DEBUG`、
`WHELPER_HOOK_DEBUG`（含义见根 README）。

## 依赖与边界

```text
whelper ──► text-picker ──────┐
    ├────► show-desktop ──────┼──► whelper-config
    ├────► window-switcher ───┤         │
    ├────► whelper-shell ─────┘         │
    └──────────────────────────► whelper-common
```

- `whelper` 只使用各组件的**公开入口**：
  `text_picker::run_ui / run_self_test / translate_blocking / probe_capture`、
  `show_desktop::service::Service`、`show_desktop::app::{report, toggle_once, self_test}`、
  `window_switcher::service::Service`、`window_switcher::app::{report, self_test, demo}`、
  `whelper_shell::Shell::{start, open_settings}`、`whelper_shell::{icon_report, open_settings_in_running_instance}`。
- 不直接写 Win32 业务代码（只用了 `whelper-common` 的单实例与控制台隐藏）。

## 新增一个组件

1. 新建 `crates/<name>` 库 crate，只依赖 `whelper-common` / `whelper-config`；
2. 在 `crates/whelper/Cargo.toml` 加 path 依赖，根 `Cargo.toml` 的 `members` 登记；
3. 在 `main.rs` 里：常驻服务在 `run_ui` 之前启动、在其后 `drop`；一次性命令加到参数分发里。

## 测试

本 crate 没有自己的测试（它只是编排）。回归依赖：

```powershell
cargo test --workspace      # 149 项
cargo run -- --self-test
cargo run -- --desktop-self-test
cargo run -- --switcher-self-test
cargo run -- --icon-report
cargo run -- --config-report
```
