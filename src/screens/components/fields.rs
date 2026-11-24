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
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use ratatui_textarea::{TextArea, WrapMode};
use unicode_width::UnicodeWidthStr;

use crate::screens::UiContext;

/// Columns taken by the borders plus the horizontal padding on each side.
const FIELD_CHROME: u16 = 4;

/// The block every field is drawn in. The focused field gets the focus color
/// and bold (bold keeps the cue under `NO_COLOR`); a validation `error` turns
/// the border red and is written on the bottom border, next to the field.
fn field_block(title: &str, active: bool, error: Option<&str>, ui: &UiContext) -> Block<'static> {
    let (border, title_style) = if error.is_some() {
        (ui.danger_style(), ui.danger_style())
    } else if active {
        let focus = ui.border_focus_style().add_modifier(Modifier::BOLD);
        (focus, ui.accent_style().add_modifier(Modifier::BOLD))
    } else {
        (Style::default(), Style::default())
    };
    let mut block = Block::default()
        .title(Line::from(Span::styled(title.to_string(), title_style)))
        .borders(Borders::ALL)
        .border_style(border)
        .padding(Padding::horizontal(1));
    if let Some(error) = error {
        let message = format!(" {} {error} ", ui.glyphs().error);
        block = block.title_bottom(Line::from(Span::styled(message, ui.danger_style())));
    }
    block
}

/// Renders an editable text field. `error` is the validation message to show
/// next to the field, if the field is invalid.
pub fn render_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    active: bool,
    error: Option<&str>,
    ui: &UiContext,
) {
    let block = field_block(title, active, error, ui);
    frame.render_widget(
        Paragraph::new(value.to_string())
            .block(block)
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Renders a multi-line description `TextArea` inside the shared field block,
/// focused when `active`. The textarea is cloned so the caller's state is
/// untouched, and word-wrapped to the area width (like `Paragraph` with
/// `Wrap`) so long lines stay fully visible.
pub fn render_description_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &TextArea<'_>,
    active: bool,
    ui: &UiContext,
) {
    let block = field_block(title, active, None, ui);
    let mut textarea = value.clone();
    textarea.set_block(block);
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    frame.render_widget(&textarea, area);
}

/// Screen column for a cursor placed after `text` inside the field drawn in
/// `area` (borders and padding excluded). Widths use terminal display columns
/// (CJK glyphs take two), and the result is clamped so the cursor never leaves
/// the field.
pub fn field_cursor_x(area: Rect, text: &str) -> u16 {
    let inner = area.width.saturating_sub(FIELD_CHROME) as usize;
    let used = text.width().min(inner) as u16;
    area.x.saturating_add(FIELD_CHROME / 2).saturating_add(used)
}

/// Renders a value the user cycles with arrow keys; the title advertises the
/// `(←/→)` hint and the field is focused while `active`.
pub fn render_readonly_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    active: bool,
    ui: &UiContext,
) {
    let block = field_block(&format!("{title} (←/→)"), active, None, ui);
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
    use crate::screens::UiContext;
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
        fn render(&self, frame: &mut Frame, area: Rect, ui: &UiContext) {
            match self.kind {
                FieldKind::Editable { error } => {
                    let message = error.then_some("Name is required");
                    render_field(
                        frame,
                        area,
                        self.title,
                        self.value,
                        self.active,
                        message,
                        ui,
                    );
                }
                FieldKind::Readonly => {
                    render_readonly_field(frame, area, self.title, self.value, self.active, ui);
                }
                FieldKind::Description => {
                    let textarea = TextArea::from(self.value.lines());
                    render_description_field(frame, area, self.title, &textarea, self.active, ui);
                }
            }
        }
    }

    fn render_to_buffer(target: &impl Component, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| target.render(frame, frame.area(), &UiContext::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn field_cursor_counts_display_columns_and_clamps() {
        let area = Rect::new(2, 1, 10, 3);
        // Ascii: after 3 chars -> x + border + padding + 3.
        assert_eq!(field_cursor_x(area, "abc"), 2 + 2 + 3);
        // CJK: two chars occupy four columns.
        assert_eq!(field_cursor_x(area, "中文"), 2 + 2 + 4);
        // Longer than the inner width (10 - 4): clamped to the right padding.
        assert_eq!(field_cursor_x(area, &"x".repeat(50)), 2 + 2 + 6);
        // Degenerate width never underflows.
        assert_eq!(field_cursor_x(Rect::new(0, 0, 0, 0), "abc"), 2);
    }

    #[test]
    fn render_field_inactive_renders_exact_bordered_value() {
        let stub = FieldStub::editable("Name", "Alpha", false, false);

        let output = render_to_string(&stub, 20, 5);

        let expected = format!(
            "┌Name{}┐\n│ Alpha{}│\n│{}│\n│{}│\n└{}┘",
            "─".repeat(14),
            " ".repeat(12),
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
    fn render_field_active_uses_focus_color_and_bold_border() {
        let stub = FieldStub::editable("Name", "Alpha", true, false);

        let buffer = render_to_buffer(&stub, 20, 5);

        // Focus is shown by color and by bold, so it survives NO_COLOR.
        assert_eq!(buffer[(0, 0)].fg, UiContext::default().theme().border_focus);
        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        // The title takes the accent color; the value keeps the default style.
        assert_eq!(buffer[(1, 0)].fg, UiContext::default().theme().accent);
        assert!(!buffer[(2, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_field_error_takes_precedence_over_active_style() {
        let stub = FieldStub::editable("Name", "Alpha", true, true);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert_eq!(buffer[(0, 0)].fg, UiContext::default().theme().danger);
        assert!(!buffer[(0, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_field_error_is_written_on_the_bottom_border() {
        let stub = FieldStub::editable("Name", "", false, true);

        let output = render_to_string(&stub, 30, 3);
        let bottom = output.lines().last().unwrap();

        assert!(bottom.contains("Name is required"), "{output}");
    }

    #[test]
    fn render_readonly_field_renders_exact_bordered_value_with_arrow_hint() {
        let stub = FieldStub::readonly("Status", "Todo", false);

        let output = render_to_string(&stub, 20, 3);

        let expected = format!(
            "┌Status (←/→){}┐\n│ Todo{}│\n└{}┘",
            "─".repeat(6),
            " ".repeat(13),
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
    fn render_description_field_active_uses_focus_border() {
        let stub = FieldStub::description("Description", "Alpha", true);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert_eq!(buffer[(0, 0)].fg, UiContext::default().theme().border_focus);
        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_description_field_inactive_applies_no_bold_border() {
        let stub = FieldStub::description("Description", "Alpha", false);

        let buffer = render_to_buffer(&stub, 20, 5);

        assert!(!buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 0)].modifier.contains(Modifier::BOLD));
    }
}
