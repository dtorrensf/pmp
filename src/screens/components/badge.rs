// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! A small inline chip that labels a value.
//!
//! Badges carry a leading glyph so the value is not communicated by color
//! alone, which keeps the UI readable for colorblind users and in
//! monochrome terminals.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::domain::task::{Priority, Status};
use crate::screens::components::glyphs;
use crate::screens::components::theme;

/// A label with an optional leading glyph and its own style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    label: String,
    glyph: Option<String>,
    style: Style,
}

impl Badge {
    /// A badge showing only `label`.
    pub fn new(label: impl Into<String>) -> Badge {
        Badge {
            label: label.into(),
            glyph: None,
            style: Style::default(),
        }
    }

    /// Adds a leading glyph.
    pub fn with_glyph(mut self, glyph: impl Into<String>) -> Badge {
        self.glyph = Some(glyph.into());
        self
    }

    /// Sets the style of the whole badge.
    pub fn with_style(mut self, style: Style) -> Badge {
        self.style = style;
        self
    }

    /// The badge of a task status: its glyph, its label and its theme color.
    pub fn status(status: Status) -> Badge {
        Badge::new(status.label())
            .with_glyph(glyphs::status_icon(status))
            .with_style(theme::status_style(status))
    }

    /// The badge of a task priority: its glyph, its label and its theme color.
    pub fn priority(priority: Priority) -> Badge {
        Badge::new(priority.label())
            .with_glyph(glyphs::priority_icon(priority))
            .with_style(theme::priority_style(priority))
    }

    /// The plain text of the badge, glyph included.
    pub fn text(&self) -> String {
        match &self.glyph {
            Some(glyph) => format!("{glyph} {}", self.label),
            None => self.label.clone(),
        }
    }

    /// The badge as a styled span.
    pub fn span(&self) -> Span<'static> {
        Span::styled(self.text(), self.style)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use crate::domain::task::{Priority, Status};

    use super::*;

    #[test]
    fn badge_without_glyph_shows_only_the_label() {
        let badge = Badge::new("Todo");

        assert_eq!(badge.text(), "Todo");
    }

    #[test]
    fn badge_with_glyph_prefixes_it() {
        let badge = Badge::new("Todo").with_glyph("○");

        assert_eq!(badge.text(), "○ Todo");
    }

    #[test]
    fn status_badge_combines_glyph_label_and_color() {
        let badge = Badge::status(Status::InProgress);

        assert_eq!(badge.text(), "◐ In Progress");
        assert_eq!(badge.style.fg, Some(theme::theme().status_in_progress));
    }

    #[test]
    fn status_badge_uses_the_glyph_set() {
        assert_eq!(Badge::status(Status::Todo).text(), "○ Todo");
        assert_eq!(Badge::status(Status::Done).text(), "✔ Done");
    }

    #[test]
    fn priority_badge_combines_glyph_label_and_color() {
        let badge = Badge::priority(Priority::High);

        assert_eq!(badge.text(), "▲ High");
        assert_eq!(badge.style.fg, Some(theme::theme().priority_high));
    }

    #[test]
    fn badge_span_carries_the_style() {
        let badge = Badge::new("Todo")
            .with_glyph("○")
            .with_style(Style::default().fg(Color::Cyan));

        let span = badge.span();

        assert_eq!(span.content, "○ Todo");
        assert_eq!(span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn badge_follows_the_glyph_set_in_use() {
        use crate::screens::components::glyphs::{self, Glyphs};

        let badge = glyphs::with_glyphs(Glyphs::ascii(), || Badge::status(Status::Done));

        assert_eq!(badge.text(), "x Done");
    }
}
