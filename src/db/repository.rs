// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};

use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::task::{Priority, Status, Task, TaskDependency, TaskNote};
use crate::domain::version::{Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;

pub trait Repository: Send + Sync {
    fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError>;
    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError>;
    fn get_project(&self, id: i64) -> Result<Project, AppError>;
    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError>;
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError>;
    fn create_version(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
    ) -> Result<Version, AppError>;
    fn get_version(&self, id: i64) -> Result<Version, AppError>;
    fn update_version(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: VersionStatus,
    ) -> Result<Version, AppError>;
    fn delete_version(&self, id: i64) -> Result<(), AppError>;
    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError>;
    /// Suggests the next semver version name for the project (see
    /// `domain::version::next_version_name`).
    fn next_version_name(&self, project_id: i64) -> Result<String, AppError> {
        let versions = self.list_versions_by_project(project_id)?;
        Ok(crate::domain::version::next_version_name(
            versions.iter().map(|v| v.name.as_str()),
        ))
    }
    /// Version that receives tasks when none is chosen explicitly: the newest
    /// open version, falling back to the newest version of any status.
    fn default_version_id(&self, project_id: i64) -> Result<i64, AppError> {
        self.list_versions_by_project(project_id)?
            .first()
            .and_then(|v| v.id)
            .ok_or(AppError::NotFound)
    }
    fn create_task(
        &self,
        project_id: i64,
        version_id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError>;
    fn get_task(&self, id: i64) -> Result<Task, AppError>;
    fn update_task(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError>;
    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError>;
    fn update_task_priority(&self, id: i64, priority: Priority) -> Result<Task, AppError>;
    /// Reassigns a task to another version of its own project. Only `version_id`
    /// changes. Moving to the current version is a no-op. Tasks that have
    /// dependencies or dependents are rejected: those edges must stay within
    /// one version, so they have to be removed first.
    fn move_task(&self, task_id: i64, version_id: i64) -> Result<Task, AppError>;
    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError>;
    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError>;
    /// Replaces the full set of tasks that `task_id` depends on, atomically.
    /// Validates the whole set before writing; on error nothing changes.
    fn set_dependencies(&self, task_id: i64, depends_on_ids: &[i64]) -> Result<(), AppError>;
    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError>;
    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError>;
    fn delete_project(&self, id: i64) -> Result<(), AppError>;
    fn delete_task(&self, id: i64) -> Result<(), AppError>;
    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError>;
    fn would_create_cycle(&self, task_id: i64, candidate_id: i64) -> Result<bool, AppError>;
    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError>;
    fn add_task_note(&self, task_id: i64, content: &str) -> Result<TaskNote, AppError>;
    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError>;
    fn list_version_summaries(&self, version_id: i64) -> Result<Vec<VersionSummary>, AppError>;
    fn get_version_summary(&self, id: i64) -> Result<VersionSummary, AppError>;
    fn create_version_summary(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError>;
    fn delete_version_summary(&self, id: i64) -> Result<(), AppError>;
    /// Atomically compacts a version: replaces its previous summary (if any),
    /// stores the new one and deletes its `Done` tasks. Either all of these
    /// writes are applied or none is; on error the version is left unchanged.
    fn compact_version(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError>;
    /// Deletes only the `Done` tasks of a version in bulk. Dependencies and
    /// notes of the removed tasks are cascade-deleted, including edges from
    /// `Todo` tasks that depended on them. Tasks in other states are kept.
    fn delete_done_tasks_by_version(&self, version_id: i64) -> Result<(), AppError>;

    /// Builds the frozen JSON snapshot used on compaction: the version, its
    /// previous summary (when recompacting), and the `Done` tasks with their
    /// notes and dependencies. `Todo`/`InProgress` tasks are never included.
    /// `previous_summary.snapshot_json` recursively embeds the prior snapshot,
    /// preserving history.
    fn build_compact_snapshot(&self, version_id: i64) -> Result<String, AppError> {
        let version = self.get_version(version_id)?;
        let previous_summary = self.list_version_summaries(version_id)?.into_iter().next();
        let tasks: Vec<Task> = self
            .list_tasks_by_version(version_id)?
            .into_iter()
            .filter(|t| t.status == Status::Done)
            .collect();
        let mut notes = Vec::new();
        let mut dependencies = Vec::new();
        for task in &tasks {
            if let Some(id) = task.id {
                notes.extend(self.list_task_notes(id)?);
                dependencies.extend(self.get_dependencies(id)?.into_iter().map(|depends_on_id| {
                    TaskDependency {
                        task_id: id,
                        depends_on_id,
                    }
                }));
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

    /// Validates the preconditions for compacting a version. Compaction is
    /// rejected while any task is `InProgress`. It also requires something to
    /// freeze: at least one `Done` task, or an existing previous summary (so
    /// a version with only `Todo` tasks and no summary is rejected).
    fn can_compact(&self, version_id: i64) -> Result<(), AppError> {
        let tasks = self.list_tasks_by_version(version_id)?;
        if tasks.iter().any(|t| t.status == Status::InProgress) {
            return Err(AppError::Validation(
                "version has tasks in progress; finish or pause them before compacting".to_string(),
            ));
        }
        let has_done = tasks.iter().any(|t| t.status == Status::Done);
        if !has_done && self.list_version_summaries(version_id)?.is_empty() {
            return Err(AppError::Validation(
                "nothing to compact: the version has no done tasks and no previous summary"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// Validates that `task_id` may depend on exactly `depends_on_ids`, given the
/// current dependency `graph` (task id -> ids it depends on). The current edges
/// of `task_id` are replaced by the new set, so only cycles created by the new
/// set are reported. Tasks are not checked for existence or version here.
pub(crate) fn check_dependency_set(
    graph: &HashMap<i64, Vec<i64>>,
    task_id: i64,
    depends_on_ids: &[i64],
) -> Result<(), AppError> {
    let mut edges = graph.clone();
    edges.remove(&task_id);
    for &dep in depends_on_ids {
        if dep == task_id || reaches(&edges, dep, task_id) {
            return Err(AppError::DependencyCycle);
        }
        edges.entry(task_id).or_default().push(dep);
    }
    Ok(())
}

/// Returns true if `target` is reachable from `from` following dependency edges.
fn reaches(graph: &HashMap<i64, Vec<i64>>, from: i64, target: i64) -> bool {
    let mut visited = HashSet::new();
    let mut stack = vec![from];
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(next) = graph.get(&current) {
            stack.extend(next);
        }
    }
    false
}

/// The rule for moving a task to `next_status`: a task cannot enter Done while
/// it has incomplete dependencies. Done to Done is a no-op and always allowed,
/// so a task finished before a new blocker was added keeps its status.
/// `has_incomplete_dependencies` is only called when the move enters Done.
/// Every `update_task` and `update_task_status` implementation calls this, so
/// the TUI and the MCP server get the same behaviour.
pub(crate) fn check_done_transition(
    current_status: Status,
    next_status: Status,
    has_incomplete_dependencies: impl FnOnce() -> Result<bool, AppError>,
) -> Result<(), AppError> {
    if next_status != Status::Done || current_status == Status::Done {
        return Ok(());
    }
    if has_incomplete_dependencies()? {
        return Err(AppError::Validation(
            "cannot mark task as done: it has incomplete dependencies".to_string(),
        ));
    }
    Ok(())
}

/// Validates a task move to `target` (already loaded, so it exists). The
/// caller handles the no-op case where the task is already in `target`.
pub(crate) fn check_task_move(
    task: &Task,
    target: &Version,
    dependencies: &[i64],
    dependents: &[i64],
) -> Result<(), AppError> {
    if target.project_id != task.project_id {
        return Err(AppError::Validation(
            "version does not belong to the project".to_string(),
        ));
    }
    if !dependencies.is_empty() || !dependents.is_empty() {
        return Err(AppError::Validation(
            "task has dependencies or dependents; remove them before moving it, \
             because they must stay within the same version"
                .to_string(),
        ));
    }
    Ok(())
}

/// Removes duplicate ids while keeping the first occurrence order.
pub(crate) fn unique_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Repository, check_done_transition};
    use crate::domain::error::AppError;
    use crate::domain::task::Status;

    #[test]
    fn repository_trait_is_object_safe() {
        let _: Option<Arc<dyn Repository>> = None;
    }

    #[test]
    fn done_transition_rule_covers_every_case() {
        let blocked = || Ok(true);
        let clear = || Ok(false);
        // Entering Done with incomplete dependencies is rejected.
        assert!(matches!(
            check_done_transition(Status::Todo, Status::Done, blocked),
            Err(AppError::Validation(_))
        ));
        // Entering Done without incomplete dependencies is allowed.
        assert!(check_done_transition(Status::Todo, Status::Done, clear).is_ok());
        // Done to Done is a no-op and is allowed even when blocked.
        assert!(check_done_transition(Status::Done, Status::Done, blocked).is_ok());
        // Leaving Done or moving between open states never consults dependencies.
        let fail = || -> Result<bool, AppError> { panic!("dependencies must not be checked") };
        assert!(check_done_transition(Status::Done, Status::Todo, fail).is_ok());
        assert!(check_done_transition(Status::Todo, Status::InProgress, fail).is_ok());
    }
}
