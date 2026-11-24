// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task list scoped to one version: the task table of `TaskListView` plus the
//! version header. Replaces the former project-wide task screen (removed in #94).
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
};

use crate::application;
use crate::db::repository::Repository;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::version::{Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;
use crate::screens::components::help::{self, HelpSection};
use crate::screens::components::keys::key;
use crate::screens::task_list_view::{DependencySummary, TaskListView};
pub use crate::screens::task_list_view::{FilterField, FilterState, TaskListMode};
use crate::screens::{AUTO_REFRESH_INTERVAL, Action, Component, Screen, UiContext};

pub struct VersionDashboard {
    repo: Arc<dyn Repository>,
    project_id: i64,
    version_id: i64,
    project_name: Option<String>,
    version: Option<Version>,
    summaries: Vec<VersionSummary>,
    view: TaskListView,
    /// Last state seen for `version_id`, used to detect a celebration.
    observed: Option<VersionMilestones>,
    /// Frame time at which the current celebration started, if any.
    celebration: Option<Duration>,
    /// A milestone was just reached; the next tick stamps its start time.
    pending_celebration: bool,
}

/// The milestones a version can celebrate: all its tasks done, or a summary
/// created by compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VersionMilestones {
    version_id: i64,
    complete: bool,
    compacted: bool,
}

impl VersionDashboard {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
        (key(KeyCode::Up), "Up"),
        (key(KeyCode::Down), "Down"),
        (key(KeyCode::Enter), "Edit"),
        (key(KeyCode::Tab), "Cycle"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('r')), "Refresh"),
        (key(KeyCode::Char('f')), "Filter"),
        (key(KeyCode::Char('x')), "Clear filters"),
        (key(KeyCode::Char('o')), "Sort"),
        (key(KeyCode::Char('m')), "Move"),
        (key(KeyCode::Char('s')), "Summary"),
        (key(KeyCode::Char('e')), "Export"),
        (key(KeyCode::Char('c')), "New"),
        (key(KeyCode::Esc), "Back"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            version_id: 0,
            project_name: None,
            version: None,
            summaries: Vec::new(),
            view: TaskListView::new(),
            observed: None,
            celebration: None,
            pending_celebration: false,
        }
    }

    pub fn set_ids(&mut self, project_id: i64, version_id: i64) {
        self.project_id = project_id;
        self.version_id = version_id;
    }

    pub fn project_id(&self) -> i64 {
        self.project_id
    }

    pub fn version_id(&self) -> i64 {
        self.version_id
    }

    pub fn selected_task(&self) -> usize {
        self.view.selected()
    }

    pub fn set_selected_task(&mut self, index: usize) {
        self.view.set_selected(index);
    }

    pub fn tasks(&self) -> &[Task] {
        self.view.tasks()
    }

    pub fn filter_status(&self) -> Option<Status> {
        self.view.filter_status()
    }

    pub fn filter_priority(&self) -> Option<Priority> {
        self.view.filter_priority()
    }

    /// Opens the move popup for the selected task with every other version of
    /// the project as destination.
    ///
    /// The popup is not opened when the version lookup fails: showing "this is
    /// the only version" would be a lie and would hide the failure. The error
    /// travels to the app as [`Action::ReportError`].
    fn open_move_popup(&mut self) -> Option<Action> {
        let versions = match application::versions::list(self.repo.as_ref(), self.project_id) {
            Ok(versions) => versions,
            Err(error) => {
                return Some(Action::ReportError {
                    message: error.user_message(),
                });
            }
        };
        let targets = versions
            .into_iter()
            .filter(|v| v.id != Some(self.version_id))
            .collect();
        self.view.open_move(targets);
        None
    }

    fn load_tasks(&mut self) -> anyhow::Result<()> {
        let version = application::versions::get(self.repo.as_ref(), self.version_id)?;
        self.project_id = version.project_id;
        self.project_name =
            Some(application::projects::get(self.repo.as_ref(), version.project_id)?.name);
        self.version = Some(version);
        self.summaries = application::summaries::list(self.repo.as_ref(), self.version_id)?;
        let tasks = application::tasks::list_by_version(self.repo.as_ref(), self.version_id)?;
        // Dependencies live in the same version, so their status is known
        // from the tasks just loaded.
        let statuses: std::collections::HashMap<i64, Status> = tasks
            .iter()
            .filter_map(|task| task.id.map(|id| (id, task.status)))
            .collect();
        let mut dependency_summaries = std::collections::HashMap::new();
        for id in tasks.iter().filter_map(|task| task.id) {
            let dependencies = application::dependencies::list(self.repo.as_ref(), id)?;
            let pending = dependencies
                .iter()
                .filter(|dep| statuses.get(dep) != Some(&Status::Done))
                .count();
            dependency_summaries.insert(
                id,
                DependencySummary {
                    total: dependencies.len(),
                    pending,
                },
            );
        }
        self.view.set_tasks(tasks);
        self.view.set_dependency_summaries(dependency_summaries);
        if self.observe_milestones() {
            self.pending_celebration = true;
        }
        Ok(())
    }

    /// Reports whether the version has just reached one of its milestones. The
    /// first state seen for a version is a baseline, so opening an already
    /// finished version does not celebrate. The frame loop stamps the actual
    /// start time in [`Component::tick`], the first place that has a clock.
    fn observe_milestones(&mut self) -> bool {
        let current = VersionMilestones {
            version_id: self.version_id,
            complete: self.is_complete(),
            compacted: !self.summaries.is_empty(),
        };
        let crossed = self.observed.is_some_and(|previous| {
            previous.version_id == current.version_id
                && ((!previous.complete && current.complete)
                    || (!previous.compacted && current.compacted))
        });
        self.observed = Some(current);
        crossed
    }

    /// `true` when the version has tasks and all of them are done.
    fn is_complete(&self) -> bool {
        let tasks = self.view.all_tasks();
        !tasks.is_empty() && tasks.iter().all(|task| task.status == Status::Done)
    }

    fn latest_summary_id(&self) -> Option<i64> {
        self.summaries.first().and_then(|s| s.id)
    }
}

impl Component for VersionDashboard {
    fn tick(&mut self, ui: &UiContext) {
        if self.pending_celebration {
            self.pending_celebration = false;
            self.celebration = ui.frame_time();
        }
    }

    fn init(&mut self) -> anyhow::Result<()> {
        self.load_tasks()?;
        self.view.set_mode(TaskListMode::Browsing);
        Ok(())
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.load_tasks()
    }

    /// Tasks are the subject here, so task actions go under Tasks.
    fn help_sections(&self) -> Vec<HelpSection> {
        help::sections(self.keybindings(), |label| match label {
            "Down" | "Up" => help::NAVIGATION,
            "New" | "Edit" | "Cycle" | "Delete" | "Filter" | "Clear filters" | "Sort" | "Move" => {
                help::TASKS
            }
            "Summary" | "Export" => help::VERSIONS,
            _ => help::APPLICATION,
        })
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn auto_refresh_interval(&self) -> Option<Duration> {
        self.view.is_browsing().then_some(AUTO_REFRESH_INTERVAL)
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            let code = *code;
            if code == KeyCode::Char('q') {
                return Some(Action::Quit);
            }
            if self.view.is_browsing() {
                match code {
                    KeyCode::Char('r') => return Some(Action::Refresh),
                    KeyCode::Char('m') => return self.open_move_popup(),
                    KeyCode::Char('c') => {
                        return Some(Action::Navigate(Screen::TaskForm {
                            version_id: self.version_id,
                            task_id: None,
                        }));
                    }
                    KeyCode::Char('s') => {
                        if let Some(summary_id) = self.latest_summary_id() {
                            return Some(Action::Navigate(Screen::VersionSummary {
                                project_id: self.project_id,
                                version_id: self.version_id,
                                summary_id,
                            }));
                        }
                        return None;
                    }
                    KeyCode::Char('e') => {
                        if let Some(summary_id) = self.latest_summary_id() {
                            return Some(Action::ExportVersionSummary { summary_id });
                        }
                        return None;
                    }
                    KeyCode::Esc | KeyCode::Backspace => {
                        return Some(Action::Navigate(Screen::ProjectDashboard {
                            project_id: self.project_id,
                        }));
                    }
                    _ => {}
                }
            }
            self.view.handle_key(code, &*self.repo)
        } else {
            None
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, ui: &UiContext) {
        let mut block = Block::default()
            .title(self.breadcrumb(ui))
            .title_bottom(self.view.table_title(ui))
            .borders(Borders::ALL);
        if ui.celebrating(self.celebration) {
            block = block.border_style(ui.success_style().add_modifier(Modifier::BOLD));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.view.render(frame, inner, ui);
    }
}

impl VersionDashboard {
    /// `Project › Version › Status`, plus a badge with the summary count when
    /// the version is released or already has summaries.
    fn breadcrumb(&self, ui: &UiContext) -> Line<'static> {
        let Some(version) = &self.version else {
            return Line::from("Version");
        };
        let separator = Span::styled(" › ", ui.muted_style());
        let mut spans = Vec::new();
        if let Some(project) = &self.project_name {
            spans.push(Span::styled(
                project.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(separator.clone());
        }
        spans.push(Span::styled(version.name.clone(), ui.text_style()));
        spans.push(separator);
        spans.push(Span::styled(
            version.status.label(),
            ui.version_status_style(version.status),
        ));
        if version.status == VersionStatus::Released || !self.summaries.is_empty() {
            let count = self.summaries.len();
            let label = if count == 1 {
                "1 summary".to_string()
            } else {
                format!("{count} summaries")
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(" {label} "),
                ui.accent_style().add_modifier(Modifier::REVERSED),
            ));
        }
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::repository::*;
    use std::sync::Arc;

    use crate::screens::components::motion;

    fn ui() -> UiContext {
        UiContext::default()
    }

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;
    use crate::screens::components::confirm_dialog::ConfirmDialog;
    use crate::screens::test_utils::{buffer_to_string, key_event, render_to_string};

    use super::*;

    fn setup_with_tasks() -> (Arc<StubRepository>, VersionDashboard, i64) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First project").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T1",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T2",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T3",
            "",
            Status::Todo,
            Priority::High,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T4",
            "",
            Status::InProgress,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T5",
            "",
            Status::Done,
            Priority::Medium,
        )
        .unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        // Versions are created in Backlog; this fixture exercises an active one.
        let version = repo.get_version(version_id).unwrap();
        repo.update_version(
            version_id,
            &version.name,
            &version.description,
            VersionStatus::Open,
        )
        .unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, version_id);
        screen.init().unwrap();
        (repo, screen, project_id)
    }

    #[test]
    fn task_list_keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = VersionDashboard::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![
                (KeyCode::Char('j'), "Down"),
                (KeyCode::Char('k'), "Up"),
                (KeyCode::Up, "Up"),
                (KeyCode::Down, "Down"),
                (KeyCode::Enter, "Edit"),
                (KeyCode::Tab, "Cycle"),
                (KeyCode::Char('d'), "Delete"),
                (KeyCode::Char('r'), "Refresh"),
                (KeyCode::Char('f'), "Filter"),
                (KeyCode::Char('x'), "Clear filters"),
                (KeyCode::Char('o'), "Sort"),
                (KeyCode::Char('m'), "Move"),
                (KeyCode::Char('s'), "Summary"),
                (KeyCode::Char('e'), "Export"),
                (KeyCode::Char('c'), "New"),
                (KeyCode::Esc, "Back"),
                (KeyCode::Char('q'), "Quit"),
            ]
        );
    }

    #[test]
    fn init_loads_project_and_tasks() {
        let (_repo, screen, _id) = setup_with_tasks();
        assert_eq!(screen.tasks().len(), 5);
        assert_eq!(screen.version.as_ref().map(|v| v.name.as_str()), Some("v0"));
    }

    #[test]
    fn init_clamps_selection_when_task_list_shrinks() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "Only",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, repo.default_version_id(project_id).unwrap());
        screen.view.set_selected(5);
        screen.init().unwrap();
        assert_eq!(screen.selected_task(), 0);
    }

    #[test]
    fn init_on_empty_project_renders_no_tasks_message() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let project_id = project.id.unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, repo.default_version_id(project_id).unwrap());
        screen.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("No tasks"), "{text}");
        assert!(text.contains("Press c to create one."), "{text}");
    }

    #[test]
    fn render_shows_version_header() {
        let (_repo, screen, _id) = setup_with_tasks();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Alpha › v0 › Open"));
    }

    #[test]
    fn empty_list_renders_no_tasks_message() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(
            project.id.unwrap(),
            repo.default_version_id(project.id.unwrap()).unwrap(),
        );
        screen.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("No tasks"), "{text}");
        assert!(text.contains("Press c to create one."), "{text}");
    }

    #[test]
    fn confirm_delete_without_dependents_renders_simple_message() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Delete task Leaf?",
                Action::DeleteTask { task_id: 1 },
            )));

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Delete task Leaf?"));
        assert!(text.contains("[y] Confirm"));
        assert!(text.contains("[n/Esc] Cancel"));
        assert!(!text.contains("dependency links"));
    }

    #[test]
    fn confirm_delete_with_dependents_renders_links_warning() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Deleting will remove 3 dependency links. Continue?",
                Action::DeleteTask { task_id: 1 },
            )));

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Deleting will remove 3 dependency links"));
    }

    #[test]
    fn list_rows_render_name_status_priority_columns() {
        let (_repo, screen, _id) = setup_with_tasks();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Tasks"));
        assert!(text.contains("Name"));
        assert!(text.contains("Status"));
        assert!(text.contains("Priority"));
        assert!(text.contains("Done"));
        assert!(text.contains("Todo"));
    }

    #[test]
    fn q_key_emits_quit_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('q')));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn t_key_navigates_to_new_task_form() {
        let (_repo, mut screen, _project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('c')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                version_id: screen.version_id(),
                task_id: None,
            }))
        );
    }

    #[test]
    fn s_key_navigates_to_version_summary_when_summary_exists() {
        let (repo, mut screen, project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        for task in screen.tasks().to_vec() {
            repo.update_task_status(task.id.unwrap(), Status::Done)
                .unwrap();
        }
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        let summary = repo
            .create_version_summary(version_id, "Summary", &snapshot)
            .unwrap();
        let summary_id = summary.id.unwrap();
        screen.init().unwrap();

        let action = screen.handle_event(&key_event(KeyCode::Char('s')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionSummary {
                project_id,
                version_id,
                summary_id,
            }))
        );
    }

    #[test]
    fn e_key_exports_when_summary_exists_and_does_nothing_otherwise() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        assert_eq!(screen.handle_event(&key_event(KeyCode::Char('e'))), None);

        let version_id = screen.version_id();
        for task in screen.tasks().to_vec() {
            repo.update_task_status(task.id.unwrap(), Status::Done)
                .unwrap();
        }
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        let summary = repo
            .create_version_summary(version_id, "Summary", &snapshot)
            .unwrap();
        let summary_id = summary.id.unwrap();
        screen.init().unwrap();

        let action = screen.handle_event(&key_event(KeyCode::Char('e')));
        assert_eq!(action, Some(Action::ExportVersionSummary { summary_id }));
    }

    #[test]
    fn released_header_shows_summary_count() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        for task in screen.tasks().to_vec() {
            repo.update_task_status(task.id.unwrap(), Status::Done)
                .unwrap();
        }
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.create_version_summary(version_id, "Summary", &snapshot)
            .unwrap();
        let version = repo.get_version(version_id).unwrap();
        repo.update_version(
            version_id,
            &version.name,
            &version.description,
            VersionStatus::Released,
        )
        .unwrap();
        screen.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Released"));
        assert!(text.contains("1 summary"));
    }

    #[test]
    fn open_header_shows_summary_count_with_pending_tasks() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        let first = screen.tasks()[0].id.unwrap();
        repo.update_task_status(first, Status::Done).unwrap();
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.create_version_summary(version_id, "Partial", &snapshot)
            .unwrap();
        screen.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area(), &ui()))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Alpha › v0 › Open"), "{text}");
        assert!(text.contains("1 summary"), "{text}");
        assert_eq!(
            repo.get_version(version_id).unwrap().status,
            VersionStatus::Open
        );
        assert!(!screen.tasks().is_empty(), "pending tasks must remain");
    }

    #[test]
    fn esc_navigates_to_dashboard() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
    }

    #[test]
    fn backspace_navigates_to_dashboard() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Backspace));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
    }

    #[test]
    fn up_clamps_at_top_bound() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.view.set_selected(1);
        screen.handle_event(&key_event(KeyCode::Up));
        assert_eq!(screen.selected_task(), 0);
        screen.handle_event(&key_event(KeyCode::Up));
        assert_eq!(screen.selected_task(), 0);
    }

    #[test]
    fn down_clamps_at_bottom_bound() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let last = screen.tasks().len() - 1;
        screen.view.set_selected(last);
        screen.handle_event(&key_event(KeyCode::Down));
        assert_eq!(screen.selected_task(), last);
    }

    #[test]
    fn enter_on_selected_navigates_to_edit_form() {
        let (_repo, mut screen, _project_id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                version_id: screen.version_id(),
                task_id: Some(task_id),
            }))
        );
    }

    #[test]
    fn tab_cycles_todo_to_inprogress() {
        let (_repo, mut screen, _project_id) = setup_with_tasks();
        screen.view.set_selected(0);
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(
            action,
            Some(Action::UpdateTaskStatus {
                task_id,
                new_status: Status::InProgress,
            })
        );
    }

    #[test]
    fn tab_cycles_done_to_todo() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let done_index = screen
            .tasks()
            .iter()
            .position(|t| t.status == Status::Done)
            .unwrap();
        screen.view.set_selected(done_index);
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(
            action,
            Some(Action::UpdateTaskStatus {
                task_id,
                new_status: Status::Todo,
            })
        );
    }

    #[test]
    fn tab_noop_when_empty_list() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let project_id = project.id.unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, repo.default_version_id(project_id).unwrap());
        screen.init().unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(action, None);
    }

    #[test]
    fn d_enters_confirm_mode_with_dependents_count() {
        let (repo, mut screen, _id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let dep = repo
            .create_task(
                screen.project_id,
                repo.default_version_id(screen.project_id).unwrap(),
                "Dep",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        repo.add_dependency(dep.id.unwrap(), task_id).unwrap();

        let action = screen.handle_event(&key_event(KeyCode::Char('d')));
        assert_eq!(action, None);
        match screen.view.mode() {
            TaskListMode::ConfirmDelete(dialog) => {
                assert_eq!(dialog.on_confirm(), &Action::DeleteTask { task_id });
                assert!(dialog.message().contains("1 dependency link"));
            }
            other => panic!("expected confirm mode, got {:?}", other),
        }
    }

    #[test]
    fn y_confirms_delete_and_emits_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        assert!(matches!(screen.view.mode(), TaskListMode::ConfirmDelete(_)));
        let action = screen.handle_event(&key_event(KeyCode::Char('y')));
        assert_eq!(action, Some(Action::DeleteTask { task_id }));
    }

    #[test]
    fn n_cancels_confirm_mode() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        let action = screen.handle_event(&key_event(KeyCode::Char('n')));
        assert_eq!(action, None);
        assert!(matches!(screen.view.mode(), TaskListMode::Browsing));
    }

    #[test]
    fn esc_cancels_confirm_mode() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        let action = screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(action, None);
        assert!(matches!(screen.view.mode(), TaskListMode::Browsing));
    }

    #[test]
    fn other_keys_keep_the_confirm_dialog_open() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        for code in [KeyCode::Enter, KeyCode::Char('x'), KeyCode::Char('Y')] {
            let action = screen.handle_event(&key_event(code));
            assert_eq!(action, None);
            assert!(matches!(screen.view.mode(), TaskListMode::ConfirmDelete(_)));
        }
    }

    #[test]
    fn long_task_list_scrolls_to_selected_task() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        for i in 0..30 {
            repo.create_task(
                project_id,
                version_id,
                &format!("Task{i:02}"),
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        }
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, version_id);
        screen.init().unwrap();

        let before = render_to_string(&screen, 60, 16);
        assert!(before.contains("Task00"));
        assert!(!before.contains("Task29"));

        for _ in 0..29 {
            screen.handle_event(&key_event(KeyCode::Down));
        }
        assert_eq!(screen.selected_task(), 29);
        let after = render_to_string(&screen, 60, 16);
        assert!(after.contains("Task29"), "selection not visible:\n{after}");
        assert!(
            !after.contains("Task00"),
            "top row should have scrolled out:\n{after}"
        );
    }

    #[test]
    fn version_dashboard_with_tasks_snapshot() {
        let (_repo, screen, _id) = setup_with_tasks();
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn version_dashboard_with_summary_snapshot() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        for task in screen.tasks().to_vec() {
            repo.update_task_status(task.id.unwrap(), Status::Done)
                .unwrap();
        }
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.create_version_summary(version_id, "Summary", &snapshot)
            .unwrap();
        let version = repo.get_version(version_id).unwrap();
        repo.update_version(
            version_id,
            &version.name,
            &version.description,
            VersionStatus::Released,
        )
        .unwrap();
        screen.init().unwrap();
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn version_dashboard_empty_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "No tasks here").unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(
            project.id.unwrap(),
            repo.default_version_id(project.id.unwrap()).unwrap(),
        );
        screen.init().unwrap();
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn version_dashboard_confirm_delete_leaf_snapshot() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                format!("Delete task {}?", screen.tasks()[0].name),
                Action::DeleteTask { task_id: 1 },
            )));
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn version_dashboard_confirm_delete_with_deps_snapshot() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Deleting will remove 1 dependency links. Continue?",
                Action::DeleteTask { task_id: 1 },
            )));
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn r_key_emits_refresh_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('r')));
        assert_eq!(action, Some(Action::Refresh));
    }

    #[test]
    fn f_key_enters_filtering_mode() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        assert!(matches!(screen.view.mode(), TaskListMode::Filtering(_)));
    }

    #[test]
    fn filter_enter_without_changes_keeps_all_tasks() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let initial_count = screen.tasks().len();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(screen.tasks().len(), initial_count);
    }

    #[test]
    fn filter_by_status_shows_only_requested() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered: Vec<Status> = screen.tasks().iter().map(|t| t.status).collect();
        assert!(filtered.iter().all(|s| *s == Status::Todo));
        assert!(filtered.len() < 5);
    }

    #[test]
    fn filter_by_priority_shows_only_requested() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Tab));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered: Vec<Priority> = screen.tasks().iter().map(|t| t.priority).collect();
        assert!(filtered.iter().all(|p| *p == Priority::High));
    }

    #[test]
    fn filter_by_both_shows_intersection() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Tab));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        for task in screen.tasks() {
            assert_eq!(task.status, Status::Todo);
            assert_eq!(task.priority, Priority::High);
        }
    }

    #[test]
    fn filter_esc_cancels_and_keeps_previous_filter() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered_count = screen.tasks().len();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(screen.tasks().len(), filtered_count);
    }

    #[test]
    fn filter_tab_switches_selected_field() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Tab));
        match screen.view.mode() {
            TaskListMode::Filtering(state) => {
                assert_eq!(state.selected_field, FilterField::Priority);
            }
            _ => panic!("expected Filtering mode"),
        }
    }

    #[test]
    fn filter_right_cycles_status_to_any() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        for _ in 0..4 {
            screen.handle_event(&key_event(KeyCode::Right));
        }
        match screen.view.mode() {
            TaskListMode::Filtering(state) => {
                assert_eq!(state.filter_status, None);
            }
            _ => panic!("expected Filtering mode"),
        }
    }

    #[test]
    fn filter_navigation_clamps_on_filtered_list() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        assert!(screen.tasks().len() > 1);
        screen.handle_event(&key_event(KeyCode::Down));
        assert_eq!(screen.selected_task(), 1);
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        assert!(screen.selected_task() < screen.tasks().len());
    }

    fn setup_with_second_version() -> (Arc<StubRepository>, VersionDashboard, i64, i64) {
        let (repo, screen, project_id) = setup_with_tasks();
        let other = repo
            .create_version(project_id, "v0.2.0", "")
            .unwrap()
            .id
            .unwrap();
        (repo, screen, project_id, other)
    }

    #[test]
    fn m_opens_move_popup_listing_other_versions() {
        let (_repo, mut screen, _pid, _other) = setup_with_second_version();
        assert!(
            screen
                .handle_event(&key_event(KeyCode::Char('m')))
                .is_none()
        );
        match screen.view.mode() {
            TaskListMode::MovingTask(state) => {
                assert_eq!(state.targets.len(), 1);
                assert_eq!(state.targets[0].name, "v0.2.0");
            }
            other => panic!("expected MovingTask, got {other:?}"),
        }
    }

    #[test]
    fn enter_in_move_popup_emits_move_action_for_selected_task() {
        let (_repo, mut screen, _pid, other) = setup_with_second_version();
        let task_id = screen.tasks()[0].id.unwrap();
        screen.handle_event(&key_event(KeyCode::Char('m')));
        let action = screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::MoveTask {
                task_id,
                version_id: other
            })
        );
        assert!(screen.view.is_browsing());
    }

    #[test]
    fn esc_in_move_popup_cancels_without_moving() {
        let (_repo, mut screen, _pid, _other) = setup_with_second_version();
        screen.handle_event(&key_event(KeyCode::Char('m')));
        assert!(screen.handle_event(&key_event(KeyCode::Esc)).is_none());
        assert!(screen.view.is_browsing());
    }

    #[test]
    fn esc_in_move_popup_does_not_navigate_back() {
        let (_repo, mut screen, _pid, _other) = setup_with_second_version();
        screen.handle_event(&key_event(KeyCode::Char('m')));
        let action = screen.handle_event(&key_event(KeyCode::Esc));
        assert!(!matches!(action, Some(Action::Navigate(_))));
    }

    #[test]
    fn move_popup_with_single_version_shows_message_and_moves_nothing() {
        let (_repo, mut screen, _pid) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('m')));
        let text = render_to_string(&screen, 60, 20);
        assert!(text.contains("This is the only version of the project."));
        assert!(screen.handle_event(&key_event(KeyCode::Enter)).is_none());
        assert!(screen.view.is_browsing());
    }

    #[test]
    fn move_popup_without_selected_task_does_nothing() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Empty", "").unwrap().id.unwrap();
        repo.create_version(project_id, "v0.2.0", "").unwrap();
        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(project_id, repo.default_version_id(project_id).unwrap());
        screen.init().unwrap();
        screen.handle_event(&key_event(KeyCode::Char('m')));
        assert!(screen.view.is_browsing());
    }

    fn complete_every_task(repo: &StubRepository, version_id: i64) {
        for task in repo.list_tasks_by_version(version_id).unwrap() {
            repo.update_task_status(task.id.unwrap(), Status::Done)
                .unwrap();
        }
    }

    fn ui_at(elapsed: Duration) -> UiContext {
        UiContext::default().with_frame_time(elapsed)
    }

    #[test]
    fn completing_the_last_task_celebrates_on_refresh() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        let start = Duration::from_secs(10);

        complete_every_task(&repo, version_id);
        screen.refresh().unwrap();
        screen.tick(&ui_at(start));
        assert!(ui_at(start).celebrating(screen.celebration));
        assert!(!ui_at(start + motion::CELEBRATION_DURATION).celebrating(screen.celebration));
    }

    #[test]
    fn opening_an_already_finished_version_does_not_celebrate() {
        let (repo, _screen, _project_id) = setup_with_tasks();
        let version_id = repo.default_version_id(1).unwrap();
        complete_every_task(&repo, version_id);

        let mut screen = VersionDashboard::new(repo.clone());
        screen.set_ids(1, version_id);
        screen.init().unwrap();
        screen.tick(&ui_at(Duration::from_secs(10)));

        assert_eq!(screen.celebration, None);
    }

    #[test]
    fn a_summary_created_by_compaction_celebrates() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        let version_id = screen.version_id();
        complete_every_task(&repo, version_id);
        screen.refresh().unwrap();
        screen.tick(&ui_at(Duration::from_secs(1)));
        assert!(screen.celebration.is_some());

        screen.celebration = None;
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.compact_version(version_id, "Summary", &snapshot)
            .unwrap();
        screen.refresh().unwrap();
        screen.tick(&ui_at(Duration::from_secs(2)));

        assert_eq!(screen.celebration, Some(Duration::from_secs(2)));
    }

    #[test]
    fn celebration_is_silent_without_animation() {
        let (repo, mut screen, _project_id) = setup_with_tasks();
        complete_every_task(&repo, screen.version_id());

        screen.refresh().unwrap();
        screen.tick(&UiContext::default());

        assert!(!UiContext::default().celebrating(screen.celebration));
    }
}

#[cfg(test)]
mod error_propagation_tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;

    use crate::db::repository::{
        ProjectRepository, RepositoryPolicy, TaskRepository, VersionRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::test_utils::{FaultyRepository, key_event};

    use super::*;

    /// Seeds a stub with one project, two versions and one task in the first.
    fn seeded() -> (StubRepository, i64, i64) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        repo.create_version(project_id, "v0.2.0", "").unwrap();
        repo.create_task(
            project_id,
            version_id,
            "T1",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
        (repo, project_id, version_id)
    }

    fn dashboard(repo: Arc<dyn Repository>, project_id: i64, version_id: i64) -> VersionDashboard {
        let mut screen = VersionDashboard::new(repo);
        screen.set_ids(project_id, version_id);
        screen
    }

    /// A failed version lookup must surface as an error action and must not
    /// open the popup, which would claim "this is the only version".
    #[test]
    fn move_popup_reports_the_error_instead_of_opening() {
        let (inner, project_id, version_id) = seeded();
        let repo = Arc::new(FaultyRepository {
            inner,
            fail_versions: true,
            ..FaultyRepository::default()
        });
        let mut screen = dashboard(repo.clone(), project_id, version_id);
        screen.init().unwrap();

        let action = screen.handle_event(&key_event(KeyCode::Char('m')));

        assert!(
            matches!(action, Some(Action::ReportError { .. })),
            "expected an error action, got {action:?}"
        );
        assert!(
            screen.view.is_browsing(),
            "the move popup must not open without the target list"
        );
    }

    /// A failed dependency query must fail the load instead of rendering the
    /// list with misleading (zero) dependency markers.
    #[test]
    fn dependency_query_failure_fails_the_load() {
        let (inner, project_id, version_id) = seeded();
        let repo = Arc::new(FaultyRepository {
            inner,
            fail_dependencies: true,
            ..FaultyRepository::default()
        });
        let mut screen = dashboard(repo, project_id, version_id);

        assert!(screen.init().is_err());
    }

    /// The same load succeeds when the query works, so the test above fails
    /// for the right reason.
    #[test]
    fn dependency_markers_load_when_the_query_works() {
        let (inner, project_id, version_id) = seeded();
        let repo = Arc::new(FaultyRepository {
            inner,
            ..FaultyRepository::default()
        });
        let mut screen = dashboard(repo, project_id, version_id);

        assert!(screen.init().is_ok());
    }
}
