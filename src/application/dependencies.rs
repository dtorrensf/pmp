// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task dependency use cases.
//!
//! Every operation that receives two ids verifies both tasks exist first, so a
//! caller gets "not found" instead of a foreign-key failure from the store.

use serde::{Deserialize, Serialize};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status};

/// Dependencies of a task (its blockers).
pub fn list(repo: &dyn Repository, task_id: i64) -> Result<Vec<i64>, AppError> {
    repo.get_task(task_id)?;
    repo.get_dependencies(task_id)
}

/// A task named from a dependency edge: enough to identify it and judge whether
/// it is finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskRef {
    pub id: i64,
    pub name: String,
    pub status: Status,
    pub priority: Priority,
}

/// A task's dependency neighbourhood, resolved to names and states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DependencyView {
    pub task_id: i64,
    /// Todo and every blocker is done: the task can be claimed now.
    pub ready: bool,
    /// Tasks this one waits for.
    pub blocked_by: Vec<TaskRef>,
    /// Tasks waiting for this one.
    pub blocks: Vec<TaskRef>,
}

/// Resolves the dependency neighbourhood of `task_id`.
pub fn view(repo: &dyn Repository, task_id: i64) -> Result<DependencyView, AppError> {
    let task = repo.get_task(task_id)?;
    let blocked_by = resolve(repo, repo.get_dependencies(task_id)?)?;
    let blocks = resolve(repo, repo.get_dependents(task_id)?)?;
    let ready = task.status == Status::Todo && blocked_by.iter().all(|t| t.status == Status::Done);
    Ok(DependencyView {
        task_id,
        ready,
        blocked_by,
        blocks,
    })
}

fn resolve(repo: &dyn Repository, ids: Vec<i64>) -> Result<Vec<TaskRef>, AppError> {
    ids.into_iter()
        .map(|id| {
            let task = repo.get_task(id)?;
            Ok(TaskRef {
                id,
                name: task.name,
                status: task.status,
                priority: task.priority,
            })
        })
        .collect()
}

/// Tasks that depend on `task_id` (what it blocks).
pub fn dependents(repo: &dyn Repository, task_id: i64) -> Result<Vec<i64>, AppError> {
    repo.get_task(task_id)?;
    repo.get_dependents(task_id)
}

pub fn add(repo: &dyn Repository, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
    repo.get_task(task_id)?;
    repo.get_task(depends_on_id)?;
    repo.add_dependency(task_id, depends_on_id)
}

/// Whether adding `candidate_id` would close a cycle.
pub fn would_create_cycle(
    repo: &dyn Repository,
    task_id: i64,
    candidate_id: i64,
) -> Result<bool, AppError> {
    repo.would_create_cycle(task_id, candidate_id)
}

/// Replaces the full dependency set atomically and returns the stored ids.
pub fn set(
    repo: &dyn Repository,
    task_id: i64,
    depends_on_ids: &[i64],
) -> Result<Vec<i64>, AppError> {
    repo.set_dependencies(task_id, depends_on_ids)?;
    repo.get_dependencies(task_id)
}

pub fn remove(repo: &dyn Repository, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
    repo.get_task(task_id)?;
    repo.get_task(depends_on_id)?;
    repo.remove_dependency(task_id, depends_on_id)
}

/// Makes the dependency set of `task_id` match `depends_on_ids`, touching only
/// the edges that differ. Used when an edit form resends the whole list.
pub fn sync(repo: &dyn Repository, task_id: i64, depends_on_ids: &[i64]) -> Result<(), AppError> {
    let current = repo.get_dependencies(task_id)?;
    for &dep_id in &current {
        if !depends_on_ids.contains(&dep_id) {
            repo.remove_dependency(task_id, dep_id)?;
        }
    }
    for &dep_id in depends_on_ids {
        if !current.contains(&dep_id) {
            repo.add_dependency(task_id, dep_id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{
        DependencyRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    use super::{add, dependents, list, remove, set, sync};

    fn repo_with_tasks(names: &[&str]) -> (StubRepository, Vec<i64>) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let ids = names
            .iter()
            .map(|name| {
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
            })
            .collect();
        (repo, ids)
    }

    #[test]
    fn sync_adds_and_removes_only_the_difference() {
        let (repo, ids) = repo_with_tasks(&["A", "B", "C"]);
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        repo.add_dependency(a, b).unwrap();

        sync(&repo, a, &[c]).unwrap();

        assert_eq!(list(&repo, a).unwrap(), vec![c]);
    }

    #[test]
    fn sync_is_idempotent_for_equal_sets() {
        let (repo, ids) = repo_with_tasks(&["A", "B"]);
        let (a, b) = (ids[0], ids[1]);
        sync(&repo, a, &[b]).unwrap();
        sync(&repo, a, &[b]).unwrap();
        assert_eq!(list(&repo, a).unwrap(), vec![b]);
    }

    #[test]
    fn add_requires_both_tasks_to_exist() {
        let (repo, ids) = repo_with_tasks(&["A"]);
        assert!(add(&repo, ids[0], 999).is_err());
        assert!(add(&repo, 999, ids[0]).is_err());
    }

    #[test]
    fn add_and_remove_round_trip() {
        let (repo, ids) = repo_with_tasks(&["A", "B"]);
        let (a, b) = (ids[0], ids[1]);
        add(&repo, a, b).unwrap();
        assert_eq!(dependents(&repo, b).unwrap(), vec![a]);
        remove(&repo, a, b).unwrap();
        assert!(list(&repo, a).unwrap().is_empty());
    }

    #[test]
    fn set_returns_the_stored_ids() {
        let (repo, ids) = repo_with_tasks(&["A", "B", "C"]);
        let stored = set(&repo, ids[0], &[ids[1], ids[2]]).unwrap();
        assert_eq!(stored.len(), 2);
    }

    #[test]
    fn missing_task_is_reported_for_reads() {
        let (repo, _) = repo_with_tasks(&["A"]);
        assert!(list(&repo, 999).is_err());
        assert!(dependents(&repo, 999).is_err());
    }
}
