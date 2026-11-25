// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live agent activity, fed by the TUI event loop.
//!
//! The loop is the only place that sees agent events, so it records them here:
//! this is the state C3 and C7 will render (an agent working on a task, the
//! last thing it said, whether the connection is still alive). Until those
//! screens exist it is also what makes the wiring observable and testable.
//!
//! Recording never touches the repository: an agent writes to PMP through the
//! MCP tools, not through the TUI, so the loop stays a passive consumer.

use std::collections::BTreeMap;

use crate::acp::{AcpEvent, SessionId};
use crate::app::App;
use crate::tui::events::AgentEvent;

/// What the TUI knows about one agent session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentActivity {
    /// Profile the session was launched with.
    pub agent: String,
    /// Number of events received: chunks, tool calls, plans and requests.
    pub updates: usize,
    /// Text of the most recent streamed chunk.
    pub last_text: Option<String>,
    /// Whether the connection to the agent ended.
    pub closed: bool,
}

impl App {
    /// Records one event from an agent session. Called once per event by the
    /// loop; the session is created on its first event.
    pub fn record_agent_event(&mut self, event: AgentEvent) {
        let AgentEvent {
            session,
            agent,
            event,
        } = event;
        let activity = self.agents.entry(session).or_insert_with(|| AgentActivity {
            agent,
            ..AgentActivity::default()
        });
        match event {
            AcpEvent::Update(notification) => {
                activity.updates += 1;
                if let Some(text) = notification.text() {
                    activity.last_text = Some(text.to_string());
                }
            }
            AcpEvent::Unserved { .. } => activity.updates += 1,
            AcpEvent::Closed { .. } => activity.closed = true,
        }
    }

    /// The live agent activity, keyed by session.
    pub fn agent_activity(&self) -> &BTreeMap<SessionId, AgentActivity> {
        &self.agents
    }
}
