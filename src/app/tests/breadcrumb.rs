// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::VersionRepository;
use crate::screens::Screen;

use super::*;

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
