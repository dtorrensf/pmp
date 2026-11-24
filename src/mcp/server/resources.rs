// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use rmcp::ErrorData;
use rmcp::model::{
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceResult, Resource,
    ResourceTemplate,
};

use crate::application;

use super::PmpServer;
use super::common::*;

impl PmpServer {
    pub async fn list_resources_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resource = Resource::new(PROJECTS_URI, "projects")
            .with_description(
                "All user projects. Read this to see what projects the user is working on.",
            )
            .with_mime_type("application/json");
        let (resources, next_cursor) = paginate_page(vec![resource], cursor, RESOURCE_PAGE_SIZE)?;
        let mut result = ListResourcesResult::with_all_items(resources);
        result.next_cursor = next_cursor;
        Ok(result)
    }

    pub async fn list_resource_templates_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let templates = vec![
            ResourceTemplate::new(PROJECT_TEMPLATE, "project")
                .with_description("A single project by id. Use to get details about a specific project.")
                .with_mime_type("application/json"),
            ResourceTemplate::new(PROJECT_TASKS_TEMPLATE, "project_tasks")
                .with_description("Tasks for a project across all its versions. Use to see all tasks and their status for a specific project.")
                .with_mime_type("application/json"),
            ResourceTemplate::new(PROJECT_VERSIONS_TEMPLATE, "project_versions")
                .with_description("Versions of a project, open ones first. Use to see the release lines of a project and their status.")
                .with_mime_type("application/json"),
            ResourceTemplate::new(VERSION_TASKS_TEMPLATE, "version_tasks")
                .with_description("Tasks of one version. Use to see the work scheduled for a specific release.")
                .with_mime_type("application/json"),
            ResourceTemplate::new(VERSION_SUMMARIES_TEMPLATE, "version_summaries")
                .with_description("Summaries of a version after compaction. Use to review what was frozen.")
                .with_mime_type("application/json"),
            ResourceTemplate::new(SUMMARY_TEMPLATE, "version_summary")
                .with_description("A single version summary by id, including its frozen snapshot.")
                .with_mime_type("application/json"),
        ];
        let (templates, next_cursor) = paginate_page(templates, cursor, RESOURCE_PAGE_SIZE)?;
        let mut result = ListResourceTemplatesResult::with_all_items(templates);
        result.next_cursor = next_cursor;
        Ok(result)
    }

    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        let repo = self.repo.clone();

        match uri {
            PROJECTS_URI => {
                read_json_resource(repo, self.timeout, uri, |repo| {
                    application::projects::list(repo.as_ref())
                })
                .await
            }
            uri if uri.starts_with(VERSIONS_PREFIX) => {
                let tail = &uri[VERSIONS_PREFIX.len()..];
                if let Some(prefix) = tail.strip_suffix("/summaries") {
                    let id = prefix
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid version id", None))?;
                    read_json_resource(repo, self.timeout, uri, move |repo| {
                        application::summaries::list(repo.as_ref(), id)
                    })
                    .await
                } else {
                    let id = tail
                        .strip_suffix("/tasks")
                        .ok_or_else(|| ErrorData::invalid_params("unsupported resource uri", None))?
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid version id", None))?;
                    read_json_resource(repo, self.timeout, uri, move |repo| {
                        application::tasks::list_by_version(repo.as_ref(), id)
                    })
                    .await
                }
            }
            uri if uri.starts_with(SUMMARIES_PREFIX) => {
                let id = uri[SUMMARIES_PREFIX.len()..]
                    .parse::<i64>()
                    .map_err(|_| ErrorData::invalid_params("invalid summary id", None))?;
                read_json_resource(repo, self.timeout, uri, move |repo| {
                    application::summaries::get(repo.as_ref(), id)
                })
                .await
            }
            uri if uri.starts_with(PROJECTS_PREFIX) => {
                let tail = &uri[PROJECTS_PREFIX.len()..];
                if let Some(prefix) = tail.strip_suffix("/versions") {
                    let id = prefix
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    read_json_resource(repo, self.timeout, uri, move |repo| {
                        application::versions::list(repo.as_ref(), id)
                    })
                    .await
                } else if let Some(prefix) = tail.strip_suffix("/tasks") {
                    let id = prefix
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    read_json_resource(repo, self.timeout, uri, move |repo| {
                        application::tasks::list_by_project(repo.as_ref(), id)
                    })
                    .await
                } else {
                    let id = tail
                        .parse::<i64>()
                        .map_err(|_| ErrorData::invalid_params("invalid project id", None))?;
                    read_json_resource(repo, self.timeout, uri, move |repo| {
                        application::projects::get(repo.as_ref(), id)
                    })
                    .await
                }
            }
            _ => Err(ErrorData::invalid_params("unsupported resource uri", None)),
        }
    }
}
