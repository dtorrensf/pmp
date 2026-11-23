// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::{
    DependencyRepository, NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
    VersionRepository,
};
use crate::domain::task::{Priority, Status};
use crate::screens::components::toast::Severity;
use crate::screens::{Action, Screen};

use super::*;

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
fn save_task_syncs_dependencies_added_and_removed() {
    let (repo, mut app, project_id, version_id) = setup_ids();
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
    let (repo, mut app, project_id, version_id) = setup_ids();
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
fn delete_task_from_project_dashboard_refreshes_it() {
    let (repo, mut app, project_id, version_id) = setup_ids();
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
fn task_status_change_outside_dashboards_is_ignored() {
    let (repo, mut app, project_id, version_id) = setup_ids();
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
fn add_note_success_requests_refresh_with_confirmation() {
    let (repo, mut app, project_id, version_id) = setup_ids();
    let a = task(&repo, project_id, version_id, "A");
    let next = app.dispatch(Action::AddTaskNote {
        task_id: a,
        content: "progreso".to_string(),
    });
    assert_eq!(next, Some(Action::Refresh));
    assert_eq!(app.toasts().current().unwrap().message, "Note added");
    assert_eq!(repo.list_task_notes(a).unwrap().len(), 1);
}
