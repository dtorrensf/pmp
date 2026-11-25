// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! MCP tests for the single-record tools (`get_*`) and the guarded
//! `delete_*` tools.

use super::*;
use crate::db::repository::{DependencyRepository, NoteRepository, RepositoryPolicy};

/// One project with a default version holding one task of `status`.
fn seeded(status: Status) -> (PmpServer, Arc<StubRepository>, i64, i64, i64) {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let tid = repo
        .create_task(pid, vid, "Task", "", status, Priority::Medium)
        .unwrap()
        .id
        .unwrap();
    (server, repo, pid, vid, tid)
}

#[tokio::test]
async fn get_project_returns_details_with_counts() {
    let (server, _, pid, _, _) = seeded(Status::Todo);
    let result = server
        .get_project(Parameters(ProjectIdParams { project_id: pid }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["project"]["name"], "Alpha");
    assert_eq!(value["versions"], 1);
    assert_eq!(value["tasks"]["todo"], 1);
    assert_eq!(value["tasks"]["done"], 0);
}

#[tokio::test]
async fn get_project_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .get_project(Parameters(ProjectIdParams { project_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("project not found"));
}

#[tokio::test]
async fn get_version_returns_counts_and_summary_count() {
    let (server, _, _, vid, _) = seeded(Status::InProgress);
    let result = server
        .get_version(Parameters(VersionIdParams { version_id: vid }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["version"]["id"], vid);
    assert_eq!(value["tasks"]["in_progress"], 1);
    assert_eq!(value["summaries"], 0);
}

#[tokio::test]
async fn get_version_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .get_version(Parameters(VersionIdParams { version_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("version not found"));
}

#[tokio::test]
async fn get_task_returns_sections_recent_notes_and_links() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let description = "## Context\n\nwhy\n\n## Requirements\n\nwhat\n\n## Benefits\n\nvalue";
    let tid = repo
        .create_task(pid, vid, "Task", description, Status::Todo, Priority::High)
        .unwrap()
        .id
        .unwrap();
    let blocker = repo
        .create_task(pid, vid, "Blocker", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    let blocked = repo
        .create_task(pid, vid, "Blocked", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(tid, blocker).unwrap();
    repo.add_dependency(blocked, tid).unwrap();
    for n in 0..7 {
        repo.add_task_note(tid, &format!("note {n}")).unwrap();
    }

    let result = server
        .get_task(Parameters(TaskIdParams { task_id: tid }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["task"]["name"], "Task");
    assert_eq!(value["sections"]["context"], "why");
    assert_eq!(value["sections"]["requirements"], "what");
    assert_eq!(value["sections"]["benefits"], "value");
    assert_eq!(value["recent_notes"].as_array().unwrap().len(), 5);
    assert_eq!(value["depends_on"], serde_json::json!([blocker]));
    assert_eq!(value["blocks"], serde_json::json!([blocked]));
}

#[tokio::test]
async fn get_task_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .get_task(Parameters(TaskIdParams { task_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("task not found"));
}

#[tokio::test]
async fn delete_task_with_dependents_is_refused_until_confirmed() {
    let (server, repo, pid, vid, tid) = seeded(Status::Todo);
    let dependent = repo
        .create_task(pid, vid, "Dependent", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(dependent, tid).unwrap();

    let refused = server
        .delete_task(Parameters(DeleteTaskParams {
            task_id: tid,
            confirm: false,
        }))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true));
    let info = tool_error_info(&refused);
    assert_eq!(info["code"], "confirmation_required");
    assert_eq!(info["blockers"], serde_json::json!([dependent]));
    assert!(repo.get_task(tid).is_ok());

    let deleted = server
        .delete_task(Parameters(DeleteTaskParams {
            task_id: tid,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_ne!(deleted.is_error, Some(true));
    let value: serde_json::Value = serde_json::from_str(&tool_text(deleted)).unwrap();
    assert_eq!(value["kind"], "task");
    assert_eq!(value["removed"]["tasks"]["todo"], 1);
    assert!(repo.get_task(tid).is_err());
}

#[tokio::test]
async fn delete_free_todo_task_needs_no_confirmation() {
    let (server, repo, _, _, tid) = seeded(Status::Todo);
    let result = server
        .delete_task(Parameters(DeleteTaskParams {
            task_id: tid,
            confirm: false,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(repo.get_task(tid).is_err());
}

#[tokio::test]
async fn delete_task_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .delete_task(Parameters(DeleteTaskParams {
            task_id: 999,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("task not found"));
}

#[tokio::test]
async fn delete_version_with_active_work_is_refused_and_keeps_data() {
    let (server, repo, _, vid, tid) = seeded(Status::Todo);
    let refused = server
        .delete_version(Parameters(DeleteVersionParams {
            version_id: vid,
            confirm: false,
        }))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true));
    let info = tool_error_info(&refused);
    assert_eq!(info["code"], "confirmation_required");
    assert_eq!(info["blockers"], serde_json::json!([tid]));
    assert!(tool_text_clone(&refused).contains("confirm=true"));
    assert!(repo.get_version(vid).is_ok());
    assert!(repo.get_task(tid).is_ok());
}

#[tokio::test]
async fn delete_version_with_only_done_tasks_reports_removal() {
    let (server, repo, pid, vid, tid) = seeded(Status::Done);
    let other = repo.create_version(pid, "1.0.0", "").unwrap().id.unwrap();
    let result = server
        .delete_version(Parameters(DeleteVersionParams {
            version_id: vid,
            confirm: false,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["kind"], "version");
    assert_eq!(value["removed"]["tasks"]["done"], 1);
    assert!(repo.get_task(tid).is_err());
    assert!(repo.get_version(other).is_ok());
}

#[tokio::test]
async fn delete_version_with_confirmation_removes_active_work() {
    let (server, repo, _, vid, tid) = seeded(Status::InProgress);
    let result = server
        .delete_version(Parameters(DeleteVersionParams {
            version_id: vid,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(repo.get_task(tid).is_err());
}

#[tokio::test]
async fn delete_version_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .delete_version(Parameters(DeleteVersionParams {
            version_id: 999,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("version not found"));
}

#[tokio::test]
async fn delete_project_with_active_work_is_refused_and_keeps_data() {
    let (server, repo, pid, _, tid) = seeded(Status::Todo);
    let refused = server
        .delete_project(Parameters(DeleteProjectParams {
            project_id: pid,
            confirm: false,
        }))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true));
    let info = tool_error_info(&refused);
    assert_eq!(info["code"], "confirmation_required");
    assert_eq!(info["blockers"], serde_json::json!([tid]));
    assert!(repo.get_project(pid).is_ok());
}

#[tokio::test]
async fn delete_project_with_confirmation_reports_everything_removed() {
    let (server, repo, pid, _, tid) = seeded(Status::Todo);
    let result = server
        .delete_project(Parameters(DeleteProjectParams {
            project_id: pid,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["kind"], "project");
    assert_eq!(value["removed"]["versions"], 1);
    assert_eq!(value["removed"]["tasks"]["todo"], 1);
    assert!(repo.get_task(tid).is_err());
    assert!(repo.get_project(pid).is_err());
}

#[tokio::test]
async fn delete_project_unknown_returns_tool_error() {
    let (server, _, _, _, _) = seeded(Status::Todo);
    let result = server
        .delete_project(Parameters(DeleteProjectParams {
            project_id: 999,
            confirm: true,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text_clone(&result).contains("project not found"));
}

/// Text of the first content block, without consuming the result.
fn tool_text_clone(result: &CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|c| c.as_text().map(|t| t.text.clone()))
        .expect("tool result should have text content")
}
