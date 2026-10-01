// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared form-field renderers used by the task/project forms and the task
//! list filter popup. Both helpers draw a `Borders::ALL` block titled after
//! the field and render the current value inside it; they never move the
//! cursor (callers own cursor placement).

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Renders an editable text field: red border on `error`, bold border when
/// `active` (error takes precedence), plain otherwise.
pub fn render_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    active: bool,
    error: bool,
) {
    let style = if error {
        Style::default().fg(Color::Red)
    } else if active {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);
    frame.render_widget(
        Paragraph::new(value.to_string())
            .block(block)
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Renders a value the user cycles with arrow keys; the title advertises the
/// `(←/→)` hint and the border is bold while the field is active.
pub fn render_readonly_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    active: bool,
) {
    let style = if active {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(format!("{} (←/→)", title))
        .borders(Borders::ALL)
        .border_style(style);
    frame.render_widget(Paragraph::new(value.to_string()).block(block), area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};
    use ratatui::{Frame, Terminal, layout::Rect};

    use crate::screens::Component;
    use crate::screens::test_utils::render_to_string;

    use super::{render_field, render_readonly_field};

    /// Adapts the free field-renderer functions to the `Component` interface
    /// so they can be rendered through the shared `test_utils` helpers.
    struct FieldStub {
        title: &'static str,
        value: &'static str,
        active: bool,
        kind: FieldKind,
    }

    #[derive(Clone, Copy)]
    enum FieldKind {
        Editable { error: bool },
        Readonly,
    }

    impl FieldStub {
        fn editable(title: &'static str, value: &'static str, active: bool, error: bool) -> Self {
            Self {
                title,
                value,
                active,
                kind: FieldKind::Editable { error },
            }
        }

        fn readonly(title: &'static str, value: &'static str, active: bool) -> Self {
            Self {
                title,
                value,
                active,
                kind: FieldKind::Readonly,
            }
        }
    }

    impl Component for FieldStub {
        fn render(&self, frame: &mut Frame, area: Rect) {
            match self.kind {
                FieldKind::Editable { error } => {
                    render_field(frame, area, self.title, self.value, self.active, error);
                }
                FieldKind::Readonly => {
                    render_readonly_field(frame, area, self.title, self.value, self.active);
                }
            }
        }
    }

    fn render_to_buffer(target: &impl Component, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| target.render(frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn render_field_inactive_renders_exact_bordered_value() {
        let stub = FieldStub::editable("Name", "Alpha", false, false);

        let output = render_to_string(&stub, 20, 5);

        let expected = format!(
            "┌Name{}┐\n│Alpha{}│\n│{}│\n│{}│\n└{}┘",
            "─".repeat(14),
            " ".repeat(13),
            " ".repeat(18),
            " ".repeat(18),
            "─".repeat(18),
        );
        assert_eq!(output, expected);
    }

    #[test]
    fn render_field_inactive_applies_no_style() {
        let stub = FieldStub::editable("Name", "Alpha", false, false);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert_eq!(buffer[(0, 0)].fg, Color::Reset);
        assert!(!buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_field_active_bolds_border_and_title_but_not_value() {
        let stub = FieldStub::editable("Name", "Alpha", true, false);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(0, 0)].fg, Color::Reset);
    }

    #[test]
    fn render_field_error_takes_precedence_over_active_style() {
        let stub = FieldStub::editable("Name", "Alpha", true, true);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert_eq!(buffer[(0, 0)].fg, Color::Red);
        assert!(!buffer[(0, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_readonly_field_renders_exact_bordered_value_with_arrow_hint() {
        let stub = FieldStub::readonly("Status", "Todo", false);

        let output = render_to_string(&stub, 20, 3);

        let expected = format!(
            "┌Status (←/→){}┐\n│Todo{}│\n└{}┘",
            "─".repeat(6),
            " ".repeat(14),
            "─".repeat(18),
        );
        assert_eq!(output, expected);
    }

    #[test]
    fn render_readonly_field_active_bolds_border() {
        let stub = FieldStub::readonly("Status", "Todo", true);

        let buffer = render_to_buffer(&stub, 20, 3);

        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 1)].modifier.contains(Modifier::BOLD));
    }
}
