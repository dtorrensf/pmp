// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The ACP client: PMP launches a local agent and talks to it over stdio.
//!
//! # Shape
//!
//! [`AcpClient`] owns one agent connection. [`AcpClient::launch`] spawns the
//! binary of an [`AgentProfile`](crate::config::AgentProfile), wires its pipes
//! and starts a reader task that:
//!
//! * resolves the answers to the requests PMP sent,
//! * republishes `session/update` notifications to every subscriber, and
//! * answers the requests an agent makes of the client.
//!
//! Writes are serialized behind one mutex, so a `session/cancel` can be sent
//! while a `session/prompt` is still waiting for its response: cancelling is
//! the whole point of being able to write concurrently.
//!
//! # Lifecycle
//!
//! ```no_run
//! # async fn run(profile: &pmp::config::AgentProfile, cwd: &std::path::Path) -> Result<(), pmp::domain::error::AppError> {
//! use pmp::acp::AcpClient;
//!
//! let client = AcpClient::launch(profile).await?;
//! let mut events = client.subscribe();
//! let agent = client.initialize().await?;
//! let session = client.new_session(cwd).await?;
//! let stop = client.prompt(&session, "implement the task").await?;
//! client.shutdown().await;
//! # let _ = (agent, stop, events.recv().await);
//! # Ok(())
//! # }
//! ```
//!
//! [`AcpClient::shutdown`] closes the agent's stdin, waits for it to exit and
//! kills it if it does not. Dropping the last clone is the safety net: the
//! child is spawned with `kill_on_drop`, so an interrupted process does not
//! leave an orphan behind.
//!
//! # Failures
//!
//! Every failure keeps the A5 taxonomy: a binary that cannot be started is
//! [`AppError::AgentUnavailable`] (retryable: install it and retry), a broken
//! protocol, a timeout or an error answer is [`AppError::AgentFailed`], and an
//! invalid working directory is [`AppError::Validation`]. User-facing messages
//! never include stack traces or local storage details.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::time::timeout;

use crate::config::AgentProfile;
use crate::domain::error::AppError;

use super::protocol::{
    CancelNotification, ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PROTOCOL_VERSION, PromptRequest, PromptResponse, RequestPermissionResponse,
    SessionId, SessionUpdateNotification, StopReason,
};
use super::transport::{self, Incoming, RequestId, RpcError};

/// Deadline for one handshake or session request. An agent that cannot answer
/// `initialize` or `session/new` in this time is not usable.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for one prompt turn. A turn runs an agent against a real
/// repository, so it is measured in minutes; the user can still cancel it.
pub const DEFAULT_PROMPT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long an agent has to exit after its stdin is closed before it is killed.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// How many events one subscriber may lag behind. Progress updates are
/// disposable: a slow consumer loses the oldest ones instead of blocking the
/// agent.
const EVENT_BUFFER: usize = 1024;

/// What happens on the agent connection, as seen by the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum AcpEvent {
    /// A `session/update` notification, already parsed.
    Update(SessionUpdateNotification),
    /// A request from the agent this client could not serve. Today that means
    /// `session/request_permission` (answered `cancelled`) and any method the
    /// client did not advertise (`fs/*`, `terminal/*`, ...), answered with
    /// `method not found`.
    Unserved { method: String, id: String },
    /// The connection ended: the agent exited, closed its output, or the
    /// stream broke. Sent once.
    Closed { reason: String },
}

/// One connection to one agent process.
///
/// Cloning is cheap and shares the connection; the process is killed when the
/// last clone is dropped.
#[derive(Clone)]
pub struct AcpClient {
    inner: Arc<Inner>,
    /// The child, kept apart from [`Inner`] so that the reader task — which
    /// holds `inner` — cannot keep the process alive by accident.
    process: Arc<Mutex<Option<Child>>>,
}

impl std::fmt::Debug for AcpClient {
    /// The pipes and the pending channel cannot be printed; what matters in a
    /// failure message is whether the connection is still open.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcpClient")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

struct Inner {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    /// Requests waiting for an answer, keyed by the id PMP sent.
    pending: StdMutex<HashMap<i64, oneshot::Sender<Result<Value, RpcError>>>>,
    next_id: AtomicU64,
    events: broadcast::Sender<AcpEvent>,
    request_timeout_ms: AtomicU64,
    prompt_timeout_ms: AtomicU64,
    closed: AtomicBool,
    /// Why the connection ended, once it did. A subscriber that arrives late
    /// (an agent that died before the UI started listening) can still tell
    /// what happened.
    closed_reason: StdMutex<Option<String>>,
}

impl AcpClient {
    /// Starts the agent described by `profile` in the current directory.
    pub async fn launch(profile: &AgentProfile) -> Result<Self, AppError> {
        Self::spawn(profile, None).await
    }

    /// Starts the agent with `cwd` as its working directory. The session has
    /// its own `cwd` too (see [`AcpClient::new_session`]); launching in the
    /// workspace keeps relative paths in the agent's own tooling sane.
    pub async fn launch_in(
        profile: &AgentProfile,
        cwd: impl AsRef<Path>,
    ) -> Result<Self, AppError> {
        Self::spawn(profile, Some(cwd.as_ref())).await
    }

    async fn spawn(profile: &AgentProfile, cwd: Option<&Path>) -> Result<Self, AppError> {
        let mut command = Command::new(&profile.command);
        command
            .args(&profile.args)
            .envs(&profile.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            let cwd = absolute_path(cwd)?;
            ensure_dir(&cwd)?;
            command.current_dir(cwd);
        }
        tracing::debug!(command = %profile.command, args = ?profile.args, "starting an ACP agent");
        let mut child = command
            .spawn()
            .map_err(|error| unavailable(profile, &error))?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AppError::AgentUnavailable(format!(
                "cannot open a stdin pipe for '{}'",
                profile.command
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AppError::AgentUnavailable(format!(
                "cannot open a stdout pipe for '{}'",
                profile.command
            ))
        })?;
        if let Some(stderr) = child.stderr.take() {
            drain_stderr(stderr, profile.command.clone());
        }
        let client = Self::from_streams(stdout, stdin);
        *client.process.lock().await = Some(child);
        Ok(client)
    }

    /// Connects over an existing pipe pair: `reader` carries the agent's
    /// output and `writer` its input. Used by the simulated agent in tests and
    /// by any future transport that is not a child process.
    ///
    /// Must be called from a Tokio runtime: the reader task is spawned here.
    pub async fn from_io<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self::from_streams(reader, writer)
    }

    fn from_streams<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let inner = Arc::new(Inner {
            writer: Mutex::new(Box::new(writer)),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            events,
            request_timeout_ms: AtomicU64::new(millis(DEFAULT_REQUEST_TIMEOUT)),
            prompt_timeout_ms: AtomicU64::new(millis(DEFAULT_PROMPT_TIMEOUT)),
            closed: AtomicBool::new(false),
            closed_reason: StdMutex::new(None),
        });
        let reader: Box<dyn AsyncBufRead + Send + Unpin> = Box::new(BufReader::new(reader));
        tokio::spawn(drive(Arc::clone(&inner), reader));
        Self {
            inner,
            process: Arc::new(Mutex::new(None)),
        }
    }

    /// Uses `timeout` for every handshake and session request. Zero is ignored
    /// with a warning: it would fail every call immediately.
    pub fn with_request_timeout(self, timeout: Duration) -> Self {
        set_timeout(&self.inner.request_timeout_ms, timeout, "request");
        self
    }

    /// Uses `timeout` for one prompt turn.
    pub fn with_prompt_timeout(self, timeout: Duration) -> Self {
        set_timeout(&self.inner.prompt_timeout_ms, timeout, "prompt");
        self
    }

    /// Subscribes to the connection events. A subscriber that falls behind
    /// loses the oldest updates; see [`broadcast::error::RecvError::Lagged`].
    pub fn subscribe(&self) -> broadcast::Receiver<AcpEvent> {
        self.inner.events.subscribe()
    }

    /// Whether the connection is closed: the agent exited, the stream broke or
    /// [`AcpClient::shutdown`] ran.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Why the connection ended, if it did. Lets a late subscriber report a
    /// session that was already gone when it started listening.
    pub fn closed_reason(&self) -> Option<String> {
        lock(&self.inner.closed_reason).clone()
    }

    /// Negotiates the protocol version and capabilities.
    ///
    /// An agent that answers with a different major version is refused: the
    /// wire format is not compatible and continuing would fail in worse ways.
    pub async fn initialize(&self) -> Result<InitializeResponse, AppError> {
        let params = to_params("initialize", &InitializeRequest::default())?;
        let result = self.request("initialize", params).await?;
        let response: InitializeResponse =
            serde_json::from_value(result).map_err(|error| protocol_error("initialize", &error))?;
        if response.protocol_version != PROTOCOL_VERSION {
            return Err(AppError::AgentFailed(format!(
                "the agent speaks ACP v{} and pmp supports v{PROTOCOL_VERSION}",
                response.protocol_version
            )));
        }
        tracing::debug!(
            agent = %response.agent_label(),
            load_session = response.agent_capabilities.load_session,
            auth_methods = response.auth_methods.len(),
            "ACP connection initialized"
        );
        Ok(response)
    }

    /// Creates a session in `cwd`, which must be an existing absolute
    /// directory.
    pub async fn new_session(&self, cwd: &Path) -> Result<SessionId, AppError> {
        self.new_session_with(NewSessionRequest {
            cwd: absolute_dir(cwd)?,
            mcp_servers: Vec::new(),
        })
        .await
    }

    /// Creates a session with a caller-built request: MCP servers (C5) and
    /// additional workspace roots will travel here.
    pub async fn new_session_with(
        &self,
        request: NewSessionRequest,
    ) -> Result<SessionId, AppError> {
        let params = to_params("session/new", &request)?;
        let result = self.request("session/new", params).await?;
        let response: NewSessionResponse = serde_json::from_value(result)
            .map_err(|error| protocol_error("session/new", &error))?;
        if response.session_id.is_empty() {
            return Err(AppError::AgentFailed(
                "the agent returned an empty session id".to_string(),
            ));
        }
        Ok(response.session_id)
    }

    /// Sends `text` as the turn's prompt and waits for the stop reason.
    /// Progress arrives on the events stream while this future is pending.
    pub async fn prompt(
        &self,
        session_id: &SessionId,
        text: impl Into<String>,
    ) -> Result<StopReason, AppError> {
        self.prompt_blocks(session_id, vec![ContentBlock::text(text)])
            .await
    }

    /// Sends a prompt made of several content blocks.
    pub async fn prompt_blocks(
        &self,
        session_id: &SessionId,
        prompt: Vec<ContentBlock>,
    ) -> Result<StopReason, AppError> {
        if prompt.is_empty() {
            return Err(AppError::Validation(
                "a prompt needs at least one content block".to_string(),
            ));
        }
        let request = PromptRequest {
            session_id: session_id.clone(),
            prompt,
        };
        let params = to_params("session/prompt", &request)?;
        let result = self.request("session/prompt", params).await?;
        let response: PromptResponse = serde_json::from_value(result)
            .map_err(|error| protocol_error("session/prompt", &error))?;
        Ok(response.stop_reason)
    }

    /// Asks the agent to stop the running turn. The `session/prompt` future
    /// then resolves with the `cancelled` stop reason.
    pub async fn cancel(&self, session_id: &SessionId) -> Result<(), AppError> {
        let params = to_params(
            "session/cancel",
            &CancelNotification {
                session_id: session_id.clone(),
            },
        )?;
        self.inner
            .write(transport::notification("session/cancel", params))
            .await
    }

    /// Closes the connection in an orderly way: reject further writes, close
    /// the agent's stdin, and wait for it to exit (killing it after
    /// [`SHUTDOWN_GRACE`]). Callers still awaiting a request get an error.
    pub async fn shutdown(&self) {
        self.inner.mark_closed("pmp closed the connection");
        self.inner.fail_all("pmp closed the connection");
        self.inner.close_writer().await;
        let mut slot = self.process.lock().await;
        let Some(mut child) = slot.take() else {
            return;
        };
        match timeout(SHUTDOWN_GRACE, child.wait()).await {
            Ok(Ok(status)) => tracing::debug!(?status, "the agent process exited"),
            Ok(Err(error)) => tracing::warn!(%error, "cannot wait for the agent process"),
            Err(_) => {
                tracing::warn!("the agent did not exit in time; killing it");
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }

    /// Sends one request and waits for its answer, with a method-dependent
    /// deadline.
    async fn request(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let key = i64::try_from(id).unwrap_or(i64::MAX);
        let (sender, receiver) = oneshot::channel();
        lock(&self.inner.pending).insert(key, sender);
        let message = transport::request(id, method, params);
        if let Err(error) = self.inner.write(message).await {
            lock(&self.inner.pending).remove(&key);
            return Err(error);
        }
        let deadline = self.inner.timeout_for(method);
        match timeout(deadline, receiver).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => {
                tracing::warn!(method, code = error.code, %error, "the agent rejected a request");
                Err(AppError::AgentFailed(format!("{method} failed: {error}")))
            }
            Ok(Err(_)) => Err(AppError::AgentFailed(format!(
                "the agent connection closed during {method}"
            ))),
            Err(_) => {
                lock(&self.inner.pending).remove(&key);
                self.inner.cancel_request(key).await;
                Err(AppError::AgentFailed(format!(
                    "{method} timed out after {}",
                    describe(deadline)
                )))
            }
        }
    }
}

impl Inner {
    /// Writes one message, serialized with every other writer of the
    /// connection.
    async fn write(&self, message: Value) -> Result<(), AppError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(AppError::AgentFailed(
                "the agent connection is closed".to_string(),
            ));
        }
        let mut writer = self.writer.lock().await;
        transport::write_message(&mut *writer, &message)
            .await
            .map_err(|error| AppError::AgentFailed(error.to_string()))
    }

    /// Sends `$/cancel_request` for a request the caller stopped waiting for.
    /// Cancellation is advisory; the agent may still answer.
    async fn cancel_request(&self, id: i64) {
        let message =
            transport::notification("$/cancel_request", serde_json::json!({ "requestId": id }));
        if let Err(error) = self.write(message).await {
            tracing::debug!(%error, "cannot cancel the abandoned request");
        }
    }

    /// Drops the write end of the pipe: the agent reads EOF and exits.
    async fn close_writer(&self) {
        let mut writer = self.writer.lock().await;
        let _ = writer.flush().await;
        *writer = Box::new(tokio::io::sink());
    }

    /// Marks the connection closed and announces it once.
    fn mark_closed(&self, reason: &str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        *lock(&self.closed_reason) = Some(reason.to_string());
        tracing::debug!(reason, "ACP connection closed");
        let _ = self.events.send(AcpEvent::Closed {
            reason: reason.to_string(),
        });
    }

    /// Fails every waiting request, so no caller hangs on a dead connection.
    fn fail_all(&self, reason: &str) {
        let pending: Vec<_> = lock(&self.pending).drain().collect();
        for (_, sender) in pending {
            let error = RpcError::new(
                RpcError::INTERNAL,
                format!("the agent connection closed: {reason}"),
            );
            let _ = sender.send(Err(error));
        }
    }

    /// Hands a response to the caller waiting for it.
    fn resolve(&self, id: RequestId, result: Result<Value, RpcError>) {
        let Some(key) = id.as_number() else {
            tracing::warn!(id = %id, "the agent answered with an unknown string id");
            return;
        };
        let Some(sender) = lock(&self.pending).remove(&key) else {
            // A timed-out request leaves nothing behind; the late answer is
            // expected and harmless.
            tracing::debug!(id = %id, "ignoring a response nobody waits for");
            return;
        };
        let _ = sender.send(result);
    }

    fn publish(&self, event: AcpEvent) {
        if self.events.send(event).is_err() {
            tracing::trace!("no subscriber for the ACP event");
        }
    }

    fn timeout_for(&self, method: &str) -> Duration {
        let millis = if method == "session/prompt" {
            self.prompt_timeout_ms.load(Ordering::SeqCst)
        } else {
            self.request_timeout_ms.load(Ordering::SeqCst)
        };
        Duration::from_millis(millis)
    }
}

/// Reads the connection until it ends, resolving answers and republishing
/// updates.
async fn drive(inner: Arc<Inner>, mut reader: Box<dyn AsyncBufRead + Send + Unpin>) {
    loop {
        let incoming = match transport::read_message(&mut reader).await {
            Ok(message) => message,
            Err(error) => {
                let reason = error.to_string();
                inner.mark_closed(&reason);
                inner.fail_all(&reason);
                return;
            }
        };
        match incoming {
            Incoming::Response { id, result } => match id {
                Some(id) => inner.resolve(id, result),
                None => tracing::warn!("the agent sent a response without an id"),
            },
            Incoming::Notification { method, params } => dispatch(&inner, &method, params),
            Incoming::Request { id, method, params } => {
                serve(&inner, id, &method, params).await;
            }
        }
    }
}

/// Forwards a `session/update` notification. Unknown notifications and
/// malformed payloads are logged and dropped: one bad update must not take the
/// session down.
fn dispatch(inner: &Inner, method: &str, params: Value) {
    if method != "session/update" {
        tracing::debug!(method, "ignoring an ACP notification");
        return;
    }
    match serde_json::from_value::<SessionUpdateNotification>(params) {
        Ok(notification) => inner.publish(AcpEvent::Update(notification)),
        Err(error) => {
            tracing::warn!(%error, "ignoring a malformed session/update notification");
        }
    }
}

/// Answers a request from the agent.
///
/// Permission dialogs arrive in C4; until then the safe, spec-compliant answer
/// is `cancelled`, which never leaves the agent waiting. Every other method is
/// one the client did not advertise, so `method not found` tells the agent to
/// stop asking. Both are reported to the UI as [`AcpEvent::Unserved`].
async fn serve(inner: &Inner, id: RequestId, method: &str, _params: Value) {
    tracing::debug!(method, id = %id, "the agent requested a client method");
    inner.publish(AcpEvent::Unserved {
        method: method.to_string(),
        id: id.to_string(),
    });
    let reply = if method == "session/request_permission" {
        let result =
            serde_json::to_value(RequestPermissionResponse::cancelled()).unwrap_or(Value::Null);
        transport::success(id, result)
    } else {
        transport::failure(id, &RpcError::method_not_found(method))
    };
    if let Err(error) = inner.write(reply).await {
        tracing::warn!(%error, method, "cannot answer the agent request");
    }
}

/// Drains the agent's stderr into the log. It is the agent's own logging
/// channel and never protocol data.
fn drain_stderr(stderr: tokio::process::ChildStderr, command: String) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = line.trim_end();
                    if !text.is_empty() {
                        tracing::debug!(agent = %command, "agent stderr: {text}");
                    }
                }
            }
        }
    });
}

/// Maps a spawn failure to the A5 taxonomy.
fn unavailable(profile: &AgentProfile, error: &std::io::Error) -> AppError {
    tracing::warn!(command = %profile.command, %error, "cannot start the agent");
    if error.kind() == std::io::ErrorKind::NotFound {
        AppError::AgentUnavailable(format!(
            "'{}' was not found in PATH; install it or fix the profile in config.toml",
            profile.command
        ))
    } else {
        AppError::AgentUnavailable(format!("cannot start '{}': {error}", profile.command))
    }
}

/// Serializes request parameters, which only fails on a programming error.
fn to_params<T: Serialize>(method: &str, value: &T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|error| {
        tracing::error!(%error, method, "cannot serialize the ACP request");
        AppError::Internal
    })
}

fn protocol_error(method: &str, error: &serde_json::Error) -> AppError {
    tracing::warn!(%error, method, "the agent returned a malformed response");
    AppError::AgentFailed(format!(
        "the agent returned an invalid {method} response: {error}"
    ))
}

/// Resolves `cwd` to an absolute string and checks it is an existing
/// directory. ACP requires absolute paths, and an agent that gets a missing
/// one fails much later and much less clearly.
fn absolute_dir(cwd: &Path) -> Result<String, AppError> {
    let absolute = absolute_path(cwd)?;
    ensure_dir(&absolute)?;
    Ok(absolute.to_string_lossy().into_owned())
}

/// Makes `cwd` absolute without requiring it to exist, so the error can still
/// name the path the user configured.
fn absolute_path(cwd: &Path) -> Result<PathBuf, AppError> {
    std::path::absolute(cwd).map_err(|error| {
        AppError::Validation(format!(
            "working directory '{}' cannot be resolved: {error}",
            cwd.display()
        ))
    })
}

fn ensure_dir(path: &Path) -> Result<(), AppError> {
    if path.is_dir() {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "working directory '{}' does not exist or is not a directory",
        path.display()
    )))
}

/// Human-readable deadline: seconds when there are any, milliseconds when the
/// caller configured a sub-second deadline (tests and embedded transports).
fn describe(deadline: Duration) -> String {
    if deadline.as_secs() > 0 {
        format!("{}s", deadline.as_secs())
    } else {
        format!("{}ms", deadline.as_millis())
    }
}

fn set_timeout(field: &AtomicU64, value: Duration, name: &str) {
    if value.is_zero() {
        tracing::warn!(name, "ignoring a zero ACP timeout");
        return;
    }
    field.store(millis(value), Ordering::SeqCst);
}

fn millis(value: Duration) -> u64 {
    u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
}

/// Locks a plain mutex, recovering from a poisoned lock: the guarded data is a
/// map of channels, which stays consistent even if a holder panicked.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
