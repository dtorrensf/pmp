// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Prompt discovery and rendering, both through the router and through the
//! `ServerHandler` methods a real MCP client reaches.

use super::*;
use rmcp::ServerHandler;
use rmcp::model::{GetPromptRequestParams, GetPromptResult};

fn seeded_prompt_server() -> (PmpServer, Arc<StubRepository>, i64) {
    let repo = Arc::new(StubRepository::default());
    let pid = repo.create_project("Alpha", "", None).unwrap().id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let blocker = repo
        .create_task(
            pid,
            vid,
            "Shared use case",
            "",
            Status::InProgress,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap();
    let task_id = repo
        .create_task(
            pid,
            vid,
            "Implement M7",
            "## Context\n\nThe server needs prompts.\n\n## Requirements\n\nAdd MCP annotations.\n\n## Benefits\n\nClients get a stable contract.",
            Status::Todo,
            Priority::High,
        )
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(task_id, blocker).unwrap();
    (PmpServer::new(repo.clone()), repo, task_id)
}

async fn get_prompt(
    server: PmpServer,
    name: &str,
    arguments: serde_json::Value,
) -> Result<GetPromptResult, ErrorData> {
    let request = GetPromptRequestParams::new(name)
        .with_arguments(arguments.as_object().cloned().unwrap_or_default());
    let (client, server_stream) = tokio::io::duplex(1024);
    let running =
        rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(server, server_stream, None);
    let context = rmcp::service::RequestContext::new(
        rmcp::model::NumberOrString::Number(1),
        running.peer().clone(),
    );
    let result = ServerHandler::get_prompt(running.service(), request, context).await;
    drop(client);
    drop(running);
    result
}

fn prompt_text(result: &GetPromptResult) -> &str {
    result.messages[0]
        .content
        .as_text()
        .map(|text| text.text.as_str())
        .expect("a prompt must be a text message")
}

#[test]
fn prompt_router_lists_the_four_standard_prompts() {
    let mut prompts = PmpServer::prompt_router().list_all();
    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "implement-task",
            "plan-version",
            "release-notes",
            "review-version"
        ]
    );
    for prompt in prompts {
        let arguments = prompt.arguments.expect("prompts need arguments");
        assert_eq!(arguments.len(), 1);
        assert_eq!(arguments[0].required, Some(true));
    }
}

#[tokio::test]
async fn implement_task_prompt_embeds_task_sections_and_blockers() {
    let (_, repo, task_id) = seeded_prompt_server();
    let detail = crate::application::records::task_detail(repo.as_ref(), task_id).unwrap();
    let dependencies = crate::application::dependencies::view(repo.as_ref(), task_id).unwrap();
    let expected = crate::application::prompts::implement_task(&detail, &dependencies);

    let result = get_prompt(
        PmpServer::new(repo),
        "implement-task",
        serde_json::json!({ "task_id": task_id.to_string() }),
    )
    .await
    .expect("the prompt is rendered");
    assert_eq!(result.messages.len(), 1);
    assert_eq!(result.messages[0].role, rmcp::model::Role::User);
    let text = prompt_text(&result);
    for needle in [
        "Implement M7",
        "The server needs prompts.",
        "Add MCP annotations.",
        "Clients get a stable contract.",
        "Shared use case",
        "in_progress",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert_eq!(text, expected, "MCP and ACP must share the same template");
}

#[tokio::test]
async fn version_prompts_carry_the_requested_id() {
    let (_, repo, _) = seeded_prompt_server();
    let version_id = repo.default_version_id(1).unwrap();
    for (name, marker) in [
        ("plan-version", "Plan PMP version"),
        ("review-version", "Review PMP version"),
        ("release-notes", "Write release notes"),
    ] {
        let result = get_prompt(
            PmpServer::new(repo.clone()),
            name,
            serde_json::json!({ "version_id": version_id.to_string() }),
        )
        .await
        .unwrap();
        let text = prompt_text(&result);
        assert!(text.contains(&format!("#{version_id}")), "{name}: {text}");
        assert!(text.contains(marker), "{name}: {text}");
    }
}

#[tokio::test]
async fn invalid_prompt_requests_are_invalid_params() {
    let (server, _, _) = seeded_prompt_server();
    let unknown = get_prompt(server, "not-a-prompt", serde_json::json!({}))
        .await
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::INVALID_PARAMS);

    let (server, _, _) = seeded_prompt_server();
    let missing = get_prompt(server, "implement-task", serde_json::json!({}))
        .await
        .unwrap_err();
    assert_eq!(missing.code, ErrorCode::INVALID_PARAMS);

    let (server, _, _) = seeded_prompt_server();
    let invalid = get_prompt(
        server,
        "implement-task",
        serde_json::json!({ "task_id": "zero" }),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code, ErrorCode::INVALID_PARAMS);

    let (server, _, task_id) = seeded_prompt_server();
    let missing_task = get_prompt(
        server,
        "implement-task",
        serde_json::json!({ "task_id": (task_id + 999).to_string() }),
    )
    .await
    .unwrap_err();
    assert_eq!(missing_task.code, ErrorCode::INVALID_PARAMS);
    assert!(missing_task.message.contains("task not found"));
}
