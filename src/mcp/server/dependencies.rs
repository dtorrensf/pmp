// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;

use super::*;

#[tool_router(router = tool_router_dependencies, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "get_task_dependencies",
        description = "List the task ids that a task depends on (its blockers). Check before starting a task: incomplete dependencies block marking it done."
    )]
    pub(super) async fn get_task_dependencies(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::list(repo.as_ref(), task_id)
        })
        .await?;
        tool_result(
            result.map(|depends_on_ids| {
                serde_json::json!({
                    "task_id": task_id,
                    "depends_on_ids": depends_on_ids,
                })
            }),
            TASK_NOT_FOUND,
        )
    }

    #[tool(
        name = "get_task_dependents",
        description = "List the task ids that depend on a task (tasks it blocks). Use to understand the impact of changing or deleting a task."
    )]
    pub(super) async fn get_task_dependents(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::dependents(repo.as_ref(), task_id)
        })
        .await?;
        tool_result(
            result.map(|task_ids| {
                serde_json::json!({
                    "task_id": task_id,
                    "task_ids": task_ids,
                })
            }),
            TASK_NOT_FOUND,
        )
    }

    #[tool(
        name = "add_task_dependency",
        description = "Make one task depend on another: task_id depends on depends_on_id. Both tasks must be in the same version. Rejects self-dependencies and circular dependencies."
    )]
    pub(super) async fn add_task_dependency(
        &self,
        Parameters(params): Parameters<TaskDependencyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let depends_on_id = params.depends_on_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::add(repo.as_ref(), task_id, depends_on_id)
        })
        .await?;
        tool_result(
            result.map(|()| {
                serde_json::json!({
                    "task_id": task_id,
                    "depends_on_id": depends_on_id,
                })
            }),
            TASK_NOT_FOUND,
        )
    }

    #[tool(
        name = "set_task_dependencies",
        description = "Set the complete list of tasks that a task depends on, replacing the current list in one atomic operation. Prefer this over add/remove when changing several dependencies. Pass an empty list to clear them. Rejects self-dependencies, cycles and tasks from another version; on error nothing is changed. Returns the resulting depends_on ids."
    )]
    pub(super) async fn set_task_dependencies(
        &self,
        Parameters(params): Parameters<SetTaskDependenciesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::set(repo.as_ref(), task_id, &params.depends_on_ids)
        })
        .await?;
        tool_result(
            result.map(|ids| {
                serde_json::json!({
                    "task_id": task_id,
                    "depends_on_ids": ids,
                })
            }),
            TASK_NOT_FOUND,
        )
    }

    #[tool(
        name = "remove_task_dependency",
        description = "Remove a dependency between two tasks. The task_id depends on depends_on_id."
    )]
    pub(super) async fn remove_task_dependency(
        &self,
        Parameters(params): Parameters<TaskDependencyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let depends_on_id = params.depends_on_id;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::dependencies::remove(repo.as_ref(), task_id, depends_on_id)
        })
        .await?;
        tool_result(
            result.map(|()| {
                serde_json::json!({
                    "task_id": task_id,
                    "depends_on_id": depends_on_id,
                })
            }),
            TASK_NOT_FOUND,
        )
    }
}
