// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! ACP (Agent Client Protocol) client.
//!
//! PMP drives local coding agents: it launches them as subprocesses, hands
//! them a task and follows the turn while they work, instead of depending on a
//! particular vendor. The protocol is JSON-RPC 2.0 over the child's stdio, one
//! message per line, as specified in the ACP v1 schema.
//!
//! The module is split so each piece can be tested on its own:
//!
//! * [`protocol`] — the wire types PMP reads and writes.
//! * [`transport`] — line framing and JSON-RPC envelopes.
//! * [`client`] — process launch, request/response correlation, event stream
//!   and orderly shutdown.
//! * [`profiles`] — the `[agents.<name>]` entries of `config.toml`.
//!
//! This is infrastructure: it holds no domain rules. The use case that sends a
//! PMP task to an agent lives in the application layer, so the TUI and a
//! future headless CLI share it (C3 and C8).

pub mod client;
pub mod permissions;
pub mod profiles;
pub mod protocol;
pub mod tasks;
pub mod transport;

pub use client::{
    AcpClient, AcpEvent, DEFAULT_PROMPT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, PermissionHandler,
    SHUTDOWN_GRACE,
};
pub use permissions::{Allowlist, Decision, PermissionRequest, Rule};
pub use protocol::{
    AgentCapabilities, AuthMethod, CancelNotification, ContentBlock, Implementation,
    InitializeResponse, McpCapabilities, McpServer, NewSessionRequest, PROTOCOL_VERSION, PlanEntry,
    PromptCapabilities, PromptRequest, PromptResponse, SessionId, SessionUpdate,
    SessionUpdateNotification, StopReason, ToolCallStatus, ToolKind,
};
pub use tasks::send_implement_task;
pub use transport::{Incoming, RequestId, RpcError, TransportError};

#[cfg(test)]
pub(crate) mod sim;
#[cfg(test)]
mod tests;
