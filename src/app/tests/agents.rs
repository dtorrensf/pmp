// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI keeps a small, observable record of what each agent session is
//! doing; C3 and C7 render it.

use std::collections::BTreeMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::acp::{
    AcpEvent, ContentBlock, SessionId, SessionUpdate, SessionUpdateNotification, StopReason,
};
use crate::config::AgentProfile;
use crate::db::repository::{
    DependencyRepository, NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
};
use crate::db::stub::StubRepository;
use crate::domain::task::{Priority, Status};
use crate::screens::Action;
use crate::screens::components::agent::AgentRunState;
use crate::tui::events::{AgentEvent, AgentFinished, AgentRunOutcome};

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

fn profile(command: &str) -> AgentProfile {
    AgentProfile {
        command: command.to_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// A project with a temporary workspace and one task ready to implement.
fn ready_app(profiles: &[(&str, &str)]) -> (Arc<StubRepository>, App, i64, tempfile::TempDir) {
    let repo = Arc::new(StubRepository::default());
    let workspace = tempfile::tempdir().unwrap();
    let project = repo
        .create_project("Alpha", "", Some(workspace.path().to_str().unwrap()))
        .unwrap();
    let project_id = project.id.unwrap();
    let version_id = repo.default_version_id(project_id).unwrap();
    let task_id = repo
        .create_task(
            project_id,
            version_id,
            "Implement C3",
            "## Context\n\nctx\n\n## Requirements\n\nreq\n\n## Benefits\n\nben",
            Status::Todo,
            Priority::High,
        )
        .unwrap()
        .id
        .unwrap();
    let mut app = App::new(repo.clone());
    app.set_agent_profiles(
        profiles
            .iter()
            .map(|(name, command)| (name.to_string(), profile(command)))
            .collect(),
    );
    (repo, app, task_id, workspace)
}

fn finished(task_id: i64, session: &SessionId, text: &str) -> AgentFinished {
    AgentFinished {
        session: session.clone(),
        agent: "claude".to_string(),
        task_id,
        outcome: AgentRunOutcome::Finished(StopReason::from("end_turn")),
        transcript: text.to_string(),
    }
}

#[test]
fn request_task_run_validates_and_queues_the_only_profile() {
    let (_repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);

    app.dispatch(Action::ImplementTask { task_id });

    let request = app.take_agent_request().expect("the run is queued");
    assert_eq!(request.agent, "claude");
    assert_eq!(request.prepared.task_id, task_id);
    assert!(request.prepared.prompt.contains("Implement C3"));
    assert_eq!(app.agent_run().unwrap().state, AgentRunState::Starting);
    assert_eq!(app.error(), None);
    assert!(
        app.take_agent_request().is_none(),
        "the request is consumed once"
    );
}

#[test]
fn request_task_run_refuses_incomplete_dependencies() {
    let (repo, mut app, _task_id, _workspace) = ready_app(&[("claude", "unused")]);
    let project_id = repo.list_projects().unwrap()[0].id.unwrap();
    let version_id = repo.default_version_id(project_id).unwrap();
    let blocker = repo
        .create_task(
            project_id,
            version_id,
            "Blocker",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap();
    let blocked = repo
        .create_task(
            project_id,
            version_id,
            "Blocked",
            "",
            Status::Todo,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(blocked, blocker).unwrap();

    app.dispatch(Action::ImplementTask { task_id: blocked });

    assert!(app.agent_run().is_none());
    assert!(app.take_agent_request().is_none());
    assert!(
        app.error().unwrap().contains("incomplete dependencies"),
        "{:?}",
        app.error()
    );
}

#[test]
fn request_task_run_without_profiles_explains_the_configuration() {
    let (_repo, mut app, task_id, _workspace) = ready_app(&[]);

    app.dispatch(Action::ImplementTask { task_id });

    assert!(app.agent_run().is_none());
    assert!(app.take_agent_request().is_none());
    assert!(
        app.error().unwrap().contains("agent profiles"),
        "{:?}",
        app.error()
    );
}

#[test]
fn multiple_profiles_open_the_picker_and_enter_queues_the_chosen_one() {
    let (_repo, mut app, task_id, _workspace) =
        ready_app(&[("claude", "claude-acp"), ("gemini", "gemini-acp")]);

    app.dispatch(Action::ImplementTask { task_id });
    assert!(app.agent_run().is_none());
    assert_eq!(app.agent_picker().unwrap().1, 0);

    assert!(app.handle_agent_key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.agent_picker().unwrap().1, 1);
    assert!(app.handle_agent_key(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.agent_picker().is_none());
    assert_eq!(app.take_agent_request().unwrap().agent, "gemini");
    assert_eq!(app.agent_run().unwrap().state, AgentRunState::Starting);
}

#[test]
fn escape_closes_the_picker_without_queueing() {
    let (_repo, mut app, task_id, _workspace) =
        ready_app(&[("claude", "claude-acp"), ("gemini", "gemini-acp")]);
    app.dispatch(Action::ImplementTask { task_id });

    assert!(app.handle_agent_key(KeyCode::Esc, KeyModifiers::NONE));

    assert!(app.agent_picker().is_none());
    assert!(app.agent_run().is_none());
    assert!(app.take_agent_request().is_none());
}

#[test]
fn ctrl_x_cancels_a_running_session_once() {
    let (_repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);
    app.dispatch(Action::ImplementTask { task_id });
    let session = SessionId::new("sess-1");
    app.agent_session_started(session.clone());

    assert!(app.handle_agent_key(KeyCode::Char('x'), KeyModifiers::CONTROL));

    assert_eq!(app.agent_run().unwrap().state, AgentRunState::Cancelling);
    assert_eq!(app.take_cancel_request(), Some(session));
    assert!(app.take_cancel_request().is_none());
}

#[test]
fn cancelling_while_starting_waits_for_the_session() {
    let (_repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);
    app.dispatch(Action::ImplementTask { task_id });

    assert!(app.handle_agent_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
    assert!(app.take_cancel_request().is_none());

    let session = SessionId::new("sess-1");
    app.agent_session_started(session.clone());

    assert_eq!(app.agent_run().unwrap().state, AgentRunState::Cancelling);
    assert_eq!(app.take_cancel_request(), Some(session));
}

#[test]
fn finishing_a_run_writes_a_note_and_keeps_the_task_in_progress() {
    let (repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);
    app.dispatch(Action::ImplementTask { task_id });
    let session = SessionId::new("sess-1");
    app.agent_session_started(session.clone());
    repo.update_task_status(task_id, Status::InProgress)
        .unwrap();

    app.finish_agent_run(&finished(task_id, &session, "Implemented and tested."));

    assert!(app.agent_run().is_none());
    assert_eq!(repo.get_task(task_id).unwrap().status, Status::InProgress);
    let notes = repo.list_task_notes(task_id).unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].content.contains("end_turn"));
    assert!(notes[0].content.contains("Implemented and tested."));
    assert!(app.agent_activity().is_empty());
}

#[test]
fn a_late_finish_for_another_session_is_ignored() {
    let (repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);
    app.dispatch(Action::ImplementTask { task_id });
    app.agent_session_started(SessionId::new("sess-1"));

    app.finish_agent_run(&finished(
        task_id,
        &SessionId::new("sess-old"),
        "stale output",
    ));

    assert!(app.agent_run().is_some(), "the live run stays visible");
    assert!(repo.list_task_notes(task_id).unwrap().is_empty());
}

#[test]
fn a_failed_launch_clears_the_run_and_reports_the_error() {
    let (_repo, mut app, task_id, _workspace) = ready_app(&[("claude", "unused")]);
    app.dispatch(Action::ImplementTask { task_id });

    app.agent_start_failed("cannot start 'unused'");

    assert!(app.agent_run().is_none());
    assert_eq!(app.error(), Some("Cannot start 'unused'"));
}
