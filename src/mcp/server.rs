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
use crate::mcp::task_description::compose_description;

pub struct PmpServer {
    repo: Arc<dyn Repository>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    pub project_id: i64,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskStatusParams {
    pub task_id: i64,
    pub status: Status,
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
    pub name: String,
    pub context: String,
    pub requirements: String,
    pub benefits: String,
    pub priority: Option<String>,
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
                .with_description("Tasks for a project. Use to see all tasks and their status for a specific project.")
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
            uri if uri.starts_with("pmp://projects/") => {
                let tail = &uri["pmp://projects/".len()..];
                if let Some(prefix) = tail.strip_suffix("/tasks") {
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
        description = "List tasks for a project. Use when the user asks about their tasks, to-dos, pending work, or project progress. Supports filtering by status (todo, in_progress, done) and priority (Low, Medium, High)."
    )]
    async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let repo = self.repo.clone();
        let result = task::spawn_blocking(move || {
            repo.get_project(params.project_id)?;
            repo.list_tasks_by_project(params.project_id)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None::<serde_json::Value>))?;

        match result {
            Ok(tasks) => {
                let filtered: Vec<Task> = tasks
                    .into_iter()
                    .filter(|t| {
                        params.status.is_none_or(|s| t.status == s)
                            && params.priority.is_none_or(|p| t.priority == p)
                    })
                    .collect();
                let content = ContentBlock::json(&filtered)
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
        name = "create_task",
        description = "Create a new task in a project. Use when the user wants to add a task, to-do, or work item. Requires the project id, task name, and a structured description with context, requirements, and benefits. Optionally accepts a priority (Low, Medium, High); defaults to Medium."
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
            let priority = get_priority(params.priority.as_deref().unwrap_or("Medium"));
            repo.create_task(
                params.project_id,
                &params.name,
                &description,
                Status::Todo,
                priority,
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
        description = "Switch the status of a task. Use when the user (or an AI agent) starts working on a task (switch to In Progress) or finishes it (switch to Done). Rejects Done status if the task has incomplete dependencies."
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
        description = "List the task ids that a task depends on. Use to inspect blockers before planning or implementing a task."
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
        description = "List the task ids that depend on a task. Use to understand the impact of changing or deleting a task."
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
        description = "Make one task depend on another. The task_id depends on depends_on_id. Rejects self-dependencies and circular dependencies."
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
        description = "List notes for a task. Use to read progress updates, context, and reminders attached to a task."
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
        description = "Add a note to a task. Use to append context, progress updates, or reminders."
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

fn get_priority(priority: &str) -> Priority {
    match priority {
        "High" => Priority::High,
        "Medium" => Priority::Medium,
        "Low" => Priority::Low,
        _ => Priority::Medium,
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
            .with_instructions("Personal project manager. Use for queries about user's projects, tasks, and productivity tracking.")
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
    use rmcp::model::{CallToolResult, ErrorCode};

    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::domain::error::AppError;
    use crate::domain::project::Project;
    use crate::domain::task::{Priority, Status, Task, TaskNote};

    use super::{
        AddTaskNoteParams, CreateTaskParams, ListTasksParams, NoteIdParams, PmpServer,
        TaskDependencyParams, TaskIdParams, UpdateTaskStatusParams,
    };

    fn server() -> (PmpServer, Arc<StubRepository>) {
        let repo = Arc::new(StubRepository::default());
        (PmpServer::new(repo.clone()), repo)
    }

    fn seeded_server() -> (PmpServer, Arc<StubRepository>, i64) {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        repo.create_task(pid, "Low Todo", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, "High Done", "", Status::Done, Priority::High)
            .unwrap();
        repo.create_task(
            pid,
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
            _: &str,
            _: &str,
            _: Status,
            _: Priority,
        ) -> Result<Task, AppError> {
            unimplemented!()
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

        fn add_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
            unimplemented!()
        }

        fn remove_dependency(&self, _: i64, _: i64) -> Result<(), AppError> {
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
        repo.create_task(pid, "Low task", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, "High task", "", Status::InProgress, Priority::High)
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
                status: None,
                priority: Some(Priority::High),
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
                status: Some(Status::Todo),
                priority: Some(Priority::Low),
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
    async fn update_task_status_rejects_done_with_incomplete_deps() {
        let (server, repo) = server();
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let task_a = repo
            .create_task(pid, "Task A", "", Status::Todo, Priority::High)
            .unwrap();
        let task_b = repo
            .create_task(pid, "Task B", "", Status::Todo, Priority::High)
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
            .create_task(pid, "Task A", "", Status::Done, Priority::High)
            .unwrap();
        let task_b = repo
            .create_task(pid, "Task B", "", Status::Todo, Priority::High)
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
                name: "New task".to_string(),
                context: "We need this.".to_string(),
                requirements: "- Do it well".to_string(),
                benefits: "Ship faster.".to_string(),
                priority: None,
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
                name: "High priority task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: Some("High".to_string()),
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
                name: "Task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: None,
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
                name: "   ".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: None,
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
                name: "Task".to_string(),
                context: "".to_string(),
                requirements: "".to_string(),
                benefits: "".to_string(),
                priority: None,
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
            .create_task(pid, "Dependency", "", Status::Done, Priority::Medium)
            .unwrap();
        let task = repo
            .create_task(pid, "Task", "", Status::Todo, Priority::Medium)
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
            .create_task(pid, "A", "", Status::Todo, Priority::Medium)
            .unwrap();
        let b = repo
            .create_task(pid, "B", "", Status::Todo, Priority::Medium)
            .unwrap();
        let c = repo
            .create_task(pid, "C", "", Status::Todo, Priority::Medium)
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
        repo.create_task(pid, "Low task", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(pid, "High task", "", Status::Done, Priority::High)
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
        assert!(tools.iter().any(|t| t["name"] == "update_task_status"));
        assert!(tools.iter().any(|t| t["name"] == "create_task"));
        assert!(tools.iter().any(|t| t["name"] == "get_task_dependencies"));
        assert!(tools.iter().any(|t| t["name"] == "get_task_dependents"));
        assert!(tools.iter().any(|t| t["name"] == "add_task_dependency"));
        assert!(tools.iter().any(|t| t["name"] == "remove_task_dependency"));
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
            .create_task(pid, "Task", "", Status::Todo, Priority::Medium)
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
            .create_task(pid, "Task", "", Status::Todo, Priority::Medium)
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
            .create_task(pid, "Task", "", Status::Todo, Priority::Medium)
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
            .create_task(pid, "Task", "", Status::Todo, Priority::Medium)
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
}
