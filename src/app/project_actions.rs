// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Result, anyhow};

use crate::db::repository::Repository;
use crate::screens::{Action, Screen};

use super::outcome::Outcome;

/// Creates or updates a project. Editing returns to `return_to`; creating
/// opens the new project's dashboard.
pub(super) fn save(
    repo: &dyn Repository,
    project_id: Option<i64>,
    name: &str,
    description: &str,
    return_to: &Screen,
) -> Result<Outcome> {
    if name.trim().is_empty() {
        return Err(anyhow!("Project name cannot be empty"));
    }
    let project = match project_id {
        Some(id) => repo.update_project(id, name, description)?,
        None => repo.create_project(name, description)?,
    };
    let next = match project_id {
        Some(_) => return_to.clone(),
        None => Screen::ProjectDashboard {
            project_id: project.id.unwrap_or(0),
        },
    };
    Ok(Outcome::confirmed(
        "Project saved",
        Some(Action::Navigate(next)),
    ))
}

pub(super) fn delete(repo: &dyn Repository, project_id: i64) -> Result<Outcome> {
    repo.delete_project(project_id)?;
    Ok(Outcome::confirmed(
        "Project deleted",
        Some(Action::Navigate(Screen::ProjectList)),
    ))
}
