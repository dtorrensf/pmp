// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::db::repository::Repository;
use crate::domain::task::{Priority, Status, Task};
use crate::screens::components::confirm_dialog::ConfirmDialog;
use crate::screens::components::fields::render_readonly_field;
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};
use crate::screens::{Action, Screen};

#[derive(Debug, Clone)]
pub enum TaskListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
    Filtering(FilterState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Status,
    Priority,
}

#[derive(Debug, Clone, Copy)]
pub struct FilterState {
    pub filter_status: Option<Status>,
    pub filter_priority: Option<Priority>,
    pub selected_field: FilterField,
}

/// Shared task-list component: owns the task data, selection, active filters
/// and modal state (confirm-delete / filtering), handles the task-list keys
/// and renders the table or the active popup.
///
/// Embedding screens keep only their screen-level keys (Quit, Refresh,
/// New Task, Back, ...) and delegate everything else to this view via
/// [`TaskListView::handle_key`]. The view's keys are:
///
/// - `j`/`k` or `Up`/`Down`: move the selection
/// - `Enter`: edit the selected task
/// - `Tab`: cycle the selected task's status
/// - `d`: delete the selected task (with confirmation)
/// - `f`: filter the list
pub struct TaskListView {
    all_tasks: Vec<Task>,
    tasks: Vec<Task>,
    selected: usize,
    filter_status: Option<Status>,
    filter_priority: Option<Priority>,
    mode: TaskListMode,
}

impl TaskListView {
    pub fn new() -> Self {
        Self {
            all_tasks: Vec::new(),
            tasks: Vec::new(),
            selected: 0,
            filter_status: None,
            filter_priority: None,
            mode: TaskListMode::Browsing,
        }
    }

    /// The currently visible (filtered) task list.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The complete task list, ignoring the active filters. Intended for
    /// project-wide aggregates (e.g. dashboard charts) that must reflect
    /// the whole project regardless of the list filter.
    pub fn all_tasks(&self) -> &[Task] {
        &self.all_tasks
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn set_selected(&mut self, index: usize) {
        self.selected = index;
    }

    pub fn filter_status(&self) -> Option<Status> {
        self.filter_status
    }

    pub fn filter_priority(&self) -> Option<Priority> {
        self.filter_priority
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

    /// Replace the source list, re-apply the active filters and clamp the
    /// selection. The UI mode is intentionally left untouched so that data
    /// reloads do not destroy open dialogs.
    pub fn set_tasks(&mut self, tasks: Vec<Task>) {
        self.all_tasks = tasks;
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        self.tasks = self
            .all_tasks
            .iter()
            .filter(|task| {
                self.filter_status
                    .is_none_or(|status| task.status == status)
                    && self
                        .filter_priority
                        .is_none_or(|priority| task.priority == priority)
            })
            .cloned()
            .collect();
        self.selected = self.selected.min(self.tasks.len().saturating_sub(1));
    }

    /// Handle a key press for the current mode.
    ///
    /// Returns `Some(Action)` when the view emits an app action (navigate to
    /// the edit form, update a task status, delete a task); `None` when the
    /// key was handled internally or is not owned by the view. Screen-level
    /// keys (e.g. `q`, `r`, `c`, `e`, `Esc` while browsing) stay with the
    /// embedding screen, which should consult [`TaskListView::is_browsing`]
    /// before applying them.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        project_id: i64,
        repo: &dyn Repository,
    ) -> Option<Action> {
        match &mut self.mode {
            TaskListMode::Browsing => match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected + 1 < self.tasks.len() {
                        self.selected += 1;
                    }
                    None
                }
                KeyCode::Enter => self.tasks.get(self.selected).map(|task| {
                    Action::Navigate(Screen::TaskForm {
                        project_id,
                        task_id: task.id,
                    })
                }),
                KeyCode::Tab => {
                    self.tasks
                        .get(self.selected)
                        .map(|task| Action::UpdateTaskStatus {
                            task_id: task.id.unwrap_or(0),
                            new_status: task.status.next(),
                        })
                }
                KeyCode::Char('d') => {
                    if let Some(task) = self.tasks.get(self.selected) {
                        let task_id = task.id.unwrap_or(0);
                        let task_name = task.name.clone();
                        let dependents_count =
                            repo.get_dependents(task_id).unwrap_or_default().len();
                        let message = if dependents_count > 0 {
                            format!(
                                "Deleting will remove {} dependency links. Continue? (y/N)",
                                dependents_count
                            )
                        } else {
                            format!("Delete task {}? (y/N)", task_name)
                        };
                        self.mode = TaskListMode::ConfirmDelete(ConfirmDialog::new(
                            message,
                            Action::DeleteTask { task_id },
                        ));
                    }
                    None
                }
                KeyCode::Char('f') => {
                    self.mode = TaskListMode::Filtering(FilterState {
                        filter_status: self.filter_status,
                        filter_priority: self.filter_priority,
                        selected_field: FilterField::Status,
                    });
                    None
                }
                _ => None,
            },
            TaskListMode::ConfirmDelete(dialog) => {
                let action = dialog.handle_key(code);
                self.mode = TaskListMode::Browsing;
                action
            }
            TaskListMode::Filtering(state) => match code {
                KeyCode::Esc => {
                    self.mode = TaskListMode::Browsing;
                    None
                }
                KeyCode::Enter => {
                    self.filter_status = state.filter_status;
                    self.filter_priority = state.filter_priority;
                    self.apply_filter();
                    self.mode = TaskListMode::Browsing;
                    None
                }
                KeyCode::Tab => {
                    state.selected_field = match state.selected_field {
                        FilterField::Status => FilterField::Priority,
                        FilterField::Priority => FilterField::Status,
                    };
                    None
                }
                KeyCode::Left => {
                    match state.selected_field {
                        FilterField::Status => {
                            state.filter_status = state
                                .filter_status
                                .map(Status::cycle_previous)
                                .unwrap_or(Some(Status::Done));
                        }
                        FilterField::Priority => {
                            state.filter_priority = state
                                .filter_priority
                                .map(Priority::cycle_previous)
                                .unwrap_or(Some(Priority::Low));
                        }
                    };
                    None
                }
                KeyCode::Right => {
                    match state.selected_field {
                        FilterField::Status => {
                            state.filter_status = state
                                .filter_status
                                .map(Status::cycle_next)
                                .unwrap_or(Some(Status::Todo));
                        }
                        FilterField::Priority => {
                            state.filter_priority = state
                                .filter_priority
                                .map(Priority::cycle_next)
                                .unwrap_or(Some(Priority::High));
                        }
                    };
                    None
                }
                _ => None,
            },
        }
    }

    /// Render the task table (browsing) or the active modal popup.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        match &self.mode {
            TaskListMode::Browsing => self.render_table(frame, area),
            TaskListMode::ConfirmDelete(dialog) => dialog.render(frame, area),
            TaskListMode::Filtering(state) => render_filter_popup(state, frame, area),
        }
    }

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        if self.tasks.is_empty() {
            let paragraph = Paragraph::new("No tasks. Press c to create one.")
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        let header = Row::new(vec!["Name", "Status", "Priority"]).style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        let rows: Vec<Row> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, task)| {
                let style = if i == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    task.status.row_style()
                };
                Row::new(vec![
                    Cell::from(task.name.clone()),
                    Cell::from(task.status.label()).style(task.status.style()),
                    Cell::from(task.priority.label()).style(task.priority.style()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ];
        let title = filter_title(self.filter_status, self.filter_priority);
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(title).borders(Borders::ALL));
        frame.render_widget(table, area);
    }
}

impl Default for TaskListView {
    fn default() -> Self {
        Self::new()
    }
}

fn filter_title(status: Option<Status>, priority: Option<Priority>) -> String {
    match (status, priority) {
        (None, None) => "Tasks".to_string(),
        (Some(s), None) => format!("Tasks (Status: {})", s.label()),
        (None, Some(p)) => format!("Tasks (Priority: {})", p.label()),
        (Some(s), Some(p)) => format!("Tasks ({} - {})", s.label(), p.label()),
    }
}

fn render_filter_popup(state: &FilterState, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(52, 11, area);
    let inner = render_popup_scaffold(frame, popup_area, "Filter Tasks");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let status_label = state
        .filter_status
        .map_or_else(|| "Any".to_string(), |s| s.label().to_string());
    let priority_label = state
        .filter_priority
        .map_or_else(|| "Any".to_string(), |p| p.label().to_string());

    render_readonly_field(
        frame,
        chunks[0],
        "Status",
        &status_label,
        state.selected_field == FilterField::Status,
    );
    render_readonly_field(
        frame,
        chunks[1],
        "Priority",
        &priority_label,
        state.selected_field == FilterField::Priority,
    );

    let help = "Tab switch  ← → change  Enter apply  Esc cancel";
    let help_para = Paragraph::new(help)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[2]);
}

#[cfg(test)]
mod tests {
    use crate::db::stub::StubRepository;

    use super::*;

    fn task(name: &str, status: Status) -> Task {
        Task {
            id: None,
            project_id: 1,
            name: name.to_string(),
            description: String::new(),
            status,
            priority: Priority::Low,
            created_at: String::new(),
        }
    }

    #[test]
    fn all_tasks_ignores_active_filter() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(vec![task("A", Status::Todo), task("B", Status::Done)]);

        // Apply a status filter (Status: Todo) through the filter popup.
        view.handle_key(KeyCode::Char('f'), 1, &repo);
        view.handle_key(KeyCode::Right, 1, &repo);
        view.handle_key(KeyCode::Enter, 1, &repo);

        assert_eq!(view.tasks().len(), 1);
        assert_eq!(view.all_tasks().len(), 2);
    }

    #[test]
    fn filter_title_renders_active_status() {
        assert_eq!(
            filter_title(Some(Status::Todo), None),
            "Tasks (Status: Todo)"
        );
    }

    #[test]
    fn filter_title_renders_active_priority() {
        assert_eq!(
            filter_title(None, Some(Priority::High)),
            "Tasks (Priority: High)"
        );
    }

    #[test]
    fn filter_title_renders_both_active() {
        assert_eq!(
            filter_title(Some(Status::Done), Some(Priority::Low)),
            "Tasks (Done - Low)"
        );
    }

    #[test]
    fn filter_title_renders_default_when_none() {
        assert_eq!(filter_title(None, None), "Tasks");
    }

    #[test]
    fn set_tasks_preserves_mode() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(vec![
            task("A", Status::Todo),
            task("B", Status::Done),
            task("C", Status::Todo),
        ]);

        // Apply a status filter through the filter popup.
        view.handle_key(KeyCode::Char('f'), 1, &repo);
        view.handle_key(KeyCode::Right, 1, &repo); // status -> Todo
        view.handle_key(KeyCode::Enter, 1, &repo); // apply
        assert_eq!(view.tasks().len(), 2);

        // Re-enter filtering mode to verify it survives a data reload.
        view.handle_key(KeyCode::Char('f'), 1, &repo);
        assert!(matches!(view.mode(), TaskListMode::Filtering(_)));

        // Reload data while the filter dialog is open.
        view.set_tasks(vec![
            task("D", Status::Todo),
            task("E", Status::Done),
            task("F", Status::Todo),
            task("G", Status::Todo),
        ]);

        // Mode, reapplied filter, and clamped selection must survive.
        assert!(matches!(view.mode(), TaskListMode::Filtering(_)));
        assert_eq!(view.tasks().len(), 3);
        assert!(view.tasks().iter().all(|t| t.status == Status::Todo));
        assert!(view.selected() < view.tasks().len());
    }
}
