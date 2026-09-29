use std::time::Duration;

use crossterm::event::{Event, KeyEvent};
use ratatui::{Frame, layout::Rect};

pub mod components;
pub mod dashboard;
pub mod keybinding_footer;
pub mod project_form;
pub mod project_list;
pub mod task_form;
pub mod task_list;
pub mod task_list_view;
#[cfg(test)]
pub mod test_utils;

pub use dashboard::ProjectDashboard;
pub use project_form::ProjectForm;
pub use project_list::ProjectListScreen;
pub use task_form::TaskForm;
pub use task_list::TaskListScreen;

pub trait Component {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Reload data without destroying transient UI state. Defaults to
    /// [`init`](Self::init) for components that do not have modal state.
    fn refresh(&mut self) -> anyhow::Result<()> {
        self.init()
    }
    fn handle_event(&mut self, _event: &Event) -> Option<Action> {
        None
    }
    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }
    fn render(&self, frame: &mut Frame, area: Rect);
    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        &[]
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
        project_id: i64,
        task_id: Option<i64>,
    },
    TaskList {
        project_id: i64,
    },
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
        project_id: i64,
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
    Refresh,
}

#[cfg(test)]
pub mod tests {
    use crate::db::repository::Repository;
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
                project_id: 2,
                task_id: None
            }
        );
        assert_ne!(
            Screen::ProjectDashboard { project_id: 1 },
            Screen::TaskList { project_id: 1 }
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
        let list = ProjectListScreen::new(repo);
        assert_eq!(list.auto_refresh_interval(), None);
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let c = repo
            .create_task(project_id, "C", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "Low", "", Status::Todo, Priority::Low)
            .unwrap();
        let high_old = repo
            .create_task(project_id, "HighOld", "", Status::Todo, Priority::High)
            .unwrap();
        let high_new = repo
            .create_task(project_id, "HighNew", "", Status::Todo, Priority::High)
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
