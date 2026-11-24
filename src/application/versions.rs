// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Version use cases.

use serde::{Deserialize, Serialize};

use crate::db::repository::{Repository, RepositoryPolicy};
use crate::domain::error::AppError;
use crate::domain::validation::{validate_name, validate_text};
use crate::domain::version::{Version, VersionStatus};

/// Fields of a version to create. The status is applied in the same write, so
/// the version never exists in an intermediate state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewVersion {
    pub project_id: i64,
    pub name: String,
    pub description: String,
    pub status: VersionStatus,
}

/// Changes to apply to an existing version. `None` fields keep their current
/// value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionEdit {
    pub name: String,
    pub description: Option<String>,
    pub status: Option<VersionStatus>,
}

pub fn list(repo: &dyn Repository, project_id: i64) -> Result<Vec<Version>, AppError> {
    repo.get_project(project_id)?;
    repo.list_versions_by_project(project_id)
}

/// Semver name suggested for the next version of a project.
pub fn next_name(repo: &dyn Repository, project_id: i64) -> Result<String, AppError> {
    repo.next_version_name(project_id)
}

pub fn get(repo: &dyn Repository, version_id: i64) -> Result<Version, AppError> {
    repo.get_version(version_id)
}

pub fn create(repo: &dyn Repository, new: NewVersion) -> Result<Version, AppError> {
    validate_name("version", &new.name)?;
    validate_text("version", "description", &new.description)?;
    repo.get_project(new.project_id)?;
    repo.create_version_with_status(new.project_id, &new.name, &new.description, new.status)
}

pub fn edit(
    repo: &dyn Repository,
    version_id: i64,
    edit: VersionEdit,
) -> Result<Version, AppError> {
    validate_name("version", &edit.name)?;
    let current = repo.get_version(version_id)?;
    let description = edit.description.unwrap_or(current.description);
    validate_text("version", "description", &description)?;
    let status = edit.status.unwrap_or(current.status);
    repo.update_version(version_id, &edit.name, &description, status)
}

pub fn delete(repo: &dyn Repository, version_id: i64) -> Result<(), AppError> {
    repo.delete_version(version_id)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{ProjectRepository, VersionRepository};
    use crate::db::stub::StubRepository;
    use crate::domain::version::VersionStatus;

    use super::{NewVersion, VersionEdit, create, delete, edit, get, list};

    fn repo_with_project() -> (StubRepository, i64) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        (repo, project_id)
    }

    #[test]
    fn create_applies_the_requested_status() {
        let (repo, project_id) = repo_with_project();
        let version = create(
            &repo,
            NewVersion {
                project_id,
                name: "2.0.0".to_string(),
                description: "Next".to_string(),
                status: VersionStatus::Open,
            },
        )
        .unwrap();
        assert_eq!(version.status, VersionStatus::Open);
        assert_eq!(list(&repo, project_id).unwrap().len(), 2);
    }

    #[test]
    fn create_requires_an_existing_project() {
        let (repo, _) = repo_with_project();
        let result = create(
            &repo,
            NewVersion {
                project_id: 999,
                name: "v".to_string(),
                description: String::new(),
                status: VersionStatus::Backlog,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn edit_keeps_omitted_fields() {
        let (repo, project_id) = repo_with_project();
        let version = repo.create_version(project_id, "1.1.0", "keep").unwrap();
        let version_id = version.id.unwrap();
        let edited = edit(
            &repo,
            version_id,
            VersionEdit {
                name: "1.2.0".to_string(),
                description: None,
                status: Some(VersionStatus::Released),
            },
        )
        .unwrap();
        assert_eq!(edited.name, "1.2.0");
        assert_eq!(edited.description, "keep");
        assert_eq!(edited.status, VersionStatus::Released);
    }

    #[test]
    fn delete_removes_the_version() {
        let (repo, project_id) = repo_with_project();
        let version_id = repo
            .create_version(project_id, "1.1.0", "")
            .unwrap()
            .id
            .unwrap();
        delete(&repo, version_id).unwrap();
        assert!(get(&repo, version_id).is_err());
        assert_eq!(list(&repo, project_id).unwrap().len(), 1);
    }
}
