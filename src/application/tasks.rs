// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Task use cases: creation, edits, transitions and paginated listing.
//!
//! The TUI edits a plain description and resends status, priority and
//! dependencies; MCP patches one or more Context/Requirements/Benefits
//! sections. Both go through [`edit`], which keeps whatever the caller omits,
//! so the two channels can no longer drift apart.

use serde::{Deserialize, Serialize};

use crate::db::repository::{Readiness, Repository, TaskFilter};
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::task_description::{compose_description, split_description};
use crate::domain::timestamp::normalize_since;
use crate::domain::validation::{validate_name, validate_text};

use super::dependencies;
use super::pagination::PageMeta;

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

/// Lightweight view of a task for listings: identity, state, timestamps and
/// dependency counters. The description is included only on request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskSummary {
    pub id: Option<i64>,
    pub project_id: i64,
    pub version_id: i64,
    pub name: String,
    pub status: Status,
    pub priority: Priority,
    pub created_at: String,
    pub completed_at: Option<String>,
    /// Tasks this one waits for.
    pub depends_on_count: u32,
    /// Tasks waiting for this one.
    pub blocks_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One page of task summaries plus the metadata a client needs to ask for the next one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskPage {
    pub tasks: Vec<TaskSummary>,
    #[serde(flatten)]
    pub meta: PageMeta,
}

fn summarize(
    repo: &dyn Repository,
    task: Task,
    include_description: bool,
) -> Result<TaskSummary, AppError> {
    let id = task.id.ok_or(AppError::Internal)?;
    let depends_on_count = repo.get_dependencies(id)?.len() as u32;
    let blocks_count = repo.get_dependents(id)?.len() as u32;
    Ok(TaskSummary {
        id: task.id,
        project_id: task.project_id,
        version_id: task.version_id,
        name: task.name,
        status: task.status,
        priority: task.priority,
        created_at: task.created_at,
        completed_at: task.completed_at,
        depends_on_count,
        blocks_count,
        description: include_description.then_some(task.description),
    })
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
    include_description: bool,
) -> Result<TaskPage, AppError> {
    let filter = normalize_filter(filter)?;
    ensure_scope(repo, filter.project_id, filter.version_id)?;
    let tasks = repo.list_tasks_page(&filter, limit, offset)?;
    let total = repo.count_tasks(&filter)?;
    to_page(repo, tasks, total, limit, offset, include_description)
}

/// Largest number of ids accepted by one `ids` filter.
pub const MAX_ID_FILTER: usize = 500;

/// Validates client input for a listing filter and normalizes it: the search
/// text is trimmed and must not be blank, the id list is bounded, and the time
/// bound becomes a full UTC timestamp.
pub fn normalize_filter(filter: &TaskFilter) -> Result<TaskFilter, AppError> {
    let mut filter = filter.clone();
    if let Some(text) = &filter.text {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation("query must not be blank".to_string()));
        }
        validate_text("task", "query", trimmed)?;
        filter.text = Some(trimmed.to_string());
    }
    if filter
        .ids
        .as_ref()
        .is_some_and(|ids| ids.len() > MAX_ID_FILTER)
    {
        return Err(AppError::Validation(format!(
            "ids accepts at most {MAX_ID_FILTER} values"
        )));
    }
    if let Some(since) = &filter.touched_since {
        filter.touched_since = Some(normalize_since(since)?);
    }
    Ok(filter)
}

/// Maps the two MCP flags to a readiness filter. Asking for both is rejected
/// because no task can satisfy both.
pub fn readiness_from_flags(
    only_ready: bool,
    only_blocked: bool,
) -> Result<Option<Readiness>, AppError> {
    match (only_ready, only_blocked) {
        (true, true) => Err(AppError::Validation(
            "only_ready and only_blocked cannot both be true".to_string(),
        )),
        (true, false) => Ok(Some(Readiness::Ready)),
        (false, true) => Ok(Some(Readiness::Blocked)),
        (false, false) => Ok(None),
    }
}

/// Checks that a project exists and, when given, that the version belongs to it.
fn ensure_scope(
    repo: &dyn Repository,
    project_id: i64,
    version_id: Option<i64>,
) -> Result<(), AppError> {
    repo.get_project(project_id)?;
    if let Some(version_id) = version_id {
        let mismatch =
            || AppError::Validation("version does not belong to the project".to_string());
        let version = repo.get_version(version_id).map_err(|_| mismatch())?;
        if version.project_id != project_id {
            return Err(mismatch());
        }
    }
    Ok(())
}

fn to_page(
    repo: &dyn Repository,
    tasks: Vec<Task>,
    total: u64,
    limit: u32,
    offset: u32,
    include_description: bool,
) -> Result<TaskPage, AppError> {
    let returned = tasks.len();
    let tasks = tasks
        .into_iter()
        .map(|task| summarize(repo, task, include_description))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TaskPage {
        tasks,
        meta: super::pagination::meta(total, returned, limit, offset),
    })
}

/// Tasks ready to be implemented (todo, every dependency done), highest
/// priority first, paginated like [`page`].
pub fn ready_page(
    repo: &dyn Repository,
    project_id: i64,
    version_id: Option<i64>,
    limit: u32,
    offset: u32,
    include_description: bool,
) -> Result<TaskPage, AppError> {
    ensure_scope(repo, project_id, version_id)?;
    let ready = repo.list_ready_tasks(project_id, version_id)?;
    let total = ready.len() as u64;
    let (window, _) = super::pagination::window(ready, limit, offset);
    to_page(repo, window, total, limit, offset, include_description)
}

/// Claims the next ready task: moves it to `InProgress` and, when given,
/// records `note` as its start note, in one atomic step.
pub fn claim_next(
    repo: &dyn Repository,
    project_id: i64,
    version_id: Option<i64>,
    note: Option<&str>,
) -> Result<Task, AppError> {
    ensure_scope(repo, project_id, version_id)?;
    if let Some(note) = note {
        validate_text("note", "content", note)?;
    }
    repo.claim_next_ready_task(project_id, version_id, note)
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
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
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
        let other = repo.create_project("Beta", "", None).unwrap().id.unwrap();
        let foreign_version = repo.default_version_id(other).unwrap();
        let result = page(
            &repo,
            &TaskFilter {
                project_id,
                version_id: Some(foreign_version),
                status: None,
                priority: None,
                ..Default::default()
            },
            10,
            0,
            false,
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
                ..Default::default()
            },
            2,
            0,
            false,
        )
        .unwrap();
        assert_eq!(first.tasks.len(), 2);
        assert_eq!(first.meta.total, 3);
        assert!(first.meta.has_more);

        let second = page(
            &repo,
            &TaskFilter {
                project_id,
                version_id: Some(version_id),
                status: None,
                priority: None,
                ..Default::default()
            },
            2,
            2,
            false,
        )
        .unwrap();
        assert_eq!(second.tasks.len(), 1);
        assert!(!second.meta.has_more);
    }

    #[test]
    fn page_is_a_summary_unless_the_description_is_requested() {
        let (repo, project_id, version_id) = repo_with_version();
        let task = create(&repo, new_task(version_id, "A")).unwrap();
        let filter = TaskFilter {
            project_id,
            version_id: Some(version_id),
            status: None,
            priority: None,
            ..Default::default()
        };

        let light = page(&repo, &filter, 10, 0, false).unwrap();
        assert_eq!(light.tasks[0].name, "A");
        assert_eq!(light.tasks[0].description, None);

        let full = page(&repo, &filter, 10, 0, true).unwrap();
        assert_eq!(
            full.tasks[0].description.as_deref(),
            Some(task.description.as_str())
        );
    }

    #[test]
    fn summary_counts_dependencies_and_blockers() {
        let (repo, project_id, version_id) = repo_with_version();
        let a = create(&repo, new_task(version_id, "A"))
            .unwrap()
            .id
            .unwrap();
        let b = create(&repo, new_task(version_id, "B"))
            .unwrap()
            .id
            .unwrap();
        let c = create(&repo, new_task(version_id, "C"))
            .unwrap()
            .id
            .unwrap();
        // B waits for A and C; nothing waits for B.
        repo.set_dependencies(b, &[a, c]).unwrap();
        let filter = TaskFilter {
            project_id,
            version_id: Some(version_id),
            status: None,
            priority: None,
            ..Default::default()
        };
        let page = page(&repo, &filter, 10, 0, false).unwrap();
        let by_name = |name: &str| page.tasks.iter().find(|t| t.name == name).unwrap();
        assert_eq!(
            (by_name("B").depends_on_count, by_name("B").blocks_count),
            (2, 0)
        );
        assert_eq!(
            (by_name("A").depends_on_count, by_name("A").blocks_count),
            (0, 1)
        );
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
    use super::{PageMeta, TaskEdit, TaskPage};
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
            meta: PageMeta {
                total: 0,
                limit: 10,
                offset: 0,
                has_more: false,
            },
        };
        let json = serde_json::to_string(&page).unwrap();
        let back: TaskPage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
    }
}

#[cfg(test)]
mod filter_tests {
    use super::{MAX_ID_FILTER, normalize_filter, readiness_from_flags};
    use crate::db::repository::{Readiness, TaskFilter};
    use crate::domain::error::AppError;

    fn filter() -> TaskFilter {
        TaskFilter {
            project_id: 1,
            ..Default::default()
        }
    }

    #[test]
    fn query_is_trimmed_and_blank_queries_are_rejected() {
        let trimmed = normalize_filter(&TaskFilter {
            text: Some("  api  ".to_string()),
            ..filter()
        })
        .unwrap();
        assert_eq!(trimmed.text.as_deref(), Some("api"));

        let blank = normalize_filter(&TaskFilter {
            text: Some("   ".to_string()),
            ..filter()
        });
        assert!(matches!(blank, Err(AppError::Validation(_))));
    }

    #[test]
    fn ids_filter_is_bounded() {
        let at_limit = normalize_filter(&TaskFilter {
            ids: Some((1..=MAX_ID_FILTER as i64).collect()),
            ..filter()
        });
        assert!(at_limit.is_ok());

        let over = normalize_filter(&TaskFilter {
            ids: Some((1..=MAX_ID_FILTER as i64 + 1).collect()),
            ..filter()
        });
        assert!(matches!(over, Err(AppError::Validation(_))));
    }

    #[test]
    fn time_bound_is_normalized_to_a_full_timestamp() {
        let normalized = normalize_filter(&TaskFilter {
            touched_since: Some("2026-03-04".to_string()),
            ..filter()
        })
        .unwrap();
        assert_eq!(
            normalized.touched_since.as_deref(),
            Some("2026-03-04 00:00:00")
        );

        let bad = normalize_filter(&TaskFilter {
            touched_since: Some("last week".to_string()),
            ..filter()
        });
        assert!(matches!(bad, Err(AppError::Validation(_))));
    }

    #[test]
    fn only_ready_and_only_blocked_cannot_be_combined() {
        assert_eq!(readiness_from_flags(false, false).unwrap(), None);
        assert_eq!(
            readiness_from_flags(true, false).unwrap(),
            Some(Readiness::Ready)
        );
        assert_eq!(
            readiness_from_flags(false, true).unwrap(),
            Some(Readiness::Blocked)
        );
        assert!(matches!(
            readiness_from_flags(true, true),
            Err(AppError::Validation(_))
        ));
    }
}
