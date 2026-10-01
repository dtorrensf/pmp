// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, Row, params};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("src/db/migrations");
}

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
        let conn = self.conn.lock().unwrap();
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

impl Repository for SqliteRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, name, description, created_at FROM projects ORDER BY id")?;
        let projects = stmt
            .query_map([], project_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(projects)
    }

    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (name, description) VALUES (?1, ?2)",
            [name, description],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_project(id)
    }

    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE projects SET name = ?1, description = ?2 WHERE id = ?3",
            params![name, description, id],
        )?;
        drop(conn);
        self.get_project(id)
    }

    fn get_project(&self, id: i64) -> Result<Project, AppError> {
        let conn = self.conn.lock().unwrap();
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

    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, description, status, priority, created_at
             FROM tasks WHERE project_id = ?1
             ORDER BY CASE WHEN status = 'done' THEN 1 ELSE 0 END, priority DESC, created_at DESC",
        )?;
        let tasks = stmt
            .query_map([project_id], task_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tasks)
    }

    fn create_task(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (project_id, name, description, status, priority)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project_id,
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
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, project_id, name, description, status, priority, created_at
             FROM tasks WHERE id = ?1",
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET status = ?1 WHERE id = ?2",
            params![new_status.as_str(), id],
        )?;
        drop(conn);
        self.get_task(id)
    }

    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        if task_id == depends_on_id {
            return Err(AppError::DependencyCycle);
        }
        let graph = self.load_dependency_graph()?;
        if would_create_cycle(&graph, task_id, depends_on_id) {
            return Err(AppError::DependencyCycle);
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?1, ?2)",
            params![task_id, depends_on_id],
        )?;
        Ok(())
    }

    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_id = ?2",
            params![task_id, depends_on_id],
        )?;
        Ok(())
    }

    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?1 ORDER BY depends_on_id",
        )?;
        let deps = stmt
            .query_map([task_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(deps)
    }

    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies d
             JOIN tasks t ON d.depends_on_id = t.id
             WHERE d.task_id = ?1 AND t.status != 'done'",
            [task_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn delete_project(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    fn delete_task(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let conn = self.conn.lock().unwrap();
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

    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, content, created_at
             FROM task_notes WHERE task_id = ?1
             ORDER BY created_at DESC, id DESC",
        )?;
        let notes = stmt
            .query_map([task_id], task_note_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(notes)
    }

    fn add_task_note(&self, task_id: i64, content: &str) -> Result<TaskNote, AppError> {
        let note = TaskNote::new(task_id, content)?;
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM task_notes WHERE id = ?1", [note_id])?;
        if rows == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
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
    let status: String = row.get(4)?;
    let priority: i64 = row.get(5)?;
    Ok(Task {
        id: Some(row.get(0)?),
        project_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        status: Status::try_from_str(&status).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        priority: Priority::try_from(priority).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?,
        created_at: row.get(6)?,
    })
}

fn task_note_from_row(row: &Row) -> Result<TaskNote, rusqlite::Error> {
    Ok(TaskNote {
        id: Some(row.get(0)?),
        task_id: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::domain::task::{Priority, Status};

    use super::*;

    fn setup() -> (tempfile::TempDir, SqliteRepository) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let repo = SqliteRepository::open(&path).unwrap();
        (dir, repo)
    }

    #[test]
    fn open_creates_db_with_wal_and_foreign_keys() {
        let (_dir, repo) = setup();
        let journal_mode: String = repo
            .conn
            .lock()
            .unwrap()
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");

        let foreign_keys: i32 = repo
            .conn
            .lock()
            .unwrap()
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
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migrations_create_task_notes_table() {
        let (_dir, repo) = setup();
        let count: i64 = repo
            .conn
            .lock()
            .unwrap()
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
            .create_task(project_id, "T1", "desc", Status::Todo, Priority::Low)
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
    fn add_dependency_is_idempotent() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "").unwrap();
        let a = repo
            .create_task(project.id.unwrap(), "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project.id.unwrap(), "B", "", Status::Todo, Priority::Low)
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
            .create_task(project.id.unwrap(), "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let result = repo.add_dependency(a.id.unwrap(), a.id.unwrap());
        assert!(matches!(result, Err(AppError::DependencyCycle)));
    }

    #[test]
    fn add_dependency_rejects_cycles_via_dfs() {
        let (_dir, repo) = setup();
        let project = repo.create_project("P", "").unwrap();
        let a = repo
            .create_task(project.id.unwrap(), "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project.id.unwrap(), "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let c = repo
            .create_task(project.id.unwrap(), "C", "", Status::Todo, Priority::Low)
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
            .create_task(project.id.unwrap(), "A", "", Status::Done, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project.id.unwrap(), "B", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let c = repo
            .create_task(project_id, "C", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "Low", "", Status::Todo, Priority::Low)
            .unwrap();
        let high_old = repo
            .create_task(project_id, "HighOld", "", Status::Todo, Priority::High)
            .unwrap();
        let high_new = repo
            .create_task(project_id, "HighNew", "", Status::Todo, Priority::High)
            .unwrap();

        repo.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE tasks SET created_at = ?1 WHERE id = ?2",
                params!["2026-01-01 00:00:00", high_old.id.unwrap()],
            )
            .unwrap();
        repo.conn
            .lock()
            .unwrap()
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
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
            .create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        let task_id = task.id.unwrap();

        repo.add_task_note(task_id, "note to cascade").unwrap();
        repo.delete_task(task_id).unwrap();

        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }
}
