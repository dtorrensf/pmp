// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::{ProjectRepository, TaskRepository, VersionRepository};
use crate::domain::task::{Priority, Status};
use crate::screens::components::toast::Severity;
use crate::screens::{Action, Screen};
use crossterm::event::KeyCode;

use crate::domain::error::AppError;

use super::*;

#[test]
fn success_toast_expires_after_three_seconds() {
    let (_repo, mut app) = setup_app();
    let clock = manual_clock(&mut app);
    app.dispatch(Action::SaveProject {
        project_id: None,
        name: "Beta".to_string(),
        description: String::new(),
        working_dir: String::new(),
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
        working_dir: String::new(),
    });
    assert!(!app.handle_toast_key(KeyCode::Esc));
    assert!(app.toasts().current().is_some());
}

#[test]
fn successful_actions_emit_a_confirmation() {
    let (repo, mut app) = setup_app();
    let project_id = repo.create_project("P", "", None).unwrap().id.unwrap();
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

#[test]
fn noop_and_quit_keep_an_unread_error() {
    let (_repo, mut app, _, _) = setup_ids();
    app.dispatch(Action::SaveProject {
        project_id: None,
        name: " ".to_string(),
        description: String::new(),
        working_dir: String::new(),
    });
    assert_eq!(app.dispatch(Action::Noop), None);
    assert_eq!(app.dispatch(Action::Quit), None);
    assert_eq!(app.error(), Some("Project name cannot be empty"));
}

#[test]
fn successful_save_clears_a_previous_error() {
    let (_repo, mut app, _, _) = setup_ids();
    app.dispatch(Action::SaveProject {
        project_id: None,
        name: " ".to_string(),
        description: String::new(),
        working_dir: String::new(),
    });
    app.dispatch(Action::SaveProject {
        project_id: None,
        name: "Beta".to_string(),
        description: String::new(),
        working_dir: String::new(),
    });
    assert_eq!(app.error(), None);
}

#[test]
fn storage_failures_show_a_generic_message_and_never_sqlite_details() {
    let (_repo, mut app) = setup_app();

    let next = app.settle(Err(AppError::DbError(rusqlite::Error::InvalidQuery)));

    assert_eq!(next, None);
    assert_eq!(app.error(), Some("Internal error"));
}

#[test]
fn dependency_errors_surface_the_canonical_message_capitalized() {
    let (_repo, mut app) = setup_app();

    app.settle(Err(AppError::DependencyBlocked {
        message: "cannot mark task as done: it has incomplete dependencies".to_string(),
        blockers: vec![2],
    }));

    assert_eq!(
        app.error(),
        Some("Cannot mark task as done: it has incomplete dependencies")
    );
}

#[test]
fn toast_messages_start_with_a_capital_letter() {
    assert_eq!(
        crate::app::capitalize("project name cannot be empty"),
        "Project name cannot be empty"
    );
    assert_eq!(crate::app::capitalize(""), "");
}

#[test]
fn report_error_from_a_screen_becomes_an_error_toast() {
    let (_repo, mut app) = setup_app();

    let next = app.dispatch(Action::ReportError {
        message: "internal error".to_string(),
    });

    assert_eq!(next, None);
    assert_eq!(app.error(), Some("Internal error"));
}

#[test]
fn report_error_replaces_a_previous_error_instead_of_piling_up() {
    let (_repo, mut app) = setup_app();
    app.dispatch(Action::SaveProject {
        project_id: None,
        name: " ".to_string(),
        description: String::new(),
        working_dir: String::new(),
    });

    app.dispatch(Action::ReportError {
        message: "database is busy; retry the operation".to_string(),
    });

    assert_eq!(app.toasts().len(), 1);
    assert_eq!(app.error(), Some("Database is busy; retry the operation"));
}
