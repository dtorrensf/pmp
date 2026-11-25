// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

/// JSON snapshot in which store timestamps are redacted: the stub writes the
/// real UTC time, which would otherwise make the snapshot change on every run.
macro_rules! assert_json_snapshot_redacted {
    ($($arg:tt)*) => {
        insta::with_settings!({
            filters => vec![(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}", "[timestamp]")]
        }, {
            insta::assert_json_snapshot!($($arg)*);
        })
    };
}

use super::*;

#[tokio::test]
async fn list_resources_includes_projects() {
    let (server, _) = server();
    let result = server.list_resources_page(None).await.unwrap();
    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources[0].uri, "pmp://projects");
}

#[tokio::test]
async fn list_resource_templates_include_project_and_tasks() {
    let (server, _) = server();
    let result = server.list_resource_templates_page(None).await.unwrap();
    assert_json_snapshot_redacted!(
        result
            .resource_templates
            .iter()
            .map(|t| (&t.uri_template, &t.name))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn every_listed_template_is_readable() {
    let (server, _) = server();
    let templates = server
        .list_resource_templates_page(None)
        .await
        .unwrap()
        .resource_templates;
    assert!(!templates.is_empty());
    for template in templates {
        let uri = template.uri_template.replace("{id}", "1");
        if let Err(err) = server.read_resource(&uri).await {
            assert_ne!(
                err.message, "unsupported resource uri",
                "template {} is listed but not readable",
                template.uri_template
            );
        }
    }
}

#[tokio::test]
async fn read_projects_returns_all_projects() {
    let (server, repo) = server();
    repo.create_project("Alpha", "First project", None).unwrap();
    repo.create_project("Beta", "Second project", None).unwrap();
    let result = server.read_resource("pmp://projects").await.unwrap();
    assert_json_snapshot_redacted!(result);
}

#[tokio::test]
async fn read_project_by_id_returns_project() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "First project", None).unwrap();
    let result = server
        .read_resource(&format!("pmp://projects/{}", project.id.unwrap()))
        .await
        .unwrap();
    assert_json_snapshot_redacted!(result);
}

#[tokio::test]
async fn read_project_tasks_returns_ordered_tasks() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "First project", None).unwrap();
    let pid = project.id.unwrap();
    repo.create_task(
        pid,
        repo.default_version_id(pid).unwrap(),
        "Low task",
        "",
        Status::Todo,
        Priority::Low,
    )
    .unwrap();
    repo.create_task(
        pid,
        repo.default_version_id(pid).unwrap(),
        "High task",
        "",
        Status::InProgress,
        Priority::High,
    )
    .unwrap();
    let result = server
        .read_resource(&format!("pmp://projects/{}/tasks", pid))
        .await
        .unwrap();
    assert_json_snapshot_redacted!(result);
}

#[tokio::test]
async fn read_unknown_project_returns_resource_not_found() {
    let (server, _) = server();
    let err = server
        .read_resource("pmp://projects/999")
        .await
        .unwrap_err();
    let ErrorData { code, .. } = err;
    assert_eq!(code, ErrorCode::RESOURCE_NOT_FOUND);
    assert_json_snapshot_redacted!(err);
}

#[tokio::test]
async fn resource_lists_reject_invalid_cursor() {
    let (server, _) = server();
    let err = server
        .list_resources_page(Some("not-a-number"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    let err = server
        .list_resource_templates_page(Some("999"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[test]
fn paginate_page_slices_and_reports_next_cursor() {
    let (page, next) = paginate_page(vec![1, 2, 3, 4, 5], None, 2).unwrap();
    assert_eq!(page, vec![1, 2]);
    assert_eq!(next.as_deref(), Some("2"));
    let (page, next) = paginate_page(vec![1, 2, 3, 4, 5], Some("2"), 2).unwrap();
    assert_eq!(page, vec![3, 4]);
    assert_eq!(next.as_deref(), Some("4"));
    let (page, next) = paginate_page(vec![1, 2, 3, 4, 5], Some("4"), 2).unwrap();
    assert_eq!(page, vec![5]);
    assert!(next.is_none());
}

#[test]
fn json_content_rejects_oversized_responses() {
    let big = "a".repeat(MAX_RESPONSE_BYTES + 1);
    let err = json_content(&big).unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn resource_templates_include_versions_and_version_tasks() {
    let (server, _) = server();
    let result = server.list_resource_templates_page(None).await.unwrap();
    let uris: Vec<&str> = result
        .resource_templates
        .iter()
        .map(|t| t.uri_template.as_str())
        .collect();
    assert!(uris.contains(&"pmp://projects/{id}/versions"));
    assert!(uris.contains(&"pmp://versions/{id}/tasks"));
}

#[tokio::test]
async fn read_project_versions_returns_versions() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    repo.create_version(pid, "v0.1.0", "").unwrap();
    let text = resource_text(
        server
            .read_resource(&format!("pmp://projects/{pid}/versions"))
            .await
            .unwrap(),
    );
    let versions: Vec<Version> = serde_json::from_str(&text).unwrap();
    assert_eq!(versions.len(), 2);
}

#[tokio::test]
async fn read_version_tasks_returns_only_that_version() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let v0 = repo.default_version_id(pid).unwrap();
    let v1 = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
    repo.create_task(pid, v1, "Scoped", "", Status::Todo, Priority::Low)
        .unwrap();
    repo.create_task(pid, v0, "Other", "", Status::Todo, Priority::Low)
        .unwrap();
    let text = resource_text(
        server
            .read_resource(&format!("pmp://versions/{v1}/tasks"))
            .await
            .unwrap(),
    );
    let tasks: Vec<Task> = serde_json::from_str(&text).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Scoped");
}

#[tokio::test]
async fn read_version_tasks_rejects_invalid_id() {
    let (server, _) = server();
    let err = server
        .read_resource("pmp://versions/abc/tasks")
        .await
        .unwrap_err();
    assert!(err.message.contains("invalid version id"));
}
