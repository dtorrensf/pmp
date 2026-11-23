// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::ProjectRepository;
use crate::screens::{Action, Screen};

use super::*;

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
fn delete_project_navigates_to_list_with_confirmation() {
    let (repo, mut app, project_id, _) = setup_ids();
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
