# whelper

<img src="crates/whelper-shell/assets/logo.png" width="88" alt="whelper logo" align="right">

Windows 桌面助手：**一个进程、一个可执行文件**，同时提供三件事，外加一个托盘入口与设置页——

- **划词工具条**：在任意应用里选中文字，光标附近弹出「复制 / 翻译」条。
- **单屏显示桌面**：接管系统 `Win+D`，只最小化/还原**光标所在显示器**的窗口，再按一次按原 Z 序还原。[ShowDesktopOneMonitor](https://github.com/ruzrobert/ShowDesktopOneMonitor) 的 Rust 移植。
- **Alt+Tab 切换器**：接管系统 `Alt+Tab`，用**缩略图 / 网格**两种视图列出应用窗口，带实时预览、布局过渡动画与焦点切换。交互模型参考 [sigoden/window-switcher](https://github.com/sigoden/window-switcher)。
- **托盘与设置**：常驻托盘图标（左键打开设置、右键菜单），设置页集中管理全部开关，支持**开机自启**；改动即时写回 `%APPDATA%\whelper\config.ini`。

`cargo run` 只产出 `whelper.exe`；三个组件、托盘外壳与配置模块都以库的形式链接进**同一个进程**，
共享主线程消息循环与一条键盘钩子链。

## 特性

- **触发**：全局低级鼠标/键盘钩子识别拖拽划词（≥4px）、双击/三击选词、`Ctrl+A`、`Shift+方向键`。
- **取词**：UI Automation `TextPattern.GetSelection()` 只读选区（不动键盘、不动剪贴板），并用 `GetBoundingRectangles()` 把工具条放在**选区上方居中**；UIA 不可用时回退 `Ctrl+C` + 剪贴板备份/还原。
- **终端不丢选区**：终端的复制语义是“复制并取消选区”，因此对 conhost / Windows Terminal / IDE 宿主改为**先弹条、点按钮再取词**。
- **过滤**：空文本、纯空白/不可见字符、超长文本不弹；密码框、焦点与命中元素跨进程（IME 候选窗）不弹。
- **工具条**：`WS_EX_NOACTIVATE` 不抢焦点，DPI 自适应，空间不足自动翻到下方并夹紧工作区；跟随系统深浅色。
- **单屏显示桌面**：`Win+D` 只影响光标显示器；期间窗口增减则按 Win+D 语义重新隐藏。
- **Alt+Tab 切换器**：`Alt+Tab` 循环选择、松开 Alt 提交、`Esc` 取消；`` ` `` 在缩略图/网格间切换，方向键/滚轮/鼠标均可操作。
- **窗口预览**：`DwmRegisterThumbnail` 实时缩略图（含最小化窗口），随卡片移动、随淡入淡出同步；注册失败（受保护内容）降级为图标 + “无预览”。
- **覆盖层**：背景完全透明的“卡片浮空”布局（观感参考 windows-helper）——带 1px 描边与实色底的 8px 圆角卡片直接浮在桌面上，Segoe UI，标题行为“图标 + 标题”，选中态为整张卡片的强调色底并辅以系统强调色边框；打开时 96%→100% 轻微放大 + 淡入，切换视图/移动选择时平滑插值。
- **托盘**：`Shell_NotifyIconW` + 自绘菜单，左键打开设置、右键「打开设置 / 开机自启 / 退出」；explorer 未就绪时自动重试。
- **设置页**：左侧分类（常规 / 划词 / 显示桌面 / 切换器 / 关于）+ 右侧自绘开关与分段控件，改动即时保存，深浅色跟随主题。
- **开机自启**：写入当前用户 `HKCU\...\Run`，值指向 `"<exe>" --autostart`；`--autostart` 会脱离控制台，登录时不弹黑框。
- **图标资源**：多尺寸 `.ico` / `logo.png` 由 `tools/make-icon.ps1` 生成并 `include_bytes!` 编进程序，托盘、任务栏与关于页共用。

## 架构

```text
                ┌──────────────────────────────────────────────────┐
                │                   whelper.exe                    │
                │             （唯一 bin，组合根 crate）            │
                │                                                  │
                │  主线程                                          │
                │   ├─ text-picker      工具条窗口 + 主消息循环     │
                │   ├─ show-desktop     消息窗口（收 MSG_TOGGLE）   │
                │   ├─ window-switcher  覆盖窗口（收 Alt+Tab 消息） │
                │   └─ whelper-shell    隐藏外壳 + 托盘 + 设置窗口  │
                │  共享：环境块 / 控制台 / 一条键盘钩子链            │
                └─────────────────────────┬────────────────────────┘
                                          │ 只依赖
                ┌─────────────────────────▼────────────────────────┐
                │                   whelper-config                 │
                │     全部开关与参数（INI 持久化、进程内共享）      │
                └─────────────────────────┬────────────────────────┘
                                          │
                ┌─────────────────────────▼────────────────────────┐
                │                   whelper-common                 │
                │  geometry / window / string / monitor / theme    │
                │  instance / keyboard / icon / applink / console  │
                │  hook（多处理器链）                               │
                └──────────────────────────────────────────────────┘
```

| crate | 类型 | 依赖 | 职责 | 文档 |
| --- | --- | --- | --- | --- |
| `crates/whelper` | bin | text-picker、show-desktop、window-switcher、whelper-shell、whelper-config、whelper-common | 唯一可执行文件：CLI、单实例、组件编排与生命周期 | [README](crates/whelper/README.md) |
| `crates/text-picker` | lib | whelper-common、whelper-config | 划词取词 + 工具条（不含桌面逻辑） | [README](crates/text-picker/README.md) |
| `crates/show-desktop` | lib | whelper-common、whelper-config | 单屏显示桌面：窗口枚举/决策/记忆 + 可嵌入服务 | [README](crates/show-desktop/README.md) |
| `crates/window-switcher` | lib | whelper-common、whelper-config | Alt+Tab 切换器：两种视图、窗口预览、焦点切换 | [README](crates/window-switcher/README.md) |
| `crates/whelper-shell` | lib | whelper-common、whelper-config | 托盘图标与菜单、设置页、开机自启 | [README](crates/whelper-shell/README.md) |
| `crates/whelper-config` | lib | —（仅 std） | 全部配置项、INI 解析/生成、进程内共享与持久化 | [README](crates/whelper-config/README.md) |
| `crates/whelper-common` | lib | windows | 跨组件复用的 Win32 基础封装（无业务语义） | [README](crates/whelper-common/README.md) |

依赖方向是单向的：`whelper → 组件 → whelper-config / whelper-common`，组件之间**互不依赖**。
`show-desktop` 与 `window-switcher` 的候选窗口枚举共用 `whelper-common` 的同一份实现；
所有组件读同一份配置，不存在各自散落的开关。

## 快速开始

要求 Windows 10+ 交互式桌面、Rust stable（edition 2021）。

```powershell
cd whelper
cargo run --release        # 仓库根；产物 target/release/whelper.exe
```

`cargo run` 的默认成员是 `whelper`；只构建库用 `cargo build --workspace`。

常驻后在记事本/浏览器/Word 里拖选文字即可看到工具条；按 `Win+D` 只隐藏当前显示器；
按 `Alt+Tab` 打开窗口切换器（按住 Alt 连按 Tab 循环，松开 Alt 切换，`` ` `` 换视图，`Esc` 取消）；
点托盘图标打开设置页。

## 设置与配置

托盘左键（或 `whelper --settings`）打开设置页：

| 分类 | 可配置项 |
| --- | --- |
| 常规 | 开机自启、界面主题（跟随系统 / 浅色 / 深色）、调试日志、配置文件位置 |
| 划词工具条 | 总开关、双击/三击选词、拖拽触发距离（2/4/8 px） |
| 显示桌面 | 是否接管 `Win+D` |
| 窗口切换器 | 是否接管 `Alt+Tab`、默认视图、仅显示鼠标所在屏幕窗口、窗口预览、过渡动画 |
| 关于 | 版本、配置路径、打开配置目录、恢复默认设置 |

- 配置文件：`%APPDATA%\whelper\config.ini`，带注释、可手工编辑（退出 whelper 后改，重启生效）。
- 设置页改动**立即写盘并即时生效**：主题立刻重绘工具条 / 切换器 / 设置页，开关直接改变钩子行为。
- 开机自启不写配置文件，而是 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的 `whelper` 值；
  设置页与托盘菜单都读取注册表的真实状态，程序被挪走后会显示为“未开启”。
- 环境变量是调试覆盖（优先级高于配置），见下文。

## 命令行

```text
whelper [--settings | --autostart | --demo | --self-test | --translate <text>]
        [--desktop-report | --toggle-desktop | --desktop-self-test]
        [--switcher-report | --switcher-self-test | --switcher-demo]
        [--icon-report | --config-report]

  (无参数)             常驻：划词工具条 + 单屏显示桌面 + Alt+Tab 切换器 + 托盘
  --settings           打开设置页；已在运行时把现有实例的设置页拉到前台
  --autostart          开机自启入口：隐藏并脱离控制台后常驻（注册表启动项使用）
  --demo               只演示工具条 UI（不装钩子、不建托盘）
  --self-test          取词 + 剪贴板还原自检
  --capture-probe      只读取词诊断：打印当前选区、选区矩形与是否动过剪贴板
  --translate <text>   直接调用翻译接口并打印结果（调试用）
  --desktop-report     只读预览：光标显示器的候选窗口与下一步动作
  --toggle-desktop     立即执行一次单屏显示桌面
  --desktop-self-test  单屏显示桌面：最小化→还原往返自检
  --switcher-report    只读预览：Alt+Tab 候选窗口、预览抓取与两种视图尺寸
  --switcher-self-test Alt+Tab 切换器：激活另一个窗口→校验前台→切回
  --switcher-demo      接管 Alt+Tab 并跑独立消息循环（先退出常驻实例）
  --icon-report        检查嵌入的图标资源与系统图标尺寸
  --config-report      打印当前配置（文件路径与生效值）
```

一次性命令不参与单实例互斥，常驻进程运行时也可执行（`--switcher-demo` 会抢 Alt+Tab，需先退出常驻实例）。
`--settings` 在已有实例时会通过进程内窗口消息转发，不会启动第二个进程。

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `TEXT_PICKER_DEBUG=1` | 打印工具条 show/hide 与锚点 |
| `TEXT_PICKER_THEME=light\|dark` | 强制工具条主题（调试用，覆盖设置页） |
| `SHOW_DESKTOP_DEBUG=1` | 打印每次切换的窗口数量 |
| `SWITCHER_DEBUG=1` | 打印切窗会话的打开/选择/视图切换/提交结果 |
| `WHELPER_HOOK_DEBUG=1` | 打印键盘钩子收到的每个事件与处理器数量（底层，设置页的“调试日志”不包含它） |

## 验证

```powershell
cargo test --workspace      # 149 项：common 18 + config 17 + show-desktop 21(+1 doc) + text-picker 37 + window-switcher 43(+1 doc) + shell 11
cargo run -- --self-test
cargo run -- --capture-probe
cargo run -- --desktop-report
cargo run -- --switcher-report
cargo run -- --switcher-self-test
cargo run -- --icon-report
cargo run -- --config-report
```

图标与配置自检的真实输出：

```text
whelper.ico：25927 字节，尺寸 ["16px", "20px", "24px", "32px", "40px", "48px", "64px", "128px", "256px"]
系统图标尺寸：大 32 / 小 16
加载结果：大 OK / 小 OK
```

```text
配置文件：C:\Users\<你>\AppData\Roaming\whelper\config.ini（已存在）
主题：dark　调试日志：false
划词工具条：关闭（双击选词 开启，拖拽阈值 4px）
单屏显示桌面：开启
Alt+Tab 切换器：开启（默认视图 thumbnails，窗口预览 开启，过渡动画 开启）
```

终端里选中 `Microsoft` 后 `--capture-probe` 的输出（UIA 路径，无副作用）：

```text
captured="Microsoft"
bounds=(86, 127)-(167, 146)
clipboard_changed=false
```

终端内没有选区时返回 `deferred=true`：工具条照常出现，但不复制、不动选区。

## 设计要点

1. **单进程而不是多进程**：`cargo run` 只有一个 exe，所有组件共享消息循环，省去 IPC 与窗口句柄同步；代价是崩溃域共享。
2. **一条键盘钩子链**：`whelper-common::win32::hook` 维护处理器列表，`show-desktop` 先注册并吞掉 `Win+D`，`window-switcher` 排在其后只吞 `Alt+Tab` 及其导航键，`text-picker` 最后注册、只判定从不吞键；任一处理器返回 `true` 即吞键。
3. **UIA 优先**：复制在终端里会取消选区，UIA 只读且能给出选区矩形，正好同时解决“取词”和“定位”。
4. **终端/IDE 延迟取词**：这些窗口不自动复制，工具条先出现、点按钮才取词；代价是 IDE 里拖动标签也会弹条（无法从窗口句柄区分终端面板与编辑器）。
5. **掩码键**：吞掉 `Win+D` / `Alt+Tab` 后，系统只看到修饰键起落，会误判成“单独按 Win”或激活菜单栏；因此按 PowerToys 的做法注入一次未文档化的 `0xFF` dummy key（down+up），并在 `dwExtraInfo` 写注入标记防止自己处理自己的事件。实现在 `whelper-common::win32::keyboard`，两个热键共用。
6. **候选窗口只有一份**：`show-desktop` 与 `window-switcher` 都调用 `whelper-common::win32::window::alt_tab_windows()`，cloaked/工具窗口/`IsAltTabWindow` 的过滤与 Z 序不会出现两套。
7. **预览用 DWM 实时缩略图**：`DwmRegisterThumbnail` 由合成器直接把源窗口画面画进卡片，零抓屏开销、内容实时更新；每帧同步目标矩形（跟随布局插值）与 `opacity`（跟随淡入淡出），滚出/关闭即注销。缩略图画在绘制面之上、只能直角，所以选中环画在卡片外缘、标题行放在缩略图下方。
8. **配置单一来源**：`whelper-config` 定义全部开关并负责 INI 解析/生成；组件每次打开弹窗或收到按键时读一份快照，设置页写盘后递增 `revision`，工具条定时器据此立即换主题。
9. **托盘 / 设置与弹窗同一套绘制**：设置页不堆标准控件，而是和工具条、切换器共用 GDI 双缓冲、圆角、DPI 与深浅色；绘制与点击共用同一份 `plan`，看到的位置就是点得到的位置。
10. **图标是编进程序的资源**：`tools/make-icon.ps1` 生成 9 个尺寸的 ICO，`include_bytes!` + `CreateIconFromResourceEx` 直接建 `HICON`，不需要 `.rc` / `rc.exe`，也不往磁盘写临时文件。
11. **边界**：`text-picker` 不引入 `show-desktop` / `window-switcher`，两个桌面组件之间也互不依赖；托盘与设置只通过 `whelper-common::win32::applink` 的类名/消息约定与组件往来；共性只放 `whelper-common`。
12. **切换器视觉取“卡片浮空”**（windows-helper 同款）：覆盖窗口用 `UpdateLayeredWindow` + 32bpp DIB 自绘——
    背景完全透明（面板矩形只用于布局），带 1px 描边与实色底的 8px 圆角卡片按圆角回填不透明 alpha，
    直接浮在桌面上；窗口画面由 `DwmRegisterThumbnail` 实时合成；
    字体 Segoe UI，选中环取系统强调色（`DwmGetColorizationColor`），入场为轻微放大 + 淡入。
    图标在后台线程取（`WM_GETICON` 会阻塞），整批投递只重绘一次。

## 已知限制

- 仅支持 Windows 10+ 的交互式桌面（服务/会话 0 无效）。
- **UIPI**：以普通权限运行时，既读不到、也注入不进管理员权限的窗口，也无法把管理员权限窗口切到前台；需要提权运行或签名的 `UIAccess`。
- 不暴露 UIA 的应用回退 `Ctrl+C`；终端类窗口不回退，改为延迟取词。
- `Win+D` 的“记忆清单”只存在常驻进程内，退出即丢；单屏显示桌面不可用时（钩子安装失败）退回系统原生 `Win+D`。
- 切换器的预览是 DWM 直角矩形，不能参与卡片圆角；受保护内容注册失败时显示“无预览”占位卡。
- 切换器的候选顺序取 Z 序而非系统内部 MRU；独占全屏应用之上覆盖层不可见。
- 切换器背景完全透明、无色罩（不是亚克力）：浅色壁纸上浅色卡片的对比度靠 1px 描边承托。
- 卡片间空隙仍属于覆盖窗口，按住 Alt 期间点在上面不会传给下面的应用。
- `--switcher-demo` 会接管真实 Alt+Tab，与常驻实例不能同时使用。
- 设置页写配置失败（目录只读、磁盘满）时状态栏会提示，内存里的改动仍然生效，重启后回落到文件内容。
- 开机自启写 `HKCU` 启动项，不需要管理员；部分安全软件会拦截启动项写入。
- 深色标题栏使用 Win11 的 DWM 属性，Windows 10 上回退到系统默认标题栏配色。
- 翻译使用 MyMemory 公共接口，单次最多 480 字符；失败时提供浏览器打开入口。
- `show-desktop` 移植自 GPL-3.0 的 ShowDesktopOneMonitor，该 crate 声明 `GPL-3.0-only`。

## 相关文档

- 各 crate 的模块划分与用法：见上表的 crate 文档列。
- 图标 / logo 生成脚本：`tools/make-icon.ps1`（产物在 `crates/whelper-shell/assets/`）。
- 配置项与 INI 格式：见 [crates/whelper-config](crates/whelper-config/README.md)。
