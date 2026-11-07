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

#[test]
fn move_task_reassigns_only_version_id() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let id = repo
            .create_task(pid, v1, "A", "desc", Status::InProgress, Priority::High)
            .unwrap()
            .id
            .unwrap();
        let before = repo.get_task(id).unwrap();

        let moved = repo.move_task(id, v2).expect(name);
        assert_eq!(moved.version_id, v2, "{name}");
        assert_eq!(moved.name, before.name, "{name}");
        assert_eq!(moved.description, before.description, "{name}");
        assert_eq!(moved.status, before.status, "{name}");
        assert_eq!(moved.priority, before.priority, "{name}");
        assert_eq!(moved.created_at, before.created_at, "{name}");
        assert_eq!(repo.get_task(id).unwrap().version_id, v2, "{name}");
        assert!(repo.list_tasks_by_version(v1).unwrap().is_empty(), "{name}");
        assert_eq!(repo.list_tasks_by_version(v2).unwrap().len(), 1, "{name}");
    });
}

#[test]
fn move_task_keeps_notes() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let id = repo
            .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        repo.add_task_note(id, "progress").unwrap();
        repo.move_task(id, v2).expect(name);
        let notes = repo.list_task_notes(id).unwrap();
        assert_eq!(notes.len(), 1, "{name}");
        assert_eq!(notes[0].content, "progress", "{name}");
    });
}

#[test]
fn move_task_rejects_version_from_another_project() {
    for_each_repo(|name, repo| {
        let p1 = repo.create_project("P1", "").unwrap().id.unwrap();
        let p2 = repo.create_project("P2", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(p1).unwrap();
        let foreign = repo.default_version_id(p2).unwrap();
        let id = repo
            .create_task(p1, v1, "A", "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        let err = repo.move_task(id, foreign).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{name}: {err:?}");
        assert_eq!(repo.get_task(id).unwrap().version_id, v1, "{name}");
    });
}

#[test]
fn move_task_rejects_tasks_with_dependencies_or_dependents() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let v2 = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v1, "A");
        let b = task_in_version(repo, pid, v1, "B");
        repo.add_dependency(b, a).unwrap();

        // b depends on a: both are blocked from moving.
        let err = repo.move_task(b, v2).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{name}: {err:?}");
        let err = repo.move_task(a, v2).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{name}: {err:?}");
        assert_eq!(repo.get_task(a).unwrap().version_id, v1, "{name}");
        assert_eq!(repo.get_task(b).unwrap().version_id, v1, "{name}");
        assert_eq!(repo.get_dependencies(b).unwrap(), vec![a], "{name}");

        // Once the edge is removed, the move succeeds.
        repo.remove_dependency(b, a).unwrap();
        repo.move_task(a, v2).expect(name);
    });
}

#[test]
fn move_task_to_current_version_is_noop() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let a = task_in_version(repo, pid, v1, "A");
        let b = task_in_version(repo, pid, v1, "B");
        repo.add_dependency(b, a).unwrap();
        // No-op even with dependencies: nothing is written.
        let same = repo.move_task(b, v1).expect(name);
        assert_eq!(same.version_id, v1, "{name}");
        assert_eq!(repo.get_dependencies(b).unwrap(), vec![a], "{name}");
    });
}

#[test]
fn move_task_unknown_task_or_version_returns_not_found() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v1 = repo.default_version_id(pid).unwrap();
        let id = task_in_version(repo, pid, v1, "A");
        let err = repo.move_task(999_999, v1).unwrap_err();
        assert!(matches!(err, AppError::NotFound), "{name}: task {err:?}");
        let err = repo.move_task(id, 999_999).unwrap_err();
        assert!(matches!(err, AppError::NotFound), "{name}: version {err:?}");
    });
}

#[test]
fn can_compact_rejects_non_done_tasks() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        assert!(
            matches!(repo.can_compact(v), Err(AppError::Validation(_))),
            "{name}: todo task must block compaction"
        );
        repo.update_task_status(a, Status::Done).unwrap();
        assert!(repo.can_compact(v).is_ok(), "{name}");
    });
}

#[test]
fn can_compact_rejects_empty_version_without_previous_summary() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        assert!(
            matches!(repo.can_compact(v), Err(AppError::Validation(_))),
            "{name}"
        );
    });
}

#[test]
fn can_compact_accepts_empty_version_with_previous_summary() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        repo.create_version_summary(v, "First", &snapshot).unwrap();
        repo.delete_done_tasks_by_version(v).unwrap();
        assert!(repo.can_compact(v).is_ok(), "{name}");
    });
}

#[test]
fn build_compact_snapshot_omits_previous_summary_on_first_compaction() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        let json: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
        assert!(json["previous_summary"].is_null(), "{name}");
        assert_eq!(json["tasks"].as_array().unwrap().len(), 1, "{name}");
    });
}

#[test]
fn build_compact_snapshot_includes_previous_summary_recursively() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();

        let first = repo.build_compact_snapshot(v).unwrap();
        repo.create_version_summary(v, "First", &first).unwrap();

        let second = repo.build_compact_snapshot(v).unwrap();
        let second_json: serde_json::Value = serde_json::from_str(&second).unwrap();
        let prev = &second_json["previous_summary"];
        assert!(!prev.is_null(), "{name}: previous_summary must be present");
        assert_eq!(prev["summary"], "First", "{name}");

        let prev_snapshot: serde_json::Value =
            serde_json::from_str(prev["snapshot_json"].as_str().unwrap()).unwrap();
        let first_json: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(prev_snapshot, first_json, "{name}");
    });
}

#[test]
fn delete_done_tasks_by_version_cascades_notes_and_dependencies() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let keep_v = repo.default_version_id(pid).unwrap();
        let keep = task_in_version(repo, pid, keep_v, "Keep");
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        let b = task_in_version(repo, pid, v, "B");
        // b (todo) depends on a (done): the edge must disappear with a.
        repo.add_dependency(b, a).unwrap();
        repo.add_task_note(a, "note A").unwrap();
        repo.update_task_status(a, Status::Done).unwrap();

        repo.delete_done_tasks_by_version(v).unwrap();

        assert!(
            matches!(repo.get_task(a), Err(AppError::NotFound)),
            "{name}"
        );
        assert!(repo.get_dependencies(b).unwrap().is_empty(), "{name}");
        assert!(repo.list_task_notes(a).unwrap().is_empty(), "{name}");
        assert!(repo.get_task(keep).is_ok(), "{name}");
    });
}

#[test]
fn delete_done_tasks_by_version_keeps_todo_tasks_intact() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let done = task_in_version(repo, pid, v, "Done");
        let todo = task_in_version(repo, pid, v, "Todo");
        repo.add_task_note(todo, "keep me").unwrap();
        repo.update_task_status(done, Status::Done).unwrap();

        repo.delete_done_tasks_by_version(v).unwrap();

        assert!(
            matches!(repo.get_task(done), Err(AppError::NotFound)),
            "{name}"
        );
        let remaining = repo.get_task(todo).unwrap();
        assert_eq!(remaining.status, Status::Todo, "{name}");
        assert_eq!(repo.list_task_notes(todo).unwrap().len(), 1, "{name}");
        assert_eq!(repo.list_tasks_by_version(v).unwrap().len(), 1, "{name}");
    });
}

#[test]
fn can_compact_rejects_in_progress_tasks() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let done = task_in_version(repo, pid, v, "Done");
        let wip = task_in_version(repo, pid, v, "WIP");
        repo.update_task_status(done, Status::Done).unwrap();
        repo.update_task_status(wip, Status::InProgress).unwrap();
        assert!(
            matches!(repo.can_compact(v), Err(AppError::Validation(_))),
            "{name}: in_progress must block compaction"
        );
    });
}

#[test]
fn can_compact_rejects_only_todo_without_previous_summary() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        task_in_version(repo, pid, v, "A");
        task_in_version(repo, pid, v, "B");
        assert!(
            matches!(repo.can_compact(v), Err(AppError::Validation(_))),
            "{name}"
        );
    });
}

#[test]
fn can_compact_accepts_mix_of_done_and_todo() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let done = task_in_version(repo, pid, v, "Done");
        task_in_version(repo, pid, v, "Todo");
        repo.update_task_status(done, Status::Done).unwrap();
        assert!(repo.can_compact(v).is_ok(), "{name}");
    });
}

#[test]
fn build_compact_snapshot_contains_only_done_tasks_with_their_notes_and_deps() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        let b = task_in_version(repo, pid, v, "B");
        let todo = task_in_version(repo, pid, v, "Todo");
        repo.add_dependency(b, a).unwrap();
        repo.add_dependency(todo, b).unwrap();
        repo.add_task_note(a, "note A").unwrap();
        repo.add_task_note(todo, "note todo").unwrap();
        repo.update_task_status(a, Status::Done).unwrap();
        repo.update_task_status(b, Status::Done).unwrap();

        let snapshot = repo.build_compact_snapshot(v).unwrap();
        let json: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
        let tasks = json["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2, "{name}");
        assert!(tasks.iter().all(|t| t["status"] == "done"), "{name}");
        let notes = json["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 1, "{name}");
        assert_eq!(notes[0]["task_id"], a, "{name}");
        let deps = json["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 1, "{name}");
        assert_eq!(deps[0]["task_id"], b, "{name}");
        assert_eq!(deps[0]["depends_on_id"], a, "{name}");
    });
}

#[test]
fn create_version_summary_rejects_duplicate_for_same_version() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        repo.create_version_summary(v, "First", &snapshot).unwrap();
        assert!(
            matches!(
                repo.create_version_summary(v, "Second", &snapshot),
                Err(AppError::Validation(_))
            ),
            "{name}"
        );
    });
}

#[test]
fn delete_version_summary_allows_recreate() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();
        let snapshot = repo.build_compact_snapshot(v).unwrap();
        let summary = repo.create_version_summary(v, "First", &snapshot).unwrap();
        let id = summary.id.unwrap();

        repo.delete_version_summary(id).unwrap();
        assert!(
            matches!(repo.get_version_summary(id), Err(AppError::NotFound)),
            "{name}"
        );

        let recreated = repo.create_version_summary(v, "Second", &snapshot).unwrap();
        assert_eq!(recreated.summary, "Second", "{name}");
    });
}

#[test]
fn compact_flow_preserves_unique_invariant() {
    for_each_repo(|name, repo| {
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = task_in_version(repo, pid, v, "A");
        repo.update_task_status(a, Status::Done).unwrap();

        let snapshot = repo.build_compact_snapshot(v).unwrap();
        repo.delete_done_tasks_by_version(v).unwrap();
        repo.create_version_summary(v, "Summary", &snapshot)
            .unwrap();

        let summaries = repo.list_version_summaries(v).unwrap();
        assert_eq!(summaries.len(), 1, "{name}");
        assert_eq!(summaries[0].summary, "Summary", "{name}");
        assert_eq!(summaries[0].snapshot_json, snapshot, "{name}");
    });
}

#[test]
fn update_task_status_sets_and_clears_completed_at() {
    for_each_repo(|name, repo| {
        let task_id = create_task(repo);
        let task = repo.get_task(task_id).unwrap();
        assert_eq!(task.completed_at, None, "{name}");

        let done = repo.update_task_status(task_id, Status::Done).unwrap();
        assert_eq!(done.status, Status::Done, "{name}");
        assert!(done.completed_at.is_some(), "{name}");

        let reopened = repo.update_task_status(task_id, Status::Todo).unwrap();
        assert_eq!(reopened.status, Status::Todo, "{name}");
        assert_eq!(reopened.completed_at, None, "{name}");
    });
}
