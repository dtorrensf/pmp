// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wire-contract tests for the ACP types: the exact field names pmp sends and
//! the tolerance it shows for the ones it does not know.

use serde_json::{Value, json};

use crate::acp::protocol::{
    AuthMethod, CLIENT_NAME, CLIENT_TITLE, CLIENT_VERSION, CancelNotification, ContentBlock,
    InitializeRequest, InitializeResponse, McpServer, NewSessionRequest, PROTOCOL_VERSION,
    PermissionOutcome, PromptRequest, RequestPermissionResponse, SessionId, SessionUpdate,
    SessionUpdateNotification, StopReason, ToolCallStatus, ToolKind,
};

#[test]
fn initialize_request_uses_the_documented_shape() {
    let value = serde_json::to_value(InitializeRequest::default()).unwrap();
    assert_eq!(value["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(value["clientCapabilities"], json!({}));
    assert_eq!(value["clientInfo"]["name"], CLIENT_NAME);
    assert_eq!(value["clientInfo"]["title"], CLIENT_TITLE);
    assert_eq!(value["clientInfo"]["version"], CLIENT_VERSION);
}

#[test]
fn initialize_response_parses_capabilities_and_auth_methods() {
    let response: InitializeResponse = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": { "image": true, "audio": false, "embeddedContext": true },
            "mcpCapabilities": { "http": true, "sse": false },
            "somethingNew": { "ignored": true },
        },
        "agentInfo": { "name": "claude-code-acp", "title": "Claude", "version": "0.4.0" },
        "authMethods": [
            { "id": "login", "name": "Log in", "type": "terminal", "description": "opens a browser" }
        ],
    }))
    .unwrap();
    assert_eq!(response.protocol_version, 1);
    assert!(response.agent_capabilities.load_session);
    assert!(response.agent_capabilities.prompt_capabilities.image);
    assert!(
        response
            .agent_capabilities
            .prompt_capabilities
            .embedded_context
    );
    assert!(response.agent_capabilities.mcp_capabilities.http);
    assert_eq!(
        response.agent_info,
        Some(crate::acp::protocol::Implementation {
            name: "claude-code-acp".to_string(),
            title: Some("Claude".to_string()),
            version: "0.4.0".to_string(),
        })
    );
    assert_eq!(
        response.auth_methods,
        vec![AuthMethod {
            id: "login".to_string(),
            name: "Log in".to_string(),
            description: Some("opens a browser".to_string()),
            kind: Some("terminal".to_string()),
        }]
    );
    assert_eq!(response.agent_label(), "claude-code-acp 0.4.0");
}

#[test]
fn initialize_response_defaults_everything_but_the_version() {
    let response: InitializeResponse =
        serde_json::from_value(json!({ "protocolVersion": 1 })).unwrap();
    assert!(!response.agent_capabilities.load_session);
    assert!(!response.agent_capabilities.prompt_capabilities.audio);
    assert!(response.agent_info.is_none());
    assert!(response.auth_methods.is_empty());
    assert_eq!(response.agent_label(), "unknown agent");
}

#[test]
fn new_session_request_names_the_cwd_and_the_mcp_servers() {
    let request = NewSessionRequest {
        cwd: "/home/dev/project".to_string(),
        mcp_servers: vec![McpServer {
            name: "pmp".to_string(),
            command: "/usr/local/bin/pmp".to_string(),
            args: vec!["mcp".to_string()],
            env: Vec::new(),
        }],
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["cwd"], "/home/dev/project");
    assert_eq!(value["mcpServers"][0]["name"], "pmp");
    assert_eq!(value["mcpServers"][0]["command"], "/usr/local/bin/pmp");
    assert_eq!(value["mcpServers"][0]["args"], json!(["mcp"]));

    let bare = NewSessionRequest {
        cwd: "/tmp".to_string(),
        mcp_servers: Vec::new(),
    };
    let value = serde_json::to_value(&bare).unwrap();
    assert!(
        value.get("mcpServers").is_none(),
        "an empty server list is omitted"
    );
}

#[test]
fn prompt_request_carries_text_blocks_with_a_snake_case_discriminator() {
    let request = PromptRequest {
        session_id: SessionId::new("sess_1"),
        prompt: vec![ContentBlock::text("implement C1")],
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["sessionId"], "sess_1");
    assert_eq!(
        value["prompt"],
        json!([{ "type": "text", "text": "implement C1" }])
    );
}

#[test]
fn cancel_notification_uses_the_camel_case_session_id() {
    let value = serde_json::to_value(CancelNotification {
        session_id: SessionId::new("sess_1"),
    })
    .unwrap();
    assert_eq!(value, json!({ "sessionId": "sess_1" }));
}

#[test]
fn message_chunks_parse_with_their_optional_message_id() {
    let notification: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "messageId": "msg_1",
            "content": { "type": "text", "text": "hello" },
        },
    }))
    .unwrap();
    assert_eq!(notification.session_id.as_str(), "sess_1");
    assert_eq!(notification.text(), Some("hello"));
    assert_eq!(notification.update.kind(), "agent_message_chunk");
    match &notification.update {
        SessionUpdate::AgentMessageChunk { message_id, .. } => {
            assert_eq!(message_id.as_deref(), Some("msg_1"));
        }
        other => panic!("unexpected update: {other:?}"),
    }
}

#[test]
fn thought_and_user_chunks_are_distinguished() {
    for (kind, payload) in [
        (
            "user_message_chunk",
            json!({ "type": "text", "text": "do it" }),
        ),
        (
            "agent_thought_chunk",
            json!({ "type": "text", "text": "thinking" }),
        ),
    ] {
        let notification: SessionUpdateNotification = serde_json::from_value(json!({
            "sessionId": "sess_1",
            "update": { "sessionUpdate": kind, "content": payload },
        }))
        .unwrap();
        assert_eq!(notification.update.kind(), kind);
        assert!(notification.text().is_some());
    }
}

#[test]
fn tool_calls_and_updates_parse_their_open_enums() {
    let created: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "cargo test",
            "kind": "execute",
            "status": "in_progress",
            "content": [{ "type": "content", "content": { "type": "text", "text": "running" } }],
            "locations": [{ "path": "/home/dev/project" }],
            "rawInput": { "command": "cargo test" },
        },
    }))
    .unwrap();
    match created.update {
        SessionUpdate::ToolCall {
            tool_call_id,
            title,
            kind,
            status,
            content,
            locations,
            raw_input,
        } => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(title.as_deref(), Some("cargo test"));
            assert_eq!(kind, Some(ToolKind::from("execute")));
            assert_eq!(status, Some(ToolCallStatus::from("in_progress")));
            assert_eq!(content.len(), 1);
            assert_eq!(locations.len(), 1);
            assert_eq!(raw_input, Some(json!({ "command": "cargo test" })));
        }
        other => panic!("unexpected update: {other:?}"),
    }

    let updated: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "completed",
            "rawOutput": { "exit": 0 },
        },
    }))
    .unwrap();
    match updated.update {
        SessionUpdate::ToolCallUpdate {
            tool_call_id,
            status,
            content,
            raw_output,
            ..
        } => {
            assert_eq!(tool_call_id, "call_1");
            let status = status.expect("status is present");
            assert!(status.is_finished());
            assert!(content.is_none(), "omitted fields stay absent");
            assert_eq!(raw_output, Some(json!({ "exit": 0 })));
        }
        other => panic!("unexpected update: {other:?}"),
    }
}

#[test]
fn plans_and_available_commands_parse() {
    let plan: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "plan",
            "entries": [
                { "content": "read the task", "priority": "high", "status": "completed" },
                { "content": "write the code", "priority": "medium", "status": "pending" },
            ],
        },
    }))
    .unwrap();
    match plan.update {
        SessionUpdate::Plan { entries } => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].content, "read the task");
            assert_eq!(entries[0].status, "completed");
        }
        other => panic!("unexpected update: {other:?}"),
    }

    let commands: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "available_commands_update",
            "availableCommands": [{ "name": "compact", "description": "compact the session" }],
        },
    }))
    .unwrap();
    match commands.update {
        SessionUpdate::AvailableCommandsUpdate { available_commands } => {
            assert_eq!(available_commands.len(), 1)
        }
        other => panic!("unexpected update: {other:?}"),
    }

    let mode: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": { "sessionUpdate": "current_mode_update", "currentModeId": "accept_edits" },
    }))
    .unwrap();
    assert_eq!(mode.update.kind(), "current_mode_update");
}

#[test]
fn unknown_updates_and_content_blocks_degrade_instead_of_failing() {
    let future: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": { "sessionUpdate": "usage_update", "tokens": { "input": 42 } },
    }))
    .unwrap();
    assert_eq!(future.update, SessionUpdate::Unknown);
    assert_eq!(future.update.kind(), "unknown");
    assert_eq!(future.text(), None);

    let block: SessionUpdateNotification = serde_json::from_value(json!({
        "sessionId": "sess_1",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "video", "data": "...", "mimeType": "video/mp4" },
        },
    }))
    .unwrap();
    match block.update {
        SessionUpdate::AgentMessageChunk {
            content,
            message_id,
        } => {
            assert_eq!(content, ContentBlock::Other);
            assert_eq!(content.as_text(), None);
            assert!(message_id.is_none());
        }
        other => panic!("unexpected update: {other:?}"),
    }
}

#[test]
fn stop_reasons_keep_the_values_they_do_not_understand() {
    assert!(StopReason::from("end_turn").is_end_turn());
    assert!(StopReason::from("cancelled").is_cancelled());
    assert!(StopReason::from("refusal").is_refusal());
    let unknown = StopReason::from("some_future_reason");
    assert_eq!(unknown.as_str(), "some_future_reason");
    assert_eq!(unknown.to_string(), "some_future_reason");
    assert!(!unknown.is_end_turn());
}

#[test]
fn tool_call_status_knows_its_terminal_states() {
    assert!(!ToolCallStatus::from("pending").is_finished());
    assert!(!ToolCallStatus::from("in_progress").is_finished());
    assert!(ToolCallStatus::from("completed").is_finished());
    assert!(ToolCallStatus::from("failed").is_finished());
    assert_eq!(ToolKind::from("read").as_str(), "read");
    assert_eq!(ToolKind::from("read").to_string(), "read");
}

#[test]
fn permission_outcomes_serialize_as_the_protocol_expects() {
    let cancelled = serde_json::to_value(RequestPermissionResponse::cancelled()).unwrap();
    assert_eq!(cancelled, json!({ "outcome": { "outcome": "cancelled" } }));

    let selected = serde_json::to_value(RequestPermissionResponse {
        outcome: PermissionOutcome::Selected {
            option_id: "allow".to_string(),
        },
    })
    .unwrap();
    assert_eq!(
        selected,
        json!({ "outcome": { "outcome": "selected", "optionId": "allow" } })
    );
}

#[test]
fn session_ids_are_transparent_strings() {
    let id = SessionId::new("sess_1");
    assert_eq!(id.as_str(), "sess_1");
    assert_eq!(id.as_ref(), "sess_1");
    assert_eq!(id.to_string(), "sess_1");
    assert!(!id.is_empty());
    assert!(SessionId::new("").is_empty());
    assert_eq!(
        serde_json::to_value(&id).unwrap(),
        Value::String("sess_1".to_string())
    );
    assert_eq!(SessionId::from("sess_2"), SessionId::new("sess_2"));
}
