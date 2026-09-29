use std::sync::Arc;

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::Status;
use crate::screens::{
    Action, Component, ProjectDashboard, ProjectForm, ProjectListScreen, Screen, TaskForm,
    TaskListScreen,
};

pub struct App {
    pub screen: Screen,
    pub previous_screen: Option<Screen>,
    pub repo: Arc<dyn Repository>,
    pub action: Option<Action>,
    pub error: Option<String>,
    components: Components,
}

struct Components {
    project_list: ProjectListScreen,
    dashboard: ProjectDashboard,
    project_form: ProjectForm,
    task_form: TaskForm,
    task_list: TaskListScreen,
}

impl App {
    pub fn new(repo: Arc<dyn Repository>) -> Self {
        let mut app = Self {
            screen: Screen::ProjectList,
            previous_screen: None,
            repo: repo.clone(),
            action: None,
            error: None,
            components: Components {
                project_list: ProjectListScreen::new(repo.clone()),
                dashboard: ProjectDashboard::new(repo.clone()),
                project_form: ProjectForm::new(repo.clone()),
                task_form: TaskForm::new(repo.clone()),
                task_list: TaskListScreen::new(repo.clone()),
            },
        };
        if let Err(e) = app.init_screen(&Screen::ProjectList) {
            app.error = Some(e.to_string());
        }
        app
    }

    pub fn dispatch(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Navigate(screen) => {
                let previous = self.screen.clone();
                match self.init_screen(&screen) {
                    Ok(()) => {
                        if matches!(screen, Screen::TaskForm { .. }) {
                            self.components.task_form.set_return_to(&previous);
                        }
                        if matches!(screen, Screen::ProjectForm { .. }) {
                            self.components.project_form.set_return_to(&previous);
                        }
                        self.previous_screen = Some(previous);
                        self.screen = screen;
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        self.screen = self.previous_screen.take().unwrap_or(Screen::ProjectList);
                    }
                }
                None
            }
            Action::SaveProject {
                project_id,
                name,
                description,
            } => {
                if name.trim().is_empty() {
                    self.error = Some("Project name cannot be empty".to_string());
                    return None;
                }
                let result = match project_id {
                    Some(id) => self.repo.update_project(id, &name, &description),
                    None => self.repo.create_project(&name, &description),
                };
                match result {
                    Ok(project) => {
                        self.error = None;
                        Some(Action::Navigate(match project_id {
                            Some(_) => self.components.project_form.return_to().clone(),
                            None => Screen::ProjectDashboard {
                                project_id: project.id.unwrap_or(0),
                            },
                        }))
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        None
                    }
                }
            }
            Action::SaveTask {
                task_id,
                project_id,
                name,
                description,
                status,
                priority,
                depends_on_ids,
            } => {
                if name.trim().is_empty() {
                    self.error = Some("Task name cannot be empty".to_string());
                    return None;
                }
                // Block Done transition when dependencies are incomplete
                // (only for existing tasks being edited, not new tasks).
                if status == Status::Done
                    && let Some(id) = task_id
                {
                    match self.repo.has_incomplete_dependencies(id) {
                        Ok(true) => {
                            self.error = Some(
                                "cannot mark task as done: it has incomplete dependencies"
                                    .to_string(),
                            );
                            return None;
                        }
                        Ok(false) => self.error = None,
                        Err(e) => {
                            self.error = Some(e.to_string());
                            return None;
                        }
                    }
                }
                let result = match task_id {
                    Some(id) => self
                        .repo
                        .update_task(id, &name, &description, status, priority),
                    None => {
                        self.repo
                            .create_task(project_id, &name, &description, status, priority)
                    }
                };
                match result {
                    Ok(task) => {
                        let saved_id = task.id.unwrap_or(0);
                        if let Err(e) = self.sync_dependencies(saved_id, &depends_on_ids) {
                            self.error = Some(e.to_string());
                            return None;
                        }
                        self.error = None;
                        Some(Action::Navigate(
                            self.components.task_form.return_to().clone(),
                        ))
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        None
                    }
                }
            }
            Action::UpdateTaskStatus {
                task_id,
                new_status,
            } => {
                let refresh = match &self.screen {
                    Screen::ProjectDashboard { project_id } => Screen::ProjectDashboard {
                        project_id: *project_id,
                    },
                    Screen::TaskList { project_id } => Screen::TaskList {
                        project_id: *project_id,
                    },
                    _ => return None,
                };
                // Block Done transition when dependencies are incomplete.
                if new_status == Status::Done {
                    match self.repo.has_incomplete_dependencies(task_id) {
                        Ok(true) => {
                            self.error = Some(
                                "cannot mark task as done: it has incomplete dependencies"
                                    .to_string(),
                            );
                            return None;
                        }
                        Ok(false) => self.error = None,
                        Err(e) => {
                            self.error = Some(e.to_string());
                            return None;
                        }
                    }
                }
                match self.repo.update_task_status(task_id, new_status) {
                    Ok(_) => {
                        self.error = None;
                        // Refresh the source screen so init_screen re-runs its init()
                        // and the cached task list is updated in place.
                        Some(Action::Navigate(refresh))
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        None
                    }
                }
            }
            Action::DeleteProject { project_id } => match self.repo.delete_project(project_id) {
                Ok(()) => {
                    self.error = None;
                    Some(Action::Navigate(Screen::ProjectList))
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                    None
                }
            },
            Action::DeleteTask { task_id } => {
                let refresh = match &self.screen {
                    Screen::ProjectDashboard { project_id } => Screen::ProjectDashboard {
                        project_id: *project_id,
                    },
                    Screen::TaskList { project_id } => Screen::TaskList {
                        project_id: *project_id,
                    },
                    _ => return None,
                };
                match self.repo.delete_task(task_id) {
                    Ok(()) => {
                        self.error = None;
                        Some(Action::Navigate(refresh))
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        None
                    }
                }
            }
            Action::Refresh => {
                if let Err(e) = self.active_component().refresh() {
                    self.error = Some(e.to_string());
                } else {
                    self.error = None;
                }
                None
            }
            Action::AddTaskNote { task_id, content } => {
                match self.repo.add_task_note(task_id, &content) {
                    Ok(_) => {
                        self.error = None;
                        Some(Action::Refresh)
                    }
                    Err(AppError::Validation(_)) => {
                        self.error = Some("Note content cannot be empty".to_string());
                        None
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        None
                    }
                }
            }
            Action::Quit => None,
            Action::Noop => None,
        }
    }

    fn sync_dependencies(&self, task_id: i64, depends_on_ids: &[i64]) -> anyhow::Result<()> {
        let current = self.repo.get_dependencies(task_id)?;
        for &dep_id in &current {
            if !depends_on_ids.contains(&dep_id) {
                self.repo.remove_dependency(task_id, dep_id)?;
            }
        }
        for &dep_id in depends_on_ids {
            if !current.contains(&dep_id) {
                self.repo.add_dependency(task_id, dep_id)?;
            }
        }
        Ok(())
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
                project_id,
                task_id,
            } => {
                self.components.task_form.set_project_id(*project_id);
                self.components.task_form.set_task_id(*task_id);
                self.components.task_form.init()
            }
            Screen::TaskList { project_id } => {
                self.components.task_list.set_project_id(*project_id);
                self.components.task_list.init()
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
                project_id,
                task_id,
            } => {
                self.components.task_form.set_project_id(*project_id);
                self.components.task_form.set_task_id(*task_id);
                &mut self.components.task_form
            }
            Screen::TaskList { project_id } => {
                self.components.task_list.set_project_id(*project_id);
                &mut self.components.task_list
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;

    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
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
        assert!(app.error.is_none());
    }

    #[test]
    fn navigate_sets_previous_screen() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        assert_eq!(app.screen, Screen::ProjectDashboard { project_id });
        assert_eq!(app.previous_screen, Some(Screen::ProjectList));
        assert!(app.error.is_none());
    }

    #[test]
    fn navigate_to_invalid_project_restores_previous_screen() {
        let (_repo, mut app) = setup_app();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard {
            project_id: 99,
        }));
        assert_eq!(app.screen, Screen::ProjectList);
        assert!(app.error.is_some());
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
        assert_eq!(app.error, Some("Project name cannot be empty".to_string()));
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn save_task_navigates_to_dashboard_on_success() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
            task_id: None,
        }));
        let next = app.dispatch(Action::SaveTask {
            task_id: None,
            project_id,
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
            project_id,
            name: "".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![],
        });
        assert_eq!(next, None);
        assert_eq!(app.error, Some("Task name cannot be empty".to_string()));
    }

    #[test]
    fn update_task_status_to_done_with_incomplete_deps_is_blocked() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.add_dependency(a.id.unwrap(), b.id.unwrap()).unwrap();

        // UpdateTaskStatus is dispatched from the dashboard (Tab in TaskList mode).
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::UpdateTaskStatus {
            task_id: a.id.unwrap(),
            new_status: Status::Done,
        });
        assert_eq!(
            app.error,
            Some("cannot mark task as done: it has incomplete dependencies".to_string())
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();

        // UpdateTaskStatus is dispatched from the dashboard (Tab in TaskList mode).
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::UpdateTaskStatus {
            task_id: a.id.unwrap(),
            new_status: Status::Done,
        });
        assert!(app.error.is_none());
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        // sanity: list is cached at Todo
        assert_eq!(app.components.dashboard.tasks()[0].status, Status::Todo);

        let next = app.dispatch(Action::UpdateTaskStatus {
            task_id,
            new_status: Status::InProgress,
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::ProjectDashboard { project_id: id })) if id == project_id),
            "expected navigate to dashboard, got {:?}",
            next
        );

        // apply the returned navigate -> init_screen -> dashboard.init() refresh
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(
            app.components.dashboard.tasks()[0].status,
            Status::InProgress,
            "dashboard cached list must reflect the cycled status"
        );
        // selection preserved across refresh
        assert_eq!(app.components.dashboard.selected_task(), 0);
        assert!(app.error.is_none());
    }

    #[test]
    fn save_task_from_task_list_returns_to_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
            task_id: None,
        }));
        assert_eq!(
            app.screen,
            Screen::TaskForm {
                project_id,
                task_id: None,
            }
        );

        let next = app.dispatch(Action::SaveTask {
            task_id: None,
            project_id,
            name: "Task 1".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on_ids: vec![],
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::TaskList { project_id: id })) if id == project_id),
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
            .create_task(project_id, "Old", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
            task_id: Some(task_id),
        }));
        let next = app.dispatch(Action::SaveTask {
            task_id: Some(task_id),
            project_id,
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(a_id, b_id).unwrap();

        app.dispatch(Action::SaveTask {
            task_id: Some(a_id),
            project_id,
            name: "A".to_string(),
            description: "".to_string(),
            status: Status::Done,
            priority: Priority::Low,
            depends_on_ids: vec![b_id],
        });

        assert_eq!(
            app.error,
            Some("cannot mark task as done: it has incomplete dependencies".to_string())
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
            .create_task(project_id, "T", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));
        assert_eq!(app.screen, Screen::TaskList { project_id });

        let next = app.dispatch(Action::UpdateTaskStatus {
            task_id,
            new_status: Status::InProgress,
        });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::TaskList { project_id: id })) if id == project_id),
            "expected navigate back to task list, got {:?}",
            next
        );

        if let Some(action) = next {
            app.dispatch(action);
        }
        assert_eq!(
            app.components.task_list.tasks()[0].status,
            Status::InProgress
        );
    }

    #[test]
    fn delete_task_from_task_list_returns_to_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(project_id, "T", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));

        let next = app.dispatch(Action::DeleteTask { task_id });
        assert!(
            matches!(next, Some(Action::Navigate(Screen::TaskList { project_id: id })) if id == project_id),
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::High)
            .unwrap();
        let t2 = repo
            .create_task(project_id, "T2", "", Status::Todo, Priority::High)
            .unwrap();
        let t3 = repo
            .create_task(project_id, "T3", "", Status::Todo, Priority::High)
            .unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.components.dashboard.set_selected_task(2);

        let next = app.dispatch(Action::DeleteTask {
            task_id: t3.id.unwrap(),
        });
        if let Some(action) = next {
            app.dispatch(action);
        }

        assert_eq!(app.components.dashboard.selected_task(), 1);
        assert!(repo.get_task(t3.id.unwrap()).is_err());
        assert!(repo.get_task(t1.id.unwrap()).is_ok());
        assert!(repo.get_task(t2.id.unwrap()).is_ok());
    }

    #[test]
    fn navigate_to_task_list_sets_screen_and_loads_tasks() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();

        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));

        assert_eq!(app.screen, Screen::TaskList { project_id });
        assert!(app.error.is_none());
        assert_eq!(app.components.task_list.tasks().len(), 1);
    }

    #[test]
    fn active_component_returns_task_list_keybindings() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));

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
        assert!(app.error.is_none());
    }

    #[test]
    fn refresh_reloads_dashboard_tasks() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        assert!(app.components.dashboard.tasks().is_empty());

        repo.create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        app.dispatch(Action::Refresh);

        assert_eq!(app.components.dashboard.tasks().len(), 1);
        assert!(app.error.is_none());
    }

    #[test]
    fn refresh_reloads_task_list() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        app.dispatch(Action::Navigate(Screen::TaskList { project_id }));
        assert!(app.components.task_list.tasks().is_empty());

        repo.create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        app.dispatch(Action::Refresh);

        assert_eq!(app.components.task_list.tasks().len(), 1);
        assert!(app.error.is_none());
    }

    #[test]
    fn add_task_note_success_refreshes_current_screen() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
            task_id: Some(task_id),
        }));
        let expected_return_to = app.components.task_form.return_to().clone();
        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "A note".to_string(),
        });

        assert_eq!(next, Some(Action::Refresh));
        assert!(app.error.is_none());
        assert_eq!(repo.list_task_notes(task_id).unwrap().len(), 1);
        assert_eq!(app.components.task_form.return_to(), &expected_return_to);
    }

    #[test]
    fn add_task_note_empty_content_sets_error() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        let next = app.dispatch(Action::AddTaskNote {
            task_id,
            content: "   ".to_string(),
        });

        assert_eq!(next, None);
        assert_eq!(app.error, Some("Note content cannot be empty".to_string()));
    }

    #[test]
    fn add_task_note_then_reopen_form_shows_note() {
        let (repo, mut app) = setup_app();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
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
            project_id,
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
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));

        // Open the filter dialog.
        app.components
            .dashboard
            .handle_event(&key_event(KeyCode::Char('f')));
        let before = render_to_string(&app.components.dashboard, 60, 24);
        assert!(before.contains("Filter Tasks"));

        // Create a new task in the repo while the dialog is open.
        repo.create_task(project_id, "NewTask", "", Status::Todo, Priority::Low)
            .unwrap();

        // Refresh must preserve the dialog and reload the data.
        app.dispatch(Action::Refresh);

        let after = render_to_string(&app.components.dashboard, 60, 24);
        assert!(after.contains("Filter Tasks"));
        assert!(
            app.components
                .dashboard
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
            .create_task(project_id, "Task", "desc", Status::Todo, Priority::Medium)
            .unwrap();
        let task_id = task.id.unwrap();

        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id,
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
                project_id,
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
}
