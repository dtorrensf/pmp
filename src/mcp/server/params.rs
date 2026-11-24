// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::Deserialize;

use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

/// Priority as accepted by MCP tools. Serialized as a plain string enum
/// (`"Low"`, `"Medium"`, `"High"`) so the generated input schema is a
/// non-nullable string enum that clients can rely on. Defaults to Medium.
#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
pub enum PriorityParam {
    Low,
    #[default]
    Medium,
    High,
}

impl From<PriorityParam> for Priority {
    fn from(value: PriorityParam) -> Self {
        match value {
            PriorityParam::Low => Priority::Low,
            PriorityParam::Medium => Priority::Medium,
            PriorityParam::High => Priority::High,
        }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    pub project_id: i64,
    /// Restrict the list to one version of the project.
    pub version_id: Option<i64>,
    pub status: Option<Status>,
    pub priority: Option<PriorityParam>,
    /// Maximum tasks to return. Defaults to 100; capped at 500.
    pub limit: Option<u32>,
    /// Number of matching tasks to skip. Defaults to 0.
    pub offset: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListVersionsParams {
    pub project_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateProjectParams {
    pub name: String,
    pub description: Option<String>,
    /// Absolute path of the project's workspace on disk. Optional: omit it for
    /// a project that is not bound to a directory yet.
    pub working_dir: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateProjectParams {
    pub project_id: i64,
    pub name: String,
    /// New description. When omitted, the current description is kept.
    pub description: Option<String>,
    /// New working directory. Omit it to keep the current one; send an empty
    /// string to clear it. Relative paths are rejected.
    pub working_dir: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateVersionParams {
    pub project_id: i64,
    pub name: String,
    pub description: Option<String>,
    /// `backlog` (default), `open` or `released`.
    pub status: Option<VersionStatus>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateVersionParams {
    pub version_id: i64,
    /// New name. Must not be blank.
    pub name: String,
    /// New description. When omitted, the current description is kept.
    pub description: Option<String>,
    /// New status: `backlog`, `open` or `released`. When omitted, the current status is kept.
    pub status: Option<VersionStatus>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskStatusParams {
    pub task_id: i64,
    pub status: Status,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MoveTaskParams {
    pub task_id: i64,
    /// Version of the same project the task moves to.
    pub version_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskPriorityParams {
    pub task_id: i64,
    pub priority: PriorityParam,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetTaskDependenciesParams {
    pub task_id: i64,
    /// Full new set of task ids that `task_id` depends on. Empty clears them.
    pub depends_on_ids: Vec<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskDependencyParams {
    pub task_id: i64,
    pub depends_on_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskIdParams {
    pub task_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    pub project_id: i64,
    /// Version the task is added to; must belong to `project_id`.
    pub version_id: i64,
    pub name: String,
    pub context: String,
    pub requirements: String,
    pub benefits: String,
    /// Task priority: `Low`, `Medium` or `High`. Defaults to `Medium`.
    #[serde(default)]
    pub priority: PriorityParam,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskParams {
    pub task_id: i64,
    /// New name. Must not be blank when provided.
    pub name: Option<String>,
    /// New context section. Omit to keep the current one.
    pub context: Option<String>,
    /// New requirements section. Omit to keep the current one.
    pub requirements: Option<String>,
    /// New benefits section. Omit to keep the current one.
    pub benefits: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AddTaskNoteParams {
    pub task_id: i64,
    pub content: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct NoteIdParams {
    pub note_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct VersionIdParams {
    pub version_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CompactVersionParams {
    pub version_id: i64,
    pub summary: String,
    /// When omitted or true, the version is marked `released` after compacting
    /// once no tasks remain. Ignored while todo tasks remain: the version stays open.
    pub mark_released: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadVersionSummaryParams {
    pub version_id: i64,
    pub summary_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SummaryIdParams {
    pub summary_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DeleteVersionSummaryParams {
    /// The version the summary belongs to; a mismatch is reported as not found.
    pub version_id: i64,
    pub summary_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExportVersionSummaryParams {
    pub summary_id: i64,
}
