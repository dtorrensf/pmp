// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared task-list component.
//!
//! The component is split by responsibility:
//!
//! - [`state`] holds the data, filters, order, selection and scroll, with no
//!   Ratatui, keyboard or repository knowledge.
//! - [`keys`] translates key presses into state changes or intents.
//! - [`table`] and [`popups`] render the table, its caption and the modals.
//! - This module is the facade the embedding screens use: it owns the modal
//!   mode, performs the repository queries needed to build dialogs and
//!   forwards rendering.
//!
//! Embedding screens keep only their screen-level keys (Quit, Refresh,
//! New Task, Back, ...) and delegate everything else to this view via
//! [`TaskListView::handle_key`]. The view's keys are:
//!
//! - `j`/`k` or `Up`/`Down`: move the selection
//! - `Enter`: edit the selected task
//! - `Tab`: cycle the selected task's status
//! - `d`: delete the selected task (with confirmation)
//! - `f`: filter the list
//! - `o`: cycle the sort order (priority, then id ascending and descending)
//! - `x`: clear every active filter
//!
//! The move popup is opened by the embedding screen through
//! [`TaskListView::open_move`] because only the screen knows the other
//! versions of the project.

use std::collections::HashMap;

use crossterm::event::KeyCode;
use ratatui::{Frame, layout::Rect, text::Line};

use crate::application;
use crate::db::repository::Repository;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::version::Version;
use crate::screens::components::confirm_dialog::ConfirmDialog;
use crate::screens::{Action, UiContext};

mod keys;
mod modes;
mod popups;
mod state;
mod table;

pub use modes::{FilterField, FilterState, MoveTaskState, TaskListMode};
pub use state::{Filters, Selection, SortMode};
pub use table::{DependencySummary, truncate_display};

use keys::BrowsingOutcome;
use state::TaskListState;

/// Task-list component: the pure list state plus the modal mode in progress
/// and the dependency summaries shown next to each name.
pub struct TaskListView {
    state: TaskListState,
    mode: TaskListMode,
    /// Dependency summary of each task, keyed by task id.
    dependency_summaries: HashMap<i64, DependencySummary>,
}

impl TaskListView {
    pub fn new() -> Self {
        Self {
            state: TaskListState::new(),
            mode: TaskListMode::Browsing,
            dependency_summaries: HashMap::new(),
        }
    }

    /// The currently visible (filtered) task list.
    pub fn tasks(&self) -> &[Task] {
        self.state.tasks()
    }

    /// The complete task list, ignoring the active filters. Intended for
    /// project-wide aggregates (e.g. dashboard charts) that must reflect
    /// the whole project regardless of the list filter.
    pub fn all_tasks(&self) -> &[Task] {
        self.state.all_tasks()
    }

    pub fn selected(&self) -> usize {
        self.state.selected()
    }

    pub fn set_selected(&mut self, index: usize) {
        self.state.set_selected(index);
    }

    pub fn filter_status(&self) -> Option<Status> {
        self.state.filters().status
    }

    pub fn filter_priority(&self) -> Option<Priority> {
        self.state.filters().priority
    }

    pub fn filter_title(&self) -> Option<&str> {
        self.state.filters().title.as_deref()
    }

    pub fn sort_mode(&self) -> SortMode {
        self.state.sort_mode()
    }

    pub fn mode(&self) -> &TaskListMode {
        &self.mode
    }

    pub fn set_mode(&mut self, mode: TaskListMode) {
        self.mode = mode;
    }

    pub fn is_browsing(&self) -> bool {
        matches!(self.mode, TaskListMode::Browsing)
    }

    /// Opens the move popup for the selected task, offering `targets` as
    /// destination versions. Does nothing when no task is selected.
    pub fn open_move(&mut self, targets: Vec<Version>) {
        if let Some(task_id) = self.state.selected_task().and_then(|task| task.id) {
            self.mode = TaskListMode::MovingTask(MoveTaskState {
                task_id,
                targets,
                selected: 0,
            });
        }
    }

    /// Replace the source list, re-apply the active filters and clamp the
    /// selection. The UI mode is intentionally left untouched so that data
    /// reloads do not destroy open dialogs.
    pub fn set_tasks(&mut self, tasks: Vec<Task>) {
        self.state.set_tasks(tasks);
    }

    /// Dependency summary of each task, keyed by task id. Shown as a compact
    /// indicator next to the task name. Missing ids show no indicator.
    pub fn set_dependency_summaries(&mut self, summaries: HashMap<i64, DependencySummary>) {
        self.dependency_summaries = summaries;
    }

    /// Handle a key press for the current mode.
    ///
    /// Returns `Some(Action)` when the view emits an app action (navigate to
    /// the edit form, update a task status, delete a task); `None` when the
    /// key was handled internally or is not owned by the view. Screen-level
    /// keys (e.g. `q`, `r`, `c`, `e`, `Esc` while browsing) stay with the
    /// embedding screen, which should consult [`TaskListView::is_browsing`]
    /// before applying them.
    pub fn handle_key(&mut self, code: KeyCode, repo: &dyn Repository) -> Option<Action> {
        if self.is_browsing() {
            return self.browsing_key(code, repo);
        }
        // Take the modal mode out so a controller can hand it back while the
        // state is borrowed mutably at the same time.
        let mut mode = std::mem::replace(&mut self.mode, TaskListMode::Browsing);
        let action = if matches!(mode, TaskListMode::ConfirmDelete(_)) {
            keys::confirm_delete(&mut mode, code)
        } else if matches!(mode, TaskListMode::MovingTask(_)) {
            keys::moving(&mut mode, code)
        } else {
            keys::filtering(&mut self.state, &mut mode, code)
        };
        self.mode = mode;
        action
    }

    /// Browsing keys, plus the queries needed to build their dialogs. The
    /// repository access stays here, in the screen flow, and not in the state
    /// or the key controllers.
    fn browsing_key(&mut self, code: KeyCode, repo: &dyn Repository) -> Option<Action> {
        match keys::browsing(&mut self.state, code) {
            BrowsingOutcome::Action(action) => Some(action),
            BrowsingOutcome::DeleteRequested { task_id, name } => {
                self.open_delete_confirm(task_id, &name, repo)
            }
            BrowsingOutcome::OpenFilter => {
                self.mode =
                    TaskListMode::Filtering(FilterState::from_filters(self.state.filters()));
                None
            }
            BrowsingOutcome::Handled | BrowsingOutcome::Ignored => None,
        }
    }

    /// Asks for the dependency impact of deleting the task and opens the
    /// confirmation with the matching message.
    ///
    /// If the query fails the dialog is *not* opened: confirming a destructive
    /// action without knowing its impact is worse than showing the failure.
    /// The error travels to the app as [`Action::ReportError`].
    fn open_delete_confirm(
        &mut self,
        task_id: i64,
        name: &str,
        repo: &dyn Repository,
    ) -> Option<Action> {
        let dependents_count = match application::dependencies::dependents(repo, task_id) {
            Ok(dependents) => dependents.len(),
            Err(error) => {
                return Some(Action::ReportError {
                    message: error.user_message(),
                });
            }
        };
        let message = if dependents_count > 0 {
            format!(
                "Deleting will remove {} dependency links. Continue?",
                dependents_count
            )
        } else {
            format!("Delete task {name}?")
        };
        self.mode = TaskListMode::ConfirmDelete(
            ConfirmDialog::new(message, Action::DeleteTask { task_id }).with_title("Delete task"),
        );
        None
    }

    /// Caption for the enclosing block: one chip per active filter, the sort
    /// direction and, while filters are active, how to clear them.
    pub fn table_title(&self, ui: &UiContext) -> Line<'static> {
        table::title(&self.state, ui)
    }

    /// Render the task table (browsing) or the active modal popup. The table
    /// stays visible, dimmed, behind the delete confirmation.
    pub fn render(&self, frame: &mut Frame, area: Rect, ui: &UiContext) {
        match &self.mode {
            TaskListMode::Browsing => {
                table::render(&self.state, &self.dependency_summaries, frame, area, ui);
            }
            TaskListMode::ConfirmDelete(dialog) => {
                table::render(&self.state, &self.dependency_summaries, frame, area, ui);
                // Center the modal on the whole screen, not just on the table.
                dialog.render(frame, frame.area(), ui);
            }
            TaskListMode::Filtering(state) => popups::render_filter(state, frame, area, ui),
            TaskListMode::MovingTask(state) => popups::render_move(state, frame, area, ui),
        }
    }
}

impl Default for TaskListView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
