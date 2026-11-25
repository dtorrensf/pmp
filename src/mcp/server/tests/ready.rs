// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::mcp::server::{ClaimNextTaskParams, ListReadyTasksParams};

fn ready_params(project_id: i64) -> ListReadyTasksParams {
    ListReadyTasksParams {
        project_id,
        version_id: None,
        limit: None,
        offset: None,
        include_description: None,
    }
}

#[tokio::test]
async fn list_ready_tasks_skips_blocked_tasks_and_returns_summaries() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let blocker = repo
        .create_task(pid, vid, "Blocker", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    let blocked = repo
        .create_task(pid, vid, "Blocked", "", Status::Todo, Priority::High)
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(blocked, blocker).unwrap();

    let result = server
        .list_ready_tasks(Parameters(ready_params(pid)))
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    let names: Vec<&str> = page["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Blocker"]);
    assert!(page["tasks"][0].get("description").is_none());
    assert_eq!(page["total"], 1);
    assert_eq!(page["has_more"], false);
}

#[tokio::test]
async fn claim_next_task_moves_the_task_and_records_the_note() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let task = repo
        .create_task(pid, vid, "Only", "", Status::Todo, Priority::Medium)
        .unwrap()
        .id
        .unwrap();

    let result = server
        .claim_next_task(Parameters(ClaimNextTaskParams {
            project_id: pid,
            version_id: None,
            note: Some("starting work".to_string()),
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    let claimed: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(claimed["id"], task);
    assert_eq!(claimed["status"], "in_progress");

    let notes = repo.list_task_notes(task).unwrap();
    assert_eq!(notes[0].content, "starting work");
}

#[tokio::test]
async fn claim_next_task_without_ready_work_is_a_tool_error_with_code() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();

    let result = server
        .claim_next_task(Parameters(ClaimNextTaskParams {
            project_id: pid,
            version_id: None,
            note: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let info = result.content[1]
        .as_text()
        .expect("error info")
        .text
        .clone();
    assert!(info.contains("no_ready_task"), "{info}");
}
