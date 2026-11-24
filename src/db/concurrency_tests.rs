// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Concurrency tests: several connections (as several processes would) writing
//! to the same SQLite file while the lock policy absorbs the contention.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use rusqlite::Connection;

use crate::db::busy::BusyOptions;
use crate::db::repository::{
    DependencyRepository, ProjectRepository, SummaryRepository, TaskRepository, VersionRepository,
};
use crate::db::sqlite::SqliteRepository;
use crate::domain::error::ErrorCode;
use crate::domain::task::{Priority, Status};

/// Policy for tests that want to contend: no waiting inside SQLite (so the lock
/// really is reported as busy) and a generous retry budget with tiny delays.
fn contended() -> BusyOptions {
    BusyOptions {
        busy_timeout: Duration::ZERO,
        max_attempts: 200,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
    }
}

fn open(path: &std::path::Path) -> SqliteRepository {
    SqliteRepository::open_with(path, contended()).unwrap()
}

fn default_version(repo: &SqliteRepository, project_id: i64) -> i64 {
    let versions = repo.list_versions_by_project(project_id).unwrap();
    assert_eq!(versions.len(), 1, "every project keeps its default version");
    versions[0].id.unwrap()
}

#[test]
fn concurrent_writers_on_the_same_file_lose_no_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    // Create the schema up front so the threads only contend on data writes.
    drop(open(&path));

    let workers = 4;
    let per_worker = 5;
    let barrier = Arc::new(Barrier::new(workers));
    let handles: Vec<_> = (0..workers)
        .map(|worker| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let repo = open(&path);
                barrier.wait();
                for index in 0..per_worker {
                    let project = repo
                        .create_project(&format!("p{worker}-{index}"), "", None)
                        .unwrap();
                    let project_id = project.id.unwrap();
                    let version_id = default_version(&repo, project_id);
                    repo.create_task(
                        project_id,
                        version_id,
                        &format!("t{worker}-{index}"),
                        "",
                        Status::Todo,
                        Priority::Medium,
                    )
                    .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("writer thread must not fail");
    }

    let repo = open(&path);
    let projects = repo.list_projects().unwrap();
    assert_eq!(projects.len(), workers * per_worker);
    for project in projects {
        let project_id = project.id.unwrap();
        default_version(&repo, project_id);
        assert_eq!(repo.list_tasks_by_project(project_id).unwrap().len(), 1);
    }
}

#[test]
fn concurrent_edges_never_close_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let repo = open(&path);
    let project_id = repo.create_project("P", "", None).unwrap().id.unwrap();
    let version_id = default_version(&repo, project_id);
    let task = |name: &str| {
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
    };
    let a = task("a");
    let b = task("b");

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [(a, b), (b, a)]
        .into_iter()
        .map(|(from, to)| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let repo = open(&path);
                barrier.wait();
                for _ in 0..20 {
                    // Either orientation may win; the store must never accept
                    // both, and a lost race is reported, not panicked.
                    let _ = repo.set_dependencies(from, &[to]);
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("edge thread must not fail");
    }

    let a_to_b = repo.get_dependencies(a).unwrap().contains(&b);
    let b_to_a = repo.get_dependencies(b).unwrap().contains(&a);
    assert!(
        !(a_to_b && b_to_a),
        "concurrent edges closed a dependency cycle"
    );
}

#[test]
fn concurrent_compactions_keep_one_summary_and_drop_done_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let repo = open(&path);
    let project_id = repo.create_project("P", "", None).unwrap().id.unwrap();
    let version_id = default_version(&repo, project_id);
    let done = repo
        .create_task(
            project_id,
            version_id,
            "Done",
            "",
            Status::Done,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|worker| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let repo = open(&path);
                barrier.wait();
                repo.compact_version(version_id, &format!("summary {worker}"), "{}")
                    .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("compaction thread must not fail");
    }

    let repo = open(&path);
    assert_eq!(repo.list_version_summaries(version_id).unwrap().len(), 1);
    assert!(
        repo.get_task(done).is_err(),
        "done tasks are removed by the compaction"
    );
}

#[test]
fn a_retry_succeeds_once_the_other_connection_releases_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let repo = SqliteRepository::open_with(
        &path,
        BusyOptions {
            busy_timeout: Duration::ZERO,
            max_attempts: 500,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
        },
    )
    .unwrap();

    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        holder.execute_batch("COMMIT").unwrap();
    });

    // Fails immediately inside SQLite, so the retry loop is what waits for the
    // other connection instead of surfacing `db_busy`.
    repo.create_project("P", "", None).unwrap();
    holder.join().unwrap();
    assert_eq!(repo.list_projects().unwrap().len(), 1);
}

#[test]
fn a_lock_that_outlasts_the_budget_reports_db_busy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let repo = SqliteRepository::open_with(
        &path,
        BusyOptions {
            busy_timeout: Duration::ZERO,
            max_attempts: 3,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
    )
    .unwrap();

    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = repo.create_project("P", "", None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DbBusy);
    assert!(error.is_retryable());
    assert!(
        repo.list_projects().unwrap().is_empty(),
        "a failed project insert must not leave rows behind"
    );

    holder.execute_batch("ROLLBACK").unwrap();
    drop(repo.create_project("P", "", None).unwrap());
}
