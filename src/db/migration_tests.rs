// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end upgrade tests (A8): a database written by PMP 0.6.x is opened by
//! the current build, which must take a backup first and then carry every row
//! through the migrations without losing data.

use std::path::Path;

use refinery::Target;
use rusqlite::Connection;

use crate::db::backup::BACKUPS_DIR;
use crate::db::repository::{
    DependencyRepository, NoteRepository, ProjectRepository, SummaryRepository, TaskRepository,
    VersionRepository,
};
use crate::db::schema;
use crate::db::sqlite::SqliteRepository;
use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

/// The schema PMP 0.6.0 shipped is migration V6 (`project_working_dir` arrived
/// in 0.6.1), so stopping the runner there reproduces the database an old
/// install leaves on disk. The fixture covers every table and relationship the
/// migrations must preserve.
fn database_from_0_6_0(path: &Path) {
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    schema::runner()
        .set_target(Target::Version(6))
        .run(&mut conn)
        .unwrap();
    conn.execute_batch(
        "INSERT INTO projects (id, name, description, created_at)
             VALUES (1, 'P', 'description', '2026-01-01 00:00:00');
         INSERT INTO versions (id, project_id, name, description, status, created_at) VALUES
             (1, 1, 'v1.0.0', '', 'open', '2026-01-02 00:00:00'),
             (2, 1, 'v2.0.0', '', 'backlog', '2026-01-03 00:00:00');
         INSERT INTO tasks
             (id, project_id, version_id, name, description, status, priority, created_at, completed_at)
             VALUES
             (1, 1, 1, 'Done', '', 'done', 3, '2026-01-04 00:00:00', '2026-01-05 00:00:00'),
             (2, 1, 1, 'Todo', '', 'todo', 1, '2026-01-06 00:00:00', NULL),
             (3, 1, 2, 'Later', '', 'todo', 2, '2026-01-07 00:00:00', NULL);
         INSERT INTO task_notes (id, task_id, content, created_at)
             VALUES (1, 1, 'kept note', '2026-01-08 00:00:00');
         INSERT INTO task_dependencies (task_id, depends_on_id) VALUES (2, 1);
         INSERT INTO version_summaries
             (id, version_id, summary, snapshot_json, generated_at, created_at)
             VALUES (1, 1, 'frozen', '{\"tasks\":[]}', '2026-01-09 00:00:00', '2026-01-09 00:00:00');",
    )
    .unwrap();
}

fn backup_files(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("cannot list backups: {error}"),
    }
}

#[test]
fn upgrading_from_the_0_6_0_schema_backs_up_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pmp.db");
    database_from_0_6_0(&path);

    let repo = SqliteRepository::open(&path).unwrap();

    // A backup was taken before the pending migrations ran, and it is a full
    // database with the old schema and its rows.
    let backups = backup_files(&dir.path().join(BACKUPS_DIR));
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(
        backups[0].contains("pre-migration-schema-6-to-"),
        "{}",
        backups[0]
    );
    let copy = Connection::open(dir.path().join(BACKUPS_DIR).join(&backups[0])).unwrap();
    let copied_tasks: i64 = copy
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(copied_tasks, 3);

    // Nothing was lost or changed by the upgrade.
    assert_eq!(repo.list_projects().unwrap().len(), 1);
    let versions = repo.list_versions_by_project(1).unwrap();
    assert_eq!(versions.len(), 2);
    let version = |name: &str| {
        versions
            .iter()
            .find(|version| version.name == name)
            .unwrap_or_else(|| panic!("version {name} was lost"))
            .clone()
    };
    assert_eq!(version("v1.0.0").status, VersionStatus::Open);
    assert_eq!(version("v2.0.0").status, VersionStatus::Backlog);

    let tasks = repo.list_tasks_by_project(1).unwrap();
    assert_eq!(tasks.len(), 3);
    let done = repo.get_task(1).unwrap();
    assert_eq!(done.status, Status::Done);
    assert_eq!(done.priority, Priority::High);
    assert_eq!(done.completed_at.as_deref(), Some("2026-01-05 00:00:00"));
    assert_eq!(repo.get_task(2).unwrap().status, Status::Todo);

    assert_eq!(repo.get_dependencies(2).unwrap(), vec![1]);
    assert_eq!(repo.list_task_notes(1).unwrap()[0].content, "kept note");
    assert_eq!(repo.list_version_summaries(1).unwrap()[0].summary, "frozen");

    let mut conn = Connection::open(&path).unwrap();
    assert_eq!(
        schema::applied_version(&mut conn).unwrap(),
        schema::latest_version()
    );
}

#[test]
fn reopening_an_up_to_date_database_does_not_create_another_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pmp.db");
    database_from_0_6_0(&path);

    drop(SqliteRepository::open(&path).unwrap());
    let after_first = backup_files(&dir.path().join(BACKUPS_DIR));
    assert_eq!(after_first.len(), 1);

    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(backup_files(&dir.path().join(BACKUPS_DIR)), after_first);
}

#[test]
fn a_fresh_database_starts_without_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pmp.db");

    let repo = SqliteRepository::open(&path).unwrap();

    assert!(repo.list_projects().unwrap().is_empty());
    assert!(backup_files(&dir.path().join(BACKUPS_DIR)).is_empty());
    let mut conn = Connection::open(&path).unwrap();
    assert_eq!(
        schema::applied_version(&mut conn).unwrap(),
        schema::latest_version()
    );
}
