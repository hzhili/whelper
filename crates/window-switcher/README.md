# window-switcher（Alt+Tab 切换器）

接管系统 `Alt+Tab`，默认用深色居中托盘单行展示应用图标；网格视图逐窗口显示实时预览卡片，并支持焦点切换。

交互模型参考 [sigoden/window-switcher](https://github.com/sigoden/window-switcher)（按住 Alt 连续按键循环、
松开才真正切换）；默认视觉采用深色半透明托盘、应用图标和蓝色整卡片选中高亮。
网格窗口画面由 **DWM 实时缩略图**（`DwmRegisterThumbnail`）合成。集成方式按本项目约定：覆盖窗口 + GDI 绘制 +
共享键盘钩子链，作为库链入 `whelper.exe`，不产出独立 exe。

## 交互

| 操作 | 行为 |
| --- | --- |
| `Alt+Tab` | 打开切换器并预选下一个窗口；保持按住再按继续后移 |
| `Shift+Alt+Tab` | 反向移动 |
| 松开 `Alt` | 提交：激活当前选择，覆盖层淡出 |
| `Esc` | 取消：不切换，覆盖层淡出 |
| `` ` ``（Alt 按住时） | 应用图标条 / 窗口预览网格切换（带布局过渡动画） |
| `←` `→` / `↑` `↓`（`PageUp/Down`）/ `Home` `End` | 移动选择；图标条按应用前后移动，网格按行列移动 |
| 鼠标移动 / 点击 / 滚轮 | 悬停高亮、点击即提交、滚轮前后移动 |

## 两种视图

| 视图 | 布局 | 适用 |
| --- | --- | --- |
| 缩略图 `Thumbnails` | 屏幕居中深色托盘，单行只显示应用图标，同一应用的多个窗口合并为一个候选 | 默认的应用级切换视图 |
| 网格 `Grid` | 每个窗口显示 DWM 实时预览和标题，按多行网格排列 | 逐窗口选择，保留当前预览卡片行为 |

图标条选择某个应用时，提交该应用 Z 序最前的窗口；网格中同一应用的多个窗口分别显示。
切换视图时用 `layout::lerp` 过渡布局；打开时面板从 96% 轻微放大到 100% 并淡入，移动选择时列表平移。
界面按检测到的系统版本调整托盘形状与选中边框：Windows 10 使用直角托盘，Windows 11 使用圆角托盘。
图标较多时横向滚动，并确保当前选中的应用图标可见。

## 配置

设置页维护在 `%APPDATA%\whelper\config.ini` 的 `[switcher]`（`[general] theme` 也影响调色板）：

| 键 | 作用 |
| --- | --- |
| `enabled` | 关闭后钩子不再接管 `Alt+Tab`；若切换器正开着会先收到取消消息 |
| `cursor-monitor-only` | 开启后候选只包含鼠标所在显示器上的窗口；关闭时跨显示器显示 |
| `default-view` | 每次打开使用的视图（`thumbnails` 应用图标条 / `grid` 窗口预览网格）；会话内 `` ` `` 只影响本次，不回写配置 |
| `previews` | 关闭后网格视图不注册 DWM 实时缩略图，预览卡片改为应用图标 |
| `animation` | 关闭后不做淡入/缩放/插值，直接显示目标布局（适合远程桌面/低配机） |

这些值在**每次打开会话时**读取一次，改完下一次 `Alt+Tab` 即生效。

## 实现要点

1. **枚举与 `show-desktop` 共用一份实现**：`whelper_common::win32::window::alt_tab_windows()`。
   cloaked / `WS_DISABLED` / `WS_EX_NOACTIVATE` / `WS_EX_TOOLWINDOW` / `IsAltTabWindow` 的过滤只有一份，
   顺序是 `EnumWindows` 的 Z 序（索引 0 = 当前前台窗口），两个功能看到的候选集合不会漂移。
2. **只吞 Tab、放行 Alt**：前台应用已经收到 Alt 按下，吞掉配对的抬起会让它以为 Alt 一直按着；
   但只看到 Alt 起落会激活菜单栏，所以第一次吞 Tab 时注入一次 `0xFF` 掩码键
   （与 `Win+D` 防开始菜单是同一份 `keyboard::send_dummy_key`）。Esc 取消同样放行 Alt 抬起。
3. **网格预览是 DWM 实时缩略图**：`DwmRegisterThumbnail` 把源窗口画面直接合成到托盘预览项内，
   播放视频/滚动都会实时更新，零抓屏开销；注册失败（受保护窗口等）时显示应用图标占位。
   DWM 预览画在绘制面**之上**且只能是直角矩形，蓝色边框环绕预览区域，标题行放在缩略图下方。
   应用图标视图不注册 DWM 缩略图，只显示按可执行文件分组后的应用图标。
4. **网格缩略图随帧同步**：每帧对每个可见缩略图 `DwmUpdateThumbnailProperties`——目标矩形跟随布局
   插值（列表平移/入场缩放时缩略图跟着走），`opacity` 跟随整窗淡入淡出；滚出可见区或
   源窗口关闭的缩略图立即注销。注册失败的窗口记入失败集，本会话内不再重试。
5. **焦点切换**（`whelper_common::win32::window::activate`）：最小化先 `SW_RESTORE`；
   再 `AttachThreadInput` + `SetForegroundWindow` 并轮询确认；被前台锁拒绝时注入一次掩码键重试，
   最后退到系统 Alt+Tab 自己用的 `SwitchToThisWindow`。
6. **覆盖窗口不抢焦点**：`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED`；
   窗口尺寸就是面板矩形，`SetWindowRgn` 将命中区域限制在面板范围。
7. **图标在后台线程**：`WM_GETICON` 走 `SendMessageTimeout`，遇到卡死窗口会等 50ms，
   留在主线程会把入场动画拖成“直接跳到终点”；图标整批投递，只触发一次重绘。

## 外观（Windows 10 风格托盘 + DWM 实时缩略图）

| 元素 | 取值 |
| --- | --- |
| 背景 | Windows 10 深色托盘 `#202020`、直角、约 90% 不透明；Windows 11 使用圆角托盘、约 88% 不透明 |
| 预览项 | 缩略图视图为单行应用图标；网格视图为 DWM 实时窗口预览（`DwmRegisterThumbnail`，含窗口边框） |
| 字体 | 网格标题使用 Segoe UI 12px（中文由 GDI 字体链接回退雅黑），白色 |
| 标题行 | 仅网格视图显示：26px，16px 应用图标 + 居中标题，位于预览下方 |
| 选中 | 当前卡片整体使用蓝色高亮底（`#005696`），外圈叠加系统蓝色边框（`#0078D7`）；Windows 10 为 1px，Windows 11 为 2px |
| 间距 | 面板内边距 20px、预览项间距 12px；无独立卡片留白 |
| 动画 | 打开：96% → 100% + 淡入 110ms；切换视图 160ms 插值；淡出 90ms |

绘制管线（每帧重建 32bpp 绘制面，再 `UpdateLayeredWindow`）：

```text
1. 清零                                      → 背景全透明
2. GDI：深色托盘、预览边框、占位图标与标题       → 写入 RGB
3. sync_thumbnails：注册/注销/移动缩略图 + 同步不透明度 → DWM 合成
4. set_alpha_round_rect：托盘区域按系统版本设置不透明度 → 逐像素 alpha
5. UpdateLayeredWindow(整窗 alpha=淡入值)          → 交给 DWM 合成
```

- 面板由 alpha 遮罩绘制半透明托盘，绘制端在最终合成前回填 alpha，避免 GDI 写入的零 alpha 使托盘消失。
- 帧透明度按时间轴现算（`effective_alpha`），消息把 `WM_TIMER` 挤后也不会卡在全透明。
- 首次绘制可能很慢，入场动画的时钟从第一个 `WM_TIMER` 起算（`anim_deferred`），
  避免被首帧耗时直接“跳过”。
- DWM 缩略图画在绘制面之上、只能是直角矩形：预览项使用直角边框，标题行放在缩略图下方；
  缩略图在目标矩形内保持源宽高比（letterbox 露出深色托盘）。

## 模块

| 模块 | 职责 |
| --- | --- |
| `model` | 会话状态：窗口清单、当前选择、视图模式、导航步长（纯逻辑，可单测） |
| `layout` | 应用图标条与窗口预览网格的几何、命中测试、布局插值、缩略图等比适配（纯逻辑，可单测） |
| `anim` | 时间轴与缓动（纯逻辑，可单测） |
| `shortcut` | Alt+Tab 状态机（纯判定）+ 共享钩子处理器 + 消息投递 |
| `win32` | 候选窗口枚举、光标显示器工作区与 DPI、调试开关 |
| `preview` | 应用图标：Shell 大尺寸图标优先、窗口图标回退、图标句柄缓存与绘制 |
| `surface` | 32bpp DIB 绘制面：逐像素 alpha、圆角遮罩、`UpdateLayeredWindow` |
| `overlay` | 覆盖窗口：应用图标/预览卡片绘制、DWM 缩略图同步、动画帧、鼠标交互、消息分发 |
| `service` | 常驻形态：建覆盖窗口 + 注册钩子，供宿主嵌入 |
| `app` | 一次性命令：`report()` / `self_test()` / `demo()` |

## 嵌入方式

```rust
// 宿主（whelper）在主线程、进入自己的消息循环之前启动
let service = window_switcher::service::Service::start();
// ... 宿主消息循环（GetMessage/DispatchMessage）
drop(service);   // 摘钩子；覆盖窗口随 Overlay 析构销毁
```

触发链：

```text
WH_KEYBOARD_LL（whelper-common 的共享钩子链）
  └─ window-switcher 处理器：Alt 按住 + Tab 按下 → 吞掉 Tab、注入掩码键
        └─ PostMessage(MSG_START / MSG_STEP / MSG_COMMIT / …) ──► 覆盖窗口 wndproc
                                                                   ├─ 枚举窗口、布局、后台取图标、注册缩略图
                                                                   └─ 提交时 activate(选中窗口)
```

## 一次性命令

由 `whelper` 的 CLI 转发到 `app`：

| 命令 | 行为 |
| --- | --- |
| `--switcher-report` | 只读：候选窗口、应用分组数量、图标条与预览网格尺寸 |
| `--switcher-self-test` | 激活另一个窗口 → 校验前台 → 切回原窗口 |
| `--switcher-demo` | 接管 Alt+Tab 并跑独立消息循环（交互式验证；会与常驻实例抢同一个热键） |

## 测试

```powershell
cargo test -p window-switcher      # 单元测试与文档测试
```

覆盖：选择环绕与行移动、视图切换、Alt+Tab 状态机（启动/连按/反向/提交/取消/吞抬起/掩码键）、
应用图标条与窗口预览网格的几何和命中测试、布局插值 / 整体缩放、等比适配、高 DPI 与极小工作区、
alpha 遮罩（圆角判定、半径夹紧）。

真实链路验证：

```powershell
cargo run -- --switcher-report
cargo run -- --switcher-self-test
```

`--switcher-report` 输出示例：

```text
Alt+Tab 候选窗口 16 个（与单屏显示桌面共用同一份枚举与 Z 序）：
  0xa421f0   可见    微信
  0x10d8c    可见    主页 / X - Thorium
  ...
缩略图视图：单行应用图标，可见 12/13 个，面板 784×80，图标格 56×56
网格视图：4 列 × 4 行，面板 1120×848，卡片 261×193
```

`--switcher-demo` + 注入 Alt+Tab（`SWITCHER_DEBUG=1`，连续按三次 Tab 后松开 Alt）：

```text
[switcher] 打开会话：13 个候选，选中 1（Thumbnails）
[switcher] 选择 3/13
[switcher] 选择 4/13
[switcher] 提交 0xa421f0：激活成功
```

## 已知限制

- 网格预览是 **DWM 直角矩形**：不能参与 GDI 边框绘制；托盘和预览项边框分别由分层窗口与 GDI 绘制，标题行放在缩略图下方。
- 受保护内容或已注销源窗口的缩略图注册失败，卡片只显示居中的应用图标。
- 当前按 Windows 10/11 选择对应托盘形状和选中边框，选中卡片使用整卡片高亮；系统原生界面仍可能随 Windows 更新和系统设置略有差异，本项目使用自绘覆盖层。
- 面板及预览项间空隙属于覆盖窗口（按住 Alt 期间点击不会传给下面的应用）。
- 候选顺序当前仍是 `EnumWindows` 的 Z 序，不是系统内部维护的 MRU；在 Windows 10/11 实机对照完成前，不宣称与系统原生排序完全一致。核心按键行为（Alt+Tab 前进、Shift 反向、释放 Alt 提交、Esc 取消）按原生切换习惯实现；网格、反引号切换视图及鼠标操作属于保留的扩展功能。
- 普通权限无法切换管理员权限窗口（UIPI），`activate` 会返回 `false`（`SWITCHER_DEBUG=1` 可看到）。
- 独占全屏（exclusive fullscreen）之上覆盖层不可见。
- 仅 Windows 10+ 交互式桌面会话；`--switcher-demo` 与常驻实例不能同时接管 Alt+Tab。
