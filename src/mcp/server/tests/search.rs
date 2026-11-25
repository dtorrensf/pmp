// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::mcp::server::ListTasksParams;

/// Parameters that match every task of the project; tests override fields.
fn search(project_id: i64) -> ListTasksParams {
    ListTasksParams {
        project_id,
        version_id: None,
        status: None,
        priority: None,
        limit: None,
        offset: None,
        include_description: None,
        query: None,
        ids: None,
        only_ready: None,
        only_blocked: None,
        updated_or_created_since: None,
    }
}

async fn names(server: &PmpServer, params: ListTasksParams) -> Vec<String> {
    let result = server.list_tasks(Parameters(params)).await.unwrap();
    let page: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    page["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn query_finds_tasks_by_name_description_and_notes() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    repo.create_task(
        pid,
        vid,
        "Login",
        "uses OAuth",
        Status::Todo,
        Priority::Medium,
    )
    .unwrap();
    let noted = repo
        .create_task(pid, vid, "Storage", "", Status::Todo, Priority::Medium)
        .unwrap()
        .id
        .unwrap();
    repo.create_task(pid, vid, "Unrelated", "", Status::Todo, Priority::Medium)
        .unwrap();
    repo.add_task_note(noted, "migrated to OAUTH tokens")
        .unwrap();

    let found = names(
        &server,
        ListTasksParams {
            query: Some("oauth".to_string()),
            ..search(pid)
        },
    )
    .await;
    // Tasks created in the same second tie on created_at, so compare as sets.
    let mut found = found;
    found.sort();
    assert_eq!(found, vec!["Login", "Storage"]);
}

#[tokio::test]
async fn only_blocked_and_ids_narrow_the_listing() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let blocker = repo
        .create_task(pid, vid, "Blocker", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    let waiting = repo
        .create_task(pid, vid, "Waiting", "", Status::Todo, Priority::High)
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(waiting, blocker).unwrap();

    let blocked = names(
        &server,
        ListTasksParams {
            only_blocked: Some(true),
            ..search(pid)
        },
    )
    .await;
    assert_eq!(blocked, vec!["Waiting"]);

    let by_id = names(
        &server,
        ListTasksParams {
            ids: Some(vec![blocker]),
            ..search(pid)
        },
    )
    .await;
    assert_eq!(by_id, vec!["Blocker"]);
}

#[tokio::test]
async fn contradictory_readiness_flags_are_a_tool_error() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            only_ready: Some(true),
            only_blocked: Some(true),
            ..search(pid)
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn malformed_time_bound_is_a_tool_error() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            updated_or_created_since: Some("yesterday".to_string()),
            ..search(pid)
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn time_bound_includes_tasks_created_at_or_after_it() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    repo.create_task(pid, vid, "Fresh", "", Status::Todo, Priority::Medium)
        .unwrap();
    let recent = names(
        &server,
        ListTasksParams {
            updated_or_created_since: Some("2000-01-01".to_string()),
            ..search(pid)
        },
    )
    .await;
    assert_eq!(recent, vec!["Fresh"]);
    let future = names(
        &server,
        ListTasksParams {
            updated_or_created_since: Some("9999-12-31".to_string()),
            ..search(pid)
        },
    )
    .await;
    assert!(future.is_empty());
}
