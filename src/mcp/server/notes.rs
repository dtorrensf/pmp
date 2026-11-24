// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;

use super::*;

#[tool_router(router = tool_router_notes, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "list_task_notes",
        description = "List the notes of a task, newest first. Use to read progress updates, decisions, context and reminders before continuing work on it."
    )]
    pub(super) async fn list_task_notes(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| application::notes::list(repo.as_ref(), task_id),
        )
        .await
    }

    #[tool(
        name = "add_task_note",
        description = "Add a note to a task to record progress, decisions or context. Add notes as you work so the task history reflects what was actually done."
    )]
    pub(super) async fn add_task_note(
        &self,
        Parameters(params): Parameters<AddTaskNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let content = params.content;
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| application::notes::add(repo.as_ref(), task_id, &content),
        )
        .await
    }

    #[tool(
        name = "delete_task_note",
        description = "Delete a task note by its note id."
    )]
    pub(super) async fn delete_task_note(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let note_id = params.note_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::notes::delete(repo.as_ref(), note_id)
        })
        .await?;
        tool_result(
            result.map(|()| serde_json::json!({ "note_id": note_id })),
            NOTE_NOT_FOUND,
        )
    }
}
