// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Helpers that send a PMP task to an already-started ACP session.
//!
//! The prompt text comes from [`crate::application::prompts`], the same source
//! used by the MCP `implement-task` prompt. This module only resolves the task
//! and its dependencies and hands the result to the ACP client.

use crate::application;
use crate::db::repository::Repository;
use crate::domain::error::AppError;

use super::client::AcpClient;
use super::protocol::{SessionId, StopReason};

/// Loads `task_id` and sends the shared implementation prompt to `session`.
///
/// The caller owns the client and session lifecycle: this helper does not
/// launch the agent, choose a profile or inject the PMP MCP server (C3/C5).
pub async fn send_implement_task(
    client: &AcpClient,
    session: &SessionId,
    repo: &dyn Repository,
    task_id: i64,
) -> Result<StopReason, AppError> {
    let detail = application::records::task_detail(repo, task_id)?;
    let dependencies = application::dependencies::view(repo, task_id)?;
    let prompt = application::prompts::implement_task(&detail, &dependencies);
    client.prompt(session, prompt).await
}
