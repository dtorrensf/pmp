// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::db::repository::Repository;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::version::Version;
use crate::screens::components::badge::Badge;
use crate::screens::components::confirm_dialog::{ConfirmAnswer, ConfirmDialog};
use crate::screens::components::empty_state::EmptyState;
use crate::screens::components::fields::{render_field, render_readonly_field};
use crate::screens::components::glyphs;
use crate::screens::components::list_navigation::move_selection;
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};
use crate::screens::components::scroll::{ListScroll, list_rows, render_list_scrollbar};
use crate::screens::components::theme::{self, priority_style, status_row_style, status_style};
use crate::screens::{Action, Screen};

#[derive(Debug, Clone)]
pub enum TaskListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
    Filtering(FilterState),
    MovingTask(MoveTaskState),
}

/// State of the "move task to version" popup: the task being moved, the other
/// versions of its project and the highlighted target.
#[derive(Debug, Clone)]
pub struct MoveTaskState {
    pub task_id: i64,
    pub targets: Vec<Version>,
    pub selected: usize,
}

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

    /// Direction arrow shown in the table title, `None` for the default order.
    pub fn arrow(self) -> Option<&'static str> {
        let glyphs = glyphs::glyphs();
        match self {
            SortMode::Priority => None,
            SortMode::IdAsc => Some(glyphs.sort_asc),
            SortMode::IdDesc => Some(glyphs.sort_desc),
        }
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
/// - `o`: cycle the sort order (priority, then id ascending and descending)
///
/// The move popup is opened by the embedding screen through
/// [`TaskListView::open_move`] because only the screen knows the other
/// versions of the project.
pub struct TaskListView {
    all_tasks: Vec<Task>,
    tasks: Vec<Task>,
    selected: usize,
    scroll: ListScroll,
    filter_status: Option<Status>,
    filter_priority: Option<Priority>,
    filter_title: Option<String>,
    sort_mode: SortMode,
    mode: TaskListMode,
    /// Dependency summary of each task, keyed by task id.
    dependency_summaries: HashMap<i64, DependencySummary>,
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
            sort_mode: SortMode::default(),
            mode: TaskListMode::Browsing,
            dependency_summaries: HashMap::new(),
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

    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
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
        if let Some(task_id) = self.tasks.get(self.selected).and_then(|task| task.id) {
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
        self.all_tasks = tasks;
        self.apply_filter();
    }

    /// Dependency summary of each task, keyed by task id. Shown as a compact
    /// indicator next to the task name. Missing ids show no indicator.
    pub fn set_dependency_summaries(&mut self, summaries: HashMap<i64, DependencySummary>) {
        self.dependency_summaries = summaries;
    }

    fn apply_filter(&mut self) {
        let mut tasks: Vec<Task> = self
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
        sort_by_id(&mut tasks, self.sort_mode);
        self.tasks = tasks;
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
        match self.mode {
            TaskListMode::Browsing => self.handle_browsing_key(code, repo),
            TaskListMode::ConfirmDelete(_) => self.handle_confirm_delete_key(code),
            TaskListMode::MovingTask(_) => self.handle_moving_key(code),
            TaskListMode::Filtering(_) => self.handle_filtering_key(code),
        }
    }

    /// Advances the sort mode and keeps the same task selected, found by id.
    fn cycle_sort(&mut self) {
        let selected_id = self.tasks.get(self.selected).and_then(|task| task.id);
        self.sort_mode = self.sort_mode.next();
        self.apply_filter();
        if let Some(index) =
            selected_id.and_then(|id| self.tasks.iter().position(|task| task.id == Some(id)))
        {
            self.selected = index;
        }
    }

    /// Keys while browsing the table: move the selection, open, change status,
    /// delete, filter, or change the sort order.
    fn handle_browsing_key(&mut self, code: KeyCode, repo: &dyn Repository) -> Option<Action> {
        match code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                move_selection(&mut self.selected, self.tasks.len(), code);
                None
            }
            KeyCode::Char('o') => {
                self.cycle_sort();
                None
            }
            KeyCode::Enter => self.tasks.get(self.selected).map(|task| {
                Action::Navigate(Screen::TaskForm {
                    version_id: task.version_id,
                    task_id: task.id,
                })
            }),
            KeyCode::Tab => self
                .tasks
                .get(self.selected)
                .map(|task| Action::UpdateTaskStatus {
                    task_id: task.id.unwrap_or(0),
                    new_status: task.status.next(),
                }),
            KeyCode::Char('d') => {
                if let Some(task) = self.tasks.get(self.selected) {
                    let task_id = task.id.unwrap_or(0);
                    let task_name = task.name.clone();
                    let dependents_count = repo.get_dependents(task_id).unwrap_or_default().len();
                    let message = if dependents_count > 0 {
                        format!(
                            "Deleting will remove {} dependency links. Continue?",
                            dependents_count
                        )
                    } else {
                        format!("Delete task {}?", task_name)
                    };
                    self.mode = TaskListMode::ConfirmDelete(
                        ConfirmDialog::new(message, Action::DeleteTask { task_id })
                            .with_title("Delete task"),
                    );
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
        }
    }

    /// Keys in the delete confirmation: `y` confirms, `n`/`Esc` cancel and
    /// any other key keeps the dialog open.
    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Option<Action> {
        let TaskListMode::ConfirmDelete(dialog) = &self.mode else {
            return None;
        };
        match dialog.handle_key(code) {
            ConfirmAnswer::Confirmed(action) => {
                self.mode = TaskListMode::Browsing;
                Some(action)
            }
            ConfirmAnswer::Cancelled => {
                self.mode = TaskListMode::Browsing;
                None
            }
            ConfirmAnswer::Ignored => None,
        }
    }

    /// Keys in the move-to-version popup.
    fn handle_moving_key(&mut self, code: KeyCode) -> Option<Action> {
        let TaskListMode::MovingTask(state) = &mut self.mode else {
            return None;
        };
        match code {
            KeyCode::Esc => {
                self.mode = TaskListMode::Browsing;
                None
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                move_selection(&mut state.selected, state.targets.len(), code);
                None
            }
            KeyCode::Enter => {
                let action = state.targets.get(state.selected).and_then(|target| {
                    target.id.map(|version_id| Action::MoveTask {
                        task_id: state.task_id,
                        version_id,
                    })
                });
                // Without targets (only one version) the popup just closes.
                self.mode = TaskListMode::Browsing;
                action
            }
            _ => None,
        }
    }

    /// Keys in the filter popup: edit the title, cycle status and priority.
    fn handle_filtering_key(&mut self, code: KeyCode) -> Option<Action> {
        let TaskListMode::Filtering(state) = &mut self.mode else {
            return None;
        };
        match code {
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
        }
    }

    /// Render the task table (browsing) or the active modal popup. The table
    /// stays visible, dimmed, behind the delete confirmation.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        match &self.mode {
            TaskListMode::Browsing => self.render_table(frame, area),
            TaskListMode::ConfirmDelete(dialog) => {
                self.render_table(frame, area);
                // Center the modal on the whole screen, not just on the table.
                dialog.render(frame, frame.area());
            }
            TaskListMode::Filtering(state) => render_filter_popup(state, frame, area),
            TaskListMode::MovingTask(state) => render_move_popup(state, frame, area),
        }
    }

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        if self.tasks.is_empty() {
            EmptyState::new("No tasks")
                .with_icon(glyphs::glyphs().status_todo)
                .with_hint("Press c to create one.")
                .render(frame, area);
            return;
        }

        // Rows that fit below the top border and the header row.
        let row_height = list_rows(area, 1);
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

        let header = Row::new(vec![
            Cell::from(Line::from("ID").right_aligned()),
            Cell::from("Name"),
            Cell::from("Status"),
            Cell::from("Priority"),
        ])
        .style(theme::theme().header_style());

        let widths = [
            Constraint::Length(id_width),
            Constraint::Percentage(47),
            Constraint::Percentage(26),
            Constraint::Percentage(26),
        ];
        let mut title = task_list_title(
            self.filter_title.as_deref(),
            self.filter_status,
            self.filter_priority,
        );
        if let Some(arrow) = self.sort_mode.arrow() {
            title = format!("{title} · ID {arrow}");
        }
        let block = Block::default().title(title).borders(Borders::ALL);
        // Width of the name column, so long names can be cut to fit it.
        let name_width = Layout::horizontal(widths)
            .spacing(1)
            .flex(Flex::Start)
            .split(block.inner(area))[1]
            .width as usize;

        let rows: Vec<Row> = self.tasks[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, task)| {
                let i = range.start + offset;
                let selected = i == self.selected;
                let style = if selected {
                    theme::theme().selection_style()
                } else {
                    status_row_style(task.status)
                };
                let dependencies = task
                    .id
                    .and_then(|id| self.dependency_summaries.get(&id).copied());
                Row::new(vec![
                    Cell::from(
                        Line::from(task.id.map_or_else(|| "-".to_string(), |id| id.to_string()))
                            .right_aligned(),
                    ),
                    name_cell(&task.name, dependencies, selected, name_width),
                    Cell::from(Badge::status(task.status).text()).style(status_style(task.status)),
                    Cell::from(Badge::priority(task.priority).text())
                        .style(priority_style(task.priority)),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(rows, widths).header(header).block(block);
        frame.render_widget(table, area);

        render_list_scrollbar(frame, area, 1, self.tasks.len(), range.start);
    }
}

impl Default for TaskListView {
    fn default() -> Self {
        Self::new()
    }
}

/// How many dependencies a task has and how many of them are not done yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencySummary {
    pub total: usize,
    pub pending: usize,
}

/// Name cell: the selection marker, the name cut to `width` columns and,
/// when the task has dependencies, a compact count such as `↳ 2`. The count
/// is green when every dependency is done and yellow while any is pending.
fn name_cell(
    name: &str,
    dependencies: Option<DependencySummary>,
    selected: bool,
    width: usize,
) -> Cell<'static> {
    let glyphs = glyphs::glyphs();
    let marker = if selected { glyphs.selection } else { " " };
    let (suffix, suffix_style) = match dependencies {
        Some(summary) if summary.total > 0 => {
            let style = if summary.pending == 0 {
                theme::theme().success_style()
            } else {
                theme::theme().warning_style()
            };
            (format!("  {} {}", glyphs.dependency, summary.total), style)
        }
        _ => (String::new(), Style::default()),
    };
    let available = width.saturating_sub(marker.width() + 1 + suffix.width());
    Cell::from(Line::from(vec![
        Span::raw(format!("{marker} {}", truncate_display(name, available))),
        Span::styled(suffix, suffix_style),
    ]))
}

/// Cuts `text` to at most `width` terminal columns. A cut text ends with an
/// ellipsis that counts as one column.
fn truncate_display(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
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

fn render_move_popup(state: &MoveTaskState, frame: &mut Frame, area: Rect) {
    let height = (state.targets.len() as u16).max(1) + 4;
    let popup_area = centered_rect(52, height, area);
    let inner = render_popup_scaffold(frame, popup_area, "Move task to version");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    if state.targets.is_empty() {
        let message = Paragraph::new("This is the only version of the project.")
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(message, chunks[0]);
    } else {
        let lines: Vec<ratatui::text::Line> = state
            .targets
            .iter()
            .enumerate()
            .map(|(i, version)| {
                let text = format!("{} · {}", version.name, version.status.label());
                let style = if i == state.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                ratatui::text::Line::styled(text, style)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), chunks[0]);
    }

    let help = if state.targets.is_empty() {
        "Esc close"
    } else {
        "↑ ↓ choose  Enter move  Esc cancel"
    };
    let help_para = Paragraph::new(help)
        .style(theme::theme().muted_style())
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[1]);
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
        .style(theme::theme().muted_style())
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[3]);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

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
            completed_at: None,
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
    fn long_names_are_cut_with_an_ellipsis_to_the_name_column() {
        let mut view = TaskListView::new();
        view.set_tasks(vec![task(
            "Una descripción de tarea bastante más larga que la columna",
            Status::Todo,
        )]);

        let rendered = render_view(&view, 40, 6);

        assert!(rendered.contains('…'), "{rendered}");
        assert!(!rendered.contains("columna"), "{rendered}");
    }

    #[test]
    fn truncate_display_measures_columns_not_chars() {
        // Each CJK character is two columns wide.
        assert_eq!(truncate_display("ab漢字", 4), "ab…");
        assert_eq!(truncate_display("abc", 3), "abc");
        assert_eq!(truncate_display("abcd", 0), "");
    }

    #[test]
    fn dependency_count_is_shown_next_to_the_name() {
        let mut with_deps = task("Alpha", Status::Todo);
        with_deps.id = Some(7);
        let mut view = TaskListView::new();
        view.set_tasks(vec![with_deps, task("Beta", Status::Todo)]);
        view.set_dependency_summaries(HashMap::from([(
            7,
            DependencySummary {
                total: 2,
                pending: 2,
            },
        )]));

        let rendered = render_view(&view, 60, 8);

        assert!(rendered.contains("↳ 2"), "{rendered}");
        assert_eq!(rendered.matches("↳").count(), 1, "{rendered}");
    }

    /// Foreground color of the dependency marker in a view rendered at 60x8.
    fn dependency_marker_color(summary: DependencySummary) -> Option<ratatui::style::Color> {
        let mut with_deps = task("Alpha", Status::Todo);
        with_deps.id = Some(7);
        let mut view = TaskListView::new();
        view.set_tasks(vec![with_deps]);
        view.set_dependency_summaries(HashMap::from([(7, summary)]));

        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .find(|cell| cell.symbol() == "↳")
            .map(|cell| cell.fg)
    }

    #[test]
    fn dependency_marker_is_green_when_all_dependencies_are_done() {
        let color = dependency_marker_color(DependencySummary {
            total: 2,
            pending: 0,
        });

        assert_eq!(color, Some(theme::theme().success_style().fg.unwrap()));
    }

    #[test]
    fn dependency_marker_is_yellow_while_a_dependency_is_pending() {
        let color = dependency_marker_color(DependencySummary {
            total: 2,
            pending: 1,
        });

        assert_eq!(color, Some(theme::theme().warning_style().fg.unwrap()));
    }

    #[test]
    fn selected_row_is_marked_with_the_selection_glyph() {
        let mut view = TaskListView::new();
        view.set_tasks(vec![
            task("Alpha", Status::Todo),
            task("Beta", Status::Todo),
        ]);

        let rendered = render_view(&view, 60, 8);

        assert!(rendered.contains("▸ Alpha"), "{rendered}");
        assert!(!rendered.contains("▸ Beta"), "{rendered}");
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
    fn status_and_priority_cells_carry_their_glyph() {
        let mut view = TaskListView::new();
        view.set_tasks(vec![task("Alpha", Status::InProgress)]);

        let rendered = render_view(&view, 60, 10);

        // The state is not communicated by color alone.
        assert!(rendered.contains("◐ In Progress"), "{rendered}");
    }

    #[test]
    fn header_row_uses_the_theme_instead_of_hardcoded_white() {
        let mut view = TaskListView::new();
        view.set_tasks(vec![task("Alpha", Status::Todo)]);

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // The "ID" header cell sits at the first column of the header row.
        let header_cell = &buffer[(1, 1)];
        assert_eq!(header_cell.fg, theme::theme().text);
        // White is invisible on light terminals.
        assert_ne!(header_cell.fg, ratatui::style::Color::White);
        assert!(header_cell.modifier.contains(Modifier::BOLD));
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

    fn task_with_id(id: Option<i64>, name: &str, status: Status, priority: Priority) -> Task {
        Task {
            id,
            priority,
            ..task(name, status)
        }
    }

    fn ids(view: &TaskListView) -> Vec<Option<i64>> {
        view.tasks().iter().map(|task| task.id).collect()
    }

    /// Five tasks in repository order (priority, as the repo returns them),
    /// one of them without id.
    fn sortable_view() -> TaskListView {
        let mut view = TaskListView::new();
        view.set_tasks(vec![
            task_with_id(Some(12), "Twelve", Status::Done, Priority::High),
            task_with_id(None, "Unsaved", Status::Todo, Priority::High),
            task_with_id(Some(3), "Three", Status::Todo, Priority::Low),
            task_with_id(Some(7), "Seven", Status::InProgress, Priority::Medium),
            task_with_id(Some(5), "Five", Status::Todo, Priority::Low),
        ]);
        view
    }

    #[test]
    fn sort_mode_cycles_priority_then_id_asc_then_id_desc() {
        let repo = StubRepository::default();
        let mut view = TaskListView::new();
        assert_eq!(view.sort_mode(), SortMode::Priority);

        view.handle_key(KeyCode::Char('o'), &repo);
        assert_eq!(view.sort_mode(), SortMode::IdAsc);
        view.handle_key(KeyCode::Char('o'), &repo);
        assert_eq!(view.sort_mode(), SortMode::IdDesc);
        view.handle_key(KeyCode::Char('o'), &repo);
        assert_eq!(view.sort_mode(), SortMode::Priority);
    }

    #[test]
    fn default_priority_mode_keeps_repository_order() {
        let view = sortable_view();
        assert_eq!(ids(&view), vec![Some(12), None, Some(3), Some(7), Some(5)]);
    }

    #[test]
    fn id_asc_orders_by_id_and_puts_tasks_without_id_last() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);

        assert_eq!(ids(&view), vec![Some(3), Some(5), Some(7), Some(12), None]);
    }

    #[test]
    fn id_desc_orders_by_id_descending_and_still_puts_missing_ids_last() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);
        view.handle_key(KeyCode::Char('o'), &repo);

        assert_eq!(ids(&view), vec![Some(12), Some(7), Some(5), Some(3), None]);
    }

    #[test]
    fn id_sort_does_not_move_done_tasks_to_the_end() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);

        // Twelve is Done but keeps its place by id, not at the bottom.
        assert_eq!(view.tasks()[3].name, "Twelve");
    }

    #[test]
    fn id_sort_combines_with_status_filter() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);

        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Right, &repo); // status -> Todo
        view.handle_key(KeyCode::Enter, &repo);

        assert_eq!(ids(&view), vec![Some(3), Some(5), None]);
    }

    #[test]
    fn id_sort_combines_with_title_filter() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);
        view.handle_key(KeyCode::Char('o'), &repo);

        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Tab, &repo);
        view.handle_key(KeyCode::Tab, &repo);
        for character in "e".chars() {
            view.handle_key(KeyCode::Char(character), &repo);
        }
        view.handle_key(KeyCode::Enter, &repo);

        // Every name contains "e"; the list is still ordered by id descending.
        assert_eq!(ids(&view), vec![Some(12), Some(7), Some(5), Some(3), None]);
        view.handle_key(KeyCode::Char('o'), &repo); // back to priority order
        assert_eq!(ids(&view), vec![Some(12), None, Some(3), Some(7), Some(5)]);
    }

    #[test]
    fn cycling_sort_keeps_the_same_task_selected() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        // Select "Seven", which is at index 3 in priority order.
        view.set_selected(3);
        assert_eq!(view.tasks()[view.selected()].id, Some(7));

        view.handle_key(KeyCode::Char('o'), &repo); // IdAsc: Seven is at index 2
        assert_eq!(view.tasks()[view.selected()].id, Some(7));
        assert_eq!(view.selected(), 2);

        view.handle_key(KeyCode::Char('o'), &repo); // IdDesc: Seven is at index 1
        assert_eq!(view.tasks()[view.selected()].id, Some(7));
        assert_eq!(view.selected(), 1);
    }

    #[test]
    fn sort_key_is_ignored_while_filter_popup_is_open() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('f'), &repo);
        view.handle_key(KeyCode::Tab, &repo);
        view.handle_key(KeyCode::Tab, &repo);

        view.handle_key(KeyCode::Char('o'), &repo);

        assert_eq!(view.sort_mode(), SortMode::Priority);
        match view.mode() {
            TaskListMode::Filtering(state) => assert_eq!(state.filter_title, "o"),
            other => panic!("expected filter popup, got {other:?}"),
        }
    }

    #[test]
    fn sort_mode_survives_data_reload() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        view.handle_key(KeyCode::Char('o'), &repo);

        view.set_tasks(vec![
            task_with_id(Some(9), "Nine", Status::Todo, Priority::Low),
            task_with_id(Some(1), "One", Status::Todo, Priority::Low),
        ]);

        assert_eq!(view.sort_mode(), SortMode::IdAsc);
        assert_eq!(ids(&view), vec![Some(1), Some(9)]);
    }

    #[test]
    fn table_title_shows_the_active_id_direction() {
        let repo = StubRepository::default();
        let mut view = sortable_view();
        assert!(!render_view(&view, 60, 10).contains("ID ↑"));

        view.handle_key(KeyCode::Char('o'), &repo);
        assert!(render_view(&view, 60, 10).contains("Tasks · ID ↑"));

        view.handle_key(KeyCode::Char('o'), &repo);
        assert!(render_view(&view, 60, 10).contains("Tasks · ID ↓"));
    }
}
