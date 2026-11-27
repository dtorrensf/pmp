// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Status strip and profile picker for an ACP task run.
//!
//! C3 renders a compact, non-modal strip above the footer: the user keeps
//! navigating while the agent works, sees its last message and can cancel with
//! `Ctrl+x`. C7 replaces this strip with the full session panel.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::screens::UiContext;
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};

/// Lifecycle of the run as the strip shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunState {
    /// The agent process is starting and the session is not created yet.
    Starting,
    /// The prompt is running and updates are streaming.
    Running,
    /// `Ctrl+x` asked the agent to stop; the turn is still winding down.
    Cancelling,
}

impl AgentRunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
        }
    }
}

/// What the strip needs, borrowed from the app so rendering stays stateless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatus<'a> {
    pub agent: &'a str,
    pub task_id: i64,
    pub task_name: &'a str,
    pub state: AgentRunState,
    pub updates: usize,
    pub last_text: Option<&'a str>,
}

/// Height the strip reserves in the frame. Zero when no run is active.
pub const STATUS_HEIGHT: u16 = 2;

/// Renders the live run strip in `area`, exactly two rows tall.
pub fn render_status(frame: &mut Frame, area: Rect, status: &AgentStatus<'_>, ui: &UiContext) {
    if area.height == 0 {
        return;
    }
    let (glyph, style) = match status.state {
        AgentRunState::Cancelling => (ui.glyphs().warning, ui.warning_style()),
        AgentRunState::Starting | AgentRunState::Running => (ui.spinner_glyph(), ui.accent_style()),
    };
    let hint = match status.state {
        AgentRunState::Starting => "please wait".to_string(),
        AgentRunState::Running => "Ctrl+x cancel".to_string(),
        AgentRunState::Cancelling => "stopping…".to_string(),
    };
    let first = Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(
            format!(
                "{} · #{} {} · {} · {} update{}",
                status.agent,
                status.task_id,
                status.task_name,
                status.state.label(),
                status.updates,
                if status.updates == 1 { "" } else { "s" }
            ),
            ui.text_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {hint}"), ui.muted_style()),
    ]);
    let second = Line::from(Span::styled(
        status
            .last_text
            .filter(|text| !text.is_empty())
            .unwrap_or("Waiting for the agent…"),
        ui.muted_style(),
    ));
    frame.render_widget(Paragraph::new(vec![first, second]), area);
}

/// Renders the profile picker shown when more than one agent is configured.
pub fn render_picker(
    frame: &mut Frame,
    area: Rect,
    agents: &[String],
    selected: usize,
    ui: &UiContext,
) {
    let height = (agents.len() as u16).max(1) + 5;
    let popup_area = centered_rect(52, height.min(area.height), area);
    let inner = render_popup_scaffold(frame, popup_area, "Send task to agent", ui);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    if agents.is_empty() {
        frame.render_widget(
            Paragraph::new("No agent profiles are configured.").alignment(Alignment::Center),
            chunks[0],
        );
    } else {
        let lines: Vec<Line> = agents
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                let style = if index == selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Line::styled(agent.clone(), style)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), chunks[0]);
    }

    frame.render_widget(
        Paragraph::new("↑ ↓ choose  Enter start  Esc cancel")
            .style(ui.muted_style())
            .alignment(Alignment::Center),
        chunks[1],
    );
}

/// The labels of the permission dialog, in choice order.
pub const PERMISSION_CHOICES: [&str; 3] = ["Allow once", "Allow always for this project", "Deny"];

/// What the permission dialog shows for the request at the front of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDialog<'a> {
    pub agent: &'a str,
    pub task_id: i64,
    /// What the agent wants to do, in its own words.
    pub title: &'a str,
    pub tool_kind: &'a str,
    /// Highlighted choice, an index into [`PERMISSION_CHOICES`].
    pub selected: usize,
    /// Requests waiting behind this one, this one excluded.
    pub waiting: usize,
}

/// Modal asking the user to allow or deny an agent's permission request.
pub fn render_permission(
    frame: &mut Frame,
    area: Rect,
    dialog: &PermissionDialog<'_>,
    ui: &UiContext,
) {
    let popup_area = centered_rect(66, 11, area);
    let inner = render_popup_scaffold(frame, popup_area, "Agent permission", ui);
    let mut lines = vec![
        Line::from(format!(
            "{} on task #{} asks permission to:",
            dialog.agent, dialog.task_id
        )),
        Line::styled(
            dialog.title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::styled(format!("kind: {}", dialog.tool_kind), ui.muted_style()),
        Line::raw(""),
    ];
    for (index, label) in PERMISSION_CHOICES.iter().enumerate() {
        let style = if index == dialog.selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::styled(format!(" {label} "), style));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let footer_area = Rect {
        y: popup_area.y + popup_area.height.saturating_sub(2),
        height: 1,
        ..popup_area
    };
    let more = if dialog.waiting > 0 {
        format!("  (+{} waiting)", dialog.waiting)
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(format!("↑ ↓ choose  Enter confirm  o/a/d  Esc deny{more}"))
            .style(ui.muted_style())
            .alignment(Alignment::Center),
        footer_area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::screens::test_utils::buffer_to_string;

    use super::*;

    fn render_permission_text(dialog: &PermissionDialog<'_>) -> String {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_permission(frame, frame.area(), dialog, &UiContext::default()))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn permission_dialog_shows_the_request_the_choices_and_the_queue() {
        let text = render_permission_text(&PermissionDialog {
            agent: "claude",
            task_id: 206,
            title: "Run cargo test",
            tool_kind: "execute",
            selected: 0,
            waiting: 2,
        });
        assert!(text.contains("claude on task #206"), "{text}");
        assert!(text.contains("Run cargo test"), "{text}");
        assert!(text.contains("kind: execute"), "{text}");
        assert!(text.contains("Allow once"), "{text}");
        assert!(text.contains("Allow always for this project"), "{text}");
        assert!(text.contains("Deny"), "{text}");
        assert!(text.contains("(+2 waiting)"), "{text}");
    }

    fn render_status_text(status: &AgentStatus<'_>) -> String {
        let backend = TestBackend::new(80, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_status(frame, frame.area(), status, &UiContext::default()))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    fn status(state: AgentRunState) -> AgentStatus<'static> {
        AgentStatus {
            agent: "claude",
            task_id: 205,
            task_name: "Send a task to an agent",
            state,
            updates: 3,
            last_text: Some("Reading the repository"),
        }
    }

    #[test]
    fn running_strip_shows_the_profile_task_progress_and_cancel_hint() {
        let text = render_status_text(&status(AgentRunState::Running));
        assert!(text.contains("claude"), "{text}");
        assert!(text.contains("#205 Send a task to an agent"), "{text}");
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("3 updates"), "{text}");
        assert!(text.contains("Ctrl+x cancel"), "{text}");
        assert!(text.contains("Reading the repository"), "{text}");
    }

    #[test]
    fn starting_strip_waits_and_cancelling_strip_reports_the_stop() {
        let starting = render_status_text(&status(AgentRunState::Starting));
        assert!(starting.contains("starting"), "{starting}");
        assert!(starting.contains("please wait"), "{starting}");

        let cancelling = render_status_text(&status(AgentRunState::Cancelling));
        assert!(cancelling.contains("cancelling"), "{cancelling}");
        assert!(cancelling.contains("stopping"), "{cancelling}");
    }

    #[test]
    fn strip_uses_the_muted_placeholder_without_agent_text() {
        let mut status = status(AgentRunState::Running);
        status.last_text = None;
        let text = render_status_text(&status);
        assert!(text.contains("Waiting for the agent"), "{text}");
    }

    #[test]
    fn picker_lists_every_profile_with_the_selected_one_reversed() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_picker(
                    frame,
                    frame.area(),
                    &["claude".to_string(), "gemini".to_string()],
                    1,
                    &UiContext::default(),
                )
            })
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Send task to agent"), "{text}");
        assert!(text.contains("claude"), "{text}");
        assert!(text.contains("gemini"), "{text}");
        assert!(text.contains("Enter start"), "{text}");
    }
}
