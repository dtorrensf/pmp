// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live agent activity and the TUI state of one implementation run.
//!
//! The loop is the only place that sees agent events, so it records them here:
//! this is the state C3 renders (an agent working on a task, the last thing it
//! said) and C7 will replace with a full panel.
//!
//! Recording an update never touches the repository: an agent writes to PMP
//! through the MCP tools, not through the TUI. Only the end of a turn writes
//! one closing note, so the user keeps the final verification.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::acp::{AcpEvent, SessionId};
use crate::app::App;
use crate::application::runs::{self, PreparedRun};
use crate::config::AgentProfile;
use crate::screens::components::agent::{AgentRunState, AgentStatus};
use crate::tui::events::{AgentEvent, AgentFinished, AgentRunOutcome};

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

/// One task sent to an agent, from validation to the closing note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRun {
    pub task_id: i64,
    pub task_name: String,
    pub version_id: i64,
    pub agent: String,
    /// `None` while the process and the session are being created.
    pub session: Option<SessionId>,
    pub state: AgentRunState,
    /// The user asked to cancel before the session existed.
    pub cancel_requested: bool,
}

/// A validated run waiting for the loop to launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRequest {
    pub agent: String,
    pub prepared: PreparedRun,
}

/// Profile selection when `config.toml` defines more than one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPicker {
    pub prepared: PreparedRun,
    pub agents: Vec<String>,
    pub selected: usize,
}

impl App {
    /// Installs the resolved `[agents.*]` profiles once, at startup.
    pub fn set_agent_profiles(&mut self, profiles: BTreeMap<String, AgentProfile>) {
        self.agent_profiles = profiles;
    }

    /// The profile with this name, for the loop that launches it.
    pub fn agent_profile(&self, name: &str) -> Option<&AgentProfile> {
        self.agent_profiles.get(name)
    }

    /// The run currently in flight, if any.
    pub fn agent_run(&self) -> Option<&AgentRun> {
        self.active_run.as_ref()
    }

    /// Data for the status strip, borrowing the live activity of the session.
    pub fn agent_status(&self) -> Option<AgentStatus<'_>> {
        let run = self.active_run.as_ref()?;
        let activity = run
            .session
            .as_ref()
            .and_then(|session| self.agents.get(session));
        Some(AgentStatus {
            agent: &run.agent,
            task_id: run.task_id,
            task_name: &run.task_name,
            state: run.state,
            updates: activity.map_or(0, |activity| activity.updates),
            last_text: activity.and_then(|activity| activity.last_text.as_deref()),
        })
    }

    /// The profile picker data, when it is open.
    pub fn agent_picker(&self) -> Option<(&[String], usize)> {
        self.agent_picker
            .as_ref()
            .map(|picker| (picker.agents.as_slice(), picker.selected))
    }

    /// Validates `task_id` and either queues the run or opens the profile
    /// picker. Every refusal becomes an error toast; nothing is launched.
    pub(crate) fn request_task_run(&mut self, task_id: i64) {
        if self.active_run.is_some() {
            self.push_error("an agent run is already in progress");
            return;
        }
        if self.agent_picker.is_some() {
            return;
        }
        let prepared = match runs::prepare(self.repo.as_ref(), task_id) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.push_error(error.user_message());
                return;
            }
        };
        let agents: Vec<String> = self.agent_profiles.keys().cloned().collect();
        match agents.len() {
            0 => self.push_error(
                "no agent profiles are configured; add an [agents.<name>] entry to config.toml",
            ),
            1 => self.queue_run(agents[0].clone(), prepared),
            _ => {
                self.agent_picker = Some(AgentPicker {
                    prepared,
                    agents,
                    selected: 0,
                });
            }
        }
    }

    fn queue_run(&mut self, agent: String, prepared: PreparedRun) {
        self.agent_picker = None;
        self.active_run = Some(AgentRun {
            task_id: prepared.task_id,
            task_name: prepared.task_name.clone(),
            version_id: prepared.version_id,
            agent: agent.clone(),
            session: None,
            state: AgentRunState::Starting,
            cancel_requested: false,
        });
        self.pending_agent_request = Some(AgentRequest { agent, prepared });
    }

    /// The run the loop must launch, exactly once.
    pub fn take_agent_request(&mut self) -> Option<AgentRequest> {
        self.pending_agent_request.take()
    }

    /// The session to cancel, exactly once.
    pub fn take_cancel_request(&mut self) -> Option<SessionId> {
        self.pending_cancel.take()
    }

    /// The session exists: the run moves from starting to running, or asks for
    /// the cancellation the user requested while the agent was starting.
    pub fn agent_session_started(&mut self, session: SessionId) {
        let Some(run) = self.active_run.as_mut() else {
            return;
        };
        run.session = Some(session.clone());
        if run.cancel_requested {
            run.state = AgentRunState::Cancelling;
            self.pending_cancel = Some(session);
        } else {
            run.state = AgentRunState::Running;
        }
    }

    /// The launch failed before a session existed: clear the run and report it.
    pub fn agent_start_failed(&mut self, message: impl Into<String>) {
        self.active_run = None;
        self.pending_agent_request = None;
        self.push_error(message.into());
    }

    /// Cancelling failed. The run stays visible: the turn may end on its own.
    pub fn agent_cancel_failed(&mut self, message: impl Into<String>) {
        self.push_error(message.into());
    }

    /// Handles the picker and the global cancel shortcut. Returns `true` when
    /// the key was consumed and must not reach the screen.
    pub fn handle_agent_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.agent_picker.is_some() {
            self.handle_picker_key(code);
            return true;
        }
        let Some(run) = self.active_run.as_mut() else {
            return false;
        };
        if code != KeyCode::Char('x') || modifiers != KeyModifiers::CONTROL {
            return false;
        }
        if run.state != AgentRunState::Cancelling {
            run.cancel_requested = true;
            run.state = AgentRunState::Cancelling;
            if let Some(session) = &run.session {
                self.pending_cancel = Some(session.clone());
            }
        }
        true
    }

    fn handle_picker_key(&mut self, code: KeyCode) {
        let mut chosen = false;
        if let Some(picker) = self.agent_picker.as_mut() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.selected = picker.selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    picker.selected =
                        (picker.selected + 1).min(picker.agents.len().saturating_sub(1));
                }
                KeyCode::Enter => chosen = true,
                KeyCode::Esc => {
                    self.agent_picker = None;
                    return;
                }
                _ => {}
            }
        }
        if chosen
            && let Some(picker) = self.agent_picker.take()
            && let Some(agent) = picker.agents.get(picker.selected).cloned()
        {
            self.queue_run(agent, picker.prepared);
        }
    }

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

    /// Closes the run: records the agent's text as a note and leaves the task
    /// `in_progress`. The user decides when the verification is complete.
    pub fn finish_agent_run(&mut self, finished: &AgentFinished) {
        // A late event from a run that is no longer active must not clear the
        // live one.
        if self
            .active_run
            .as_ref()
            .is_none_or(|run| run.session.as_ref() != Some(&finished.session))
        {
            return;
        }
        let Some(run) = self.active_run.take() else {
            return;
        };
        let note = closing_note(
            &run.agent,
            &finished.outcome,
            &finished.transcript,
            run.cancel_requested,
        );
        match runs::record_result(self.repo.as_ref(), run.task_id, &note) {
            Ok(_) => self.push_success(format!(
                "Agent {} finished; note added to task #{}",
                run.agent, run.task_id
            )),
            Err(error) => self.push_error(error.user_message()),
        }
        self.agents.remove(&finished.session);
    }

    /// The live agent activity, keyed by session.
    pub fn agent_activity(&self) -> &BTreeMap<SessionId, AgentActivity> {
        &self.agents
    }
}

/// The note written when a turn ends. The header names the agent and the stop
/// reason; the body is whatever the agent streamed, capped at 16 KiB.
fn closing_note(
    agent: &str,
    outcome: &AgentRunOutcome,
    transcript: &str,
    cancel_requested: bool,
) -> String {
    let header = match outcome {
        AgentRunOutcome::Finished(stop_reason)
            if cancel_requested || stop_reason.is_cancelled() =>
        {
            format!("Agent run cancelled with {agent}.")
        }
        AgentRunOutcome::Finished(stop_reason) => {
            format!("Agent run finished with {agent} ({stop_reason}).")
        }
        AgentRunOutcome::Failed(message) => {
            format!("Agent run failed with {agent}: {message}")
        }
    };
    let body = transcript.trim();
    if body.is_empty() {
        format!("{header}\n\nThe agent produced no output.")
    } else {
        format!("{header}\n\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use crate::acp::StopReason;

    use super::closing_note;

    #[test]
    fn finished_note_names_the_agent_and_the_stop_reason() {
        let note = closing_note(
            "claude",
            &crate::tui::events::AgentRunOutcome::Finished(StopReason::from("end_turn")),
            "Implemented the task.",
            false,
        );
        assert!(note.contains("Agent run finished with claude (end_turn)"));
        assert!(note.contains("Implemented the task."));
    }

    #[test]
    fn cancelled_note_says_so_and_keeps_the_partial_output() {
        let note = closing_note(
            "claude",
            &crate::tui::events::AgentRunOutcome::Finished(StopReason::from("cancelled")),
            "partial",
            true,
        );
        assert!(note.contains("cancelled"));
        assert!(note.contains("partial"));
    }

    #[test]
    fn a_cancel_requested_turn_is_noted_as_cancelled_even_if_the_agent_ends_it() {
        let note = closing_note(
            "claude",
            &crate::tui::events::AgentRunOutcome::Finished(StopReason::from("end_turn")),
            "",
            true,
        );
        assert!(note.contains("cancelled"));
        assert!(!note.contains("finished"));
    }

    #[test]
    fn failed_and_empty_turns_still_produce_a_readable_note() {
        let failed = closing_note(
            "gemini",
            &crate::tui::events::AgentRunOutcome::Failed("the agent exited".to_string()),
            "",
            false,
        );
        assert!(failed.contains("Agent run failed with gemini: the agent exited"));
        assert!(failed.contains("no output"));
    }
}
