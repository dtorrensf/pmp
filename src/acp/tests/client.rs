// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client tests against the simulated agent: negotiation, session setup,
//! prompt streaming, cancellation, permission requests, unknown updates and
//! orderly shutdown.

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::acp::sim::{self, AgentAsk, SimOptions};
use crate::acp::{AcpClient, AcpEvent, ContentBlock, SessionId, SessionUpdate};
use crate::domain::error::ErrorCode;

/// A real directory, so the working-directory validation passes.
fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary workspace")
}

/// Collects the events published so far, after the prompt future resolved.
fn drain(events: &mut tokio::sync::broadcast::Receiver<AcpEvent>) -> Vec<AcpEvent> {
    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    collected
}

#[tokio::test]
async fn initialize_negotiates_the_protocol_and_reports_the_agent() {
    let (client, _agent) = sim::spawn(SimOptions::default()).await;
    let response = client.initialize().await.expect("initialize succeeds");
    assert_eq!(response.protocol_version, 1);
    assert!(response.agent_capabilities.load_session);
    assert!(response.agent_capabilities.prompt_capabilities.image);
    assert_eq!(response.agent_label(), "simulated 0.0.1");
    assert!(!client.is_closed());
    client.shutdown().await;
}

#[tokio::test]
async fn initialize_refuses_a_protocol_version_it_cannot_speak() {
    let (client, _agent) = sim::spawn(SimOptions {
        protocol_version: 2,
        ..SimOptions::default()
    })
    .await;
    let error = client.initialize().await.expect_err("v2 is refused");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(
        error.user_message().contains("ACP v2 and pmp supports v1"),
        "{}",
        error.user_message()
    );
    client.shutdown().await;
}

#[tokio::test]
async fn initialize_surfaces_the_agent_error() {
    let (client, _agent) = sim::spawn(SimOptions {
        fail_initialize: Some("no credits left".to_string()),
        ..SimOptions::default()
    })
    .await;
    let error = client.initialize().await.expect_err("the agent refused");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(error.user_message().contains("no credits left"));
    assert!(!error.is_retryable());
    client.shutdown().await;
}

#[tokio::test]
async fn garbage_from_the_agent_fails_the_request_and_closes_the_connection() {
    let (client, _agent) = sim::spawn(SimOptions {
        garbage: Some("{\"jsonrpc\":".to_string()),
        ..SimOptions::default()
    })
    .await;
    let mut events = client.subscribe();
    let error = client.initialize().await.expect_err("garbage is fatal");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(error.user_message().contains("invalid JSON"));
    assert!(client.is_closed());
    assert!(matches!(
        events.recv().await.expect("a closed event"),
        AcpEvent::Closed { .. }
    ));
    client.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_dies_closes_the_connection() {
    let (client, _agent) = sim::spawn(SimOptions {
        die_on_initialize: true,
        ..SimOptions::default()
    })
    .await;
    let error = client.initialize().await.expect_err("the agent is gone");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(client.is_closed());
    assert_eq!(
        client.initialize().await.expect_err("still closed").code(),
        ErrorCode::AgentFailed
    );
    client.shutdown().await;
}

#[tokio::test]
async fn a_hung_agent_hits_the_request_deadline() {
    let (client, _agent) = sim::spawn(SimOptions {
        hang: true,
        ..SimOptions::default()
    })
    .await;
    let client = client.with_request_timeout(Duration::from_millis(40));
    let error = client.initialize().await.expect_err("the deadline fires");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(
        error.user_message().contains("timed out after 40ms"),
        "{}",
        error.user_message()
    );
    client.shutdown().await;
}

#[tokio::test]
async fn new_session_sends_the_working_directory() {
    let (client, agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client
        .new_session(workspace.path())
        .await
        .expect("a session id");
    assert_eq!(session.as_str(), "sess-sim");
    let params = agent.session_params().expect("session/new was sent");
    let expected = std::path::absolute(workspace.path()).unwrap();
    assert_eq!(params["cwd"], expected.to_string_lossy().as_ref());
    assert!(
        params.get("mcpServers").is_none(),
        "no servers are injected yet"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn new_session_with_injects_mcp_servers() {
    use crate::acp::{McpServer, NewSessionRequest};

    let (client, agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let request = NewSessionRequest {
        cwd: workspace.path().to_string_lossy().into_owned(),
        mcp_servers: vec![McpServer {
            name: "pmp".to_string(),
            command: "/usr/local/bin/pmp".to_string(),
            args: vec!["mcp".to_string()],
            env: Vec::new(),
        }],
    };
    client.new_session_with(request).await.unwrap();
    let params = agent.session_params().expect("session/new was sent");
    assert_eq!(params["mcpServers"][0]["name"], "pmp");
    assert_eq!(params["mcpServers"][0]["command"], "/usr/local/bin/pmp");
    client.shutdown().await;
}

#[tokio::test]
async fn new_session_rejects_a_working_directory_that_is_not_one() {
    let (client, _agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();

    let relative = client
        .new_session(Path::new("definitely/not/here"))
        .await
        .expect_err("a relative path is not a directory");
    assert_eq!(relative.code(), ErrorCode::Validation);
    assert!(relative.user_message().contains("working directory"));

    let missing = client
        .new_session(Path::new("/definitely/not/a/pmp/workspace"))
        .await
        .expect_err("the directory does not exist");
    assert_eq!(missing.code(), ErrorCode::Validation);
    assert!(!missing.is_retryable());
    client.shutdown().await;
}

#[tokio::test]
async fn prompt_streams_updates_in_order_and_returns_the_stop_reason() {
    let options = SimOptions {
        updates: vec![
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "step one " },
            }),
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "checking the tests" },
            }),
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call-1",
                "title": "cargo test",
                "kind": "execute",
                "status": "in_progress",
            }),
            json!({
                "sessionUpdate": "plan",
                "entries": [{ "content": "verify", "priority": "high", "status": "pending" }],
            }),
        ],
        ..SimOptions::default()
    };
    let (client, agent) = sim::spawn(options).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let mut events = client.subscribe();

    let stop = client
        .prompt(&session, "implement C1")
        .await
        .expect("the turn finishes");
    assert!(stop.is_end_turn());

    let updates: Vec<_> = drain(&mut events)
        .into_iter()
        .map(|event| match event {
            AcpEvent::Update(notification) => notification,
            other => panic!("unexpected event: {other:?}"),
        })
        .collect();
    let kinds: Vec<&str> = updates.iter().map(|u| u.update.kind()).collect();
    assert_eq!(
        kinds,
        [
            "agent_message_chunk",
            "agent_thought_chunk",
            "tool_call",
            "plan"
        ]
    );
    assert_eq!(updates[0].text(), Some("step one "));
    assert_eq!(updates[0].session_id, session);
    match &updates[2].update {
        SessionUpdate::ToolCall { title, status, .. } => {
            assert_eq!(title.as_deref(), Some("cargo test"));
            assert_eq!(status.as_ref().map(|s| s.as_str()), Some("in_progress"));
        }
        other => panic!("unexpected update: {other:?}"),
    }

    let prompt = agent.prompt_params().expect("session/prompt was sent");
    assert_eq!(prompt["sessionId"], "sess-sim");
    assert_eq!(prompt["prompt"][0]["text"], "implement C1");
    client.shutdown().await;
}

#[tokio::test]
async fn prompt_blocks_serialize_every_block() {
    let (client, agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    client
        .prompt_blocks(
            &session,
            vec![ContentBlock::text("first"), ContentBlock::text("second")],
        )
        .await
        .unwrap();
    let prompt = agent.prompt_params().unwrap();
    assert_eq!(prompt["prompt"].as_array().unwrap().len(), 2);
    client.shutdown().await;
}

#[tokio::test]
async fn an_empty_prompt_is_a_validation_error() {
    let (client, _agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let error = client
        .prompt_blocks(&session, Vec::new())
        .await
        .expect_err("an empty prompt is rejected");
    assert_eq!(error.code(), ErrorCode::Validation);
    client.shutdown().await;
}

#[tokio::test]
async fn a_refusal_is_reported_as_a_stop_reason() {
    let (client, _agent) = sim::spawn(SimOptions {
        stop_reason: "refusal".to_string(),
        ..SimOptions::default()
    })
    .await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let stop = client.prompt(&session, "do something risky").await.unwrap();
    assert!(stop.is_refusal());
    client.shutdown().await;
}

#[tokio::test]
async fn cancel_stops_the_turn_with_the_cancelled_stop_reason() {
    let options = SimOptions {
        updates: vec![json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "working" },
        })],
        wait_for_cancel: true,
        ..SimOptions::default()
    };
    let (client, agent) = sim::spawn(options).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let mut events = client.subscribe();

    let prompt = tokio::spawn({
        let client = client.clone();
        let session = session.clone();
        async move { client.prompt(&session, "go").await }
    });
    // The agent streams one update before waiting for the cancel, so the test
    // waits for real progress instead of sleeping.
    let first = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an update arrives")
        .expect("the stream is open");
    assert!(matches!(first, AcpEvent::Update(_)));

    client.cancel(&session).await.expect("cancel is sent");
    let stop = prompt
        .await
        .expect("the task finishes")
        .expect("the turn ends");
    assert!(stop.is_cancelled());
    assert!(agent.was_cancelled());
    client.shutdown().await;
}

#[tokio::test]
async fn a_permission_request_is_answerable_without_a_ui() {
    let options = SimOptions {
        ask: Some(AgentAsk::permission(&SessionId::new("sess-sim"))),
        ..SimOptions::default()
    };
    let (client, agent) = sim::spawn(options).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let mut events = client.subscribe();

    let stop = client.prompt(&session, "run the tests").await.unwrap();
    assert!(stop.is_end_turn());

    let answers = agent.answers();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].method, "session/request_permission");
    assert_eq!(
        answers[0].result,
        Ok(json!({ "outcome": { "outcome": "cancelled" } })),
        "without a dialog the safe answer is cancelled"
    );
    let unserved = drain(&mut events)
        .into_iter()
        .find(|event| matches!(event, AcpEvent::Unserved { .. }))
        .expect("the UI is told about the request");
    assert!(matches!(
        unserved,
        AcpEvent::Unserved { ref method, .. } if method == "session/request_permission"
    ));
    client.shutdown().await;
}

#[tokio::test]
async fn a_method_the_client_never_advertised_is_rejected_with_method_not_found() {
    let options = SimOptions {
        ask: Some(AgentAsk::new(
            "fs/read_text_file",
            json!({ "sessionId": "sess-sim", "path": "/tmp/x" }),
        )),
        ..SimOptions::default()
    };
    let (client, agent) = sim::spawn(options).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();

    client.prompt(&session, "read a file").await.unwrap();
    let answers = agent.answers();
    let error = answers[0]
        .result
        .as_ref()
        .expect_err("the method is unsupported");
    assert_eq!(error.code, crate::acp::RpcError::METHOD_NOT_FOUND);
    assert!(client.initialize().await.is_ok(), "the session survives");
    client.shutdown().await;
}

#[tokio::test]
async fn an_unknown_update_does_not_break_the_turn() {
    let options = SimOptions {
        updates: vec![
            json!({ "sessionUpdate": "usage_update", "tokens": { "input": 10 } }),
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "still here" },
            }),
        ],
        ..SimOptions::default()
    };
    let (client, agent) = sim::spawn(options).await;
    client.initialize().await.unwrap();
    let workspace = workspace();
    let session = client.new_session(workspace.path()).await.unwrap();
    let mut events = client.subscribe();

    let stop = client.prompt(&session, "go").await.unwrap();
    assert!(stop.is_end_turn());
    let updates: Vec<_> = drain(&mut events)
        .into_iter()
        .filter_map(|event| match event {
            AcpEvent::Update(notification) => Some(notification.update),
            _ => None,
        })
        .collect();
    assert_eq!(
        updates,
        vec![
            SessionUpdate::Unknown,
            SessionUpdate::AgentMessageChunk {
                content: ContentBlock::text("still here"),
                message_id: None,
            },
        ]
    );
    assert!(agent.answers().is_empty());
    client.shutdown().await;
}

#[tokio::test]
async fn shutdown_closes_the_pipe_and_announces_it_once() {
    let (client, agent) = sim::spawn(SimOptions::default()).await;
    assert!(!client.is_closed());
    let mut events = client.subscribe();
    client.shutdown().await;
    assert!(client.is_closed());
    assert!(agent.finished().await, "the agent saw EOF and ended");
    assert!(matches!(
        events.recv().await.expect("a closed event"),
        AcpEvent::Closed { .. }
    ));
    let error = client
        .cancel(&SessionId::new("sess-sim"))
        .await
        .expect_err("the connection is closed");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(error.user_message().contains("closed"));
}

#[tokio::test]
async fn from_io_connects_over_existing_pipes() {
    // Nobody answers on the other end, so the deadline is what ends the call.
    let (client_write, _agent_read) = tokio::io::duplex(64 * 1024);
    let (_agent_write, client_read) = tokio::io::duplex(64 * 1024);
    let client = AcpClient::from_io(client_read, client_write)
        .await
        .with_request_timeout(Duration::from_millis(30));
    assert!(!client.is_closed());
    let error = client.initialize().await.expect_err("nobody answers");
    assert!(error.user_message().contains("timed out"));
    client.shutdown().await;
    assert!(client.is_closed());
}
