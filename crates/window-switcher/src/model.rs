//! 切换会话的纯状态：候选窗口、当前选择、视图模式与导航。
//!
//! 不依赖 Win32：窗口清单由后端枚举后填入，导航的“行”步长由布局给出，
//! 因此可以在任意平台单测。

use std::collections::HashMap;

use whelper_common::window::WindowId;

/// 一个可切换的窗口（枚举结果的一次快照）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEntry {
    pub id: WindowId,
    /// 所属应用的稳定标识（优先为可执行文件路径），用于图标视图合并同应用窗口。
    pub app_id: String,
    /// 窗口标题；空标题（只有图标的窗口）由界面回退成“未命名窗口”。
    pub title: String,
    pub minimized: bool,
}

impl WindowEntry {
    /// 界面上显示的名字：标题为空时给一个占位，避免出现空白卡片。
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            "未命名窗口"
        } else {
            &self.title
        }
    }
}

/// 视图模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// 缩略图：单行显示应用图标，同一应用的多个窗口合并为一个候选。
    Thumbnails,
    /// 网格图：每个窗口显示预览卡片，同一应用的多个窗口分别列出。
    Grid,
}

impl ViewMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Thumbnails => Self::Grid,
            Self::Grid => Self::Thumbnails,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Thumbnails => "缩略图视图",
            Self::Grid => "网格视图",
        }
    }
}

/// 一次选择移动。`Row*` 需要布局提供列数；单行应用图标条按 1 步移动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Prev,
    Next,
    RowPrev,
    RowNext,
    First,
    Last,
}

impl Step {
    /// 编码成 `MSG_STEP` 的 wparam：键盘钩子与覆盖窗口之间只传 `isize`。
    pub fn code(self) -> isize {
        match self {
            Self::Prev => 0,
            Self::Next => 1,
            Self::RowPrev => 2,
            Self::RowNext => 3,
            Self::First => 4,
            Self::Last => 5,
        }
    }

    pub fn from_code(code: isize) -> Option<Self> {
        match code {
            0 => Some(Self::Prev),
            1 => Some(Self::Next),
            2 => Some(Self::RowPrev),
            3 => Some(Self::RowNext),
            4 => Some(Self::First),
            5 => Some(Self::Last),
            _ => None,
        }
    }
}

/// 一次 Alt+Tab 会话：打开时的窗口快照、应用分组、当前候选和视图模式。
#[derive(Debug, Clone)]
pub struct Session {
    windows: Vec<WindowEntry>,
    /// 按窗口 Z 序排列的应用分组；每组首窗口作为该应用的激活目标。
    app_groups: Vec<Vec<usize>>,
    selected: usize,
    mode: ViewMode,
}

impl Session {
    /// 空列表返回 `None`：没有候选窗口时不应该弹出覆盖层。
    pub fn new(windows: Vec<WindowEntry>, mode: ViewMode) -> Option<Self> {
        if windows.is_empty() {
            return None;
        }
        let mut app_groups: Vec<Vec<usize>> = Vec::new();
        let mut group_by_app = HashMap::<String, usize>::new();
        for (index, window) in windows.iter().enumerate() {
            let key = if window.app_id.is_empty() {
                format!("window:{}", window.id)
            } else {
                window.app_id.to_lowercase()
            };
            if let Some(group) = group_by_app.get(&key).copied() {
                app_groups[group].push(index);
            } else {
                let group = app_groups.len();
                group_by_app.insert(key, group);
                app_groups.push(vec![index]);
            }
        }
        Some(Self {
            windows,
            app_groups,
            selected: 0,
            mode,
        })
    }

    /// 原始窗口快照，网格视图每项对应一个窗口。
    pub fn windows(&self) -> &[WindowEntry] {
        &self.windows
    }

    /// 当前模式下的候选数：图标视图按应用计数，网格视图按窗口计数。
    pub fn len(&self) -> usize {
        match self.mode {
            ViewMode::Thumbnails => self.app_groups.len(),
            ViewMode::Grid => self.windows.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 当前候选代表的窗口。图标视图选择同应用 Z 序最前的窗口作为激活目标。
    pub fn candidate_window(&self, index: usize) -> Option<&WindowEntry> {
        match self.mode {
            ViewMode::Thumbnails => self
                .app_groups
                .get(index)
                .and_then(|group| group.first())
                .and_then(|window| self.windows.get(*window)),
            ViewMode::Grid => self.windows.get(index),
        }
    }

    pub fn selected_window(&self) -> Option<&WindowEntry> {
        self.candidate_window(self.selected)
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ViewMode) {
        if self.mode == mode {
            return;
        }
        let selected_id = self.selected_window().map(|window| window.id);
        self.mode = mode;
        self.selected = selected_id
            .and_then(|id| match mode {
                ViewMode::Thumbnails => self.app_groups.iter().position(|group| {
                    group.iter().any(|index| {
                        self.windows
                            .get(*index)
                            .is_some_and(|window| window.id == id)
                    })
                }),
                ViewMode::Grid => self.windows.iter().position(|window| window.id == id),
            })
            .unwrap_or(0);
    }

    pub fn toggle_mode(&mut self) -> ViewMode {
        self.set_mode(self.mode.toggled());
        self.mode
    }

    pub fn select(&mut self, index: usize) {
        if index < self.len() {
            self.selected = index;
        }
    }

    /// 移动选择。越界一律环绕：Alt+Tab 的习惯是转一圈回到起点。
    pub fn apply(&mut self, step: Step, columns: usize) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let len_isize = len as isize;
        let current = self.selected as isize;
        let columns = columns.max(1) as isize;
        let next = match step {
            Step::Prev => current - 1,
            Step::Next => current + 1,
            Step::RowPrev => current - columns,
            Step::RowNext => current + columns,
            Step::First => 0,
            Step::Last => len_isize - 1,
        };
        self.selected = next.rem_euclid(len_isize) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: isize, title: &str) -> WindowEntry {
        WindowEntry {
            id: WindowId(id),
            app_id: format!("app-{id}"),
            title: title.to_string(),
            minimized: false,
        }
    }

    fn session() -> Session {
        Session::new(
            vec![
                entry(1, "one"),
                entry(2, "two"),
                entry(3, "three"),
                entry(4, "four"),
            ],
            ViewMode::Thumbnails,
        )
        .expect("非空窗口列表")
    }

    #[test]
    fn empty_window_list_has_no_session() {
        assert!(Session::new(Vec::new(), ViewMode::Grid).is_none());
    }

    #[test]
    fn selection_starts_at_the_foreground_window() {
        let session = session();
        assert_eq!(session.selected(), 0);
        assert_eq!(session.selected_window().map(|w| w.id), Some(WindowId(1)));
    }

    #[test]
    fn horizontal_steps_wrap_around() {
        let mut session = session();
        session.apply(Step::Next, 1);
        assert_eq!(session.selected(), 1);
        session.apply(Step::Prev, 1);
        assert_eq!(session.selected(), 0);
        // 从第一个再往前：绕到最后一个。
        session.apply(Step::Prev, 1);
        assert_eq!(session.selected(), 3);
        session.apply(Step::Next, 1);
        assert_eq!(session.selected(), 0);
    }

    #[test]
    fn row_steps_use_the_grid_columns() {
        let mut session = session();
        session.apply(Step::RowNext, 2);
        assert_eq!(session.selected(), 2);
        session.apply(Step::RowPrev, 2);
        assert_eq!(session.selected(), 0);
        // 列数大于窗口数时也要环绕而不是 panic。
        session.apply(Step::RowPrev, 9);
        assert_eq!(session.selected(), 3);
    }

    #[test]
    fn first_and_last_jump_to_the_ends() {
        let mut session = session();
        session.apply(Step::Last, 1);
        assert_eq!(session.selected(), 3);
        session.apply(Step::First, 1);
        assert_eq!(session.selected(), 0);
    }

    #[test]
    fn select_out_of_range_is_ignored() {
        let mut session = session();
        session.select(2);
        assert_eq!(session.selected(), 2);
        session.select(99);
        assert_eq!(session.selected(), 2);
    }

    #[test]
    fn mode_toggles_and_survives_in_the_session() {
        let mut session = session();
        assert_eq!(session.mode(), ViewMode::Thumbnails);
        assert_eq!(session.toggle_mode(), ViewMode::Grid);
        assert_eq!(session.mode(), ViewMode::Grid);
        assert_eq!(session.toggle_mode(), ViewMode::Thumbnails);
    }

    #[test]
    fn step_codes_round_trip() {
        for step in [
            Step::Prev,
            Step::Next,
            Step::RowPrev,
            Step::RowNext,
            Step::First,
            Step::Last,
        ] {
            assert_eq!(Step::from_code(step.code()), Some(step));
        }
        assert_eq!(Step::from_code(42), None);
    }

    #[test]
    fn empty_title_falls_back_to_placeholder() {
        assert_eq!(entry(1, "").display_title(), "未命名窗口");
        assert_eq!(entry(1, "   ").display_title(), "未命名窗口");
        assert_eq!(entry(1, "记事本").display_title(), "记事本");
    }
}
