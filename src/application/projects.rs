// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Project use cases.

use serde::{Deserialize, Serialize};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::validation::{validate_name, validate_text};

/// Fields of a project to create.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub description: String,
}

/// Changes to apply to an existing project. `description: None` keeps the
/// current one, so a client can rename without resending the whole record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectEdit {
    pub name: String,
    pub description: Option<String>,
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
    repo.create_project(&new.name, &new.description)
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
    repo.update_project(project_id, &edit.name, &description)
}

pub fn delete(repo: &dyn Repository, project_id: i64) -> Result<(), AppError> {
    repo.delete_project(project_id)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::ProjectRepository;
    use crate::db::stub::StubRepository;

    use super::{NewProject, ProjectEdit, create, delete, edit, get, list};

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
            },
        )
        .unwrap();
        assert_eq!(project.name, "Alpha");
        assert_eq!(list(&repo).unwrap().len(), 1);
    }

    #[test]
    fn edit_keeps_the_description_when_it_is_omitted() {
        let repo = repo();
        let project = repo.create_project("Alpha", "keep me").unwrap();
        let project_id = project.id.unwrap();

        let edited = edit(
            &repo,
            project_id,
            ProjectEdit {
                name: "Beta".to_string(),
                description: None,
            },
        )
        .unwrap();

        assert_eq!(edited.name, "Beta");
        assert_eq!(edited.description, "keep me");
    }

    #[test]
    fn edit_replaces_the_description_when_provided() {
        let repo = repo();
        let project_id = repo.create_project("Alpha", "old").unwrap().id.unwrap();
        let edited = edit(
            &repo,
            project_id,
            ProjectEdit {
                name: "Alpha".to_string(),
                description: Some("new".to_string()),
            },
        )
        .unwrap();
        assert_eq!(edited.description, "new");
    }

    #[test]
    fn delete_removes_the_project() {
        let repo = repo();
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        delete(&repo, project_id).unwrap();
        assert!(get(&repo, project_id).is_err());
    }
}
