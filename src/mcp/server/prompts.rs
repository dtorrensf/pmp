// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reusable MCP prompts for the main PMP workflows.
//!
//! The prompt text lives in [`crate::application::prompts`]; this module only
//! validates MCP arguments and loads the records needed by `implement-task`.

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{PromptMessage, Role};
use rmcp::{prompt, prompt_router};
use serde::Deserialize;

use crate::application;
use crate::domain::error::AppError;

use super::*;

#[derive(Deserialize, schemars::JsonSchema)]
struct TaskPromptParams {
    /// Positive id of the task to implement.
    task_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct VersionPromptParams {
    /// Positive id of the version to plan, review or summarise.
    version_id: String,
}

#[prompt_router(router = "prompt_router", vis = "pub(super)")]
impl PmpServer {
    #[prompt(
        name = "implement-task",
        description = "Prepare an agent to implement one PMP task with its sections, dependencies and verification workflow."
    )]
    pub(super) async fn implement_task_prompt(
        &self,
        Parameters(params): Parameters<TaskPromptParams>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let task_id = parse_id(&params.task_id, "task_id")?;
        let detail = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::records::task_detail(repo.as_ref(), task_id)
        })
        .await?
        .map_err(|error| prompt_error(error, "task not found"))?;
        let dependencies = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::view(repo.as_ref(), task_id)
        })
        .await?
        .map_err(|error| prompt_error(error, "task not found"))?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            application::prompts::implement_task(&detail, &dependencies),
        )])
    }

    #[prompt(
        name = "plan-version",
        description = "Plan a PMP version in read-only mode from its tasks, dependencies and acceptance criteria."
    )]
    pub(super) async fn plan_version_prompt(
        &self,
        Parameters(params): Parameters<VersionPromptParams>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let version_id = parse_id(&params.version_id, "version_id")?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            application::prompts::plan_version(version_id),
        )])
    }

    #[prompt(
        name = "review-version",
        description = "Review a PMP version for release readiness without changing its records."
    )]
    pub(super) async fn review_version_prompt(
        &self,
        Parameters(params): Parameters<VersionPromptParams>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let version_id = parse_id(&params.version_id, "version_id")?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            application::prompts::review_version(version_id),
        )])
    }

    #[prompt(
        name = "release-notes",
        description = "Draft Markdown release notes from a PMP version's completed tasks and compacted summaries."
    )]
    pub(super) async fn release_notes_prompt(
        &self,
        Parameters(params): Parameters<VersionPromptParams>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let version_id = parse_id(&params.version_id, "version_id")?;
        Ok(vec![PromptMessage::new_text(
            Role::User,
            application::prompts::release_notes(version_id),
        )])
    }
}

fn parse_id(value: &str, field: &str) -> Result<i64, ErrorData> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            ErrorData::invalid_params(format!("{field} must be a positive integer"), None)
        })
}

fn prompt_error(error: AppError, not_found: &'static str) -> ErrorData {
    match error {
        AppError::NotFound => ErrorData::invalid_params(not_found, None::<serde_json::Value>),
        other => app_error_to_mcp(other),
    }
}
