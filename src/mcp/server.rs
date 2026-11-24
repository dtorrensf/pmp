// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceRequestParams,
    ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::tool_handler;

use crate::config::DEFAULT_MCP_TIMEOUT;
use crate::db::repository::Repository;

mod common;
mod dependencies;
mod notes;
mod params;
mod projects;
mod resources;
mod summaries;
mod tasks;
#[cfg(test)]
mod tests;
mod versions;

use common::*;
pub use params::*;

pub struct PmpServer {
    repo: Arc<dyn Repository>,
    /// Upper bound for one repository operation, from the unified config.
    timeout: Duration,
}

impl PmpServer {
    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            timeout: DEFAULT_MCP_TIMEOUT,
        }
    }

    /// Uses `timeout` for every repository operation of this server. A zero
    /// timeout would fail every call, so it is ignored with a warning.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        if timeout.is_zero() {
            tracing::warn!("ignoring a zero MCP timeout");
            return self;
        }
        self.timeout = timeout;
        self
    }

    /// Every tool of the server, registered by domain module.
    fn tool_router() -> ToolRouter<Self> {
        Self::tool_router_projects()
            + Self::tool_router_versions()
            + Self::tool_router_tasks()
            + Self::tool_router_dependencies()
            + Self::tool_router_notes()
            + Self::tool_router_summaries()
    }
}

#[tool_handler]
impl ServerHandler for PmpServer {
    async fn list_resources(
        &self,
        request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.list_resources_page(request.as_ref().and_then(|r| r.cursor.as_deref()))
            .await
    }

    async fn list_resource_templates(
        &self,
        request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.list_resource_templates_page(request.as_ref().and_then(|r| r.cursor.as_deref()))
            .await
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

pub async fn serve(repo: Arc<dyn Repository>, timeout: Duration) -> Result<()> {
    let server = PmpServer::new(repo).with_timeout(timeout);
    let service = rmcp::service::serve_server(server, rmcp::transport::io::stdio()).await?;
    eprintln!("[pmp] MCP server listening on stdio");
    service.waiting().await?;
    Ok(())
}
