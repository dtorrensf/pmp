// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::db::repository::Repository;
use crate::screens::components::toast::{Severity, Toasts};
use crate::screens::{
    Action, Component, ProjectDashboard, ProjectForm, ProjectListScreen, Screen, TaskForm,
    VersionDashboard, VersionForm, VersionSummaryScreen,
};

mod outcome;
mod project_actions;
mod summary_actions;
mod task_actions;
mod version_actions;

use outcome::Outcome;

/// Returns the current time. Boxed so tests can substitute their own clock.
type Clock = Box<dyn Fn() -> Instant + Send>;

pub struct App {
    pub screen: Screen,
    pub previous_screen: Option<Screen>,
    pub repo: Arc<dyn Repository>,
    pub action: Option<Action>,
    /// Feedback messages shown above the footer. See [`Toasts`].
    toasts: Toasts,
    /// Source of the current time for toast expiry. Overridable from tests.
    clock: Clock,
    /// Newer pmp version reported by the background update check, if any.
    update_available: Option<String>,
    /// Whether the update banner is currently open.
    show_update: bool,
    /// Whether the `?` shortcut overlay is open. While it is, every key goes
    /// to the overlay, so the screen behind it does not react.
    show_help: bool,
    pending_update: Option<Arc<OnceLock<String>>>,
    update_check_cancel: Option<Arc<AtomicBool>>,
    /// Version compiled into the running binary, used in the update banner.
    /// Defaults to `CARGO_PKG_VERSION` and is overridable from tests.
    current_version: String,
    components: Components,
}

struct Components {
    project_list: ProjectListScreen,
    dashboard: ProjectDashboard,
    project_form: ProjectForm,
    task_form: TaskForm,
    version_dashboard: VersionDashboard,
    version_form: VersionForm,
    version_summary: VersionSummaryScreen,
}

impl App {
    pub fn new(repo: Arc<dyn Repository>) -> Self {
        let mut app = Self {
            screen: Screen::ProjectList,
            previous_screen: None,
            repo: repo.clone(),
            action: None,
            toasts: Toasts::default(),
            clock: Box::new(Instant::now),
            update_available: None,
            show_update: false,
            show_help: false,
            pending_update: None,
            update_check_cancel: None,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            components: Components {
                project_list: ProjectListScreen::new(repo.clone()),
                dashboard: ProjectDashboard::new(repo.clone()),
                project_form: ProjectForm::new(repo.clone()),
                task_form: TaskForm::new(repo.clone()),
                version_dashboard: VersionDashboard::new(repo.clone()),
                version_form: VersionForm::new(repo.clone()),
                version_summary: VersionSummaryScreen::new(repo.clone()),
            },
        };
        if let Err(e) = app.init_screen(&Screen::ProjectList) {
            app.push_error(e.to_string());
        }
        app
    }

    /// Registers the slot filled by the background update check.
    pub fn set_pending_update(&mut self, update_check: (Arc<OnceLock<String>>, Arc<AtomicBool>)) {
        let (slot, cancel) = update_check;
        self.pending_update = Some(slot);
        self.update_check_cancel = Some(cancel);
    }

    pub(crate) fn update_check_cancel_handle(&self) -> Option<Arc<AtomicBool>> {
        self.update_check_cancel.clone()
    }

    /// Copies the background update result, if already available, into
    /// `update_available`.
    pub fn poll_update(&mut self) {
        if let Some(version) = self.pending_update.as_ref().and_then(|slot| slot.get()) {
            self.update_available = Some(version.clone());
        }
    }

    /// Opens the update banner. No-op when no update is available.
    pub fn open_update_banner(&mut self) {
        if self.update_available.is_some() {
            self.show_update = true;
        }
    }

    pub fn close_update_banner(&mut self) {
        self.show_update = false;
    }

    pub fn set_update_available(&mut self, version: String) {
        self.update_available = Some(version);
    }

    pub fn has_update(&self) -> bool {
        self.update_available.is_some()
    }

    pub fn update_version(&self) -> Option<&str> {
        self.update_available.as_deref()
    }

    /// The toast queue, for rendering and inspection.
    pub fn toasts(&self) -> &Toasts {
        &self.toasts
    }

    /// Text of the most recent error still on the queue, if any.
    pub fn error(&self) -> Option<&str> {
        self.toasts.error()
    }

    /// Replaces the clock used for toast expiry. Lets tests advance time
    /// without sleeping.
    pub fn set_clock(&mut self, clock: impl Fn() -> Instant + Send + 'static) {
        self.clock = Box::new(clock);
    }

    /// Drops the toasts whose time is up. Called once per event-loop tick.
    pub fn tick(&mut self) {
        let now = (self.clock)();
        self.toasts.tick(now);
    }

    /// How long the toast on screen has been queued, if there is one.
    pub fn toast_age(&self) -> Option<std::time::Duration> {
        let now = (self.clock)();
        self.toasts.current().map(|toast| toast.age(now))
    }

    /// Handles `Esc` while an error toast is on screen: closes it and consumes
    /// the key, so the screen behind does not also go back. Returns `true`
    /// when the key was consumed.
    pub fn handle_toast_key(&mut self, code: KeyCode) -> bool {
        let showing_error = self
            .toasts
            .current()
            .is_some_and(|toast| toast.severity == Severity::Error);
        if code == KeyCode::Esc && showing_error {
            self.toasts.dismiss_current();
            return true;
        }
        false
    }

    fn push_error(&mut self, message: impl Into<String>) {
        let now = (self.clock)();
        self.toasts.push(Severity::Error, message, now);
    }

    fn push_success(&mut self, message: impl Into<String>) {
        let now = (self.clock)();
        self.toasts.push(Severity::Success, message, now);
    }

    /// Context of the active screen as breadcrumb segments, outermost first.
    /// Names that can no longer be loaded fall back to a generic label.
    pub fn breadcrumb(&self) -> Vec<String> {
        let project = |id: i64| {
            self.repo
                .get_project(id)
                .map(|p| p.name)
                .unwrap_or_else(|_| "Project".to_string())
        };
        let version = |id: i64| {
            self.repo
                .get_version(id)
                .map(|v| (v.project_id, v.name))
                .unwrap_or_else(|_| (0, "Version".to_string()))
        };
        let crumbs: Vec<&str> = match &self.screen {
            Screen::ProjectList => return vec!["Projects".to_string()],
            Screen::ProjectDashboard { project_id } => return vec![project(*project_id)],
            Screen::ProjectForm { project_id: None } => vec!["Projects", "New project"],
            Screen::ProjectForm {
                project_id: Some(id),
            } => return vec![project(*id), "Edit project".to_string()],
            Screen::TaskForm {
                version_id,
                task_id,
            } => {
                let (project_id, version_name) = version(*version_id);
                let task = if task_id.is_some() {
                    "Edit task"
                } else {
                    "New task"
                };
                return vec![project(project_id), version_name, task.to_string()];
            }
            Screen::VersionDashboard {
                project_id,
                version_id,
            } => {
                return vec![project(*project_id), version(*version_id).1];
            }
            Screen::VersionForm {
                project_id,
                version_id,
            } => {
                let label = if version_id.is_some() {
                    "Edit version"
                } else {
                    "New version"
                };
                return vec![project(*project_id), label.to_string()];
            }
            Screen::VersionSummary {
                project_id,
                version_id,
                ..
            } => {
                return vec![
                    project(*project_id),
                    version(*version_id).1,
                    "Summary".to_string(),
                ];
            }
        };
        crumbs.into_iter().map(str::to_string).collect()
    }

    /// Returns the version compiled into the running binary, used in the
    /// update banner as the "current" version the user is running.
    pub fn current_version(&self) -> &str {
        &self.current_version
    }

    /// Overrides the version reported as "current" in the update banner.
    /// Intended for tests that need a deterministic value independent of
    /// whatever `CARGO_PKG_VERSION` happens to be at compile time.
    pub fn set_current_version(&mut self, version: &str) {
        self.current_version = version.to_string();
    }

    pub fn is_update_banner_open(&self) -> bool {
        self.show_update
    }

    /// Returns `true` when the key belongs to update-banner handling.
    pub fn handle_update_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.show_update {
            if code == KeyCode::Esc {
                self.close_update_banner();
            }
            return true;
        }
        if code == KeyCode::Char('u')
            && modifiers == KeyModifiers::NONE
            && self.has_update()
            && !self.screen.accepts_text_input()
        {
            self.open_update_banner();
            return true;
        }
        false
    }

    /// Returns `true` when the `?` shortcut overlay is open.
    pub fn is_help_open(&self) -> bool {
        self.show_help
    }

    /// Handles the keys of the `?` overlay. `?` opens it on screens without a
    /// text field (on forms it is typed text). While it is open, `?` and `Esc`
    /// close it and every other key is swallowed. Returns `true` when the key
    /// was consumed.
    pub fn handle_help_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.show_help {
            if matches!(code, KeyCode::Char('?') | KeyCode::Esc) {
                self.show_help = false;
            }
            return true;
        }
        let plain_or_shift = modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT;
        if code == KeyCode::Char('?') && plain_or_shift && !self.screen.accepts_text_input() {
            self.show_help = true;
            return true;
        }
        false
    }

    pub fn dispatch(&mut self, action: Action) -> Option<Action> {
        // A real action supersedes the previous failure. Refresh is exempt: it
        // runs on a timer and must not hide an error the user has not read.
        if !matches!(action, Action::Refresh | Action::Noop | Action::Quit) {
            self.toasts.clear_errors();
        }
        match action {
            Action::Navigate(screen) => {
                self.navigate(screen);
                None
            }
            Action::SaveProject {
                project_id,
                name,
                description,
            } => {
                let return_to = self.components.project_form.return_to().clone();
                let result = project_actions::save(
                    self.repo.as_ref(),
                    project_id,
                    &name,
                    &description,
                    &return_to,
                );
                self.settle(result)
            }
            Action::SaveTask {
                task_id,
                version_id,
                name,
                description,
                status,
                priority,
                depends_on_ids,
            } => {
                let return_to = self.components.task_form.return_to().clone();
                let result = task_actions::save(
                    self.repo.as_ref(),
                    task_id,
                    version_id,
                    &name,
                    &description,
                    status,
                    priority,
                    &depends_on_ids,
                    &return_to,
                );
                self.settle(result)
            }
            Action::UpdateTaskStatus {
                task_id,
                new_status,
            } => {
                // Only dashboards list tasks, so only they can change a status.
                if !self.on_dashboard() {
                    return None;
                }
                let refresh = self.screen.clone();
                let result =
                    task_actions::update_status(self.repo.as_ref(), task_id, new_status, refresh);
                self.settle(result)
            }
            Action::SaveVersion {
                version_id,
                project_id,
                name,
                description,
                status,
            } => {
                let result = version_actions::save(
                    self.repo.as_ref(),
                    version_id,
                    project_id,
                    &name,
                    &description,
                    status,
                );
                self.settle(result)
            }
            Action::DeleteVersion { version_id } => {
                let result = version_actions::delete(self.repo.as_ref(), version_id);
                self.settle(result)
            }
            Action::DeleteProject { project_id } => {
                let result = project_actions::delete(self.repo.as_ref(), project_id);
                self.settle(result)
            }
            Action::MoveTask {
                task_id,
                version_id,
            } => {
                // Moving only makes sense from the version it leaves.
                if !matches!(self.screen, Screen::VersionDashboard { .. }) {
                    return None;
                }
                let refresh = self.screen.clone();
                let result =
                    task_actions::move_to_version(self.repo.as_ref(), task_id, version_id, refresh);
                self.settle(result)
            }
            Action::DeleteTask { task_id } => {
                if !self.on_dashboard() {
                    return None;
                }
                let refresh = self.screen.clone();
                let result = task_actions::delete(self.repo.as_ref(), task_id, refresh);
                self.settle(result)
            }
            Action::Refresh => {
                if let Err(e) = self.active_component().refresh() {
                    self.push_error(e.to_string());
                }
                None
            }
            Action::AddTaskNote { task_id, content } => {
                let result = task_actions::add_note(self.repo.as_ref(), task_id, &content);
                self.settle(result)
            }
            Action::ExportVersionSummary { summary_id } => {
                let result = summary_actions::export(self.repo.as_ref(), summary_id);
                self.settle(result)
            }
            Action::Quit => None,
            Action::Noop => None,
        }
    }

    /// Turns a use case result into user feedback: a confirmation on success,
    /// an error toast on failure. Returns the follow-up action, if any.
    fn settle(&mut self, result: anyhow::Result<Outcome>) -> Option<Action> {
        match result {
            Ok(outcome) => {
                if let Some(message) = outcome.confirmation {
                    self.push_success(message);
                }
                outcome.next
            }
            Err(e) => {
                self.push_error(e.to_string());
                None
            }
        }
    }

    /// Applies a navigation, restoring the previous screen if it cannot load.
    fn navigate(&mut self, screen: Screen) {
        let previous = self.screen.clone();
        match self.init_screen(&screen) {
            Ok(()) => {
                if matches!(screen, Screen::TaskForm { .. }) {
                    self.components.task_form.set_return_to(&previous);
                }
                if matches!(screen, Screen::ProjectForm { .. }) {
                    self.components.project_form.set_return_to(&previous);
                }
                if matches!(screen, Screen::VersionForm { .. }) {
                    self.components.version_form.set_return_to(&previous);
                }
                self.previous_screen = Some(previous);
                self.screen = screen;
            }
            Err(e) => {
                self.push_error(e.to_string());
                self.screen = self.previous_screen.take().unwrap_or(Screen::ProjectList);
            }
        }
    }

    fn on_dashboard(&self) -> bool {
        matches!(
            self.screen,
            Screen::ProjectDashboard { .. } | Screen::VersionDashboard { .. }
        )
    }

    fn init_screen(&mut self, screen: &Screen) -> anyhow::Result<()> {
        match screen {
            Screen::ProjectList => self.components.project_list.init(),
            Screen::ProjectDashboard { project_id } => {
                self.components.dashboard.set_project_id(*project_id);
                self.components.dashboard.init()
            }
            Screen::ProjectForm { project_id } => {
                self.components.project_form.set_project_id(*project_id);
                self.components.project_form.init()
            }
            Screen::TaskForm {
                version_id,
                task_id,
            } => {
                self.components.task_form.set_version_id(*version_id);
                self.components.task_form.set_task_id(*task_id);
                self.components.task_form.init()
            }
            Screen::VersionDashboard {
                project_id,
                version_id,
            } => {
                self.components
                    .version_dashboard
                    .set_ids(*project_id, *version_id);
                self.components.version_dashboard.init()
            }
            Screen::VersionForm {
                project_id,
                version_id,
            } => {
                self.components.version_form.set_project_id(*project_id);
                self.components.version_form.set_version_id(*version_id);
                self.components.version_form.init()
            }
            Screen::VersionSummary {
                project_id,
                version_id,
                summary_id,
            } => {
                self.components
                    .version_summary
                    .set_ids(*project_id, *version_id, *summary_id);
                self.components.version_summary.init()
            }
        }
    }

    pub fn active_component(&mut self) -> &mut dyn Component {
        match &self.screen {
            Screen::ProjectList => &mut self.components.project_list,
            Screen::ProjectDashboard { project_id } => {
                self.components.dashboard.set_project_id(*project_id);
                &mut self.components.dashboard
            }
            Screen::ProjectForm { project_id } => {
                self.components.project_form.set_project_id(*project_id);
                &mut self.components.project_form
            }
            Screen::TaskForm {
                version_id,
                task_id,
            } => {
                self.components.task_form.set_version_id(*version_id);
                self.components.task_form.set_task_id(*task_id);
                &mut self.components.task_form
            }
            Screen::VersionDashboard {
                project_id,
                version_id,
            } => {
                self.components
                    .version_dashboard
                    .set_ids(*project_id, *version_id);
                &mut self.components.version_dashboard
            }
            Screen::VersionForm {
                project_id,
                version_id,
            } => {
                self.components.version_form.set_project_id(*project_id);
                self.components.version_form.set_version_id(*version_id);
                &mut self.components.version_form
            }
            Screen::VersionSummary {
                project_id,
                version_id,
                summary_id,
            } => {
                self.components
                    .version_summary
                    .set_ids(*project_id, *version_id, *summary_id);
                &mut self.components.version_summary
            }
        }
    }
}

#[cfg(test)]
mod breadcrumb_tests {
    use std::sync::Arc;

    use crate::db::repository::{ProjectRepository, RepositoryPolicy, VersionRepository};
    use crate::db::stub::StubRepository;
    use crate::screens::Screen;

    use super::App;

    fn app_with_project() -> (Arc<StubRepository>, App, i64, i64) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let app = App::new(repo.clone());
        (repo, app, project_id, version_id)
    }

    #[test]
    fn project_list_breadcrumb_is_the_root() {
        let (_repo, app, _, _) = app_with_project();
        assert_eq!(app.breadcrumb(), vec!["Projects"]);
    }

    #[test]
    fn project_dashboard_breadcrumb_shows_the_project_name() {
        let (_repo, mut app, project_id, _) = app_with_project();
        app.screen = Screen::ProjectDashboard { project_id };
        assert_eq!(app.breadcrumb(), vec!["Alpha"]);
    }

    #[test]
    fn version_dashboard_breadcrumb_shows_project_then_version() {
        let (repo, mut app, project_id, version_id) = app_with_project();
        let version_name = repo.get_version(version_id).unwrap().name;
        app.screen = Screen::VersionDashboard {
            project_id,
            version_id,
        };
        assert_eq!(app.breadcrumb(), vec!["Alpha".to_string(), version_name]);
    }

    #[test]
    fn task_form_breadcrumb_adds_the_action() {
        let (_repo, mut app, _, version_id) = app_with_project();
        app.screen = Screen::TaskForm {
            version_id,
            task_id: None,
        };
        let crumbs = app.breadcrumb();
        assert_eq!(crumbs.first().map(String::as_str), Some("Alpha"));
        assert_eq!(crumbs.last().map(String::as_str), Some("New task"));
        assert_eq!(crumbs.len(), 3);
    }

    #[test]
    fn missing_project_falls_back_to_generic_label() {
        let (_repo, mut app, _, _) = app_with_project();
        app.screen = Screen::ProjectDashboard { project_id: 999 };
        assert_eq!(app.breadcrumb(), vec!["Project"]);
    }
}

#[cfg(test)]
mod tests {
    use crate::db::repository::RepositoryPolicy;
    use std::sync::Arc;

    use crossterm::event::KeyCode;

    use crate::db::repository::{
        DependencyRepository, NoteRepository, ProjectRepository, SummaryRepository, TaskRepository,
        VersionRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;
    use crate::screens::test_utils::{char_event, key_event, render_to_string};
    use crate::screens::{Action, Screen};

    use super::*;

    fn setup_app() -> (Arc<StubRepository>, App) {
        let repo = Arc::new(StubRepository::default());
        let app = App::new(repo.clone());
        (repo, app)
    }

    #[test]
    fn app_starts_at_entry_screen() {
        let (_repo, app) = setup_app();
        assert_eq!(app.screen, Screen::ProjectList);
        assert!(app.error().is_none());
    }

    #[test]
    fn navigate_sets_previous_screen() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        assert_eq!(app.screen, Screen::ProjectDashboard { project_id });
        assert_eq!(app.previous_screen, Some(Screen::ProjectList));
        assert!(app.error().is_none());
    }

    #[test]
    fn navigate_to_invalid_project_restores_previous_screen() {
        let (_repo, mut app) = setup_app();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard {
            project_id: 99,
        }));
        assert_eq!(app.screen, Screen::ProjectList);
        assert!(app.error().is_some());
    }

    #[test]
    fn create_project_navigates_to_dashboard_on_success() {
        let (_repo, mut app) = setup_app();
        let next = app.dispatch(Action::SaveProject {
            project_id: None,
            name: "Alpha".to_string(),
            description: "".to_string(),
        });
        assert!(
            matches!(
                next,
                Some(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }))
            ),
            "expected navigate to dashboard, got {:?}",
            next
        );
        // applying the internal navigate action moves the screen
        if let Some(action) = next {
            app.dispatch(action);
        }
        assert_eq!(app.screen, Screen::ProjectDashboard { project_id: 1 });
    }

    #[test]
    fn create_project_empty_name_sets_error() {
        let (_repo, mut app) = setup_app();
        let next = app.dispatch(Action::SaveProject {
            project_id: None,
            name: "   ".to_string(),
            description: "".to_string(),
        });
        assert_eq!(next, None);
        assert_eq!(app.error(), Some("Project name cannot be empty"));
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn save_task_navigates_to_dashboard_on_success() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: None,
        }));
        let next = app.dispatch(Action::SaveTask {
            task_id: None,
            version_id: app.repo.default_version_id(project_id).unwrap(),
            name: "Task 1".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![],
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::ProjectDashboard { project_id: id })) if id == project_id),
            "expected navigate to dashboard, got {:?}",
            next
        );
    }

    #[test]
    fn save_task_empty_name_sets_error() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let next = app.dispatch(Action::SaveTask {
            task_id: None,
            version_id: app.repo.default_version_id(project_id).unwrap(),
            name: "".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![],
        });
        assert_eq!(next, None);
        assert_eq!(app.error(), Some("Task name cannot be empty"));
    }

    #[test]
    fn update_task_status_to_done_with_incomplete_deps_is_blocked() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        repo.add_dependency(a.id.unwrap(), b.id.unwrap()).unwrap();

        // UpdateTaskStatus is dispatched from the dashboard (Tab in the version task list).
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::UpdateTaskStatus {
            task_id: a.id.unwrap(),
            new_status: Status::Done,
        });
        assert_eq!(
            app.error(),
            Some("cannot mark task as done: it has incomplete dependencies")
        );

        // transition was blocked — task stays Todo
        let task = repo.get_task(a.id.unwrap()).unwrap();
        assert_eq!(task.status, Status::Todo);
    }

    #[test]
    fn update_task_status_to_done_without_deps_clears_error() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();

        // UpdateTaskStatus is dispatched from the dashboard (Tab in the version task list).
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::UpdateTaskStatus {
            task_id: a.id.unwrap(),
            new_status: Status::Done,
        });
        assert!(app.error().is_none());
    }

    // SC3: cycling a task's status must refresh the dashboard's cached list.
    // `dispatch(UpdateTaskStatus)` returns a Navigate back to the dashboard so
    // `init_screen` re-runs `dashboard.init()` and re-reads `list_tasks_by_project`.
    #[test]
    fn update_task_status_dispatch_returns_navigate_to_refresh_dashboard() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        // sanity: list is cached at Todo
        assert_eq!(
            app.components.version_dashboard.tasks()[0].status,
            Status::Todo
        );

        let next = app.dispatch(Action::UpdateTaskStatus {
            task_id,
            new_status: Status::InProgress,
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::VersionDashboard { project_id: id, .. })) if id == project_id),
            "expected navigate to dashboard, got {:?}",
            next
        );

        // apply the returned navigate -> init_screen -> dashboard.init() refresh
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(
            app.components.version_dashboard.tasks()[0].status,
            Status::InProgress,
            "dashboard cached list must reflect the cycled status"
        );
        // selection preserved across refresh
        assert_eq!(app.components.version_dashboard.selected_task(), 0);
        assert!(app.error().is_none());
    }

    #[test]
    fn save_task_from_task_list_returns_to_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: None,
        }));
        assert_eq!(
            app.screen,
            Screen::TaskForm {
                version_id: app.repo.default_version_id(project_id).unwrap(),
                task_id: None,
            }
        );

        let next = app.dispatch(Action::SaveTask {
            task_id: None,
            version_id: app.repo.default_version_id(project_id).unwrap(),
            name: "Task 1".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![],
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::VersionDashboard { project_id: id, .. })) if id == project_id),
            "expected navigate back to task list, got {:?}",
            next
        );
    }

    #[test]
    fn save_task_with_task_id_calls_update_task() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "Old",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: Some(task_id),
        }));
        let next = app.dispatch(Action::SaveTask {
            task_id: Some(task_id),
            version_id: app.repo.default_version_id(project_id).unwrap(),
            name: "New".to_string(),
            description: "".to_string(),
            status: Status::InProgress,
            priority: Priority::High,
            depends_on_ids: vec![],
        });

        assert!(
            matches!(next, Some(Action::Navigate(Screen::ProjectDashboard { project_id: id })) if id == project_id),
            "expected navigate to dashboard, got {:?}",
            next
        );
        let updated = repo.get_task(task_id).unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.status, Status::InProgress);
        assert_eq!(updated.priority, Priority::High);
    }

    #[test]
    fn save_task_with_task_id_and_done_status_blocked_by_incomplete_deps() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(a_id, b_id).unwrap();

        app.dispatch(Action::SaveTask {
            task_id: Some(a_id),
            version_id: app.repo.default_version_id(project_id).unwrap(),
            name: "A".to_string(),
            description: "".to_string(),
            status: Status::Done,
            priority: Priority::Low,
            depends_on_ids: vec![b_id],
        });

        assert_eq!(
            app.error(),
            Some("cannot mark task as done: it has incomplete dependencies")
        );
        // transition was blocked — task stays Todo
        let task = repo.get_task(a_id).unwrap();
        assert_eq!(task.status, Status::Todo);
    }

    #[test]
    fn delete_task_dispatch_calls_repo_delete_task() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));

        let next = app.dispatch(Action::DeleteTask { task_id });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::ProjectDashboard { project_id: id })) if id == project_id),
            "expected navigate to dashboard, got {:?}",
            next
        );
        assert!(repo.get_task(task_id).is_err());
    }

    #[test]
    fn update_task_status_from_task_list_returns_to_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        assert_eq!(
            app.screen,
            Screen::VersionDashboard {
                project_id,
                version_id: app.repo.default_version_id(project_id).unwrap()
            }
        );

        let next = app.dispatch(Action::UpdateTaskStatus {
            task_id,
            new_status: Status::InProgress,
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::VersionDashboard { project_id: id, .. })) if id == project_id),
            "expected navigate back to task list, got {:?}",
            next
        );

        if let Some(action) = next {
            app.dispatch(action);
        }
        assert_eq!(
            app.components.version_dashboard.tasks()[0].status,
            Status::InProgress
        );
    }

    #[test]
    fn delete_task_from_task_list_returns_to_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));

        let next = app.dispatch(Action::DeleteTask { task_id });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::VersionDashboard { project_id: id, .. })) if id == project_id),
            "expected navigate back to task list, got {:?}",
            next
        );
        assert!(repo.get_task(task_id).is_err());
    }

    #[test]
    fn delete_task_clamps_selection_to_new_last() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let t1 = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let t2 = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T2",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let t3 = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T3",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        app.components.version_dashboard.set_selected_task(2);

        let next = app.dispatch(Action::DeleteTask {
            task_id: t3.id.unwrap(),
        });
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(app.components.version_dashboard.selected_task(), 1);
        assert!(repo.get_task(t3.id.unwrap()).is_err());
        assert!(repo.get_task(t1.id.unwrap()).is_ok());
        assert!(repo.get_task(t2.id.unwrap()).is_ok());
    }

    #[test]
    fn navigate_to_task_list_sets_screen_and_loads_tasks() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
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

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));

        assert_eq!(
            app.screen,
            Screen::VersionDashboard {
                project_id,
                version_id: app.repo.default_version_id(project_id).unwrap()
            }
        );
        assert!(app.error().is_none());
        assert_eq!(app.components.version_dashboard.tasks().len(), 1);
    }

    #[test]
    fn active_component_returns_task_list_keybindings() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));

        let keys: Vec<_> = app
            .active_component()
            .keybindings()
            .iter()
            .map(|(k, _)| k.code)
            .collect();
        assert!(keys.contains(&KeyCode::Char('d')));
        assert!(keys.contains(&KeyCode::Tab));
    }

    #[test]
    fn refresh_reloads_entry_screen_projects() {
        let (repo, mut app) = setup_app();
        assert_eq!(app.screen, Screen::ProjectList);
        assert!(app.components.project_list.projects().is_empty());

        repo.create_project("Alpha", "").unwrap();
        app.dispatch(Action::Refresh);

        assert_eq!(app.components.project_list.projects().len(), 1);
        assert!(app.error().is_none());
    }

    #[test]
    fn refresh_reloads_dashboard_tasks() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        assert!(app.components.version_dashboard.tasks().is_empty());

        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T1",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        app.dispatch(Action::Refresh);

        assert_eq!(app.components.version_dashboard.tasks().len(), 1);
        assert!(app.error().is_none());
    }

    #[test]
    fn refresh_reloads_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: app.repo.default_version_id(project_id).unwrap(),
        }));
        assert!(app.components.version_dashboard.tasks().is_empty());

        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "T1",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        app.dispatch(Action::Refresh);

        assert_eq!(app.components.version_dashboard.tasks().len(), 1);
        assert!(app.error().is_none());
    }

    #[test]
    fn add_task_note_success_refreshes_current_screen() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: Some(task_id),
        }));
        let expected_return_to = app.components.task_form.return_to().clone();
        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "A note".to_string(),
        });

        assert_eq!(next, Some(Action::Refresh));
        assert!(app.error().is_none());
        assert_eq!(repo.list_task_notes(task_id).unwrap().len(), 1);
        assert_eq!(app.components.task_form.return_to(), &expected_return_to);
    }

    #[test]
    fn add_task_note_empty_content_sets_error() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "   ".to_string(),
        });

        assert_eq!(next, None);
        assert_eq!(app.error(), Some("Note content cannot be empty"));
    }

    #[test]
    fn add_task_note_then_reopen_form_shows_note() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: Some(task_id),
        }));

        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "My note".to_string(),
        });
        assert_eq!(next, Some(Action::Refresh));
        if let Some(action) = next {
            app.dispatch(action);
        }

        // Navigate away and reopen the task form to prove the note persists.
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: Some(task_id),
        }));

        let notes = app.components.task_form.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].task_id, task_id);
        assert_eq!(notes[0].content, "My note");
    }

    #[test]
    fn refresh_preserves_filter_dialog_and_reloads_tasks() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id,
            version_id: repo.default_version_id(project_id).unwrap(),
        }));

        // Open the filter dialog.
        app.components
            .version_dashboard
            .handle_event(&key_event(KeyCode::Char('f')));
        let before = render_to_string(&app.components.version_dashboard, 60, 24);
        assert!(before.contains("Filter Tasks"));

        // Create a new task in the repo while the dialog is open.
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "NewTask",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();

        // Refresh must preserve the dialog and reload the data.
        app.dispatch(Action::Refresh);

        let after = render_to_string(&app.components.version_dashboard, 60, 24);
        assert!(after.contains("Filter Tasks"));
        assert!(
            app.components
                .version_dashboard
                .tasks()
                .iter()
                .any(|t| t.name == "NewTask")
        );
    }

    #[test]
    fn refresh_after_add_task_note_preserves_form_state() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "Task",
                "desc",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: Some(task_id),
        }));

        // Mutate transient UI state through real key events.
        app.components.task_form.handle_event(&char_event(' '));
        app.components.task_form.handle_event(&char_event('e'));
        app.components.task_form.handle_event(&char_event('d'));
        app.components.task_form.handle_event(&char_event('i'));
        app.components.task_form.handle_event(&char_event('t'));
        app.components.task_form.handle_event(&char_event('e'));
        app.components.task_form.handle_event(&char_event('d'));
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Tab)); // -> description
        for _ in 0.."desc".len() {
            app.components
                .task_form
                .handle_event(&key_event(KeyCode::Right));
        }
        app.components.task_form.handle_event(&char_event(' '));
        app.components.task_form.handle_event(&char_event('m'));
        app.components.task_form.handle_event(&char_event('o'));
        app.components.task_form.handle_event(&char_event('r'));
        app.components.task_form.handle_event(&char_event('e'));
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Tab)); // -> status
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Right)); // Todo -> InProgress
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Tab)); // -> priority
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Right)); // Medium -> High
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Tab)); // -> dependencies
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Tab)); // -> notes
        app.components.task_form.handle_event(&char_event('d'));
        app.components.task_form.handle_event(&char_event('r'));
        app.components.task_form.handle_event(&char_event('a'));
        app.components.task_form.handle_event(&char_event('f'));
        app.components.task_form.handle_event(&char_event('t'));
        // Cycle back to the dependencies field and open the popup.
        for _ in 0..5 {
            app.components
                .task_form
                .handle_event(&key_event(KeyCode::Tab));
        }
        app.components
            .task_form
            .handle_event(&key_event(KeyCode::Enter));

        // Sanity: the expected transient state is in place before the refresh.
        assert_eq!(app.components.task_form.name(), "Task edited");
        assert_eq!(
            app.components.task_form.description().lines(),
            ["desc more"]
        );
        assert_eq!(app.components.task_form.status(), Status::InProgress);
        assert_eq!(app.components.task_form.priority(), Priority::High);
        assert_eq!(app.components.task_form.active_field(), 4);
        assert_eq!(app.components.task_form.new_note(), "draft");
        assert!(app.components.task_form.show_dep_popup());

        // AddTaskNote produces a Refresh; applying it must reload the notes
        // without destroying the form edits or popup.
        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "My note".to_string(),
        });
        assert_eq!(next, Some(Action::Refresh));
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(app.components.task_form.notes().len(), 1);
        assert_eq!(app.components.task_form.notes()[0].content, "My note");
        assert_eq!(
            app.screen,
            Screen::TaskForm {
                version_id: app.repo.default_version_id(project_id).unwrap(),
                task_id: Some(task_id),
            }
        );
        assert_eq!(app.components.task_form.name(), "Task edited");
        assert_eq!(
            app.components.task_form.description().lines(),
            ["desc more"]
        );
        assert_eq!(app.components.task_form.status(), Status::InProgress);
        assert_eq!(app.components.task_form.priority(), Priority::High);
        assert_eq!(app.components.task_form.active_field(), 4);
        assert_eq!(app.components.task_form.new_note(), "draft");
        assert!(app.components.task_form.show_dep_popup());
    }

    #[test]
    fn app_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<App>();
    }

    #[test]
    fn update_is_hidden_by_default() {
        let app = App::new(Arc::new(StubRepository::default()));
        assert!(!app.has_update());
        assert!(!app.is_update_banner_open());
    }

    #[test]
    fn poll_update_copies_slot_value() {
        let mut app = App::new(Arc::new(StubRepository::default()));
        let slot = Arc::new(OnceLock::new());
        app.set_pending_update((slot.clone(), Arc::new(AtomicBool::new(false))));

        app.poll_update();
        assert!(!app.has_update());

        slot.set("0.4.0".to_string()).unwrap();
        app.poll_update();
        assert_eq!(app.update_version(), Some("0.4.0"));
    }

    #[test]
    fn open_update_banner_requires_an_update() {
        let mut app = App::new(Arc::new(StubRepository::default()));
        app.open_update_banner();
        assert!(!app.is_update_banner_open());

        app.set_update_available("0.4.0".to_string());
        app.open_update_banner();
        assert!(app.is_update_banner_open());

        app.close_update_banner();
        assert!(!app.is_update_banner_open());
    }

    #[test]
    fn save_version_rejects_blank_name_without_touching_repo() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::SaveVersion {
            version_id: None,
            project_id,
            name: "  ".to_string(),
            description: String::new(),
            status: VersionStatus::Open,
        });
        assert!(next.is_none());
        assert!(app.error().is_some());
        assert_eq!(repo.list_versions_by_project(project_id).unwrap().len(), 1);
    }

    #[test]
    fn save_version_creates_and_returns_to_project_dashboard() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::SaveVersion {
            version_id: None,
            project_id,
            name: "v0.1.0".to_string(),
            description: String::new(),
            status: VersionStatus::Open,
        });
        assert_eq!(
            next,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
        assert!(
            repo.list_versions_by_project(project_id)
                .unwrap()
                .iter()
                .any(|v| v.name == "v0.1.0")
        );
        let created = repo
            .list_versions_by_project(project_id)
            .unwrap()
            .into_iter()
            .find(|v| v.name == "v0.1.0")
            .unwrap();
        assert_eq!(created.status, VersionStatus::Open);
    }

    #[test]
    fn save_version_creates_in_backlog_when_chosen() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::SaveVersion {
            version_id: None,
            project_id,
            name: "v0.2.0".to_string(),
            description: String::new(),
            status: VersionStatus::Backlog,
        });
        assert_eq!(
            next,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
        let created = repo
            .list_versions_by_project(project_id)
            .unwrap()
            .into_iter()
            .find(|v| v.name == "v0.2.0")
            .unwrap();
        assert_eq!(created.status, VersionStatus::Backlog);
    }

    #[test]
    fn v_from_project_dashboard_opens_version_edit_form() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let mut app = App::new(repo.clone());
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));

        let action = app
            .active_component()
            .handle_event(&key_event(KeyCode::Char('v')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionForm {
                project_id,
                version_id: Some(version_id),
            }))
        );

        if let Some(action) = action {
            app.dispatch(action);
        }
        assert_eq!(
            app.screen,
            Screen::VersionForm {
                project_id,
                version_id: Some(version_id),
            }
        );
        assert_eq!(
            app.components.version_form.return_to(),
            &Screen::ProjectDashboard { project_id }
        );
    }

    #[test]
    fn save_version_with_id_updates_and_returns_to_dashboard() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let version_id = repo
            .create_version(project_id, "v1.0.0", "old")
            .unwrap()
            .id
            .unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::SaveVersion {
            version_id: Some(version_id),
            project_id,
            name: "v2.0.0".to_string(),
            description: "new".to_string(),
            status: VersionStatus::Released,
        });
        assert!(app.error().is_none());
        assert_eq!(
            next,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
        let updated = repo.get_version(version_id).unwrap();
        assert_eq!(updated.name, "v2.0.0");
        assert_eq!(updated.description, "new");
        assert_eq!(updated.status, VersionStatus::Released);
    }

    #[test]
    fn save_version_with_duplicate_name_keeps_old_name_and_sets_error() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let version_id = repo
            .create_version(project_id, "v1.0.0", "")
            .unwrap()
            .id
            .unwrap();
        repo.create_version(project_id, "v2.0.0", "").unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::SaveVersion {
            version_id: Some(version_id),
            project_id,
            name: "v2.0.0".to_string(),
            description: String::new(),
            status: VersionStatus::Open,
        });
        assert_eq!(next, None);
        assert!(app.error().is_some());
        assert_eq!(repo.get_version(version_id).unwrap().name, "v1.0.0");
    }

    #[test]
    fn delete_version_removes_it_and_navigates_to_project() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let version_id = repo
            .create_version(project_id, "v1.0.0", "")
            .unwrap()
            .id
            .unwrap();
        let mut app = App::new(repo.clone());
        let next = app.dispatch(Action::DeleteVersion { version_id });
        assert_eq!(
            next,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
        assert!(repo.get_version(version_id).is_err());
    }

    #[test]
    fn question_mark_opens_help_only_outside_text_forms() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let mut app = App::new(repo);

        assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::SHIFT));
        assert!(app.is_help_open());
        // Esc closes it; `?` closes it too.
        assert!(app.handle_help_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.is_help_open());
        assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(!app.is_help_open());

        app.dispatch(Action::Navigate(Screen::ProjectForm { project_id: None }));
        assert!(!app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(!app.is_help_open());

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
    }

    #[test]
    fn help_swallows_keys_with_ctrl_modifiers_too() {
        let repo = Arc::new(StubRepository::default());
        let mut app = App::new(repo);
        app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE);

        assert!(app.handle_help_key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.is_help_open());
    }

    #[test]
    fn accepts_text_input_only_on_forms() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let mut app = App::new(repo);
        assert!(!app.screen.accepts_text_input());

        app.dispatch(Action::Navigate(Screen::ProjectForm { project_id: None }));
        assert!(app.screen.accepts_text_input());

        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(project_id).unwrap(),
            task_id: None,
        }));
        assert!(app.screen.accepts_text_input());
    }

    #[test]
    fn move_task_moves_it_and_refreshes_version_dashboard() {
        let (repo, mut app) = setup_app();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v0.2.0", "").unwrap().id.unwrap();
        let task = repo
            .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id: pid,
            version_id: v1,
        }));
        let next = app.dispatch(Action::MoveTask {
            task_id: task.id.unwrap(),
            version_id: v2,
        });
        assert_eq!(
            next,
            Some(Action::Navigate(Screen::VersionDashboard {
                project_id: pid,
                version_id: v1,
            }))
        );
        assert_eq!(repo.get_task(task.id.unwrap()).unwrap().version_id, v2);
        assert!(app.error().is_none());
    }

    #[test]
    fn move_task_with_dependencies_sets_error_and_keeps_version() {
        let (repo, mut app) = setup_app();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v0.2.0", "").unwrap().id.unwrap();
        let a = repo
            .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        let b = repo
            .create_task(pid, v1, "B", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        repo.add_dependency(b, a).unwrap();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id: pid,
            version_id: v1,
        }));
        let next = app.dispatch(Action::MoveTask {
            task_id: b,
            version_id: v2,
        });
        assert_eq!(next, None);
        assert!(app.error().unwrap().contains("dependencies or dependents"));
        assert_eq!(repo.get_task(b).unwrap().version_id, v1);
    }

    #[test]
    fn move_task_outside_version_dashboard_is_ignored() {
        let (repo, mut app) = setup_app();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v0.2.0", "").unwrap().id.unwrap();
        let task = repo
            .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard {
            project_id: pid,
        }));
        let next = app.dispatch(Action::MoveTask {
            task_id: task.id.unwrap(),
            version_id: v2,
        });
        assert_eq!(next, None);
        assert_eq!(repo.get_task(task.id.unwrap()).unwrap().version_id, v1);
    }

    #[test]
    fn export_version_summary_writes_file_and_sets_notice() {
        let (repo, mut app) = setup_app();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let task = repo
            .create_task(pid, v, "A", "", Status::Done, Priority::Low)
            .unwrap();
        repo.update_task_status(task.id.unwrap(), Status::Done)
            .unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        let summary = repo
            .create_version_summary(v, "Summary", &snapshot)
            .unwrap();
        let summary_id = summary.id.unwrap();

        let next = app.dispatch(Action::ExportVersionSummary { summary_id });
        assert_eq!(next, None);

        let notice = app.toasts().current().unwrap().message.clone();
        assert_eq!(app.toasts().current().unwrap().severity, Severity::Success);
        assert!(notice.starts_with("Exported to "));
        let path = notice.trim_start_matches("Exported to ");
        assert!(std::path::Path::new(path).exists());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn esc_from_version_summary_returns_to_version_dashboard() {
        let (repo, mut app) = setup_app();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let task = repo
            .create_task(pid, v, "A", "", Status::Done, Priority::Low)
            .unwrap();
        repo.update_task_status(task.id.unwrap(), Status::Done)
            .unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        let summary = repo
            .create_version_summary(v, "Summary", &snapshot)
            .unwrap();
        let summary_id = summary.id.unwrap();

        app.dispatch(Action::Navigate(Screen::VersionSummary {
            project_id: pid,
            version_id: v,
            summary_id,
        }));
        let action = app
            .active_component()
            .handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionDashboard {
                project_id: pid,
                version_id: v,
            }))
        );
    }

    /// Replaces the app clock with one the test moves by hand.
    fn manual_clock(app: &mut App) -> Arc<std::sync::atomic::AtomicU64> {
        use std::sync::atomic::{AtomicU64, Ordering};
        let base = Instant::now();
        let millis = Arc::new(AtomicU64::new(0));
        let handle = millis.clone();
        app.set_clock(move || {
            base + std::time::Duration::from_millis(handle.load(Ordering::SeqCst))
        });
        millis
    }

    fn set_millis(millis: &Arc<std::sync::atomic::AtomicU64>, value: u64) {
        millis.store(value, std::sync::atomic::Ordering::SeqCst);
    }

    fn blank_project_save() -> Action {
        Action::SaveProject {
            project_id: None,
            name: "   ".to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn success_toast_expires_after_three_seconds() {
        let (_repo, mut app) = setup_app();
        let clock = manual_clock(&mut app);
        app.dispatch(Action::SaveProject {
            project_id: None,
            name: "Beta".to_string(),
            description: String::new(),
        });
        let shown = app.toasts().current().unwrap();
        assert_eq!(shown.severity, Severity::Success);
        assert_eq!(shown.message, "Project saved");

        set_millis(&clock, 2_999);
        app.tick();
        assert!(app.toasts().current().is_some());

        set_millis(&clock, 3_000);
        app.tick();
        assert!(app.toasts().current().is_none());
    }

    #[test]
    fn error_toast_stays_until_the_next_action() {
        let (_repo, mut app) = setup_app();
        let clock = manual_clock(&mut app);
        app.dispatch(blank_project_save());

        set_millis(&clock, 60_000);
        app.tick();
        assert_eq!(app.error(), Some("Project name cannot be empty"));

        app.dispatch(Action::Navigate(Screen::ProjectList));
        assert_eq!(app.error(), None);
    }

    #[test]
    fn auto_refresh_does_not_hide_an_unread_error() {
        let (_repo, mut app) = setup_app();
        app.dispatch(blank_project_save());

        app.dispatch(Action::Refresh);

        assert_eq!(app.error(), Some("Project name cannot be empty"));
    }

    #[test]
    fn esc_closes_an_error_toast_and_consumes_the_key() {
        let (_repo, mut app) = setup_app();
        app.dispatch(blank_project_save());

        assert!(app.handle_toast_key(KeyCode::Esc));
        assert_eq!(app.error(), None);
        assert!(!app.handle_toast_key(KeyCode::Esc));
    }

    #[test]
    fn esc_passes_through_when_the_toast_is_not_an_error() {
        let (_repo, mut app) = setup_app();
        app.dispatch(Action::SaveProject {
            project_id: None,
            name: "Beta".to_string(),
            description: String::new(),
        });

        assert!(!app.handle_toast_key(KeyCode::Esc));
        assert!(app.toasts().current().is_some());
    }

    #[test]
    fn successful_actions_emit_a_confirmation() {
        let (repo, mut app) = setup_app();
        let project_id = repo.create_project("P", "").unwrap().id.unwrap();
        let version_id = repo
            .create_version(project_id, "v1", "")
            .unwrap()
            .id
            .unwrap();
        let task = repo
            .create_task(
                project_id,
                version_id,
                "T",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::UpdateTaskStatus {
            task_id: task.id.unwrap(),
            new_status: Status::Done,
        });
        assert_eq!(
            app.toasts().current().unwrap().message,
            format!("Status set to {}", Status::Done.label())
        );
        app.dispatch(Action::DeleteTask {
            task_id: task.id.unwrap(),
        });
        // Queued behind the earlier confirmation: the queue is first-in, first-out.
        let last = app.toasts().iter().last().unwrap();
        assert_eq!(last.message, "Task deleted");
    }

    #[test]
    fn repeated_failures_do_not_pile_up() {
        let (_repo, mut app) = setup_app();
        for _ in 0..10 {
            app.dispatch(Action::Refresh);
            app.dispatch(Action::Noop);
        }
        app.dispatch(blank_project_save());
        app.dispatch(Action::Noop);
        app.dispatch(blank_project_save());
        assert_eq!(app.toasts().len(), 1);
    }
}

#[cfg(test)]
mod characterization_tests {
    use std::sync::Arc;

    use crate::db::repository::{
        DependencyRepository, NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::components::toast::Severity;
    use crate::screens::{Action, Screen};

    use super::App;

    fn setup() -> (Arc<StubRepository>, App, i64, i64) {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let app = App::new(repo.clone());
        (repo, app, project_id, version_id)
    }

    fn task(repo: &StubRepository, project_id: i64, version_id: i64, name: &str) -> i64 {
        repo.create_task(
            project_id,
            version_id,
            name,
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap()
    }

    #[test]
    fn save_task_syncs_dependencies_added_and_removed() {
        let (repo, mut app, project_id, version_id) = setup();
        let a = task(&repo, project_id, version_id, "A");
        let b = task(&repo, project_id, version_id, "B");
        let c = task(&repo, project_id, version_id, "C");
        repo.add_dependency(a, b).unwrap();

        app.dispatch(Action::SaveTask {
            task_id: Some(a),
            version_id,
            name: "A".to_string(),
            description: String::new(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![c],
        });

        assert_eq!(repo.get_dependencies(a).unwrap(), vec![c]);
    }

    #[test]
    fn save_task_returns_navigate_and_success_toast() {
        let (repo, mut app, project_id, version_id) = setup();
        let a = task(&repo, project_id, version_id, "A");

        let next = app.dispatch(Action::SaveTask {
            task_id: Some(a),
            version_id,
            name: "A2".to_string(),
            description: String::new(),
            status: Status::Todo,
            priority: Priority::High,
            depends_on_ids: vec![],
        });

        assert!(matches!(next, Some(Action::Navigate(_))));
        let toast = app.toasts().current().unwrap();
        assert_eq!(toast.severity, Severity::Success);
        assert_eq!(toast.message, "Task saved");
    }

    #[test]
    fn delete_project_navigates_to_list_with_confirmation() {
        let (repo, mut app, project_id, _) = setup();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));

        let next = app.dispatch(Action::DeleteProject { project_id });
        assert_eq!(next, Some(Action::Navigate(Screen::ProjectList)));
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(app.screen, Screen::ProjectList);
        assert!(repo.get_project(project_id).is_err());
        assert_eq!(app.toasts().current().unwrap().message, "Project deleted");
    }

    #[test]
    fn task_status_change_outside_dashboards_is_ignored() {
        let (repo, mut app, project_id, version_id) = setup();
        let a = task(&repo, project_id, version_id, "A");

        let next = app.dispatch(Action::UpdateTaskStatus {
            task_id: a,
            new_status: Status::Done,
        });

        assert_eq!(next, None);
        assert_eq!(repo.get_task(a).unwrap().status, Status::Todo);
        assert!(app.toasts().is_empty());
    }

    #[test]
    fn delete_task_from_project_dashboard_refreshes_it() {
        let (repo, mut app, project_id, version_id) = setup();
        let a = task(&repo, project_id, version_id, "A");
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));

        let next = app.dispatch(Action::DeleteTask { task_id: a });

        assert_eq!(
            next,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
        assert!(repo.get_task(a).is_err());
    }

    #[test]
    fn add_note_success_requests_refresh_with_confirmation() {
        let (repo, mut app, project_id, version_id) = setup();
        let a = task(&repo, project_id, version_id, "A");

        let next = app.dispatch(Action::AddTaskNote {
            task_id: a,
            content: "progreso".to_string(),
        });

        assert_eq!(next, Some(Action::Refresh));
        assert_eq!(app.toasts().current().unwrap().message, "Note added");
        assert_eq!(repo.list_task_notes(a).unwrap().len(), 1);
    }

    #[test]
    fn noop_and_quit_keep_an_unread_error() {
        let (_repo, mut app, _, _) = setup();
        app.dispatch(Action::SaveProject {
            project_id: None,
            name: " ".to_string(),
            description: String::new(),
        });

        assert_eq!(app.dispatch(Action::Noop), None);
        assert_eq!(app.dispatch(Action::Quit), None);

        assert_eq!(app.error(), Some("Project name cannot be empty"));
    }

    #[test]
    fn successful_save_clears_a_previous_error() {
        let (_repo, mut app, _, _) = setup();
        app.dispatch(Action::SaveProject {
            project_id: None,
            name: " ".to_string(),
            description: String::new(),
        });

        app.dispatch(Action::SaveProject {
            project_id: None,
            name: "Beta".to_string(),
            description: String::new(),
        });

        assert_eq!(app.error(), None);
    }
}
