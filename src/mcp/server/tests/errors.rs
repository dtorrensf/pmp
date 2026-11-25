// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn db_error_details_are_not_sent_to_client() {
    let server = PmpServer::new(Arc::new(FailingRepository));
    let err = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: 1,
            version_id: None,
            status: None,
            priority: None,
            limit: None,
            offset: None,
            include_description: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    assert_eq!(err.message, "internal error");
    assert!(!err.message.contains("InvalidQuery"));
    assert_eq!(
        err.data.as_ref().and_then(|d| d.get("code")),
        Some(&serde_json::json!("internal"))
    );
}

#[test]
fn app_error_mapping_is_stable_for_each_variant() {
    let not_found = app_error_to_mcp(AppError::NotFound);
    assert_eq!(not_found.code, ErrorCode::RESOURCE_NOT_FOUND);
    let duplicate = app_error_to_mcp(AppError::DuplicateName("version already exists".into()));
    let data = duplicate.data.unwrap();
    assert_eq!(data["code"], "duplicate_name");
    assert_eq!(data["retryable"], false);

    let busy = app_error_to_mcp(AppError::DbBusy);
    let data = busy.data.unwrap();
    assert_eq!(data["code"], "db_busy");
    assert_eq!(data["retryable"], true);

    let blocked = app_error_to_mcp(AppError::DependencyBlocked {
        message: "cannot mark task as done: it has incomplete dependencies".into(),
        blockers: vec![3],
    });
    let data = blocked.data.unwrap();
    assert_eq!(data["code"], "dependency_blocked");
    assert_eq!(data["blockers"], serde_json::json!([3]));

    let db = app_error_to_mcp(AppError::DbError(rusqlite::Error::InvalidQuery));
    assert_eq!(db.code, ErrorCode::INTERNAL_ERROR);
    assert_eq!(db.message, "internal error");
    let validation = app_error_to_mcp(AppError::Validation("bad".into()));
    assert_eq!(validation.code, ErrorCode::INVALID_PARAMS);
    let cycle = app_error_to_mcp(AppError::DependencyCycle);
    assert_eq!(cycle.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn list_tasks_db_error_returns_internal_error() {
    let repo = Arc::new(FailingRepository);
    let server = PmpServer::new(repo);
    let err = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: 1,
            version_id: None,
            status: None,
            priority: None,
            limit: None,
            offset: None,
            include_description: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
}

#[tokio::test]
async fn update_task_status_db_error_returns_internal_error() {
    let repo = Arc::new(FailingRepository);
    let server = PmpServer::new(repo);
    let err = server
        .update_task_status(Parameters(UpdateTaskStatusParams {
            task_id: 1,
            status: Status::Done,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
}

#[tokio::test]
async fn create_task_db_error_returns_internal_error() {
    let repo = Arc::new(FailingRepository);
    let server = PmpServer::new(repo);
    let err = server
        .create_task(Parameters(CreateTaskParams {
            project_id: 1,
            version_id: 1,
            name: "Task".to_string(),
            context: "".to_string(),
            requirements: "".to_string(),
            benefits: "".to_string(),
            priority: PriorityParam::Medium,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
}
