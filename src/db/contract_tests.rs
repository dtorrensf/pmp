// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared behavioral contract for `Repository` implementations.
//!
//! Every implementation of the trait must satisfy these tests so that
//! callers can rely on uniform behavior regardless of the backend
//! (Liskov Substitution Principle). Add new behavioral guarantees here
//! whenever the trait contract grows.

use crate::db::repository::Repository;
use crate::db::sqlite::SqliteRepository;
use crate::db::stub::StubRepository;
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

fn sqlite_repo() -> (tempfile::TempDir, SqliteRepository) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contract.db");
    let repo = SqliteRepository::open(&path).unwrap();
    (dir, repo)
}

/// Runs the same assertion against every `Repository` implementation.
/// The implementation name is included in failure messages so the
/// offending backend is identifiable.
fn for_each_repo(f: impl Fn(&str, &dyn Repository)) {
    let (_dir, sqlite) = sqlite_repo();
    f("SqliteRepository", &sqlite);
    f("StubRepository", &StubRepository::default());
}

fn create_task(repo: &dyn Repository) -> i64 {
    let project = repo.create_project("P", "").unwrap();
    repo.create_task(
        project.id.unwrap(),
        repo.default_version_id(project.id.unwrap()).unwrap(),
        "T",
        "",
        Status::Todo,
        Priority::Medium,
    )
    .unwrap()
    .id
    .unwrap()
}

#[test]
fn add_task_note_rejects_empty_content() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        let result = repo.add_task_note(task_id, "");
        assert!(
            matches!(result, Err(AppError::Validation(_))),
            "{name}: expected Validation error for empty content, got {result:?}"
        );
    });
}

#[test]
fn add_task_note_rejects_whitespace_only_content() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        let result = repo.add_task_note(task_id, "  \t\n  ");
        assert!(
            matches!(result, Err(AppError::Validation(_))),
            "{name}: expected Validation error for whitespace-only content, got {result:?}"
        );
    });
}

#[test]
fn add_task_note_accepts_valid_content() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        let note = repo
            .add_task_note(task_id, "valid note")
            .unwrap_or_else(|e| panic!("{name}: valid content rejected: {e}"));
        assert_eq!(note.task_id, task_id, "{name}");
        assert_eq!(note.content, "valid note", "{name}");
        assert!(note.id.is_some(), "{name}");
    });
}

#[test]
fn list_task_notes_orders_newest_first() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        let first = repo.add_task_note(task_id, "first").unwrap();
        let second = repo.add_task_note(task_id, "second").unwrap();
        let notes = repo.list_task_notes(task_id).unwrap();
        assert_eq!(notes.len(), 2, "{name}");
        assert_eq!(
            notes[0].id, second.id,
            "{name}: newest note must come first"
        );
        assert_eq!(notes[1].id, first.id, "{name}: oldest note must come last");
    });
}

#[test]
fn delete_task_note_unknown_id_returns_not_found() {
    for_each_repo(|name, repo| {
        let result = repo.delete_task_note(999);
        assert!(
            matches!(result, Err(AppError::NotFound)),
            "{name}: expected NotFound for unknown note id, got {result:?}"
        );
    });
}

#[test]
fn delete_task_cascades_notes() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        repo.add_task_note(task_id, "cascade me").unwrap();
        repo.delete_task(task_id).unwrap();
        assert!(
            repo.list_task_notes(task_id).unwrap().is_empty(),
            "{name}: notes must be cascade-deleted with their task"
        );
    });
}

#[test]
fn create_version_round_trips_and_defaults_to_open() {
    for_each_repo(|name, repo| {
        let project = repo.create_project("P", "").unwrap();
        let pid = project.id.unwrap();
        let version = repo.create_version(pid, "v1.0.0", "first").unwrap();
        assert!(version.id.is_some(), "{name}");
        assert_eq!(version.name, "v1.0.0", "{name}");
        assert_eq!(version.description, "first", "{name}");
        assert_eq!(version.status, VersionStatus::Open, "{name}");
        assert_eq!(
            repo.get_version(version.id.unwrap()).unwrap(),
            version,
            "{name}"
        );
    });
}

#[test]
fn create_version_rejects_empty_and_duplicate_names() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        assert!(
            matches!(
                repo.create_version(pid, "  ", ""),
                Err(AppError::Validation(_))
            ),
            "{name}: blank name"
        );
        repo.create_version(pid, "v1.0.0", "").unwrap();
        assert!(
            matches!(
                repo.create_version(pid, "v1.0.0", ""),
                Err(AppError::Validation(_))
            ),
            "{name}: duplicate name"
        );
    });
}

#[test]
fn new_project_has_default_version() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let versions = repo.list_versions_by_project(pid).unwrap();
        assert_eq!(versions.len(), 1, "{name}");
        assert_eq!(versions[0].name, "v0", "{name}");
    });
}

#[test]
fn list_versions_puts_open_before_released() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let released = repo.create_version(pid, "v1.0.0", "").unwrap();
        repo.update_version(released.id.unwrap(), "v1.0.0", "", VersionStatus::Released)
            .unwrap();
        repo.create_version(pid, "v1.1.0", "").unwrap();
        let names: Vec<String> = repo
            .list_versions_by_project(pid)
            .unwrap()
            .into_iter()
            .map(|v| v.name)
            .collect();
        assert_eq!(
            names.last().map(String::as_str),
            Some("v1.0.0"),
            "{name}: {names:?}"
        );
        assert_eq!(
            names.first().map(String::as_str),
            Some("v1.1.0"),
            "{name}: {names:?}"
        );
    });
}

#[test]
fn update_version_rejects_duplicate_name_and_missing_id() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let a = repo.create_version(pid, "v1.0.0", "").unwrap();
        repo.create_version(pid, "v1.1.0", "").unwrap();
        assert!(
            matches!(
                repo.update_version(a.id.unwrap(), "v1.1.0", "", VersionStatus::Open),
                Err(AppError::Validation(_))
            ),
            "{name}: rename to existing"
        );
        assert!(
            matches!(
                repo.update_version(9_999, "x", "", VersionStatus::Open),
                Err(AppError::NotFound)
            ),
            "{name}: missing id"
        );
    });
}

#[test]
fn next_version_name_follows_highest_semver() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        assert_eq!(repo.next_version_name(pid).unwrap(), "v0.1.0", "{name}");
        repo.create_version(pid, "v0.1.0", "").unwrap();
        repo.create_version(pid, "v1.2.3", "").unwrap();
        assert_eq!(repo.next_version_name(pid).unwrap(), "v1.3.0", "{name}");
    });
}

#[test]
fn create_task_rejects_version_from_another_project() {
    for_each_repo(|name, repo| {
        let a = repo.create_project("A", "").unwrap().id.unwrap();
        let b = repo.create_project("B", "").unwrap().id.unwrap();
        let foreign = repo.default_version_id(b).unwrap();
        assert!(
            matches!(
                repo.create_task(a, foreign, "T", "", Status::Todo, Priority::Low),
                Err(AppError::Validation(_))
            ),
            "{name}"
        );
    });
}

#[test]
fn add_dependency_rejects_cross_version_tasks() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = repo
            .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        let b = repo
            .create_task(pid, v2, "B", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        assert!(
            matches!(repo.add_dependency(a, b), Err(AppError::Validation(_))),
            "{name}"
        );
    });
}

#[test]
fn delete_version_removes_its_tasks_and_dependencies() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let keep = repo
            .create_task(pid, v1, "Keep", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        let gone = repo
            .create_task(pid, v2, "Gone", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        let gone2 = repo
            .create_task(pid, v2, "Gone2", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        repo.add_dependency(gone, gone2).unwrap();
        repo.delete_version(v2).unwrap();
        assert!(repo.get_version(v2).is_err(), "{name}");
        assert!(
            matches!(repo.get_task(gone), Err(AppError::NotFound)),
            "{name}"
        );
        assert!(repo.get_dependencies(gone).unwrap().is_empty(), "{name}");
        let remaining: Vec<i64> = repo
            .list_tasks_by_project(pid)
            .unwrap()
            .into_iter()
            .filter_map(|t| t.id)
            .collect();
        assert_eq!(remaining, vec![keep], "{name}");
    });
}

fn task_in_version(repo: &dyn Repository, pid: i64, version: i64, name: &str) -> i64 {
    repo.create_task(pid, version, name, "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap()
}

#[test]
fn set_dependencies_replaces_full_set_and_clears_on_empty() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.default_version_id(pid).unwrap();
        let a = task_in_version(repo, pid, v, "A");
        let b = task_in_version(repo, pid, v, "B");
        let c = task_in_version(repo, pid, v, "C");
        repo.add_dependency(a, b).unwrap();

        repo.set_dependencies(a, &[c, c]).expect(name);
        assert_eq!(repo.get_dependencies(a).unwrap(), vec![c], "{name}");

        repo.set_dependencies(a, &[]).expect(name);
        assert!(repo.get_dependencies(a).unwrap().is_empty(), "{name}");
    });
}

#[test]
fn set_dependencies_rejects_invalid_sets_without_changing_anything() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v2", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v1, "A");
        let b = task_in_version(repo, pid, v1, "B");
        let other = task_in_version(repo, pid, v2, "Other");
        repo.add_dependency(a, b).unwrap();

        // self-dependency
        assert!(
            matches!(
                repo.set_dependencies(a, &[b, a]),
                Err(AppError::DependencyCycle)
            ),
            "{name}: self"
        );
        // cross-version
        assert!(
            matches!(
                repo.set_dependencies(a, &[other]),
                Err(AppError::Validation(_))
            ),
            "{name}: cross-version"
        );
        // cycle: b depends on a while a depends on b
        assert!(
            matches!(
                repo.set_dependencies(b, &[a]),
                Err(AppError::DependencyCycle)
            ),
            "{name}: cycle"
        );
        // unknown dependency id
        assert!(
            matches!(
                repo.set_dependencies(a, &[999_999]),
                Err(AppError::NotFound)
            ),
            "{name}: unknown"
        );
        // unknown task
        assert!(
            matches!(
                repo.set_dependencies(999_999, &[a]),
                Err(AppError::NotFound)
            ),
            "{name}: unknown task"
        );
        assert_eq!(repo.get_dependencies(a).unwrap(), vec![b], "{name}");
    });
}

#[test]
fn set_dependencies_detects_cycles_inside_the_new_set() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.default_version_id(pid).unwrap();
        let a = task_in_version(repo, pid, v, "A");
        let b = task_in_version(repo, pid, v, "B");
        let c = task_in_version(repo, pid, v, "C");
        // c -> b already exists; setting b -> c would close a cycle with the new set.
        repo.add_dependency(c, b).unwrap();
        assert!(
            matches!(
                repo.set_dependencies(b, &[a, c]),
                Err(AppError::DependencyCycle)
            ),
            "{name}"
        );
        assert!(repo.get_dependencies(b).unwrap().is_empty(), "{name}");
    });
}

#[test]
fn update_task_priority_changes_only_priority() {
    for_each_repo(|name, repo| {
        let id = create_task(repo);
        let before = repo.get_task(id).unwrap();
        let updated = repo.update_task_priority(id, Priority::High).expect(name);
        assert_eq!(updated.priority, Priority::High, "{name}");
        assert_eq!(updated.name, before.name, "{name}");
        assert_eq!(updated.status, before.status, "{name}");
        assert_eq!(updated.description, before.description, "{name}");
        assert_eq!(
            repo.get_task(id).unwrap().priority,
            Priority::High,
            "{name}"
        );
    });
}

#[test]
fn update_task_priority_unknown_id_returns_not_found() {
    for_each_repo(|name, repo| {
        let err = repo
            .update_task_priority(999_999, Priority::Low)
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound), "{name}: {err:?}");
    });
}

#[test]
fn list_tasks_by_version_only_returns_that_version() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        repo.create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, v2, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let tasks = repo.list_tasks_by_version(v2).unwrap();
        assert_eq!(tasks.len(), 1, "{name}");
        assert_eq!(tasks[0].name, "B", "{name}");
        assert_eq!(repo.list_tasks_by_project(pid).unwrap().len(), 2, "{name}");
    });
}
