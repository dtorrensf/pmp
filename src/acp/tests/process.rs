// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Process lifecycle tests: a profile that cannot start, a child that exits
//! on its own, and a client that is dropped without an orderly shutdown.

use std::time::Duration;

use crate::acp::{AcpClient, AcpEvent};
use crate::config::AgentProfile;
use crate::domain::error::ErrorCode;

fn profile(command: &str) -> AgentProfile {
    AgentProfile {
        command: command.to_string(),
        args: Vec::new(),
        env: Default::default(),
    }
}

#[tokio::test]
async fn a_missing_binary_is_an_unavailable_agent() {
    let missing = profile("pmp-agent-that-does-not-exist");
    let error = AcpClient::launch(&missing)
        .await
        .expect_err("the binary is not installed");
    assert_eq!(error.code(), ErrorCode::AgentUnavailable);
    assert!(error.is_retryable(), "installing it and retrying works");
    assert!(
        error.user_message().contains("was not found in PATH"),
        "{}",
        error.user_message()
    );
    assert!(
        error
            .user_message()
            .contains("pmp-agent-that-does-not-exist")
    );
}

#[tokio::test]
async fn launch_in_rejects_a_working_directory_that_is_not_one() {
    let shell = profile("/bin/sh");
    let error = AcpClient::launch_in(&shell, "/definitely/not/a/pmp/workspace")
        .await
        .expect_err("the directory does not exist");
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.user_message().contains("working directory"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_child_that_exits_immediately_closes_the_connection() {
    let mut shell = profile("/bin/sh");
    shell.args = vec!["-c".to_string(), "exit 3".to_string()];
    let client = AcpClient::launch(&shell).await.expect("the shell starts");
    let error = client.initialize().await.expect_err("the child is gone");
    assert_eq!(error.code(), ErrorCode::AgentFailed);
    assert!(client.is_closed());
    // The child may die before a subscriber exists, so the reason is stored
    // instead of relying on the broadcast; the Closed event itself is covered
    // by the simulated agent in the client tests.
    assert!(
        client.closed_reason().is_some(),
        "the connection remembers why it ended"
    );
    // Shutting down a child that already exited must not hang.
    tokio::time::timeout(Duration::from_secs(10), client.shutdown())
        .await
        .expect("shutdown returns");
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_client_kills_the_child() {
    // `sleep` ignores a closed stdin, so only the kill-on-drop guard can end
    // it. The test proves the guard works by waiting for the process to go.
    let mut shell = profile("/bin/sh");
    shell.args = vec!["-c".to_string(), "exec sleep 30".to_string()];
    let client = AcpClient::launch(&shell).await.expect("the shell starts");
    let mut events = client.subscribe();
    drop(client);

    let closed = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("the pipe closes")
        .expect("a closed event");
    assert!(matches!(closed, AcpEvent::Closed { .. }));
}
