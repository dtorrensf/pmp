// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn list_tasks_returns_all_tasks_for_project() {
    let (server, _, pid) = seeded_server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: None,
            priority: None,
            limit: None,
            offset: None,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let tasks = page_tasks(&text);
    assert_eq!(tasks.len(), 3);
}

#[tokio::test]
async fn list_tasks_paginates_with_limit_and_offset() {
    let (server, _, pid) = seeded_server();
    let first = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: None,
            priority: None,
            limit: Some(2),
            offset: None,
        }))
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_str(&tool_text(first)).unwrap();
    assert_eq!(page["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(page["total"], 3);
    assert_eq!(page["has_more"], true);

    let second = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: None,
            priority: None,
            limit: Some(2),
            offset: Some(2),
        }))
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_str(&tool_text(second)).unwrap();
    assert_eq!(page["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], 3);
    assert_eq!(page["has_more"], false);
}

#[tokio::test]
async fn list_tasks_filters_by_status() {
    let (server, _, pid) = seeded_server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: Some(Status::Done),
            priority: None,
            limit: None,
            offset: None,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let tasks = page_tasks(&text);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, Status::Done);
}

#[tokio::test]
async fn list_tasks_filters_by_priority() {
    let (server, _, pid) = seeded_server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: None,
            priority: Some(PriorityParam::High),
            limit: None,
            offset: None,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let tasks = page_tasks(&text);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].priority, Priority::High);
}

#[tokio::test]
async fn list_tasks_filters_by_status_and_priority() {
    let (server, _, pid) = seeded_server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
            version_id: None,
            status: Some(Status::Todo),
            priority: Some(PriorityParam::Low),
            limit: None,
            offset: None,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let tasks = page_tasks(&text);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, Status::Todo);
    assert_eq!(tasks[0].priority, Priority::Low);
}

#[tokio::test]
async fn list_tasks_unknown_project_returns_tool_error() {
    let (server, _) = server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: 999,
            version_id: None,
            status: None,
            priority: None,
            limit: None,
            offset: None,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("project not found"));
}

#[tokio::test]
async fn update_task_status_changes_status() {
    let (server, _, pid) = seeded_server();
    let task_id = {
        let tasks = server.repo.list_tasks_by_project(pid).unwrap();
        tasks
            .iter()
            .find(|t| t.status == Status::Todo)
            .unwrap()
            .id
            .unwrap()
    };
    let result = server
        .update_task_status(Parameters(UpdateTaskStatusParams {
            task_id,
            status: Status::InProgress,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let task: Task = serde_json::from_str(&text).unwrap();
    assert_eq!(task.status, Status::InProgress);
    assert_eq!(task.id, Some(task_id));
}

#[tokio::test]
async fn update_task_status_unknown_task_returns_tool_error() {
    let (server, _) = server();
    let result = server
        .update_task_status(Parameters(UpdateTaskStatusParams {
            task_id: 999,
            status: Status::Done,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("task not found"));
}

#[tokio::test]
async fn update_task_changes_only_given_sections_and_keeps_status() {
    let (server, repo, pid) = seeded_server();
    let version = repo.default_version_id(pid).unwrap();
    let created = repo
        .create_task(
            pid,
            version,
            "Original",
            &compose_description("ctx", "req", "ben"),
            Status::InProgress,
            Priority::High,
        )
        .unwrap();
    let task_id = created.id.unwrap();

    let updated: Task = serde_json::from_str(&tool_text(
        server
            .update_task(Parameters(UpdateTaskParams {
                task_id,
                name: None,
                context: None,
                requirements: Some("- new requirement".to_string()),
                benefits: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(updated.name, "Original");
    assert_eq!(updated.status, Status::InProgress);
    assert_eq!(updated.priority, Priority::High);
    assert_eq!(
        updated.description,
        compose_description("ctx", "- new requirement", "ben")
    );
}

#[tokio::test]
async fn update_task_rename_keeps_description_untouched() {
    let (server, repo, pid) = seeded_server();
    let version = repo.default_version_id(pid).unwrap();
    let description = "hand written, no headings";
    let task_id = repo
        .create_task(
            pid,
            version,
            "Old",
            description,
            Status::Todo,
            Priority::Low,
        )
        .unwrap()
        .id
        .unwrap();

    let updated: Task = serde_json::from_str(&tool_text(
        server
            .update_task(Parameters(UpdateTaskParams {
                task_id,
                name: Some("New".to_string()),
                context: None,
                requirements: None,
                benefits: None,
            }))
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(updated.name, "New");
    assert_eq!(updated.description, description);
}

#[tokio::test]
async fn update_task_rejects_blank_name_and_unknown_task() {
    let (server, repo, pid) = seeded_server();
    let version = repo.default_version_id(pid).unwrap();
    let task_id = repo
        .create_task(pid, version, "T", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();

    let blank = server
        .update_task(Parameters(UpdateTaskParams {
            task_id,
            name: Some("   ".to_string()),
            context: None,
            requirements: None,
            benefits: None,
        }))
        .await
        .unwrap();
    assert_eq!(blank.is_error, Some(true));
    assert!(tool_text(blank).contains("task name cannot be empty"));

    let unknown = server
        .update_task(Parameters(UpdateTaskParams {
            task_id: 999_999,
            name: Some("X".to_string()),
            context: None,
            requirements: None,
            benefits: None,
        }))
        .await
        .unwrap();
    assert_eq!(unknown.is_error, Some(true));
    assert!(tool_text(unknown).contains("task not found"));
}

#[tokio::test]
async fn move_task_unknown_ids_return_error() {
    let (server, _, pid) = seeded_server();
    let v1 = server.repo.default_version_id(pid).unwrap();
    let task_id = server.repo.list_tasks_by_version(v1).unwrap()[0]
        .id
        .unwrap();
    let result = server
        .move_task(Parameters(MoveTaskParams {
            task_id: 999_999,
            version_id: v1,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let result = server
        .move_task(Parameters(MoveTaskParams {
            task_id,
            version_id: 999_999,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn update_task_priority_changes_priority() {
    let (server, _, pid) = seeded_server();
    let task_id = server
        .repo
        .list_tasks_by_project(pid)
        .unwrap()
        .iter()
        .find(|t| t.priority == Priority::Medium)
        .unwrap()
        .id
        .unwrap();
    let result = server
        .update_task_priority(Parameters(UpdateTaskPriorityParams {
            task_id,
            priority: PriorityParam::High,
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let task: Task = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(task.id, Some(task_id));
    assert_eq!(task.priority, Priority::High);
    assert_eq!(
        server.repo.get_task(task_id).unwrap().priority,
        Priority::High
    );
}

#[tokio::test]
async fn update_task_priority_unknown_task_returns_error() {
    let (server, _) = server();
    let result = server
        .update_task_priority(Parameters(UpdateTaskPriorityParams {
            task_id: 999,
            priority: PriorityParam::Low,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("task not found"));
}

#[tokio::test]
async fn update_task_status_rejects_done_with_incomplete_deps() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task_a = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task A",
            "",
            Status::Todo,
            Priority::High,
        )
        .unwrap();
    let task_b = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task B",
            "",
            Status::Todo,
            Priority::High,
        )
        .unwrap();
    let task_a_id = task_a.id.unwrap();
    let task_b_id = task_b.id.unwrap();
    repo.add_dependency(task_b_id, task_a_id).unwrap();

    let result = server
        .update_task_status(Parameters(UpdateTaskStatusParams {
            task_id: task_b_id,
            status: Status::Done,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let info = tool_error_info(&result);
    let message = tool_text(result);
    assert!(message.contains("incomplete dependencies"));
    assert_eq!(info["code"], "dependency_blocked");
    assert_eq!(info["blockers"], serde_json::json!([task_a_id]));
    assert_eq!(info["retryable"], false);
}

#[tokio::test]
async fn update_task_status_allows_done_when_deps_complete() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let task_a = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task A",
            "",
            Status::Done,
            Priority::High,
        )
        .unwrap();
    let task_b = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Task B",
            "",
            Status::Todo,
            Priority::High,
        )
        .unwrap();
    let task_a_id = task_a.id.unwrap();
    let task_b_id = task_b.id.unwrap();
    repo.add_dependency(task_b_id, task_a_id).unwrap();

    let result = server
        .update_task_status(Parameters(UpdateTaskStatusParams {
            task_id: task_b_id,
            status: Status::Done,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let task: Task = serde_json::from_str(&text).unwrap();
    assert_eq!(task.status, Status::Done);
}

#[tokio::test]
async fn create_task_creates_todo_task_with_composed_description() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();

    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: pid,
            version_id: repo.default_version_id(pid).unwrap(),
            name: "New task".to_string(),
            context: "We need this.".to_string(),
            requirements: "- Do it well".to_string(),
            benefits: "Ship faster.".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let task: Task = serde_json::from_str(&text).unwrap();
    assert_eq!(task.name, "New task");
    assert_eq!(task.status, Status::Todo);
    assert_eq!(task.priority, Priority::Medium);
    assert!(task.description.contains("## Context"));
    assert!(task.description.contains("We need this."));
    assert!(task.description.contains("## Requirements"));
    assert!(task.description.contains("- Do it well"));
    assert!(task.description.contains("## Benefits"));
    assert!(task.description.contains("Ship faster."));
}

#[tokio::test]
async fn create_task_uses_provided_priority() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();

    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: pid,
            version_id: repo.default_version_id(pid).unwrap(),
            name: "High priority task".to_string(),
            context: "".to_string(),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::High,
        }))
        .await
        .unwrap();
    let text = tool_text(result);
    let task: Task = serde_json::from_str(&text).unwrap();
    assert_eq!(task.priority, Priority::High);
}

#[tokio::test]
async fn create_task_unknown_project_returns_tool_error() {
    let (server, _) = server();
    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: 999,
            version_id: 1,
            name: "Task".to_string(),
            context: "".to_string(),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("project not found"));
}

#[tokio::test]
async fn create_task_empty_name_returns_tool_error() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();

    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: pid,
            version_id: repo.default_version_id(pid).unwrap(),
            name: "   ".to_string(),
            context: "".to_string(),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("task name cannot be empty"));
}

#[tokio::test]
async fn create_task_rejects_oversized_arguments() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();

    let result = server
        .create_task(Parameters(CreateTaskParams {
            project_id: pid,
            version_id: repo.default_version_id(pid).unwrap(),
            name: "Task".to_string(),
            context: "a".repeat(MAX_TEXT_LEN + 1),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let message = tool_text(result);
    assert!(message.contains("cannot exceed"), "{message}");
}
