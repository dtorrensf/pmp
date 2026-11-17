// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shared "nothing here yet" placeholder.
//!
//! Every screen with an empty list renders the same centered block: an
//! icon, a message and, when there is one, the shortcut that fills the
//! list. Sharing it keeps the tone of the app consistent.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::screens::components::theme;

/// A centered empty-state block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    icon: Option<String>,
    message: String,
    hint: Option<String>,
    message_style: Style,
}

impl EmptyState {
    /// An empty state with just a message.
    pub fn new(message: impl Into<String>) -> EmptyState {
        EmptyState {
            icon: None,
            message: message.into(),
            hint: None,
            message_style: Style::default(),
        }
    }

    /// Adds the icon shown above the message.
    pub fn with_icon(mut self, icon: impl Into<String>) -> EmptyState {
        self.icon = Some(icon.into());
        self
    }

    /// Adds the hint shown below the message, usually a shortcut.
    pub fn with_hint(mut self, hint: impl Into<String>) -> EmptyState {
        self.hint = Some(hint.into());
        self
    }

    /// Overrides the style of the message line.
    pub fn with_style(mut self, style: Style) -> EmptyState {
        self.message_style = style;
        self
    }

    /// The lines of the block, in order.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(icon) = &self.icon {
            lines.push(Line::from(Span::styled(
                icon.clone(),
                theme::theme().muted_style(),
            )));
        }
        let message_style = if self.message_style.fg.is_some() {
            self.message_style
        } else {
            theme::theme().text_style()
        };
        lines.push(Line::from(Span::styled(
            self.message.clone(),
            message_style,
        )));
        if let Some(hint) = &self.hint {
            lines.push(Line::from(Span::styled(
                hint.clone(),
                theme::theme().muted_style(),
            )));
        }
        lines
    }

    /// Renders the block centered in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lines = self.lines();
        let paragraph = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
        frame.render_widget(paragraph, area);
    }
}

/// An empty state styled as a dimmed hint, used for secondary panels such as
/// the notes list of a task.
pub fn hint_empty_state(message: impl Into<String>) -> EmptyState {
    EmptyState::new(message).with_style(theme::theme().muted_style())
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::screens::test_utils::buffer_to_string;

    use super::*;

    #[test]
    fn message_only_block_has_a_single_line() {
        let empty = EmptyState::new("No tasks yet");

        assert_eq!(empty.lines().len(), 1);
        assert_eq!(empty.lines()[0].spans[0].content, "No tasks yet");
    }

    #[test]
    fn icon_and_hint_add_lines_around_the_message() {
        let empty = EmptyState::new("No tasks yet")
            .with_icon("○")
            .with_hint("Press c to create one.");

        let lines = empty.lines();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content, "○");
        assert_eq!(lines[1].spans[0].content, "No tasks yet");
        assert_eq!(lines[2].spans[0].content, "Press c to create one.");
    }

    #[test]
    fn message_is_centred_in_the_area() {
        let empty = EmptyState::new("No tasks yet");
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| empty.render(frame, frame.area()))
            .unwrap();

        let output = buffer_to_string(terminal.backend().buffer());
        let line = output.lines().next().unwrap();

        // "No tasks yet" is 11 columns in a 20-column area.
        assert_eq!(line.trim_start(), "No tasks yet");
        assert_eq!(line.chars().take(4).collect::<String>(), "    ");
    }

    #[test]
    fn hint_style_uses_the_muted_color() {
        let empty = hint_empty_state("No notes yet");

        assert_eq!(
            empty.lines()[0].spans[0].style.fg,
            Some(theme::theme().muted)
        );
    }

    #[test]
    fn icon_follows_the_glyph_set_in_use() {
        use crate::screens::components::glyphs::{self, Glyphs};

        let empty = EmptyState::new("No tasks yet").with_icon(glyphs::glyphs().status_todo);
        assert_eq!(empty.lines()[0].spans[0].content, "○");

        let empty = glyphs::with_glyphs(Glyphs::ascii(), || {
            EmptyState::new("No tasks yet").with_icon(glyphs::glyphs().status_todo)
        });
        assert_eq!(empty.lines()[0].spans[0].content, "o");
    }

    #[test]
    fn default_message_style_comes_from_the_theme() {
        let empty = EmptyState::new("No tasks yet");

        assert_eq!(
            empty.lines()[0].spans[0].style.fg,
            Some(theme::theme().text)
        );
    }
}
