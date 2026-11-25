// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn create_project_creates_project_with_default_version() {
    let (server, repo) = server();
    let project: Project = serde_json::from_str(&tool_text(
        server
            .create_project(Parameters(CreateProjectParams {
                name: "Gamma".to_string(),
                description: Some("desc".to_string()),
                working_dir: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(project.name, "Gamma");
    assert_eq!(project.description, "desc");
    let pid = project.id.unwrap();
    assert_eq!(repo.list_versions_by_project(pid).unwrap().len(), 1);
}

#[tokio::test]
async fn move_task_reassigns_version_and_returns_task() {
    let (server, _, pid) = seeded_server();
    let v1 = server.repo.default_version_id(pid).unwrap();
    let v2 = server
        .repo
        .create_version(pid, "v0.2.0", "")
        .unwrap()
        .id
        .unwrap();
    let task_id = server.repo.list_tasks_by_version(v1).unwrap()[0]
        .id
        .unwrap();
    let result = server
        .move_task(Parameters(MoveTaskParams {
            task_id,
            version_id: v2,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let task: Task = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(task.id, Some(task_id));
    assert_eq!(task.version_id, v2);
    assert_eq!(server.repo.get_task(task_id).unwrap().version_id, v2);
}

#[tokio::test]
async fn create_version_applies_the_shared_name_rule() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let blank = server
        .create_version(Parameters(CreateVersionParams {
            project_id: pid,
            name: "   ".to_string(),
            description: None,
            status: None,
        }))
        .await
        .unwrap();
    assert_eq!(blank.is_error, Some(true));
    assert!(tool_text(blank).contains("version name cannot be empty"));
}

#[tokio::test]
async fn create_task_rejects_version_from_another_project() {
    let (server, repo) = server();
    let alpha = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let beta = repo.create_project("Beta", "", None).unwrap().id.unwrap();
    let foreign_version = repo.default_version_id(beta).unwrap();
    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: alpha,
            version_id: foreign_version,
            name: "Task".to_string(),
            context: "".to_string(),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("version does not belong to the project"));
}

#[tokio::test]
async fn list_tasks_filters_by_version_id() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let v0 = repo.default_version_id(pid).unwrap();
    let v1 = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
    repo.create_task(pid, v0, "Old", "", Status::Todo, Priority::Low)
        .unwrap();
    repo.create_task(pid, v1, "New", "", Status::Todo, Priority::Low)
        .unwrap();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: Some(v1),
            status: None,
            priority: None,
            limit: None,
            offset: None,
            include_description: None,
        }))
        .await
        .unwrap();
    let tasks = page_tasks(&tool_text(result));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "New");
}

#[tokio::test]
async fn list_tasks_rejects_version_from_another_project() {
    let (server, repo) = server();
    let alpha = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let beta = repo.create_project("Beta", "", None).unwrap().id.unwrap();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: alpha,
            version_id: Some(repo.default_version_id(beta).unwrap()),
            status: None,
            priority: None,
            limit: None,
            offset: None,
            include_description: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("version does not belong to the project"));
}

#[tokio::test]
async fn list_versions_returns_open_then_backlog_then_released() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let released = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
    repo.update_version(released, "v0.1.0", "", VersionStatus::Released)
        .unwrap();
    let open = repo.create_version(pid, "v0.2.0", "").unwrap().id.unwrap();
    repo.update_version(open, "v0.2.0", "", VersionStatus::Open)
        .unwrap();
    repo.create_version(pid, "v0.3.0", "").unwrap();
    let result = server
        .list_versions(Parameters(ListVersionsParams { project_id: pid }))
        .await
        .unwrap();
    let versions: Vec<Version> = serde_json::from_str(&tool_text(result)).unwrap();
    let names: Vec<&str> = versions.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["v0.2.0", "v0.3.0", "v0", "v0.1.0"]);
}

#[tokio::test]
async fn list_versions_unknown_project_returns_tool_error() {
    let (server, _) = server();
    let result = server
        .list_versions(Parameters(ListVersionsParams { project_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("project not found"));
}

#[tokio::test]
async fn create_version_defaults_to_backlog_and_accepts_other_statuses() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let backlog = server
        .create_version(Parameters(CreateVersionParams {
            project_id: pid,
            name: "v0.1.0".to_string(),
            description: Some("First".to_string()),
            status: None,
        }))
        .await
        .unwrap();
    let backlog: Version = serde_json::from_str(&tool_text(backlog)).unwrap();
    assert_eq!(backlog.status, VersionStatus::Backlog);
    assert_eq!(backlog.description, "First");

    let open = server
        .create_version(Parameters(CreateVersionParams {
            project_id: pid,
            name: "v0.2.0".to_string(),
            description: None,
            status: Some(VersionStatus::Open),
        }))
        .await
        .unwrap();
    let open: Version = serde_json::from_str(&tool_text(open)).unwrap();
    assert_eq!(open.status, VersionStatus::Open);

    let released = server
        .create_version(Parameters(CreateVersionParams {
            project_id: pid,
            name: "v0.3.0".to_string(),
            description: None,
            status: Some(VersionStatus::Released),
        }))
        .await
        .unwrap();
    let released: Version = serde_json::from_str(&tool_text(released)).unwrap();
    assert_eq!(released.status, VersionStatus::Released);
}

#[tokio::test]
async fn create_version_rejects_empty_duplicate_and_invalid_status() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let params = |name: &str, status: Option<VersionStatus>| CreateVersionParams {
        project_id: pid,
        name: name.to_string(),
        description: None,
        status,
    };
    let empty = server
        .create_version(Parameters(params("  ", None)))
        .await
        .unwrap();
    assert_eq!(empty.is_error, Some(true));
    assert!(tool_text(empty).contains("version name cannot be empty"));

    server
        .create_version(Parameters(params("v0.1.0", None)))
        .await
        .unwrap();
    let duplicate = server
        .create_version(Parameters(params("v0.1.0", None)))
        .await
        .unwrap();
    assert_eq!(duplicate.is_error, Some(true));
    assert!(tool_text(duplicate).contains("version already exists"));

    // Unknown statuses are rejected by the schema, before the tool runs.
    let invalid = serde_json::from_value::<CreateVersionParams>(serde_json::json!({
        "project_id": pid,
        "name": "v0.3.0",
        "status": "closed",
    }));
    assert!(invalid.is_err());
}

#[tokio::test]
async fn update_version_renames_and_keeps_omitted_fields() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version_id = repo
        .create_version(pid, "v0.1.0", "keep me")
        .unwrap()
        .id
        .unwrap();

    let renamed: Version = serde_json::from_str(&tool_text(
        server
            .update_version(Parameters(UpdateVersionParams {
                version_id,
                name: "v0.2.0".to_string(),
                description: None,
                status: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(renamed.name, "v0.2.0");
    assert_eq!(renamed.description, "keep me");
    assert_eq!(renamed.status, VersionStatus::Backlog);

    let described: Version = serde_json::from_str(&tool_text(
        server
            .update_version(Parameters(UpdateVersionParams {
                version_id,
                name: "v0.2.0".to_string(),
                description: Some("changed".to_string()),
                status: Some(VersionStatus::Released),
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(described.description, "changed");
    assert_eq!(described.status, VersionStatus::Released);
}

#[tokio::test]
async fn update_version_rejects_empty_duplicate_unknown_and_invalid_status() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version_id = repo.create_version(pid, "v0.1.0", "").unwrap().id.unwrap();
    repo.create_version(pid, "v0.2.0", "").unwrap();

    let empty = server
        .update_version(Parameters(UpdateVersionParams {
            version_id,
            name: "  ".to_string(),
            description: None,
            status: None,
        }))
        .await
        .unwrap();
    assert_eq!(empty.is_error, Some(true));
    assert!(tool_text(empty).contains("version name cannot be empty"));

    let duplicate = server
        .update_version(Parameters(UpdateVersionParams {
            version_id,
            name: "v0.2.0".to_string(),
            description: None,
            status: None,
        }))
        .await
        .unwrap();
    assert_eq!(duplicate.is_error, Some(true));
    assert!(tool_text(duplicate).contains("version already exists"));

    let unknown = server
        .update_version(Parameters(UpdateVersionParams {
            version_id: 999,
            name: "v1.0.0".to_string(),
            description: None,
            status: None,
        }))
        .await
        .unwrap();
    assert_eq!(unknown.is_error, Some(true));
    assert!(tool_text(unknown).contains("version not found"));

    let invalid = serde_json::from_value::<UpdateVersionParams>(serde_json::json!({
        "version_id": version_id,
        "name": "v0.1.0",
        "status": "closed",
    }));
    assert!(invalid.is_err());
}

#[tokio::test]
async fn project_version_flow_create_version_task_and_filter() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();

    let version: Version = serde_json::from_str(&tool_text(
        server
            .create_version(Parameters(CreateVersionParams {
                project_id: pid,
                name: "v0.1.0".to_string(),
                description: None,
                status: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    let version_id = version.id.unwrap();

    let task: Task = serde_json::from_str(&tool_text(
        server
            .create_task(Parameters(CreateTaskParams {
                project_id: pid,
                version_id,
                name: "Release work".to_string(),
                context: "ctx".to_string(),
                requirements: "req".to_string(),
                benefits: "ben".to_string(),
                priority: PriorityParam::High,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(task.version_id, version_id);

    let filtered = page_tasks(&tool_text(
        server
            .list_tasks(Parameters(ListTasksParams {
                project_id: pid,
                version_id: Some(version_id),
                status: None,
                priority: None,
                limit: None,
                offset: None,
                include_description: None,
            }))
            .await
            .unwrap(),
    ));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, task.id);
}
