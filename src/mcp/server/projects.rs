// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use crate::application;
use application::projects::{NewProject, ProjectEdit};

use super::*;

#[tool_router(router = tool_router_projects, vis = "pub(super)")]
impl PmpServer {
    #[tool(
        name = "list_projects",
        description = "List all projects of the user. Use to discover project ids before listing versions or tasks, or when the user asks what projects they have."
    )]
    pub(super) async fn list_projects(&self) -> Result<CallToolResult, ErrorData> {
        run_tool(self.repo.clone(), self.timeout, PROJECT_NOT_FOUND, |repo| {
            application::projects::list(repo.as_ref())
        })
        .await
    }

    #[tool(
        name = "create_project",
        description = "Create a new project. Use when the user asks to start a new project. Requires a non-empty name; description and working_dir are optional. working_dir must be an absolute path; it is normalized before storing. The project starts with a default version (see list_versions)."
    )]
    pub(super) async fn create_project(
        &self,
        Parameters(params): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| {
                application::projects::create(
                    repo.as_ref(),
                    NewProject {
                        name: params.name,
                        description: params.description.unwrap_or_default(),
                        working_dir: params.working_dir,
                    },
                )
            },
        )
        .await
    }

    #[tool(
        name = "update_project",
        description = "Rename a project and/or change its description or working directory. Requires the project id and a non-empty name. Omit description or working_dir to keep the current value; send an empty working_dir to clear it. Relative paths are rejected. Use to edit project metadata instead of editing the database."
    )]
    pub(super) async fn update_project(
        &self,
        Parameters(params): Parameters<UpdateProjectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        run_tool(
            self.repo.clone(),
            self.timeout,
            PROJECT_NOT_FOUND,
            move |repo| {
                application::projects::edit(
                    repo.as_ref(),
                    params.project_id,
                    ProjectEdit {
                        name: params.name,
                        description: params.description,
                        working_dir: params.working_dir,
                    },
                )
            },
        )
        .await
    }
}
