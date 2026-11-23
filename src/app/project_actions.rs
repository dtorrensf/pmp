// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Project use cases dispatched from the TUI: save (create or edit) and
//! delete.

use anyhow::{Result, anyhow};

use crate::application::projects::{self, NewProject, ProjectEdit};
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
    let next = match project_id {
        Some(id) => {
            projects::edit(
                repo,
                id,
                ProjectEdit {
                    name: name.to_string(),
                    description: Some(description.to_string()),
                },
            )?;
            return_to.clone()
        }
        None => Screen::ProjectDashboard {
            project_id: projects::create(
                repo,
                NewProject {
                    name: name.to_string(),
                    description: description.to_string(),
                },
            )?
            .id
            .unwrap_or(0),
        },
    };
    Ok(Outcome::confirmed(
        "Project saved",
        Some(Action::Navigate(next)),
    ))
}

pub(super) fn delete(repo: &dyn Repository, project_id: i64) -> Result<Outcome> {
    projects::delete(repo, project_id)?;
    Ok(Outcome::confirmed(
        "Project deleted",
        Some(Action::Navigate(Screen::ProjectList)),
    ))
}
