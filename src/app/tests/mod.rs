// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests for `App` as a coordinator: routing, screen lifecycle, use case
//! orchestration and feedback. One module per concern; shared fixtures live
//! here.

use super::App;
use crate::db::repository::{ProjectRepository, RepositoryPolicy, TaskRepository};
use crate::db::stub::StubRepository;
use crate::domain::task::{Priority, Status};
use crate::screens::Action;
use std::sync::Arc;
use std::time::Instant;

mod breadcrumb;
mod misc;
mod navigation;
mod projects;
mod tasks;
mod toasts;
mod versions;

fn app_with_project() -> (Arc<StubRepository>, App, i64, i64) {
    let repo = Arc::new(StubRepository::default());
    let project = repo.create_project("Alpha", "", None).unwrap();
    let project_id = project.id.unwrap();
    let version_id = repo.default_version_id(project_id).unwrap();
    let app = App::new(repo.clone());
    (repo, app, project_id, version_id)
}

fn setup_app() -> (Arc<StubRepository>, App) {
    let repo = Arc::new(StubRepository::default());
    let app = App::new(repo.clone());
    (repo, app)
}

fn setup_ids() -> (Arc<StubRepository>, App, i64, i64) {
    let repo = Arc::new(StubRepository::default());
    let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version_id = repo.default_version_id(project_id).unwrap();
    let app = App::new(repo.clone());
    (repo, app, project_id, version_id)
}

fn task(repo: &StubRepository, project_id: i64, version_id: i64, name: &str) -> i64 {
    repo.create_task(
        project_id,
        version_id,
        name,
        "",
        Status::Todo,
        Priority::Medium,
    )
    .unwrap()
    .id
    .unwrap()
}

/// Replaces the app clock with one the test moves by hand.
fn manual_clock(app: &mut App) -> Arc<std::sync::atomic::AtomicU64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    let base = Instant::now();
    let millis = Arc::new(AtomicU64::new(0));
    let handle = millis.clone();
    app.set_clock(move || base + std::time::Duration::from_millis(handle.load(Ordering::SeqCst)));
    millis
}

fn set_millis(millis: &Arc<std::sync::atomic::AtomicU64>, value: u64) {
    millis.store(value, std::sync::atomic::Ordering::SeqCst);
}

fn blank_project_save() -> Action {
    Action::SaveProject {
        project_id: None,
        name: "   ".to_string(),
        description: String::new(),
        working_dir: String::new(),
    }
}
