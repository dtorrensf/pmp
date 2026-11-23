// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pure state of the task list: data, filters, order, selection and scroll.
//!
//! Nothing here knows about Ratatui, keyboard codes, dialogs or the
//! repository, so the rules that are easiest to get wrong (which tasks are
//! visible, in which order, and which one stays selected after a reload) can be
//! tested without a terminal.

use std::ops::Range;

use crate::domain::task::{Priority, Status, Task};
use crate::screens::components::list_navigation;
use crate::screens::components::scroll::ListScroll;

/// Order of the visible task list. `Priority` keeps the order given by the
/// repository; the ID modes sort in memory by the numeric task id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Priority,
    IdAsc,
    IdDesc,
}

impl SortMode {
    /// The next mode in the cycle Priority -> IdAsc -> IdDesc -> Priority.
    pub fn next(self) -> Self {
        match self {
            SortMode::Priority => SortMode::IdAsc,
            SortMode::IdAsc => SortMode::IdDesc,
            SortMode::IdDesc => SortMode::Priority,
        }
    }
}

/// The active filters, as applied to the visible list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// Already normalized: trimmed and lowercased.
    pub title: Option<String>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.status.is_none() && self.priority.is_none()
    }

    fn matches(&self, task: &Task) -> bool {
        self.status.is_none_or(|status| task.status == status)
            && self
                .priority
                .is_none_or(|priority| task.priority == priority)
            && self
                .title
                .as_ref()
                .is_none_or(|title| task.name.to_lowercase().contains(title))
    }
}

/// Sorts `tasks` by id for the ID modes. Tasks without an id go last in both
/// directions so the order is total. The priority mode leaves the list as is.
fn sort_by_id(tasks: &mut [Task], mode: SortMode) {
    let descending = match mode {
        SortMode::Priority => return,
        SortMode::IdAsc => false,
        SortMode::IdDesc => true,
    };
    tasks.sort_by(|a, b| match (a.id, b.id) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) if descending => y.cmp(&x),
        (Some(x), Some(y)) => x.cmp(&y),
    });
}

/// Direction of a selection move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Up,
    Down,
}

/// Everything the list needs to show its rows, minus the presentation.
pub struct TaskListState {
    all_tasks: Vec<Task>,
    tasks: Vec<Task>,
    selected: usize,
    scroll: ListScroll,
    filters: Filters,
    sort_mode: SortMode,
}

impl TaskListState {
    pub fn new() -> Self {
        Self {
            all_tasks: Vec::new(),
            tasks: Vec::new(),
            selected: 0,
            scroll: ListScroll::new(),
            filters: Filters::default(),
            sort_mode: SortMode::default(),
        }
    }

    /// The currently visible (filtered) task list.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The complete task list, ignoring the active filters. Intended for
    /// project-wide aggregates that must reflect the whole project.
    pub fn all_tasks(&self) -> &[Task] {
        &self.all_tasks
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn set_selected(&mut self, index: usize) {
        self.selected = index;
    }

    pub fn filters(&self) -> &Filters {
        &self.filters
    }

    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.tasks.get(self.selected)
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Row indices to render for a viewport of `height` data rows, keeping the
    /// selection visible.
    pub fn visible_range(&self, height: usize) -> Range<usize> {
        self.scroll
            .visible_range(self.tasks.len(), self.selected, height)
    }

    /// Replace the source list, re-apply the active filters and clamp the
    /// selection. The visible selection is kept by position, not by task, so a
    /// reload does not jump to another row.
    pub fn set_tasks(&mut self, tasks: Vec<Task>) {
        self.all_tasks = tasks;
        self.apply_filter();
    }

    /// Replace every filter and re-apply them.
    pub fn set_filters(&mut self, filters: Filters) {
        self.filters = filters;
        self.apply_filter();
    }

    /// Drops every active filter at once.
    pub fn clear_filters(&mut self) {
        self.filters = Filters::default();
        self.apply_filter();
    }

    /// Advances the sort mode and keeps the same task selected, found by id.
    pub fn cycle_sort(&mut self) {
        let selected_id = self.selected_task().and_then(|task| task.id);
        self.sort_mode = self.sort_mode.next();
        self.apply_filter();
        if let Some(index) =
            selected_id.and_then(|id| self.tasks.iter().position(|task| task.id == Some(id)))
        {
            self.selected = index;
        }
    }

    /// Moves the selection one row up or down, clamped to the visible rows.
    pub fn move_selection(&mut self, direction: Selection) {
        let code = match direction {
            Selection::Up => crossterm::event::KeyCode::Up,
            Selection::Down => crossterm::event::KeyCode::Down,
        };
        list_navigation::move_selection(&mut self.selected, self.tasks.len(), code);
    }

    fn apply_filter(&mut self) {
        let mut tasks: Vec<Task> = self
            .all_tasks
            .iter()
            .filter(|task| self.filters.matches(task))
            .cloned()
            .collect();
        sort_by_id(&mut tasks, self.sort_mode);
        self.tasks = tasks;
        self.selected = self.selected.min(self.tasks.len().saturating_sub(1));
    }
}

impl Default for TaskListState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Filters, Selection, SortMode, TaskListState};
    use crate::domain::task::{Priority, Status, Task};

    fn task(id: Option<i64>, name: &str, status: Status, priority: Priority) -> Task {
        Task {
            id,
            project_id: 1,
            version_id: 1,
            name: name.to_string(),
            description: String::new(),
            status,
            priority,
            created_at: String::new(),
            completed_at: None,
        }
    }

    fn sample() -> Vec<Task> {
        vec![
            task(Some(3), "Write docs", Status::Todo, Priority::High),
            task(Some(1), "Fix bug", Status::Done, Priority::Low),
            task(Some(2), "Review PR", Status::InProgress, Priority::Medium),
            task(None, "Unsaved", Status::Todo, Priority::Low),
        ]
    }

    fn state_with(filters: Filters) -> TaskListState {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        state.set_filters(filters);
        state
    }

    #[test]
    fn filtering_by_title_is_case_insensitive_and_partial() {
        let state = state_with(Filters {
            title: Some("pr".to_string()),
            ..Filters::default()
        });
        assert_eq!(state.tasks().len(), 1);
        assert_eq!(state.tasks()[0].name, "Review PR");
    }

    #[test]
    fn filters_combine() {
        let state = state_with(Filters {
            title: Some("r".to_string()),
            status: Some(Status::Todo),
            priority: Some(Priority::High),
        });
        assert_eq!(state.tasks().len(), 1);
        assert_eq!(state.tasks()[0].name, "Write docs");
    }

    #[test]
    fn all_tasks_ignores_the_active_filter() {
        let state = state_with(Filters {
            status: Some(Status::Done),
            ..Filters::default()
        });
        assert_eq!(state.tasks().len(), 1);
        assert_eq!(state.all_tasks().len(), 4);
    }

    #[test]
    fn clearing_filters_restores_every_row() {
        let mut state = state_with(Filters {
            title: Some("docs".to_string()),
            status: Some(Status::Done),
            priority: Some(Priority::Low),
        });
        assert!(state.tasks().is_empty());
        state.clear_filters();
        assert_eq!(state.tasks().len(), 4);
    }

    #[test]
    fn id_sort_modes_order_by_id_and_put_missing_ids_last() {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        assert_eq!(state.sort_mode(), SortMode::Priority);
        assert_eq!(state.tasks()[0].name, "Write docs");

        state.cycle_sort();
        assert_eq!(state.sort_mode(), SortMode::IdAsc);
        let ids: Vec<Option<i64>> = state.tasks().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![Some(1), Some(2), Some(3), None]);

        state.cycle_sort();
        assert_eq!(state.sort_mode(), SortMode::IdDesc);
        let ids: Vec<Option<i64>> = state.tasks().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![Some(3), Some(2), Some(1), None]);
    }

    #[test]
    fn cycling_the_sort_keeps_the_same_task_selected() {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        // Select "Review PR" (id 2), third row in repository order.
        state.set_selected(2);
        let before = state.selected_task().unwrap().id;

        state.cycle_sort();
        state.cycle_sort();

        assert_eq!(state.selected_task().unwrap().id, before);
    }

    #[test]
    fn id_sort_combines_with_filters() {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        state.set_filters(Filters {
            status: Some(Status::Todo),
            ..Filters::default()
        });
        state.cycle_sort();
        let names: Vec<&str> = state.tasks().iter().map(|t| t.name.as_str()).collect();
        // Tasks without an id go last in both id directions.
        assert_eq!(names, vec!["Write docs", "Unsaved"]);
    }

    #[test]
    fn reloading_clamps_the_selection_to_the_new_list() {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        state.set_selected(3);
        state.set_tasks(sample()[..2].to_vec());
        assert_eq!(state.selected(), 1);
        assert_eq!(state.selected_task().unwrap().name, "Fix bug");
    }

    #[test]
    fn selection_never_moves_past_the_last_row_with_the_keyboard() {
        let mut state = TaskListState::new();
        state.set_tasks(sample());
        state.set_selected(3);
        state.move_selection(Selection::Down);
        assert_eq!(state.selected(), 3);
        state.move_selection(Selection::Up);
        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn an_empty_list_keeps_the_selection_at_zero() {
        let mut state = TaskListState::new();
        state.set_tasks(Vec::new());
        assert!(state.is_empty());
        assert_eq!(state.selected(), 0);
        assert!(state.selected_task().is_none());
    }

    #[test]
    fn visible_range_follows_the_selection() {
        let mut state = TaskListState::new();
        state.set_tasks(
            (0..30)
                .map(|i| task(Some(i), &format!("T{i}"), Status::Todo, Priority::Low))
                .collect(),
        );
        // First frame: the viewport starts at the top.
        assert_eq!(state.visible_range(10), 0..10);
        state.set_selected(15);
        let range = state.visible_range(10);
        assert!(
            range.contains(&15),
            "selection must stay visible: {range:?}"
        );
    }
}
