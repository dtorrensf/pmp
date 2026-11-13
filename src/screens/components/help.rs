// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The global shortcut overlay opened with `?`.
//!
//! Shortcuts are grouped into fixed sections so the full map of keys can be
//! read without leaving the screen. Each screen decides the section of its
//! shortcuts through [`sections`]; the overlay itself only lays them out.

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::screens::components::popup::{centered_rect, dim_area, render_popup_scaffold};
use crate::screens::components::theme;
use crate::screens::keybinding_footer::group_keys;

pub const NAVIGATION: &str = "Navegación";
pub const PROJECTS: &str = "Proyectos";
pub const TASKS: &str = "Tareas";
pub const VERSIONS: &str = "Versiones";
pub const APPLICATION: &str = "Aplicación";

/// Display order of the sections.
const SECTION_ORDER: [&str; 5] = [NAVIGATION, PROJECTS, TASKS, VERSIONS, APPLICATION];

/// Widest overlay, in terminal columns.
const MAX_WIDTH: u16 = 76;
/// Narrowest width of one column; below twice this the overlay uses one column.
const COLUMN_MIN_WIDTH: u16 = 26;
const CLOSE_HINT: &str = "? o Esc: cerrar";

/// One shortcut row: the joined key names and the action label.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpEntry {
    pub keys: String,
    pub label: &'static str,
}

/// A titled group of shortcuts.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpSection {
    pub title: &'static str,
    pub entries: Vec<HelpEntry>,
}

/// Builds the overlay sections from a keybinding table. `section_of` maps each
/// action label to one of the section titles above. Empty sections are
/// dropped; the rest keep [`SECTION_ORDER`].
pub fn sections(
    keys: &[(KeyEvent, &'static str)],
    section_of: impl Fn(&str) -> &'static str,
) -> Vec<HelpSection> {
    let mut out: Vec<HelpSection> = SECTION_ORDER
        .iter()
        .map(|title| HelpSection {
            title,
            entries: Vec::new(),
        })
        .collect();
    for (events, label) in group_keys(keys) {
        let title = section_of(label);
        let index = SECTION_ORDER
            .iter()
            .position(|t| *t == title)
            .unwrap_or(SECTION_ORDER.len() - 1);
        out[index].entries.push(HelpEntry {
            keys: events.join("/"),
            label,
        });
    }
    out.retain(|section| !section.entries.is_empty());
    out
}

/// Draws the overlay over `area`, dimming what is behind it.
pub fn render_overlay(frame: &mut Frame, area: Rect, sections: &[HelpSection]) {
    dim_area(frame);

    let width = MAX_WIDTH.min(area.width);
    let columns = if width.saturating_sub(2) >= 2 * COLUMN_MIN_WIDTH {
        2
    } else {
        1
    };
    let groups = balanced_split(sections, columns);
    let content_height = groups.iter().map(|g| group_height(g)).max().unwrap_or(0);
    // Border (2) plus the close hint line.
    let height = (content_height + 3).min(area.height);
    let popup = centered_rect(width, height, area);
    let inner = render_popup_scaffold(frame, popup, "Atajos");

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(CLOSE_HINT).style(theme::theme().muted_style()),
        rows[1],
    );

    let key_width = sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .map(|e| e.keys.width())
        .max()
        .unwrap_or(0)
        + 2;
    let column_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, columns as u32); columns])
        .split(rows[0]);
    for (group, column) in groups.iter().zip(column_areas.iter()) {
        let lines: Vec<Line<'static>> = group
            .iter()
            .flat_map(|section| section_lines(section, key_width))
            .collect();
        frame.render_widget(Paragraph::new(lines), *column);
    }
}

/// Splits the sections into `columns` consecutive groups whose heights are as
/// balanced as possible, keeping the display order.
fn balanced_split(sections: &[HelpSection], columns: usize) -> Vec<&[HelpSection]> {
    if columns < 2 || sections.len() < 2 {
        return vec![sections];
    }
    let split = (0..=sections.len())
        .min_by_key(|&k| {
            let left = group_height(&sections[..k]);
            let right = group_height(&sections[k..]);
            left.max(right)
        })
        .unwrap_or(sections.len());
    vec![&sections[..split], &sections[split..]]
}

/// Lines a group of sections occupies, without the trailing blank line.
fn group_height(sections: &[HelpSection]) -> u16 {
    let total: usize = sections.iter().map(|s| s.entries.len() + 2).sum();
    total.saturating_sub(1) as u16
}

/// A section title, its shortcuts aligned on the key column, and a blank line.
fn section_lines(section: &HelpSection, key_width: usize) -> Vec<Line<'static>> {
    let theme = theme::theme();
    let mut lines = vec![Line::from(Span::styled(
        section.title,
        theme.accent_style().add_modifier(Modifier::BOLD),
    ))];
    for entry in &section.entries {
        let padding = " ".repeat(key_width.saturating_sub(entry.keys.width()));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", entry.keys),
                theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::raw(padding),
            Span::styled(entry.label, theme.text_style()),
        ]));
    }
    lines.push(Line::raw(""));
    lines
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;
    use crate::screens::components::keys::key;

    const TABLE: &[(KeyEvent, &str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('c')), "New"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    fn section_for(label: &str) -> &'static str {
        match label {
            "Down" => NAVIGATION,
            "New" | "Delete" => TASKS,
            _ => APPLICATION,
        }
    }

    #[test]
    fn sections_group_actions_in_display_order_and_drop_empty_ones() {
        let result = sections(TABLE, section_for);
        let titles: Vec<&str> = result.iter().map(|s| s.title).collect();
        assert_eq!(titles, vec![NAVIGATION, TASKS, APPLICATION]);
        let tasks = &result[1];
        assert_eq!(tasks.entries.len(), 2);
        assert_eq!(tasks.entries[0].label, "New");
    }

    #[test]
    fn sections_merge_keys_sharing_a_label() {
        let keys = &[
            (key(KeyCode::Char('q')), "Quit"),
            (key(KeyCode::Esc), "Quit"),
        ];
        let result = sections(keys, |_| APPLICATION);
        assert_eq!(result[0].entries[0].keys, "q/Esc");
    }

    #[test]
    fn unknown_section_falls_back_to_application() {
        let result = sections(TABLE, |_| "Desconocida");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, APPLICATION);
    }

    #[test]
    fn balanced_split_keeps_order_and_balances_heights() {
        let make = |title: &'static str, n: usize| HelpSection {
            title,
            entries: (0..n)
                .map(|_| HelpEntry {
                    keys: "x".into(),
                    label: "y",
                })
                .collect(),
        };
        let sections = vec![make("a", 6), make("b", 1), make("c", 1), make("d", 1)];
        let groups = balanced_split(&sections, 2);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1);
        assert_eq!(groups[1].len(), 3);
    }

    #[test]
    fn overlay_renders_titles_keys_and_close_hint_at_80_cols() {
        use ratatui::{Terminal, backend::TestBackend};

        let sections = sections(TABLE, section_for);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_overlay(frame, frame.area(), &sections))
            .unwrap();
        let text = crate::screens::test_utils::buffer_to_string(terminal.backend().buffer());

        assert!(text.contains("Atajos"));
        assert!(text.contains("Navegación"));
        assert!(text.contains("Tareas"));
        assert!(text.contains("j"));
        assert!(text.contains("Delete"));
        assert!(text.contains("? o Esc: cerrar"));
    }

    #[test]
    fn overlay_never_panics_on_tiny_areas() {
        use ratatui::{Terminal, backend::TestBackend};

        let sections = sections(TABLE, section_for);
        for (w, h) in [(0, 0), (1, 1), (10, 3), (30, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| render_overlay(frame, frame.area(), &sections))
                .unwrap();
        }
    }

    #[test]
    fn single_column_keeps_every_section() {
        let result = sections(TABLE, section_for);
        let groups = balanced_split(&result, 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), result.len());
    }
}
