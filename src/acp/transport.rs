// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! JSON-RPC 2.0 framing for the ACP stdio transport.
//!
//! ACP carries one JSON-RPC message per line: newline delimited, UTF-8, and
//! never with an embedded newline (the spec forbids it, and compact
//! serialization guarantees it). The agent writes nothing else to its
//! standard output; its logging goes to standard error.
//!
//! This module knows nothing about the protocol semantics: it turns bytes into
//! [`Incoming`] messages and messages into bytes. That keeps it usable by the
//! client and by the simulated agent the tests speak to, so both sides of the
//! wire are exercised by the same code.

use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Value of the `jsonrpc` field every message must carry.
pub const JSONRPC_VERSION: &str = "2.0";

/// Largest message accepted from an agent, in bytes. A tool call may carry a
/// big diff or file listing, but an unbounded line would let a broken agent
/// exhaust memory, so the reader stops before that.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Identifier of a request. ACP uses numbers in the examples, but JSON-RPC
/// allows strings and an agent may prefer them, so both are accepted and
/// echoed back verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    Text(String),
}

impl RequestId {
    /// The numeric form, when this id is a number. PMP only sends numbers, so
    /// responses to its own requests always resolve.
    pub fn as_number(&self) -> Option<i64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Text(_) => None,
        }
    }
}

impl From<i64> for RequestId {
    fn from(id: i64) -> Self {
        Self::Number(id)
    }
}

impl From<u64> for RequestId {
    fn from(id: u64) -> Self {
        Self::Number(i64::try_from(id).unwrap_or(i64::MAX))
    }
}

impl From<&str> for RequestId {
    fn from(id: &str) -> Self {
        Self::Text(id.to_string())
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(formatter, "{number}"),
            Self::Text(text) => formatter.write_str(text),
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// The receiver does not implement the method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters are malformed or unsupported.
    pub const INVALID_PARAMS: i64 = -32602;
    /// The receiver failed unexpectedly.
    pub const INTERNAL: i64 = -32603;
    /// The request was cancelled.
    pub const CANCELLED: i64 = -32800;
    /// The request requires authentication first.
    pub const AUTH_REQUIRED: i64 = -32000;
    /// A referenced session or file does not exist.
    pub const NOT_FOUND: i64 = -32002;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            Self::METHOD_NOT_FOUND,
            format!("method not found: {method}"),
        )
    }

    pub fn cancelled() -> Self {
        Self::new(Self::CANCELLED, "cancelled")
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// One message read from the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// An answer to a request PMP sent. `id` is `None` when the agent answered
    /// with a null id (it could not parse the request), which no pending call
    /// can claim.
    Response {
        id: Option<RequestId>,
        result: Result<Value, RpcError>,
    },
    /// A request the agent sends to the client.
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    /// A one-way notification.
    Notification { method: String, params: Value },
}

/// Why framing failed. Every variant becomes an
/// [`AgentFailed`](crate::domain::error::AppError::AgentFailed) except
/// [`TransportError::Closed`], which can also mean an orderly shutdown.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("cannot read from the agent: {0}")]
    Read(#[source] io::Error),
    #[error("cannot write to the agent: {0}")]
    Write(#[source] io::Error),
    #[error("the agent closed the connection")]
    Closed,
    #[error("the agent sent a message larger than {MAX_MESSAGE_BYTES} bytes")]
    TooLarge,
    #[error("the agent sent invalid UTF-8")]
    InvalidUtf8,
    #[error("the agent sent invalid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
    #[error("the agent sent an invalid JSON-RPC message: {0}")]
    InvalidMessage(String),
}

/// Builds a request envelope.
pub fn request(id: impl Into<RequestId>, method: &str, params: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id.into(),
        "method": method,
        "params": params,
    })
}

/// Builds a notification envelope.
pub fn notification(method: &str, params: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params,
    })
}

/// Builds a successful response envelope.
pub fn success(id: RequestId, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    })
}

/// Builds an error response envelope.
pub fn failure(id: RequestId, error: &RpcError) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": error,
    })
}

/// Writes one message followed by a newline and flushes it.
pub async fn write_message<W>(writer: &mut W, message: &Value) -> Result<(), TransportError>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    // Compact serialization escapes newlines, so one message is always one
    // line and the framing cannot break.
    let mut line = serde_json::to_string(message)
        .map_err(|error| TransportError::Write(io::Error::other(error)))?;
    debug_assert!(!line.contains('\n'));
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(TransportError::Write)?;
    writer.flush().await.map_err(TransportError::Write)
}

/// Reads the next message. Blank lines are skipped; EOF without a message is
/// [`TransportError::Closed`].
pub async fn read_message<R>(reader: &mut R) -> Result<Incoming, TransportError>
where
    R: AsyncBufRead + Unpin + ?Sized,
{
    loop {
        let Some(line) = read_line(reader).await? else {
            return Err(TransportError::Closed);
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        return parse_message(line);
    }
}

/// Parses one line without reading it, so the envelope rules are testable on
/// their own.
pub fn parse_message(line: &str) -> Result<Incoming, TransportError> {
    let value: Value = serde_json::from_str(line).map_err(TransportError::InvalidJson)?;
    let object = value
        .as_object()
        .ok_or_else(|| TransportError::InvalidMessage("expected a JSON object".to_string()))?;
    match object.get("jsonrpc") {
        Some(Value::String(version)) if version == JSONRPC_VERSION => {}
        Some(other) => {
            return Err(TransportError::InvalidMessage(format!(
                "unsupported jsonrpc version {other}"
            )));
        }
        None => {
            return Err(TransportError::InvalidMessage(
                "missing jsonrpc version".to_string(),
            ));
        }
    }

    if let Some(method) = object.get("method") {
        let method = method
            .as_str()
            .ok_or_else(|| TransportError::InvalidMessage("method must be a string".to_string()))?;
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        return match object.get("id") {
            Some(id) => Ok(Incoming::Request {
                id: parse_id(id)?,
                method: method.to_string(),
                params,
            }),
            None => Ok(Incoming::Notification {
                method: method.to_string(),
                params,
            }),
        };
    }

    let id = match object.get("id") {
        None | Some(Value::Null) => None,
        Some(id) => Some(parse_id(id)?),
    };
    if let Some(error) = object.get("error") {
        let error: RpcError = serde_json::from_value(error.clone()).map_err(|error| {
            TransportError::InvalidMessage(format!("invalid error object: {error}"))
        })?;
        return Ok(Incoming::Response {
            id,
            result: Err(error),
        });
    }
    let result = object.get("result").cloned().ok_or_else(|| {
        TransportError::InvalidMessage("a response needs a result or an error".to_string())
    })?;
    Ok(Incoming::Response {
        id,
        result: Ok(result),
    })
}

fn parse_id(value: &Value) -> Result<RequestId, TransportError> {
    match value {
        Value::Number(number) => number.as_i64().map(RequestId::Number).ok_or_else(|| {
            TransportError::InvalidMessage(format!("id {number} is not an integer"))
        }),
        Value::String(text) => Ok(RequestId::Text(text.clone())),
        other => Err(TransportError::InvalidMessage(format!(
            "id must be a number or a string, got {other}"
        ))),
    }
}

/// Reads one line with a bounded buffer, so a message without a newline cannot
/// grow without limit. A final line without a trailing newline is accepted:
/// some agents close the pipe right after the last message.
async fn read_line<R>(reader: &mut R) -> Result<Option<String>, TransportError>
where
    R: AsyncBufRead + Unpin + ?Sized,
{
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(TransportError::Read)?;
        if available.is_empty() {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                decode(buffer).map(Some)
            };
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(position) => {
                buffer.extend_from_slice(&available[..position]);
                reader.consume(position + 1);
                if buffer.len() > MAX_MESSAGE_BYTES {
                    return Err(TransportError::TooLarge);
                }
                return decode(buffer).map(Some);
            }
            None => {
                let consumed = available.len();
                buffer.extend_from_slice(available);
                reader.consume(consumed);
                if buffer.len() > MAX_MESSAGE_BYTES {
                    return Err(TransportError::TooLarge);
                }
            }
        }
    }
}

fn decode(buffer: Vec<u8>) -> Result<String, TransportError> {
    let mut line = String::from_utf8(buffer).map_err(|_| TransportError::InvalidUtf8)?;
    // Tolerate CRLF: the JSON itself never needs the carriage return.
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}
