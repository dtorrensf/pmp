// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared form-field renderers used by the task/project forms and the task
//! list filter popup. Both helpers draw a `Borders::ALL` block titled after
//! the field and render the current value inside it; they never move the
//! cursor (callers own cursor placement).

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use ratatui_textarea::{TextArea, WrapMode};
use unicode_width::UnicodeWidthStr;

use crate::screens::components::theme;

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
        theme::theme().danger_style()
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

/// Renders a multi-line description `TextArea` inside a `Borders::ALL` block
/// titled `title`, bold border when `active`. The textarea is cloned so the
/// caller's state is untouched, and word-wrapped to the area width (like
/// `Paragraph` with `Wrap`) so long lines stay fully visible.
pub fn render_description_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &TextArea<'_>,
    active: bool,
) {
    let style = if active {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);
    let mut textarea = value.clone();
    textarea.set_block(block);
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    frame.render_widget(&textarea, area);
}

/// Screen column for a cursor placed after `text` inside the `Borders::ALL`
/// field drawn in `area`. Widths use terminal display columns (CJK glyphs take
/// two), and the result is clamped so the cursor never leaves the field.
pub fn field_cursor_x(area: Rect, text: &str) -> u16 {
    let inner = area.width.saturating_sub(2) as usize;
    let used = text.width().min(inner) as u16;
    area.x.saturating_add(1).saturating_add(used)
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
    use ratatui_textarea::TextArea;

    use crate::screens::Component;
    use crate::screens::components::theme;
    use crate::screens::test_utils::render_to_string;

    use super::{field_cursor_x, render_description_field, render_field, render_readonly_field};

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
        Description,
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

        fn description(title: &'static str, value: &'static str, active: bool) -> Self {
            Self {
                title,
                value,
                active,
                kind: FieldKind::Description,
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
                FieldKind::Description => {
                    let textarea = TextArea::from(self.value.lines());
                    render_description_field(frame, area, self.title, &textarea, self.active);
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
    fn field_cursor_counts_display_columns_and_clamps() {
        let area = Rect::new(2, 1, 10, 3);
        // Ascii: after 3 chars -> x + 1 border + 3.
        assert_eq!(field_cursor_x(area, "abc"), 6);
        // CJK: two chars occupy four columns.
        assert_eq!(field_cursor_x(area, "中文"), 2 + 1 + 4);
        // Longer than the inner width (8): clamped to the right border.
        assert_eq!(field_cursor_x(area, &"x".repeat(50)), 2 + 1 + 8);
        // Degenerate width never underflows.
        assert_eq!(field_cursor_x(Rect::new(0, 0, 0, 0), "abc"), 1);
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

        assert_eq!(buffer[(0, 0)].fg, theme::theme().danger);
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

    #[test]
    fn render_description_field_wraps_long_line_to_width() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
        let stub = FieldStub::description("Description", text, false);

        let output = render_to_string(&stub, 20, 8);
        let lines: Vec<&str> = output.split('\n').collect();

        // Inner rows (index 1..) must hold consecutive wrapped fragments of the
        // same line, not a single clipped row followed by empty rows.
        assert!(lines[1].contains("alpha"), "row 1: {:?}", lines[1]);
        assert!(
            lines[2].contains('│') && !lines[2].trim_matches(['│', ' ']).is_empty(),
            "row 2 should hold wrapped text: {:?}",
            lines[2]
        );
        assert!(
            !lines[3].trim_matches(['│', ' ']).is_empty(),
            "row 3 should hold wrapped text: {:?}",
            lines[3]
        );
        assert!(
            output.contains("lambda"),
            "last word must be visible:\n{output}"
        );
    }

    #[test]
    fn render_description_field_active_bolds_border() {
        let stub = FieldStub::description("Description", "Alpha", true);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(0, 0)].fg, Color::Reset);
    }

    #[test]
    fn render_description_field_inactive_applies_no_bold_border() {
        let stub = FieldStub::description("Description", "Alpha", false);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert!(!buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 0)].modifier.contains(Modifier::BOLD));
    }
}
