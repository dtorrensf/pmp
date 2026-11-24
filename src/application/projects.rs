// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Project use cases.

use serde::{Deserialize, Serialize};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::validation::{normalize_working_dir, validate_name, validate_text};

/// Fields of a project to create.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub description: String,
    /// Absolute path of the project's workspace; blank or absent means none.
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Changes to apply to an existing project. `description: None` keeps the
/// current one, so a client can rename without resending the whole record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectEdit {
    pub name: String,
    pub description: Option<String>,
    /// `None` keeps the current directory, `Some("")` clears it and any other
    /// value replaces it (normalized by the validator).
    #[serde(default)]
    pub working_dir: Option<String>,
}

pub fn list(repo: &dyn Repository) -> Result<Vec<Project>, AppError> {
    repo.list_projects()
}

pub fn get(repo: &dyn Repository, project_id: i64) -> Result<Project, AppError> {
    repo.get_project(project_id)
}

pub fn create(repo: &dyn Repository, new: NewProject) -> Result<Project, AppError> {
    validate_name("project", &new.name)?;
    validate_text("project", "description", &new.description)?;
    let working_dir = normalize_working_dir(new.working_dir.as_deref())?;
    repo.create_project(&new.name, &new.description, working_dir.as_deref())
}

pub fn edit(
    repo: &dyn Repository,
    project_id: i64,
    edit: ProjectEdit,
) -> Result<Project, AppError> {
    validate_name("project", &edit.name)?;
    let current = repo.get_project(project_id)?;
    let description = edit.description.unwrap_or(current.description);
    validate_text("project", "description", &description)?;
    let working_dir = match edit.working_dir {
        None => current.working_dir,
        Some(value) => normalize_working_dir(Some(&value))?,
    };
    repo.update_project(project_id, &edit.name, &description, working_dir.as_deref())
}

pub fn delete(repo: &dyn Repository, project_id: i64) -> Result<(), AppError> {
    repo.delete_project(project_id)
}

/// The working directory required by flows that operate on the filesystem,
/// such as launching an ACP agent or scoping an MCP workspace.
///
/// Projects without one stay fully usable everywhere else; only these flows
/// fail, with an actionable message.
pub fn require_filesystem_working_dir(project: &Project) -> Result<&str, AppError> {
    project.working_dir.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "project '{}' has no working directory; set one before running an agent",
            project.name
        ))
    })
}

#[cfg(test)]
mod tests {
    use crate::db::repository::ProjectRepository;
    use crate::db::stub::StubRepository;
    use crate::domain::error::AppError;

    use super::{
        NewProject, ProjectEdit, create, delete, edit, get, list, require_filesystem_working_dir,
    };

    fn repo() -> StubRepository {
        StubRepository::default()
    }

    #[test]
    fn create_returns_the_stored_project() {
        let repo = repo();
        let project = create(
            &repo,
            NewProject {
                name: "Alpha".to_string(),
                description: "First".to_string(),
                working_dir: Some("/home/me/alpha".to_string()),
            },
        )
        .unwrap();
        assert_eq!(project.name, "Alpha");
        assert_eq!(project.working_dir.as_deref(), Some("/home/me/alpha"));
        assert_eq!(list(&repo).unwrap().len(), 1);
    }

    #[test]
    fn filesystem_flows_require_a_working_dir() {
        let repo = repo();
        let bare = create(
            &repo,
            NewProject {
                name: "Bare".to_string(),
                description: String::new(),
                working_dir: None,
            },
        )
        .unwrap();
        let error = require_filesystem_working_dir(&bare).unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("has no working directory")),
            "{error:?}"
        );

        let bound = create(
            &repo,
            NewProject {
                name: "Bound".to_string(),
                description: String::new(),
                working_dir: Some("/srv/repos/bound".to_string()),
            },
        )
        .unwrap();
        assert_eq!(
            require_filesystem_working_dir(&bound).unwrap(),
            "/srv/repos/bound"
        );
    }

    #[test]
    fn edit_keeps_the_description_when_it_is_omitted() {
        let repo = repo();
        let project = repo.create_project("Alpha", "keep me", None).unwrap();
        let project_id = project.id.unwrap();

        let edited = edit(
            &repo,
            project_id,
            ProjectEdit {
                name: "Beta".to_string(),
                description: None,
                working_dir: None,
            },
        )
        .unwrap();

        assert_eq!(edited.name, "Beta");
        assert_eq!(edited.description, "keep me");
        assert_eq!(edited.working_dir, None);
    }

    #[test]
    fn edit_replaces_the_description_when_provided() {
        let repo = repo();
        let project_id = repo
            .create_project("Alpha", "old", None)
            .unwrap()
            .id
            .unwrap();
        let edited = edit(
            &repo,
            project_id,
            ProjectEdit {
                name: "Alpha".to_string(),
                description: Some("new".to_string()),
                working_dir: None,
            },
        )
        .unwrap();
        assert_eq!(edited.description, "new");
    }

    #[test]
    fn delete_removes_the_project() {
        let repo = repo();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        delete(&repo, project_id).unwrap();
        assert!(get(&repo, project_id).is_err());
    }
}
