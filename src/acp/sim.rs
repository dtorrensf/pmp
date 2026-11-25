// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! A scripted ACP agent used by the tests.
//!
//! It speaks the real wire format through the same [`transport`](super::transport)
//! helpers as the client, over an in-memory duplex pipe pair. That exercises
//! framing, envelopes, correlation and update parsing without spawning a
//! process, and keeps the tests deterministic: the agent answers exactly when
//! the test tells it to, or not at all.
//!
//! What the simulated agent observed (permission answers, a cancel
//! notification) is kept in the returned [`SimAgent`], so tests can assert on
//! both sides of the dialogue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;

use super::client::AcpClient;
use super::protocol::{PROTOCOL_VERSION, SessionId};
use super::transport::{self, Incoming, RequestId, RpcError};

/// Size of each in-memory pipe. Big enough that a test update never blocks.
const PIPE: usize = 64 * 1024;

/// How the simulated agent behaves. The default is a compliant agent that
/// answers `initialize`, `session/new` and `session/prompt` immediately.
#[derive(Debug, Clone)]
pub struct SimOptions {
    /// Version reported in the `initialize` response.
    pub protocol_version: u32,
    /// Session id returned by `session/new`.
    pub session_id: String,
    /// `session/update` payloads sent, in order, before the prompt answer.
    pub updates: Vec<Value>,
    /// Stop reason of the prompt answer.
    pub stop_reason: String,
    /// A request the agent sends to the client during the prompt turn.
    pub ask: Option<AgentAsk>,
    /// Never answer the prompt until the client cancels it.
    pub wait_for_cancel: bool,
    /// Answer `initialize` with this error instead of a result.
    pub fail_initialize: Option<String>,
    /// Write this raw text instead of a JSON answer to `initialize`.
    pub garbage: Option<String>,
    /// Close the connection when `initialize` arrives (an agent that crashes).
    pub die_on_initialize: bool,
    /// Never answer anything (an agent that hangs).
    pub hang: bool,
}

impl Default for SimOptions {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id: "sess-sim".to_string(),
            updates: Vec::new(),
            stop_reason: "end_turn".to_string(),
            ask: None,
            wait_for_cancel: false,
            fail_initialize: None,
            garbage: None,
            die_on_initialize: false,
            hang: false,
        }
    }
}

/// A request the simulated agent sends to the client.
#[derive(Debug, Clone)]
pub struct AgentAsk {
    pub method: String,
    pub params: Value,
}

impl AgentAsk {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            method: method.to_string(),
            params,
        }
    }

    /// The permission request an agent sends before a risky tool call.
    pub fn permission(session_id: &SessionId) -> Self {
        Self::new(
            "session/request_permission",
            json!({
                "sessionId": session_id,
                "toolCall": {
                    "toolCallId": "call-1",
                    "title": "Run the test suite",
                    "kind": "execute",
                    "status": "pending",
                },
                "options": [{ "optionId": "allow", "name": "Allow once", "kind": "allow_once" }],
            }),
        )
    }
}

/// What the client answered to an [`AgentAsk`].
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub method: String,
    pub result: Result<Value, RpcError>,
}

/// Handle to a running simulated agent.
pub struct SimAgent {
    handle: Option<JoinHandle<()>>,
    answers: Arc<Mutex<Vec<Answer>>>,
    cancelled: Arc<AtomicBool>,
    session_params: Arc<Mutex<Option<Value>>>,
    prompt_params: Arc<Mutex<Option<Value>>>,
}

impl SimAgent {
    /// Every answer the client gave to an [`AgentAsk`], in order.
    pub fn answers(&self) -> Vec<Answer> {
        self.answers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Whether the client sent `session/cancel`.
    pub fn was_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Parameters of the last `session/new`, for asserting what the client
    /// asked the agent to work on.
    pub fn session_params(&self) -> Option<Value> {
        self.session_params
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Parameters of the last `session/prompt`, including its content blocks.
    pub fn prompt_params(&self) -> Option<Value> {
        self.prompt_params
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Waits for the agent loop to end, which happens when the client closes
    /// its side of the pipe. Taking `&mut self` lets the test keep inspecting
    /// what the agent observed.
    pub async fn finished(&mut self) -> bool {
        let Some(handle) = self.handle.take() else {
            return true;
        };
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .is_ok()
    }
}

/// Starts a simulated agent and a client connected to it.
pub async fn spawn(options: SimOptions) -> (AcpClient, SimAgent) {
    let (client_write, agent_read) = tokio::io::duplex(PIPE);
    let (agent_write, client_read) = tokio::io::duplex(PIPE);
    let client = AcpClient::from_io(client_read, client_write).await;
    (client, SimAgent::start(agent_read, agent_write, options))
}

impl SimAgent {
    fn start<R, W>(reader: R, writer: W, options: SimOptions) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let answers = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let session_params = Arc::new(Mutex::new(None));
        let prompt_params = Arc::new(Mutex::new(None));
        let handle = tokio::spawn(run(
            BufReader::new(reader),
            writer,
            options,
            Arc::clone(&answers),
            Arc::clone(&cancelled),
            Arc::clone(&session_params),
            Arc::clone(&prompt_params),
        ));
        Self {
            handle: Some(handle),
            answers,
            cancelled,
            session_params,
            prompt_params,
        }
    }
}

async fn run<R, W>(
    mut reader: R,
    mut writer: W,
    options: SimOptions,
    answers: Arc<Mutex<Vec<Answer>>>,
    cancelled: Arc<AtomicBool>,
    session_params: Arc<Mutex<Option<Value>>>,
    prompt_params: Arc<Mutex<Option<Value>>>,
) where
    R: AsyncBufRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    let session_id = SessionId::new(options.session_id.clone());
    loop {
        let message = match transport::read_message(&mut reader).await {
            Ok(message) => message,
            // The client closed the pipe: an orderly end for the agent too.
            Err(_) => return,
        };
        let (id, method, params) = match message {
            Incoming::Request { id, method, params } => (id, method, params),
            Incoming::Notification { method, .. } => {
                if method == "session/cancel" {
                    cancelled.store(true, Ordering::SeqCst);
                }
                continue;
            }
            Incoming::Response { .. } => continue,
        };

        match method.as_str() {
            "initialize" => {
                if options.hang {
                    std::future::pending::<()>().await;
                }
                if options.die_on_initialize {
                    return;
                }
                if let Some(garbage) = &options.garbage {
                    write_raw(&mut writer, garbage).await;
                    continue;
                }
                if let Some(message) = &options.fail_initialize {
                    reply_error(&mut writer, id, transport::RpcError::INTERNAL, message).await;
                    continue;
                }
                reply(
                    &mut writer,
                    id,
                    json!({
                        "protocolVersion": options.protocol_version,
                        "agentCapabilities": {
                            "loadSession": true,
                            "promptCapabilities": { "image": true, "audio": false, "embeddedContext": true },
                            "mcpCapabilities": { "http": false, "sse": false },
                        },
                        "agentInfo": { "name": "simulated", "title": "Simulated Agent", "version": "0.0.1" },
                        "authMethods": [],
                    }),
                )
                .await;
            }
            "session/new" => {
                store(&session_params, &params);
                reply(&mut writer, id, json!({ "sessionId": session_id })).await;
            }
            "session/prompt" => {
                store(&prompt_params, &params);
                if options.hang {
                    std::future::pending::<()>().await;
                }
                for update in &options.updates {
                    notify(
                        &mut writer,
                        "session/update",
                        json!({ "sessionId": session_id, "update": update }),
                    )
                    .await;
                }
                if let Some(ask) = &options.ask
                    && !ask_client(&mut reader, &mut writer, ask, &answers).await
                {
                    return;
                }
                if options.wait_for_cancel {
                    if !wait_for_cancel(&mut reader, &cancelled).await {
                        return;
                    }
                    reply(&mut writer, id, json!({ "stopReason": "cancelled" })).await;
                } else {
                    reply(
                        &mut writer,
                        id,
                        json!({ "stopReason": options.stop_reason }),
                    )
                    .await;
                }
            }
            other => {
                reply_error(
                    &mut writer,
                    id,
                    transport::RpcError::METHOD_NOT_FOUND,
                    &format!("method not found: {other}"),
                )
                .await;
            }
        }
    }
}

/// Sends an agent request and records the client's answer. Returns `false`
/// when the connection ended.
async fn ask_client<R, W>(
    reader: &mut R,
    writer: &mut W,
    ask: &AgentAsk,
    answers: &Arc<Mutex<Vec<Answer>>>,
) -> bool
where
    R: AsyncBufRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    let id = RequestId::Text("sim-ask".to_string());
    let request = transport::request(id.clone(), &ask.method, ask.params.clone());
    if transport::write_message(writer, &request).await.is_err() {
        return false;
    }
    loop {
        match transport::read_message(reader).await {
            Ok(Incoming::Response {
                id: Some(response_id),
                result,
            }) if response_id == id => {
                answers
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push(Answer {
                        method: ask.method.clone(),
                        result,
                    });
                return true;
            }
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
}

/// Reads until `session/cancel` arrives. Returns `false` on a broken pipe.
async fn wait_for_cancel<R>(reader: &mut R, cancelled: &Arc<AtomicBool>) -> bool
where
    R: AsyncBufRead + Send + Unpin,
{
    loop {
        match transport::read_message(reader).await {
            Ok(Incoming::Notification { method, .. }) if method == "session/cancel" => {
                cancelled.store(true, Ordering::SeqCst);
                return true;
            }
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
}

fn store(slot: &Arc<Mutex<Option<Value>>>, params: &Value) {
    *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(params.clone());
}

async fn reply<W>(writer: &mut W, id: RequestId, result: Value)
where
    W: AsyncWrite + Send + Unpin,
{
    let message = transport::success(id, result);
    let _ = transport::write_message(writer, &message).await;
}

async fn reply_error<W>(writer: &mut W, id: RequestId, code: i64, message: &str)
where
    W: AsyncWrite + Send + Unpin,
{
    let error = transport::RpcError::new(code, message.to_string());
    let message = transport::failure(id, &error);
    let _ = transport::write_message(writer, &message).await;
}

async fn notify<W>(writer: &mut W, method: &str, params: Value)
where
    W: AsyncWrite + Send + Unpin,
{
    let message = transport::notification(method, params);
    let _ = transport::write_message(writer, &message).await;
}

async fn write_raw<W>(writer: &mut W, text: &str)
where
    W: AsyncWrite + Send + Unpin,
{
    let _ = writer.write_all(format!("{text}\n").as_bytes()).await;
    let _ = writer.flush().await;
}
