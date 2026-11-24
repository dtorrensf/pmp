// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn preview_compact_version_returns_snapshot_json() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let task = repo
        .create_task(
            pid,
            version.id.unwrap(),
            "A",
            "",
            Status::Done,
            Priority::Low,
        )
        .unwrap();
    repo.add_task_note(task.id.unwrap(), "done!").unwrap();

    let result = server
        .preview_compact_version(Parameters(VersionIdParams {
            version_id: version.id.unwrap(),
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["version"]["name"], "v1.0.0");
    assert_eq!(value["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(value["notes"].as_array().unwrap().len(), 1);
    assert!(value["previous_summary"].is_null());
}

#[tokio::test]
async fn compact_version_rejects_tasks_in_progress() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    repo.create_task(pid, vid, "Working", "", Status::InProgress, Priority::Low)
        .unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = tool_text(result);
    assert!(text.contains("in progress"));
}

#[tokio::test]
async fn compact_version_rejects_empty_version_without_summary() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("nothing to compact"));
}

#[tokio::test]
async fn compact_version_compacts_and_marks_released() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    let task = repo
        .create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    repo.add_task_note(task.id.unwrap(), "note").unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(summary.summary, "Summary");
    assert_eq!(summary.version_id, vid);
    assert!(repo.list_tasks_by_version(vid).unwrap().is_empty());
    assert_eq!(
        repo.get_version(vid).unwrap().status,
        VersionStatus::Released
    );
}

#[tokio::test]
async fn compact_version_respects_mark_released_false() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    repo.update_version(vid, "v1.0.0", "", VersionStatus::Open)
        .unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: Some(false),
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(summary.summary, "Summary");
    assert_eq!(repo.get_version(vid).unwrap().status, VersionStatus::Open);
}

#[tokio::test]
async fn list_read_and_delete_version_summaries() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();

    let listed = server
        .list_version_summaries(Parameters(VersionIdParams { version_id: vid }))
        .await
        .unwrap();
    let summaries: Vec<VersionSummary> = serde_json::from_str(&tool_text(listed)).unwrap();
    assert_eq!(summaries.len(), 1);
    let id = summaries[0].id.unwrap();

    let read = server
        .read_version_summary(Parameters(ReadVersionSummaryParams {
            version_id: vid,
            summary_id: id,
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(read)).unwrap();
    assert_eq!(summary.summary, "Summary");

    let deleted = server
        .delete_version_summary(Parameters(DeleteVersionSummaryParams {
            version_id: vid,
            summary_id: id,
        }))
        .await
        .unwrap();
    assert_eq!(deleted.is_error, Some(false));
    assert!(repo.get_version_summary(id).is_err());
}

#[tokio::test]
async fn delete_version_summary_rejects_mismatched_version() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let id = repo.list_version_summaries(vid).unwrap()[0].id.unwrap();

    let result = server
        .delete_version_summary(Parameters(DeleteVersionSummaryParams {
            version_id: 999_999,
            summary_id: id,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("summary not found"));
    assert!(repo.get_version_summary(id).is_ok(), "summary must survive");
}

#[tokio::test]
async fn read_version_summary_rejects_mismatched_version() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summaries = repo.list_version_summaries(vid).unwrap();
    let id = summaries[0].id.unwrap();

    let result = server
        .read_version_summary(Parameters(ReadVersionSummaryParams {
            version_id: 999_999,
            summary_id: id,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("summary not found"));
}

#[tokio::test]
async fn compact_version_recompacts_preserving_previous_summary() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();

    // First cycle: one done task, compact it.
    let first = repo
        .create_task(pid, vid, "First", "", Status::Todo, Priority::Low)
        .unwrap();
    repo.update_task_status(first.id.unwrap(), Status::Done)
        .unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "First summary".to_string(),
            mark_released: Some(false),
        }))
        .await
        .unwrap();

    // Second cycle: add a new task to the compacted version and recompact.
    let second = repo
        .create_task(pid, vid, "Second", "", Status::Todo, Priority::Low)
        .unwrap();
    repo.update_task_status(second.id.unwrap(), Status::Done)
        .unwrap();
    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Second summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(summary.summary, "Second summary");

    let snapshot: serde_json::Value = serde_json::from_str(&summary.snapshot_json).unwrap();
    let prev = &snapshot["previous_summary"];
    assert!(!prev.is_null());
    assert_eq!(prev["summary"], "First summary");
}

#[tokio::test]
async fn partial_compaction_freezes_only_done_and_keeps_version_open() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
    let done = repo
        .create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    let todo = repo
        .create_task(pid, vid, "Todo", "", Status::Todo, Priority::Low)
        .unwrap();
    repo.add_task_note(todo.id.unwrap(), "todo note").unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Partial".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(result)).unwrap();

    let snapshot: serde_json::Value = serde_json::from_str(&summary.snapshot_json).unwrap();
    let tasks = snapshot["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], done.id.unwrap());
    assert!(snapshot["notes"].as_array().unwrap().is_empty());

    let remaining = repo.list_tasks_by_version(vid).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, todo.id);
    assert_eq!(repo.get_version(vid).unwrap().status, VersionStatus::Open);
}

#[tokio::test]
async fn compact_version_rejects_when_in_progress_even_with_done_tasks() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
    repo.create_task(pid, vid, "Done", "", Status::Done, Priority::Low)
        .unwrap();
    repo.create_task(pid, vid, "WIP", "", Status::InProgress, Priority::Low)
        .unwrap();

    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(repo.list_version_summaries(vid).unwrap().is_empty());
    assert_eq!(repo.list_tasks_by_version(vid).unwrap().len(), 2);
}

#[tokio::test]
async fn partial_then_full_compaction_releases_version() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
    repo.create_task(pid, vid, "First", "", Status::Done, Priority::Low)
        .unwrap();
    let later = repo
        .create_task(pid, vid, "Later", "", Status::Todo, Priority::Low)
        .unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Partial".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    assert_eq!(repo.get_version(vid).unwrap().status, VersionStatus::Open);

    repo.update_task_status(later.id.unwrap(), Status::Done)
        .unwrap();
    let result = server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Final".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summary: VersionSummary = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(summary.summary, "Final");
    assert!(repo.list_tasks_by_version(vid).unwrap().is_empty());
    assert_eq!(
        repo.get_version(vid).unwrap().status,
        VersionStatus::Released
    );

    let snapshot: serde_json::Value = serde_json::from_str(&summary.snapshot_json).unwrap();
    assert_eq!(snapshot["previous_summary"]["summary"], "Partial");
}

#[tokio::test]
async fn export_version_summary_writes_markdown_with_mermaid() {
    let (server, repo) = server();
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let version = repo.create_version(pid, "v1.0.0", "").unwrap();
    let vid = version.id.unwrap();
    let task = repo
        .create_task(pid, vid, "A", "", Status::Done, Priority::High)
        .unwrap();
    repo.add_task_note(task.id.unwrap(), "done note").unwrap();
    server
        .compact_version(Parameters(CompactVersionParams {
            version_id: vid,
            summary: "Prose summary".to_string(),
            mark_released: None,
        }))
        .await
        .unwrap();
    let summaries = repo.list_version_summaries(vid).unwrap();
    let summary_id = summaries[0].id.unwrap();

    let result = server
        .export_version_summary(Parameters(ExportVersionSummaryParams { summary_id }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    let written = std::path::PathBuf::from(value["path"].as_str().unwrap());
    let exports = dirs::data_dir().unwrap().join("pmp").join("exports");
    assert!(written.starts_with(&exports), "{}", written.display());

    let markdown = std::fs::read_to_string(&written).unwrap();
    assert!(markdown.contains("Prose summary"));
    assert!(markdown.contains("```mermaid"));
    assert!(markdown.contains("pie title Tasks by status"));
    assert!(markdown.contains("pie title Tasks by priority"));
    assert!(markdown.contains("flowchart LR"));
    assert!(markdown.contains("done note"));
    std::fs::remove_file(written).unwrap();
}
