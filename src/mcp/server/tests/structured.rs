// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every successful tool response must keep its historical text block and add
//! the equivalent JSON in `structuredContent`.

use super::*;

fn result_text(result: &CallToolResult) -> &str {
    result.content[0]
        .as_text()
        .map(|text| text.text.as_str())
        .expect("the first content block is text")
}

#[tokio::test]
async fn object_results_mirror_the_textual_json() {
    let (server, _, pid) = seeded_server();
    let result = server
        .list_tasks(Parameters(ListTasksParams {
            project_id: pid,
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
        }))
        .await
        .unwrap();
    let text: serde_json::Value = serde_json::from_str(result_text(&result)).unwrap();
    assert_eq!(result.structured_content.as_ref(), Some(&text));
}

#[tokio::test]
async fn array_results_use_an_items_envelope_only_in_structured_content() {
    let (server, _, _) = seeded_server();
    let result = server.list_projects().await.unwrap();
    let text: serde_json::Value = serde_json::from_str(result_text(&result)).unwrap();
    assert!(
        text.is_array(),
        "the textual compatibility payload stays an array"
    );
    assert_eq!(
        result.structured_content.as_ref(),
        Some(&serde_json::json!({ "items": text }))
    );
}

#[tokio::test]
async fn tool_errors_keep_their_existing_shape() {
    let (server, _, _) = seeded_server();
    let result = server
        .get_project(Parameters(ProjectIdParams { project_id: 999 }))
        .await
        .unwrap();
    assert_eq!(result.content.len(), 2);
    assert!(!result_text(&result).is_empty());
    assert!(result.structured_content.is_none());
}

#[test]
fn scalar_tool_values_are_rejected() {
    let error = tool_success(&42).unwrap_err();
    assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
}
