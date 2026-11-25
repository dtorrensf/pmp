// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wire types for ACP v1, the part PMP needs to drive an agent.
//!
//! The protocol is a JSON-RPC 2.0 dialogue where every object property is
//! `camelCase` and every string carried by a discriminator is `snake_case`
//! (see `https://agentclientprotocol.com/protocol/v1/schema`). Only the
//! objects PMP reads or writes are modelled; unknown fields are ignored and
//! unknown open values (stop reasons, tool kinds, update variants) degrade to
//! an opaque value or [`SessionUpdate::Unknown`] instead of failing the whole
//! message. Agents evolve faster than this client, and dropping one unknown
//! update is always better than killing a running session.
//!
//! Requests are typed too, so the contract pmp sends is documented and
//! testable instead of living in `json!` literals scattered around.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The ACP major version PMP implements. Sent in `initialize`; the agent
/// answers with the version it will speak, which must be the same one.
pub const PROTOCOL_VERSION: u32 = 1;

/// Client name reported in `initialize`.
pub const CLIENT_NAME: &str = "pmp";

/// Human-readable client name reported in `initialize`.
pub const CLIENT_TITLE: &str = "Project Manager Planner";

/// Client version reported in `initialize`.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name and version of one side of the connection (`clientInfo`,
/// `agentInfo`). `title` is optional in the protocol but PMP always sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub version: String,
}

impl Default for Implementation {
    fn default() -> Self {
        Self {
            name: CLIENT_NAME.to_string(),
            title: Some(CLIENT_TITLE.to_string()),
            version: CLIENT_VERSION.to_string(),
        }
    }
}

/// Capabilities PMP advertises in `initialize`.
///
/// Empty on purpose: ACP treats every omitted capability as unsupported, and
/// the TUI does not serve `fs/*`, `terminal/*`, elicitation or plan operations
/// yet. Advertising nothing keeps a well-behaved agent from asking for them;
/// an agent that asks anyway is answered with a clear rejection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {}

/// `initialize` request parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

impl Default for InitializeRequest {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: ClientCapabilities::default(),
            client_info: Implementation::default(),
        }
    }
}

/// Capabilities the agent reports in its `initialize` response. Unknown ones
/// are ignored; the defaults match the protocol's own defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCapabilities {
    /// The agent can replay a previous session with `session/load`.
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    pub mcp_capabilities: McpCapabilities,
}

/// Content types the agent accepts inside a prompt.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

/// MCP transports the agent can connect to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpCapabilities {
    pub http: bool,
    pub sse: bool,
}

/// One authentication method the agent offers. PMP does not implement any
/// authentication flow yet, but the list is parsed so a required login can be
/// reported to the user instead of surfacing as an opaque failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `agent`, `env_var` or `terminal`; kept as text to stay forward
    /// compatible with new methods.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `initialize` response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    #[serde(default)]
    pub agent_info: Option<Implementation>,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
}

impl InitializeResponse {
    /// Human-readable agent identity for logs and notes.
    pub fn agent_label(&self) -> String {
        match &self.agent_info {
            Some(info) => format!("{} {}", info.name, info.version),
            None => "unknown agent".to_string(),
        }
    }
}

/// Stable identifier of one session, as returned by `session/new`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for SessionId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One environment variable of an MCP server launched by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

/// An MCP server the agent should connect to for this session.
///
/// Only the stdio transport is modelled: it is the one every agent must
/// support, and the one PMP uses to inject its own `pmp mcp` server. HTTP and
/// SSE servers carry a `type` discriminator and will be added when needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVariable>,
}

/// `session/new` request parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionRequest {
    /// Absolute path of the workspace the agent works in.
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServer>,
}

/// `session/new` response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: SessionId,
}

/// One block of a prompt or of a streamed message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    #[serde(rename_all = "camelCase")]
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    Image { data: String, mime_type: String },
    #[serde(rename_all = "camelCase")]
    Audio { data: String, mime_type: String },
    #[serde(rename_all = "camelCase")]
    ResourceLink {
        uri: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// A future content type; the update is kept, its payload is not read.
    #[serde(other)]
    Other,
}

impl ContentBlock {
    /// A plain text block, the only kind PMP sends.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            annotations: None,
        }
    }

    /// The text of a text block; `None` for every other kind.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// Why the agent stopped processing a prompt turn. Open string: unknown
/// reasons are preserved, and callers can ask about the ones they understand.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StopReason(String);

impl StopReason {
    /// The turn ended successfully.
    pub const END_TURN: &'static str = "end_turn";
    /// The agent hit its token budget.
    pub const MAX_TOKENS: &'static str = "max_tokens";
    /// The agent hit the request budget for one turn.
    pub const MAX_TURN_REQUESTS: &'static str = "max_turn_requests";
    /// The agent refused to continue.
    pub const REFUSAL: &'static str = "refusal";
    /// The client cancelled the turn with `session/cancel`.
    pub const CANCELLED: &'static str = "cancelled";

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_end_turn(&self) -> bool {
        self.0 == Self::END_TURN
    }

    pub fn is_cancelled(&self) -> bool {
        self.0 == Self::CANCELLED
    }

    pub fn is_refusal(&self) -> bool {
        self.0 == Self::REFUSAL
    }
}

impl From<&str> for StopReason {
    fn from(reason: &str) -> Self {
        Self(reason.to_string())
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// `session/prompt` request parameters.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub session_id: SessionId,
    pub prompt: Vec<ContentBlock>,
}

/// `session/prompt` response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: StopReason,
}

/// `session/cancel` notification parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelNotification {
    pub session_id: SessionId,
}

/// Lifecycle of one tool call. Open string for the same reason as
/// [`StopReason`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallStatus(String);

impl ToolCallStatus {
    pub const PENDING: &'static str = "pending";
    pub const IN_PROGRESS: &'static str = "in_progress";
    pub const COMPLETED: &'static str = "completed";
    pub const FAILED: &'static str = "failed";

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the call reached a terminal state.
    pub fn is_finished(&self) -> bool {
        matches!(self.0.as_str(), Self::COMPLETED | Self::FAILED)
    }
}

impl From<&str> for ToolCallStatus {
    fn from(status: &str) -> Self {
        Self(status.to_string())
    }
}

impl fmt::Display for ToolCallStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Category of a tool call, used by clients to pick an icon. Open string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolKind(String);

impl ToolKind {
    pub const READ: &'static str = "read";
    pub const EDIT: &'static str = "edit";
    pub const DELETE: &'static str = "delete";
    pub const MOVE: &'static str = "move";
    pub const SEARCH: &'static str = "search";
    pub const EXECUTE: &'static str = "execute";
    pub const THINK: &'static str = "think";
    pub const FETCH: &'static str = "fetch";
    pub const OTHER: &'static str = "other";

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ToolKind {
    fn from(kind: &str) -> Self {
        Self(kind.to_string())
    }
}

impl fmt::Display for ToolKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One entry of the agent's execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    /// `high`, `medium` or `low`; kept as text.
    pub priority: String,
    /// `pending`, `in_progress` or `completed`; kept as text.
    pub status: String,
}

/// One `session/update` payload, discriminated by `sessionUpdate`.
///
/// Only the variants the TUI renders are modelled. Anything else — a future
/// protocol addition or an update from an extension — becomes
/// [`SessionUpdate::Unknown`], which is logged and otherwise ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    #[serde(rename_all = "camelCase")]
    UserMessageChunk {
        content: ContentBlock,
        #[serde(default)]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AgentMessageChunk {
        content: ContentBlock,
        #[serde(default)]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AgentThoughtChunk {
        content: ContentBlock,
        #[serde(default)]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        tool_call_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        kind: Option<ToolKind>,
        #[serde(default)]
        status: Option<ToolCallStatus>,
        #[serde(default)]
        content: Vec<Value>,
        #[serde(default)]
        locations: Vec<Value>,
        #[serde(default)]
        raw_input: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallUpdate {
        tool_call_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        kind: Option<ToolKind>,
        #[serde(default)]
        status: Option<ToolCallStatus>,
        /// Present only when the agent replaces the content collection.
        #[serde(default)]
        content: Option<Vec<Value>>,
        #[serde(default)]
        raw_input: Option<Value>,
        #[serde(default)]
        raw_output: Option<Value>,
    },
    Plan {
        #[serde(default)]
        entries: Vec<PlanEntry>,
    },
    #[serde(rename_all = "camelCase")]
    AvailableCommandsUpdate {
        #[serde(default)]
        available_commands: Vec<Value>,
    },
    #[serde(rename_all = "camelCase")]
    CurrentModeUpdate { current_mode_id: String },
    /// A variant this client does not know yet.
    #[serde(other)]
    Unknown,
}

impl SessionUpdate {
    /// Short label for logs and transcripts.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UserMessageChunk { .. } => "user_message_chunk",
            Self::AgentMessageChunk { .. } => "agent_message_chunk",
            Self::AgentThoughtChunk { .. } => "agent_thought_chunk",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolCallUpdate { .. } => "tool_call_update",
            Self::Plan { .. } => "plan",
            Self::AvailableCommandsUpdate { .. } => "available_commands_update",
            Self::CurrentModeUpdate { .. } => "current_mode_update",
            Self::Unknown => "unknown",
        }
    }
}

/// `session/update` notification parameters.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateNotification {
    pub session_id: SessionId,
    pub update: SessionUpdate,
}

impl SessionUpdateNotification {
    /// The streamed text of this update, when it carries one.
    pub fn text(&self) -> Option<&str> {
        match &self.update {
            SessionUpdate::UserMessageChunk { content, .. }
            | SessionUpdate::AgentMessageChunk { content, .. }
            | SessionUpdate::AgentThoughtChunk { content, .. } => content.as_text(),
            _ => None,
        }
    }
}

/// Response to an agent's `session/request_permission`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionResponse {
    pub outcome: PermissionOutcome,
}

impl RequestPermissionResponse {
    /// The answer PMP gives when there is nobody to ask. The protocol allows
    /// `cancelled` at any moment and the agent must not wait for the user.
    pub fn cancelled() -> Self {
        Self {
            outcome: PermissionOutcome::Cancelled,
        }
    }
}

/// What the user decided about an agent's permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Selected {
        option_id: String,
    },
    Cancelled,
}
