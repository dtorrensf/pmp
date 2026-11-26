// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sending a task to an ACP agent: validation, prompt and closing note.
//!
//! The TUI and, from C8, the headless CLI share this use case. It owns the
//! rules that must hold *before* an agent is launched and the note that is
//! written *after* the turn ends; the ACP transport itself stays in
//! [`crate::acp`], so this module never talks to a subprocess.

use std::path::PathBuf;

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::{Status, Task, TaskNote};

use super::dependencies;
use super::notes;
use super::prompts;
use super::records;
use super::tasks;

/// Everything the TUI and the CLI need to launch one implementation run.
///
/// It is produced only after the task and the project pass every check, so a
/// caller never launches an agent for work that cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRun {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub version_id: i64,
    /// Existing absolute directory the agent is launched in.
    pub working_dir: PathBuf,
    /// The shared M7 implementation prompt.
    pub prompt: String,
}

/// Validates and prepares a run for `task_id`.
///
/// The task must not be done, every dependency must be done and the project
/// must have an existing `working_dir`. The prompt comes from
/// [`prompts::implement_task`], the same template the MCP `implement-task`
/// prompt renders, so both transports send identical instructions.
pub fn prepare(repo: &dyn Repository, task_id: i64) -> Result<PreparedRun, AppError> {
    let detail = records::task_detail(repo, task_id)?;
    if detail.task.status == Status::Done {
        return Err(AppError::Validation(format!(
            "task #{task_id} is already done; reopen it before sending it to an agent"
        )));
    }

    let dependencies = dependencies::view(repo, task_id)?;
    let blockers: Vec<i64> = dependencies
        .blocked_by
        .iter()
        .filter(|blocker| blocker.status != Status::Done)
        .map(|blocker| blocker.id)
        .collect();
    if !blockers.is_empty() {
        let ids = blockers
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::DependencyBlocked {
            message: format!("task #{task_id} has incomplete dependencies: {ids}"),
            blockers,
        });
    }

    let project = repo.get_project(detail.task.project_id)?;
    let working_dir = project.working_dir.clone().ok_or_else(|| {
        AppError::Validation(format!(
            "project '{}' has no working directory; set one before launching an agent",
            project.name
        ))
    })?;
    let working_dir = PathBuf::from(working_dir);
    if !working_dir.is_dir() {
        return Err(AppError::Validation(format!(
            "working directory '{}' does not exist or is not a directory",
            working_dir.display()
        )));
    }

    Ok(PreparedRun {
        task_id,
        task_name: detail.task.name.clone(),
        project_id: detail.task.project_id,
        version_id: detail.task.version_id,
        working_dir,
        prompt: prompts::implement_task(&detail, &dependencies),
    })
}

/// Moves the task to `InProgress` right before the prompt is sent.
///
/// Already working tasks are left untouched, so re-sending a task to another
/// agent does not rewrite its start state. A done task is refused: `prepare`
/// rejects it, and doing it here too keeps the rule true even if a caller
/// skipped the preparation.
pub fn mark_started(repo: &dyn Repository, task_id: i64) -> Result<Task, AppError> {
    let task = repo.get_task(task_id)?;
    match task.status {
        Status::InProgress => Ok(task),
        Status::Done => Err(AppError::Validation(format!(
            "task #{task_id} is already done; reopen it before sending it to an agent"
        ))),
        Status::Todo => tasks::set_status(repo, task_id, Status::InProgress),
    }
}

/// Records the outcome of a run as a task note.
///
/// The final verification stays with the user, so this never changes the task
/// status: a successful turn leaves it `InProgress` until the user marks it
/// done.
pub fn record_result(
    repo: &dyn Repository,
    task_id: i64,
    summary: &str,
) -> Result<TaskNote, AppError> {
    if summary.trim().is_empty() {
        return Err(AppError::Validation(
            "agent run summary cannot be empty".to_string(),
        ));
    }
    notes::add(repo, task_id, summary)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{
        DependencyRepository, NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::error::AppError;
    use crate::domain::task::{Priority, Status};

    use super::{mark_started, prepare, record_result};

    fn project_with_working_dir() -> (StubRepository, tempfile::TempDir, i64, i64) {
        let repo = StubRepository::default();
        let workspace = tempfile::tempdir().unwrap();
        let project = repo
            .create_project("Alpha", "", Some(workspace.path().to_str().unwrap()))
            .unwrap();
        let project_id = project.id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        (repo, workspace, project_id, version_id)
    }

    fn task(repo: &StubRepository, project_id: i64, version_id: i64) -> i64 {
        repo.create_task(
            project_id,
            version_id,
            "Implement C3",
            "## Context\n\nctx\n\n## Requirements\n\nreq\n\n## Benefits\n\nben",
            Status::Todo,
            Priority::High,
        )
        .unwrap()
        .id
        .unwrap()
    }

    #[test]
    fn prepare_resolves_the_working_dir_and_the_shared_prompt() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let task_id = task(&repo, project_id, version_id);

        let prepared = prepare(&repo, task_id).unwrap();

        assert_eq!(prepared.task_id, task_id);
        assert_eq!(prepared.task_name, "Implement C3");
        assert_eq!(prepared.project_id, project_id);
        assert_eq!(prepared.version_id, version_id);
        assert!(prepared.working_dir.is_dir());
        let detail = super::super::records::task_detail(&repo, task_id).unwrap();
        let dependencies = super::super::dependencies::view(&repo, task_id).unwrap();
        assert_eq!(
            prepared.prompt,
            super::super::prompts::implement_task(&detail, &dependencies)
        );
        assert!(prepared.prompt.contains("Implement C3"));
        assert!(prepared.prompt.contains("## Requirements"));
    }

    #[test]
    fn prepare_refuses_incomplete_dependencies() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let blocker = task(&repo, project_id, version_id);
        let task_id = repo
            .create_task(
                project_id,
                version_id,
                "Blocked",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap()
            .id
            .unwrap();
        repo.add_dependency(task_id, blocker).unwrap();

        match prepare(&repo, task_id) {
            Err(AppError::DependencyBlocked {
                blockers, message, ..
            }) => {
                assert_eq!(blockers, vec![blocker]);
                assert!(message.contains(&format!("#{blocker}")));
            }
            other => panic!("expected dependency_blocked, got {other:?}"),
        }
    }

    #[test]
    fn prepare_ignores_blockers_that_are_done() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let blocker = task(&repo, project_id, version_id);
        repo.update_task_status(blocker, Status::Done).unwrap();
        let task_id = task(&repo, project_id, version_id);
        repo.add_dependency(task_id, blocker).unwrap();

        assert!(prepare(&repo, task_id).is_ok());
    }

    #[test]
    fn prepare_refuses_a_done_task() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let task_id = task(&repo, project_id, version_id);
        repo.update_task_status(task_id, Status::Done).unwrap();

        assert!(matches!(
            prepare(&repo, task_id),
            Err(AppError::Validation(message)) if message.contains("already done")
        ));
    }

    #[test]
    fn prepare_requires_an_existing_working_directory() {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
        let version_id = repo.default_version_id(project_id).unwrap();
        let task_id = task(&repo, project_id, version_id);

        assert!(matches!(
            prepare(&repo, task_id),
            Err(AppError::Validation(message)) if message.contains("no working directory")
        ));

        let missing = repo
            .create_project("Beta", "", Some("/definitely/not/a/pmp/workspace"))
            .unwrap()
            .id
            .unwrap();
        let version = repo.default_version_id(missing).unwrap();
        let missing_task = task(&repo, missing, version);
        assert!(matches!(
            prepare(&repo, missing_task),
            Err(AppError::Validation(message)) if message.contains("does not exist")
        ));
    }

    #[test]
    fn mark_started_moves_todo_to_in_progress_and_is_idempotent() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let task_id = task(&repo, project_id, version_id);

        assert_eq!(
            mark_started(&repo, task_id).unwrap().status,
            Status::InProgress
        );
        assert_eq!(
            mark_started(&repo, task_id).unwrap().status,
            Status::InProgress
        );
        assert_eq!(repo.get_task(task_id).unwrap().status, Status::InProgress);
    }

    #[test]
    fn record_result_adds_a_note_without_marking_the_task_done() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let task_id = task(&repo, project_id, version_id);
        mark_started(&repo, task_id).unwrap();

        record_result(&repo, task_id, "Agent run finished (end_turn).\n\nDone.").unwrap();

        assert_eq!(repo.get_task(task_id).unwrap().status, Status::InProgress);
        let notes = repo.list_task_notes(task_id).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].content.contains("Agent run finished"));
    }

    #[test]
    fn blank_results_are_rejected() {
        let (repo, _workspace, project_id, version_id) = project_with_working_dir();
        let task_id = task(&repo, project_id, version_id);

        assert!(matches!(
            record_result(&repo, task_id, "   "),
            Err(AppError::Validation(_))
        ));
        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }
}
