// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task use cases dispatched from the TUI: save (with dependency
//! synchronisation), status change, move between versions, delete and notes.

use crate::application::notes;
use crate::application::tasks::{self, NewTask, TaskEdit};
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
) -> Result<Outcome, AppError> {
    match task_id {
        Some(id) => tasks::edit(
            repo,
            id,
            TaskEdit {
                name: Some(name.to_string()),
                description: Some(description.to_string()),
                status: Some(status),
                priority: Some(priority),
                depends_on: Some(depends_on_ids.to_vec()),
                ..TaskEdit::default()
            },
        )?,
        None => tasks::create(
            repo,
            NewTask {
                project_id: None,
                version_id,
                name: name.to_string(),
                description: description.to_string(),
                status,
                priority,
                depends_on: depends_on_ids.to_vec(),
            },
        )?,
    };
    Ok(Outcome::confirmed(
        "Task saved",
        Some(Action::Navigate(return_to.clone())),
    ))
}

/// Changes a task's status, then reloads `refresh` so its cached list updates.
pub(super) fn update_status(
    repo: &dyn Repository,
    task_id: i64,
    new_status: Status,
    refresh: Screen,
) -> Result<Outcome, AppError> {
    tasks::set_status(repo, task_id, new_status)?;
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
) -> Result<Outcome, AppError> {
    tasks::move_to_version(repo, task_id, version_id)?;
    // Re-running init reloads the list without the moved task.
    Ok(Outcome::confirmed(
        "Task moved",
        Some(Action::Navigate(refresh)),
    ))
}

pub(super) fn delete(
    repo: &dyn Repository,
    task_id: i64,
    refresh: Screen,
) -> Result<Outcome, AppError> {
    tasks::delete(repo, task_id)?;
    Ok(Outcome::confirmed(
        "Task deleted",
        Some(Action::Navigate(refresh)),
    ))
}

pub(super) fn add_note(
    repo: &dyn Repository,
    task_id: i64,
    content: &str,
) -> Result<Outcome, AppError> {
    notes::add(repo, task_id, content)?;
    Ok(Outcome::confirmed("Note added", Some(Action::Refresh)))
}
