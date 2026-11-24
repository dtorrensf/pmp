// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn set_task_dependencies_replaces_the_full_set() {
    let (server, repo, pid) = seeded_server();
    let version = repo.default_version_id(pid).unwrap();
    let mk = |name: &str| {
        repo.create_task(pid, version, name, "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap()
    };
    let (a, b, c) = (mk("A"), mk("B"), mk("C"));
    repo.add_dependency(a, b).unwrap();

    let result = server
        .set_task_dependencies(Parameters(SetTaskDependenciesParams {
            task_id: a,
            depends_on_ids: vec![c],
        }))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let body: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(body["depends_on_ids"], serde_json::json!([c]));
    assert_eq!(repo.get_dependencies(a).unwrap(), vec![c]);
}

#[tokio::test]
async fn set_task_dependencies_rejects_cycle_and_keeps_previous_set() {
    let (server, repo, pid) = seeded_server();
    let version = repo.default_version_id(pid).unwrap();
    let mk = |name: &str| {
        repo.create_task(pid, version, name, "", Status::Todo, Priority::Low)
            .unwrap()
            .id
            .unwrap()
    };
    let (a, b, c) = (mk("A"), mk("B"), mk("C"));
    repo.add_dependency(b, c).unwrap();
    repo.add_dependency(a, c).unwrap();

    // C depends on A would close A -> C -> A ... via B: B -> C -> A -> B
    let result = server
        .set_task_dependencies(Parameters(SetTaskDependenciesParams {
            task_id: c,
            depends_on_ids: vec![a],
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("dependency cycle"));
    assert!(repo.get_dependencies(c).unwrap().is_empty());
    assert_eq!(repo.get_dependencies(b).unwrap(), vec![c]);
}

#[tokio::test]
async fn move_task_with_dependencies_returns_error_and_keeps_version() {
    let (server, _, pid) = seeded_server();
    let v1 = server.repo.default_version_id(pid).unwrap();
    let v2 = server
        .repo
        .create_version(pid, "v0.2.0", "")
        .unwrap()
        .id
        .unwrap();
    let a = server
        .repo
        .create_task(pid, v1, "A", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    let b = server
        .repo
        .create_task(pid, v1, "B", "", Status::Todo, Priority::Low)
        .unwrap()
        .id
        .unwrap();
    server.repo.add_dependency(b, a).unwrap();
    let result = server
        .move_task(Parameters(MoveTaskParams {
            task_id: b,
            version_id: v2,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("dependencies or dependents"));
    assert_eq!(server.repo.get_task(b).unwrap().version_id, v1);
}

#[tokio::test]
async fn task_dependencies_can_be_added_read_and_removed() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let dependency = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "Dependency",
            "",
            Status::Done,
            Priority::Medium,
        )
        .unwrap();
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
    let dependency_id = dependency.id.unwrap();
    let task_id = task.id.unwrap();

    let result = server
        .add_task_dependency(Parameters(TaskDependencyParams {
            task_id,
            depends_on_id: dependency_id,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));

    let result = server
        .get_task_dependencies(Parameters(TaskIdParams { task_id }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["task_id"], task_id);
    assert_eq!(value["depends_on_ids"], serde_json::json!([dependency_id]));

    let result = server
        .get_task_dependents(Parameters(TaskIdParams {
            task_id: dependency_id,
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&tool_text(result)).unwrap();
    assert_eq!(value["task_ids"], serde_json::json!([task_id]));

    server
        .remove_task_dependency(Parameters(TaskDependencyParams {
            task_id,
            depends_on_id: dependency_id,
        }))
        .await
        .unwrap();
    assert!(repo.get_dependencies(task_id).unwrap().is_empty());
}

#[tokio::test]
async fn add_task_dependency_rejects_cycles() {
    let (server, repo) = server();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let a = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "A",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let b = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "B",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let c = repo
        .create_task(
            pid,
            repo.default_version_id(pid).unwrap(),
            "C",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap();
    let a_id = a.id.unwrap();
    let b_id = b.id.unwrap();
    let c_id = c.id.unwrap();

    repo.add_dependency(a_id, b_id).unwrap();
    repo.add_dependency(b_id, c_id).unwrap();
    let result = server
        .add_task_dependency(Parameters(TaskDependencyParams {
            task_id: c_id,
            depends_on_id: a_id,
        }))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("dependency cycle detected"));
}

#[tokio::test]
async fn task_dependency_tools_reject_unknown_tasks() {
    let (server, _) = server();
    let result = server
        .get_task_dependencies(Parameters(TaskIdParams { task_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("task not found"));

    let result = server
        .add_task_dependency(Parameters(TaskDependencyParams {
            task_id: 999,
            depends_on_id: 1000,
        }))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(tool_text(result).contains("task not found"));
}
