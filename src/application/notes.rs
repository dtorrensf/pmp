// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task note use cases.

use serde::{Deserialize, Serialize};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::TaskNote;
use crate::domain::validation::validate_text;

use super::pagination::{PageMeta, clamp_limit, window};

pub fn list(repo: &dyn Repository, task_id: i64) -> Result<Vec<TaskNote>, AppError> {
    repo.get_task(task_id)?;
    repo.list_task_notes(task_id)
}

/// One page of a task's notes, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotePage {
    pub notes: Vec<TaskNote>,
    #[serde(flatten)]
    pub meta: PageMeta,
}

pub fn page(
    repo: &dyn Repository,
    task_id: i64,
    limit: Option<u32>,
    offset: u32,
) -> Result<NotePage, AppError> {
    let notes = list(repo, task_id)?;
    let (notes, meta) = window(notes, clamp_limit(limit), offset);
    Ok(NotePage { notes, meta })
}

pub fn add(repo: &dyn Repository, task_id: i64, content: &str) -> Result<TaskNote, AppError> {
    repo.get_task(task_id)?;
    validate_text("note", "content", content)?;
    repo.add_task_note(task_id, content)
}

pub fn delete(repo: &dyn Repository, note_id: i64) -> Result<(), AppError> {
    repo.delete_task_note(note_id)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{ProjectRepository, RepositoryPolicy, TaskRepository};
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    use super::{add, delete, list};

    fn repo_with_task() -> (StubRepository, i64) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let task_id = repo
            .create_task(
                project_id,
                version_id,
                "A",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap()
            .id
            .unwrap();
        (repo, task_id)
    }

    #[test]
    fn add_stores_the_note() {
        let (repo, task_id) = repo_with_task();
        add(&repo, task_id, "progress").unwrap();
        assert_eq!(list(&repo, task_id).unwrap().len(), 1);
    }

    #[test]
    fn add_requires_an_existing_task() {
        let (repo, _) = repo_with_task();
        assert!(add(&repo, 999, "progress").is_err());
    }

    #[test]
    fn blank_content_is_rejected() {
        let (repo, task_id) = repo_with_task();
        assert!(add(&repo, task_id, "   ").is_err());
        assert!(list(&repo, task_id).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_the_note() {
        let (repo, task_id) = repo_with_task();
        let note = add(&repo, task_id, "progress").unwrap();
        delete(&repo, note.id.unwrap()).unwrap();
        assert!(list(&repo, task_id).unwrap().is_empty());
    }
}
