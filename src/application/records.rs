// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Single-record views and guarded deletions (M1).
//!
//! The store deletes projects, versions and tasks in cascade: a version takes
//! its tasks, notes, dependency edges and summaries with it. The guarded
//! deletions here measure that impact first and refuse to proceed while it
//! would destroy active work or dependents, unless the caller confirms.

use serde::Serialize;

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Status, Task, TaskNote};
use crate::domain::task_description::{TaskSections, split_description};
use crate::domain::version::Version;

/// Notes included in a task view. The full history stays behind
/// `list_task_notes`.
pub const RECENT_NOTES: usize = 5;

/// Task counts by status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TaskCounts {
    pub todo: u32,
    pub in_progress: u32,
    pub done: u32,
}

impl TaskCounts {
    fn of(tasks: &[Task]) -> Self {
        let mut counts = Self::default();
        for task in tasks {
            match task.status {
                Status::Todo => counts.todo += 1,
                Status::InProgress => counts.in_progress += 1,
                Status::Done => counts.done += 1,
            }
        }
        counts
    }

    /// Tasks not yet done: the work a deletion would throw away.
    pub fn active(&self) -> u32 {
        self.todo + self.in_progress
    }

    fn total(&self) -> u32 {
        self.todo + self.in_progress + self.done
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectDetail {
    pub project: Project,
    pub versions: u32,
    pub tasks: TaskCounts,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionDetail {
    pub version: Version,
    pub tasks: TaskCounts,
    pub summaries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskDetail {
    pub task: Task,
    pub sections: TaskSections,
    pub recent_notes: Vec<TaskNote>,
    pub depends_on: Vec<i64>,
    pub blocks: Vec<i64>,
}

/// What a deletion removed, including everything dropped in cascade.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Removed {
    pub versions: u32,
    pub tasks: TaskCounts,
    pub summaries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Deleted {
    pub kind: &'static str,
    pub id: i64,
    pub removed: Removed,
}

pub fn project_detail(repo: &dyn Repository, project_id: i64) -> Result<ProjectDetail, AppError> {
    let project = repo.get_project(project_id)?;
    let versions = repo.list_versions_by_project(project_id)?;
    let tasks = repo.list_tasks_by_project(project_id)?;
    Ok(ProjectDetail {
        project,
        versions: versions.len() as u32,
        tasks: TaskCounts::of(&tasks),
    })
}

pub fn version_detail(repo: &dyn Repository, version_id: i64) -> Result<VersionDetail, AppError> {
    let version = repo.get_version(version_id)?;
    let tasks = repo.list_tasks_by_version(version_id)?;
    let summaries = repo.list_version_summaries(version_id)?;
    Ok(VersionDetail {
        version,
        tasks: TaskCounts::of(&tasks),
        summaries: summaries.len() as u32,
    })
}

pub fn task_detail(repo: &dyn Repository, task_id: i64) -> Result<TaskDetail, AppError> {
    let task = repo.get_task(task_id)?;
    let sections = split_description(&task.description);
    let recent_notes = repo
        .list_task_notes(task_id)?
        .into_iter()
        .take(RECENT_NOTES)
        .collect();
    let depends_on = repo.get_dependencies(task_id)?;
    let blocks = repo.get_dependents(task_id)?;
    Ok(TaskDetail {
        task,
        sections,
        recent_notes,
        depends_on,
        blocks,
    })
}

/// Refuses a destructive change unless the caller confirmed it.
fn guard(refuse: bool, blockers: Vec<i64>, message: String, confirm: bool) -> Result<(), AppError> {
    if refuse && !confirm {
        return Err(AppError::ConfirmationRequired { message, blockers });
    }
    Ok(())
}

fn ids(tasks: &[Task]) -> Vec<i64> {
    tasks.iter().filter_map(|task| task.id).collect()
}

pub fn delete_project(
    repo: &dyn Repository,
    project_id: i64,
    confirm: bool,
) -> Result<Deleted, AppError> {
    let project = repo.get_project(project_id)?;
    let versions = repo.list_versions_by_project(project_id)?;
    let tasks = repo.list_tasks_by_project(project_id)?;
    let mut summaries = 0u32;
    for version in &versions {
        if let Some(version_id) = version.id {
            summaries += repo.list_version_summaries(version_id)?.len() as u32;
        }
    }
    let counts = TaskCounts::of(&tasks);
    let active: Vec<Task> = tasks
        .iter()
        .filter(|task| task.status != Status::Done)
        .cloned()
        .collect();
    let message = format!(
        "project '{}' has {} active task(s) (todo {}, in progress {}). Deleting it removes {} task(s), {} version(s) and {} summary(ies). Retry with confirm=true to delete it anyway.",
        project.name,
        counts.active(),
        counts.todo,
        counts.in_progress,
        counts.total(),
        versions.len(),
        summaries,
    );
    guard(counts.active() > 0, ids(&active), message, confirm)?;
    repo.delete_project(project_id)?;
    Ok(Deleted {
        kind: "project",
        id: project_id,
        removed: Removed {
            versions: versions.len() as u32,
            tasks: counts,
            summaries,
        },
    })
}

pub fn delete_version(
    repo: &dyn Repository,
    version_id: i64,
    confirm: bool,
) -> Result<Deleted, AppError> {
    let version = repo.get_version(version_id)?;
    let tasks = repo.list_tasks_by_version(version_id)?;
    let summaries = repo.list_version_summaries(version_id)?.len() as u32;
    let counts = TaskCounts::of(&tasks);
    let active: Vec<Task> = tasks
        .iter()
        .filter(|task| task.status != Status::Done)
        .cloned()
        .collect();
    let message = format!(
        "version '{}' has {} active task(s) (todo {}, in progress {}). Deleting it removes {} task(s) and {} summary(ies). Retry with confirm=true to delete it anyway.",
        version.name,
        counts.active(),
        counts.todo,
        counts.in_progress,
        counts.total(),
        summaries,
    );
    guard(counts.active() > 0, ids(&active), message, confirm)?;
    repo.delete_version(version_id)?;
    Ok(Deleted {
        kind: "version",
        id: version_id,
        removed: Removed {
            versions: 1,
            tasks: counts,
            summaries,
        },
    })
}

pub fn delete_task(
    repo: &dyn Repository,
    task_id: i64,
    confirm: bool,
) -> Result<Deleted, AppError> {
    let task = repo.get_task(task_id)?;
    let dependents = repo.get_dependents(task_id)?;
    let in_progress = task.status == Status::InProgress;
    let message = if dependents.is_empty() {
        format!("task {task_id} is in progress. Retry with confirm=true to delete it anyway.")
    } else {
        format!(
            "task {task_id} is a dependency of {} task(s) ({}). Deleting it removes those links and may unblock them. Retry with confirm=true to delete it anyway.",
            dependents.len(),
            dependents
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        )
    };
    guard(
        in_progress || !dependents.is_empty(),
        dependents,
        message,
        confirm,
    )?;
    repo.delete_task(task_id)?;
    let counts = TaskCounts {
        todo: u32::from(task.status == Status::Todo),
        in_progress: u32::from(in_progress),
        done: u32::from(task.status == Status::Done),
    };
    Ok(Deleted {
        kind: "task",
        id: task_id,
        removed: Removed {
            versions: 0,
            tasks: counts,
            summaries: 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{
        DependencyRepository, NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
        VersionRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::error::AppError;
    use crate::domain::task::{Priority, Status};

    use super::{
        delete_project, delete_task, delete_version, project_detail, task_detail, version_detail,
    };

    fn repo_with_project() -> (StubRepository, i64, i64) {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        (repo, project_id, version_id)
    }

    fn task(repo: &StubRepository, project_id: i64, version_id: i64, status: Status) -> i64 {
        repo.create_task(project_id, version_id, "T", "", status, Priority::Medium)
            .unwrap()
            .id
            .unwrap()
    }

    #[test]
    fn project_detail_counts_versions_and_tasks_by_status() {
        let (repo, pid, vid) = repo_with_project();
        task(&repo, pid, vid, Status::Todo);
        task(&repo, pid, vid, Status::Done);
        let detail = project_detail(&repo, pid).unwrap();
        assert_eq!(detail.versions, 1);
        assert_eq!(detail.tasks.todo, 1);
        assert_eq!(detail.tasks.done, 1);
        assert_eq!(detail.tasks.active(), 1);
    }

    #[test]
    fn version_detail_reports_its_own_counts() {
        let (repo, pid, vid) = repo_with_project();
        task(&repo, pid, vid, Status::InProgress);
        let detail = version_detail(&repo, vid).unwrap();
        assert_eq!(detail.tasks.in_progress, 1);
        assert_eq!(detail.summaries, 0);
    }

    #[test]
    fn task_detail_splits_sections_and_keeps_recent_notes() {
        let (repo, pid, vid) = repo_with_project();
        let id = repo
            .create_task(
                pid,
                vid,
                "T",
                "## Context\n\nctx\n\n## Requirements\n\nreq\n\n## Benefits\n\nben",
                Status::Todo,
                Priority::Low,
            )
            .unwrap()
            .id
            .unwrap();
        for n in 0..7 {
            repo.add_task_note(id, &format!("note {n}")).unwrap();
        }
        let detail = task_detail(&repo, id).unwrap();
        assert_eq!(detail.sections.context, "ctx");
        assert_eq!(detail.sections.requirements, "req");
        assert_eq!(detail.sections.benefits, "ben");
        assert_eq!(detail.recent_notes.len(), super::RECENT_NOTES);
    }

    #[test]
    fn views_of_unknown_records_are_not_found() {
        let (repo, _, _) = repo_with_project();
        assert!(matches!(
            project_detail(&repo, 999),
            Err(AppError::NotFound)
        ));
        assert!(matches!(
            version_detail(&repo, 999),
            Err(AppError::NotFound)
        ));
        assert!(matches!(task_detail(&repo, 999), Err(AppError::NotFound)));
    }

    #[test]
    fn delete_version_with_active_tasks_needs_confirmation_and_keeps_data() {
        let (repo, pid, vid) = repo_with_project();
        let active = task(&repo, pid, vid, Status::Todo);
        task(&repo, pid, vid, Status::Done);
        match delete_version(&repo, vid, false) {
            Err(AppError::ConfirmationRequired { blockers, message }) => {
                assert_eq!(blockers, vec![active]);
                assert!(message.contains("confirm=true"));
            }
            other => panic!("expected confirmation_required, got {other:?}"),
        }
        assert_eq!(repo.list_tasks_by_version(vid).unwrap().len(), 2);
    }

    #[test]
    fn delete_version_with_only_done_tasks_needs_no_confirmation_and_reports_removal() {
        let (repo, pid, vid) = repo_with_project();
        task(&repo, pid, vid, Status::Done);
        task(&repo, pid, vid, Status::Done);
        let deleted = delete_version(&repo, vid, false).unwrap();
        assert_eq!(deleted.kind, "version");
        assert_eq!(deleted.removed.tasks.done, 2);
        assert!(repo.get_version(vid).is_err());
    }

    #[test]
    fn delete_version_with_confirmation_removes_active_tasks() {
        let (repo, pid, vid) = repo_with_project();
        let active = task(&repo, pid, vid, Status::InProgress);
        let deleted = delete_version(&repo, vid, true).unwrap();
        assert_eq!(deleted.removed.tasks.in_progress, 1);
        assert!(repo.get_task(active).is_err());
    }

    #[test]
    fn delete_unknown_version_is_not_found() {
        let (repo, _, _) = repo_with_project();
        assert!(matches!(
            delete_version(&repo, 999, true),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn delete_project_blocks_on_active_work_in_any_version() {
        let (repo, pid, vid) = repo_with_project();
        let other = repo.create_version(pid, "1.0.0", "").unwrap().id.unwrap();
        task(&repo, pid, vid, Status::Done);
        let active = task(&repo, pid, other, Status::Todo);
        match delete_project(&repo, pid, false) {
            Err(AppError::ConfirmationRequired { blockers, .. }) => {
                assert_eq!(blockers, vec![active]);
            }
            other => panic!("expected confirmation_required, got {other:?}"),
        }
        assert!(repo.get_project(pid).is_ok());
    }

    #[test]
    fn delete_project_with_confirmation_reports_everything_removed() {
        let (repo, pid, vid) = repo_with_project();
        task(&repo, pid, vid, Status::Todo);
        task(&repo, pid, vid, Status::Done);
        let deleted = delete_project(&repo, pid, true).unwrap();
        assert_eq!(deleted.removed.tasks.total(), 2);
        assert_eq!(deleted.removed.versions, 1);
        assert!(repo.get_project(pid).is_err());
    }

    #[test]
    fn delete_task_with_dependents_needs_confirmation() {
        let (repo, pid, vid) = repo_with_project();
        let dependency = task(&repo, pid, vid, Status::Todo);
        let dependent = task(&repo, pid, vid, Status::Todo);
        repo.add_dependency(dependent, dependency).unwrap();
        match delete_task(&repo, dependency, false) {
            Err(AppError::ConfirmationRequired { blockers, .. }) => {
                assert_eq!(blockers, vec![dependent]);
            }
            other => panic!("expected confirmation_required, got {other:?}"),
        }
        assert!(repo.get_task(dependency).is_ok());
        assert!(delete_task(&repo, dependency, true).is_ok());
    }

    #[test]
    fn delete_task_in_progress_needs_confirmation_but_free_todo_does_not() {
        let (repo, pid, vid) = repo_with_project();
        let working = task(&repo, pid, vid, Status::InProgress);
        assert!(matches!(
            delete_task(&repo, working, false),
            Err(AppError::ConfirmationRequired { .. })
        ));
        let free = task(&repo, pid, vid, Status::Todo);
        assert_eq!(
            delete_task(&repo, free, false).unwrap().removed.tasks.todo,
            1
        );
    }

    #[test]
    fn delete_unknown_task_and_project_are_not_found() {
        let (repo, _, _) = repo_with_project();
        assert!(matches!(
            delete_task(&repo, 999, true),
            Err(AppError::NotFound)
        ));
        assert!(matches!(
            delete_project(&repo, 999, true),
            Err(AppError::NotFound)
        ));
    }
}
