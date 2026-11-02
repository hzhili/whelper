# whelper-shell（托盘与设置）

`whelper` 的桌面外壳：托盘图标与菜单、设置页、开机自启。和另外三个组件一样是**库**，
由 `whelper` 在同一个进程、同一个主线程里加载，窗口消息由工具条的主循环 `DispatchMessage` 驱动。

## 托盘

- 左键 / 双击：打开设置页（关闭只是隐藏，再开是瞬时的）。
- 右键菜单：`打开设置` / `开机自启`（勾选状态读注册表）/ `退出 whelper`。
- `Shell_NotifyIconW` 失败时（开机自启时 explorer 可能尚未就绪）每 2 秒重试，最多 5 次。
- 退出菜单不是直接 `exit`，而是给工具条窗口发 `WM_CLOSE`，走正常的“销毁窗口 → 摘钩子 → 退出消息循环”。

## 设置页

自绘窗口（GDI 双缓冲），不依赖 comctl32，和工具条 / Alt+Tab 覆盖层共用圆角、配色、DPI 处理：

```text
┌───────────────┬─────────────────────────────────────────────┐
│ [W] whelper   │  划词工具条                                  │
│    桌面助手    │  选中文字后弹出的复制 / 翻译条                 │
│               │                                             │
│  常规         │  ┌───────────────────────────────────────┐  │
│  划词工具条    │  │ 启用划词工具条                  [ ● ] │  │
│  显示桌面      │  ├───────────────────────────────────────┤  │
│  窗口切换器    │  │ 双击 / 三击选词                 [ ● ] │  │
│  关于         │  ├───────────────────────────────────────┤  │
│               │  │ 拖拽触发距离        [2px][4px][8px]   │  │
│               │  └───────────────────────────────────────┘  │
│ 设置即时保存   │                                 已保存       │
└───────────────┴─────────────────────────────────────────────┘
```

| 分类 | 内容 |
| --- | --- |
| 常规 | 开机自启、界面主题、调试日志、配置文件位置（打开目录） |
| 划词工具条 | 总开关、双击/三击选词、拖拽触发距离；组件关闭时这些行变灰 |
| 显示桌面 | `Win+D` 接管开关 + 行为说明 |
| 窗口切换器 | Alt+Tab 接管、默认视图、窗口预览、过渡动画、快捷键说明 |
| 关于 | logo、版本、三个组件的简介、配置路径、打开配置目录、恢复默认设置 |

细节：

- **绘制与点击共用同一份几何**（[`plan`](src/plan.rs)）：导航、行、开关/分段/按钮的矩形在一处算出来，
  不会出现“看到的位置点不到”；每个控件携带一个 [`Action`](src/plan.rs)，点击直接分发。
- **总开关永远可点**：组件关闭时只把它的子选项置灰（第 1 行之后），第 0 行的启用开关不参与禁用逻辑，
  否则会出现“关掉后再也点不开”。点击坐标取自鼠标消息本身，不依赖全局光标位置。
- **即时保存**：任何改动走 `whelper_config::update`，成功后窗口右下角出现 1.8 秒的“已保存”。
- **主题**：`system/light/dark` 三档；深色时用 DWM 的 immersive dark + 标题栏底色/文字色，
  Windows 10 上自动退化。
- **DPI**：`WM_DPICHANGED` 时重建字体并按建议矩形调整窗口；布局全部按 `dpi/96` 缩放。
- **关闭**：`WM_CLOSE` / `Esc` 只隐藏窗口，进程退出时由 `Drop` 销毁。

## 开机自启

写当前用户的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `whelper`：

```text
"D:\path\to\whelper.exe" --autostart
```

- 读取时要求值与当前 exe 完全一致：程序被挪走后设置页显示“未开启”，重新开启会覆盖成新路径。
- `--autostart` 启动路径会先隐藏并 `FreeConsole()` 脱离控制台，登录时不会弹黑框。
- 不需要管理员权限；部分安全软件会拦截启动项写入，此时设置页显示“无法写入启动项”。

## 图标资源

`assets/whelper.ico`（16/20/24/32/40/48/64/128/256，PNG 压缩）与 `assets/logo.png`（README / 关于页）由
[`tools/make-icon.ps1`](../../tools/make-icon.ps1) 生成：

```powershell
powershell -ExecutionPolicy Bypass -File tools/make-icon.ps1
```

设计是圆角方块 + 蓝紫渐变（`#2F6FEB → #7C3AED`）+ 白色描边 “W”，与工具条/切换器的强调色一致；
16px 下只保留粗 W，保证托盘小图标可辨认。

运行时加载走 `whelper-common::win32::icon`：解析 ICO 目录 → 选最接近 `SM_CXICON` / `SM_CXSMICON` 的一项 →
`CreateIconFromResourceEx`；图标通过 `include_bytes!` 编进程序，不写临时文件。Windows 构建还会用 `winresource` 将同一 ICO 嵌入 EXE 的 PE 资源，供资源管理器和快捷方式读取。

## 嵌入方式

```rust
let mut shell = whelper_shell::Shell::start();     // 创建隐藏外壳窗口 + 托盘
// ... 宿主消息循环 ...
if let Some(shell) = shell.as_mut() {
    shell.open_settings();                          // 托盘左键 / --settings 共用
}
```

- 外壳窗口类名 `WHelperShell`，`whelper-common::win32::applink` 通过它投递 `MSG_OPEN_SETTINGS`，
  因此 `whelper --settings` 在已有实例时只是把设置页拉到前台。
- `Shell::start()` 会设置进程 AppUserModelID，任务栏把设置窗口归到 whelper 图标下。

## 模块

| 模块 | 职责 |
| --- | --- |
| `lib` | 外壳窗口、托盘图标、菜单、`open_settings` |
| `settings` | 设置窗口：绘制、交互、即时保存、主题与 DPI |
| `plan` | 设置页的几何与动作表（纯逻辑，可单测） |
| `autostart` | `Run` 启动项的读写 |
| `icons` | 嵌入图标 → `HICON`，`--icon-report` |

## 测试与验证

```powershell
cargo test -p whelper-shell      # 11 项：导航/行/控件几何、禁用态、总开关回归、关于页按钮、缩放、极小窗口、鼠标坐标解码
cargo run -- --icon-report       # 图标资源可被系统解析
cargo run -- --settings          # 打开设置页（已有实例时转发）
cargo run -- --config-report     # 打印当前生效配置
```

托盘回调、单实例转发与设置项落盘做过端到端验证：模拟托盘左键能打开设置页；关闭后
`whelper --settings` 能把已有实例的设置页拉到前台并以 0 退出；开机自启开关会真正写入 / 删除
注册表值；改动后 `config.ini` 与 `--config-report` 一致。

## 已知限制

- 设置页需要写 `%APPDATA%`；写失败时状态栏提示，内存改动仍生效。
- 深色标题栏依赖 Win11 的 DWM 属性，Windows 10 回退到系统配色。
- 托盘图标默认可能被 Windows 收进“隐藏的图标”溢出区，需要用户自行固定。
