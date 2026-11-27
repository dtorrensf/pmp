// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Automatic backups of the shared SQLite file before schema migrations (A8).
//!
//! Upgrading from 0.6.x runs new migrations over `pmp.db`. If one of them fails
//! halfway, the store can be left between two schemas, so the repository copies
//! the database first and restores that copy when a migration reports an error.
//!
//! The copy uses `VACUUM INTO`, which writes a consistent snapshot (WAL
//! included) into a temporary file and never reads a half-written page. Only
//! after the copy is on disk is it renamed to its final name, so a crash leaves
//! either no backup or a complete one, never a truncated file that looks valid.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use refinery::Migration;
use rusqlite::Connection;

use crate::db::busy::{BusyOptions, retry_busy};
use crate::domain::error::AppError;
use crate::domain::timestamp::now_utc;

/// Subdirectory of the database's directory that holds the automatic backups.
pub const BACKUPS_DIR: &str = "backups";

/// Copies the database before `pending` migrations run and returns where the
/// backup went. Returns `None` when there is nothing to protect: the schema is
/// already current (no pending migrations), the database is brand new (`from`
/// is 0, so no data can be lost) or it lives in memory.
///
/// A failure to write the backup aborts the upgrade on purpose: migrating
/// without a fallback is exactly what this task exists to prevent.
pub(crate) fn backup_before_migrations(
    conn: &Connection,
    db_path: &Path,
    from: u32,
    pending: &[Migration],
    options: &BusyOptions,
) -> Result<Option<PathBuf>, AppError> {
    let Some(to) = pending.last().map(Migration::version) else {
        return Ok(None);
    };
    if from == 0 || db_path == Path::new(":memory:") {
        return Ok(None);
    }
    let label = format!("pre-migration-schema-{from}-to-{to}");
    backup_database(conn, db_path, &label, options).map(Some)
}

/// Writes a consistent copy of the database inside its `backups/` directory and
/// returns the path. The copy is written to a temporary file, flushed and only
/// then renamed, so it is either absent or complete.
pub(crate) fn backup_database(
    conn: &Connection,
    db_path: &Path,
    label: &str,
    options: &BusyOptions,
) -> Result<PathBuf, AppError> {
    if db_path == Path::new(":memory:") {
        return Err(AppError::Validation(
            "an in-memory database cannot be backed up".to_string(),
        ));
    }
    let dir = backups_dir(db_path);
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::Validation(format!(
            "cannot create backup directory {}: {e}",
            dir.display()
        ))
    })?;
    let target = unique_path(&dir, &format!("{}-{}", file_stem(db_path), sanitize(label)));
    let tmp = dir.join(format!(".{}.tmp", file_stem(db_path)));
    retry_busy(options, || {
        // A failed attempt may have left a partial file; `VACUUM INTO` refuses
        // to overwrite an existing destination.
        let _ = fs::remove_file(&tmp);
        conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])
            .map(|_| ())
            .map_err(AppError::from)
    })
    .map_err(|e| AppError::Validation(format!("cannot back up the database: {e}")))?;
    // Flush before renaming: the rename is atomic, the page cache is not.
    if let Ok(file) = File::open(&tmp) {
        let _ = file.sync_all();
    }
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        AppError::Validation(format!(
            "cannot finalize the backup {}: {e}",
            target.display()
        ))
    })?;
    tracing::info!(backup = %target.display(), "database backed up before migrating");
    Ok(target)
}

/// Puts `backup` back in place of `db_path`, atomically. The `-wal` and `-shm`
/// sidecars are removed first: they belong to the failed schema and replaying
/// them over the restored file would corrupt it.
///
/// The caller must have dropped every open connection to `db_path`.
pub(crate) fn restore_backup(db_path: &Path, backup: &Path) -> Result<(), AppError> {
    let tmp = db_path.with_file_name(format!(".restore-{}.tmp", stamp()));
    fs::copy(backup, &tmp).map_err(|e| {
        AppError::Validation(format!("cannot copy the backup {}: {e}", backup.display()))
    })?;
    if let Ok(file) = File::open(&tmp) {
        let _ = file.sync_all();
    }
    for suffix in ["-wal", "-shm"] {
        match fs::remove_file(sidecar(db_path, suffix)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                return Err(AppError::Validation(format!(
                    "cannot remove the stale SQLite sidecar of {}: {error}",
                    db_path.display()
                )));
            }
        }
    }
    fs::rename(&tmp, db_path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        AppError::Validation(format!(
            "cannot restore the database from {}: {e}",
            backup.display()
        ))
    })?;
    Ok(())
}

/// Reports a failed migration and, when possible, restores the backup taken
/// before it ran. The message names the backup path either way, so a user can
/// restore it by hand when the automatic restore also fails.
pub(crate) fn failed_migration(db_path: &Path, backup: &Path, error: AppError) -> AppError {
    match restore_backup(db_path, backup) {
        Ok(()) => AppError::Validation(format!(
            "migration failed ({error}); the database was restored from the backup at {}",
            backup.display()
        )),
        Err(restore) => AppError::Validation(format!(
            "migration failed ({error}) and the automatic restore also failed ({restore}); \
             the database was left untouched and the backup is at {}",
            backup.display()
        )),
    }
}

/// `backups/` next to the database file, so a copied data directory keeps its
/// backups together with the store they belong to.
pub fn backups_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(BACKUPS_DIR)
}

fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let stem = format!("{stem}-{}", stamp());
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("{stem}.db")
        } else {
            format!("{stem}-{attempt}.db")
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.db", std::process::id()))
}

fn stamp() -> String {
    now_utc().chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn file_stem(db_path: &Path) -> String {
    let raw = db_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pmp".to_string());
    sanitize(&raw)
}

/// Collapses a value into a single, safe file-name component: the label is
/// built from migration numbers today, but a stray separator must never move
/// the backup out of `backups/`.
fn sanitize(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "pmp".to_string()
    } else {
        sanitized
    }
}

fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::ProjectRepository;
    use crate::db::schema;
    use crate::db::sqlite::SqliteRepository;

    fn seed(path: &Path) -> Connection {
        use refinery::Target;
        let mut conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        schema::runner()
            .set_target(Target::Version(6))
            .run(&mut conn)
            .unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, description, created_at) \
             VALUES (1, 'kept', '', '2026-01-01 00:00:00')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn backup_before_migrations_copies_the_database_into_its_backups_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmp.db");
        let mut conn = seed(&path);
        let from = schema::applied_version(&mut conn).unwrap();
        let pending = schema::pending(&mut conn).unwrap();
        assert!(
            !pending.is_empty(),
            "the 0.6.0 schema must have pending work"
        );

        let backup =
            backup_before_migrations(&conn, &path, from, &pending, &BusyOptions::default())
                .unwrap()
                .expect("an upgrade must be backed up");

        assert_eq!(backup.parent().unwrap(), dir.path().join("backups"));
        assert!(backup.exists());
        // The copy is a full database: it holds the row written before it.
        let copy = Connection::open(&backup).unwrap();
        let kept: String = copy
            .query_row("SELECT name FROM projects WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(kept, "kept");
    }

    #[test]
    fn no_backup_is_written_when_nothing_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmp.db");
        let conn = Connection::open(&path).unwrap();
        schema::migrate(&mut Connection::open(&path).unwrap()).unwrap();

        let backup =
            backup_before_migrations(&conn, &path, 7, &[], &BusyOptions::default()).unwrap();
        assert!(backup.is_none());
        assert!(!backups_dir(&path).exists());
    }

    #[test]
    fn a_brand_new_database_is_not_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        let mut conn = Connection::open(&path).unwrap();
        let pending = schema::pending(&mut conn).unwrap();
        let backup =
            backup_before_migrations(&conn, &path, 0, &pending, &BusyOptions::default()).unwrap();
        assert!(backup.is_none());
    }

    #[test]
    fn restore_backup_replaces_the_database_with_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmp.db");
        let repo = SqliteRepository::open(&path).unwrap();
        repo.create_project("kept", "", None).unwrap();

        let backup = {
            let conn = Connection::open(&path).unwrap();
            backup_database(&conn, &path, "manual", &BusyOptions::default()).unwrap()
        };
        // Work done after the backup must be gone once it is restored.
        repo.create_project("later", "", None).unwrap();
        drop(repo);
        restore_backup(&path, &backup).unwrap();

        let reopened = SqliteRepository::open(&path).unwrap();
        let names: Vec<String> = reopened
            .list_projects()
            .unwrap()
            .into_iter()
            .map(|project| project.name)
            .collect();
        assert_eq!(names, vec!["kept".to_string()]);
    }

    #[test]
    fn failed_migration_reports_the_backup_path_and_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmp.db");
        let conn = Connection::open(&path).unwrap();
        schema::migrate(&mut Connection::open(&path).unwrap()).unwrap();
        let backup = backup_database(&conn, &path, "manual", &BusyOptions::default()).unwrap();

        let error = failed_migration(
            &path,
            &backup,
            AppError::Validation("synthetic failure".to_string()),
        );
        let AppError::Validation(message) = error else {
            panic!("expected a validation error");
        };
        assert!(message.contains("restored"), "{message}");
        assert!(message.contains(&backup.display().to_string()), "{message}");
    }

    #[test]
    fn backup_file_names_are_unique_and_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("weird name!.db")).unwrap();
        let first = backup_database(
            &conn,
            &dir.path().join("weird name!.db"),
            "a b/c",
            &BusyOptions::default(),
        )
        .unwrap();
        let second = backup_database(
            &conn,
            &dir.path().join("weird name!.db"),
            "a b/c",
            &BusyOptions::default(),
        )
        .unwrap();
        assert_ne!(first, second);
        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(!name.contains(' ') && !name.contains('/'), "{name}");
            assert!(name.starts_with("weird-name-"), "{name}");
        }
    }

    #[test]
    fn an_in_memory_database_is_never_backed_up() {
        let conn = Connection::open_in_memory().unwrap();
        let error = backup_database(&conn, Path::new(":memory:"), "x", &BusyOptions::default())
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }
}
