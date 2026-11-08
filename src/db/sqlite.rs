// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, Value, ValueRef};
use rusqlite::{Connection, Row, params, params_from_iter};

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
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )?;
        embedded::migrations::runner()
            .run(&mut conn)
            .map_err(|e| AppError::Validation(format!("migration error: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn load_dependency_graph(&self) -> Result<HashMap<i64, Vec<i64>>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
}

impl ProjectRepository for SqliteRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description, created_at FROM projects ORDER BY id LIMIT {LIST_LIMIT}"
        ))?;
        let projects = stmt
            .query_map([], project_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(projects)
    }

    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO projects (name, description) VALUES (?1, ?2)",
            [name, description],
        )?;
        let id = conn.last_insert_rowid();
        // Every project starts with the default version, as the V3 backfill does.
        conn.execute(
            "INSERT INTO versions (project_id, name) VALUES (?1, ?2)",
            params![id, version::DEFAULT_VERSION_NAME],
        )?;
        drop(conn);
        self.get_project(id)
    }

    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE projects SET name = ?1, description = ?2 WHERE id = ?3",
            params![name, description, id],
        )?;
        drop(conn);
        self.get_project(id)
    }

    fn get_project(&self, id: i64) -> Result<Project, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, name, description, created_at FROM projects WHERE id = ?1",
            [id],
            project_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            _ => e.into(),
        })
    }

    fn delete_project(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }
}

impl VersionRepository for SqliteRepository {
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM versions WHERE project_id = ?1
                 ORDER BY CASE WHEN status = ?2 THEN 1 ELSE 0 END, id DESC
                 LIMIT {LIST_LIMIT}"
        ))?;
        let versions = stmt
            .query_map(
                params![project_id, VersionStatus::Released.as_str()],
                version_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        check_list_size(versions)
    }

    fn create_version(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        self.get_project(project_id)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO versions (project_id, name, description) VALUES (?1, ?2, ?3)",
            params![project_id, name, description],
        )
        .map_err(|e| version_write_error(e, name))?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_version(id)
    }

    fn get_version(&self, id: i64) -> Result<Version, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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

    fn update_version(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: VersionStatus,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let updated = conn
            .execute(
                "UPDATE versions SET name = ?1, description = ?2, status = ?3 WHERE id = ?4",
                params![name, description, status.as_str(), id],
            )
            .map_err(|e| version_write_error(e, name))?;
        drop(conn);
        if updated == 0 {
            return Err(AppError::NotFound);
        }
        self.get_version(id)
    }

    fn delete_version(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM versions WHERE id = ?1", [id])?;
        Ok(())
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
        let version_project: Option<i64> = self
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT project_id FROM versions WHERE id = ?1",
                [version_id],
                |row| row.get(0),
            )
            .ok();
        if version_project != Some(project_id) {
            return Err(AppError::Validation(
                "version does not belong to the project".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
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
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_task(id)
    }

    fn get_task(&self, id: i64) -> Result<Task, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let current = self.get_task(id)?;
        check_done_transition(current.status, status, || {
            self.has_incomplete_dependencies(id)
        })?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE tasks
             SET name = ?1, description = ?2, status = ?3, priority = ?4
             WHERE id = ?5",
            params![name, description, status.as_str(), i64::from(priority), id],
        )?;
        drop(conn);
        self.get_task(id)
    }

    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError> {
        let current = self.get_task(id)?;
        check_done_transition(current.status, new_status, || {
            self.has_incomplete_dependencies(id)
        })?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        match new_status {
            Status::Done => {
                conn.execute(
                    "UPDATE tasks SET status = ?1, completed_at = COALESCE(completed_at, datetime('now')) WHERE id = ?2",
                    params![new_status.as_str(), id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE tasks SET status = ?1, completed_at = NULL WHERE id = ?2",
                    params![new_status.as_str(), id],
                )?;
            }
        }
        drop(conn);
        self.get_task(id)
    }

    fn update_task_priority(&self, id: i64, priority: Priority) -> Result<Task, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let updated = conn.execute(
            "UPDATE tasks SET priority = ?1 WHERE id = ?2",
            params![i64::from(priority), id],
        )?;
        drop(conn);
        if updated == 0 {
            return Err(AppError::NotFound);
        }
        self.get_task(id)
    }

    fn move_task(&self, task_id: i64, version_id: i64) -> Result<Task, AppError> {
        let task = self.get_task(task_id)?;
        let target = self.get_version(version_id)?;
        if task.version_id == version_id {
            return Ok(task);
        }
        check_task_move(
            &task,
            &target,
            &self.get_dependencies(task_id)?,
            &self.get_dependents(task_id)?,
        )?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE tasks SET version_id = ?1 WHERE id = ?2",
            params![version_id, task_id],
        )?;
        drop(conn);
        self.get_task(task_id)
    }

    fn delete_task(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    fn delete_done_tasks_by_version(&self, version_id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM tasks WHERE version_id = ?1 AND status = ?2",
            params![version_id, Status::Done.as_str()],
        )?;
        Ok(())
    }
}

impl DependencyRepository for SqliteRepository {
    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        if task_id == depends_on_id {
            return Err(AppError::DependencyCycle);
        }
        let task = self.get_task(task_id)?;
        let depends_on = self.get_task(depends_on_id)?;
        if task.version_id != depends_on.version_id {
            return Err(AppError::Validation(
                "dependencies must be within the same version".to_string(),
            ));
        }
        let graph = self.load_dependency_graph()?;
        if would_create_cycle(&graph, task_id, depends_on_id) {
            return Err(AppError::DependencyCycle);
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
            params![task_id, depends_on_id],
        )?;
        Ok(())
    }

    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
            params![task_id, depends_on_id],
        )?;
        Ok(())
    }

    fn set_dependencies(&self, task_id: i64, depends_on_ids: &[i64]) -> Result<(), AppError> {
        validation::validate_dependency_count(depends_on_ids.len())?;
        let task = self.get_task(task_id)?;
        let unique = unique_ids(depends_on_ids);
        for &dep in &unique {
            let depends_on = self.get_task(dep)?;
            if depends_on.version_id != task.version_id {
                return Err(AppError::Validation(
                    "dependencies must be within the same version".to_string(),
                ));
            }
        }
        let graph = self.load_dependency_graph()?;
        check_dependency_set(&graph, task_id, &unique)?;

        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
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
        tx.commit()?;
        Ok(())
    }

    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1 ORDER BY depends_on_id",
        )?;
        let deps = stmt
            .query_map([task_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(deps)
    }

    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies d
             JOIN tasks t ON d.depends_on_id = t.id
             WHERE d.task_id = ?1 AND t.status != ?2",
            params![task_id, Status::Done.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT task_id FROM task_dependencies WHERE depends_on_id = ?1 ORDER BY task_id",
        )?;
        let dependents = stmt
            .query_map([task_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(dependents)
    }

    fn would_create_cycle(&self, task_id: i64, candidate_id: i64) -> Result<bool, AppError> {
        if task_id == candidate_id {
            return Ok(true);
        }
        let graph = self.load_dependency_graph()?;
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO task_notes(task_id, content) VALUES(?1, ?2)",
            params![note.task_id, note.content],
        )?;
        let id = conn.last_insert_rowid();
        let note = conn.query_row(
            "SELECT id, task_id, content, created_at FROM task_notes WHERE id = ?1",
            [id],
            task_note_from_row,
        )?;
        Ok(note)
    }

    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let rows = conn.execute("DELETE FROM task_notes WHERE id = ?1", [note_id])?;
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO version_summaries (version_id, summary, snapshot_json)
             VALUES (?1, ?2, ?3)",
            params![version_id, summary, snapshot_json],
        )
        .map_err(version_summary_write_error)?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_version_summary(id)
    }

    fn delete_version_summary(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let rows = conn.execute("DELETE FROM version_summaries WHERE id = ?1", [id])?;
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
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Single transaction: dropping it without commit (on any error) rolls
        // back every write, so done tasks are never deleted without a snapshot.
        let tx = conn.transaction()?;
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
        tx.commit()?;
        drop(conn);
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
            AppError::Validation(format!("version already exists: {name}"))
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
            AppError::Validation("version already has a summary".to_string())
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
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
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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

        let created = repo.create_project("Alpha", "First project").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let project = repo.create_project("P", "").unwrap();
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
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
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
        const ALL_STATUSES: [VersionStatus; 2] = [VersionStatus::Open, VersionStatus::Released];
        fn status_slot(status: VersionStatus) -> usize {
            match status {
                VersionStatus::Open => 0,
                VersionStatus::Released => 1,
            }
        }
        for status in ALL_STATUSES {
            assert_eq!(ALL_STATUSES[status_slot(status)], status);
        }
        assert_eq!(ALL_STATUSES.len(), status_slot(VersionStatus::Released) + 1);

        let (_dir, repo) = setup();
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        for (i, status) in ALL_STATUSES.into_iter().enumerate() {
            let name = format!("v{i}.0.0");
            let version = repo.create_version(pid, &name, "").unwrap();
            let id = version.id.unwrap();
            let updated = repo.update_version(id, &name, "", status).unwrap();
            assert_eq!(updated.status, status);
            assert_eq!(repo.get_version(id).unwrap().status, status);
        }
    }
}
