// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent sessions the TUI is streaming.
//!
//! A session is created by the flow that sends a PMP task to an agent (C3).
//! This registry owns the plumbing around it: a bridge task that forwards the
//! client's events into the single TUI channel, and an orderly shutdown when
//! the loop ends, so closing the interface never leaves an agent process
//! running.
//!
//! Progress is forwarded as [`TuiEvent::Agent`], tagged with the session and
//! the profile, so the loop can route it without knowing which agent produced
//! it. Updates are disposable under back-pressure: a slow consumer loses the
//! oldest ones instead of stalling the agent.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::acp::{AcpClient, AcpEvent, SessionId, SessionUpdate, SessionUpdateNotification};
use crate::application::runs::{self, PreparedRun};
use crate::config::AgentProfile;
use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::tui::events::{AgentEvent, AgentFinished, AgentRunOutcome, TuiEvent};

/// Largest agent message text kept for the closing note, in bytes. A long turn
/// keeps its last output instead of growing without bound.
const MAX_TRANSCRIPT_BYTES: usize = 16 * 1024;

/// One streaming session: the client, the profile it came from and the task
/// that forwards its updates.
struct Session {
    agent: String,
    client: AcpClient,
    bridge: JoinHandle<()>,
}

/// Every live agent session, keyed by session id.
///
/// Dropping the registry without [`AgentSessions::cancel_all`] is the safety
/// net, not the plan: each client kills its child when dropped, so nothing is
/// orphaned even if the TUI is torn down abruptly. The loop always cancels
/// first, which closes stdin and gives the agent a chance to exit on its own.
pub struct AgentSessions {
    sender: UnboundedSender<TuiEvent>,
    sessions: BTreeMap<SessionId, Session>,
}

impl AgentSessions {
    /// A registry that publishes into the TUI channel behind `sender`.
    pub fn new(sender: UnboundedSender<TuiEvent>) -> Self {
        Self {
            sender,
            sessions: BTreeMap::new(),
        }
    }

    /// Starts streaming `client`'s events as `session`.
    ///
    /// Registering the same session twice replaces the previous entry: its
    /// bridge stops and its client is dropped, which kills a leaked child.
    pub fn register(&mut self, agent: impl Into<String>, session: SessionId, client: AcpClient) {
        let agent = agent.into();
        let bridge = bridge(&agent, &session, client.clone(), &self.sender);
        self.sessions.insert(
            session,
            Session {
                agent,
                client,
                bridge,
            },
        );
    }

    /// Whether `session` is being streamed.
    pub fn contains(&self, session: &SessionId) -> bool {
        self.sessions.contains_key(session)
    }

    /// Stops streaming `session` and hands its client back, so the caller can
    /// keep the connection for the rest of the turn (C3) or shut it down.
    pub fn remove(&mut self, session: &SessionId) -> Option<AcpClient> {
        let session = self.sessions.remove(session)?;
        session.bridge.abort();
        Some(session.client)
    }

    /// Names and ids of the live sessions, for the status line C7 adds.
    pub fn running(&self) -> impl Iterator<Item = (&str, &SessionId)> {
        self.sessions
            .iter()
            .map(|(session, handle)| (handle.agent.as_str(), session))
    }

    /// Launches `profile` in the prepared workspace, creates the session, marks
    /// the task `in_progress` and starts the turn in the background.
    ///
    /// The profile process and the session are cleaned up when any step after
    /// the spawn fails, so a failed launch never leaves the task in progress
    /// with nobody working on it.
    pub async fn start_task(
        &mut self,
        agent: &str,
        profile: &AgentProfile,
        prepared: &PreparedRun,
        repo: Arc<dyn Repository>,
    ) -> Result<SessionId, AppError> {
        let client = AcpClient::launch_in(profile, &prepared.working_dir).await?;
        let session = match open_session(&client, &prepared.working_dir).await {
            Ok(session) => session,
            Err(error) => {
                client.shutdown().await;
                return Err(error);
            }
        };
        let prompt = prepared.prompt.clone();
        if let Err(error) = self.start_prompt(
            client.clone(),
            session.clone(),
            agent,
            prepared.task_id,
            prompt,
            repo,
        ) {
            client.shutdown().await;
            return Err(error);
        }
        Ok(session)
    }

    /// Starts the prompt turn on an already-open client and registers the
    /// session, so production and the simulated agent share one path.
    ///
    /// Marks the task `in_progress` before sending the prompt, forwards the
    /// streamed updates through the normal bridge and reports the end of the
    /// turn as [`TuiEvent::AgentFinished`] with the agent's accumulated text.
    pub fn start_prompt(
        &mut self,
        client: AcpClient,
        session: SessionId,
        agent: &str,
        task_id: i64,
        prompt: String,
        repo: Arc<dyn Repository>,
    ) -> Result<(), AppError> {
        runs::mark_started(repo.as_ref(), task_id)?;
        self.register(agent, session.clone(), client.clone());
        let sender = self.sender.clone();
        let agent = agent.to_string();
        let finished_session = session.clone();
        let mut events = client.subscribe();
        tokio::spawn(async move {
            let turn = client.prompt(&session, prompt);
            tokio::pin!(turn);
            let mut transcript = String::new();
            let result = loop {
                tokio::select! {
                    result = &mut turn => break result,
                    event = events.recv() => match event {
                        Ok(AcpEvent::Update(update)) => push_agent_text(&mut transcript, &update),
                        Ok(_) | Err(RecvError::Lagged(_)) => {}
                        // The connection ended; the prompt future already owns
                        // the matching error and resolves right away.
                        Err(RecvError::Closed) => break turn.as_mut().await,
                    },
                }
            };
            // The response is read after every update, but the broadcast
            // subscriber may still have messages queued: drain them so the
            // closing note includes the agent's complete last message.
            while let Ok(event) = events.try_recv() {
                if let AcpEvent::Update(update) = event {
                    push_agent_text(&mut transcript, &update);
                }
            }
            let outcome = match result {
                Ok(stop_reason) => AgentRunOutcome::Finished(stop_reason),
                Err(error) => AgentRunOutcome::Failed(error.user_message()),
            };
            let _ = sender.send(TuiEvent::AgentFinished(Box::new(AgentFinished {
                session: finished_session,
                agent,
                task_id,
                outcome,
                transcript,
            })));
        });
        Ok(())
    }

    /// Asks one session to stop its turn. A session that already ended is not
    /// an error: the loop may have seen `Ctrl+x` a moment after the agent
    /// finished.
    pub async fn cancel(&self, session: &SessionId) -> Result<(), AppError> {
        match self.sessions.get(session) {
            Some(handle) => handle.client.cancel(session).await,
            None => Ok(()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Cancels every turn and shuts each agent down.
    ///
    /// Called once when the TUI exits, whatever the reason. Cancelling is best
    /// effort — the agent may already be gone — but shutting the client down
    /// is not: it closes stdin, waits briefly and then kills the process.
    pub async fn cancel_all(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for (session, handle) in sessions {
            handle.bridge.abort();
            if let Err(error) = handle.client.cancel(&session).await {
                tracing::debug!(%error, session = %session, "cannot cancel the agent turn");
            }
            handle.client.shutdown().await;
        }
    }
}

/// Negociates the protocol and creates a session in `cwd`. Kept apart so the
/// caller can shut the process down when either step fails.
async fn open_session(client: &AcpClient, cwd: &Path) -> Result<SessionId, AppError> {
    client.initialize().await?;
    client.new_session(cwd).await
}

/// Appends one agent message chunk to the turn transcript, keeping the most
/// recent bytes. The tail matters: the closing summary is the last thing a
/// well-behaved agent writes.
fn push_agent_text(transcript: &mut String, notification: &SessionUpdateNotification) {
    let SessionUpdate::AgentMessageChunk { content, .. } = &notification.update else {
        return;
    };
    let Some(text) = content.as_text() else {
        return;
    };
    transcript.push_str(text);
    if transcript.len() > MAX_TRANSCRIPT_BYTES {
        let mut start = transcript.len() - MAX_TRANSCRIPT_BYTES;
        while !transcript.is_char_boundary(start) {
            start += 1;
        }
        let tail = transcript.split_off(start);
        *transcript = format!("…{tail}");
    }
}

/// Forwards the client's updates into the TUI channel until the connection
/// closes or the loop is gone.
fn bridge(
    agent: &str,
    session: &SessionId,
    client: AcpClient,
    sender: &UnboundedSender<TuiEvent>,
) -> JoinHandle<()> {
    let mut events = client.subscribe();
    let agent = agent.to_string();
    let session = session.clone();
    let sender = sender.clone();
    tokio::spawn(async move {
        // An agent that already died before the UI started listening would
        // never publish again: report its closure and stop.
        if let Some(reason) = client.closed_reason() {
            let event = AcpEvent::Closed { reason };
            let _ = sender.send(TuiEvent::Agent(Box::new(AgentEvent {
                session,
                agent,
                event,
            })));
            return;
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    let forwarded = TuiEvent::Agent(Box::new(AgentEvent {
                        session: session.clone(),
                        agent: agent.clone(),
                        event,
                    }));
                    if sender.send(forwarded).is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        session = %session,
                        "agent updates dropped by a slow TUI"
                    );
                }
                Err(RecvError::Closed) => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use crate::acp::sim::{self, SimOptions};
    use crate::db::repository::{ProjectRepository, RepositoryPolicy, TaskRepository};
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::tui::events::{EventSource, TuiEvent, TuiEvents};

    use super::*;

    fn deadline() -> Duration {
        Duration::from_millis(50)
    }

    /// One text chunk, so a bridge has something to forward.
    fn one_update() -> SimOptions {
        SimOptions {
            updates: vec![json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "working" },
            })],
            ..SimOptions::default()
        }
    }

    async fn session_with(
        options: SimOptions,
    ) -> (AcpClient, SessionId, tempfile::TempDir, sim::SimAgent) {
        let (client, agent) = sim::spawn(options).await;
        client.initialize().await.expect("the agent initializes");
        let workspace = tempfile::tempdir().expect("a workspace");
        let session = client
            .new_session(workspace.path())
            .await
            .expect("a session");
        (client, session, workspace, agent)
    }

    #[tokio::test]
    async fn agent_progress_reaches_the_tui_channel() {
        let (client, session, _workspace, _agent) = session_with(one_update()).await;
        let mut events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());
        sessions.register("claude", session.clone(), client.clone());
        assert!(sessions.contains(&session));
        assert_eq!(sessions.len(), 1);

        let turn = tokio::spawn({
            let client = client.clone();
            let session = session.clone();
            async move { client.prompt(&session, "go").await }
        });
        let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
            .await
            .expect("an event arrives")
            .expect("the channel is open");
        match event {
            TuiEvent::Agent(event) => {
                assert_eq!(event.session, session);
                assert_eq!(event.agent, "claude");
                match event.event {
                    crate::acp::AcpEvent::Update(notification) => {
                        assert_eq!(notification.text(), Some("working"));
                    }
                    other => panic!("unexpected agent event: {other:?}"),
                }
            }
            other => panic!("unexpected TUI event: {other:?}"),
        }
        assert!(turn.await.unwrap().unwrap().is_end_turn());
        sessions.cancel_all().await;
    }

    #[tokio::test]
    async fn cancel_all_cancels_and_stops_every_agent() {
        let options = SimOptions {
            wait_for_cancel: true,
            ..one_update()
        };
        let (client, session, _workspace, mut agent) = session_with(options).await;
        let mut events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());
        sessions.register("claude", session.clone(), client.clone());

        let turn = tokio::spawn({
            let client = client.clone();
            let session = session.clone();
            async move { client.prompt(&session, "go").await }
        });
        // The agent streams one update before waiting for the cancel, so the
        // turn is really in flight when the TUI closes.
        let _ = tokio::time::timeout(Duration::from_secs(5), events.next_event())
            .await
            .expect("the turn started")
            .expect("the channel is open");

        sessions.cancel_all().await;

        assert!(sessions.is_empty());
        assert!(!sessions.contains(&session));
        assert!(client.is_closed(), "the connection is closed");
        // Waiting for the agent task also proves the cancel was written first:
        // the pipe delivers `session/cancel` before the end of the stream.
        assert!(agent.finished().await, "the agent process ended");
        assert!(agent.was_cancelled(), "the agent was asked to stop");
        // The turn ends however the pipe reports it; the TUI no longer cares.
        let _ = turn.await;
    }

    #[tokio::test]
    async fn removing_a_session_stops_forwarding_and_returns_the_client() {
        let (client, session, _workspace, _agent) = session_with(one_update()).await;
        let events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());
        sessions.register("claude", session.clone(), client.clone());
        let names: Vec<(&str, String)> = sessions
            .running()
            .map(|(agent, session)| (agent, session.to_string()))
            .collect();
        assert_eq!(names, vec![("claude", session.to_string())]);

        let removed = sessions.remove(&session).expect("the session is removed");
        assert!(sessions.is_empty());
        assert!(!sessions.contains(&session));
        assert!(removed.initialize().await.is_ok());
        removed.shutdown().await;
    }

    #[tokio::test]
    async fn a_session_that_died_before_the_bridge_started_reports_it() {
        let (client, session, _workspace, _agent) = session_with(one_update()).await;
        // The connection ends before the TUI starts listening.
        client.shutdown().await;
        let mut events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());
        sessions.register("claude", session.clone(), client.clone());

        let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
            .await
            .expect("the closure arrives")
            .expect("the channel is open");
        match event {
            TuiEvent::Agent(event) => {
                assert_eq!(event.session, session);
                assert!(matches!(event.event, crate::acp::AcpEvent::Closed { .. }));
            }
            other => panic!("unexpected TUI event: {other:?}"),
        }
        sessions.cancel_all().await;
    }

    #[tokio::test]
    async fn cancelling_an_empty_registry_is_harmless() {
        let events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());
        sessions.cancel_all().await;
        assert!(sessions.is_empty());
        assert_eq!(sessions.len(), 0);
        assert!(sessions.remove(&SessionId::new("nope")).is_none());
    }

    fn workspace_task() -> (Arc<StubRepository>, i64, tempfile::TempDir) {
        let repo = Arc::new(StubRepository::default());
        let workspace = tempfile::tempdir().expect("a workspace");
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
                "",
                Status::Todo,
                Priority::High,
            )
            .unwrap()
            .id
            .unwrap();
        (repo, task_id, workspace)
    }

    async fn next_until_finished(events: &mut TuiEvents) -> crate::tui::events::AgentFinished {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
                .await
                .expect("an event arrives")
                .expect("the channel is open");
            match event {
                TuiEvent::AgentFinished(finished) => return *finished,
                TuiEvent::Agent(_) | TuiEvent::Tick => {}
                other => panic!("unexpected TUI event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn start_prompt_marks_in_progress_and_reports_the_accumulated_turn() {
        let (repo, task_id, workspace) = workspace_task();
        let (client, mut agent) = sim::spawn(SimOptions {
            updates: vec![
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "Implemented" },
                }),
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": " and tested." },
                }),
            ],
            ..SimOptions::default()
        })
        .await;
        client.initialize().await.unwrap();
        let session = client.new_session(workspace.path()).await.unwrap();
        let mut events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());

        sessions
            .start_prompt(
                client.clone(),
                session.clone(),
                "claude",
                task_id,
                "implement it".to_string(),
                repo.clone(),
            )
            .expect("the turn starts");

        let finished = next_until_finished(&mut events).await;
        assert_eq!(finished.session, session);
        assert_eq!(finished.task_id, task_id);
        assert_eq!(finished.transcript, "Implemented and tested.");
        assert_eq!(
            finished.outcome,
            crate::tui::events::AgentRunOutcome::Finished(crate::acp::StopReason::from("end_turn"))
        );
        assert_eq!(
            repo.get_task(task_id).unwrap().status,
            crate::domain::task::Status::InProgress
        );
        let prompt = agent.prompt_params().expect("session/prompt was sent");
        assert_eq!(prompt["prompt"][0]["text"], "implement it");
        assert_eq!(prompt["sessionId"], "sess-sim");

        sessions.cancel_all().await;
        assert!(agent.finished().await);
    }

    #[tokio::test]
    async fn start_task_reports_an_unavailable_agent_and_leaves_the_task_untouched() {
        let (repo, task_id, workspace) = workspace_task();
        let prepared =
            crate::application::runs::prepare(repo.as_ref(), task_id).expect("a valid run");
        let profile = crate::config::AgentProfile {
            command: "/definitely/not/a/pmp/agent".to_string(),
            args: Vec::new(),
            env: Default::default(),
        };
        let events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());

        let error = sessions
            .start_task("missing", &profile, &prepared, repo.clone())
            .await
            .expect_err("the binary does not exist");

        assert!(matches!(
            error,
            crate::domain::error::AppError::AgentUnavailable(_)
        ));
        assert_eq!(
            repo.get_task(task_id).unwrap().status,
            Status::Todo,
            "a failed launch must not mark the task in progress"
        );
        assert!(sessions.is_empty());
        drop(workspace);
    }

    #[tokio::test]
    async fn cancel_stops_a_waiting_turn_and_reports_the_partial_output() {
        let (repo, task_id, workspace) = workspace_task();
        let (client, mut agent) = sim::spawn(SimOptions {
            updates: vec![json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "working" },
            })],
            wait_for_cancel: true,
            ..SimOptions::default()
        })
        .await;
        client.initialize().await.unwrap();
        let session = client.new_session(workspace.path()).await.unwrap();
        let mut events = TuiEvents::detached(None, deadline());
        let mut sessions = AgentSessions::new(events.sender());

        sessions
            .start_prompt(
                client.clone(),
                session.clone(),
                "claude",
                task_id,
                "implement it".to_string(),
                repo.clone(),
            )
            .expect("the turn starts");

        // Wait for the agent's first update: it proves `session/prompt` was
        // read, so the cancel below cannot overtake it on the pipe.
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
                .await
                .expect("the turn starts")
                .expect("the channel is open");
            if matches!(event, TuiEvent::Agent(_)) {
                break;
            }
        }

        sessions.cancel(&session).await.expect("the cancel is sent");
        let finished = next_until_finished(&mut events).await;

        assert!(matches!(
            finished.outcome,
            crate::tui::events::AgentRunOutcome::Finished(reason) if reason.is_cancelled()
        ));
        assert_eq!(finished.transcript, "working");
        assert!(agent.was_cancelled());

        sessions.cancel_all().await;
        assert!(agent.finished().await);
    }
}
