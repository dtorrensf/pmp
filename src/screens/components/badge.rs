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
use crate::screens::UiContext;

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
    pub fn status(status: Status, ui: &UiContext) -> Badge {
        Badge::new(status.label())
            .with_glyph(ui.status_icon(status))
            .with_style(ui.status_style(status))
    }

    /// Like [`Badge::status`], but a task in progress shows the animated
    /// spinner in place of its static icon.
    pub fn status_live(status: Status, ui: &UiContext) -> Badge {
        match status {
            Status::InProgress => Badge::status(status, ui).with_glyph(ui.spinner_glyph()),
            _ => Badge::status(status, ui),
        }
    }

    /// The badge of a task priority: its glyph, its label and its theme color.
    pub fn priority(priority: Priority, ui: &UiContext) -> Badge {
        Badge::new(priority.label())
            .with_glyph(ui.priority_icon(priority))
            .with_style(ui.priority_style(priority))
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
    use crate::screens::components::glyphs::Glyphs;

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
        let ui = UiContext::default();
        let badge = Badge::status(Status::InProgress, &ui);

        assert_eq!(badge.text(), "◐ In Progress");
        assert_eq!(badge.style.fg, Some(ui.theme().status_in_progress));
    }

    #[test]
    fn status_badge_uses_the_glyph_set() {
        let ui = UiContext::default();
        assert_eq!(Badge::status(Status::Todo, &ui).text(), "○ Todo");
        assert_eq!(Badge::status(Status::Done, &ui).text(), "✔ Done");
    }

    #[test]
    fn priority_badge_combines_glyph_label_and_color() {
        let ui = UiContext::default();
        let badge = Badge::priority(Priority::High, &ui);

        assert_eq!(badge.text(), "▲ High");
        assert_eq!(badge.style.fg, Some(ui.theme().priority_high));
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
        let ui = UiContext::default().with_glyphs(Glyphs::ascii());

        let badge = Badge::status(Status::Done, &ui);

        assert_eq!(badge.text(), "x Done");
    }
}
