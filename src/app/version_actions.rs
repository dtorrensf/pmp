// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Version use cases dispatched from the TUI: save (create or edit) and
//! delete.

use crate::application::versions::{self, NewVersion, VersionEdit};
use crate::db::repository::Repository;
use crate::domain::error::AppError;
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
) -> Result<Outcome, AppError> {
    match version_id {
        Some(id) => versions::edit(
            repo,
            id,
            VersionEdit {
                name: name.to_string(),
                description: Some(description.to_string()),
                status: Some(status),
            },
        )?,
        // The version is created with the chosen status in a single
        // write, so it never exists in an intermediate state.
        None => versions::create(
            repo,
            NewVersion {
                project_id,
                name: name.to_string(),
                description: description.to_string(),
                status,
            },
        )?,
    };
    Ok(Outcome::confirmed(
        "Version saved",
        Some(Action::Navigate(Screen::ProjectDashboard { project_id })),
    ))
}

pub(super) fn delete(repo: &dyn Repository, version_id: i64) -> Result<Outcome, AppError> {
    let project_id = versions::get(repo, version_id)?.project_id;
    versions::delete(repo, version_id)?;
    Ok(Outcome::confirmed(
        "Version deleted",
        Some(Action::Navigate(Screen::ProjectDashboard { project_id })),
    ))
}
