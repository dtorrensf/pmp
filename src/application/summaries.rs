// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Compaction and version summary use cases.
//!
//! Compaction is the one operation that spans several aggregates: it freezes
//! the done tasks of a version, stores the summary and moves the version to its
//! next status. Doing it here keeps the TUI, MCP and the future ACP client from
//! reimplementing that sequence.

use std::path::PathBuf;

use crate::db::repository::{Repository, RepositoryPolicy};
use crate::domain::error::AppError;
use crate::domain::validation::validate_text;
use crate::domain::version::VersionStatus;
use crate::domain::version_summary::VersionSummary;

pub fn list(repo: &dyn Repository, version_id: i64) -> Result<Vec<VersionSummary>, AppError> {
    repo.get_version(version_id)?;
    repo.list_version_summaries(version_id)
}

pub fn get(repo: &dyn Repository, summary_id: i64) -> Result<VersionSummary, AppError> {
    repo.get_version_summary(summary_id)
}

/// Reads one summary, restricted to the version it belongs to.
pub fn read(
    repo: &dyn Repository,
    version_id: i64,
    summary_id: i64,
) -> Result<VersionSummary, AppError> {
    let summary = get(repo, summary_id)?;
    if summary.version_id != version_id {
        return Err(AppError::NotFound);
    }
    Ok(summary)
}

/// The JSON snapshot `compact` would freeze, without changing anything.
pub fn preview(repo: &dyn Repository, version_id: i64) -> Result<serde_json::Value, AppError> {
    repo.get_version(version_id)?;
    let snapshot = repo.build_compact_snapshot(version_id)?;
    serde_json::from_str(&snapshot)
        .map_err(|e| AppError::Validation(format!("snapshot parse error: {e}")))
}

/// Compacts a version into `summary` and applies the version's next status.
///
/// The store replaces the previous summary, stores the new one and deletes the
/// done tasks in one transaction. Afterwards the version stays open while todo
/// tasks remain; otherwise it is released unless `mark_released` is false.
pub fn compact(
    repo: &dyn Repository,
    version_id: i64,
    summary: &str,
    mark_released: bool,
) -> Result<VersionSummary, AppError> {
    validate_text("version summary", "summary", summary)?;
    repo.can_compact(version_id)?;
    let snapshot = repo.build_compact_snapshot(version_id)?;
    let created = repo.compact_version(version_id, summary, &snapshot)?;

    let has_remaining_tasks = !repo.list_tasks_by_version(version_id)?.is_empty();
    let target_status = if has_remaining_tasks {
        Some(VersionStatus::Open)
    } else if mark_released {
        Some(VersionStatus::Released)
    } else {
        None
    };
    if let Some(status) = target_status {
        let version = repo.get_version(version_id)?;
        if version.status != status {
            repo.update_version(version_id, &version.name, &version.description, status)?;
        }
    }
    Ok(created)
}

pub fn delete(repo: &dyn Repository, version_id: i64, summary_id: i64) -> Result<(), AppError> {
    read(repo, version_id, summary_id)?;
    repo.delete_version_summary(summary_id)
}

/// Writes the Markdown export of a summary inside the app's exports folder.
pub fn export_markdown(repo: &dyn Repository, summary_id: i64) -> Result<PathBuf, AppError> {
    super::export::export_version_summary_markdown(repo, summary_id)
}

#[cfg(test)]
mod tests {
    use crate::db::repository::{ProjectRepository, TaskRepository, VersionRepository};
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;

    use super::{compact, delete, list, preview, read};

    struct Fixture {
        repo: StubRepository,
        version_id: i64,
    }

    fn fixture(open_tasks: &[Status]) -> Fixture {
        let repo = StubRepository::default();
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let version_id = repo
            .create_version(project_id, "1.0.0", "")
            .unwrap()
            .id
            .unwrap();
        repo.update_version(version_id, "1.0.0", "", VersionStatus::Open)
            .unwrap();
        for (index, status) in open_tasks.iter().enumerate() {
            repo.create_task(
                project_id,
                version_id,
                &format!("T{index}"),
                "",
                *status,
                Priority::Medium,
            )
            .unwrap();
        }
        Fixture { repo, version_id }
    }

    #[test]
    fn compact_releases_the_version_when_nothing_remains() {
        let f = fixture(&[Status::Done]);
        let summary = compact(&f.repo, f.version_id, "done", true).unwrap();
        assert_eq!(summary.version_id, f.version_id);
        assert_eq!(
            f.repo.get_version(f.version_id).unwrap().status,
            VersionStatus::Released
        );
        assert_eq!(list(&f.repo, f.version_id).unwrap().len(), 1);
    }

    #[test]
    fn compact_keeps_the_version_open_while_todo_tasks_remain() {
        let f = fixture(&[Status::Done, Status::Todo]);
        compact(&f.repo, f.version_id, "partial", true).unwrap();
        assert_eq!(
            f.repo.get_version(f.version_id).unwrap().status,
            VersionStatus::Open
        );
    }

    #[test]
    fn compact_honours_mark_released_false() {
        let f = fixture(&[Status::Done]);
        compact(&f.repo, f.version_id, "keep open", false).unwrap();
        assert_eq!(
            f.repo.get_version(f.version_id).unwrap().status,
            VersionStatus::Open
        );
    }

    #[test]
    fn compact_rejects_a_version_with_work_in_progress() {
        let f = fixture(&[Status::InProgress]);
        assert!(compact(&f.repo, f.version_id, "nope", true).is_err());
    }

    #[test]
    fn preview_does_not_change_anything() {
        let f = fixture(&[Status::Done]);
        let preview = preview(&f.repo, f.version_id).unwrap();
        assert!(preview.get("tasks").is_some());
        assert_eq!(list(&f.repo, f.version_id).unwrap().len(), 0);
        assert_eq!(
            f.repo.get_version(f.version_id).unwrap().status,
            VersionStatus::Open
        );
    }

    #[test]
    fn read_and_delete_are_scoped_to_the_version() {
        let f = fixture(&[Status::Done]);
        let summary = compact(&f.repo, f.version_id, "done", true).unwrap();
        assert!(read(&f.repo, f.version_id + 99, summary.id.unwrap()).is_err());
        assert!(read(&f.repo, f.version_id, summary.id.unwrap()).is_ok());
        delete(&f.repo, f.version_id, summary.id.unwrap()).unwrap();
        assert!(list(&f.repo, f.version_id).unwrap().is_empty());
    }

    #[test]
    fn compact_requires_an_existing_version() {
        let f = fixture(&[]);
        assert!(compact(&f.repo, 999, "x", true).is_err());
    }
}
