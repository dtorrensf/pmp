// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Embedded schema migrations and the queries that describe them.
//!
//! Kept apart from [`SqliteRepository`](super::sqlite::SqliteRepository) so the
//! pre-migration backup (A8) can ask which migrations are pending before the
//! repository applies them, and so the migration tests can replay an older
//! schema without opening the store.

use std::collections::HashSet;

use refinery::{Migration, Runner};
use rusqlite::Connection;

use crate::db::busy::is_lock_error;
use crate::domain::error::AppError;

/// Table refinery keeps its applied-migration bookkeeping in.
const MIGRATION_TABLE: &str = "refinery_schema_history";

mod embedded {
    refinery::embed_migrations!("src/db/migrations");
}

/// A fresh runner over every migration bundled in this build.
pub fn runner() -> Runner {
    embedded::migrations::runner()
}

/// Every bundled migration, oldest first.
pub fn bundled() -> Vec<Migration> {
    runner().get_migrations().clone()
}

/// Schema version this build migrates to.
pub fn latest_version() -> u32 {
    bundled().last().map(Migration::version).unwrap_or(0)
}

/// True when refinery's bookkeeping table exists, i.e. migrations ran on this
/// database at least once. A freshly created file has no table and no history.
fn has_history(conn: &Connection) -> Result<bool, AppError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [MIGRATION_TABLE],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Highest migration applied to `conn`; `0` when none ran yet.
pub fn applied_version(conn: &mut Connection) -> Result<u32, AppError> {
    if !has_history(conn)? {
        return Ok(0);
    }
    let applied = runner().get_applied_migrations(conn).map_err(error)?;
    Ok(applied.iter().map(Migration::version).max().unwrap_or(0))
}

/// Migrations bundled in this build but not applied to `conn` yet, oldest
/// first. An empty list means the schema is up to date.
pub fn pending(conn: &mut Connection) -> Result<Vec<Migration>, AppError> {
    if !has_history(conn)? {
        return Ok(bundled());
    }
    let applied: HashSet<u32> = runner()
        .get_applied_migrations(conn)
        .map_err(error)?
        .into_iter()
        .map(|migration| migration.version())
        .collect();
    Ok(bundled()
        .into_iter()
        .filter(|migration| !applied.contains(&migration.version()))
        .collect())
}

/// Applies every pending migration. Kept in one place so the repository and
/// the tests run exactly the same code.
pub fn migrate(conn: &mut Connection) -> Result<(), AppError> {
    runner().run(conn).map_err(error)?;
    Ok(())
}

/// Classifies a migration failure: a locked database is transient and retried,
/// anything else is a validation error for the caller.
pub fn error(error: refinery::Error) -> AppError {
    if is_lock_error(&error) {
        AppError::DbBusy
    } else {
        AppError::Validation(format!("migration error: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("schema.db")).unwrap();
        (dir, conn)
    }

    #[test]
    fn a_fresh_database_has_every_migration_pending() {
        let (_dir, mut conn) = temp_db();
        let pending = pending(&mut conn).unwrap();
        assert_eq!(pending.len(), bundled().len());
        assert_eq!(applied_version(&mut conn).unwrap(), 0);
    }

    #[test]
    fn applying_everything_leaves_nothing_pending() {
        let (_dir, mut conn) = temp_db();
        migrate(&mut conn).unwrap();
        assert!(pending(&mut conn).unwrap().is_empty());
        assert_eq!(applied_version(&mut conn).unwrap(), latest_version());
        assert!(latest_version() >= 1);
    }

    #[test]
    fn pending_only_reports_migrations_above_the_target() {
        use refinery::Target;
        let (_dir, mut conn) = temp_db();
        runner()
            .set_target(Target::Version(6))
            .run(&mut conn)
            .unwrap();
        assert_eq!(applied_version(&mut conn).unwrap(), 6);
        let pending = pending(&mut conn).unwrap();
        assert!(pending.iter().all(|migration| migration.version() > 6));
        assert_eq!(
            pending.len(),
            bundled()
                .iter()
                .filter(|migration| migration.version() > 6)
                .count()
        );
    }
}
