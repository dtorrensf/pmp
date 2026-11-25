// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Framing tests: one JSON-RPC message per line, bounded reads, and clear
//! failures when the agent writes something that is not a protocol message.

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::acp::transport::{
    Incoming, JSONRPC_VERSION, MAX_MESSAGE_BYTES, RequestId, RpcError, TransportError, failure,
    notification, parse_message, read_message, request, success, write_message,
};

/// A pair of in-memory pipes big enough that a write never blocks a test.
fn pipes() -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
    tokio::io::duplex(1024 * 1024)
}

#[test]
fn envelopes_carry_the_jsonrpc_version_and_the_right_fields() {
    let id = RequestId::from(7_i64);
    assert_eq!(
        request(id.clone(), "initialize", json!({ "protocolVersion": 1 })),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 7,
            "method": "initialize",
            "params": { "protocolVersion": 1 },
        })
    );
    assert_eq!(
        notification("session/cancel", json!({ "sessionId": "s" })),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": "session/cancel",
            "params": { "sessionId": "s" },
        })
    );
    assert_eq!(
        success(id.clone(), json!({ "ok": true })),
        json!({ "jsonrpc": JSONRPC_VERSION, "id": 7, "result": { "ok": true } })
    );
    let error = RpcError::new(RpcError::INTERNAL, "boom");
    assert_eq!(
        failure(id, &error),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 7,
            "error": { "code": RpcError::INTERNAL, "message": "boom" },
        })
    );
}

#[tokio::test]
async fn a_written_message_is_exactly_one_line() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    write_message(&mut client, &notification("x", json!({ "text": "a\nb" })))
        .await
        .unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.ends_with('\n'));
    assert_eq!(
        line.matches('\n').count(),
        1,
        "no embedded newlines: {line}"
    );
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(parsed["params"]["text"], "a\nb", "newlines are escaped");
}

#[tokio::test]
async fn read_message_round_trips_every_envelope() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    let messages = [
        request(1_i64, "session/new", json!({ "cwd": "/tmp" })),
        notification("session/update", json!({ "sessionId": "s" })),
        success(RequestId::from("agent-1"), json!({ "sessionId": "s" })),
        failure(RequestId::from(2_i64), &RpcError::cancelled()),
    ];
    for message in &messages {
        write_message(&mut client, message).await.unwrap();
    }
    assert!(matches!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Request { id, method, params }
            if id == RequestId::from(1_i64) && method == "session/new" && params["cwd"] == "/tmp"
    ));
    assert_eq!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Notification {
            method: "session/update".to_string(),
            params: json!({ "sessionId": "s" }),
        }
    );
    assert_eq!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Response {
            id: Some(RequestId::from("agent-1")),
            result: Ok(json!({ "sessionId": "s" })),
        }
    );
    assert_eq!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Response {
            id: Some(RequestId::from(2_i64)),
            result: Err(RpcError::cancelled()),
        }
    );
}

#[tokio::test]
async fn blank_lines_and_crlf_are_tolerated() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    client
        .write_all(b"\n\r\n{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\r\n")
        .await
        .unwrap();
    assert_eq!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Notification {
            method: "ping".to_string(),
            params: serde_json::Value::Null,
        }
    );
}

#[tokio::test]
async fn a_last_line_without_a_newline_is_still_read() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    client
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}")
        .await
        .unwrap();
    client.shutdown().await.unwrap();
    assert!(matches!(
        read_message(&mut reader).await.unwrap(),
        Incoming::Notification { .. }
    ));
}

#[tokio::test]
async fn eof_is_reported_as_closed() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    client.shutdown().await.unwrap();
    assert!(matches!(
        read_message(&mut reader).await,
        Err(TransportError::Closed)
    ));
}

#[test]
fn responses_without_an_id_are_readable_but_unclaimed() {
    assert_eq!(
        parse_message(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}"#)
            .unwrap(),
        Incoming::Response {
            id: None,
            result: Err(RpcError::new(-32700, "parse")),
        }
    );
}

#[test]
fn numbers_and_strings_are_valid_ids() {
    for (raw, expected) in [
        (r#""id":12"#, RequestId::from(12_i64)),
        (r#""id":"abc""#, RequestId::from("abc")),
    ] {
        let message = format!(r#"{{"jsonrpc":"2.0",{raw},"result":{{}}}}"#);
        match parse_message(&message).unwrap() {
            Incoming::Response { id: Some(id), .. } => assert_eq!(id, expected),
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

#[test]
fn malformed_messages_are_rejected_with_a_reason() {
    let cases: [(&str, &str); 7] = [
        ("not json", "invalid JSON"),
        ("[]", "expected a JSON object"),
        (r#"{"id":1,"method":"x"}"#, "missing jsonrpc version"),
        (
            r#"{"jsonrpc":"1.0","method":"x"}"#,
            "unsupported jsonrpc version",
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":2}"#,
            "method must be a string",
        ),
        (r#"{"jsonrpc":"2.0","id":1}"#, "needs a result or an error"),
        (
            r#"{"jsonrpc":"2.0","id":true,"result":{}}"#,
            "id must be a number or a string",
        ),
    ];
    for (raw, expected) in cases {
        let error = parse_message(raw).unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains(expected),
            "{raw:?} should fail with {expected:?}, got {text:?}"
        );
    }
}

#[test]
fn an_error_object_must_have_a_code_and_a_message() {
    let error = parse_message(r#"{"jsonrpc":"2.0","id":1,"error":{"code":1}}"#).unwrap_err();
    assert!(matches!(error, TransportError::InvalidMessage(_)));
}

#[test]
fn ids_convert_and_display() {
    assert_eq!(RequestId::from(9_u64), RequestId::Number(9));
    assert_eq!(RequestId::from(9_i64).as_number(), Some(9));
    assert_eq!(RequestId::from("x").as_number(), None);
    assert_eq!(RequestId::from(u64::MAX).to_string(), i64::MAX.to_string());
    assert_eq!(RequestId::from(3_i64).to_string(), "3");
    assert_eq!(
        RpcError::method_not_found("fs/read").to_string(),
        "method not found: fs/read (code -32601)"
    );
    assert_eq!(RpcError::cancelled().code, RpcError::CANCELLED);
    assert_eq!(RpcError::AUTH_REQUIRED, -32000);
    assert_eq!(RpcError::NOT_FOUND, -32002);
    assert_eq!(RpcError::INVALID_PARAMS, -32602);
}

#[test]
fn transport_errors_are_readable() {
    assert_eq!(
        TransportError::TooLarge.to_string(),
        format!("the agent sent a message larger than {MAX_MESSAGE_BYTES} bytes")
    );
    assert_eq!(
        TransportError::Closed.to_string(),
        "the agent closed the connection"
    );
}

#[tokio::test]
async fn a_message_without_a_newline_is_cut_off_at_the_limit() {
    let (mut client, agent) = pipes();
    let mut reader = BufReader::new(agent);
    let writer = tokio::spawn(async move {
        let chunk = vec![b'x'; 64 * 1024];
        loop {
            if client.write_all(&chunk).await.is_err() {
                return;
            }
        }
    });
    let error = read_message(&mut reader).await.unwrap_err();
    writer.abort();
    assert!(matches!(error, TransportError::TooLarge), "got {error}");
}
