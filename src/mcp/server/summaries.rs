// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;
use crate::application::summaries::SummaryPage;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::version_summary::VersionSummary;

use super::outputs::{DeletedSummary, ExportedSummary};
use super::*;

#[tool_router(router = tool_router_summaries, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "preview_compact_version",
        description = "Preview the JSON snapshot that compact_version would freeze for a version, without changing anything. Includes the version, its done tasks (todo tasks are not frozen), their notes and dependencies, plus the previous summary when recompacting. Use before compact_version to review what will be persisted.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = output_schema::<CompactSnapshot>()
    )]
    pub(super) async fn preview_compact_version(
        &self,
        Parameters(params): Parameters<VersionIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            VERSION_NOT_FOUND,
            move |repo| application::summaries::preview(repo.as_ref(), version_id),
        )
        .await
    }

    #[tool(
        name = "compact_version",
        description = "Compact a version into a frozen summary. Freezes only its done tasks (with their notes/dependencies) and deletes them; todo tasks stay in the version. Fails if any task is in_progress, or if there is nothing to freeze (no done tasks and no previous summary). If todo tasks remain, the version stays open; otherwise it is marked released by default. When recompacting, the new summary must preserve the relevant knowledge from the previous summary (available in the preview as previous_summary) while incorporating the new tasks, notes and dependencies with their completed_at.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        ),
        output_schema = output_schema::<VersionSummary>()
    )]
    pub(super) async fn compact_version(
        &self,
        Parameters(params): Parameters<CompactVersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        let summary = params.summary;
        let mark_released = params.mark_released.unwrap_or(true);
        run_tool(
            self.repo.clone(),
            self.timeout,
            VERSION_NOT_FOUND,
            move |repo| {
                application::summaries::compact(repo.as_ref(), version_id, &summary, mark_released)
            },
        )
        .await
    }

    #[tool(
        name = "list_version_summaries",
        description = "List the summaries of a version, newest first. Results are paginated: use limit (default 100, max 500) and offset, and check has_more/total to fetch the next page. A version holds at most one summary; after compaction its tasks live only in the summary snapshot. Use to review what was frozen.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = output_schema::<SummaryPage>()
    )]
    pub(super) async fn list_version_summaries(
        &self,
        Parameters(params): Parameters<ListVersionSummariesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        let limit = params.limit;
        let offset = params.offset.unwrap_or(0);
        run_tool(
            self.repo.clone(),
            self.timeout,
            VERSION_NOT_FOUND,
            move |repo| application::summaries::page(repo.as_ref(), version_id, limit, offset),
        )
        .await
    }

    #[tool(
        name = "read_version_summary",
        description = "Read one version summary by id, restricted to the given version. Returns the summary prose plus its frozen snapshot JSON. Use to inspect what a compacted version contained.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = output_schema::<VersionSummary>()
    )]
    pub(super) async fn read_version_summary(
        &self,
        Parameters(params): Parameters<ReadVersionSummaryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        let summary_id = params.summary_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            SUMMARY_NOT_FOUND,
            move |repo| application::summaries::read(repo.as_ref(), version_id, summary_id),
        )
        .await
    }

    #[tool(
        name = "delete_version_summary",
        description = "Delete a version summary by its id, restricted to the given version. Use to remove a compaction artifact; tasks already deleted during compaction are not restored.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = output_schema::<DeletedSummary>()
    )]
    pub(super) async fn delete_version_summary(
        &self,
        Parameters(params): Parameters<DeleteVersionSummaryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let version_id = params.version_id;
        let summary_id = params.summary_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            SUMMARY_NOT_FOUND,
            move |repo| {
                application::summaries::delete(repo.as_ref(), version_id, summary_id)?;
                Ok(serde_json::json!({ "summary_id": summary_id }))
            },
        )
        .await
    }

    #[tool(
        name = "export_version_summary",
        description = "Export a version summary to a self-contained Markdown file with Mermaid charts (status pie, priority pie, dependency flowchart) and a task table. The file is always written inside the app's exports folder; the tool takes no destination path. Use to share or render a compacted version outside the app. Returns the written path.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        ),
        output_schema = output_schema::<ExportedSummary>()
    )]
    pub(super) async fn export_version_summary(
        &self,
        Parameters(params): Parameters<ExportVersionSummaryParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let summary_id = params.summary_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::summaries::export_markdown(repo.as_ref(), summary_id)
        })
        .await?;
        tool_result(
            result.map(|path| serde_json::json!({ "path": path.to_string_lossy() })),
            SUMMARY_NOT_FOUND,
        )
    }
}
