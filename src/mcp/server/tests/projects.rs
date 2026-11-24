// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn create_project_defaults_description_and_rejects_empty_name() {
    let (server, _) = server();
    let project: Project = serde_json::from_str(&tool_text(
        server
            .create_project(Parameters(CreateProjectParams {
                name: "Delta".to_string(),
                description: None,
                working_dir: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(project.description, "");

    let rejected = server
        .create_project(Parameters(CreateProjectParams {
            name: "   ".to_string(),
            description: None,
            working_dir: None,
        }))
        .await
        .unwrap();
    assert_eq!(rejected.is_error, Some(true));
    assert!(tool_text(rejected).contains("project name cannot be empty"));
}

#[tokio::test]
async fn create_and_update_project_manage_the_working_dir() {
    let (server, _) = server();
    let created: Project = serde_json::from_str(&tool_text(
        server
            .create_project(Parameters(CreateProjectParams {
                name: "Workspace".to_string(),
                description: None,
                working_dir: Some("/srv/repos/../repos/alpha ".to_string()),
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        created.working_dir.as_deref(),
        Some("/srv/repos/alpha"),
        "the path is normalized before storing it"
    );
    let project_id = created.id.unwrap();

    let renamed: Project = serde_json::from_str(&tool_text(
        server
            .update_project(Parameters(UpdateProjectParams {
                project_id,
                name: "Renamed".to_string(),
                description: None,
                working_dir: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        renamed.working_dir.as_deref(),
        Some("/srv/repos/alpha"),
        "omitting the field keeps the current directory"
    );

    let cleared: Project = serde_json::from_str(&tool_text(
        server
            .update_project(Parameters(UpdateProjectParams {
                project_id,
                name: "Renamed".to_string(),
                description: None,
                working_dir: Some(String::new()),
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(cleared.working_dir, None, "an empty value clears it");

    let rejected = server
        .create_project(Parameters(CreateProjectParams {
            name: "Relative".to_string(),
            description: None,
            working_dir: Some("../repo".to_string()),
        }))
        .await
        .unwrap();
    assert_eq!(rejected.is_error, Some(true));
    assert!(tool_text(rejected).contains("absolute"));
}

#[tokio::test]
async fn update_project_renames_and_keeps_description_when_omitted() {
    let (server, repo) = server();
    let pid = repo
        .create_project("Old", "keep me", None)
        .unwrap()
        .id
        .unwrap();
    let renamed: Project = serde_json::from_str(&tool_text(
        server
            .update_project(Parameters(UpdateProjectParams {
                project_id: pid,
                name: "New".to_string(),
                description: None,
                working_dir: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(renamed.name, "New");
    assert_eq!(renamed.description, "keep me");

    let described: Project = serde_json::from_str(&tool_text(
        server
            .update_project(Parameters(UpdateProjectParams {
                project_id: pid,
                name: "New".to_string(),
                description: Some("changed".to_string()),
                working_dir: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(described.description, "changed");
}

#[tokio::test]
async fn update_project_rejects_empty_name_and_unknown_project() {
    let (server, repo) = server();
    let pid = repo.create_project("P", "", None).unwrap().id.unwrap();
    let empty = server
        .update_project(Parameters(UpdateProjectParams {
            project_id: pid,
            name: "  ".to_string(),
            description: None,
            working_dir: None,
        }))
        .await
        .unwrap();
    assert_eq!(empty.is_error, Some(true));
    assert!(tool_text(empty).contains("project name cannot be empty"));

    let unknown = server
        .update_project(Parameters(UpdateProjectParams {
            project_id: 999,
            name: "X".to_string(),
            description: None,
            working_dir: None,
        }))
        .await
        .unwrap();
    assert_eq!(unknown.is_error, Some(true));
    assert!(tool_text(unknown).contains("project not found"));
}

#[tokio::test]
async fn list_projects_returns_all_projects() {
    let (server, repo) = server();
    repo.create_project("Alpha", "", None).unwrap();
    repo.create_project("Beta", "", None).unwrap();
    let projects: Vec<Project> =
        serde_json::from_str(&tool_text(server.list_projects().await.unwrap())).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"Alpha") && names.contains(&"Beta"));
}
