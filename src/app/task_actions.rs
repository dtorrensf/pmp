// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Result, anyhow};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status};
use crate::screens::{Action, Screen};

use super::outcome::Outcome;

#[allow(clippy::too_many_arguments)]
pub(super) fn save(
    repo: &dyn Repository,
    task_id: Option<i64>,
    version_id: i64,
    name: &str,
    description: &str,
    status: Status,
    priority: Priority,
    depends_on_ids: &[i64],
    return_to: &Screen,
) -> Result<Outcome> {
    if name.trim().is_empty() {
        return Err(anyhow!("Task name cannot be empty"));
    }
    let project_id = repo.get_version(version_id)?.project_id;
    let task = match task_id {
        Some(id) => repo.update_task(id, name, description, status, priority)?,
        None => repo.create_task(project_id, version_id, name, description, status, priority)?,
    };
    sync_dependencies(repo, task.id.unwrap_or(0), depends_on_ids)?;
    Ok(Outcome::confirmed(
        "Task saved",
        Some(Action::Navigate(return_to.clone())),
    ))
}

/// Moves the task's dependency set to `depends_on_ids`, adding and removing
/// only the links that differ.
fn sync_dependencies(repo: &dyn Repository, task_id: i64, depends_on_ids: &[i64]) -> Result<()> {
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

/// Changes a task's status, then reloads `refresh` so its cached list updates.
pub(super) fn update_status(
    repo: &dyn Repository,
    task_id: i64,
    new_status: Status,
    refresh: Screen,
) -> Result<Outcome> {
    repo.update_task_status(task_id, new_status)?;
    Ok(Outcome::confirmed(
        format!("Status set to {}", new_status.label()),
        Some(Action::Navigate(refresh)),
    ))
}

pub(super) fn move_to_version(
    repo: &dyn Repository,
    task_id: i64,
    version_id: i64,
    refresh: Screen,
) -> Result<Outcome> {
    repo.move_task(task_id, version_id)?;
    // Re-running init reloads the list without the moved task.
    Ok(Outcome::confirmed(
        "Task moved",
        Some(Action::Navigate(refresh)),
    ))
}

pub(super) fn delete(repo: &dyn Repository, task_id: i64, refresh: Screen) -> Result<Outcome> {
    repo.delete_task(task_id)?;
    Ok(Outcome::confirmed(
        "Task deleted",
        Some(Action::Navigate(refresh)),
    ))
}

pub(super) fn add_note(repo: &dyn Repository, task_id: i64, content: &str) -> Result<Outcome> {
    match repo.add_task_note(task_id, content) {
        Ok(_) => Ok(Outcome::confirmed("Note added", Some(Action::Refresh))),
        Err(AppError::Validation(_)) => Err(anyhow!("Note content cannot be empty")),
        Err(e) => Err(e.into()),
    }
}
