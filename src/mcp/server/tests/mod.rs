// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

mod contract;
mod dependencies;
mod errors;
mod notes;
mod projects;
mod protocol;
mod records;
mod resources;
mod schema;
mod summaries;
mod tasks;
mod versions;

use crate::db::repository::RepositoryPolicy;
use std::sync::Arc;

use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorCode, ReadResourceResult, ResourceContents};

use crate::application::tasks::TaskSummary;
use crate::db::repository::{
    DependencyRepository, NoteRepository, ProjectRepository, SummaryRepository, TaskRepository,
    VersionRepository,
};
use crate::db::stub::StubRepository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};
use crate::domain::validation::MAX_TEXT_LEN;
use crate::domain::version::{Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;
use crate::mcp::server::PriorityParam;

use crate::domain::task_description::compose_description;
use crate::mcp::server::{
    AddTaskNoteParams, CompactVersionParams, CreateProjectParams, CreateTaskParams,
    CreateVersionParams, DeleteVersionSummaryParams, ExportVersionSummaryParams,
    ListTaskNotesParams, ListTasksParams, ListVersionSummariesParams, ListVersionsParams,
    MAX_RESPONSE_BYTES, MoveTaskParams, NoteIdParams, PmpServer, ReadVersionSummaryParams,
    SetTaskDependenciesParams, TaskDependencyParams, TaskIdParams, UpdateProjectParams,
    UpdateTaskParams, UpdateTaskPriorityParams, UpdateTaskStatusParams, UpdateVersionParams,
    VersionIdParams, app_error_to_mcp, json_content, paginate_page,
};

use crate::mcp::server::*;

fn server() -> (PmpServer, Arc<StubRepository>) {
    let repo = Arc::new(StubRepository::default());
    (PmpServer::new(repo.clone()), repo)
}

fn seeded_server() -> (PmpServer, Arc<StubRepository>, i64) {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
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

/// The structured `ErrorInfo` block a tool error carries next to its text.
fn tool_error_info(result: &CallToolResult) -> serde_json::Value {
    let raw = result
        .content
        .iter()
        .skip(1)
        .find_map(|content| content.as_text().map(|text| text.text.clone()))
        .expect("tool error should carry a structured block");
    serde_json::from_str(&raw).expect("structured block should be json")
}

fn tool_text(result: CallToolResult) -> String {
    result
        .content
        .into_iter()
        .next()
        .and_then(|c| c.as_text().map(|t| t.text.clone()))
        .expect("tool result should have text content")
}

/// Extracts the `tasks` array from a `list_tasks` page response. Items are
/// light summaries, so the description is not part of them.
fn page_tasks(text: &str) -> Vec<TaskSummary> {
    let page: serde_json::Value = serde_json::from_str(text).unwrap();
    serde_json::from_value(page["tasks"].clone()).unwrap()
}

struct FailingRepository;

impl ProjectRepository for FailingRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        Ok(vec![])
    }

    fn create_project(&self, _: &str, _: &str, _: Option<&str>) -> Result<Project, AppError> {
        unimplemented!()
    }

    fn get_project(&self, _: i64) -> Result<Project, AppError> {
        Err(AppError::DbError(rusqlite::Error::InvalidQuery))
    }

    fn update_project(
        &self,
        _: i64,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Project, AppError> {
        unimplemented!()
    }

    fn delete_project(&self, _: i64) -> Result<(), AppError> {
        unimplemented!()
    }
}

impl VersionRepository for FailingRepository {
    fn list_versions_by_project(&self, _: i64) -> Result<Vec<Version>, AppError> {
        Err(AppError::DbError(rusqlite::Error::InvalidQuery))
    }

    fn create_version_with_status(
        &self,
        _: i64,
        _: &str,
        _: &str,
        _: VersionStatus,
    ) -> Result<Version, AppError> {
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
}

impl TaskRepository for FailingRepository {
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

    fn move_task(&self, _: i64, _: i64) -> Result<Task, AppError> {
        unimplemented!()
    }

    fn delete_task(&self, _: i64) -> Result<(), AppError> {
        unimplemented!()
    }

    fn delete_done_tasks_by_version(&self, _: i64) -> Result<(), AppError> {
        unimplemented!()
    }
}

impl DependencyRepository for FailingRepository {
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

    fn get_dependents(&self, _: i64) -> Result<Vec<i64>, AppError> {
        unimplemented!()
    }

    fn would_create_cycle(&self, _: i64, _: i64) -> Result<bool, AppError> {
        unimplemented!()
    }
}

impl NoteRepository for FailingRepository {
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

impl SummaryRepository for FailingRepository {
    fn list_version_summaries(&self, _: i64) -> Result<Vec<VersionSummary>, AppError> {
        unimplemented!()
    }

    fn get_version_summary(&self, _: i64) -> Result<VersionSummary, AppError> {
        unimplemented!()
    }

    fn create_version_summary(&self, _: i64, _: &str, _: &str) -> Result<VersionSummary, AppError> {
        unimplemented!()
    }

    fn delete_version_summary(&self, _: i64) -> Result<(), AppError> {
        unimplemented!()
    }

    fn compact_version(&self, _: i64, _: &str, _: &str) -> Result<VersionSummary, AppError> {
        unimplemented!()
    }
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

fn resource_text(result: ReadResourceResult) -> String {
    match result.contents.into_iter().next().unwrap() {
        ResourceContents::TextResourceContents { text, .. } => text,
        other => panic!("unexpected resource contents: {other:?}"),
    }
}
