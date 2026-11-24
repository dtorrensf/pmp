// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
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

/// Toasts start with a capital letter while the underlying error keeps its
/// canonical, lowercase message (the same one MCP sends).
fn capitalize(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

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
        self.toasts
            .push(Severity::Error, capitalize(&message.into()), now);
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
                working_dir,
            } => {
                let return_to = self.components.project_form.return_to().clone();
                let result = project_actions::save(
                    self.repo.as_ref(),
                    project_id,
                    &name,
                    &description,
                    &working_dir,
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
            Action::ReportError { message } => {
                self.push_error(message);
                None
            }
            Action::Quit => None,
            Action::Noop => None,
        }
    }

    /// Turns a use case result into user feedback: a confirmation on success,
    /// an error toast on failure. Returns the follow-up action, if any.
    ///
    /// The toast shows [`AppError::user_message`], so storage details stay in
    /// the log and the user sees the same wording for the same failure code
    /// the MCP server reports.
    fn settle(&mut self, result: Result<Outcome, AppError>) -> Option<Action> {
        match result {
            Ok(outcome) => {
                if let Some(message) = outcome.confirmation {
                    self.push_success(message);
                }
                outcome.next
            }
            Err(error) => {
                self.push_error(error.user_message());
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
mod tests;
