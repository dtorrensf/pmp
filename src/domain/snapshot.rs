// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Task, TaskDependency, TaskNote};
use crate::domain::timestamp::now_utc;
use crate::domain::version::Version;
use crate::domain::version_summary::VersionSummary;

/// Identifier of the portable whole-store snapshot format (A8).
pub const SNAPSHOT_SCHEMA: &str = "pmp.snapshot";
/// Version of the snapshot format this build writes and reads.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Frozen point-in-time representation of a version, built before compaction.
///
/// `previous_summary` carries the summary of the prior compaction when it
/// exists; because it embeds its own `snapshot_json`, recompacting a version
/// preserves the full history recursively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompactSnapshot {
    pub version: Version,
    pub previous_summary: Option<VersionSummary>,
    pub tasks: Vec<Task>,
    pub notes: Vec<TaskNote>,
    pub dependencies: Vec<TaskDependency>,
}

/// Portable, versioned dump of the whole store: every project, version, task,
/// note, dependency and frozen summary, with their ids, so a restore keeps every
/// relationship intact (A8).
///
/// `schema` and `schema_version` identify the wire format; `pmp_version` and
/// `exported_at` describe who wrote the file. Setting `schema_version` above
/// [`SNAPSHOT_SCHEMA_VERSION`] is rejected on import instead of guessing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DataSnapshot {
    pub schema: String,
    pub schema_version: u32,
    pub pmp_version: String,
    pub exported_at: String,
    pub projects: Vec<Project>,
    pub versions: Vec<Version>,
    pub tasks: Vec<Task>,
    pub notes: Vec<TaskNote>,
    pub dependencies: Vec<TaskDependency>,
    pub summaries: Vec<VersionSummary>,
}

/// Number of rows of each kind in a snapshot.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct SnapshotCounts {
    pub projects: usize,
    pub versions: usize,
    pub tasks: usize,
    pub notes: usize,
    pub dependencies: usize,
    pub summaries: usize,
}

impl DataSnapshot {
    /// Wraps a store dump with the format metadata of this build.
    pub fn new(
        projects: Vec<Project>,
        versions: Vec<Version>,
        tasks: Vec<Task>,
        notes: Vec<TaskNote>,
        dependencies: Vec<TaskDependency>,
        summaries: Vec<VersionSummary>,
    ) -> Self {
        Self {
            schema: SNAPSHOT_SCHEMA.to_string(),
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            pmp_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at: now_utc(),
            projects,
            versions,
            tasks,
            notes,
            dependencies,
            summaries,
        }
    }

    /// Rows of each kind this snapshot carries.
    pub fn counts(&self) -> SnapshotCounts {
        SnapshotCounts {
            projects: self.projects.len(),
            versions: self.versions.len(),
            tasks: self.tasks.len(),
            notes: self.notes.len(),
            dependencies: self.dependencies.len(),
            summaries: self.summaries.len(),
        }
    }

    /// Checks that the snapshot is readable by this build and internally
    /// consistent: every record has an id, ids are unique, every reference
    /// points at a record of this snapshot and dependencies stay within one
    /// version without closing a cycle.
    ///
    /// Import calls this before writing anything, so a rejected file never
    /// leaves a partial store behind. Both backends share the same rules.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema != SNAPSHOT_SCHEMA {
            return Err(AppError::Validation(format!(
                "unknown snapshot format {:?}; expected {SNAPSHOT_SCHEMA:?}",
                self.schema
            )));
        }
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(AppError::Validation(format!(
                "unsupported snapshot schema version {}; this build reads version \
                 {SNAPSHOT_SCHEMA_VERSION}",
                self.schema_version
            )));
        }

        let projects = project_ids(&self.projects)?;
        version_ids(&self.versions)?;
        task_ids(&self.tasks)?;
        note_ids(&self.notes)?;
        summary_ids(&self.summaries)?;
        let versions: HashMap<i64, &Version> = self
            .versions
            .iter()
            .map(|v| (v.id.expect("version ids were checked"), v))
            .collect();
        let mut version_names: HashSet<(i64, &str)> = HashSet::new();
        for version in &self.versions {
            if !projects.contains(&version.project_id) {
                return Err(missing(
                    "version",
                    version.id,
                    "project",
                    version.project_id,
                ));
            }
            if !version_names.insert((version.project_id, version.name.as_str())) {
                return Err(AppError::Validation(format!(
                    "snapshot has two versions named {:?} in project {}",
                    version.name, version.project_id
                )));
            }
        }

        let tasks: HashMap<i64, &Task> = self
            .tasks
            .iter()
            .map(|t| (t.id.expect("task ids were checked"), t))
            .collect();
        for task in &self.tasks {
            let Some(version) = versions.get(&task.version_id) else {
                return Err(missing("task", task.id, "version", task.version_id));
            };
            if version.project_id != task.project_id {
                return Err(AppError::Validation(format!(
                    "snapshot task {} points at version {} of another project",
                    task.id.unwrap_or_default(),
                    task.version_id
                )));
            }
            if !projects.contains(&task.project_id) {
                return Err(missing("task", task.id, "project", task.project_id));
            }
        }

        for note in &self.notes {
            if !tasks.contains_key(&note.task_id) {
                return Err(missing("note", note.id, "task", note.task_id));
            }
        }

        let mut edges: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut pairs: HashSet<(i64, i64)> = HashSet::new();
        for dependency in &self.dependencies {
            let (task_id, depends_on_id) = (dependency.task_id, dependency.depends_on_id);
            if task_id == depends_on_id {
                return Err(AppError::Validation(format!(
                    "snapshot dependency of task {task_id} points at itself"
                )));
            }
            if !pairs.insert((task_id, depends_on_id)) {
                return Err(AppError::Validation(format!(
                    "snapshot repeats the dependency {task_id} -> {depends_on_id}"
                )));
            }
            let (Some(task), Some(blocker)) = (tasks.get(&task_id), tasks.get(&depends_on_id))
            else {
                let missing_id = if tasks.contains_key(&task_id) {
                    depends_on_id
                } else {
                    task_id
                };
                return Err(AppError::Validation(format!(
                    "snapshot dependency {task_id} -> {depends_on_id} references task \
                     {missing_id}, which is not in the snapshot"
                )));
            };
            if task.version_id != blocker.version_id {
                return Err(AppError::Validation(format!(
                    "snapshot dependency {task_id} -> {depends_on_id} crosses versions"
                )));
            }
            edges.entry(task_id).or_default().push(depends_on_id);
        }
        if has_cycle(&edges) {
            return Err(AppError::Validation(
                "snapshot dependencies contain a cycle".to_string(),
            ));
        }

        let mut summarized: HashSet<i64> = HashSet::new();
        for summary in &self.summaries {
            if !versions.contains_key(&summary.version_id) {
                return Err(missing(
                    "summary",
                    summary.id,
                    "version",
                    summary.version_id,
                ));
            }
            if !summarized.insert(summary.version_id) {
                return Err(AppError::Validation(format!(
                    "snapshot has more than one summary for version {}",
                    summary.version_id
                )));
            }
        }

        Ok(())
    }
}

fn missing(kind: &str, id: Option<i64>, target: &str, target_id: i64) -> AppError {
    AppError::Validation(format!(
        "snapshot {kind} {} references {target} {target_id}, which is not in the snapshot",
        id.map(|id| id.to_string())
            .unwrap_or_else(|| "?".to_string())
    ))
}

fn unique_ids(items: &[i64], kind: &str) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    for &id in items {
        if !seen.insert(id) {
            return Err(AppError::Validation(format!(
                "snapshot repeats {kind} id {id}"
            )));
        }
    }
    Ok(())
}

fn project_ids(projects: &[Project]) -> Result<HashSet<i64>, AppError> {
    let ids = required_ids(projects.iter().map(|p| p.id), "project")?;
    Ok(ids.into_iter().collect())
}

fn version_ids(versions: &[Version]) -> Result<HashSet<i64>, AppError> {
    Ok(required_ids(versions.iter().map(|v| v.id), "version")?
        .into_iter()
        .collect())
}

fn task_ids(tasks: &[Task]) -> Result<HashSet<i64>, AppError> {
    Ok(required_ids(tasks.iter().map(|t| t.id), "task")?
        .into_iter()
        .collect())
}

fn note_ids(notes: &[TaskNote]) -> Result<HashSet<i64>, AppError> {
    Ok(required_ids(notes.iter().map(|n| n.id), "note")?
        .into_iter()
        .collect())
}

fn summary_ids(summaries: &[VersionSummary]) -> Result<HashSet<i64>, AppError> {
    Ok(required_ids(summaries.iter().map(|s| s.id), "summary")?
        .into_iter()
        .collect())
}

/// Ids of every record, rejecting an id-less record and a repeated id.
fn required_ids(ids: impl Iterator<Item = Option<i64>>, kind: &str) -> Result<Vec<i64>, AppError> {
    let ids: Vec<i64> = ids
        .map(|id| id.ok_or_else(|| AppError::Validation(format!("snapshot {kind} has no id"))))
        .collect::<Result<_, _>>()?;
    unique_ids(&ids, kind)?;
    Ok(ids)
}

/// Kahn's algorithm over `task -> dependencies`: true when some edge is part of
/// a cycle, which would make the imported tasks impossible to finish.
fn has_cycle(graph: &HashMap<i64, Vec<i64>>) -> bool {
    let mut indegree: HashMap<i64, i64> = HashMap::new();
    for (task, dependencies) in graph {
        indegree.entry(*task).or_insert(0);
        for dependency in dependencies {
            *indegree.entry(*dependency).or_insert(0) += 1;
        }
    }
    let mut ready: Vec<i64> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(task, _)| *task)
        .collect();
    let mut visited = 0usize;
    while let Some(task) = ready.pop() {
        visited += 1;
        for dependency in graph.get(&task).into_iter().flatten() {
            if let Some(degree) = indegree.get_mut(dependency) {
                *degree -= 1;
                if *degree == 0 {
                    ready.push(*dependency);
                }
            }
        }
    }
    visited != indegree.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;

    #[test]
    fn snapshot_serializes_and_deserializes() {
        let snapshot = CompactSnapshot {
            version: Version {
                id: Some(1),
                project_id: 1,
                name: "v1.0.0".to_string(),
                description: "".to_string(),
                status: VersionStatus::Released,
                created_at: "now".to_string(),
            },
            previous_summary: None,
            tasks: vec![Task {
                id: Some(2),
                project_id: 1,
                version_id: 1,
                name: "T".to_string(),
                description: "".to_string(),
                status: Status::Done,
                priority: Priority::High,
                created_at: "now".to_string(),
                completed_at: Some("now".to_string()),
            }],
            notes: vec![],
            dependencies: vec![],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: CompactSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snapshot);
    }
}
