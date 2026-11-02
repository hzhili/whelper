//! 全局配置：默认值 + INI 持久化 + 进程内共享。
//!
//! 设计目标：
//!
//! - **一处定义**：设置页、划词工具条、单屏显示桌面、Alt+Tab 切换器读同一份结构，
//!   不再各自散落 `env` 开关；
//! - **可手工编辑**：落在 `%APPDATA%\whelper\config.ini`，缺项/写错都回落到默认值；
//! - **即时生效**：[`update`] 先改内存（递增 [`revision`]）再落盘，组件在事件或定时器里
//!   读 [`get`] 即可拿到新值；
//! - **无外部依赖**：只用 std，与仓库其它 crate 保持一致。
//!
//! 调试用的环境变量（`TEXT_PICKER_DEBUG`、`WHELPER_HOOK_DEBUG` 等）仍然保留，
//! 优先级高于配置，方便不改文件就排查问题。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

mod ini;

/// 配置目录名（`%APPDATA%\whelper`）。
pub const APP_DIR: &str = "whelper";
/// 配置文件名。
pub const FILE_NAME: &str = "config.ini";

/// 拖拽触发距离的合法范围（像素）。
pub const DRAG_MIN: i32 = 1;
pub const DRAG_MAX: i32 = 40;

// ---------- 枚举 ----------

/// 界面主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("system") || raw.eq_ignore_ascii_case("auto") || raw == "跟随系统"
        {
            Some(Self::System)
        } else if raw.eq_ignore_ascii_case("light") || raw == "浅色" {
            Some(Self::Light)
        } else if raw.eq_ignore_ascii_case("dark") || raw == "深色" {
            Some(Self::Dark)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::System)
    }
}

/// Alt+Tab 切换器的默认视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultView {
    Thumbnails,
    Grid,
}

impl DefaultView {
    pub const ALL: [Self; 2] = [Self::Thumbnails, Self::Grid];

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("thumbnails") || raw.eq_ignore_ascii_case("thumbnail") || raw == "缩略图" {
            Some(Self::Thumbnails)
        } else if raw.eq_ignore_ascii_case("grid") || raw == "网格" {
            Some(Self::Grid)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thumbnails => "thumbnails",
            Self::Grid => "grid",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Thumbnails => "缩略图",
            Self::Grid => "网格",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Thumbnails => 0,
            Self::Grid => 1,
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Thumbnails)
    }
}

// ---------- 配置结构 ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct General {
    pub theme: Theme,
    /// 在控制台打印组件运行细节（等同打开各组的 DEBUG 环境变量）。
    pub debug: bool,
}

impl Default for General {
    fn default() -> Self {
        Self { theme: Theme::System, debug: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    /// 划词工具条总开关。
    pub enabled: bool,
    /// 双击 / 三击选词是否触发工具条。
    pub double_click: bool,
    /// 拖拽触发距离（像素，[`DRAG_MIN`]..=[`DRAG_MAX`]）。
    pub drag_threshold: i32,
}

impl Default for Picker {
    fn default() -> Self {
        Self { enabled: true, double_click: true, drag_threshold: 4 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    /// 是否接管 Win+D。
    pub enabled: bool,
}

impl Default for Desktop {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switcher {
    /// 是否接管 Alt+Tab。
    pub enabled: bool,
    /// 是否只显示鼠标所在显示器上的窗口。
    pub cursor_monitor_only: bool,
    pub default_view: DefaultView,
    /// 是否显示窗口预览（DWM 实时缩略图）。
    pub previews: bool,
    /// 打开 / 切换视图 / 移动选择时是否播放过渡动画。
    pub animation: bool,
}

impl Default for Switcher {
    fn default() -> Self {
        Self {
            enabled: true,
            cursor_monitor_only: false,
            default_view: DefaultView::Thumbnails,
            previews: true,
            animation: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub general: General,
    pub picker: Picker,
    pub desktop: Desktop,
    pub switcher: Switcher,
}

impl Config {
    /// 解析 INI 文本；未知键、非法值、缺失项都保持默认值。
    pub fn parse(text: &str) -> Self {
        let mut config = Self::default();
        let ini = ini::Ini::parse(text);

        if let Some(value) = ini.get("general", "theme").and_then(Theme::parse) {
            config.general.theme = value;
        }
        if let Some(value) = ini.get("general", "debug").and_then(parse_bool) {
            config.general.debug = value;
        }

        if let Some(value) = ini.get("text-picker", "enabled").and_then(parse_bool) {
            config.picker.enabled = value;
        }
        if let Some(value) = ini.get("text-picker", "double-click").and_then(parse_bool) {
            config.picker.double_click = value;
        }
        if let Some(value) = ini.get("text-picker", "drag-threshold").and_then(|raw| raw.trim().parse::<i32>().ok()) {
            config.picker.drag_threshold = value;
        }

        if let Some(value) = ini.get("desktop", "enabled").and_then(parse_bool) {
            config.desktop.enabled = value;
        }

        if let Some(value) = ini.get("switcher", "enabled").and_then(parse_bool) {
            config.switcher.enabled = value;
        }
        if let Some(value) = ini
            .get("switcher", "cursor-monitor-only")
            .and_then(parse_bool)
        {
            config.switcher.cursor_monitor_only = value;
        }
        if let Some(value) = ini.get("switcher", "default-view").and_then(DefaultView::parse) {
            config.switcher.default_view = value;
        }
        if let Some(value) = ini.get("switcher", "previews").and_then(parse_bool) {
            config.switcher.previews = value;
        }
        if let Some(value) = ini.get("switcher", "animation").and_then(parse_bool) {
            config.switcher.animation = value;
        }

        config.sanitize();
        config
    }

    /// 把越界或非法的值拉回合法范围。
    pub fn sanitize(&mut self) {
        self.picker.drag_threshold = self.picker.drag_threshold.clamp(DRAG_MIN, DRAG_MAX);
    }

    /// 生成带注释的 INI 文本（每次保存都整份重写）。
    pub fn render(&self) -> String {
        let mut text = String::new();
        text.push_str("; whelper 配置文件 —— 设置页会自动维护，也可以退出 whelper 后手工编辑\n");
        text.push_str("; 手工修改后重启 whelper 生效；设置页里的开关会立即写回这里\n\n");

        text.push_str("[general]\n");
        text.push_str("; 界面主题：system（跟随系统）/ light / dark\n");
        text.push_str(&format!("theme = {}\n", self.general.theme.as_str()));
        text.push_str("; 在控制台打印各组件的运行细节（开发者用）\n");
        text.push_str(&format!("debug = {}\n\n", self.general.debug));

        text.push_str("[text-picker]\n");
        text.push_str("; 划词工具条总开关\n");
        text.push_str(&format!("enabled = {}\n", self.picker.enabled));
        text.push_str("; 双击 / 三击选词是否触发工具条\n");
        text.push_str(&format!("double-click = {}\n", self.picker.double_click));
        text.push_str("; 拖拽触发距离（像素，1-40）\n");
        text.push_str(&format!("drag-threshold = {}\n\n", self.picker.drag_threshold));

        text.push_str("[desktop]\n");
        text.push_str("; 单屏显示桌面：接管 Win+D，只影响光标所在显示器\n");
        text.push_str(&format!("enabled = {}\n\n", self.desktop.enabled));

        text.push_str("[switcher]\n");
        text.push_str("; Alt+Tab 窗口切换器总开关\n");
        text.push_str(&format!("enabled = {}\n", self.switcher.enabled));
        text.push_str("; 是否只显示鼠标所在显示器上的窗口\n");
        text.push_str(&format!(
            "cursor-monitor-only = {}\n",
            self.switcher.cursor_monitor_only
        ));
        text.push_str("; 默认视图：thumbnails（缩略图）/ grid（网格）\n");
        text.push_str(&format!("default-view = {}\n", self.switcher.default_view.as_str()));
        text.push_str("; 是否抓取窗口预览图\n");
        text.push_str(&format!("previews = {}\n", self.switcher.previews));
        text.push_str("; 打开 / 切换视图时是否播放过渡动画\n");
        text.push_str(&format!("animation = {}\n", self.switcher.animation));

        text
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("true") || raw == "1" || raw == "yes" || raw.eq_ignore_ascii_case("on") {
        Some(true)
    } else if raw.eq_ignore_ascii_case("false") || raw == "0" || raw == "no" || raw.eq_ignore_ascii_case("off")
    {
        Some(false)
    } else {
        None
    }
}

// ---------- 进程内共享 ----------

struct Shared {
    config: Config,
    revision: u64,
    path: PathBuf,
}

fn state() -> &'static Mutex<Shared> {
    static STATE: OnceLock<Mutex<Shared>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(Shared {
            config: Config::default(),
            revision: 0,
            path: default_path(),
        })
    })
}

fn lock() -> MutexGuard<'static, Shared> {
    state().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 配置文件路径（`%APPDATA%\whelper\config.ini`）。
pub fn config_path() -> PathBuf {
    lock().path.clone()
}

/// 默认路径：优先 `APPDATA`，其次 `LOCALAPPDATA`，再退到当前目录。
pub fn default_path() -> PathBuf {
    path_from(|key| std::env::var_os(key))
}

fn path_from(env: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let base = env("APPDATA")
        .or_else(|| env("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DIR).join(FILE_NAME)
}

/// 读盘；文件不存在或读失败都返回默认配置。
pub fn load() -> Config {
    load_from(&config_path())
}

pub fn load_from(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .map(|text| Config::parse(&text))
        .unwrap_or_default()
}

/// 启动时装载配置：之后各组件通过 [`get`] 读取。
pub fn install(config: Config) {
    let mut shared = lock();
    shared.config = config;
    shared.revision = shared.revision.wrapping_add(1);
}

/// 当前配置快照（结构很小，直接克隆，避免把锁带出调用点）。
pub fn get() -> Config {
    lock().config.clone()
}

/// 每次 [`update`] / [`install`] 递增；组件可用它判断“配置变了，需要刷新”。
pub fn revision() -> u64 {
    lock().revision
}

/// 修改配置并落盘：先改内存（失败也保留），再写文件，错误交给设置页显示。
pub fn update(mutate: impl FnOnce(&mut Config)) -> Result<(), String> {
    let (config, path) = {
        let mut shared = lock();
        mutate(&mut shared.config);
        shared.config.sanitize();
        shared.revision = shared.revision.wrapping_add(1);
        (shared.config.clone(), shared.path.clone())
    };
    save_to(&path, &config).map_err(|error| format!("保存配置失败：{error}"))
}

/// 恢复默认配置并落盘。
pub fn reset() -> Result<(), String> {
    update(|config| *config = Config::default())
}

/// 把配置写到指定路径（自动创建目录）。
pub fn save_to(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(path, config.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.general.theme, Theme::System);
        assert!(config.picker.enabled);
        assert!(config.picker.double_click);
        assert_eq!(config.picker.drag_threshold, 4);
        assert!(config.desktop.enabled);
        assert!(config.switcher.enabled);
        assert!(!config.switcher.cursor_monitor_only);
        assert_eq!(config.switcher.default_view, DefaultView::Thumbnails);
        assert!(config.switcher.previews);
        assert!(config.switcher.animation);
    }

    #[test]
    fn parses_overrides() {
        let config = Config::parse(
            "[general]\ntheme = dark\ndebug = true\n\
             [text-picker]\nenabled = false\ndouble-click = false\ndrag-threshold = 12\n\
             [desktop]\nenabled = false\n\
             [switcher]\nenabled = false\ncursor-monitor-only = true\ndefault-view = grid\npreviews = false\nanimation = false\n",
        );
        assert_eq!(config.general.theme, Theme::Dark);
        assert!(config.general.debug);
        assert!(!config.picker.enabled);
        assert!(!config.picker.double_click);
        assert_eq!(config.picker.drag_threshold, 12);
        assert!(!config.desktop.enabled);
        assert!(!config.switcher.enabled);
        assert_eq!(config.switcher.default_view, DefaultView::Grid);
        assert!(config.switcher.cursor_monitor_only);
        assert!(!config.switcher.previews);
        assert!(!config.switcher.animation);
    }

    #[test]
    fn render_roundtrips() {
        let mut config = Config::default();
        config.general.theme = Theme::Light;
        config.picker.drag_threshold = 9;
        config.switcher.default_view = DefaultView::Grid;
        config.switcher.cursor_monitor_only = true;
        config.switcher.animation = false;
        assert_eq!(Config::parse(&config.render()), config);
    }

    #[test]
    fn unknown_and_invalid_values_keep_defaults() {
        let config = Config::parse(
            "[general]\ntheme = rainbow\n[text-picker]\ndrag-threshold = abc\n\
             [switcher]\ndefault-view = spiral\n[bogus]\nfoo = bar\n",
        );
        assert_eq!(config, Config::default());
    }

    #[test]
    fn drag_threshold_is_clamped() {
        let low = Config::parse("[text-picker]\ndrag-threshold = 0\n");
        assert_eq!(low.picker.drag_threshold, DRAG_MIN);
        let high = Config::parse("[text-picker]\ndrag-threshold = 999\n");
        assert_eq!(high.picker.drag_threshold, DRAG_MAX);
    }

    #[test]
    fn full_width_labels_are_accepted() {
        assert_eq!(Theme::parse("深色"), Some(Theme::Dark));
        assert_eq!(DefaultView::parse("网格"), Some(DefaultView::Grid));
    }

    #[test]
    fn enum_index_roundtrips() {
        for theme in Theme::ALL {
            assert_eq!(Theme::from_index(theme.index()), theme);
        }
        for view in DefaultView::ALL {
            assert_eq!(DefaultView::from_index(view.index()), view);
        }
        assert_eq!(Theme::from_index(99), Theme::System);
    }

    #[test]
    fn path_prefers_appdata() {
        let path = path_from(|key| match key {
            "APPDATA" => Some(OsString::from("C:\\Users\\me\\AppData\\Roaming")),
            _ => None,
        });
        assert!(path.ends_with(Path::new("whelper").join(FILE_NAME)));
        assert!(path.starts_with("C:\\Users\\me\\AppData\\Roaming"));
    }

    #[test]
    fn path_falls_back_to_local_appdata() {
        let path = path_from(|key| match key {
            "LOCALAPPDATA" => Some(OsString::from("C:\\Local")),
            _ => None,
        });
        assert!(path.starts_with("C:\\Local"));
    }

    #[test]
    fn missing_file_loads_defaults() {
        let path = std::env::temp_dir().join("whelper-no-such-config.ini");
        let _ = std::fs::remove_file(&path);
        assert_eq!(load_from(&path), Config::default());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("whelper-config-test-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut config = Config::default();
        config.general.theme = Theme::Dark;
        config.picker.drag_threshold = 16;
        save_to(&path, &config).unwrap();
        assert_eq!(load_from(&path), config);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
