// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, Value, ValueRef};
use rusqlite::{
    Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params, params_from_iter,
};

use crate::db::busy::{BusyOptions, is_lock_error, retry_busy};

use crate::db::repository::{
    DependencyRepository, LIST_LIMIT, NoteRepository, ProjectRepository, SummaryRepository,
    TaskFilter, TaskRepository, VersionRepository, check_dependency_set, check_done_transition,
    check_list_size, check_task_move, unique_ids,
};
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};
use crate::domain::validation;
use crate::domain::version::{self, Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("src/db/migrations");
}

/// Column lists shared by every query that maps a row with `task_from_row`
/// or `version_from_row`; those read the columns by name.
const TASK_COLUMNS: &str = "id, project_id, name, description, status, priority, \
                            created_at, version_id, completed_at";
const VERSION_COLUMNS: &str = "id, project_id, name, description, status, created_at";

pub struct SqliteRepository {
    conn: Mutex<Connection>,
    options: BusyOptions,
}

impl SqliteRepository {
    /// Opens the store with the default locking policy. The unified
    /// configuration resolves the policy once and passes it to
    /// [`Self::open_with`]; this shortcut keeps tests and one-off tools simple.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::open_with(path, BusyOptions::default())
    }

    /// Opens the store with an explicit locking policy.
    pub fn open_with(path: impl AsRef<Path>, options: BusyOptions) -> Result<Self, AppError> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(options.busy_timeout)?;
        // `journal_mode` needs a write lock the first time the file switches to
        // WAL, so two processes opening a fresh database at once can collide.
        retry_busy(&options, || {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;",
            )?;
            Ok(())
        })?;
        retry_busy(&options, || {
            embedded::migrations::runner()
                .run(&mut conn)
                .map_err(migration_error)?;
            Ok(())
        })?;
        Ok(Self {
            conn: Mutex::new(conn),
            options,
        })
    }

    /// Locks the connection and runs one write statement, retrying transient
    /// lock errors. The lock is released between attempts so the other threads
    /// of this process can make progress.
    fn write<T>(
        &self,
        mut operation: impl FnMut(&Connection) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        retry_busy(&self.options, || {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            operation(&conn)
        })
    }

    /// Applies a status change together with its done-transition check inside
    /// one write transaction, so a concurrent dependency insert cannot slip
    /// between the check and the update.
    ///
    /// `apply` runs inside the transaction and must only write: no repository
    /// call (it would take the connection lock again) and no user input.
    fn checked_status_write(
        &self,
        id: i64,
        new_status: Status,
        apply: impl Fn(&Connection, Status) -> Result<usize, AppError>,
    ) -> Result<Task, AppError> {
        self.write_transaction(|tx| {
            let current = task_by_id(tx, id)?;
            check_done_transition(current.status, new_status, || {
                incomplete_dependencies_of(tx, id)
            })?;
            apply(tx, new_status)?;
            task_by_id(tx, id)
        })
    }

    /// Runs a compound write inside `BEGIN IMMEDIATE`, retrying the whole block
    /// on transient lock errors.
    ///
    /// Taking the write lock up front is what makes the retry deterministic:
    /// with a deferred transaction SQLite upgrades the lock on the first write
    /// and fails immediately with `SQLITE_BUSY` if someone else got there first,
    /// no matter the `busy_timeout`.
    ///
    /// The closure must only talk to the database: it may never wait for user
    /// input, or the write lock would be held while the UI blocks. Dropping the
    /// transaction without committing (any `?`) rolls every write back.
    fn write_transaction<T>(
        &self,
        mut operation: impl FnMut(&Transaction<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        retry_busy(&self.options, || {
            let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = operation(&tx)?;
            tx.commit()?;
            Ok(value)
        })
    }
}

/// Whole dependency graph, loaded inside a transaction when a write needs to
/// validate it against the same snapshot it is about to modify.
fn dependency_graph(conn: &Connection) -> Result<HashMap<i64, Vec<i64>>, AppError> {
    let mut stmt = conn.prepare("SELECT task_id, depends_on_id FROM task_dependencies")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    let mut graph = HashMap::new();
    for row in rows {
        let (task_id, depends_on_id) = row?;
        graph
            .entry(task_id)
            .or_insert_with(Vec::new)
            .push(depends_on_id);
    }
    Ok(graph)
}

/// Reads a task from any connection, including one inside a write transaction.
fn task_by_id(conn: &Connection, id: i64) -> Result<Task, AppError> {
    conn.query_row(
        &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
        [id],
        task_from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
        _ => e.into(),
    })
}

/// Reads a version from any connection, including one inside a write
/// transaction.
fn version_by_id(conn: &Connection, id: i64) -> Result<Version, AppError> {
    conn.query_row(
        &format!("SELECT {VERSION_COLUMNS} FROM versions WHERE id = ?1"),
        [id],
        version_from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
        _ => e.into(),
    })
}

/// Dependencies of `task_id` that are not Done yet, read from any connection so
/// the done transition can be checked inside its own write transaction.
fn incomplete_dependencies_of(conn: &Connection, task_id: i64) -> Result<Vec<i64>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT d.depends_on_id FROM task_dependencies d
         JOIN tasks t ON t.id = d.depends_on_id
         WHERE d.task_id = ?1 AND t.status != ?2
         ORDER BY d.depends_on_id",
    )?;
    let blockers = stmt
        .query_map(params![task_id, Status::Done.as_str()], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(blockers)
}

/// FROM clause and bound values of the ready tasks of a project: todo tasks
/// whose dependencies are all done. Shared by the listing and the claim so
/// both apply the same rule.
fn ready_filter(project_id: i64, version_id: Option<i64>) -> (String, Vec<Value>) {
    let mut sql = String::from(
        "FROM tasks t WHERE t.project_id = ?1 AND t.status = ?2 AND NOT EXISTS (
            SELECT 1 FROM task_dependencies d
            JOIN tasks b ON b.id = d.depends_on_id
            WHERE d.task_id = t.id AND b.status != ?3)",
    );
    let mut values = vec![
        Value::Integer(project_id),
        Value::Text(Status::Todo.as_str().to_string()),
        Value::Text(Status::Done.as_str().to_string()),
    ];
    if let Some(version_id) = version_id {
        sql.push_str(" AND t.version_id = ?4");
        values.push(Value::Integer(version_id));
    }
    (sql, values)
}

/// Direct dependencies of a task, read from any connection.
fn dependencies_of(conn: &Connection, task_id: i64) -> Result<Vec<i64>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1 ORDER BY depends_on_id",
    )?;
    let deps = stmt
        .query_map([task_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(deps)
}

/// Direct dependents of a task, read from any connection.
fn dependents_of(conn: &Connection, task_id: i64) -> Result<Vec<i64>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1 ORDER BY task_id",
    )?;
    let dependents = stmt
        .query_map([task_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(dependents)
}

/// Classifies a migration failure: a locked database is transient and retried,
/// anything else is reported as before.
fn migration_error(error: refinery::Error) -> AppError {
    if is_lock_error(&error) {
        AppError::DbBusy
    } else {
        AppError::Validation(format!("migration error: {error}"))
    }
}

impl ProjectRepository for SqliteRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description, created_at, working_dir FROM projects ORDER BY id LIMIT {LIST_LIMIT}"
        ))?;
        let projects = stmt
            .query_map([], project_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(projects)
    }

    fn create_project(
        &self,
        name: &str,
        description: &str,
        working_dir: Option<&str>,
    ) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        // The project and its default version land together: a retry must never
        // leave a project without the version every screen expects.
        let id = self.write_transaction(|tx| {
            tx.execute(
                "INSERT INTO projects (name, description, working_dir) \
                 VALUES (?1, ?2, ?3)",
                params![name, description, working_dir],
            )?;
            let id = tx.last_insert_rowid();
            // Every project starts with the default version in Backlog, as the V3
            // backfill does for pre-existing projects.
            tx.execute(
                "INSERT INTO versions (project_id, name, status) VALUES (?1, ?2, ?3)",
                params![
                    id,
                    version::DEFAULT_VERSION_NAME,
                    VersionStatus::Backlog.as_str()
                ],
            )?;
            Ok(id)
        })?;
        self.get_project(id)
    }

    fn update_project(
        &self,
        id: i64,
        name: &str,
        description: &str,
        working_dir: Option<&str>,
    ) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        self.write(|conn| {
            conn.execute(
                "UPDATE projects SET name = ?1, description = ?2, working_dir = ?3 \
                 WHERE id = ?4",
                params![name, description, working_dir, id],
            )?;
            Ok(())
        })?;
        self.get_project(id)
    }

    fn get_project(&self, id: i64) -> Result<Project, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, name, description, created_at, working_dir FROM projects WHERE id = ?1",
            [id],
            project_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            _ => e.into(),
        })
    }

    fn delete_project(&self, id: i64) -> Result<(), AppError> {
        self.write(|conn| {
            conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}

impl VersionRepository for SqliteRepository {
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM versions WHERE project_id = ?1
                 ORDER BY CASE status WHEN ?2 THEN 0 WHEN ?3 THEN 1 ELSE 2 END, id DESC
                 LIMIT {LIST_LIMIT}"
        ))?;
        let versions = stmt
            .query_map(
                params![
                    project_id,
                    VersionStatus::Open.as_str(),
                    VersionStatus::Backlog.as_str()
                ],
                version_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(versions)
    }

    fn create_version_with_status(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
        status: VersionStatus,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        self.get_project(project_id)?;
        let id = self.write(|conn| {
            conn.execute(
                "INSERT INTO versions (project_id, name, description, status) VALUES (?1, ?2, ?3, ?4)",
                params![project_id, name, description, status.as_str()],
            )
            .map_err(|e| version_write_error(e, name))?;
            Ok(conn.last_insert_rowid())
        })?;
        self.get_version(id)
    }

    fn get_version(&self, id: i64) -> Result<Version, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        version_by_id(&conn, id)
    }

    fn update_version(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: VersionStatus,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        let updated = self.write(|conn| {
            conn.execute(
                "UPDATE versions SET name = ?1, description = ?2, status = ?3 WHERE id = ?4",
                params![name, description, status.as_str(), id],
            )
            .map_err(|e| version_write_error(e, name))
        })?;
        if updated == 0 {
            return Err(AppError::NotFound);
        }
        self.get_version(id)
    }

    fn delete_version(&self, id: i64) -> Result<(), AppError> {
        self.write(|conn| {
            conn.execute("DELETE FROM versions WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}

impl TaskRepository for SqliteRepository {
    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE project_id = ?1
                 ORDER BY CASE WHEN status = ?2 THEN 1 ELSE 0 END, priority DESC, created_at DESC, id DESC
                 LIMIT {LIST_LIMIT}"
        ))?;
        let tasks = stmt
            .query_map(params![project_id, Status::Done.as_str()], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(tasks)
    }

    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE version_id = ?1
                 ORDER BY CASE WHEN status = ?2 THEN 1 ELSE 0 END, priority DESC, created_at DESC, id DESC
                 LIMIT {LIST_LIMIT}"
        ))?;
        let tasks = stmt
            .query_map(params![version_id, Status::Done.as_str()], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(tasks)
    }

    fn list_tasks_page(
        &self,
        filter: &TaskFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Task>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE project_id = ?");
        let mut values = vec![Value::Integer(filter.project_id)];
        if let Some(version_id) = filter.version_id {
            sql.push_str(" AND version_id = ?");
            values.push(Value::Integer(version_id));
        }
        if let Some(status) = filter.status {
            sql.push_str(" AND status = ?");
            values.push(Value::Text(status.as_str().to_string()));
        }
        if let Some(priority) = filter.priority {
            sql.push_str(" AND priority = ?");
            values.push(Value::Integer(i64::from(priority)));
        }
        sql.push_str(
            " ORDER BY CASE WHEN status = ? THEN 1 ELSE 0 END, priority DESC, created_at DESC, id DESC
              LIMIT ? OFFSET ?",
        );
        values.push(Value::Text(Status::Done.as_str().to_string()));
        values.push(Value::Integer(i64::from(limit)));
        values.push(Value::Integer(i64::from(offset)));
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params_from_iter(values), task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tasks)
    }

    fn count_tasks(&self, filter: &TaskFilter) -> Result<u64, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut sql = String::from("SELECT COUNT(*) FROM tasks WHERE project_id = ?");
        let mut values = vec![Value::Integer(filter.project_id)];
        if let Some(version_id) = filter.version_id {
            sql.push_str(" AND version_id = ?");
            values.push(Value::Integer(version_id));
        }
        if let Some(status) = filter.status {
            sql.push_str(" AND status = ?");
            values.push(Value::Text(status.as_str().to_string()));
        }
        if let Some(priority) = filter.priority {
            sql.push_str(" AND priority = ?");
            values.push(Value::Integer(i64::from(priority)));
        }
        let count: i64 = conn.query_row(&sql, params_from_iter(values), |row| row.get(0))?;
        Ok(count as u64)
    }

    fn list_ready_tasks(
        &self,
        project_id: i64,
        version_id: Option<i64>,
    ) -> Result<Vec<Task>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (from, values) = ready_filter(project_id, version_id);
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} {from} ORDER BY priority DESC, id ASC LIMIT {LIST_LIMIT}"
        ))?;
        let tasks = stmt
            .query_map(params_from_iter(values), task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(tasks)
    }

    fn claim_next_ready_task(
        &self,
        project_id: i64,
        version_id: Option<i64>,
        note: Option<&str>,
    ) -> Result<Task, AppError> {
        // Selection, status change and note share one IMMEDIATE transaction:
        // another claimant waits for the lock instead of seeing the same task.
        self.write_transaction(|tx| {
            let (from, values) = ready_filter(project_id, version_id);
            let id: Option<i64> = tx
                .query_row(
                    &format!("SELECT id {from} ORDER BY priority DESC, id ASC LIMIT 1"),
                    params_from_iter(values),
                    |row| row.get(0),
                )
                .optional()?;
            let id = id.ok_or_else(|| {
                AppError::NoReadyTask("there is no ready task to claim".to_string())
            })?;
            tx.execute(
                "UPDATE tasks SET status = ?1, completed_at = NULL WHERE id = ?2",
                params![Status::InProgress.as_str(), id],
            )?;
            if let Some(content) = note {
                let note = TaskNote::new(id, content)?;
                tx.execute(
                    "INSERT INTO task_notes(task_id, content) VALUES(?1, ?2)",
                    params![id, note.content],
                )?;
            }
            task_by_id(tx, id)
        })
    }

    fn create_task(
        &self,
        project_id: i64,
        version_id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError> {
        validation::validate_name("task", name)?;
        validation::validate_text("task", "description", description)?;
        // Validation and insert share one write lock, so the version cannot
        // change project between the check and the insert.
        let id = self.write_transaction(|tx| {
            let version_project: Option<i64> = tx
                .query_row(
                    "SELECT project_id FROM versions WHERE id = ?1",
                    [version_id],
                    |row| row.get(0),
                )
                .optional()?;
            if version_project != Some(project_id) {
                return Err(AppError::Validation(
                    "version does not belong to the project".to_string(),
                ));
            }
            tx.execute(
                "INSERT INTO tasks (project_id, version_id, name, description, status, priority)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    project_id,
                    version_id,
                    name,
                    description,
                    status.as_str(),
                    i64::from(priority)
                ],
            )?;
            Ok(tx.last_insert_rowid())
        })?;
        self.get_task(id)
    }

    fn get_task(&self, id: i64) -> Result<Task, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        task_by_id(&conn, id)
    }

    fn update_task(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError> {
        validation::validate_name("task", name)?;
        validation::validate_text("task", "description", description)?;
        self.checked_status_write(id, status, |conn, status| match status {
            Status::Done => Ok(conn.execute(
                "UPDATE tasks
                 SET name = ?1, description = ?2, status = ?3, priority = ?4,
                     completed_at = COALESCE(completed_at, datetime('now'))
                 WHERE id = ?5",
                params![name, description, status.as_str(), i64::from(priority), id],
            )?),
            _ => Ok(conn.execute(
                "UPDATE tasks
                 SET name = ?1, description = ?2, status = ?3, priority = ?4,
                     completed_at = NULL
                 WHERE id = ?5",
                params![name, description, status.as_str(), i64::from(priority), id],
            )?),
        })
    }

    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError> {
        self.checked_status_write(id, new_status, |conn, new_status| match new_status {
            Status::Done => Ok(conn.execute(
                "UPDATE tasks SET status = ?1, completed_at = COALESCE(completed_at, datetime('now')) WHERE id = ?2",
                params![new_status.as_str(), id],
            )?),
            _ => Ok(conn.execute(
                "UPDATE tasks SET status = ?1, completed_at = NULL WHERE id = ?2",
                params![new_status.as_str(), id],
            )?),
        })
    }

    fn update_task_priority(&self, id: i64, priority: Priority) -> Result<Task, AppError> {
        let updated = self.write(|conn| {
            conn.execute(
                "UPDATE tasks SET priority = ?1 WHERE id = ?2",
                params![i64::from(priority), id],
            )
            .map_err(AppError::from)
        })?;
        if updated == 0 {
            return Err(AppError::NotFound);
        }
        self.get_task(id)
    }

    fn move_task(&self, task_id: i64, version_id: i64) -> Result<Task, AppError> {
        // Edges and target are checked under the write lock, so a dependency
        // added meanwhile cannot leave a task in another version with edges.
        self.write_transaction(|tx| {
            let task = task_by_id(tx, task_id)?;
            let target = version_by_id(tx, version_id)?;
            if task.version_id == version_id {
                return Ok(task);
            }
            check_task_move(
                &task,
                &target,
                &dependencies_of(tx, task_id)?,
                &dependents_of(tx, task_id)?,
            )?;
            tx.execute(
                "UPDATE tasks SET version_id = ?1 WHERE id = ?2",
                params![version_id, task_id],
            )?;
            task_by_id(tx, task_id)
        })
    }

    fn delete_task(&self, id: i64) -> Result<(), AppError> {
        self.write(|conn| {
            conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    fn delete_done_tasks_by_version(&self, version_id: i64) -> Result<(), AppError> {
        self.write(|conn| {
            conn.execute(
                "DELETE FROM tasks WHERE version_id = ?1 AND status = ?2",
                params![version_id, Status::Done.as_str()],
            )?;
            Ok(())
        })
    }
}

impl DependencyRepository for SqliteRepository {
    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        if task_id == depends_on_id {
            return Err(AppError::DependencyCycle);
        }
        // Cycle detection and insert share the write lock: two processes adding
        // edges at once can no longer close a cycle between check and write.
        self.write_transaction(|tx| {
            let task = task_by_id(tx, task_id)?;
            let depends_on = task_by_id(tx, depends_on_id)?;
            if task.version_id != depends_on.version_id {
                return Err(AppError::Validation(
                    "dependencies must be within the same version".to_string(),
                ));
            }
            let graph = dependency_graph(tx)?;
            if would_create_cycle(&graph, task_id, depends_on_id) {
                return Err(AppError::DependencyCycle);
            }
            tx.execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
                params![task_id, depends_on_id],
            )?;
            Ok(())
        })
    }

    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        self.write(|conn| {
            conn.execute(
                "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
                params![task_id, depends_on_id],
            )?;
            Ok(())
        })
    }

    fn set_dependencies(&self, task_id: i64, depends_on_ids: &[i64]) -> Result<(), AppError> {
        validation::validate_dependency_count(depends_on_ids.len())?;
        let unique = unique_ids(depends_on_ids);
        self.write_transaction(|tx| {
            let task = task_by_id(tx, task_id)?;
            for &dep in &unique {
                let depends_on = task_by_id(tx, dep)?;
                if depends_on.version_id != task.version_id {
                    return Err(AppError::Validation(
                        "dependencies must be within the same version".to_string(),
                    ));
                }
            }
            let graph = dependency_graph(tx)?;
            check_dependency_set(&graph, task_id, &unique)?;
            tx.execute(
                "DELETE FROM task_dependencies WHERE task_id = ?1",
                params![task_id],
            )?;
            for dep in &unique {
                tx.execute(
                    "INSERT INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
                    params![task_id, dep],
                )?;
            }
            Ok(())
        })
    }

    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        dependencies_of(&conn, task_id)
    }

    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(!incomplete_dependencies_of(&conn, task_id)?.is_empty())
    }

    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        dependents_of(&conn, task_id)
    }

    fn would_create_cycle(&self, task_id: i64, candidate_id: i64) -> Result<bool, AppError> {
        if task_id == candidate_id {
            return Ok(true);
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let graph = dependency_graph(&conn)?;
        Ok(would_create_cycle(&graph, task_id, candidate_id))
    }
}

impl NoteRepository for SqliteRepository {
    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, task_id, content, created_at
             FROM task_notes WHERE task_id = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT {LIST_LIMIT}"
        ))?;
        let notes = stmt
            .query_map([task_id], task_note_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(notes)
    }

    fn add_task_note(&self, task_id: i64, content: &str) -> Result<TaskNote, AppError> {
        let note = TaskNote::new(task_id, content)?;
        self.write_transaction(|tx| {
            tx.execute(
                "INSERT INTO task_notes(task_id, content) VALUES(?1, ?2)",
                params![note.task_id, note.content],
            )?;
            let id = tx.last_insert_rowid();
            let note = tx.query_row(
                "SELECT id, task_id, content, created_at FROM task_notes WHERE id = ?1",
                [id],
                task_note_from_row,
            )?;
            Ok(note)
        })
    }

    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError> {
        let rows = self.write(|conn| {
            conn.execute("DELETE FROM task_notes WHERE id = ?1", [note_id])
                .map_err(AppError::from)
        })?;
        if rows == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
    }
}

impl SummaryRepository for SqliteRepository {
    fn list_version_summaries(&self, version_id: i64) -> Result<Vec<VersionSummary>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, version_id, summary, snapshot_json, generated_at, created_at
             FROM version_summaries WHERE version_id = ?1
             ORDER BY generated_at DESC, id DESC
             LIMIT {LIST_LIMIT}"
        ))?;
        let summaries = stmt
            .query_map([version_id], version_summary_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(summaries)
    }

    fn get_version_summary(&self, id: i64) -> Result<VersionSummary, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, version_id, summary, snapshot_json, generated_at, created_at
             FROM version_summaries WHERE id = ?1",
            [id],
            version_summary_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            _ => e.into(),
        })
    }

    fn create_version_summary(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError> {
        validation::validate_text("version summary", "summary", summary)?;
        self.get_version(version_id)?;
        let id = self.write(|conn| {
            conn.execute(
                "INSERT INTO version_summaries (version_id, summary, snapshot_json)
                 VALUES (?1, ?2, ?3)",
                params![version_id, summary, snapshot_json],
            )
            .map_err(version_summary_write_error)?;
            Ok(conn.last_insert_rowid())
        })?;
        self.get_version_summary(id)
    }

    fn delete_version_summary(&self, id: i64) -> Result<(), AppError> {
        let rows = self.write(|conn| {
            conn.execute("DELETE FROM version_summaries WHERE id = ?1", [id])
                .map_err(AppError::from)
        })?;
        if rows == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

    fn compact_version(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError> {
        validation::validate_text("version summary", "summary", summary)?;
        self.get_version(version_id)?;
        // Single transaction: dropping it without commit (on any error or a
        // retried busy lock) rolls back every write, so done tasks are never
        // deleted without a snapshot.
        let id = self.write_transaction(|tx| {
            tx.execute(
                "DELETE FROM version_summaries WHERE version_id = ?1",
                [version_id],
            )?;
            tx.execute(
                "INSERT INTO version_summaries (version_id, summary, snapshot_json)
                 VALUES (?1, ?2, ?3)",
                params![version_id, summary, snapshot_json],
            )
            .map_err(version_summary_write_error)?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "DELETE FROM tasks WHERE version_id = ?1 AND status = ?2",
                params![version_id, Status::Done.as_str()],
            )?;
            Ok(id)
        })?;
        self.get_version_summary(id)
    }
}

fn would_create_cycle(graph: &HashMap<i64, Vec<i64>>, task_id: i64, depends_on_id: i64) -> bool {
    let mut visited = HashSet::new();
    let mut stack = vec![depends_on_id];
    while let Some(current) = stack.pop() {
        if current == task_id {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(next) = graph.get(&current) {
            stack.extend(next);
        }
    }
    false
}

fn project_from_row(row: &Row) -> Result<Project, rusqlite::Error> {
    Ok(Project {
        id: Some(row.get(0)?),
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
        working_dir: row.get(4)?,
    })
}

fn task_from_row(row: &Row) -> Result<Task, rusqlite::Error> {
    Ok(Task {
        id: Some(row.get("id")?),
        project_id: row.get("project_id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        status: row.get("status")?,
        priority: row.get("priority")?,
        created_at: row.get("created_at")?,
        version_id: row.get("version_id")?,
        completed_at: row.get("completed_at")?,
    })
}

fn version_from_row(row: &Row) -> Result<Version, rusqlite::Error> {
    Ok(Version {
        id: Some(row.get("id")?),
        project_id: row.get("project_id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        status: row.get("status")?,
        created_at: row.get("created_at")?,
    })
}

/// Statuses and priorities are stored as their `as_str` / integer codes. These
/// impls decode them, so every query reads them through the same mapping.
impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Status::try_from(value.as_str()?).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

impl FromSql for VersionStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        VersionStatus::try_from(value.as_str()?).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

impl FromSql for Priority {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Priority::try_from(value.as_i64()?).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

/// Maps a UNIQUE(project_id, name) violation to a validation error.
fn version_write_error(err: rusqlite::Error, name: &str) -> AppError {
    match err {
        rusqlite::Error::SqliteFailure(ref failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::DuplicateName(format!("version already exists: {name}"))
        }
        other => other.into(),
    }
}

fn task_note_from_row(row: &Row) -> Result<TaskNote, rusqlite::Error> {
    Ok(TaskNote {
        id: Some(row.get(0)?),
        task_id: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn version_summary_from_row(row: &Row) -> Result<VersionSummary, rusqlite::Error> {
    Ok(VersionSummary {
        id: Some(row.get(0)?),
        version_id: row.get(1)?,
        summary: row.get(2)?,
        snapshot_json: row.get(3)?,
        generated_at: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// Maps a UNIQUE(version_id) violation to a validation error.
fn version_summary_write_error(err: rusqlite::Error) -> AppError {
    match err {
        rusqlite::Error::SqliteFailure(ref failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::DuplicateName("version already has a summary".to_string())
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use crate::db::busy::BusyOptions;
    use crate::db::repository::*;
    use crate::domain::task::{Priority, Status};

    use super::*;

    fn setup() -> (tempfile::TempDir, SqliteRepository) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let repo = SqliteRepository::open(&path).unwrap();
        (dir, repo)
    }

    #[test]
    fn compact_version_rolls_back_when_summary_insert_fails() {
        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let done = repo
            .create_task(pid, v, "Done", "", Status::Done, Priority::Low)
            .unwrap()
            .id
            .unwrap();
        repo.create_version_summary(v, "Old", "{}").unwrap();
        // Force the second write of the transaction to fail.
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute_batch(
                "CREATE TRIGGER fail_summary_insert BEFORE INSERT ON version_summaries
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
            )
            .unwrap();

        assert!(repo.compact_version(v, "New", "{}").is_err());

        let summaries = repo.list_version_summaries(v).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].summary, "Old");
        assert!(
            repo.get_task(done).is_ok(),
            "done task must survive the failure"
        );
    }

    #[test]
    fn open_creates_db_with_wal_and_foreign_keys() {
        let (_dir, repo) = setup();
        let journal_mode: String = repo
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");

        let foreign_keys: i32 = repo
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn open_applies_the_configured_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let options = BusyOptions {
            busy_timeout: Duration::from_millis(1_234),
            ..BusyOptions::default()
        };
        let repo = SqliteRepository::open_with(&path, options).unwrap();
        assert_eq!(busy_timeout_of(&repo), 1_234);

        let repo = SqliteRepository::open_with(&path, BusyOptions::default()).unwrap();
        assert_eq!(
            busy_timeout_of(&repo),
            BusyOptions::default().busy_timeout.as_millis() as i64
        );
    }

    fn busy_timeout_of(repo: &SqliteRepository) -> i64 {
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row("PRAGMA busy_timeout;", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn write_transaction_holds_the_write_lock_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let repo = SqliteRepository::open_with(&path, BusyOptions::default()).unwrap();
        let probe = Connection::open(&path).unwrap();
        probe.busy_timeout(Duration::from_millis(50)).unwrap();

        let probe_was_locked = Cell::new(false);
        let id = repo
            .write_transaction(|tx| {
                tx.execute("INSERT INTO projects (name) VALUES ('inside')", [])?;
                // A deferred transaction would let the probe take the lock here.
                match probe.execute_batch("BEGIN IMMEDIATE") {
                    Ok(()) => probe.execute_batch("ROLLBACK").unwrap(),
                    Err(error) => probe_was_locked.set(AppError::is_lock(&error)),
                }
                Ok(tx.last_insert_rowid())
            })
            .unwrap();

        assert!(
            probe_was_locked.get(),
            "BEGIN IMMEDIATE must hold the write lock up front"
        );
        assert_eq!(repo.get_project(id).unwrap().name, "inside");
    }

    #[test]
    fn create_project_rolls_back_when_the_default_version_fails() {
        let (_dir, repo) = setup();
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute_batch(
                "CREATE TRIGGER fail_version_insert BEFORE INSERT ON versions
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
            )
            .unwrap();

        assert!(repo.create_project("P", "", None).is_err());
        assert!(
            repo.list_projects().unwrap().is_empty(),
            "the project must not survive without its default version"
        );
    }

    #[test]
    fn set_dependencies_rolls_back_as_a_whole() {
        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
        let version = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let task = |name: &str| {
            repo.create_task(pid, version, name, "", Status::Todo, Priority::Medium)
                .unwrap()
                .id
                .unwrap()
        };
        let (a, b, c) = (task("a"), task("b"), task("c"));
        repo.set_dependencies(a, &[b]).unwrap();
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute_batch(
                "CREATE TRIGGER fail_dependency_insert BEFORE INSERT ON task_dependencies
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
            )
            .unwrap();

        assert!(repo.set_dependencies(a, &[c]).is_err());
        assert_eq!(
            repo.get_dependencies(a).unwrap(),
            vec![b],
            "the previous edge set must survive the failed transaction"
        );
    }

    #[test]
    fn migrations_create_projects_table() {
        let (_dir, repo) = setup();
        let count: i64 = repo
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migrations_add_a_nullable_project_working_dir() {
        let (_dir, repo) = setup();
        let conn = repo.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (count, notnull): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(\"notnull\"), 0) \
                 FROM pragma_table_info('projects') WHERE name = 'working_dir';",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "the column exists");
        assert_eq!(notnull, 0, "legacy projects keep it NULL");

        conn.execute("INSERT INTO projects (name) VALUES ('Legacy')", [])
            .unwrap();
        let legacy_id = conn.last_insert_rowid();
        drop(conn);

        let legacy = repo.get_project(legacy_id).unwrap();
        assert_eq!(legacy.working_dir, None);
    }

    #[test]
    fn project_working_dir_round_trips_and_updates() {
        let (_dir, repo) = setup();
        let created = repo
            .create_project("P", "", Some("/srv/repos/alpha"))
            .unwrap();
        assert_eq!(created.working_dir.as_deref(), Some("/srv/repos/alpha"));
        let project_id = created.id.unwrap();

        let listed = repo.list_projects().unwrap();
        assert_eq!(listed[0].working_dir.as_deref(), Some("/srv/repos/alpha"));

        let updated = repo
            .update_project(project_id, "P", "", Some("/srv/repos/beta"))
            .unwrap();
        assert_eq!(updated.working_dir.as_deref(), Some("/srv/repos/beta"));

        let cleared = repo.update_project(project_id, "P", "", None).unwrap();
        assert_eq!(cleared.working_dir, None);
    }

    #[test]
    fn migrations_create_versions_and_require_task_version() {
        let (_dir, repo) = setup();
        let conn = repo.conn.lock().unwrap_or_else(|e| e.into_inner());
        let versions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='versions';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(versions, 1);
        let notnull: i64 = conn
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('tasks') WHERE name='version_id';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(notnull, 1);
    }

    #[test]
    fn task_carries_version_id_from_database() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let version_id: i64 = repo
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT id FROM versions WHERE project_id = ?1 AND name = 'v0'",
                [project_id],
                |row| row.get(0),
            )
            .unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let fetched = repo.get_task(task.id.unwrap()).unwrap();
        assert_eq!(fetched.version_id, version_id);
        let listed = repo.list_tasks_by_project(project_id).unwrap();
        assert_eq!(listed[0].version_id, version_id);
    }

    #[test]
    fn migrations_create_task_notes_table() {
        let (_dir, repo) = setup();
        let count: i64 = repo
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_notes';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn project_crud() {
        let (_dir, repo) = setup();
        assert!(repo.list_projects().unwrap().is_empty());

        let created = repo.create_project("Alpha", "First project", None).unwrap();
        assert_eq!(created.name, "Alpha");
        assert!(created.id.is_some());

        let projects = repo.list_projects().unwrap();
        assert_eq!(projects.len(), 1);

        let fetched = repo.get_project(created.id.unwrap()).unwrap();
        assert_eq!(fetched.name, "Alpha");
        assert_eq!(fetched.description, "First project");
    }

    #[test]
    fn task_crud() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();

        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "desc",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        assert_eq!(task.name, "T1");
        assert!(task.id.is_some());

        let tasks = repo.list_tasks_by_project(project_id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Todo);
        assert_eq!(tasks[0].priority, Priority::Low);

        let updated = repo
            .update_task(
                task.id.unwrap(),
                "T1-renamed",
                "new desc",
                Status::InProgress,
                Priority::High,
            )
            .unwrap();
        assert_eq!(updated.name, "T1-renamed");
        assert_eq!(updated.status, Status::InProgress);
        assert_eq!(updated.priority, Priority::High);

        let status_updated = repo
            .update_task_status(task.id.unwrap(), Status::Done)
            .unwrap();
        assert_eq!(status_updated.status, Status::Done);

        let fetched = repo.get_task(task.id.unwrap()).unwrap();
        assert_eq!(fetched.status, Status::Done);
    }

    #[test]
    fn update_task_status_sets_and_clears_completed_at() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let id = task.id.unwrap();
        assert_eq!(task.completed_at, None);

        let done = repo.update_task_status(id, Status::Done).unwrap();
        assert_eq!(done.status, Status::Done);
        assert!(done.completed_at.is_some());

        let reopened = repo.update_task_status(id, Status::Todo).unwrap();
        assert_eq!(reopened.status, Status::Todo);
        assert_eq!(reopened.completed_at, None);
    }

    #[test]
    fn add_dependency_is_idempotent() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let a = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(a_id, b_id).unwrap();
        repo.add_dependency(a_id, b_id).unwrap();
        let deps = repo.get_dependencies(a_id).unwrap();
        assert_eq!(deps, vec![b_id]);
    }

    #[test]
    fn add_dependency_rejects_self_loop() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let a = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let result = repo.add_dependency(a.id.unwrap(), a.id.unwrap());
        assert!(matches!(result, Err(AppError::DependencyCycle)));
    }

    #[test]
    fn add_dependency_rejects_cycles_via_dfs() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let a = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let c = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "C",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        let c_id = c.id.unwrap();
        repo.add_dependency(a_id, b_id).unwrap();
        repo.add_dependency(b_id, c_id).unwrap();
        let result = repo.add_dependency(c_id, a_id);
        assert!(matches!(result, Err(AppError::DependencyCycle)));
    }

    #[test]
    fn has_incomplete_dependencies_is_true_for_unfinished_dep() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let a = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "A",
                "",
                Status::Done,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project.id.unwrap(),
                repo.default_version_id(project.id.unwrap()).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(b_id, a_id).unwrap();
        assert!(!repo.has_incomplete_dependencies(b_id).unwrap());
        repo.update_task_status(a_id, Status::Todo).unwrap();
        assert!(repo.has_incomplete_dependencies(b_id).unwrap());
    }

    #[test]
    fn delete_task_removes_task() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        repo.delete_task(task_id).unwrap();

        assert!(matches!(repo.get_task(task_id), Err(AppError::NotFound)));
    }

    #[test]
    fn delete_task_cascades_dependency_rows() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(b_id, a_id).unwrap();

        repo.delete_task(a_id).unwrap();

        assert!(repo.get_dependencies(b_id).unwrap().is_empty());
    }

    #[test]
    fn get_dependents_returns_reverse_lookup() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let a = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let b = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let c = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "C",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        let c_id = c.id.unwrap();
        repo.add_dependency(b_id, a_id).unwrap();
        repo.add_dependency(c_id, a_id).unwrap();

        let dependents = repo.get_dependents(a_id).unwrap();
        assert_eq!(dependents, vec![b_id, c_id]);
    }

    #[test]
    fn list_tasks_by_project_orders_priority_desc_created_desc() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let low = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "Low",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let high_old = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "HighOld",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let high_new = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "HighNew",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();

        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "UPDATE tasks SET created_at = ?1 WHERE id = ?2",
                params!["2026-01-01 00:00:00", high_old.id.unwrap()],
            )
            .unwrap();
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "UPDATE tasks SET created_at = ?1 WHERE id = ?2",
                params!["2026-01-02 00:00:00", high_new.id.unwrap()],
            )
            .unwrap();

        let tasks = repo.list_tasks_by_project(project_id).unwrap();
        assert_eq!(tasks[0].id, high_new.id);
        assert_eq!(tasks[1].id, high_old.id);
        assert_eq!(tasks[2].id, low.id);
    }

    #[test]
    fn task_notes_crud() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        let note = repo.add_task_note(task_id, "first note").unwrap();
        assert_eq!(note.task_id, task_id);
        assert_eq!(note.content, "first note");
        assert!(note.id.is_some());

        let notes = repo.list_task_notes(task_id).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "first note");

        repo.delete_task_note(note.id.unwrap()).unwrap();
        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }

    #[test]
    fn delete_task_cascades_notes() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "", None).unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Low,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        repo.add_task_note(task_id, "note to cascade").unwrap();
        repo.delete_task(task_id).unwrap();

        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }

    /// Every status must be accepted by the `tasks.status` CHECK and read back
    /// unchanged. `status_slot` is an exhaustive match: a new variant does not
    /// compile until it is given a slot, and `ALL_STATUSES` must cover every slot.
    #[test]
    fn every_task_status_is_accepted_by_the_schema() {
        const ALL_STATUSES: [Status; 3] = [Status::Todo, Status::InProgress, Status::Done];
        fn status_slot(status: Status) -> usize {
            match status {
                Status::Todo => 0,
                Status::InProgress => 1,
                Status::Done => 2,
            }
        }
        for status in ALL_STATUSES {
            assert_eq!(ALL_STATUSES[status_slot(status)], status);
        }
        assert_eq!(ALL_STATUSES.len(), status_slot(Status::Done) + 1);

        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
        let vid = repo.default_version_id(pid).unwrap();
        for status in ALL_STATUSES {
            let task = repo
                .create_task(pid, vid, "T", "", status, Priority::Low)
                .unwrap();
            assert_eq!(repo.get_task(task.id.unwrap()).unwrap().status, status);
        }
    }

    #[test]
    fn every_version_status_is_accepted_by_the_schema() {
        const ALL_STATUSES: [VersionStatus; 3] = [
            VersionStatus::Backlog,
            VersionStatus::Open,
            VersionStatus::Released,
        ];
        fn status_slot(status: VersionStatus) -> usize {
            match status {
                VersionStatus::Backlog => 0,
                VersionStatus::Open => 1,
                VersionStatus::Released => 2,
            }
        }
        for status in ALL_STATUSES {
            assert_eq!(ALL_STATUSES[status_slot(status)], status);
        }
        assert_eq!(ALL_STATUSES.len(), status_slot(VersionStatus::Released) + 1);

        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
        for (i, status) in ALL_STATUSES.into_iter().enumerate() {
            let name = format!("v{i}.0.0");
            let version = repo.create_version(pid, &name, "").unwrap();
            let id = version.id.unwrap();
            let updated = repo.update_version(id, &name, "", status).unwrap();
            assert_eq!(updated.status, status);
            assert_eq!(repo.get_version(id).unwrap().status, status);
        }
    }

    #[test]
    fn migrations_default_new_versions_to_backlog() {
        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
        // The TUI/MCP set the status explicitly, so the schema default covers
        // rows inserted by any other path.
        repo.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "INSERT INTO versions (project_id, name) VALUES (?1, 'v9.9.9')",
                [pid],
            )
            .unwrap();
        let versions = repo.list_versions_by_project(pid).unwrap();
        let legacy = versions.iter().find(|v| v.name == "v9.9.9").unwrap();
        assert_eq!(legacy.status, VersionStatus::Backlog);
    }

    #[test]
    fn v6_migration_preserves_existing_versions_tasks_and_summaries()
    -> Result<(), Box<dyn std::error::Error>> {
        use refinery::Target;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        // Stop at V5, the schema versions had before Backlog existed. Its
        // versions CHECK only accepted open/released.
        embedded::migrations::runner()
            .set_target(Target::Version(5))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO projects (id, name) VALUES (1, 'P');
             INSERT INTO versions (id, project_id, name, status) VALUES
                 (1, 1, 'v0', 'open'),
                 (2, 1, 'v1.0.0', 'released');
             INSERT INTO version_summaries (id, version_id, summary, snapshot_json)
                 VALUES (1, 2, 'frozen', '{}');
             INSERT INTO tasks (id, project_id, version_id, name, status, priority, completed_at)
                 VALUES (1, 1, 2, 'Done', 'done', 2, '2026-01-01 00:00:00');
             INSERT INTO task_notes (id, task_id, content) VALUES (1, 1, 'note');
             INSERT INTO tasks (id, project_id, version_id, name, status, priority)
                 VALUES (2, 1, 2, 'Blocked', 'todo', 1);
             INSERT INTO task_dependencies (task_id, depends_on_id) VALUES (2, 1);",
        )
        .unwrap();
        drop(conn);

        let repo = SqliteRepository::open(&path).unwrap();
        let versions = repo.list_versions_by_project(1).unwrap();
        let by_name = |name: &str| versions.iter().find(|v| v.name == name).unwrap();
        assert_eq!(by_name("v0").status, VersionStatus::Open);
        assert_eq!(by_name("v1.0.0").status, VersionStatus::Released);

        let done = repo.get_task(1).unwrap();
        assert_eq!(done.status, Status::Done);
        assert_eq!(done.completed_at.as_deref(), Some("2026-01-01 00:00:00"));
        assert_eq!(repo.get_dependencies(2).unwrap(), vec![1]);
        assert_eq!(repo.list_task_notes(1).unwrap()[0].content, "note");
        assert_eq!(repo.list_version_summaries(2).unwrap()[0].summary, "frozen");
        // New versions are planned in Backlog after the migration.
        let created = repo.create_version(1, "v2.0.0", "").unwrap();
        assert_eq!(created.status, VersionStatus::Backlog);
        Ok(())
    }
}
