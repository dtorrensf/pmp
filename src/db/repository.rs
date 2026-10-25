// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};

use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};
use crate::domain::version::{Version, VersionStatus};

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

/// Removes duplicate ids while keeping the first occurrence order.
pub(crate) fn unique_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Repository;

    #[test]
    fn repository_trait_is_object_safe() {
        let _: Option<Arc<dyn Repository>> = None;
    }
}
