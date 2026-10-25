// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, ListResourceTemplatesResult, ListResourcesResult,
    ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{tool, tool_handler, tool_router};
use serde::Deserialize;
use tokio::task;

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::{Priority, Status, Task};
use crate::domain::version::VersionStatus;
use crate::mcp::task_description::compose_description;

pub struct PmpServer {
    repo: Arc<dyn Repository>,
}

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
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListVersionsParams {
    pub project_id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateProjectParams {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateVersionParams {
    pub project_id: i64,
    pub name: String,
    pub description: Option<String>,
    /// `open` (default) or `released`.
    pub status: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskStatusParams {
    pub task_id: i64,
    pub status: Status,
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
pub struct AddTaskNoteParams {
    pub task_id: i64,
    pub content: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct NoteIdParams {
    pub note_id: i64,
}

#[tool_router]
impl PmpServer {
    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self { repo }
    }

    pub async fn list_resources(&self) -> Result<ListResourcesResult, ErrorData> {
        let resource = Resource::new("pmp://projects", "projects")
            .with_description(
                "All user projects. Read this to see what projects the user is working on.",
            )
            .with_mime_type("application/json");
        Ok(ListResourcesResult::with_all_items(vec![resource]))
    }

    pub async fn list_resource_templates(&self) -> Result<ListResourceTemplatesResult, ErrorData> {
        let templates = vec![
            ResourceTemplate::new("pmp://projects/{id}", "project")
                .with_description("A single project by id. Use to get details about a specific project.")
                .with_mime_type("application/json"),
            ResourceTemplate::new("pmp://projects/{id}/tasks", "project_tasks")
                .with_description("Tasks for a project across all its versions. Use to see all tasks and their status for a specific project.")
                .with_mime_type("application/json"),
            ResourceTemplate::new("pmp://projects/{id}/versions", "project_versions")
                .with_description("Versions of a project, open ones first. Use to see the release lines of a project and their status.")
                .with_mime_type("application/json"),
            ResourceTemplate::new("pmp://versions/{id}/tasks", "version_tasks")
                .with_description("Tasks of one version. Use to see the work scheduled for a specific release.")
                .with_mime_type("application/json"),
        ];
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        let repo = self.repo.clone();

        match uri {
            "pmp://projects" => {
                let projects = task::spawn_blocking(move || repo.list_projects())
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
                    .map_err(app_error_to_mcp)?;
                let text = serde_json::to_string_pretty(&projects)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(ReadResourceResult::new(vec![
                    ResourceContents::text(text, uri).with_mime_type("application/json"),
                ]))
            }
            uri if uri.starts_with("pmp://versions/") => {
                let tail = &uri["pmp://versions/".len()..];
                let id = tail
                    .strip_suffix("/tasks")
                    .ok_or_else(|| ErrorData::invalid_params("unsupported resource uri", None))?
                    .parse::<i64>()
                    .map_err(|_| ErrorData::invalid_params("invalid version id", None))?;
                let tasks = task::spawn_blocking(move || repo.list_tasks_by_version(id))
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
                    .map_err(app_error_to_mcp)?;
                let text = serde_json::to_string_pretty(&tasks)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(ReadResourceResult::new(vec![
                    ResourceContents::text(text, uri).with_mime_type("application/json"),
                ]))
            }
            uri if uri.starts_with("pmp://projects/") => {
                let tail = &uri["pmp://projects/".len()..];
                if let Some(prefix) = tail.strip_suffix("/versions") {
                    let id = prefix
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    let versions = task::spawn_blocking(move || repo.list_versions_by_project(id))
                        .await
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
                        .map_err(app_error_to_mcp)?;
                    let text = serde_json::to_string_pretty(&versions)
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                    Ok(ReadResourceResult::new(vec![
                        ResourceContents::text(text, uri).with_mime_type("application/json"),
                    ]))
                } else if let Some(prefix) = tail.strip_suffix("/tasks") {
                    let id = prefix
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    let tasks = task::spawn_blocking(move || repo.list_tasks_by_project(id))
                        .await
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
                        .map_err(app_error_to_mcp)?;
                    let text = serde_json::to_string_pretty(&tasks)
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                    Ok(ReadResourceResult::new(vec![
                        ResourceContents::text(text, uri).with_mime_type("application/json"),
                    ]))
                } else {
                    let id = tail
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    let project = task::spawn_blocking(move || repo.get_project(id))
                        .await
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
                        .map_err(app_error_to_mcp)?;
                    let text = serde_json::to_string_pretty(&project)
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                    Ok(ReadResourceResult::new(vec![
                        ResourceContents::text(text, uri).with_mime_type("application/json"),
                    ]))
                }
            }
            _ => Err(ErrorData::invalid_params("unsupported resource uri", None)),
        }
    }

    #[tool(
        name = "list_tasks",
        description = "List tasks for a project, optionally filtered by version_id (see list_versions), status (todo, in_progress, done) and priority (Low, Medium, High). Use when the user asks about their tasks, to-dos, pending work or progress, or to pick the next task to work on."
    )]
    async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            repo.get_project(params.project_id)?;
            match params.version_id {
                Some(version_id) => {
                    let version = repo.get_version(version_id).map_err(|_| {
                        AppError::Validation("version does not belong to the project".to_string())
                    })?;
                    if version.project_id != params.project_id {
                        return Err(AppError::Validation(
                            "version does not belong to the project".to_string(),
                        ));
                    }
                    repo.list_tasks_by_version(version_id)
                }
                None => repo.list_tasks_by_project(params.project_id),
            }
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(tasks) => {
                let filtered: Vec<Task> = tasks
                    .into_iter()
                    .filter(|t| {
                        params.status.is_none_or(|s| t.status == s)
                            && params
                                .priority
                                .is_none_or(|p| t.priority == Priority::from(p))
                    })
                    .collect();
                let content = ContentBlock::json(&filtered)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "project not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "list_projects",
        description = "List all projects of the user. Use to discover project ids before listing versions or tasks, or when the user asks what projects they have."
    )]
    async fn list_projects(&self) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || repo.list_projects())
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(projects) => {
                let content = ContentBlock::json(&projects)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "create_project",
        description = "Create a new project. Use when the user asks to start a new project. Requires a non-empty name; description is optional. The project starts with a default version (see list_versions)."
    )]
    async fn create_project(
        &self,
        Parameters(params): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            if params.name.trim().is_empty() {
                return Err(AppError::Validation(
                    "project name cannot be empty".to_string(),
                ));
            }
            let description = params.description.unwrap_or_default();
            repo.create_project(&params.name, &description)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(project) => {
                let content = ContentBlock::json(&project)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "list_versions",
        description = "List the versions (release lines) of a project, open ones first. Use to find the version_id that tasks must be created in, or to check which versions are released."
    )]
    async fn list_versions(
        &self,
        Parameters(params): Parameters<ListVersionsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            repo.get_project(params.project_id)?;
            repo.list_versions_by_project(params.project_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(versions) => {
                let content = ContentBlock::json(&versions)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "project not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "create_version",
        description = "Create a version (release line) in a project. Use when starting a new release, for example v0.2.0. Name must be unique within the project and not empty. Status is open (default) or released."
    )]
    async fn create_version(
        &self,
        Parameters(params): Parameters<CreateVersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            let status = match params.status.as_deref() {
                Some(value) => VersionStatus::try_from_str(value)?,
                None => VersionStatus::Open,
            };
            repo.get_project(params.project_id)?;
            let description = params.description.unwrap_or_default();
            let version = repo.create_version(params.project_id, &params.name, &description)?;
            match version.id {
                Some(id) if status != VersionStatus::Open => {
                    repo.update_version(id, &version.name, &version.description, status)
                }
                _ => Ok(version),
            }
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(version) => {
                let content = ContentBlock::json(&version)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "project not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "update_task_priority",
        description = "Change the priority of an existing task to Low, Medium or High. Use to reprioritize work; prefer this over editing the database directly."
    )]
    async fn update_task_priority(
        &self,
        Parameters(params): Parameters<UpdateTaskPriorityParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            repo.update_task_priority(params.task_id, params.priority.into())
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(task) => {
                let content = ContentBlock::json(&task)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "create_task",
        description = "Create a new task in a version of a project. Use only when the user asks for a new task or work item. Requires the project id, the version id (see list_versions), a task name, and context, requirements and benefits. The task starts as todo. Optionally accepts a priority (Low, Medium, High); defaults to Medium."
    )]
    async fn create_task(
        &self,
        Parameters(params): Parameters<CreateTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            if params.name.trim().is_empty() {
                return Err(AppError::Validation(
                    "task name cannot be empty".to_string(),
                ));
            }
            repo.get_project(params.project_id)?;
            let description =
                compose_description(&params.context, &params.requirements, &params.benefits);
            repo.create_task(
                params.project_id,
                params.version_id,
                &params.name,
                &description,
                Status::Todo,
                params.priority.into(),
            )
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(task) => {
                let content = ContentBlock::json(&task)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "project not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "update_task_status",
        description = "Change the status of a task to todo, in_progress or done. Call with in_progress when you start working on a task and with done when it is finished. Marking a task done fails with a validation error while it has incomplete dependencies; finish those first. Unknown task ids return task not found."
    )]
    async fn update_task_status(
        &self,
        Parameters(params): Parameters<UpdateTaskStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let new_status = params.status;
        let result = task::spawn_blocking(move || {
            let task = repo.get_task(task_id)?;
            if new_status == Status::Done
                && task.status != Status::Done
                && repo.has_incomplete_dependencies(task_id)?
            {
                return Err(AppError::Validation(
                    "cannot mark task as done: it has incomplete dependencies".to_string(),
                ));
            }
            repo.update_task_status(task_id, new_status)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(task) => {
                let content = ContentBlock::json(&task)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(AppError::DependencyCycle) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "dependency cycle detected".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "get_task_dependencies",
        description = "List the task ids that a task depends on (its blockers). Check before starting a task: incomplete dependencies block marking it done."
    )]
    async fn get_task_dependencies(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.get_dependencies(task_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(depends_on_ids) => {
                let content = ContentBlock::json(serde_json::json!({
                    "task_id": task_id,
                    "depends_on_ids": depends_on_ids,
                }))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "get_task_dependents",
        description = "List the task ids that depend on a task (tasks it blocks). Use to understand the impact of changing or deleting a task."
    )]
    async fn get_task_dependents(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.get_dependents(task_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(task_ids) => {
                let content = ContentBlock::json(serde_json::json!({
                    "task_id": task_id,
                    "task_ids": task_ids,
                }))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "add_task_dependency",
        description = "Make one task depend on another: task_id depends on depends_on_id. Both tasks must be in the same version. Rejects self-dependencies and circular dependencies."
    )]
    async fn add_task_dependency(
        &self,
        Parameters(params): Parameters<TaskDependencyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let depends_on_id = params.depends_on_id;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.get_task(depends_on_id)?;
            repo.add_dependency(task_id, depends_on_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(()) => {
                let content = ContentBlock::json(serde_json::json!({
                    "task_id": task_id,
                    "depends_on_id": depends_on_id,
                }))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(AppError::DependencyCycle) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "dependency cycle detected",
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "set_task_dependencies",
        description = "Set the complete list of tasks that a task depends on, replacing the current list in one atomic operation. Prefer this over add/remove when changing several dependencies. Pass an empty list to clear them. Rejects self-dependencies, cycles and tasks from another version; on error nothing is changed. Returns the resulting depends_on ids."
    )]
    async fn set_task_dependencies(
        &self,
        Parameters(params): Parameters<SetTaskDependenciesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let result = task::spawn_blocking(move || {
            repo.set_dependencies(task_id, &params.depends_on_ids)?;
            repo.get_dependencies(task_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(ids) => {
                let content = ContentBlock::json(serde_json::json!({
                    "task_id": task_id,
                    "depends_on_ids": ids,
                }))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(AppError::DependencyCycle) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "dependency cycle detected",
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "remove_task_dependency",
        description = "Remove a dependency between two tasks. The task_id depends on depends_on_id."
    )]
    async fn remove_task_dependency(
        &self,
        Parameters(params): Parameters<TaskDependencyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let depends_on_id = params.depends_on_id;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.get_task(depends_on_id)?;
            repo.remove_dependency(task_id, depends_on_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(()) => {
                let content = ContentBlock::json(serde_json::json!({
                    "task_id": task_id,
                    "depends_on_id": depends_on_id,
                }))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "list_task_notes",
        description = "List the notes of a task, newest first. Use to read progress updates, decisions, context and reminders before continuing work on it."
    )]
    async fn list_task_notes(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.list_task_notes(task_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(notes) => {
                let content = ContentBlock::json(&notes)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "add_task_note",
        description = "Add a note to a task to record progress, decisions or context. Add notes as you work so the task history reflects what was actually done."
    )]
    async fn add_task_note(
        &self,
        Parameters(params): Parameters<AddTaskNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let task_id = params.task_id;
        let content = params.content;
        let result = task::spawn_blocking(move || {
            repo.get_task(task_id)?;
            repo.add_task_note(task_id, &content)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(note) => {
                let content = ContentBlock::json(&note)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "task not found".to_string(),
            )])),
            Err(AppError::Validation(msg)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }

    #[tool(
        name = "delete_task_note",
        description = "Delete a task note by its note id."
    )]
    async fn delete_task_note(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let note_id = params.note_id;
        let result = task::spawn_blocking(move || repo.delete_task_note(note_id))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(()) => {
                let content = ContentBlock::json(serde_json::json!({ "note_id": note_id }))
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![content]))
            }
            Err(AppError::NotFound) => Ok(CallToolResult::error(vec![ContentBlock::text(
                "note not found".to_string(),
            )])),
            Err(e) => Err(app_error_to_mcp(e)),
        }
    }
}

#[tool_handler]
impl ServerHandler for PmpServer {
    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.list_resources().await
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.list_resource_templates().await
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        self.read_resource(&request.uri).await
    }

    fn get_info(&self) -> ServerInfo {
        let mut capabilities = ServerCapabilities::builder()
            .enable_resources()
            .enable_tools()
            .build();
        capabilities.resources.as_mut().unwrap().list_changed = Some(false);
        capabilities.tools.as_mut().unwrap().list_changed = Some(false);
        ServerInfo::new(capabilities)
            .with_instructions(
                "Personal project manager for the user's projects, tasks and productivity.\n\n\
Expected workflow for keeping task state in sync with real work:\n\
1. Find the task with list_tasks (filter by version_id, status or priority).\n\
2. Check blockers with get_task_dependencies; a task with incomplete dependencies cannot be marked done.\n\
3. When you start a task, call update_task_status with in_progress.\n\
4. Record progress, decisions and context as you go with add_task_note.\n\
5. When the work is finished, call update_task_status with done.\n\n\
Only change status for tasks you are actually working on. Use create_task only when the user asks for a new task.",
            )
    }
}

pub async fn serve(repo: Arc<dyn Repository>) -> Result<()> {
    let server = PmpServer::new(repo);
    let service = rmcp::service::serve_server(server, rmcp::transport::io::stdio()).await?;
    eprintln!("[pmp] MCP server listening on stdio");
    service.waiting().await?;
    Ok(())
}

fn app_error_to_mcp(err: AppError) -> ErrorData {
    match err {
        AppError::NotFound => ErrorData::new(
            ErrorCode(-32602),
            "resource not found".to_string(),
            None::<serde_json::Value>,
        ),
        AppError::DbError(_) => {
            ErrorData::internal_error(err.to_string(), None::<serde_json::Value>)
        }
        AppError::Validation(_) | AppError::DependencyCycle => {
            ErrorData::invalid_params(err.to_string(), None::<serde_json::Value>)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rmcp::ErrorData;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::{CallToolResult, ErrorCode, ReadResourceResult, ResourceContents};

    use super::PriorityParam;
    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::domain::error::AppError;
    use crate::domain::project::Project;
    use crate::domain::task::{Priority, Status, Task, TaskNote};
    use crate::domain::version::{Version, VersionStatus};

    use super::{
        AddTaskNoteParams, CreateProjectParams, CreateTaskParams, CreateVersionParams,
        ListTasksParams, ListVersionsParams, NoteIdParams, PmpServer, SetTaskDependenciesParams,
        TaskDependencyParams, TaskIdParams, UpdateTaskPriorityParams, UpdateTaskStatusParams,
    };

    fn server() -> (PmpServer, Arc<StubRepository>) {
        let repo = Arc::new(StubRepository::default());
        (PmpServer::new(repo.clone()), repo)
    }

    fn seeded_server() -> (PmpServer, Arc<StubRepository>, i64) {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Low Todo",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "High Done",
            "",
            Status::Done,
            Priority::High,
        )
        .unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Medium InProgress",
            "",
            Status::InProgress,
            Priority::Medium,
        )
        .unwrap();
        (server, repo, pid)
    }

    fn tool_text(result: CallToolResult) -> String {
        result
            .content
            .into_iter()
            .next()
            .and_then(|c| c.as_text().map(|t| t.text.clone()))
            .expect("tool result should have text content")
    }

    struct FailingRepository;

    impl Repository for FailingRepository {
        fn list_projects(&self) -> Result<Vec<Project>, AppError> {
            Ok(vec![])
        }

        fn create_project(&self, _: &str, _: &str) -> Result<Project, AppError> {
            unimplemented!()
        }

        fn get_project(&self, _: i64) -> Result<Project, AppError> {
            Err(AppError::DbError(rusqlite::Error::InvalidQuery))
        }

        fn update_project(&self, _: i64, _: &str, _: &str) -> Result<Project, AppError> {
            unimplemented!()
        }

        fn list_tasks_by_project(&self, _: i64) -> Result<Vec<Task>, AppError> {
            Err(AppError::DbError(rusqlite::Error::InvalidQuery))
        }

        fn create_task(
            &self,
            _: i64,
            _: i64,
            _: &str,
            _: &str,
            _: Status,
            _: Priority,
        ) -> Result<Task, AppError> {
            unimplemented!()
        }

        fn list_versions_by_project(&self, _: i64) -> Result<Vec<Version>, AppError> {
            Err(AppError::DbError(rusqlite::Error::InvalidQuery))
        }

        fn create_version(&self, _: i64, _: &str, _: &str) -> Result<Version, AppError> {
            unimplemented!()
        }

        fn get_version(&self, _: i64) -> Result<Version, AppError> {
            unimplemented!()
        }

        fn update_version(
            &self,
            _: i64,
            _: &str,
            _: &str,
            _: VersionStatus,
        ) -> Result<Version, AppError> {
            unimplemented!()
        }

        fn delete_version(&self, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn list_tasks_by_version(&self, _: i64) -> Result<Vec<Task>, AppError> {
            Err(AppError::DbError(rusqlite::Error::InvalidQuery))
        }

        fn get_task(&self, _: i64) -> Result<Task, AppError> {
            unimplemented!()
        }

        fn update_task(
            &self,
            _: i64,
            _: &str,
            _: &str,
            _: Status,
            _: Priority,
        ) -> Result<Task, AppError> {
            unimplemented!()
        }

        fn update_task_status(&self, _: i64, _: Status) -> Result<Task, AppError> {
            unimplemented!()
        }

        fn update_task_priority(&self, _: i64, _: Priority) -> Result<Task, AppError> {
            unimplemented!()
        }

        fn add_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn remove_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn set_dependencies(&self, _: i64, _: &[i64]) -> Result<(), AppError> {
            unimplemented!()
        }

        fn get_dependencies(&self, _: i64) -> Result<Vec<i64>, AppError> {
            unimplemented!()
        }

        fn has_incomplete_dependencies(&self, _: i64) -> Result<bool, AppError> {
            unimplemented!()
        }

        fn delete_project(&self, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn delete_task(&self, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn get_dependents(&self, _: i64) -> Result<Vec<i64>, AppError> {
            unimplemented!()
        }

        fn would_create_cycle(&self, _: i64, _: i64) -> Result<bool, AppError> {
            unimplemented!()
        }

        fn list_task_notes(&self, _: i64) -> Result<Vec<TaskNote>, AppError> {
            unimplemented!()
        }

        fn add_task_note(&self, _: i64, _: &str) -> Result<TaskNote, AppError> {
            unimplemented!()
        }

        fn delete_task_note(&self, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn list_resources_includes_projects() {
        let (server, _) = server();
        let result = server.list_resources().await.unwrap();
        assert_eq!(result.resources.len(), 1);
        assert_eq!(result.resources[0].uri, "pmp://projects");
    }

    #[tokio::test]
    async fn list_resource_templates_include_project_and_tasks() {
        let (server, _) = server();
        let result = server.list_resource_templates().await.unwrap();
        insta::assert_json_snapshot!(
            result
                .resource_templates
                .iter()
                .map(|t| (&t.uri_template, &t.name))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn read_projects_returns_all_projects() {
        let (server, repo) = server();
        repo.create_project("Alpha", "First project").unwrap();
        repo.create_project("Beta", "Second project").unwrap();
        let result = server.read_resource("pmp://projects").await.unwrap();
        insta::assert_json_snapshot!(result);
    }

    #[tokio::test]
    async fn read_project_by_id_returns_project() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "First project").unwrap();
        let result = server
            .read_resource(&format!("pmp://projects/{}", project.id.unwrap()))
            .await
            .unwrap();
        insta::assert_json_snapshot!(result);
    }

    #[tokio::test]
    async fn read_project_tasks_returns_ordered_tasks() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "First project").unwrap();
        let pid = project.id.unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Low task",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "High task",
            "",
            Status::InProgress,
            Priority::High,
        )
        .unwrap();
        let result = server
            .read_resource(&format!("pmp://projects/{}/tasks", pid))
            .await
            .unwrap();
        insta::assert_json_snapshot!(result);
    }

    #[tokio::test]
    async fn read_unknown_project_returns_resource_not_found() {
        let (server, _) = server();
        let err = server
            .read_resource("pmp://projects/999")
            .await
            .unwrap_err();
        let ErrorData { code, .. } = err;
        assert_eq!(code, ErrorCode(-32602));
        insta::assert_json_snapshot!(err);
    }

    #[tokio::test]
    async fn list_tasks_returns_all_tasks_for_project() {
        let (server, _, pid) = seeded_server();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: None,
                status: None,
                priority: None,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_status() {
        let (server, _, pid) = seeded_server();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: None,
                status: Some(Status::Done),
                priority: None,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Done);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_priority() {
        let (server, _, pid) = seeded_server();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: None,
                status: None,
                priority: Some(PriorityParam::High),
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, Priority::High);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_status_and_priority() {
        let (server, _, pid) = seeded_server();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: None,
                status: Some(Status::Todo),
                priority: Some(PriorityParam::Low),
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Todo);
        assert_eq!(tasks[0].priority, Priority::Low);
    }

    #[tokio::test]
    async fn list_tasks_unknown_project_returns_tool_error() {
        let (server, _) = server();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: 999,
                version_id: None,
                status: None,
                priority: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("project not found"));
    }

    #[tokio::test]
    async fn list_tasks_db_error_returns_internal_error() {
        let repo = Arc::new(FailingRepository);
        let server = PmpServer::new(repo);
        let err = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: 1,
                version_id: None,
                status: None,
                priority: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode(-32603));
    }

    #[tokio::test]
    async fn update_task_status_changes_status() {
        let (server, _, pid) = seeded_server();
        let task_id = {
            let tasks = server.repo.list_tasks_by_project(pid).unwrap();
            tasks
                .iter()
                .find(|t| t.status == Status::Todo)
                .unwrap()
                .id
                .unwrap()
        };
        let result = server
            .update_task_status(Parameters(UpdateTaskStatusParams {
                task_id,
                status: Status::InProgress,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let task: Task = serde_json::from_str(&text).unwrap();
        assert_eq!(task.status, Status::InProgress);
        assert_eq!(task.id, Some(task_id));
    }

    #[tokio::test]
    async fn update_task_status_unknown_task_returns_tool_error() {
        let (server, _) = server();
        let result = server
            .update_task_status(Parameters(UpdateTaskStatusParams {
                task_id: 999,
                status: Status::Done,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("task not found"));
    }

    #[tokio::test]
    async fn create_project_creates_project_with_default_version() {
        let (server, repo) = server();
        let project: Project = serde_json::from_str(&tool_text(
            server
                .create_project(Parameters(CreateProjectParams {
                    name: "Gamma".to_string(),
                    description: Some("desc".to_string()),
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(project.name, "Gamma");
        assert_eq!(project.description, "desc");
        let pid = project.id.unwrap();
        assert_eq!(repo.list_versions_by_project(pid).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_project_defaults_description_and_rejects_empty_name() {
        let (server, _) = server();
        let project: Project = serde_json::from_str(&tool_text(
            server
                .create_project(Parameters(CreateProjectParams {
                    name: "Delta".to_string(),
                    description: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(project.description, "");

        let rejected = server
            .create_project(Parameters(CreateProjectParams {
                name: "   ".to_string(),
                description: None,
            }))
            .await
            .unwrap();
        assert_eq!(rejected.is_error, Some(true));
        assert!(tool_text(rejected).contains("project name cannot be empty"));
    }

    #[tokio::test]
    async fn set_task_dependencies_replaces_the_full_set() {
        let (server, repo, pid) = seeded_server();
        let version = repo.default_version_id(pid).unwrap();
        let mk = |name: &str| {
            repo.create_task(pid, version, name, "", Status::Todo, Priority::Low)
                .unwrap()
                .id
                .unwrap()
        };
        let (a, b, c) = (mk("A"), mk("B"), mk("C"));
        repo.add_dependency(a, b).unwrap();

        let result = server
            .set_task_dependencies(Parameters(SetTaskDependenciesParams {
                task_id: a,
                depends_on_ids: vec![c],
            }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        let body: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
        assert_eq!(body["depends_on_ids"], serde_json::json!([c]));
        assert_eq!(repo.get_dependencies(a).unwrap(), vec![c]);
    }

    #[tokio::test]
    async fn set_task_dependencies_rejects_cycle_and_keeps_previous_set() {
        let (server, repo, pid) = seeded_server();
        let version = repo.default_version_id(pid).unwrap();
        let mk = |name: &str| {
            repo.create_task(pid, version, name, "", Status::Todo, Priority::Low)
                .unwrap()
                .id
                .unwrap()
        };
        let (a, b, c) = (mk("A"), mk("B"), mk("C"));
        repo.add_dependency(b, c).unwrap();
        repo.add_dependency(a, c).unwrap();

        // C depends on A would close A -> C -> A ... via B: B -> C -> A -> B
        let result = server
            .set_task_dependencies(Parameters(SetTaskDependenciesParams {
                task_id: c,
                depends_on_ids: vec![a],
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("dependency cycle"));
        assert!(repo.get_dependencies(c).unwrap().is_empty());
        assert_eq!(repo.get_dependencies(b).unwrap(), vec![c]);
    }

    #[tokio::test]
    async fn list_projects_returns_all_projects() {
        let (server, repo) = server();
        repo.create_project("Alpha", "").unwrap();
        repo.create_project("Beta", "").unwrap();
        let projects: Vec<Project> =
            serde_json::from_str(&tool_text(server.list_projects().await.unwrap())).unwrap();
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Alpha") && names.contains(&"Beta"));
    }

    #[tokio::test]
    async fn update_task_priority_changes_priority() {
        let (server, _, pid) = seeded_server();
        let task_id = server
            .repo
            .list_tasks_by_project(pid)
            .unwrap()
            .iter()
            .find(|t| t.priority == Priority::Medium)
            .unwrap()
            .id
            .unwrap();
        let result = server
            .update_task_priority(Parameters(UpdateTaskPriorityParams {
                task_id,
                priority: PriorityParam::High,
            }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        let task: Task = serde_json::from_str(&tool_text(result)).unwrap();
        assert_eq!(task.id, Some(task_id));
        assert_eq!(task.priority, Priority::High);
        assert_eq!(
            server.repo.get_task(task_id).unwrap().priority,
            Priority::High
        );
    }

    #[tokio::test]
    async fn update_task_priority_unknown_task_returns_error() {
        let (server, _) = server();
        let result = server
            .update_task_priority(Parameters(UpdateTaskPriorityParams {
                task_id: 999,
                priority: PriorityParam::Low,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("task not found"));
    }

    #[tokio::test]
    async fn update_task_status_rejects_done_with_incomplete_deps() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task_a = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task A",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let task_b = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task B",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let task_a_id = task_a.id.unwrap();
        let task_b_id = task_b.id.unwrap();
        repo.add_dependency(task_b_id, task_a_id).unwrap();

        let result = server
            .update_task_status(Parameters(UpdateTaskStatusParams {
                task_id: task_b_id,
                status: Status::Done,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("incomplete dependencies"));
    }

    #[tokio::test]
    async fn update_task_status_allows_done_when_deps_complete() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task_a = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task A",
                "",
                Status::Done,
                Priority::High,
            )
            .unwrap();
        let task_b = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task B",
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap();
        let task_a_id = task_a.id.unwrap();
        let task_b_id = task_b.id.unwrap();
        repo.add_dependency(task_b_id, task_a_id).unwrap();

        let result = server
            .update_task_status(Parameters(UpdateTaskStatusParams {
                task_id: task_b_id,
                status: Status::Done,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let task: Task = serde_json::from_str(&text).unwrap();
        assert_eq!(task.status, Status::Done);
    }

    #[tokio::test]
    async fn update_task_status_db_error_returns_internal_error() {
        let repo = Arc::new(FailingRepository);
        let server = PmpServer::new(repo);
        let err = server
            .update_task_status(Parameters(UpdateTaskStatusParams {
                task_id: 1,
                status: Status::Done,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode(-32603));
    }

    #[tokio::test]
    async fn create_task_creates_todo_task_with_composed_description() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();

        let result = server
            .create_task(Parameters(CreateTaskParams {
                project_id: pid,
                version_id: repo.default_version_id(pid).unwrap(),
                name: "New task".to_string(),
                context: "We need this.".to_string(),
                requirements: "- Do it well".to_string(),
                benefits: "Ship faster.".to_string(),
                priority: PriorityParam::Medium,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let task: Task = serde_json::from_str(&text).unwrap();
        assert_eq!(task.name, "New task");
        assert_eq!(task.status, Status::Todo);
        assert_eq!(task.priority, Priority::Medium);
        assert!(task.description.contains("## Context"));
        assert!(task.description.contains("We need this."));
        assert!(task.description.contains("## Requirements"));
        assert!(task.description.contains("- Do it well"));
        assert!(task.description.contains("## Benefits"));
        assert!(task.description.contains("Ship faster."));
    }

    #[tokio::test]
    async fn create_task_uses_provided_priority() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();

        let result = server
            .create_task(Parameters(CreateTaskParams {
                project_id: pid,
                version_id: repo.default_version_id(pid).unwrap(),
                name: "High priority task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: PriorityParam::High,
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let task: Task = serde_json::from_str(&text).unwrap();
        assert_eq!(task.priority, Priority::High);
    }

    #[tokio::test]
    async fn create_task_unknown_project_returns_tool_error() {
        let (server, _) = server();
        let result = server
            .create_task(Parameters(CreateTaskParams {
                project_id: 999,
                version_id: 1,
                name: "Task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: PriorityParam::Medium,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("project not found"));
    }

    #[tokio::test]
    async fn create_task_empty_name_returns_tool_error() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();

        let result = server
            .create_task(Parameters(CreateTaskParams {
                project_id: pid,
                version_id: repo.default_version_id(pid).unwrap(),
                name: "   ".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: PriorityParam::Medium,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("task name cannot be empty"));
    }

    #[tokio::test]
    async fn create_task_db_error_returns_internal_error() {
        let repo = Arc::new(FailingRepository);
        let server = PmpServer::new(repo);
        let err = server
            .create_task(Parameters(CreateTaskParams {
                project_id: 1,
                version_id: 1,
                name: "Task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: PriorityParam::Medium,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode(-32603));
    }

    #[tokio::test]
    async fn task_dependencies_can_be_added_read_and_removed() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let dependency = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Dependency",
                "",
                Status::Done,
                Priority::Medium,
            )
            .unwrap();
        let task = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let dependency_id = dependency.id.unwrap();
        let task_id = task.id.unwrap();

        let result = server
            .add_task_dependency(Parameters(TaskDependencyParams {
                task_id,
                depends_on_id: dependency_id,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));

        let result = server
            .get_task_dependencies(Parameters(TaskIdParams { task_id }))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
        assert_eq!(value["task_id"], task_id);
        assert_eq!(value["depends_on_ids"], serde_json::json!([dependency_id]));

        let result = server
            .get_task_dependents(Parameters(TaskIdParams {
                task_id: dependency_id,
            }))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
        assert_eq!(value["task_ids"], serde_json::json!([task_id]));

        server
            .remove_task_dependency(Parameters(TaskDependencyParams {
                task_id,
                depends_on_id: dependency_id,
            }))
            .await
            .unwrap();
        assert!(repo.get_dependencies(task_id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_task_dependency_rejects_cycles() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let a = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let b = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let c = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "C",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        let c_id = c.id.unwrap();

        repo.add_dependency(a_id, b_id).unwrap();
        repo.add_dependency(b_id, c_id).unwrap();
        let result = server
            .add_task_dependency(Parameters(TaskDependencyParams {
                task_id: c_id,
                depends_on_id: a_id,
            }))
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("dependency cycle detected"));
    }

    #[tokio::test]
    async fn task_dependency_tools_reject_unknown_tasks() {
        let (server, _) = server();
        let result = server
            .get_task_dependencies(Parameters(TaskIdParams { task_id: 999 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("task not found"));

        let result = server
            .add_task_dependency(Parameters(TaskDependencyParams {
                task_id: 999,
                depends_on_id: 1000,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("task not found"));
    }

    #[tokio::test]
    async fn call_tool_dispatches_list_tasks_with_json_params() {
        let (server, _, pid) = seeded_server();
        let mut args = serde_json::Map::new();
        args.insert("project_id".to_string(), serde_json::json!(pid));
        args.insert("status".to_string(), serde_json::json!("done"));

        let request = rmcp::model::CallToolRequestParams::new("list_tasks").with_arguments(args);
        let (client, server_stream) = tokio::io::duplex(1024);
        let running = rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(
            server,
            server_stream,
            None,
        );
        let peer = running.peer().clone();
        let context =
            rmcp::service::RequestContext::new(rmcp::model::NumberOrString::Number(1), peer);

        let result = rmcp::ServerHandler::call_tool(running.service(), request, context)
            .await
            .unwrap();
        drop(client);
        drop(running);

        let text = tool_text(result);
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Done);
    }

    async fn write_json_line<W>(writer: &mut W, value: &serde_json::Value)
    where
        W: tokio::io::AsyncWriteExt + Unpin,
    {
        let mut text = value.to_string();
        text.push('\n');
        writer.write_all(text.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    }

    async fn read_json_line<R>(reader: &mut R) -> serde_json::Value
    where
        R: tokio::io::AsyncBufReadExt + Unpin,
    {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("expected json, got: {line:?}"))
    }

    #[tokio::test]
    async fn mcp_server_e2e_lifecycle_and_list_tasks() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let default_version = repo.default_version_id(pid).unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Low task",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "High task",
            "",
            Status::Done,
            Priority::High,
        )
        .unwrap();

        let (client, server_stream) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            let server = PmpServer::new(repo);
            rmcp::service::serve_server(server, server_stream)
                .await
                .unwrap()
        });

        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = tokio::io::BufReader::new(reader);

        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        });
        write_json_line(&mut writer, &init_request).await;
        let init_response = read_json_line(&mut reader).await;
        assert_eq!(init_response["jsonrpc"], "2.0");
        assert_eq!(init_response["id"], 1);
        let capabilities = &init_response["result"]["capabilities"];
        assert!(capabilities["tools"].is_object());
        assert!(capabilities["resources"].is_object());
        assert_eq!(capabilities["tools"]["listChanged"], false);
        assert_eq!(capabilities["resources"]["listChanged"], false);

        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        write_json_line(&mut writer, &initialized).await;

        let tools_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        write_json_line(&mut writer, &tools_request).await;
        let tools_response = read_json_line(&mut reader).await;
        let tools = tools_response["result"]["tools"]
            .as_array()
            .expect("tools/list should return a tools array");
        assert!(tools.iter().any(|t| t["name"] == "list_tasks"));
        assert!(tools.iter().any(|t| t["name"] == "list_projects"));
        assert!(tools.iter().any(|t| t["name"] == "create_project"));
        assert!(tools.iter().any(|t| t["name"] == "update_task_status"));
        assert!(tools.iter().any(|t| t["name"] == "create_task"));
        assert!(tools.iter().any(|t| t["name"] == "get_task_dependencies"));
        assert!(tools.iter().any(|t| t["name"] == "get_task_dependents"));
        assert!(tools.iter().any(|t| t["name"] == "add_task_dependency"));
        assert!(tools.iter().any(|t| t["name"] == "remove_task_dependency"));
        assert!(tools.iter().any(|t| t["name"] == "set_task_dependencies"));
        assert!(tools.iter().any(|t| t["name"] == "list_task_notes"));
        assert!(tools.iter().any(|t| t["name"] == "add_task_note"));
        assert!(tools.iter().any(|t| t["name"] == "delete_task_note"));

        let create_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": {
                    "project_id": pid,
                    "version_id": default_version,
                    "name": "E2E task",
                    "context": "End-to-end context.",
                    "requirements": "- Must pass",
                    "benefits": "Confidence.",
                    "priority": "High"
                }
            }
        });
        write_json_line(&mut writer, &create_request).await;
        let create_response = read_json_line(&mut reader).await;
        let created_task: Task = serde_json::from_str(
            create_response["result"]["content"][0]["text"]
                .as_str()
                .expect("tool result should have text content"),
        )
        .unwrap();
        assert_eq!(created_task.name, "E2E task");
        assert_eq!(created_task.status, Status::Todo);
        assert_eq!(created_task.priority, Priority::High);
        assert!(created_task.description.contains("## Context"));

        let call_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "list_tasks",
                "arguments": { "project_id": pid, "status": "done" }
            }
        });
        write_json_line(&mut writer, &call_request).await;
        let call_response = read_json_line(&mut reader).await;
        let content = call_response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result should have text content");
        let tasks: Vec<Task> = serde_json::from_str(content).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Done);

        drop(writer);
        drop(reader);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn list_task_notes_returns_notes_for_task() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();
        repo.add_task_note(task_id, "first note").unwrap();
        repo.add_task_note(task_id, "second note").unwrap();

        let result = server
            .list_task_notes(Parameters(TaskIdParams { task_id }))
            .await
            .unwrap();
        let text = tool_text(result);
        let notes: Vec<TaskNote> = serde_json::from_str(&text).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].content, "second note");
        assert_eq!(notes[1].content, "first note");
    }

    #[tokio::test]
    async fn list_task_notes_unknown_task_returns_error() {
        let (server, _) = server();
        let result = server
            .list_task_notes(Parameters(TaskIdParams { task_id: 999 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("task not found"));
    }

    #[tokio::test]
    async fn add_task_note_appends_note() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        let result = server
            .add_task_note(Parameters(AddTaskNoteParams {
                task_id,
                content: "new note".to_string(),
            }))
            .await
            .unwrap();
        let text = tool_text(result);
        let note: TaskNote = serde_json::from_str(&text).unwrap();
        assert_eq!(note.task_id, task_id);
        assert_eq!(note.content, "new note");
        assert!(note.id.is_some());

        let notes = repo.list_task_notes(task_id).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "new note");
    }

    #[tokio::test]
    async fn add_task_note_empty_content_returns_error() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        let result = server
            .add_task_note(Parameters(AddTaskNoteParams {
                task_id,
                content: "   ".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let message = tool_text(result);
        assert!(message.contains("note content cannot be empty"));
    }

    #[tokio::test]
    async fn add_task_note_unknown_task_returns_error() {
        let (server, _) = server();
        let result = server
            .add_task_note(Parameters(AddTaskNoteParams {
                task_id: 999,
                content: "note".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("task not found"));
    }

    #[tokio::test]
    async fn delete_task_note_removes_note() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task = repo
            .create_task(
                pid,
                repo.default_version_id(pid).unwrap(),
                "Task",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();
        let note = repo.add_task_note(task_id, "to delete").unwrap();
        let note_id = note.id.unwrap();

        let result = server
            .delete_task_note(Parameters(NoteIdParams { note_id }))
            .await
            .unwrap();
        let text = tool_text(result);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["note_id"], note_id);

        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_task_note_unknown_note_returns_error() {
        let (server, _) = server();
        let result = server
            .delete_task_note(Parameters(NoteIdParams { note_id: 999 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("note not found"));
    }

    fn resource_text(result: ReadResourceResult) -> String {
        match result.contents.into_iter().next().unwrap() {
            ResourceContents::TextResourceContents { text, .. } => text,
            other => panic!("unexpected resource contents: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_task_rejects_version_from_another_project() {
        let (server, repo) = server();
        let alpha = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let beta = repo.create_project("Beta", "").unwrap().id.unwrap();
        let foreign_version = repo.default_version_id(beta).unwrap();
        let result = server
            .create_task(Parameters(CreateTaskParams {
                project_id: alpha,
                version_id: foreign_version,
                name: "Task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: PriorityParam::Medium,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("version does not belong to the project"));
    }

    #[tokio::test]
    async fn list_tasks_filters_by_version_id() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v0 = repo.default_version_id(pid).unwrap();
        let v1 = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
        repo.create_task(pid, v0, "Old", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, v1, "New", "", Status::Todo, Priority::Low)
            .unwrap();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: Some(v1),
                status: None,
                priority: None,
            }))
            .await
            .unwrap();
        let tasks: Vec<Task> = serde_json::from_str(&tool_text(result)).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "New");
    }

    #[tokio::test]
    async fn list_tasks_rejects_version_from_another_project() {
        let (server, repo) = server();
        let alpha = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let beta = repo.create_project("Beta", "").unwrap().id.unwrap();
        let result = server
            .list_tasks(Parameters(ListTasksParams {
                project_id: alpha,
                version_id: Some(repo.default_version_id(beta).unwrap()),
                status: None,
                priority: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("version does not belong to the project"));
    }

    #[tokio::test]
    async fn list_versions_returns_open_before_released() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let released = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
        repo.update_version(released, "v0.1.0", "", VersionStatus::Released)
            .unwrap();
        repo.create_version(pid, "v0.2.0", "").unwrap();
        let result = server
            .list_versions(Parameters(ListVersionsParams { project_id: pid }))
            .await
            .unwrap();
        let versions: Vec<Version> = serde_json::from_str(&tool_text(result)).unwrap();
        let names: Vec<&str> = versions.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["v0.2.0", "v0", "v0.1.0"]);
    }

    #[tokio::test]
    async fn list_versions_unknown_project_returns_tool_error() {
        let (server, _) = server();
        let result = server
            .list_versions(Parameters(ListVersionsParams { project_id: 999 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(tool_text(result).contains("project not found"));
    }

    #[tokio::test]
    async fn create_version_defaults_to_open_and_accepts_released() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let open = server
            .create_version(Parameters(CreateVersionParams {
                project_id: pid,
                name: "v0.1.0".to_string(),
                description: Some("First".to_string()),
                status: None,
            }))
            .await
            .unwrap();
        let open: Version = serde_json::from_str(&tool_text(open)).unwrap();
        assert_eq!(open.status, VersionStatus::Open);
        assert_eq!(open.description, "First");

        let released = server
            .create_version(Parameters(CreateVersionParams {
                project_id: pid,
                name: "v0.2.0".to_string(),
                description: None,
                status: Some("released".to_string()),
            }))
            .await
            .unwrap();
        let released: Version = serde_json::from_str(&tool_text(released)).unwrap();
        assert_eq!(released.status, VersionStatus::Released);
    }

    #[tokio::test]
    async fn create_version_rejects_empty_duplicate_and_invalid_status() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let params = |name: &str, status: Option<&str>| CreateVersionParams {
            project_id: pid,
            name: name.to_string(),
            description: None,
            status: status.map(str::to_string),
        };
        let empty = server
            .create_version(Parameters(params("  ", None)))
            .await
            .unwrap();
        assert_eq!(empty.is_error, Some(true));
        assert!(tool_text(empty).contains("version name cannot be empty"));

        server
            .create_version(Parameters(params("v0.1.0", None)))
            .await
            .unwrap();
        let duplicate = server
            .create_version(Parameters(params("v0.1.0", None)))
            .await
            .unwrap();
        assert_eq!(duplicate.is_error, Some(true));
        assert!(tool_text(duplicate).contains("version already exists"));

        let invalid = server
            .create_version(Parameters(params("v0.3.0", Some("closed"))))
            .await
            .unwrap();
        assert_eq!(invalid.is_error, Some(true));
        assert!(tool_text(invalid).contains("invalid version status"));
    }

    #[tokio::test]
    async fn resource_templates_include_versions_and_version_tasks() {
        let (server, _) = server();
        let result = server.list_resource_templates().await.unwrap();
        let uris: Vec<&str> = result
            .resource_templates
            .iter()
            .map(|t| t.uri_template.as_str())
            .collect();
        assert!(uris.contains(&"pmp://projects/{id}/versions"));
        assert!(uris.contains(&"pmp://versions/{id}/tasks"));
    }

    #[tokio::test]
    async fn read_project_versions_returns_versions() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        repo.create_version(pid, "v0.1.0", "").unwrap();
        let text = resource_text(
            server
                .read_resource(&format!("pmp://projects/{pid}/versions"))
                .await
                .unwrap(),
        );
        let versions: Vec<Version> = serde_json::from_str(&text).unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn read_version_tasks_returns_only_that_version() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();
        let v0 = repo.default_version_id(pid).unwrap();
        let v1 = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
        repo.create_task(pid, v1, "Scoped", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, v0, "Other", "", Status::Todo, Priority::Low)
            .unwrap();
        let text = resource_text(
            server
                .read_resource(&format!("pmp://versions/{v1}/tasks"))
                .await
                .unwrap(),
        );
        let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Scoped");
    }

    #[tokio::test]
    async fn read_version_tasks_rejects_invalid_id() {
        let (server, _) = server();
        let err = server
            .read_resource("pmp://versions/abc/tasks")
            .await
            .unwrap_err();
        assert!(err.message.contains("invalid version id"));
    }

    #[tokio::test]
    async fn project_version_flow_create_version_task_and_filter() {
        let (server, repo) = server();
        let pid = repo.create_project("Alpha", "").unwrap().id.unwrap();

        let version: Version = serde_json::from_str(&tool_text(
            server
                .create_version(Parameters(CreateVersionParams {
                    project_id: pid,
                    name: "v0.1.0".to_string(),
                    description: None,
                    status: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        let version_id = version.id.unwrap();

        let task: Task = serde_json::from_str(&tool_text(
            server
                .create_task(Parameters(CreateTaskParams {
                    project_id: pid,
                    version_id,
                    name: "Release work".to_string(),
                    context: "ctx".to_string(),
                    requirements: "req".to_string(),
                    benefits: "ben".to_string(),
                    priority: PriorityParam::High,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(task.version_id, version_id);

        let filtered: Vec<Task> = serde_json::from_str(&tool_text(
            server
                .list_tasks(Parameters(ListTasksParams {
                    project_id: pid,
                    version_id: Some(version_id),
                    status: None,
                    priority: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, task.id);
    }
}

#[cfg(test)]
mod priority_param_tests {
    use super::*;

    fn schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap()
    }

    #[test]
    fn create_task_priority_schema_is_plain_string_enum() {
        let schema = schema_for::<CreateTaskParams>();
        let prop = &schema["properties"]["priority"];
        let refd = prop.get("$ref").and_then(|r| r.as_str());
        let def = match refd {
            Some(r) => &schema["$defs"][r.trim_start_matches("#/$defs/")],
            None => prop,
        };
        assert_eq!(def["type"], "string", "priority must be a string: {def}");
        assert_eq!(
            def["enum"],
            serde_json::json!(["Low", "Medium", "High"]),
            "priority must enumerate the three levels: {def}"
        );
        assert!(
            !prop.to_string().contains("null"),
            "priority must not be nullable: {prop}"
        );
        let required = schema["required"].as_array().unwrap();
        assert!(!required.iter().any(|r| r == "priority"));
    }

    #[test]
    fn create_task_params_accept_named_priority() {
        let params: CreateTaskParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b",
            "priority": "High"
        }))
        .unwrap();
        assert_eq!(Priority::from(params.priority), Priority::High);
    }

    #[test]
    fn create_task_params_default_to_medium_when_omitted() {
        let params: CreateTaskParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b"
        }))
        .unwrap();
        assert_eq!(Priority::from(params.priority), Priority::Medium);
    }

    #[test]
    fn create_task_params_reject_unknown_priority() {
        let err = serde_json::from_value::<CreateTaskParams>(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b",
            "priority": "Urgent"
        }))
        .err()
        .expect("unknown priority must be rejected");
        assert!(err.to_string().contains("Urgent"), "{err}");
    }

    #[test]
    fn list_tasks_params_accept_named_priority() {
        let params: ListTasksParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "priority": "High"
        }))
        .unwrap();
        assert_eq!(params.priority.map(Priority::from), Some(Priority::High));
    }
}

#[cfg(test)]
mod instructions_tests {
    use super::*;

    #[test]
    fn server_instructions_describe_status_workflow() {
        let repo = Arc::new(crate::db::stub::StubRepository::default());
        let info = PmpServer::new(repo).get_info();
        let text = info.instructions.expect("instructions should be set");
        for needle in ["update_task_status", "in_progress", "done", "add_task_note"] {
            assert!(text.contains(needle), "instructions must mention {needle}");
        }
    }
}
