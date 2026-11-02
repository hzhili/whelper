//! 切换决策（纯逻辑，无 Win32）。
//!
//! 与上游 `MainAppContext.OnShowDesktopKeyComb` 的三步一一对应：
//!
//! 1. 收集光标所在显示器上的窗口及其状态；
//! 2. 若「当前没有一个窗口是可见的」且「记忆清单与当前清单只差状态」，
//!    说明这是同一次隐藏之后的第二次按下 → 还原；
//! 3. 否则最小化当前可见的窗口，并把本次清单记为记忆状态。

use std::collections::HashMap;

use crate::window::{WindowEntry, WindowId, WindowState};

/// 执行计划。`targets` 已经是**应用顺序**，调用方按顺序执行即可。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 自上而下最小化。顺序沿用枚举顺序（`EnumWindows` 为 Z 序自上而下），
    /// 等价于上游按 `GetWindowZOrder` 降序排序。
    Minimize(Vec<WindowId>),
    /// 自下而上还原。上游是 `PrevState.Reverse()`，这样最后还原的窗口在最上层。
    Restore(Vec<WindowId>),
}

impl Action {
    pub fn targets(&self) -> &[WindowId] {
        match self {
            Self::Minimize(targets) | Self::Restore(targets) => targets,
        }
    }
}

/// 决定这次按下该做什么。
///
/// `current` 必须按 Z 序自上而下排列（`EnumWindows` 的自然顺序）；`remembered` 是上次
/// 最小化时存下的清单（同样按 Z 序）。
pub fn decide(current: &[WindowEntry], remembered: Option<&[WindowEntry]>) -> Action {
    let all_hidden = current.iter().all(|entry| entry.state != WindowState::Visible);
    let restore = all_hidden
        && remembered.is_some_and(|previous| differs_only_by_state(current, previous));

    if restore {
        let previous = remembered.expect("restore 分支已确认记忆存在");
        Action::Restore(
            previous
                .iter()
                .rev()
                .filter(|entry| entry.state == WindowState::Visible)
                .map(|entry| entry.id)
                .collect(),
        )
    } else {
        Action::Minimize(
            current
                .iter()
                .filter(|entry| entry.state == WindowState::Visible)
                .map(|entry| entry.id)
                .collect(),
        )
    }
}

/// 清单是同一批窗口、但至少一个窗口的状态变了。
///
/// 对应上游 `DoesPrevStateDiffersOnlyByWindowsStyle`：数量相同 → 互相包含 → 且存在状态差异。
/// 最后一条很关键：状态完全一致时不做还原，避免“已经最小化过”的清单反复触发还原。
fn differs_only_by_state(current: &[WindowEntry], previous: &[WindowEntry]) -> bool {
    if current.len() != previous.len() {
        return false;
    }
    let previous: HashMap<WindowId, WindowState> =
        previous.iter().map(|entry| (entry.id, entry.state)).collect();
    if !current.iter().all(|entry| previous.contains_key(&entry.id)) {
        return false;
    }
    current
        .iter()
        .any(|entry| previous.get(&entry.id) != Some(&entry.state))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowId = WindowId(0xA);
    const B: WindowId = WindowId(0xB);
    const C: WindowId = WindowId(0xC);

    fn entry(id: WindowId, state: WindowState) -> WindowEntry {
        WindowEntry { id, state }
    }

    #[test]
    fn first_press_minimizes_visible_windows_in_z_order() {
        let current = [
            entry(A, WindowState::Visible),
            entry(B, WindowState::Minimized),
            entry(C, WindowState::Visible),
        ];
        assert_eq!(decide(&current, None), Action::Minimize(vec![A, C]));
    }

    #[test]
    fn already_minimized_windows_are_not_minimized_again() {
        let current = [entry(A, WindowState::Minimized), entry(B, WindowState::Disabled)];
        assert_eq!(decide(&current, None), Action::Minimize(vec![]));
    }

    #[test]
    fn second_press_restores_remembered_visible_windows_bottom_up() {
        let remembered = [
            entry(A, WindowState::Visible),
            entry(B, WindowState::Minimized),
            entry(C, WindowState::Visible),
        ];
        let current = [
            entry(A, WindowState::Minimized),
            entry(B, WindowState::Minimized),
            entry(C, WindowState::Minimized),
        ];
        // 记忆里 B 本来就是最小化 → 还原时不能碰；顺序是 C 再 A（自下而上）。
        assert_eq!(decide(&current, Some(&remembered)), Action::Restore(vec![C, A]));
    }

    #[test]
    fn identical_state_does_not_trigger_restore() {
        let previous = [entry(A, WindowState::Minimized), entry(B, WindowState::Disabled)];
        let current = [entry(A, WindowState::Minimized), entry(B, WindowState::Disabled)];
        // 没有状态差异 → 与上游一样走最小化分支（此时没有可最小化的窗口）。
        assert_eq!(decide(&current, Some(&previous)), Action::Minimize(vec![]));
    }

    #[test]
    fn window_opened_while_hidden_forces_minimize_again() {
        let previous = [entry(A, WindowState::Minimized)];
        let current = [entry(A, WindowState::Minimized), entry(C, WindowState::Visible)];
        assert_eq!(decide(&current, Some(&previous)), Action::Minimize(vec![C]));
    }

    #[test]
    fn window_closed_while_hidden_forces_minimize_again() {
        let previous = [entry(A, WindowState::Minimized), entry(B, WindowState::Minimized)];
        let current = [entry(A, WindowState::Minimized)];
        assert_eq!(decide(&current, Some(&previous)), Action::Minimize(vec![]));
    }

    #[test]
    fn different_window_set_with_same_count_is_not_a_restore() {
        let previous = [entry(A, WindowState::Visible), entry(B, WindowState::Visible)];
        let current = [entry(A, WindowState::Minimized), entry(C, WindowState::Minimized)];
        assert_eq!(decide(&current, Some(&previous)), Action::Minimize(vec![]));
    }

    #[test]
    fn any_visible_window_prevents_restore() {
        let previous = [entry(A, WindowState::Visible)];
        let current = [entry(A, WindowState::Minimized), entry(B, WindowState::Visible)];
        assert_eq!(decide(&current, Some(&previous)), Action::Minimize(vec![B]));
    }
}
