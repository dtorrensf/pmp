// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI keeps a small, observable record of what each agent session is
//! doing; C3 and C7 render it.

use std::sync::Arc;

use crate::acp::{AcpEvent, ContentBlock, SessionId, SessionUpdate, SessionUpdateNotification};
use crate::db::stub::StubRepository;
use crate::tui::events::AgentEvent;

use super::*;

fn chunk(session: &str, text: &str) -> AgentEvent {
    AgentEvent {
        session: SessionId::new(session),
        agent: "claude".to_string(),
        event: AcpEvent::Update(SessionUpdateNotification {
            session_id: SessionId::new(session),
            update: SessionUpdate::AgentMessageChunk {
                content: ContentBlock::text(text),
                message_id: None,
            },
        }),
    }
}

#[test]
fn agent_activity_starts_empty_and_records_the_profile() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    assert!(app.agent_activity().is_empty());

    app.record_agent_event(chunk("sess-1", "step one"));
    let activity = app.agent_activity().get(&SessionId::new("sess-1")).unwrap();
    assert_eq!(activity.agent, "claude");
    assert_eq!(activity.updates, 1);
    assert_eq!(activity.last_text.as_deref(), Some("step one"));
    assert!(!activity.closed);
}

#[test]
fn agent_activity_counts_updates_and_keeps_the_last_chunk() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    app.record_agent_event(chunk("sess-1", "step one"));
    app.record_agent_event(chunk("sess-1", "step two"));

    let activity = app.agent_activity().get(&SessionId::new("sess-1")).unwrap();
    assert_eq!(activity.updates, 2);
    assert_eq!(activity.last_text.as_deref(), Some("step two"));
}

#[test]
fn agent_activity_marks_a_closed_connection() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    app.record_agent_event(AgentEvent {
        session: SessionId::new("sess-1"),
        agent: "gemini".to_string(),
        event: AcpEvent::Closed {
            reason: "the agent exited".to_string(),
        },
    });

    let activity = app.agent_activity().get(&SessionId::new("sess-1")).unwrap();
    assert_eq!(activity.agent, "gemini");
    assert!(activity.closed);
    assert_eq!(activity.updates, 0);
    assert!(activity.last_text.is_none());
}

#[test]
fn agent_activity_keeps_one_entry_per_session() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    app.record_agent_event(chunk("sess-1", "one"));
    app.record_agent_event(chunk("sess-2", "two"));

    let activity = app.agent_activity();
    assert_eq!(activity.len(), 2);
    assert_eq!(
        activity[&SessionId::new("sess-1")].last_text.as_deref(),
        Some("one")
    );
    assert_eq!(
        activity[&SessionId::new("sess-2")].last_text.as_deref(),
        Some("two")
    );
}

#[test]
fn unserved_agent_requests_count_as_activity() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    app.record_agent_event(AgentEvent {
        session: SessionId::new("sess-1"),
        agent: "claude".to_string(),
        event: AcpEvent::Unserved {
            method: "session/request_permission".to_string(),
            id: "sim-ask".to_string(),
        },
    });

    let activity = app.agent_activity().get(&SessionId::new("sess-1")).unwrap();
    assert_eq!(activity.updates, 1);
    assert!(activity.last_text.is_none());
}
