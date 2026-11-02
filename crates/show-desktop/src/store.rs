//! 按显示器记忆「最小化之前的窗口清单」。
//!
//! 上游用 `List<DesktopWindowID>[] PrevStateByScreen`（下标 = 屏幕序号）；这里换成
//! 以 [`MonitorId`] 为键的哈希表，显示器增减不会让记忆错位。

use std::collections::HashMap;

use whelper_common::geometry::MonitorId;

use crate::window::{WindowEntry, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedState {
    pub windows: Vec<WindowEntry>,
    pub foreground: Option<WindowId>,
}

#[derive(Debug, Default)]
pub struct StateStore {
    saved: HashMap<MonitorId, SavedState>,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出并清除某个显示器的窗口与前台记忆。旧记忆只参与一次切换决策。
    pub fn take(&mut self, monitor: MonitorId) -> Option<SavedState> {
        self.saved.remove(&monitor)
    }

    /// 只读查看，供预览（不改状态）使用。
    pub fn get(&self, monitor: MonitorId) -> Option<&SavedState> {
        self.saved.get(&monitor)
    }

    pub fn save(
        &mut self,
        monitor: MonitorId,
        windows: Vec<WindowEntry>,
        foreground: Option<WindowId>,
    ) {
        self.saved.insert(monitor, SavedState { windows, foreground });
    }

    pub fn clear(&mut self) {
        self.saved.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.saved.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::{WindowId, WindowState};

    const MONITOR_A: MonitorId = MonitorId(1);
    const MONITOR_B: MonitorId = MonitorId(2);

    fn snapshot() -> Vec<WindowEntry> {
        vec![WindowEntry { id: WindowId(7), state: WindowState::Visible }]
    }

    #[test]
    fn take_removes_the_entry() {
        let mut store = StateStore::new();
        store.save(MONITOR_A, snapshot(), None);
        assert!(store.get(MONITOR_A).is_some());
        assert_eq!(
            store.take(MONITOR_A),
            Some(SavedState { windows: snapshot(), foreground: None })
        );
        assert!(store.get(MONITOR_A).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn monitors_are_independent() {
        let mut store = StateStore::new();
        store.save(MONITOR_A, snapshot(), None);
        store.save(MONITOR_B, Vec::new(), None);
        assert_eq!(store.get(MONITOR_A).map(|state| state.windows.len()), Some(1));
        assert_eq!(store.get(MONITOR_B).map(|state| state.windows.len()), Some(0));
        assert_eq!(
            store.take(MONITOR_B),
            Some(SavedState { windows: Vec::new(), foreground: None })
        );
        assert!(store.get(MONITOR_A).is_some());
    }

    #[test]
    fn take_on_unknown_monitor_returns_none() {
        assert_eq!(StateStore::new().take(MONITOR_A), None);
    }
}
