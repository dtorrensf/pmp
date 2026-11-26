// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, JsonObject, ReadResourceResult, ResourceContents,
};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::task;

use crate::db::repository::Repository;
use crate::domain::error::AppError;

/// Tool-error texts for missing records.
pub(super) const PROJECT_NOT_FOUND: &str = "project not found";
pub(super) const VERSION_NOT_FOUND: &str = "version not found";
pub(super) const TASK_NOT_FOUND: &str = "task not found";
pub(super) const TASK_OR_VERSION_NOT_FOUND: &str = "task or version not found";
pub(super) const NOTE_NOT_FOUND: &str = "note not found";
pub(super) const SUMMARY_NOT_FOUND: &str = "summary not found";
pub(super) const RESOURCE_NOT_FOUND_MESSAGE: &str = "resource not found";

/// Resource URIs. Each template is listed to clients and parsed by
/// `read_resource`; the test `every_listed_template_is_readable` keeps them in sync.
pub(super) const PROJECTS_URI: &str = "pmp://projects";
pub(super) const PROJECTS_PREFIX: &str = "pmp://projects/";
pub(super) const VERSIONS_PREFIX: &str = "pmp://versions/";
pub(super) const SUMMARIES_PREFIX: &str = "pmp://summaries/";
pub(super) const PROJECT_TEMPLATE: &str = "pmp://projects/{id}";
pub(super) const PROJECT_TASKS_TEMPLATE: &str = "pmp://projects/{id}/tasks";
pub(super) const PROJECT_VERSIONS_TEMPLATE: &str = "pmp://projects/{id}/versions";
pub(super) const VERSION_TASKS_TEMPLATE: &str = "pmp://versions/{id}/tasks";
pub(super) const VERSION_SUMMARIES_TEMPLATE: &str = "pmp://versions/{id}/summaries";
pub(super) const SUMMARY_TEMPLATE: &str = "pmp://summaries/{id}";

/// Upper bound for the textual JSON representation of one tool or resource
/// response. Structured tool responses repeat the same JSON in
/// `structuredContent`, so the wire payload can be roughly twice this size.
/// Pagination keeps normal pages below the cap; it protects the client context
/// window when a single record (a summary snapshot, a note) is still large.
pub(super) const MAX_RESPONSE_BYTES: usize = 1_000_000;

/// Items per page when listing resources or resource templates.
pub(super) const RESOURCE_PAGE_SIZE: usize = 50;

/// Internal failure sent to the client. The detail goes to the server log
/// only: it can contain file paths or SQL text.
pub(super) fn internal_error(detail: impl std::fmt::Display) -> ErrorData {
    tracing::error!(error = %detail, "internal MCP error");
    ErrorData::internal_error(
        "internal error",
        Some(serde_json::json!({ "code": "internal", "retryable": false })),
    )
}

pub(super) async fn execute_repository<T, F>(
    repo: Arc<dyn Repository>,
    timeout: Duration,
    operation: F,
) -> Result<Result<T, AppError>, ErrorData>
where
    T: Send + 'static,
    F: FnOnce(Arc<dyn Repository>) -> Result<T, AppError> + Send + 'static,
{
    execute_repository_with_timeout(repo, operation, timeout).await
}

pub(super) async fn execute_repository_with_timeout<T, F>(
    repo: Arc<dyn Repository>,
    operation: F,
    timeout: Duration,
) -> Result<Result<T, AppError>, ErrorData>
where
    T: Send + 'static,
    F: FnOnce(Arc<dyn Repository>) -> Result<T, AppError> + Send + 'static,
{
    let handle = task::spawn_blocking(move || operation(repo));
    match tokio::time::timeout(timeout, handle).await {
        Ok(joined) => joined.map_err(internal_error),
        Err(_) => Err(internal_error("repository operation timed out")),
    }
}

pub(super) async fn read_json_resource<T, F>(
    repo: Arc<dyn Repository>,
    timeout: Duration,
    uri: &str,
    operation: F,
) -> Result<ReadResourceResult, ErrorData>
where
    T: Serialize + Send + 'static,
    F: FnOnce(Arc<dyn Repository>) -> Result<T, AppError> + Send + 'static,
{
    let value = execute_repository(repo, timeout, operation)
        .await?
        .map_err(app_error_to_mcp)?;
    let text = serde_json::to_string_pretty(&value).map_err(internal_error)?;
    if text.len() > MAX_RESPONSE_BYTES {
        return Err(response_too_large());
    }
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(text, uri).with_mime_type("application/json"),
    ]))
}

pub(super) fn json_content<T: Serialize>(value: &T) -> Result<ContentBlock, ErrorData> {
    let bytes = serde_json::to_vec(value).map_err(internal_error)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(response_too_large());
    }
    ContentBlock::json(value).map_err(internal_error)
}

/// Generates the `outputSchema` declared by a tool. MCP requires an object at
/// the root; tool results that are arrays are wrapped in `{"items": [...]}`.
pub(super) fn output_schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    rmcp::handler::server::tool::schema_for_output::<T>().unwrap_or_else(|error| {
        panic!(
            "invalid MCP output schema for {}: {error}",
            std::any::type_name::<T>()
        )
    })
}

/// Converts a tool result to the object stored in `structuredContent`.
fn structured_value<T: Serialize>(value: &T) -> Result<serde_json::Value, ErrorData> {
    let value = serde_json::to_value(value).map_err(internal_error)?;
    match value {
        serde_json::Value::Object(_) => Ok(value),
        serde_json::Value::Array(items) => Ok(serde_json::json!({ "items": items })),
        _ => Err(internal_error(
            "a tool returned a scalar where structured content requires an object",
        )),
    }
}

/// Error returned when one response would exceed [`MAX_RESPONSE_BYTES`].
pub(super) fn response_too_large() -> ErrorData {
    ErrorData::invalid_params(
        format!("response exceeds {MAX_RESPONSE_BYTES} bytes; narrow the query or use pagination"),
        None::<serde_json::Value>,
    )
}

/// Slices `items` using an offset cursor (`"0"`, `"50"`, ...). Returns the page
/// and the cursor for the next page when one exists.
pub(super) fn paginate_page<T>(
    items: Vec<T>,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<(Vec<T>, Option<String>), ErrorData> {
    let mut items = items;
    let start = match cursor {
        Some(cursor) if !cursor.is_empty() => cursor
            .parse::<usize>()
            .map_err(|_| ErrorData::invalid_params("invalid cursor", None))?,
        _ => 0,
    };
    if start > items.len() {
        return Err(ErrorData::invalid_params("cursor is out of range", None));
    }
    let total = items.len();
    let end = (start + page_size).min(total);
    let page = items.drain(start..end).collect();
    let next_cursor = (end < total).then(|| end.to_string());
    Ok((page, next_cursor))
}

pub(super) fn tool_success<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let mut result = CallToolResult::success(vec![json_content(value)?]);
    result.structured_content = Some(structured_value(value)?);
    Ok(result)
}

pub(super) fn tool_result<T: Serialize>(
    result: Result<T, AppError>,
    not_found_message: &str,
) -> Result<CallToolResult, ErrorData> {
    let error = match result {
        Ok(value) => return tool_success(&value),
        Err(error) => error,
    };
    // Failures a client can act on stay in the tool result: the human text
    // comes first, followed by the machine-readable `ErrorInfo`.
    let message = match &error {
        AppError::NotFound => not_found_message.to_string(),
        AppError::DependencyCycle => "dependency cycle detected".to_string(),
        AppError::DbBusy | AppError::DbError(_) | AppError::Internal => {
            return Err(app_error_to_mcp(error));
        }
        other => other.user_message(),
    };
    Ok(CallToolResult::error(vec![
        ContentBlock::text(message),
        json_content(&error.info())?,
    ]))
}

/// Runs a repository operation for a tool and maps the outcome to a tool
/// result, using `not_found` as the text for a missing record.
pub(super) async fn run_tool<T, F>(
    repo: Arc<dyn Repository>,
    timeout: Duration,
    not_found: &str,
    operation: F,
) -> Result<CallToolResult, ErrorData>
where
    T: Serialize + Send + 'static,
    F: FnOnce(Arc<dyn Repository>) -> Result<T, AppError> + Send + 'static,
{
    let result = execute_repository(repo, timeout, operation).await?;
    tool_result(result, not_found)
}

/// The single mapping from domain errors to MCP protocol errors, used by
/// tools and resources alike. Database errors are logged on the server and
/// reported to the client as a generic internal error.
pub(super) fn app_error_to_mcp(err: AppError) -> ErrorData {
    match err {
        AppError::NotFound => ErrorData::new(
            ErrorCode::RESOURCE_NOT_FOUND,
            RESOURCE_NOT_FOUND_MESSAGE.to_string(),
            Some(serde_json::json!({ "code": err.code() })),
        ),
        AppError::DbError(source) => internal_error(source),
        AppError::Internal => internal_error("internal error"),
        other => ErrorData::invalid_params(
            other.user_message(),
            Some(serde_json::to_value(other.info()).unwrap_or_default()),
        ),
    }
}
