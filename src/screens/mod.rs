// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::time::Duration;

use crossterm::event::{Event, KeyEvent};
use ratatui::{Frame, layout::Rect};

use crate::screens::components::help::{self, HelpSection};

// Error-propagation policy for screens:
//
// - A query whose result is shown as real data (counts, dependency markers) or
//   that gates a destructive action fails the flow: `init`/`refresh` return the
//   error, and key handling returns [`Action::ReportError`]. The app turns it
//   into an error toast.
// - A failed query is never rendered as legitimate empty data ("only one
//   version", zero dependencies, zero tasks).
// - Fallbacks are only used for optional presentation (for example a missing
//   chart) and are commented where they happen.

pub const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub mod components;
pub mod dashboard;
pub mod keybinding_footer;
pub mod project_form;
pub mod project_list;
pub mod task_form;
pub mod task_list_view;
#[cfg(test)]
pub mod test_utils;
pub mod ui;
pub mod version_dashboard;
pub mod version_form;
pub mod version_summary;

pub use dashboard::ProjectDashboard;
pub use project_form::ProjectForm;
pub use project_list::ProjectListScreen;
pub use task_form::TaskForm;
pub use ui::UiContext;
pub use version_dashboard::VersionDashboard;
pub use version_form::VersionForm;
pub use version_summary::VersionSummaryScreen;

pub trait Component {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Reload data without destroying transient UI state. Defaults to
    /// [`init`](Self::init) for components that do not have modal state.
    fn refresh(&mut self) -> anyhow::Result<()> {
        self.init()
    }
    /// Advances time-driven state (eased values, pending celebrations). Called
    /// by the event loop about every 100 ms for the active component, with the
    /// frame context so the movement reads an explicit clock. Does nothing by
    /// default.
    fn tick(&mut self, _ui: &UiContext) {}
    fn handle_event(&mut self, _event: &Event) -> Option<Action> {
        None
    }
    fn render(&self, frame: &mut Frame, area: Rect, ui: &UiContext);
    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        &[]
    }
    /// Shortcuts grouped for the `?` overlay. By default every shortcut goes
    /// under the application section; screens override it to place their
    /// actions in the right section.
    fn help_sections(&self) -> Vec<HelpSection> {
        help::sections(self.keybindings(), |_| help::APPLICATION)
    }
    /// Interval at which this component refreshes itself automatically when
    /// idle, or `None` if it never auto-refreshes.
    fn auto_refresh_interval(&self) -> Option<Duration> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    ProjectList,
    ProjectDashboard {
        project_id: i64,
    },
    ProjectForm {
        project_id: Option<i64>,
    },
    TaskForm {
        version_id: i64,
        task_id: Option<i64>,
    },
    VersionDashboard {
        project_id: i64,
        version_id: i64,
    },
    VersionForm {
        project_id: i64,
        version_id: Option<i64>,
    },
    VersionSummary {
        project_id: i64,
        version_id: i64,
        summary_id: i64,
    },
}

impl Screen {
    /// `true` when this screen has a text field that must receive printable
    /// keys, so global shortcuts should not intercept them.
    pub fn accepts_text_input(&self) -> bool {
        matches!(
            self,
            Self::ProjectForm { .. } | Self::TaskForm { .. } | Self::VersionForm { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Action {
    #[default]
    Noop,
    Navigate(Screen),
    Quit,
    SaveProject {
        project_id: Option<i64>,
        name: String,
        description: String,
    },
    SaveTask {
        task_id: Option<i64>,
        version_id: i64,
        name: String,
        description: String,
        status: crate::domain::task::Status,
        priority: crate::domain::task::Priority,
        depends_on_ids: Vec<i64>,
    },
    UpdateTaskStatus {
        task_id: i64,
        new_status: crate::domain::task::Status,
    },
    MoveTask {
        task_id: i64,
        version_id: i64,
    },
    SaveVersion {
        version_id: Option<i64>,
        project_id: i64,
        name: String,
        description: String,
        status: crate::domain::version::VersionStatus,
    },
    DeleteVersion {
        version_id: i64,
    },
    DeleteTask {
        task_id: i64,
    },
    DeleteProject {
        project_id: i64,
    },
    AddTaskNote {
        task_id: i64,
        content: String,
    },
    ExportVersionSummary {
        summary_id: i64,
    },
    Refresh,
    /// Surfaces a failure that happened while handling a key (for example a
    /// repository query needed to build a dialog) as an error toast. Screens
    /// emit it instead of silently degrading to empty data.
    ReportError {
        message: String,
    },
}

#[cfg(test)]
pub mod tests {
    use crate::db::repository::{
        DependencyRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    use super::*;

    #[test]
    fn screen_variants_are_distinct() {
        assert_ne!(
            Screen::ProjectList,
            Screen::ProjectDashboard { project_id: 1 }
        );
        assert_ne!(
            Screen::ProjectDashboard { project_id: 1 },
            Screen::TaskForm {
                version_id: 2,
                task_id: None
            }
        );
        assert_ne!(
            Screen::ProjectDashboard { project_id: 1 },
            Screen::VersionDashboard {
                project_id: 1,
                version_id: 1
            }
        );
    }

    #[test]
    fn action_default_is_noop() {
        assert_eq!(Action::default(), Action::Noop);
    }

    #[test]
    fn action_navigate_holds_screen() {
        let action = Action::Navigate(Screen::ProjectList);
        assert!(matches!(action, Action::Navigate(Screen::ProjectList)));
    }

    #[test]
    fn component_trait_is_object_safe() {
        let _: Option<Box<dyn Component>> = None;
    }

    #[test]
    fn component_auto_refresh_defaults_to_none() {
        let repo = std::sync::Arc::new(StubRepository::default());
        let form = ProjectForm::new(repo);
        assert_eq!(form.auto_refresh_interval(), None);
    }

    #[test]
    fn stub_repository_implements_repository() {
        let repo = StubRepository::default();
        let project = repo.create_project("Test", "Desc").unwrap();
        assert_eq!(project.name, "Test");
        assert!(project.id.is_some());
    }

    #[test]
    fn stub_delete_task_removes_task_and_dependency_rows() {
        let repo = StubRepository::default();
        let project = repo.create_project("P", "").unwrap();
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

        repo.delete_task(b_id).unwrap();

        assert!(repo.get_task(b_id).is_err());
        assert!(repo.get_dependencies(a_id).unwrap().is_empty());
        assert!(repo.get_dependents(b_id).unwrap().is_empty());
    }

    #[test]
    fn stub_get_dependents_returns_reverse_lookup() {
        let repo = StubRepository::default();
        let project = repo.create_project("P", "").unwrap();
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
        let c = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "C",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        let c_id = c.id.unwrap();
        repo.add_dependency(b_id, a_id).unwrap();
        repo.add_dependency(c_id, a_id).unwrap();

        let dependents = repo.get_dependents(a_id).unwrap();
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&b_id));
        assert!(dependents.contains(&c_id));
    }

    #[test]
    fn stub_list_tasks_by_project_orders_priority_desc_created_desc() {
        let repo = StubRepository::default();
        let project = repo.create_project("P", "").unwrap();
        let project_id = project.id.unwrap();
        let low = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "Low",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let high_old = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "HighOld",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let high_new = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "HighNew",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();

        {
            let mut tasks = repo.tasks.lock().unwrap();
            let high_old_id = high_old.id.unwrap();
            let high_new_id = high_new.id.unwrap();
            for task in tasks.iter_mut() {
                if task.id == Some(high_old_id) {
                    task.created_at = "2026-01-01 00:00:00".to_string();
                } else if task.id == Some(high_new_id) {
                    task.created_at = "2026-01-02 00:00:00".to_string();
                }
            }
        }

        let tasks = repo.list_tasks_by_project(project_id).unwrap();
        assert_eq!(tasks[0].id, high_new.id);
        assert_eq!(tasks[1].id, high_old.id);
        assert_eq!(tasks[2].id, low.id);
    }
}
