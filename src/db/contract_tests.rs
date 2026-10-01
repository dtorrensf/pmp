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
    repo.create_task(project.id.unwrap(), "T", "", Status::Todo, Priority::Medium)
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
