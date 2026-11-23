// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rendering of the task table and its caption.

use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Row, Table},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::screens::components::badge::Badge;
use crate::screens::components::empty_state::EmptyState;
use crate::screens::components::glyphs;
use crate::screens::components::scroll::render_scrollbar;
use crate::screens::components::theme::{self, priority_style, status_row_style, status_style};

use super::state::{SortMode, TaskListState};

/// How many dependencies a task has and how many of them are not done yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencySummary {
    pub total: usize,
    pub pending: usize,
}

/// Renders the table, or the empty state when the filter hides every task.
pub(super) fn render(
    state: &TaskListState,
    summaries: &HashMap<i64, DependencySummary>,
    frame: &mut Frame,
    area: Rect,
) {
    if state.is_empty() {
        EmptyState::new("No tasks")
            .with_icon(glyphs::glyphs().status_todo)
            .with_hint("Press c to create one.")
            .render(frame, area);
        return;
    }

    // The last column is reserved for the scrollbar; the rest holds the table.
    let table_area = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    let row_height = table_rows(area);
    let range = state.visible_range(row_height);

    // Wide enough for the largest id in the list and for the "ID" header.
    let id_width = state
        .tasks()
        .iter()
        .filter_map(|task| task.id)
        .map(|id| id.to_string().len())
        .max()
        .unwrap_or(1)
        .max("ID".len()) as u16
        + 2;

    let header = Row::new(vec![
        Cell::from(Line::from("ID").right_aligned()),
        Cell::from("Name"),
        Cell::from("Status"),
        Cell::from("Priority"),
    ])
    .style(theme::theme().header_style());

    let widths = [
        Constraint::Length(id_width),
        Constraint::Percentage(47),
        Constraint::Percentage(26),
        Constraint::Percentage(26),
    ];
    // Width of the name column, so long names can be cut to fit it.
    let name_width = Layout::horizontal(widths)
        .spacing(1)
        .flex(Flex::Start)
        .split(table_area)[1]
        .width as usize;

    let rows: Vec<Row> = state.tasks()[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, task)| {
            let i = range.start + offset;
            let selected = i == state.selected();
            let style = if selected {
                theme::theme().selection_style()
            } else {
                status_row_style(task.status)
            };
            let dependencies = task.id.and_then(|id| summaries.get(&id).copied());
            Row::new(vec![
                Cell::from(
                    Line::from(task.id.map_or_else(|| "-".to_string(), |id| id.to_string()))
                        .right_aligned(),
                ),
                name_cell(&task.name, dependencies, selected, name_width),
                Cell::from(Badge::status_live(task.status).text()).style(status_style(task.status)),
                Cell::from(Badge::priority(task.priority).text())
                    .style(priority_style(task.priority)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);

    if row_height > 0 {
        let track = Rect::new(
            area.right().saturating_sub(1),
            area.y + 1,
            1,
            row_height as u16,
        );
        render_scrollbar(frame, track, state.tasks().len(), row_height, range.start);
    }
}

/// Caption for the enclosing block: one chip per active filter, the sort
/// direction and, while filters are active, how to clear them.
pub(super) fn title(state: &TaskListState) -> Line<'static> {
    let filters = state.filters();
    filter_chips(
        filters.title.as_deref(),
        filters.status,
        filters.priority,
        sort_arrow(state.sort_mode()),
    )
}

/// Direction arrow shown in the table title, `None` for the default order.
fn sort_arrow(mode: SortMode) -> Option<&'static str> {
    let glyphs = glyphs::glyphs();
    match mode {
        SortMode::Priority => None,
        SortMode::IdAsc => Some(glyphs.sort_asc),
        SortMode::IdDesc => Some(glyphs.sort_desc),
    }
}

/// Data rows that fit below the header row of the table.
fn table_rows(area: Rect) -> usize {
    area.height.saturating_sub(1) as usize
}

/// Name cell: the selection marker, the name cut to `width` columns and,
/// when the task has dependencies, a compact count such as `↳ 2`. The count
/// is green when every dependency is done and yellow while any is pending.
fn name_cell(
    name: &str,
    dependencies: Option<DependencySummary>,
    selected: bool,
    width: usize,
) -> Cell<'static> {
    let glyphs = glyphs::glyphs();
    let marker = if selected { glyphs.selection } else { " " };
    let (suffix, suffix_style) = match dependencies {
        Some(summary) if summary.total > 0 => {
            let style = if summary.pending == 0 {
                theme::theme().success_style()
            } else {
                theme::theme().warning_style()
            };
            (format!("  {} {}", glyphs.dependency, summary.total), style)
        }
        _ => (String::new(), Style::default()),
    };
    let available = width.saturating_sub(marker.width() + 1 + suffix.width());
    Cell::from(Line::from(vec![
        Span::raw(format!("{marker} {}", truncate_display(name, available))),
        Span::styled(suffix, suffix_style),
    ]))
}

/// Cuts `text` to at most `width` terminal columns. A cut text ends with an
/// ellipsis that counts as one column.
pub fn truncate_display(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
}

/// The table caption: "Tasks", one bracketed chip per active filter, then the
/// sort direction. Chips keep their shape without color, so they stay readable
/// under `NO_COLOR`.
pub(super) fn filter_chips(
    title: Option<&str>,
    status: Option<crate::domain::task::Status>,
    priority: Option<crate::domain::task::Priority>,
    sort_arrow: Option<&str>,
) -> Line<'static> {
    let theme = theme::theme();
    let mut spans = vec![Span::styled("Tasks", theme.text_style())];

    let mut chips = Vec::new();
    if let Some(title) = title {
        chips.push(("Title", title.to_string()));
    }
    if let Some(status) = status {
        chips.push(("Status", status.label().to_string()));
    }
    if let Some(priority) = priority {
        chips.push(("Priority", priority.label().to_string()));
    }
    let chip_style = theme.accent_style().add_modifier(Modifier::BOLD);
    for (label, value) in &chips {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("[{label}: {value}]"), chip_style));
    }
    if !chips.is_empty() {
        spans.push(Span::styled("  x clear", theme.muted_style()));
    }
    if let Some(arrow) = sort_arrow {
        spans.push(Span::styled(format!(" · ID {arrow}"), theme.muted_style()));
    }
    Line::from(spans)
}
