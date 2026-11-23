// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::{
    ProjectRepository, RepositoryPolicy, SummaryRepository, TaskRepository, VersionRepository,
};
use crate::db::stub::StubRepository;
use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;
use crate::screens::components::toast::Severity;
use crate::screens::test_utils::key_event;
use crate::screens::{Action, Screen};
use crossterm::event::KeyCode;

use super::*;

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
