// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Renderers of the filter and move popups.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Wrap},
};

use crate::screens::components::fields::{render_field, render_readonly_field};
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};
use crate::screens::components::theme;

use super::modes::{FilterField, FilterState, MoveTaskState};

pub(super) fn render_move(state: &MoveTaskState, frame: &mut Frame, area: Rect) {
    let height = (state.targets.len() as u16).max(1) + 4;
    let popup_area = centered_rect(52, height, area);
    let inner = render_popup_scaffold(frame, popup_area, "Move task to version");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    if state.targets.is_empty() {
        let message = Paragraph::new("This is the only version of the project.")
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(message, chunks[0]);
    } else {
        let lines: Vec<Line> = state
            .targets
            .iter()
            .enumerate()
            .map(|(i, version)| {
                let text = format!("{} · {}", version.name, version.status.label());
                let style = if i == state.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Line::styled(text, style)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), chunks[0]);
    }

    let help = if state.targets.is_empty() {
        "Esc close"
    } else {
        "↑ ↓ choose  Enter move  Esc cancel"
    };
    let help_para = Paragraph::new(help)
        .style(theme::theme().muted_style())
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[1]);
}

pub(super) fn render_filter(state: &FilterState, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(52, 15, area);
    let inner = render_popup_scaffold(frame, popup_area, "Filter Tasks");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let status_label = state
        .filter_status
        .map_or_else(|| "Any".to_string(), |s| s.label().to_string());
    let priority_label = state
        .filter_priority
        .map_or_else(|| "Any".to_string(), |p| p.label().to_string());

    render_field(
        frame,
        chunks[0],
        "Title",
        &state.filter_title,
        state.selected_field == FilterField::Title,
        None,
    );
    render_readonly_field(
        frame,
        chunks[1],
        "Status",
        &status_label,
        state.selected_field == FilterField::Status,
    );
    render_readonly_field(
        frame,
        chunks[2],
        "Priority",
        &priority_label,
        state.selected_field == FilterField::Priority,
    );

    let help = "Tab switch  ← → change  Enter apply  Esc cancel";
    let help_para = Paragraph::new(help)
        .style(theme::theme().muted_style())
        .alignment(Alignment::Center);
    frame.render_widget(help_para, chunks[3]);
}
