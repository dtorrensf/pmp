// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Business rules for versions and compaction. The rules read through narrow
//! query ports instead of the persistence trait, so they do not depend on
//! storage and each can be tested against a small fake.

use crate::domain::error::AppError;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::task::{Status, Task, TaskDependency, TaskNote};
use crate::domain::version::{Version, VersionStatus, next_version_name};
use crate::domain::version_summary::VersionSummary;

/// Read access to the versions of a project.
pub trait VersionQueries {
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError>;
}

/// Read access needed to check and freeze a version during compaction.
pub trait CompactionQueries: VersionQueries {
    fn get_version(&self, id: i64) -> Result<Version, AppError>;
    fn list_version_summaries(&self, version_id: i64) -> Result<Vec<VersionSummary>, AppError>;
    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError>;
    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError>;
    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError>;
}

/// Version rules: the suggested name of a new version and the version that
/// receives tasks by default.
pub struct VersionPolicy<'a, Q: VersionQueries + ?Sized> {
    versions: &'a Q,
}

impl<'a, Q: VersionQueries + ?Sized> VersionPolicy<'a, Q> {
    pub fn new(versions: &'a Q) -> Self {
        Self { versions }
    }

    /// Suggests the next semver version name for the project (see
    /// `domain::version::next_version_name`).
    pub fn suggest_name(&self, project_id: i64) -> Result<String, AppError> {
        let versions = self.versions.list_versions_by_project(project_id)?;
        Ok(next_version_name(versions.iter().map(|v| v.name.as_str())))
    }

    /// The version that receives tasks when none is chosen: the newest open
    /// version, or the newest version of any status when none is open. "Newest"
    /// means the highest id. The result does not depend on the order in which
    /// the port returns versions.
    pub fn default_version_id(&self, project_id: i64) -> Result<i64, AppError> {
        let versions = self.versions.list_versions_by_project(project_id)?;
        let newest = |candidates: Vec<&Version>| candidates.into_iter().filter_map(|v| v.id).max();
        let open: Vec<&Version> = versions
            .iter()
            .filter(|v| v.status == VersionStatus::Open)
            .collect();
        let all: Vec<&Version> = versions.iter().collect();
        newest(open)
            .or_else(|| newest(all))
            .ok_or(AppError::NotFound)
    }
}

/// Compaction rules: when a version may be compacted and how its frozen
/// snapshot is built.
pub struct CompactService<'a, Q: CompactionQueries + ?Sized> {
    queries: &'a Q,
}

impl<'a, Q: CompactionQueries + ?Sized> CompactService<'a, Q> {
    pub fn new(queries: &'a Q) -> Self {
        Self { queries }
    }

    /// Validates the preconditions for compacting a version. Compaction is
    /// rejected while any task is `InProgress`. It also requires something to
    /// freeze: at least one `Done` task, or an existing previous summary (so a
    /// version with only `Todo` tasks and no summary is rejected).
    pub fn can_compact(&self, version_id: i64) -> Result<(), AppError> {
        let tasks = self.queries.list_tasks_by_version(version_id)?;
        if tasks.iter().any(|t| t.status == Status::InProgress) {
            return Err(AppError::Validation(
                "version has tasks in progress; finish or pause them before compacting".to_string(),
            ));
        }
        let has_done = tasks.iter().any(|t| t.status == Status::Done);
        if !has_done && self.queries.list_version_summaries(version_id)?.is_empty() {
            return Err(AppError::Validation(
                "nothing to compact: the version has no done tasks and no previous summary"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Builds the frozen JSON snapshot used on compaction: the version, its
    /// previous summary (when recompacting), and the `Done` tasks with their
    /// notes and dependencies. `Todo`/`InProgress` tasks are never included.
    /// `previous_summary.snapshot_json` recursively embeds the prior snapshot,
    /// preserving history.
    pub fn build_snapshot(&self, version_id: i64) -> Result<String, AppError> {
        let version = self.queries.get_version(version_id)?;
        let previous_summary = self
            .queries
            .list_version_summaries(version_id)?
            .into_iter()
            .next();
        let tasks: Vec<Task> = self
            .queries
            .list_tasks_by_version(version_id)?
            .into_iter()
            .filter(|t| t.status == Status::Done)
            .collect();
        let mut notes = Vec::new();
        let mut dependencies = Vec::new();
        for task in &tasks {
            if let Some(id) = task.id {
                notes.extend(self.queries.list_task_notes(id)?);
                dependencies.extend(self.queries.get_dependencies(id)?.into_iter().map(
                    |depends_on_id| TaskDependency {
                        task_id: id,
                        depends_on_id,
                    },
                ));
            }
        }
        let snapshot = CompactSnapshot {
            version,
            previous_summary,
            tasks,
            notes,
            dependencies,
        };
        serde_json::to_string(&snapshot)
            .map_err(|e| AppError::Validation(format!("snapshot serialization error: {e}")))
    }
}
