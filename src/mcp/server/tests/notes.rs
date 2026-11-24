// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn list_task_notes_returns_notes_for_task() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let task_id = task.id.unwrap();
    repo.add_task_note(task_id, "first note").unwrap();
    repo.add_task_note(task_id, "second note").unwrap();

    let result = server
        .list_task_notes(Parameters(TaskIdParams { task_id }))
        .await
        .unwrap();
    let text = tool_text(result);
    let notes: Vec<TaskNote> = serde_json::from_str(&text).unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].content, "second note");
    assert_eq!(notes[1].content, "first note");
}

#[tokio::test]
async fn list_task_notes_unknown_task_returns_error() {
    let (server, _) = server();
    let result = server
        .list_task_notes(Parameters(TaskIdParams { task_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("task not found"));
}

#[tokio::test]
async fn add_task_note_appends_note() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let task_id = task.id.unwrap();

    let result = server
        .add_task_note(Parameters(AddTaskNoteParams {
            task_id,
            content: "new note".to_string(),
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let note: TaskNote = serde_json::from_str(&text).unwrap();
    assert_eq!(note.task_id, task_id);
    assert_eq!(note.content, "new note");
    assert!(note.id.is_some());

    let notes = repo.list_task_notes(task_id).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, "new note");
}

#[tokio::test]
async fn add_task_note_empty_content_returns_error() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let task_id = task.id.unwrap();

    let result = server
        .add_task_note(Parameters(AddTaskNoteParams {
            task_id,
            content: "   ".to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("note content cannot be empty"));
}

#[tokio::test]
async fn add_task_note_unknown_task_returns_error() {
    let (server, _) = server();
    let result = server
        .add_task_note(Parameters(AddTaskNoteParams {
            task_id: 999,
            content: "note".to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("task not found"));
}

#[tokio::test]
async fn delete_task_note_removes_note() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let task_id = task.id.unwrap();
    let note = repo.add_task_note(task_id, "to delete").unwrap();
    let note_id = note.id.unwrap();

    let result = server
        .delete_task_note(Parameters(NoteIdParams { note_id }))
        .await
        .unwrap();
    let text = tool_text(result);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["note_id"], note_id);

    assert!(repo.list_task_notes(task_id).unwrap().is_empty());
}

#[tokio::test]
async fn delete_task_note_unknown_note_returns_error() {
    let (server, _) = server();
    let result = server
        .delete_task_note(Parameters(NoteIdParams { note_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("note not found"));
}
