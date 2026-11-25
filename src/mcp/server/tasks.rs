// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;
use crate::db::repository::TaskFilter;
use crate::domain::task::{Priority, Status};
use crate::domain::task_description::compose_description;
use crate::domain::validation::validate_text;
use application::pagination::clamp_limit;
use application::tasks::{NewTask, TaskEdit};

use super::*;

#[tool_router(router = tool_router_tasks, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "list_tasks",
        description = "List tasks for a project. Filters, all optional and combinable: version_id (see list_versions), status (todo, in_progress, done), priority (Low, Medium, High), query (case-insensitive text found in the name, description or notes), ids (up to 500 task ids), only_ready (todo with every dependency done) or only_blocked (waiting for an unfinished dependency), and updated_or_created_since (tasks created, completed or noted at or after a UTC time, YYYY-MM-DD or YYYY-MM-DD HH:MM:SS). Each task comes as a light summary (id, names, status, priority, timestamps, depends_on_count and blocks_count); set include_description to true to add the full description, or call get_task for one task. Results are paginated: use limit (default 100, max 500) and offset, and check has_more/total to fetch the next page. Use when the user asks about their tasks, to-dos, pending work or progress, to search for a topic, or to pick the next task to work on."
    )]
    pub(super) async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let project_id = params.project_id;
        let version_id = params.version_id;
        let status = params.status;
        let priority = params.priority.map(Priority::from);
        let limit = clamp_limit(params.limit);
        let offset = params.offset.unwrap_or(0);
        let include_description = params.include_description.unwrap_or(false);
        let text = params.query;
        let ids = params.ids;
        let touched_since = params.updated_or_created_since;
        let (only_ready, only_blocked) = (
            params.only_ready.unwrap_or(false),
            params.only_blocked.unwrap_or(false),
        );
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            let filter = TaskFilter {
                project_id,
                version_id,
                status,
                priority,
                text,
                ids,
                readiness: application::tasks::readiness_from_flags(only_ready, only_blocked)?,
                touched_since,
            };
            application::tasks::page(repo.as_ref(), &filter, limit, offset, include_description)
        })
        .await?;
        tool_result(result, PROJECT_NOT_FOUND)
    }

    #[tool(
        name = "list_ready_tasks",
        description = "List the tasks ready to implement: todo tasks whose dependencies are all done, highest priority first, then lowest id. Same summary shape and pagination as list_tasks (limit, offset, total, has_more). Use to see what can be worked on now; use claim_next_task to take one."
    )]
    pub(super) async fn list_ready_tasks(
        &self,
        Parameters(params): Parameters<ListReadyTasksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let project_id = params.project_id;
        let version_id = params.version_id;
        let limit = clamp_limit(params.limit);
        let offset = params.offset.unwrap_or(0);
        let include_description = params.include_description.unwrap_or(false);
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::tasks::ready_page(
                repo.as_ref(),
                project_id,
                version_id,
                limit,
                offset,
                include_description,
            )
        })
        .await?;
        tool_result(result, PROJECT_NOT_FOUND)
    }

    #[tool(
        name = "claim_next_task",
        description = "Claim the next ready task in one atomic step: the highest-priority todo task whose dependencies are all done is moved to in_progress, and the optional note is recorded as its start note. Returns the claimed task. Fails with no_ready_task when nothing is ready; nothing changes in that case. Prefer this over list_ready_tasks followed by update_task_status when you are about to start work."
    )]
    pub(super) async fn claim_next_task(
        &self,
        Parameters(params): Parameters<ClaimNextTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| {
                application::tasks::claim_next(
                    repo.as_ref(),
                    params.project_id,
                    params.version_id,
                    params.note.as_deref(),
                )
            },
        )
        .await
    }

    #[tool(
        name = "move_task",
        description = "Move a task to another version of its own project. Only the version changes: notes, status, priority and dates stay. Moving to the current version is a no-op. Fails if the target version belongs to another project, or if the task has dependencies or dependents: they must stay within one version, so clear them first with set_task_dependencies. Returns the updated task."
    )]
    pub(super) async fn move_task(
        &self,
        Parameters(params): Parameters<MoveTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_OR_VERSION_NOT_FOUND,
            move |repo| {
                application::tasks::move_to_version(
                    repo.as_ref(),
                    params.task_id,
                    params.version_id,
                )
            },
        )
        .await
    }

    #[tool(
        name = "update_task_priority",
        description = "Change the priority of an existing task to Low, Medium or High. Use to reprioritize work; prefer this over editing the database directly."
    )]
    pub(super) async fn update_task_priority(
        &self,
        Parameters(params): Parameters<UpdateTaskPriorityParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let priority = params.priority;
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| application::tasks::set_priority(repo.as_ref(), task_id, priority.into()),
        )
        .await
    }

    #[tool(
        name = "update_task",
        description = "Edit an existing task: name and/or the context, requirements and benefits sections. Omitted fields keep their current value, so send only what changes. Status, priority and dependencies are not changed here (use update_task_status, update_task_priority and set_task_dependencies). Returns the updated task."
    )]
    pub(super) async fn update_task(
        &self,
        Parameters(params): Parameters<UpdateTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| {
                application::tasks::edit(
                    repo.as_ref(),
                    params.task_id,
                    TaskEdit {
                        name: params.name,
                        context: params.context,
                        requirements: params.requirements,
                        benefits: params.benefits,
                        ..TaskEdit::default()
                    },
                )
            },
        )
        .await
    }

    #[tool(
        name = "create_task",
        description = "Create a new task in a version of a project. Use only when the user asks for a new task or work item. Requires the project id, the version id (see list_versions), a task name, and context, requirements and benefits. The task starts as todo. Optionally accepts a priority (Low, Medium, High); defaults to Medium."
    )]
    pub(super) async fn create_task(
        &self,
        Parameters(params): Parameters<CreateTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| {
                validate_text("task", "context", &params.context)?;
                validate_text("task", "requirements", &params.requirements)?;
                validate_text("task", "benefits", &params.benefits)?;
                let description =
                    compose_description(&params.context, &params.requirements, &params.benefits);
                validate_text("task", "description", &description)?;
                application::tasks::create(
                    repo.as_ref(),
                    NewTask {
                        project_id: Some(params.project_id),
                        version_id: params.version_id,
                        name: params.name,
                        description,
                        status: Status::Todo,
                        priority: params.priority.into(),
                        depends_on: Vec::new(),
                    },
                )
            },
        )
        .await
    }

    #[tool(
        name = "update_task_status",
        description = "Change the status of a task to todo, in_progress or done. Call with in_progress when you start working on a task and with done when it is finished. Marking a task done fails with a validation error while it has incomplete dependencies; finish those first. Unknown task ids return task not found."
    )]
    pub(super) async fn update_task_status(
        &self,
        Parameters(params): Parameters<UpdateTaskStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let new_status = params.status;
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| application::tasks::set_status(repo.as_ref(), task_id, new_status),
        )
        .await
    }

    #[tool(
        name = "get_task",
        description = "Get one task by id: its name, status, priority, the Context, Requirements and Benefits sections, the 5 most recent notes, and the ids of the tasks it depends on and the tasks it blocks. Use instead of list_tasks when you need the full detail of a single task."
    )]
    pub(super) async fn get_task(
        &self,
        Parameters(params): Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        run_tool(
            self.repo.clone(),
            self.timeout,
            TASK_NOT_FOUND,
            move |repo| application::records::task_detail(repo.as_ref(), task_id),
        )
        .await
    }

    #[tool(
        name = "delete_task",
        description = "Delete a task with its notes and dependency links. Irreversible. Refused with error code confirmation_required while the task is in_progress or other tasks depend on it, unless confirm is true; the refusal lists the dependent task ids. Only delete when the user explicitly asks, for example a task created by mistake. To retire finished work, use the done status instead."
    )]
    pub(super) async fn delete_task(
        &self,
        Parameters(params): Parameters<DeleteTaskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let task_id = params.task_id;
        let confirm = params.confirm;
        let result = execute_repository(self.repo.clone(), self.timeout, move |repo| {
            application::records::delete_task(repo.as_ref(), task_id, confirm)
        })
        .await?;
        tool_result(result, TASK_NOT_FOUND)
    }
}
