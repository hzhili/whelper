# show-desktop（单屏显示桌面）

把系统 `Win+D`（显示桌面）改成**只作用于光标所在显示器**：第一次按下最小化该屏窗口，再按一次按原 Z 序还原；
期间如果有窗口打开/关闭，则按 Win+D 的语义重新隐藏。

本 crate 是**库**，不产出 exe；由 `whelper` 在同一个进程里加载。算法移植自
[ShowDesktopOneMonitor](https://github.com/ruzrobert/ShowDesktopOneMonitor)（C#/WinForms，GPL-3.0），
该 crate 声明 `GPL-3.0-only`。

## 语义

- 目标显示器 = **光标所在显示器**（`MonitorFromPoint(MONITOR_DEFAULTTONEAREST)`）。
- 候选窗口 = 与 Alt+Tab 等价的顶层窗口：排除 cloaked（其它虚拟桌面）、`WS_DISABLED`、工具窗口/非 `AppWindow` 的窗口。
  枚举与过滤实现在 `whelper-common::win32::window`，与本仓库的 Alt+Tab 切换器共用同一份。
- 第一次按下：最小化当前可见窗口，并记下清单（`MonitorId -> [WindowId, 原状态]`）。
- 第二次按下：仅当“当前没有一个窗口是可见的”且“记忆清单与当前清单只差状态”时，按**倒序**还原；否则重新最小化。
- 窗口归属显示器时，最小化窗口用**还原矩形**（`WINDOWPLACEMENT.rcNormalPosition`），避免多屏下退化成 `(-32000, -32000)` 归错屏。

## 模块

| 模块 | 职责 |
| --- | --- |
| `window` | 重新导出 `whelper-common::window`：`WindowId` / `WindowState` / `WindowKind` / 样式位 / `classify()` |
| `plan` | `decide()`：这次是最小化还是还原、执行顺序（纯逻辑） |
| `store` | 每个显示器的记忆清单 |
| `controller` | 编排：枚举 → 决策 → 执行 → 记忆；`ShowDesktop::toggle_at_cursor()` / `preview_at_cursor()` |
| `backend` | `DesktopBackend` trait（枚举/执行/显示器查询），`memory` 为测试桩 |
| `backend::win32` | 真实实现：候选枚举（来自 `whelper-common`）、`ShowWindow`、显示器归属 |
| `service` | **常驻形态**：message-only 窗口 + `MSG_TOGGLE` 处理，供宿主嵌入 |
| `shortcut` | `Win+D` 状态机与键盘钩子处理器（吞键 + 掩码） |
| `app` | 一次性命令：`report()` / `toggle_once()` / `self_test()` |

## 嵌入方式

```rust
// 宿主（whelper）在主线程、进入自己的消息循环之前启动
let service = show_desktop::service::Service::start();   // 创建 HWND_MESSAGE 窗口，注册 Win+D 处理器
// ... 宿主自己的消息循环（GetMessage/DispatchMessage）
drop(service);                                            // 摘钩子、销毁窗口
```

触发链：

```text
WH_KEYBOARD_LL（whelper-common 的共享钩子链）
  └─ show-desktop 处理器：Win 按下 + D 按下 → 吞掉 D
        ├─ 注入一次 0xFF dummy key（down+up），防止 shell 误判“只按了 Win”而弹开始菜单
        └─ PostMessage(内部 MSG_TOGGLE) ──► 服务窗口 wndproc
                                              └─ ShowDesktop::toggle_at_cursor()
```

要点：

- 钩子回调只做状态判定与 `PostMessage`，**不做窗口枚举/最小化**（避免阻塞输入链）。
- 注入的 dummy key 带 `INJECTED_FLAG`（`dwExtraInfo`），钩子链据此跳过，不会自己处理自己的事件。
- 长按 `Win+D` 的自动重复只吞不触发；`Win` 松开后可以再次触发。
- 单实例互斥体在宿主（`whelper`）里，本 crate 不再注册热键，也不创建独立进程。
- 设置页的 `[desktop] enabled` 在钩子回调里实时判断：关闭后 `Win+D` 原样放行，
  并清掉半截的按键状态；调试日志由 `[general] debug` 或 `SHOW_DESKTOP_DEBUG=1` 打开。

## 一次性命令

由 `whelper` 的 CLI 转发到 `app`：

| 命令 | 行为 |
| --- | --- |
| `--desktop-report` | 只读预览：目标显示器、候选窗口、下一次按下的动作与影响数量 |
| `--toggle-desktop` | 立即切换一次（进程退出即丢记忆，无法跨进程还原） |
| `--desktop-self-test` | 最小化 → 等动画 → 还原，验证完整往返 |

## 测试

```powershell
cargo test -p show-desktop      # 21 项 + 1 项文档测试
```

覆盖：最小化/还原判定、还原顺序、窗口增减时的重新隐藏、按显示器隔离、内存后端调用序列、
`Win+D` 状态机。Alt+Tab 等价分类的测试随实现迁移到了 `whelper-common`。
