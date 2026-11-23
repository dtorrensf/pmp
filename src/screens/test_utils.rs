// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use crate::db::stub::StubRepository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};
use crate::domain::version::{Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;

use super::Component;

pub fn key_event(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

pub fn char_event(c: char) -> Event {
    key_event(KeyCode::Char(c))
}

pub fn ctrl_s_event() -> Event {
    Event::Key(KeyEvent {
        code: KeyCode::Char('s'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

pub fn render_to_string(target: &impl Component, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| target.render(frame, frame.area()))
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

pub fn buffer_to_string(buffer: &Buffer) -> String {
    let area = buffer.area();
    let mut lines = Vec::new();
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use ratatui::{Frame, layout::Rect, widgets::Block};

    use super::*;

    struct StubScreen;

    impl Component for StubScreen {
        fn render(&self, frame: &mut Frame, area: Rect) {
            frame.render_widget(Block::bordered().title("S"), area);
        }
    }

    #[test]
    fn key_event_is_press_with_empty_modifiers() {
        let Event::Key(event) = key_event(KeyCode::Enter) else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Enter);
        assert_eq!(event.modifiers, KeyModifiers::empty());
        assert_eq!(event.kind, KeyEventKind::Press);
        assert_eq!(event.state, KeyEventState::empty());
    }

    #[test]
    fn char_event_wraps_char_keycode() {
        let Event::Key(event) = char_event('x') else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Char('x'));
        assert_eq!(event.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn ctrl_s_event_is_char_s_with_control_modifier() {
        let Event::Key(event) = ctrl_s_event() else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Char('s'));
        assert_eq!(event.modifiers, KeyModifiers::CONTROL);
        assert_eq!(event.kind, KeyEventKind::Press);
    }

    #[test]
    fn buffer_to_string_trims_trailing_whitespace_per_line() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 2));
        buffer[(0, 0)].set_symbol("A");
        buffer[(1, 0)].set_symbol("B");

        let text = buffer_to_string(&buffer);

        assert_eq!(text.split('\n').collect::<Vec<_>>(), vec!["AB", ""]);
    }

    #[test]
    fn render_to_string_renders_target_at_explicit_size() {
        let output = render_to_string(&StubScreen, 20, 5);

        let lines: Vec<&str> = output.split('\n').collect();
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| line.chars().count() <= 20));
        assert!(output.contains('S'));
    }
}

/// Repository double for the error-propagation tests: it delegates to a
/// [`StubRepository`] and makes the selected queries fail with an internal
/// error. Any operation outside the flows under test panics, so an unexpected
/// query is noticed instead of silently returning empty data.
#[derive(Default)]
pub struct FaultyRepository {
    pub inner: StubRepository,
    /// Fail `list_versions_by_project` (the move-to-version popup).
    pub fail_versions: bool,
    /// Fail `get_dependents` (the delete confirmation).
    pub fail_dependents: bool,
    /// Fail `get_dependencies` (the dependency markers of a version).
    pub fail_dependencies: bool,
}

impl FaultyRepository {
    fn boom<T>(&self) -> Result<T, AppError> {
        Err(AppError::Internal)
    }
}

macro_rules! unexpected {
    ($name:literal) => {
        unimplemented!("FaultyRepository: unexpected call to {}", $name)
    };
}

impl crate::db::repository::ProjectRepository for FaultyRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        unexpected!("list_projects")
    }
    fn create_project(&self, _: &str, _: &str) -> Result<Project, AppError> {
        unexpected!("create_project")
    }
    fn update_project(&self, _: i64, _: &str, _: &str) -> Result<Project, AppError> {
        unexpected!("update_project")
    }
    fn get_project(&self, id: i64) -> Result<Project, AppError> {
        self.inner.get_project(id)
    }
    fn delete_project(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_project")
    }
}

impl crate::db::repository::VersionRepository for FaultyRepository {
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError> {
        if self.fail_versions {
            return self.boom();
        }
        self.inner.list_versions_by_project(project_id)
    }
    fn create_version_with_status(
        &self,
        _: i64,
        _: &str,
        _: &str,
        _: VersionStatus,
    ) -> Result<Version, AppError> {
        unexpected!("create_version_with_status")
    }
    fn get_version(&self, id: i64) -> Result<Version, AppError> {
        self.inner.get_version(id)
    }
    fn update_version(
        &self,
        _: i64,
        _: &str,
        _: &str,
        _: VersionStatus,
    ) -> Result<Version, AppError> {
        unexpected!("update_version")
    }
    fn delete_version(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_version")
    }
}

impl crate::db::repository::TaskRepository for FaultyRepository {
    fn list_tasks_by_project(&self, _: i64) -> Result<Vec<Task>, AppError> {
        unexpected!("list_tasks_by_project")
    }
    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError> {
        self.inner.list_tasks_by_version(version_id)
    }
    fn create_task(
        &self,
        _: i64,
        _: i64,
        _: &str,
        _: &str,
        _: Status,
        _: Priority,
    ) -> Result<Task, AppError> {
        unexpected!("create_task")
    }
    fn get_task(&self, id: i64) -> Result<Task, AppError> {
        self.inner.get_task(id)
    }
    fn update_task(
        &self,
        _: i64,
        _: &str,
        _: &str,
        _: Status,
        _: Priority,
    ) -> Result<Task, AppError> {
        unexpected!("update_task")
    }
    fn update_task_status(&self, _: i64, _: Status) -> Result<Task, AppError> {
        unexpected!("update_task_status")
    }
    fn update_task_priority(&self, _: i64, _: Priority) -> Result<Task, AppError> {
        unexpected!("update_task_priority")
    }
    fn move_task(&self, _: i64, _: i64) -> Result<Task, AppError> {
        unexpected!("move_task")
    }
    fn delete_task(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_task")
    }
    fn delete_done_tasks_by_version(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_done_tasks_by_version")
    }
}

impl crate::db::repository::DependencyRepository for FaultyRepository {
    fn add_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
        unexpected!("add_dependency")
    }
    fn remove_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
        unexpected!("remove_dependency")
    }
    fn set_dependencies(&self, _: i64, _: &[i64]) -> Result<(), AppError> {
        unexpected!("set_dependencies")
    }
    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        if self.fail_dependencies {
            return self.boom();
        }
        self.inner.get_dependencies(task_id)
    }
    fn has_incomplete_dependencies(&self, _: i64) -> Result<bool, AppError> {
        unexpected!("has_incomplete_dependencies")
    }
    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        if self.fail_dependents {
            return self.boom();
        }
        self.inner.get_dependents(task_id)
    }
    fn would_create_cycle(&self, _: i64, _: i64) -> Result<bool, AppError> {
        unexpected!("would_create_cycle")
    }
}

impl crate::db::repository::NoteRepository for FaultyRepository {
    fn list_task_notes(&self, _: i64) -> Result<Vec<TaskNote>, AppError> {
        unexpected!("list_task_notes")
    }
    fn add_task_note(&self, _: i64, _: &str) -> Result<TaskNote, AppError> {
        unexpected!("add_task_note")
    }
    fn delete_task_note(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_task_note")
    }
}

impl crate::db::repository::SummaryRepository for FaultyRepository {
    fn list_version_summaries(&self, version_id: i64) -> Result<Vec<VersionSummary>, AppError> {
        self.inner.list_version_summaries(version_id)
    }
    fn get_version_summary(&self, _: i64) -> Result<VersionSummary, AppError> {
        unexpected!("get_version_summary")
    }
    fn create_version_summary(&self, _: i64, _: &str, _: &str) -> Result<VersionSummary, AppError> {
        unexpected!("create_version_summary")
    }
    fn delete_version_summary(&self, _: i64) -> Result<(), AppError> {
        unexpected!("delete_version_summary")
    }
    fn compact_version(&self, _: i64, _: &str, _: &str) -> Result<VersionSummary, AppError> {
        unexpected!("compact_version")
    }
}
