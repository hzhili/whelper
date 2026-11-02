# text-picker（划词取词 + 工具条）

划词取词器：在任意应用中选中文本后，光标附近弹出一个小工具条，提供 **复制** 与 **翻译**。

单一职责：只做划词取词与工具条。单屏显示桌面（[`../show-desktop`](../show-desktop)）与 Alt+Tab 切换器
（[`../window-switcher`](../window-switcher)）是独立组件，三者共用 [`../whelper-common`](../whelper-common)
的 Win32 基础封装，彼此互不依赖。

本 crate 是**库**，由 [`../whelper`](../whelper) 加载；项目总览见[根 README](../../README.md)。

## 功能

- **触发**：全局低级鼠标/键盘钩子识别 4 类动作
  - 拖拽划词（位移 ≥ 4px）
  - 双击 / 三击选词
  - `Ctrl+A` 全选
  - `Shift + 方向键` 扩选
- **取词**：优先用 UI Automation `TextPattern.GetSelection()` **只读**当前选区（不动键盘、不动剪贴板），同时取 `GetBoundingRectangles()`：矩形既用于确认选区有效，也用于把工具条放在**选区上方居中**（多行选区不会压住文字）；UIA 取不到时再回退到“模拟 `Ctrl+C` → 等待剪贴板序号变化 → 读取 → **还原取词前的剪贴板**”。
- **过滤**：空文本、纯空白/不可见字符、超过 20000 字符不弹；密码框（`UIA_IsPasswordProperty`）不取；**焦点元素与命中元素不同进程**时不弹（典型是输入法候选窗抢焦点）。
- **终端兼容**：终端的复制语义是“复制并取消选区”（Windows Terminal 的 `copy` 动作默认 `dismissSelection = true`），因此对终端类窗口（`ConsoleWindowClass`、`CASCADIA_HOSTING_WINDOW_CLASS`）和 IDE 宿主进程**不自动复制**：工具条先出现，点「复制 / 翻译」时才真正取词，选区不会因为工具条弹出而消失。
- **工具条**：置顶、**不抢焦点**（`WS_EX_NOACTIVATE`），默认出现在锚点上方，空间不足自动翻到下方，并夹紧到显示器工作区；跟随 DPI。
- **样式**：36px 高 / 5px 内边距 / 10px 卡片圆角 / 1px `#DEE0E3` 描边 / 26px 透明按钮（6px 圆角，hover 6% 黑）/ 14px 文字 / 16px 图标；跟随系统深浅色。翻译图标使用内置的 SVG 轮廓。
- **复制**：写入剪贴板并收起工具条。
- **翻译**：工具条原地展开为结果面板，调用 MyMemory 公共接口（无需 API Key），中英自动判向；点击面板底部可用浏览器打开 Bing 翻译。
- **隐藏**：点击工具条外部、滚轮、任意按键、`Esc`、前台窗口切换（400ms 宽限）、执行完命令后自动收起。

## 运行

本 crate 是**库**，不产出独立 exe：仓库根执行 `cargo run` 会构建唯一可执行文件
`whelper.exe`，并在同一个进程里同时加载本组件与 `show-desktop`。

```powershell
cargo run --release          # 仓库根：一个进程、一个 exe，划词工具条 + 单屏显示桌面
```

常驻后，在记事本 / 浏览器 / Word 等应用里拖选一段文字，即可看到工具条。

其他参数（同样是 `whelper.exe` 的参数）：

```powershell
cargo run -- --demo        # 只演示工具条 UI（不装钩子、不接管 Win+D）
cargo run -- --self-test   # 验证 "模拟 Ctrl+C → 读剪贴板 → 还原" 全链路
cargo run -- --translate "hello world"   # 调试翻译接口
cargo run -- --capture-probe             # 只读取词诊断：打印当前选区与是否动过剪贴板
```

工具条右键菜单：`设置…`（拉起托盘外壳的设置页）/ `退出 whelper`。

配置来自 `whelper-config`（设置页维护，见根 README）：

- `[general] theme`：跟随系统 / 浅色 / 深色（`TEXT_PICKER_THEME` 仍可强制覆盖，调试用）；
- `[general] debug`：打印工具条显示/隐藏原因（`TEXT_PICKER_DEBUG=1` 同样有效）；
- `[text-picker] enabled` / `double-click` / `drag-threshold`：关闭后在鼠标钩子里直接放行，
  已弹出的工具条会在 150ms 的定时器里收起；触发参数每次按下鼠标时读取，改完立即生效。

## 实现结构

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | 参数分发、单实例互斥体 |
| `src/app.rs` | 窗口、消息循环、GDI 绘制、工具条/翻译面板状态机 |
| `src/hooks.rs` | 鼠标钩子 `WH_MOUSE_LL`、取词触发判定 `TriggerDetector`、键盘判定 `on_key_event`（含单测） |
| `src/capture.rs` | 取词编排：UIA 优先、`Ctrl+C` 兜底、终端延迟取词 |
| `src/uia.rs` | 用 `TextPattern.GetSelection()` 只读选区（COM/UIA） |
| `src/clipboard.rs` | 剪贴板文本读写、按格式备份/还原 |
| `src/toolbar.rs` | 纯几何：尺寸、上下翻转、工作区夹紧、按钮命中（含单测） |
| `src/icon.rs` | SVG path 解析 + 抗锯齿光栅化 + GDI 混合（含单测） |
| `src/translate.rs` | 语言判定、URL 构造、响应解析、浏览器兜底（含单测） |
| `src/selftest.rs` | 真实 EDIT 控件 + 真实剪贴板的自测 |
| `src/winutil.rs` | 取词/UI 特有的 Win32 辅助（前台窗口、强制激活、窗口类与进程名）；通用项从 `whelper-common` 转出 |
| `../whelper-common` | 共用底座：显示器几何、宽字符串、单实例、低级键盘钩子、深浅色主题 |
| `../whelper-config` | 全局配置：启停、双击选词、拖拽阈值、主题、调试日志 |

设计要点：

1. 钩子回调里**不做取词**，只 `PostMessage` 给主窗口；取词在后台线程完成，避免阻塞输入。
2. 取词前备份剪贴板、取词后还原，避免污染用户剪贴板。
3. 工具条永不激活，因此 `Ctrl+C` 仍然发给原来的应用；失焦通过前台窗口轮询判断。
4. 所有窗口尺寸都从逻辑像素按 DPI 缩放，位置统一经过 `toolbar::place` 夹紧。
5. 键盘钩子的安装/解码在 `whelper-common`，本组件只实现“是否可能产生选区”的判定，且从不吞键。

## 验证

```powershell
cargo test --workspace   # 划词/翻译/图标测试 + 共享底座 + show-desktop / window-switcher 的独立测试
cargo run -- --self-test
```

终端取词用 `--capture-probe` 验证（在 Windows Terminal 里选中 `Microsoft` 后执行）：

```
captured="Microsoft"
bounds=(86, 127)-(167, 146)  # 选区矩形：工具条会以 (126, 127) 上方居中
clipboard_changed=false      # 走 UIA，没有发送 Ctrl+C，选区保持
```

终端内没有选区时返回 `deferred=true`：工具条照常出现，但不复制、不动选区。

`--self-test` 在真实桌面创建 EDIT 控件并全选，走与生产相同的取词链路，输出：

```
PASS 取词成功：text-picker self-test：Hello, whelper!
PASS 剪贴板已恢复到取词前的内容
SELF-TEST PASS
```

`--demo` 已用 Win32 API 验证：窗口 132×36、`WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW`、不抢前台、点击「复制」后剪贴板内容正确且工具条隐藏；点击「翻译」后窗口切换为 360×120 结果面板。

样式也用逐点取色核对过：卡片 `#FFFFFF`、描边 `#DEE0E3`、按钮未 hover 时为卡片色、hover `#F0F0F0`；深色主题下卡片 `#292929`、描边 `#5F5F5F`、hover `#383838`。

## 已知限制

- **以普通权限运行本程序时，无法从管理员权限的窗口取词**：UIA 与 `SendInput` 都受 Windows UIPI 约束（跨完整性级别既读不到 UIA 树，也注入不进按键）。要覆盖提权窗口，需要以管理员运行或使用签名的 `UIAccess`。
- UIA 路径依赖应用暴露 `TextPattern`；不暴露的应用仍走 `Ctrl+C` + 剪贴板回退（此时复制会取消选区的终端已被单独排除，见上文「终端兼容」）。
- 不支持 `Shift + 点击` 扩选触发，也不支持应用/网址黑名单。
- 剪贴板还原只覆盖 HGLOBAL 类格式；位图（`CF_BITMAP`）等句柄型格式会被跳过。
- 翻译使用公共接口，单次最多 480 字符；接口失败时面板会给出手动打开浏览器的入口。
- 卡片用区域窗口（`SetWindowRgn`）实现圆角，因此只有 1px 描边，没有 webview 那种高斯模糊投影。
- 仅支持 Windows 10+ 的交互式桌面会话（服务/会话 0 下无效）。
- 右键工具条会弹菜单（设置 / 退出），菜单需要点一下别处才会收起；工具条本身仍不抢焦点。
- 终端/IDE 采用“先弹条、点按钮再取词”：在这些窗口里拖动标签、文件也会弹条（无法从窗口句柄区分 IDE 的终端面板与编辑器）。
- UIA 读不到且不属于终端/IDE 的应用仍走 `Ctrl+C`：这类应用的复制如果会取消选区，行为与旧版一致。
