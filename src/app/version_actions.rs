// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Result, anyhow};

use crate::db::repository::Repository;
use crate::domain::version::VersionStatus;
use crate::screens::{Action, Screen};

use super::outcome::Outcome;

pub(super) fn save(
    repo: &dyn Repository,
    version_id: Option<i64>,
    project_id: i64,
    name: &str,
    description: &str,
    status: VersionStatus,
) -> Result<Outcome> {
    if name.trim().is_empty() {
        return Err(anyhow!("Version name cannot be empty"));
    }
    match version_id {
        Some(id) => repo.update_version(id, name, description, status)?,
        // The version is created with the chosen status in a single
        // write, so it never exists in an intermediate state.
        None => repo.create_version_with_status(project_id, name, description, status)?,
    };
    Ok(Outcome::confirmed(
        "Version saved",
        Some(Action::Navigate(Screen::ProjectDashboard { project_id })),
    ))
}

pub(super) fn delete(repo: &dyn Repository, version_id: i64) -> Result<Outcome> {
    let project_id = repo.get_version(version_id)?.project_id;
    repo.delete_version(version_id)?;
    Ok(Outcome::confirmed(
        "Version deleted",
        Some(Action::Navigate(Screen::ProjectDashboard { project_id })),
    ))
}
