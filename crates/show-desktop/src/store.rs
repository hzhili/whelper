//! 按显示器记忆「最小化之前的窗口清单」。
//!
//! 上游用 `List<DesktopWindowID>[] PrevStateByScreen`（下标 = 屏幕序号）；这里换成
//! 以 [`MonitorId`] 为键的哈希表，显示器增减不会让记忆错位。

use std::collections::HashMap;

use whelper_common::geometry::MonitorId;

use crate::window::WindowEntry;

#[derive(Debug, Default)]
pub struct StateStore {
    saved: HashMap<MonitorId, Vec<WindowEntry>>,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出并清除某个显示器的记忆。切换时无论走向哪条分支，旧记忆都不再有效。
    pub fn take(&mut self, monitor: MonitorId) -> Option<Vec<WindowEntry>> {
        self.saved.remove(&monitor)
    }

    /// 只读查看，供预览（不改状态）使用。
    pub fn get(&self, monitor: MonitorId) -> Option<&[WindowEntry]> {
        self.saved.get(&monitor).map(Vec::as_slice)
    }

    pub fn save(&mut self, monitor: MonitorId, snapshot: Vec<WindowEntry>) {
        self.saved.insert(monitor, snapshot);
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
        store.save(MONITOR_A, snapshot());
        assert!(store.get(MONITOR_A).is_some());
        assert_eq!(store.take(MONITOR_A), Some(snapshot()));
        assert!(store.get(MONITOR_A).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn monitors_are_independent() {
        let mut store = StateStore::new();
        store.save(MONITOR_A, snapshot());
        store.save(MONITOR_B, Vec::new());
        assert_eq!(store.get(MONITOR_A).map(<[WindowEntry]>::len), Some(1));
        assert_eq!(store.get(MONITOR_B).map(<[WindowEntry]>::len), Some(0));
        assert_eq!(store.take(MONITOR_B), Some(Vec::new()));
        assert!(store.get(MONITOR_A).is_some());
    }

    #[test]
    fn take_on_unknown_monitor_returns_none() {
        assert_eq!(StateStore::new().take(MONITOR_A), None);
    }
}
