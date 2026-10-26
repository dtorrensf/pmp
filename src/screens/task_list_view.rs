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
use crate::screens::components::fields::{render_field, render_readonly_field};
use crate::screens::components::list_navigation::move_selection;
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};
use crate::screens::components::scroll::{ListScroll, render_scrollbar};
use crate::screens::{Action, Screen};

#[derive(Debug, Clone)]
pub enum TaskListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
    Filtering(FilterState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Title,
    Status,
    Priority,
}

#[derive(Debug, Clone)]
pub struct FilterState {
    pub filter_title: String,
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
    scroll: ListScroll,
    filter_status: Option<Status>,
    filter_priority: Option<Priority>,
    filter_title: Option<String>,
    mode: TaskListMode,
}

impl TaskListView {
    pub fn new() -> Self {
        Self {
            all_tasks: Vec::new(),
            tasks: Vec::new(),
            selected: 0,
            scroll: ListScroll::new(),
            filter_status: None,
            filter_priority: None,
            filter_title: None,
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

    pub fn filter_title(&self) -> Option<&str> {
        self.filter_title.as_deref()
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
                    && self
                        .filter_title
                        .as_ref()
                        .is_none_or(|title| task.name.to_lowercase().contains(title))
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
    pub fn handle_key(&mut self, code: KeyCode, repo: &dyn Repository) -> Option<Action> {
        match &mut self.mode {
            TaskListMode::Browsing => match code {
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                    move_selection(&mut self.selected, self.tasks.len(), code);
                    None
                }
                KeyCode::Enter => self.tasks.get(self.selected).map(|task| {
                    Action::Navigate(Screen::TaskForm {
                        version_id: task.version_id,
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
                        filter_title: self.filter_title.clone().unwrap_or_default(),
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
                    self.filter_title = (!state.filter_title.trim().is_empty())
                        .then(|| state.filter_title.trim().to_lowercase());
                    self.filter_status = state.filter_status;
                    self.filter_priority = state.filter_priority;
                    self.apply_filter();
                    self.mode = TaskListMode::Browsing;
                    None
                }
                KeyCode::Tab => {
                    state.selected_field = match state.selected_field {
                        FilterField::Title => FilterField::Status,
                        FilterField::Status => FilterField::Priority,
                        FilterField::Priority => FilterField::Title,
                    };
                    None
                }
                KeyCode::Char(character) if state.selected_field == FilterField::Title => {
                    state.filter_title.push(character);
                    None
                }
                KeyCode::Backspace if state.selected_field == FilterField::Title => {
                    state.filter_title.pop();
                    None
                }
                KeyCode::Left => {
                    match state.selected_field {
                        FilterField::Title => {}
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
                        FilterField::Title => {}
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

        // Rows that fit below the top border and the header row.
        let row_height = area.height.saturating_sub(3) as usize;
        let range = self
            .scroll
            .visible_range(self.tasks.len(), self.selected, row_height);

        // Wide enough for the largest id in the list and for the "ID" header.
        let id_width = self
            .tasks
            .iter()
            .filter_map(|task| task.id)
            .map(|id| id.to_string().len())
            .max()
            .unwrap_or(1)
            .max("ID".len()) as u16
            + 2;

        let header = Row::new(vec!["ID", "Name", "Status", "Priority"]).style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        let rows: Vec<Row> = self.tasks[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, task)| {
                let i = range.start + offset;
                let style = if i == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    task.status.row_style()
                };
                Row::new(vec![
                    Cell::from(task.id.map_or_else(|| "-".to_string(), |id| id.to_string())),
                    Cell::from(task.name.clone()),
                    Cell::from(task.status.label()).style(task.status.style()),
                    Cell::from(task.priority.label()).style(task.priority.style()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Length(id_width),
            Constraint::Percentage(47),
            Constraint::Percentage(26),
            Constraint::Percentage(26),
        ];
        let title = task_list_title(
            self.filter_title.as_deref(),
            self.filter_status,
            self.filter_priority,
        );
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(title).borders(Borders::ALL));
        frame.render_widget(table, area);

        // Scrollbar on the right border, aligned with the data rows.
        if row_height > 0 && area.width > 0 {
            let track = Rect::new(area.x + area.width - 1, area.y + 2, 1, row_height as u16);
            render_scrollbar(frame, track, self.tasks.len(), row_height, range.start);
        }
    }
}

impl Default for TaskListView {
    fn default() -> Self {
        Self::new()
    }
}

fn task_list_title(
    title: Option<&str>,
    status: Option<Status>,
    priority: Option<Priority>,
) -> String {
    if title.is_none() {
        return match (status, priority) {
            (None, None) => "Tasks".to_string(),
            (Some(status), None) => format!("Tasks (Status: {})", status.label()),
            (None, Some(priority)) => format!("Tasks (Priority: {})", priority.label()),
            (Some(status), Some(priority)) => {
                format!("Tasks ({} - {})", status.label(), priority.label())
            }
        };
    }

    let mut filters = Vec::new();
    if let Some(title) = title {
        filters.push(format!("Title: {title}"));
    }
    if let Some(status) = status {
        filters.push(format!("Status: {}", status.label()));
    }
    if let Some(priority) = priority {
        filters.push(format!("Priority: {}", priority.label()));
    }

    if filters.is_empty() {
        "Tasks".to_string()
    } else {
        format!("Tasks ({})", filters.join(" - "))
    }
}

fn render_filter_popup(state: &FilterState, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(52, 15, area);
    let inner = render_popup_scaffold(frame, popup_area, "Filter Tasks");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
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

    render_field(
        frame,
        chunks[0],
        "Title",
        &state.filter_title,
        state.selected_field == FilterField::Title,
        false,
    );
    render_readonly_field(
        frame,
        chunks[1],
        "Status",
        &status_label,
        state.selected_field == FilterField::Status,
    );
    render_readonly_field(
        frame,
        chunks[2],
        "Priority",
        &priority_label,
        state.selected_field == FilterField::Priority,
    );

    let help = "Tab switch  ← → change  Enter apply  Esc cancel";
    let help_para = Paragraph::new(help)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[3]);
}

#[cfg(test)]
mod tests {
    use crate::db::stub::StubRepository;

    use super::*;

    fn task(name: &str, status: Status) -> Task {
        Task {
            id: None,
            project_id: 1,
            version_id: 1,
            name: name.to_string(),
            description: String::new(),
            status,
            priority: Priority::Low,
            created_at: String::new(),
        }
    }

    fn render_view(view: &TaskListView, width: u16, height: u16) -> String {
        use crate::screens::test_utils::buffer_to_string;
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    fn numbered_tasks(count: usize) -> Vec<Task> {
        (0..count)
            .map(|i| task(&format!("Task{i:02}"), Status::Todo))
            .collect()
    }

    #[test]
    fn selected_row_stays_visible_when_list_overflows() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(numbered_tasks(30));

        // Height 13 => 10 data rows (border + header + border take 3).
        let before = render_view(&view, 40, 13);
        assert!(before.contains("Task00"));
        assert!(!before.contains("Task10"));

        for _ in 0..12 {
            view.handle_key(KeyCode::Down, &repo);
        }
        let after = render_view(&view, 40, 13);
        assert_eq!(view.selected(), 12);
        assert!(after.contains("Task12"), "selection not visible:\n{after}");
        assert!(
            !after.contains("Task00"),
            "top row should have scrolled out:\n{after}"
        );
    }

    #[test]
    fn jumping_to_last_task_renders_the_tail_of_the_list() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(numbered_tasks(30));
        view.set_selected(29);
        view.handle_key(KeyCode::Down, &repo); // clamped at last task
        let rendered = render_view(&view, 40, 13);
        assert!(rendered.contains("Task29"));
        assert!(!rendered.contains("Task09"));
    }

    #[test]
    fn moving_up_inside_view_keeps_rows_in_place() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(numbered_tasks(30));
        for _ in 0..15 {
            view.handle_key(KeyCode::Down, &repo);
        }
        let before = render_view(&view, 40, 13);
        view.handle_key(KeyCode::Up, &repo);
        let after = render_view(&view, 40, 13);
        assert!(before.contains("Task06") && after.contains("Task06"));
        assert!(after.contains("Task14"));
    }

    #[test]
    fn short_list_renders_without_scrolling() {
        let mut view = TaskListView::new();
        view.set_tasks(numbered_tasks(3));
        let rendered = render_view(&view, 40, 13);
        assert!(rendered.contains("Task00") && rendered.contains("Task02"));
    }

    #[test]
    fn rows_show_task_id_column() {
        let mut with_id = task("Alpha", Status::Todo);
        with_id.id = Some(42);
        let mut view = TaskListView::new();
        view.set_tasks(vec![with_id, task("Beta", Status::Done)]);

        let rendered = render_view(&view, 60, 10);
        assert!(rendered.contains("ID"), "missing ID header:\n{rendered}");
        assert!(rendered.contains("42"), "missing task id:\n{rendered}");
        assert!(rendered.contains("Beta"));
    }

    #[test]
    fn all_tasks_ignores_active_filter() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(vec![task("A", Status::Todo), task("B", Status::Done)]);

        // Apply a status filter (Status: Todo) through the filter popup.
        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Right, &repo);
        view.handle_key(KeyCode::Enter, &repo);

        assert_eq!(view.tasks().len(), 1);
        assert_eq!(view.all_tasks().len(), 2);
    }

    #[test]
    fn filters_tasks_by_title_case_insensitively_and_partially() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(vec![
            task("Fix Login Flow", Status::Todo),
            task("Update README", Status::Todo),
            task("Login metrics", Status::Done),
        ]);

        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Tab, &repo);
        view.handle_key(KeyCode::Tab, &repo);
        for character in "login".chars() {
            view.handle_key(KeyCode::Char(character), &repo);
        }
        view.handle_key(KeyCode::Enter, &repo);

        assert_eq!(view.tasks().len(), 2);
        assert!(
            view.tasks()
                .iter()
                .all(|task| task.name.to_lowercase().contains("login"))
        );
    }

    #[test]
    fn title_filter_combines_with_status_and_empty_title_clears_it() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        view.set_tasks(vec![
            task("Login todo", Status::Todo),
            task("Login done", Status::Done),
            task("Other todo", Status::Todo),
        ]);

        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Right, &repo);
        view.handle_key(KeyCode::Tab, &repo);
        view.handle_key(KeyCode::Tab, &repo);
        for character in "login".chars() {
            view.handle_key(KeyCode::Char(character), &repo);
        }
        view.handle_key(KeyCode::Enter, &repo);

        assert_eq!(view.tasks().len(), 1);
        assert_eq!(view.tasks()[0].name, "Login todo");

        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Tab, &repo);
        view.handle_key(KeyCode::Tab, &repo);
        for _ in 0..5 {
            view.handle_key(KeyCode::Backspace, &repo);
        }
        view.handle_key(KeyCode::Enter, &repo);

        assert_eq!(view.filter_title(), None);
        assert_eq!(view.tasks().len(), 2);
    }

    #[test]
    fn filter_title_renders_active_status() {
        assert_eq!(
            task_list_title(None, Some(Status::Todo), None),
            "Tasks (Status: Todo)"
        );
    }

    #[test]
    fn filter_title_renders_active_priority() {
        assert_eq!(
            task_list_title(None, None, Some(Priority::High)),
            "Tasks (Priority: High)"
        );
    }

    #[test]
    fn filter_title_renders_both_active() {
        assert_eq!(
            task_list_title(None, Some(Status::Done), Some(Priority::Low)),
            "Tasks (Done - Low)"
        );
    }

    #[test]
    fn filter_title_renders_default_when_none() {
        assert_eq!(task_list_title(None, None, None), "Tasks");
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
        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Right, &repo); // status -> Todo
        view.handle_key(KeyCode::Enter, &repo); // apply
        assert_eq!(view.tasks().len(), 2);

        // Re-enter filtering mode to verify it survives a data reload.
        view.handle_key(KeyCode::Char('f'), &repo);
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
