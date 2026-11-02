# whelper-common（共享底座）

跨组件复用的 Win32 基础封装与纯逻辑类型。**不含任何业务语义**——不认识“取词”“显示桌面”“切换器”“设置”，
只提供各组件都要用的底层能力。

## 模块

| 模块 | 内容 | 说明 |
| --- | --- | --- |
| `geometry` | `Point`、`Rect`、`MonitorId`、`MonitorInfo` | 纯逻辑，跨平台可单测 |
| `window` | `WindowId`、`WindowState`、`WindowKind`、`classify()`、样式位 | Alt+Tab 等价分类的纯判定，跨平台可单测 |
| `win32::string` | `to_wide(&str) -> Vec<u16>`、`wide_ptr(&[u16]) -> PCWSTR` | 宽字符串，避免每处重复写 |
| `win32::monitor` | `cursor_monitor()`、`monitor_of_point()`、`monitor_of_window()`、`work_area()`、`info()` | 显示器归属与工作区查询 |
| `win32::instance` | `SingleInstance::acquire(name) -> Option<Self>` | `CreateMutexW` + `ERROR_ALREADY_EXISTS`；`Drop` 关闭句柄 |
| `win32::hook` | `WH_KEYBOARD_LL` 的安装、解码与**多处理器链** | 三个交互组件共用一条系统钩子 |
| `win32::keyboard` | `modifier_down()`、`send_dummy_key()` | 修饰键查询 + 吞热键后的掩码键注入 |
| `win32::window` | `alt_tab_windows()`、`title()`、`state()`、`is_minimized()`、`activate()` | 候选窗口枚举（两个桌面组件共用）与焦点切换 |
| `win32::theme` | `apps_use_light_theme()` | 系统深浅色 |
| `win32::icon` | `from_ico(bytes, size)`、`sizes(bytes)`、`destroy()` | 从嵌入的 ICO 建 `HICON`（托盘 / 任务栏） |
| `win32::applink` | `SHELL_CLASS`、`MSG_OPEN_SETTINGS`、`open_settings()`、`close_toolbar()` | 进程内窗口类名 / 消息约定 |
| `win32::console` | `hide_console()` | 自启时隐藏并脱离控制台 |
| `win32::dpi` | `enable_per_monitor_v2()` | 进程级 DPI 感知（创建窗口之前） |
| `win32` | `null_hwnd()` | 常用空句柄常量 |

平台依赖只在 Windows 编译：

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [...] }
```

## 键盘钩子：使用契约

```rust
use whelper_common::win32::hook::{self, KeyEvent};

fn on_key(event: &KeyEvent) -> bool {
    // 返回 true 表示吞掉这次按键：系统与前台应用都收不到。
    false
}

// 必须在“跑消息泵的线程”上调用：低级钩子的回调由该线程的消息循环驱动。
let installed = hook::add_handler(on_key);
// ... 退出前
hook::remove_handler(on_key);
```

关键规则：

1. **首次注册时装钩子，最后一个注销时卸载**；没有处理器就不会有钩子。
2. **必须在消息泵线程注册**；钩子回调也在该线程执行，回调里不要做重活（枚举窗口、磁盘 IO）。
3. **返回 `true` 即吞键**，链上后面的处理器不再被调用；`text-picker` 因此永远返回 `false`，`show-desktop` 只吞 `Win+D`，`window-switcher` 只吞 `Alt+Tab` 及其导航键。
4. **处理器不在锁内调用**：处理器可以注入按键（例如吞掉 `Win+D` / `Alt+Tab` 后补一个掩码键），注入事件会重入本钩子；持锁调用会自锁。
5. 处理器内部**不要**再调用 `add_handler` / `remove_handler`。
6. 处理器上限 8 个（固定容量，避免在钩子回调里分配内存）。

调试图标：`WHELPER_HOOK_DEBUG=1` 会打印每个按键事件与当前处理器数量。

## 注入标记

```rust
use whelper_common::win32::hook::{self, INJECTED_FLAG};

// 注入时把自己产生的事件标记出来（写进 dwExtraInfo）
let marker = INJECTED_FLAG;

// 处理器里跳过自己注入的事件
if hook::generated_by_us(event) {
    return false;
}
```

`generated_by_us` 判断的是 `dwExtraInfo & INJECTED_FLAG`，比 `LLKHF_INJECTED` 精确：

- `LLKHF_INJECTED` 只能说明“是注入的”，分不清是本程序还是别的自动化工具；
- 自己的事件必须跳过，否则“注入 → 重入 → 再处理”会形成回环。

## 单实例

```rust
use whelper_common::win32::instance::SingleInstance;

let _guard = SingleInstance::acquire("Local\\WHelper_SingleInstance");
if _guard.is_none() {
    eprintln!("已在运行");
}
// `_guard` 离开作用域时关闭互斥体句柄
```

## 候选窗口与焦点切换

```rust
use whelper_common::win32::window;

// 与 Alt+Tab 等价的顶层窗口，按 Z 序自上而下（索引 0 = 当前前台窗口）。
let windows = window::alt_tab_windows();
for id in windows {
    println!("{} {}", id, window::title(id));
}

// 切到前台：最小化先还原、绕过前台锁、切换是异步的（内部会轮询确认）。
let activated: bool = window::activate(id);
```

`show-desktop` 与 `window-switcher` 共用 `alt_tab_windows()` 与纯判定 `window::classify()`：
cloaked / `WS_DISABLED` / `WS_EX_NOACTIVATE` / `WS_EX_TOOLWINDOW` / `IsAltTabWindow` 的过滤只写一份，
两个功能看到的候选集合与顺序不会漂移。

## 嵌入图标

```rust
use whelper_common::win32::icon;

const ICO: &[u8] = include_bytes!("../assets/whelper.ico");

if let Some(hicon) = icon::from_ico(ICO, 32) {
    // 设为窗口图标 / 托盘图标
    // ...
    icon::destroy(hicon);
}
```

`from_ico` 解析 ICO 目录、选最接近目标尺寸的一项（同样接近时取更大的），再用
`CreateIconFromResourceEx` 建 `HICON`；支持 PNG 压缩的图标项（Vista+）。

## 进程内窗口约定

```rust
use whelper_common::win32::applink;

// 托盘外壳是隐藏的顶层窗口，类名 WHelperShell
applink::open_settings();      // 给外壳发 MSG_OPEN_SETTINGS
applink::close_toolbar();      // 让 text-picker 走正常退出
```

组件之间不互相依赖，跨组件的“打开设置 / 退出”通过类名 + 消息约定完成；`--settings` 在已有实例时
就是靠它把设置页拉到前台。

## 测试

```powershell
cargo test -p whelper-common      # 18 项
```

覆盖：矩形/点几何、Alt+Tab 等价分类（cloaked/禁用/不可见/NOACTIVATE/APPWINDOW/TOOLWINDOW/启发式）、
窗口状态判定、钩子事件解码（按下/抬起、注入标记、忽略其它消息）、注入标记识别、
ICO 目录解析（尺寸选择、损坏目录、越界数据、尺寸列表）。
