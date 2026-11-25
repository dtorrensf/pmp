// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;
use crate::domain::version::VersionStatus;
use application::versions::{NewVersion, VersionEdit};

use super::*;

#[tool_router(router = tool_router_versions, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "list_versions",
        description = "List the versions (release lines) of a project, open ones first, then backlog, then released. Use to find the version_id that tasks must be created in, or to check which versions are active or released."
    )]
    pub(super) async fn list_versions(
        &self,
        Parameters(params): Parameters<ListVersionsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| application::versions::list(repo.as_ref(), params.project_id),
        )
        .await
    }

    #[tool(
        name = "create_version",
        description = "Create a version (release line) in a project. Use when planning a new release, for example v0.2.0. Name must be unique within the project and not empty. Status is backlog (default), open or released."
    )]
    pub(super) async fn create_version(
        &self,
        Parameters(params): Parameters<CreateVersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| {
                application::versions::create(
                    repo.as_ref(),
                    NewVersion {
                        project_id: params.project_id,
                        name: params.name,
                        description: params.description.unwrap_or_default(),
                        status: params.status.unwrap_or(VersionStatus::Backlog),
                    },
                )
            },
        )
        .await
    }

    #[tool(
        name = "update_version",
        description = "Rename a version and/or change its description or status. Requires the version id and a non-empty name. Omit description or status to keep the current ones. Status is backlog, open or released. Fails when the name already exists in the same project. Use to edit version metadata instead of editing the database."
    )]
    pub(super) async fn update_version(
        &self,
        Parameters(params): Parameters<UpdateVersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            VERSION_NOT_FOUND,
            move |repo| {
                application::versions::edit(
                    repo.as_ref(),
                    params.version_id,
                    VersionEdit {
                        name: params.name,
                        description: params.description,
                        status: params.status,
                    },
                )
            },
        )
        .await
    }

    #[tool(
        name = "get_version",
        description = "Get one version by id: its details, task counts by status and number of compaction summaries. Use when the user asks about a specific release line or before deleting it."
    )]
    pub(super) async fn get_version(
        &self,
        Parameters(params): Parameters<VersionIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            VERSION_NOT_FOUND,
            move |repo| application::records::version_detail(repo.as_ref(), version_id),
        )
        .await
    }

    #[tool(
        name = "delete_version",
        description = "Delete a version with its tasks, notes and summaries. Irreversible. Refused with error code confirmation_required while it has active (todo or in_progress) tasks, unless confirm is true; the refusal lists the blocking task ids. Done tasks are removed without confirmation, so prefer compact_version to keep history. Only delete when the user explicitly asks."
    )]
    pub(super) async fn delete_version(
        &self,
        Parameters(params): Parameters<DeleteVersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        let confirm = params.confirm;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::records::delete_version(repo.as_ref(), version_id, confirm)
        })
        .await?;
        tool_result(result, VERSION_NOT_FOUND)
    }
}
