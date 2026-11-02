# whelper-config（全局配置）

`whelper` 的全部可配置项集中在这里：默认值、INI 解析 / 生成、进程内共享与落盘。
设置页、划词工具条、单屏显示桌面、Alt+Tab 切换器读的是**同一份结构**，不再各自散落环境变量。

只依赖 std，保持与仓库其它 crate 一致。

## 配置项

| 分类 | 键 | 默认 | 说明 |
| --- | --- | --- | --- |
| `[general]` | `theme` | `system` | `system` / `light` / `dark`，三个组件的弹窗统一 |
| | `debug` | `false` | 打印各组件运行细节（开发者用） |
| `[text-picker]` | `enabled` | `true` | 划词工具条总开关 |
| | `double-click` | `true` | 双击 / 三击选词是否触发 |
| | `drag-threshold` | `4` | 拖拽触发距离（像素，1–40，越界自动夹紧） |
| `[desktop]` | `enabled` | `true` | 是否接管 `Win+D` |
| `[switcher]` | `enabled` | `true` | 是否接管 `Alt+Tab` |
| | `cursor-monitor-only` | `false` | 是否只显示鼠标所在显示器上的窗口 |
| | `default-view` | `thumbnails` | `thumbnails` / `grid` |
| | `previews` | `true` | 是否抓取窗口预览图 |
| | `animation` | `true` | 打开 / 切换视图的过渡动画 |

开机自启不在配置文件里：它写 `HKCU\...\Run`，由 `whelper-shell::autostart` 直接读注册表真实状态。

## 文件位置与格式

`%APPDATA%\whelper\config.ini`（`APPDATA` 不可用时退回 `LOCALAPPDATA`，再退回当前目录）。

```ini
; whelper 配置文件 —— 设置页会自动维护，也可以退出 whelper 后手工编辑
; 手工修改后重启 whelper 生效；设置页里的开关会立即写回这里

[general]
; 界面主题：system（跟随系统）/ light / dark
theme = system
; 在控制台打印各组件的运行细节（开发者用）
debug = false

[text-picker]
enabled = true
double-click = true
drag-threshold = 4

[desktop]
enabled = true

[switcher]
enabled = true
cursor-monitor-only = false
default-view = thumbnails
previews = true
animation = true
```

解析规则：`;` / `#` 注释、UTF-8 BOM、`key = value`、大小写不敏感；未知键、非法值、缺失项都保持默认值。
每次保存都整份重写（带注释），不会把用户的其它内容写回去。

## API

```rust
whelper_config::install(whelper_config::load());      // 启动时装载
let config = whelper_config::get();                   // 组件读快照（结构很小，直接克隆）

whelper_config::update(|config| {                     // 设置页改一项：先改内存、递增 revision，再落盘
    config.switcher.previews = false;
})?;

whelper_config::revision();                           // 判断“配置变了，需要刷新”
whelper_config::reset()?;                             // 恢复默认
whelper_config::config_path();                        // 设置页 / 诊断展示
```

- `get()` 在 `install()` 之前返回默认配置，组件单独使用时不会去读盘。
- `update()` 即使写盘失败也保留内存改动，错误字符串交给设置页状态栏显示。
- 写盘是“直接覆盖写”（配置很小），没有引入原子替换的额外依赖。

## 测试

```powershell
cargo test -p whelper-config      # 17 项
```

覆盖：默认值、逐键覆盖、`render → parse` 往返、未知/非法值回落、阈值夹紧、中文枚举标签、
枚举索引往返、路径优先级（`APPDATA` → `LOCALAPPDATA`）、坏行容忍、注释与引号、文件不存在时的默认值、
保存后重新读取。
