// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task use cases: creation, edits, transitions and paginated listing.
//!
//! The TUI edits a plain description and resends status, priority and
//! dependencies; MCP patches one or more Context/Requirements/Benefits
//! sections. Both go through [`edit`], which keeps whatever the caller omits,
//! so the two channels can no longer drift apart.

use serde::{Deserialize, Serialize};

use crate::db::repository::{Repository, TaskFilter};
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::task_description::{compose_description, split_description};
use crate::domain::validation::{validate_name, validate_text};

use super::dependencies;

/// Fields of a task to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    /// Project of the task. Omit it when only the version is known (the TUI
    /// does): the version's project is used.
    pub project_id: Option<i64>,
    pub version_id: i64,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub priority: Priority,
    /// Dependencies to create with the task, applied in the same call.
    pub depends_on: Vec<i64>,
}

/// Changes to apply to an existing task. Every field is optional: `None` keeps
/// the current value. Sending any of the `context`/`requirements`/`benefits`
/// sections rewrites the description from the three sections, keeping those
/// that were not sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub context: Option<String>,
    pub requirements: Option<String>,
    pub benefits: Option<String>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
    /// Full replacement for the dependency set. `None` leaves it untouched.
    pub depends_on: Option<Vec<i64>>,
}

/// One page of tasks plus the metadata a client needs to ask for the next one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskPage {
    pub tasks: Vec<Task>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
    pub has_more: bool,
}

pub fn list_by_project(repo: &dyn Repository, project_id: i64) -> Result<Vec<Task>, AppError> {
    repo.list_tasks_by_project(project_id)
}

pub fn list_by_version(repo: &dyn Repository, version_id: i64) -> Result<Vec<Task>, AppError> {
    repo.list_tasks_by_version(version_id)
}

/// Filters and paginates the tasks of a project. A `version_id` in the filter
/// must belong to that project.
pub fn page(
    repo: &dyn Repository,
    filter: &TaskFilter,
    limit: u32,
    offset: u32,
) -> Result<TaskPage, AppError> {
    repo.get_project(filter.project_id)?;
    if let Some(version_id) = filter.version_id {
        let mismatch =
            || AppError::Validation("version does not belong to the project".to_string());
        let version = repo.get_version(version_id).map_err(|_| mismatch())?;
        if version.project_id != filter.project_id {
            return Err(mismatch());
        }
    }
    let tasks = repo.list_tasks_page(filter, limit, offset)?;
    let total = repo.count_tasks(filter)?;
    Ok(TaskPage {
        has_more: (u64::from(offset) + tasks.len() as u64) < total,
        tasks,
        total,
        limit,
        offset,
    })
}

pub fn get(repo: &dyn Repository, task_id: i64) -> Result<Task, AppError> {
    repo.get_task(task_id)
}

pub fn create(repo: &dyn Repository, new: NewTask) -> Result<Task, AppError> {
    validate_name("task", &new.name)?;
    validate_text("task", "description", &new.description)?;
    let project_id = match new.project_id {
        Some(project_id) => {
            repo.get_project(project_id)?;
            project_id
        }
        None => repo.get_version(new.version_id)?.project_id,
    };
    let task = repo.create_task(
        project_id,
        new.version_id,
        &new.name,
        &new.description,
        new.status,
        new.priority,
    )?;
    if let Some(task_id) = task.id {
        dependencies::sync(repo, task_id, &new.depends_on)?;
    }
    Ok(task)
}

pub fn edit(repo: &dyn Repository, task_id: i64, edit: TaskEdit) -> Result<Task, AppError> {
    if let Some(name) = &edit.name {
        validate_name("task", name)?;
    }
    let current = repo.get_task(task_id)?;
    let name = edit.name.unwrap_or_else(|| current.name.clone());
    let has_sections =
        edit.context.is_some() || edit.requirements.is_some() || edit.benefits.is_some();
    let description = if has_sections {
        let mut sections = split_description(&current.description);
        if let Some(context) = edit.context {
            validate_text("task", "context", &context)?;
            sections.context = context;
        }
        if let Some(requirements) = edit.requirements {
            validate_text("task", "requirements", &requirements)?;
            sections.requirements = requirements;
        }
        if let Some(benefits) = edit.benefits {
            validate_text("task", "benefits", &benefits)?;
            sections.benefits = benefits;
        }
        compose_description(
            &sections.context,
            &sections.requirements,
            &sections.benefits,
        )
    } else {
        edit.description
            .unwrap_or_else(|| current.description.clone())
    };
    validate_text("task", "description", &description)?;
    let status = edit.status.unwrap_or(current.status);
    let priority = edit.priority.unwrap_or(current.priority);
    let task = repo.update_task(task_id, &name, &description, status, priority)?;
    if let Some(ids) = edit.depends_on {
        dependencies::sync(repo, task_id, &ids)?;
    }
    Ok(task)
}

pub fn set_status(repo: &dyn Repository, task_id: i64, status: Status) -> Result<Task, AppError> {
    repo.update_task_status(task_id, status)
}

pub fn set_priority(
    repo: &dyn Repository,
    task_id: i64,
    priority: Priority,
) -> Result<Task, AppError> {
    repo.update_task_priority(task_id, priority)
}

pub fn move_to_version(
    repo: &dyn Repository,
    task_id: i64,
    version_id: i64,
) -> Result<Task, AppError> {
    repo.move_task(task_id, version_id)
}

pub fn delete(repo: &dyn Repository, task_id: i64) -> Result<(), AppError> {
    repo.delete_task(task_id)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{DependencyRepository, ProjectRepository, RepositoryPolicy};
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    use super::{
        NewTask, TaskEdit, create, edit, get, list_by_project, list_by_version, page, set_priority,
        set_status,
    };
    use crate::db::repository::TaskFilter;

    fn repo_with_version() -> (StubRepository, i64, i64) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        (repo, project_id, version_id)
    }

    fn new_task(version_id: i64, name: &str) -> NewTask {
        NewTask {
            project_id: None,
            version_id,
            name: name.to_string(),
            description: "## Context\n\nctx".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            depends_on: Vec::new(),
        }
    }

    #[test]
    fn create_resolves_the_project_from_the_version() {
        let (repo, project_id, version_id) = repo_with_version();
        let task = create(&repo, new_task(version_id, "T")).unwrap();
        assert_eq!(task.project_id, project_id);
        assert_eq!(task.version_id, version_id);
    }

    #[test]
    fn create_applies_the_initial_dependencies() {
        let (repo, _, version_id) = repo_with_version();
        let blocker = create(&repo, new_task(version_id, "A"))
            .unwrap()
            .id
            .unwrap();
        let mut input = new_task(version_id, "B");
        input.depends_on = vec![blocker];

        let task = create(&repo, input).unwrap();

        assert_eq!(
            repo.get_dependencies(task.id.unwrap()).unwrap(),
            vec![blocker]
        );
    }

    #[test]
    fn create_rejects_a_blank_name() {
        let (repo, _, version_id) = repo_with_version();
        let mut input = new_task(version_id, "  ");
        input.status = Status::Todo;
        assert!(create(&repo, input).is_err());
        assert!(list_by_project(&repo, 1).unwrap().is_empty());
    }

    #[test]
    fn edit_keeps_everything_the_caller_omits() {
        let (repo, _, version_id) = repo_with_version();
        let task = create(&repo, new_task(version_id, "A")).unwrap();
        let edited = edit(
            &repo,
            task.id.unwrap(),
            TaskEdit {
                name: Some("B".to_string()),
                ..TaskEdit::default()
            },
        )
        .unwrap();
        assert_eq!(edited.name, "B");
        assert_eq!(edited.description, task.description);
        assert_eq!(edited.status, task.status);
        assert_eq!(edited.priority, task.priority);
    }

    #[test]
    fn edit_merges_sections_and_keeps_the_untouched_ones() {
        let (repo, _, version_id) = repo_with_version();
        let mut input = new_task(version_id, "A");
        input.description =
            crate::domain::task_description::compose_description("ctx", "req", "ben");
        let task = create(&repo, input).unwrap();

        let edited = edit(
            &repo,
            task.id.unwrap(),
            TaskEdit {
                requirements: Some("- new requirement".to_string()),
                ..TaskEdit::default()
            },
        )
        .unwrap();

        let sections = crate::domain::task_description::split_description(&edited.description);
        assert_eq!(sections.context, "ctx");
        assert_eq!(sections.requirements, "- new requirement");
        assert_eq!(sections.benefits, "ben");
    }

    #[test]
    fn edit_can_replace_the_whole_description() {
        let (repo, _, version_id) = repo_with_version();
        let task = create(&repo, new_task(version_id, "A")).unwrap();
        let edited = edit(
            &repo,
            task.id.unwrap(),
            TaskEdit {
                description: Some("plain".to_string()),
                ..TaskEdit::default()
            },
        )
        .unwrap();
        assert_eq!(edited.description, "plain");
    }

    #[test]
    fn edit_applies_status_priority_and_dependencies() {
        let (repo, _, version_id) = repo_with_version();
        let blocker = create(&repo, new_task(version_id, "A"))
            .unwrap()
            .id
            .unwrap();
        let task = create(&repo, new_task(version_id, "B")).unwrap();
        let edited = edit(
            &repo,
            task.id.unwrap(),
            TaskEdit {
                status: Some(Status::InProgress),
                priority: Some(Priority::High),
                depends_on: Some(vec![blocker]),
                ..TaskEdit::default()
            },
        )
        .unwrap();
        assert_eq!(edited.status, Status::InProgress);
        assert_eq!(edited.priority, Priority::High);
        assert_eq!(
            repo.get_dependencies(task.id.unwrap()).unwrap(),
            vec![blocker]
        );
    }

    #[test]
    fn page_rejects_a_version_from_another_project() {
        let (repo, project_id, _) = repo_with_version();
        let other = repo.create_project("Beta", "").unwrap().id.unwrap();
        let foreign_version = repo.default_version_id(other).unwrap();
        let result = page(
            &repo,
            &TaskFilter {
                project_id,
                version_id: Some(foreign_version),
                status: None,
                priority: None,
            },
            10,
            0,
        );
        assert!(result.is_err());
    }

    #[test]
    fn page_reports_total_and_has_more() {
        let (repo, project_id, version_id) = repo_with_version();
        for name in ["A", "B", "C"] {
            create(&repo, new_task(version_id, name)).unwrap();
        }
        let first = page(
            &repo,
            &TaskFilter {
                project_id,
                version_id: Some(version_id),
                status: None,
                priority: None,
            },
            2,
            0,
        )
        .unwrap();
        assert_eq!(first.tasks.len(), 2);
        assert_eq!(first.total, 3);
        assert!(first.has_more);

        let second = page(
            &repo,
            &TaskFilter {
                project_id,
                version_id: Some(version_id),
                status: None,
                priority: None,
            },
            2,
            2,
        )
        .unwrap();
        assert_eq!(second.tasks.len(), 1);
        assert!(!second.has_more);
    }

    #[test]
    fn transitions_and_reads_go_through_the_repository_rules() {
        let (repo, _, version_id) = repo_with_version();
        let task = create(&repo, new_task(version_id, "A")).unwrap();
        let task_id = task.id.unwrap();
        assert_eq!(
            set_status(&repo, task_id, Status::Done).unwrap().status,
            Status::Done
        );
        assert_eq!(
            set_priority(&repo, task_id, Priority::Low)
                .unwrap()
                .priority,
            Priority::Low
        );
        assert_eq!(get(&repo, task_id).unwrap().id, Some(task_id));
        assert_eq!(list_by_version(&repo, version_id).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod wire_tests {
    use super::{TaskEdit, TaskPage};
    use crate::domain::task::Status;

    #[test]
    fn task_edit_accepts_an_empty_javascript_object() {
        let edit: TaskEdit = serde_json::from_str("{}").unwrap();
        assert_eq!(edit, TaskEdit::default());
    }

    #[test]
    fn task_edit_deserializes_partial_updates() {
        let edit: TaskEdit =
            serde_json::from_str(r#"{"status":"in_progress","name":"Renamed"}"#).unwrap();
        assert_eq!(edit.name.as_deref(), Some("Renamed"));
        assert_eq!(edit.status, Some(Status::InProgress));
        assert_eq!(edit.description, None);
    }

    #[test]
    fn task_page_round_trips_as_json() {
        let page = TaskPage {
            tasks: Vec::new(),
            total: 0,
            limit: 10,
            offset: 0,
            has_more: false,
        };
        let json = serde_json::to_string(&page).unwrap();
        let back: TaskPage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
    }
}
