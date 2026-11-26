// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn call_tool_dispatches_list_tasks_with_json_params() {
    let (server, _, pid) = seeded_server();
    let mut args = serde_json::Map::new();
    args.insert("project_id".to_string(), serde_json::json!(pid));
    args.insert("status".to_string(), serde_json::json!("done"));

    let request = rmcp::model::CallToolRequestParams::new("list_tasks").with_arguments(args);
    let (client, server_stream) = tokio::io::duplex(1024);
    let running =
        rmcp::service::serve_directly::<rmcp::RoleServer, _, _, _, _>(server, server_stream, None);
    let peer = running.peer().clone();
    let context = rmcp::service::RequestContext::new(rmcp::model::NumberOrString::Number(1), peer);

    let result = rmcp::ServerHandler::call_tool(running.service(), request, context)
        .await
        .unwrap();
    drop(client);
    drop(running);

    let text = tool_text(result);
    let tasks = page_tasks(&text);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, Status::Done);
}

#[tokio::test]
async fn mcp_server_e2e_lifecycle_and_list_tasks() {
    let repo = Arc::new(StubRepository::default());
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let default_version = repo.default_version_id(pid).unwrap();
    repo.create_task(
        pid,
        repo.default_version_id(pid).unwrap(),
        "Low task",
        "",
        Status::Todo,
        Priority::Low,
    )
    .unwrap();
    repo.create_task(
        pid,
        repo.default_version_id(pid).unwrap(),
        "High task",
        "",
        Status::Done,
        Priority::High,
    )
    .unwrap();

    let (client, server_stream) = tokio::io::duplex(1024);
    let server = tokio::spawn(async move {
        let server = PmpServer::new(repo);
        rmcp::service::serve_server(server, server_stream)
            .await
            .unwrap()
    });

    let (reader, mut writer) = tokio::io::split(client);
    let mut reader = tokio::io::BufReader::new(reader);

    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" }
        }
    });
    write_json_line(&mut writer, &init_request).await;
    let init_response = read_json_line(&mut reader).await;
    assert_eq!(init_response["jsonrpc"], "2.0");
    assert_eq!(init_response["id"], 1);
    let capabilities = &init_response["result"]["capabilities"];
    assert!(capabilities["tools"].is_object());
    assert!(capabilities["resources"].is_object());
    assert!(capabilities["prompts"].is_object());
    assert_eq!(capabilities["tools"]["listChanged"], false);
    assert_eq!(capabilities["resources"]["listChanged"], false);
    assert_eq!(capabilities["prompts"]["listChanged"], false);

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    write_json_line(&mut writer, &initialized).await;

    let tools_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    write_json_line(&mut writer, &tools_request).await;
    let tools_response = read_json_line(&mut reader).await;
    let tools = tools_response["result"]["tools"]
        .as_array()
        .expect("tools/list should return a tools array");
    assert!(tools.iter().any(|t| t["name"] == "list_tasks"));
    assert!(tools.iter().any(|t| t["name"] == "list_projects"));
    assert!(tools.iter().any(|t| t["name"] == "create_project"));
    assert!(tools.iter().any(|t| t["name"] == "update_project"));
    assert!(tools.iter().any(|t| t["name"] == "update_version"));
    assert!(tools.iter().any(|t| t["name"] == "update_task"));
    assert!(tools.iter().any(|t| t["name"] == "update_task_status"));
    assert!(tools.iter().any(|t| t["name"] == "create_task"));
    assert!(tools.iter().any(|t| t["name"] == "get_task_dependencies"));
    assert!(tools.iter().any(|t| t["name"] == "get_task_dependents"));
    assert!(tools.iter().any(|t| t["name"] == "add_task_dependency"));
    assert!(tools.iter().any(|t| t["name"] == "remove_task_dependency"));
    assert!(tools.iter().any(|t| t["name"] == "set_task_dependencies"));
    assert!(tools.iter().any(|t| t["name"] == "list_task_notes"));
    assert!(tools.iter().any(|t| t["name"] == "add_task_note"));
    assert!(tools.iter().any(|t| t["name"] == "delete_task_note"));
    assert!(tools.iter().any(|t| t["name"] == "move_task"));
    assert!(tools.iter().any(|t| t["name"] == "preview_compact_version"));
    assert!(tools.iter().any(|t| t["name"] == "compact_version"));
    assert!(tools.iter().any(|t| t["name"] == "list_version_summaries"));
    assert!(tools.iter().any(|t| t["name"] == "read_version_summary"));
    assert!(tools.iter().any(|t| t["name"] == "delete_version_summary"));
    assert!(tools.iter().any(|t| t["name"] == "export_version_summary"));
    assert!(
        tools
            .iter()
            .all(|t| t["annotations"].is_object() && t["outputSchema"].is_object()),
        "every tool advertises annotations and an outputSchema"
    );

    let prompts_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "prompts/list"
    });
    write_json_line(&mut writer, &prompts_request).await;
    let prompts_response = read_json_line(&mut reader).await;
    let prompt_names: Vec<&str> = prompts_response["result"]["prompts"]
        .as_array()
        .expect("prompts/list should return a prompts array")
        .iter()
        .filter_map(|prompt| prompt["name"].as_str())
        .collect();
    assert_eq!(
        prompt_names,
        [
            "implement-task",
            "plan-version",
            "release-notes",
            "review-version"
        ]
    );

    let prompt_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 21,
        "method": "prompts/get",
        "params": {
            "name": "plan-version",
            "arguments": { "version_id": default_version.to_string() }
        }
    });
    write_json_line(&mut writer, &prompt_request).await;
    let prompt_response = read_json_line(&mut reader).await;
    let prompt_text = prompt_response["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("the prompt should contain a text message");
    assert!(prompt_text.contains("Plan PMP version"));
    assert!(prompt_text.contains(&format!("#{default_version}")));

    let create_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "create_task",
            "arguments": {
                "project_id": pid,
                "version_id": default_version,
                "name": "E2E task",
                "context": "End-to-end context.",
                "requirements": "- Must pass",
                "benefits": "Confidence.",
                "priority": "High"
            }
        }
    });
    write_json_line(&mut writer, &create_request).await;
    let create_response = read_json_line(&mut reader).await;
    let created_text = create_response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should have text content");
    let created_task: Task = serde_json::from_str(created_text).unwrap();
    assert_eq!(
        create_response["result"]["structuredContent"],
        serde_json::from_str::<serde_json::Value>(created_text).unwrap()
    );
    assert_eq!(created_task.name, "E2E task");
    assert_eq!(created_task.status, Status::Todo);
    assert_eq!(created_task.priority, Priority::High);
    assert!(created_task.description.contains("## Context"));

    let call_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "list_tasks",
            "arguments": { "project_id": pid, "status": "done" }
        }
    });
    write_json_line(&mut writer, &call_request).await;
    let call_response = read_json_line(&mut reader).await;
    let content = call_response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should have text content");
    let tasks = page_tasks(content);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, Status::Done);

    drop(writer);
    drop(reader);
    server.await.unwrap();
}
