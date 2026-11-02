//! 切换编排：枚举 → 决策 → 执行 → 记忆。

use std::error::Error as StdError;
use std::fmt;

use whelper_common::geometry::{MonitorId, MonitorInfo};

use crate::backend::DesktopBackend;
use crate::plan::{self, Action as PlanAction};
use crate::store::StateStore;
use crate::window::{WindowEntry, WindowId, WindowState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// 取不到光标位置或光标不在任何显示器上。
    NoCursorMonitor,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCursorMonitor => write!(f, "无法确定光标所在显示器"),
        }
    }
}

impl StdError for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    Minimize,
    Restore,
}

/// 一次切换的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleReport {
    pub monitor: MonitorId,
    pub action: ToggleAction,
    /// 该显示器上参与决策的窗口数（含已最小化 / 不可见的窗口）。
    pub considered: usize,
    /// 实际下发 `ShowWindow` 的窗口数。
    pub affected: usize,
}

/// 单个窗口的预览信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDescriptor {
    pub id: WindowId,
    pub title: String,
    pub state: WindowState,
}

/// 只读预览：下一次按下会做什么，用于自检 / 日志，不改变任何窗口状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub monitor: MonitorId,
    pub monitor_info: Option<MonitorInfo>,
    pub windows: Vec<WindowDescriptor>,
    pub action: ToggleAction,
    pub affected: usize,
}

/// 「单显示器显示桌面」的状态机。
pub struct ShowDesktop<B: DesktopBackend> {
    backend: B,
    store: StateStore,
}

impl<B: DesktopBackend> ShowDesktop<B> {
    pub fn new(backend: B) -> Self {
        Self { backend, store: StateStore::new() }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// 可变借用后端，供接入方在运行期调整配置（测试里用来模拟窗口变化）。
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// 对光标所在显示器做一次「最小化 / 还原」切换。
    pub fn toggle_at_cursor(&mut self) -> Result<ToggleReport, Error> {
        let monitor = self.backend.cursor_monitor().ok_or(Error::NoCursorMonitor)?;
        Ok(self.toggle(monitor))
    }

    /// 对指定显示器切换。
    pub fn toggle(&mut self, monitor: MonitorId) -> ToggleReport {
        let current = self.entries_on(monitor);
        // 与上游一致：记忆只服务本次决策，无论走哪条分支都先取走
        // （还原分支作废，最小化分支由新清单覆盖）。
        let remembered = self.store.take(monitor);

        match plan::decide(&current, remembered.as_deref()) {
            PlanAction::Minimize(targets) => {
                for window in &targets {
                    self.backend.minimize(*window);
                }
                let report = ToggleReport {
                    monitor,
                    action: ToggleAction::Minimize,
                    considered: current.len(),
                    affected: targets.len(),
                };
                self.store.save(monitor, current);
                report
            }
            PlanAction::Restore(targets) => {
                for window in &targets {
                    self.backend.restore(*window);
                }
                ToggleReport {
                    monitor,
                    action: ToggleAction::Restore,
                    considered: current.len(),
                    affected: targets.len(),
                }
            }
        }
    }

    /// 预览下一次按下的行为（只读）。
    pub fn preview_at_cursor(&self) -> Result<Preview, Error> {
        let monitor = self.backend.cursor_monitor().ok_or(Error::NoCursorMonitor)?;
        let windows = self.descriptors_on(monitor);
        let entries: Vec<WindowEntry> = windows
            .iter()
            .map(|window| WindowEntry { id: window.id, state: window.state })
            .collect();
        let action = plan::decide(&entries, self.store.get(monitor));

        Ok(Preview {
            monitor,
            monitor_info: self.backend.monitor_info(monitor),
            windows,
            action: match action {
                PlanAction::Minimize(_) => ToggleAction::Minimize,
                PlanAction::Restore(_) => ToggleAction::Restore,
            },
            affected: action.targets().len(),
        })
    }

    /// 清空所有记忆（显示器变化、程序退出前）。
    pub fn reset(&mut self) {
        self.store.clear();
    }

    fn entries_on(&self, monitor: MonitorId) -> Vec<WindowEntry> {
        self.backend
            .app_windows()
            .into_iter()
            .filter(|window| self.backend.window_monitor(*window) == Some(monitor))
            .map(|window| WindowEntry {
                id: window,
                state: self.backend.window_state(window),
            })
            .collect()
    }

    fn descriptors_on(&self, monitor: MonitorId) -> Vec<WindowDescriptor> {
        self.backend
            .app_windows()
            .into_iter()
            .filter(|window| self.backend.window_monitor(*window) == Some(monitor))
            .map(|window| WindowDescriptor {
                id: window,
                title: self.backend.window_title(window),
                state: self.backend.window_state(window),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::{Call, MemoryBackend};

    const MONITOR_MAIN: MonitorId = MonitorId(1);
    const MONITOR_SIDE: MonitorId = MonitorId(2);
    const A: WindowId = WindowId(0xA);
    const B: WindowId = WindowId(0xB);
    const C: WindowId = WindowId(0xC);
    const D: WindowId = WindowId(0xD);

    fn backend() -> MemoryBackend {
        let mut backend = MemoryBackend::new(MONITOR_MAIN);
        backend
            .add_window(A.0, MONITOR_MAIN, WindowState::Visible, "editor")
            .add_window(B.0, MONITOR_MAIN, WindowState::Visible, "browser")
            .add_window(C.0, MONITOR_SIDE, WindowState::Visible, "other monitor");
        backend
    }

    #[test]
    fn first_toggle_minimizes_only_the_cursor_monitor() {
        let mut show_desktop = ShowDesktop::new(backend());
        let report = show_desktop.toggle_at_cursor().unwrap();

        assert_eq!(report.action, ToggleAction::Minimize);
        assert_eq!(report.considered, 2);
        assert_eq!(report.affected, 2);
        assert_eq!(show_desktop.backend().calls(), vec![Call::Minimize(A), Call::Minimize(B)]);
        assert_eq!(show_desktop.backend().state_of(C), Some(WindowState::Visible));
    }

    #[test]
    fn second_toggle_restores_in_reverse_z_order() {
        let mut show_desktop = ShowDesktop::new(backend());
        show_desktop.toggle_at_cursor().unwrap();
        show_desktop.backend().clear_calls();

        let report = show_desktop.toggle_at_cursor().unwrap();
        assert_eq!(report.action, ToggleAction::Restore);
        assert_eq!(report.affected, 2);
        // 自下而上还原：B 先于 A。
        assert_eq!(show_desktop.backend().calls(), vec![Call::Restore(B), Call::Restore(A)]);
        assert_eq!(show_desktop.backend().state_of(A), Some(WindowState::Visible));
    }

    #[test]
    fn toggle_is_repeatable_after_restore() {
        let mut show_desktop = ShowDesktop::new(backend());
        show_desktop.toggle_at_cursor().unwrap();
        show_desktop.toggle_at_cursor().unwrap();
        let report = show_desktop.toggle_at_cursor().unwrap();
        assert_eq!(report.action, ToggleAction::Minimize);
        assert_eq!(report.affected, 2);
    }

    #[test]
    fn new_window_while_hidden_is_minimized_instead_of_restoring() {
        let mut show_desktop = ShowDesktop::new(backend());
        show_desktop.toggle_at_cursor().unwrap();
        show_desktop
            .backend_mut()
            .add_window(D.0, MONITOR_MAIN, WindowState::Visible, "new window");
        show_desktop.backend().clear_calls();

        let report = show_desktop.toggle_at_cursor().unwrap();
        assert_eq!(report.action, ToggleAction::Minimize);
        assert_eq!(report.considered, 3);
        assert_eq!(report.affected, 1);
        assert_eq!(show_desktop.backend().calls(), vec![Call::Minimize(D)]);
    }

    #[test]
    fn preview_does_not_change_state() {
        let show_desktop = ShowDesktop::new(backend());
        let preview = show_desktop.preview_at_cursor().unwrap();
        assert_eq!(preview.monitor, MONITOR_MAIN);
        assert_eq!(preview.windows.len(), 2);
        assert_eq!(preview.action, ToggleAction::Minimize);
        assert_eq!(preview.affected, 2);
        assert!(show_desktop.backend().calls().is_empty());
    }

    #[test]
    fn already_minimized_window_is_not_touched_on_restore() {
        let mut backend = MemoryBackend::new(MONITOR_MAIN);
        backend
            .add_window(A.0, MONITOR_MAIN, WindowState::Visible, "editor")
            .add_window(B.0, MONITOR_MAIN, WindowState::Minimized, "already minimized");
        let mut show_desktop = ShowDesktop::new(backend);

        show_desktop.toggle_at_cursor().unwrap();
        show_desktop.backend().clear_calls();
        show_desktop.toggle_at_cursor().unwrap();

        assert_eq!(show_desktop.backend().calls(), vec![Call::Restore(A)]);
        assert_eq!(show_desktop.backend().state_of(B), Some(WindowState::Minimized));
    }
}
