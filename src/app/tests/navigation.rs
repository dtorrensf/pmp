// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::{
    ProjectRepository, RepositoryPolicy, SummaryRepository, TaskRepository, VersionRepository,
};
use crate::domain::task::{Priority, Status};
use crate::screens::Component;
use crate::screens::test_utils::{char_event, key_event, render_to_string};
use crate::screens::{Action, Screen};
use crossterm::event::KeyCode;

use super::*;

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
