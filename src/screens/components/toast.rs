// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transient feedback messages drawn just above the footer.
//!
//! Success and info messages disappear after [`AUTO_DISMISS_AFTER`]. Errors
//! stay until the next action or until the user closes them with `Esc`.
//! Messages wait in a queue and only the oldest one is shown at a time.
//!
//! The queue never reads the clock itself: callers pass the current
//! [`Instant`], which keeps auto-dismissal deterministic in tests.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::screens::components::glyphs;
use crate::screens::components::scroll::wrapped_line_count;
use crate::screens::components::theme;

/// How long success and info messages stay on screen.
pub const AUTO_DISMISS_AFTER: Duration = Duration::from_secs(3);
/// Upper bound on queued messages; the oldest one is dropped when exceeded.
const MAX_QUEUED: usize = 5;
/// Widest toast, in terminal columns. Longer messages wrap.
const MAX_WIDTH: u16 = 60;
/// Border (2) plus the icon and its trailing space.
const CHROME_WIDTH: u16 = 2 + 2;

/// How a message is styled and whether it expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Success,
    Error,
}

impl Severity {
    /// `true` when the message expires on its own.
    fn auto_dismisses(self) -> bool {
        !matches!(self, Severity::Error)
    }

    fn title(self) -> &'static str {
        match self {
            Severity::Info => "Info",
            Severity::Success => "Done",
            Severity::Error => "Error",
        }
    }

    fn icon(self) -> &'static str {
        let glyphs = glyphs::glyphs();
        match self {
            Severity::Info => glyphs.info,
            Severity::Success => glyphs.success,
            Severity::Error => glyphs.error,
        }
    }

    fn style(self) -> Style {
        let theme = theme::theme();
        match self {
            Severity::Info => theme.accent_style(),
            Severity::Success => theme.success_style(),
            Severity::Error => theme.danger_style(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub severity: Severity,
    pub message: String,
    created_at: Instant,
}

impl Toast {
    fn expired(&self, now: Instant) -> bool {
        self.severity.auto_dismisses()
            && now.saturating_duration_since(self.created_at) >= AUTO_DISMISS_AFTER
    }
}

/// FIFO queue of toasts. The front one is the one on screen.
#[derive(Debug, Default)]
pub struct Toasts {
    queue: VecDeque<Toast>,
}

impl Toasts {
    /// Queues a message. An identical message already queued (same severity
    /// and text) is not duplicated; its timer restarts instead. This keeps a
    /// failure that repeats on every auto-refresh from piling up.
    pub fn push(&mut self, severity: Severity, message: impl Into<String>, now: Instant) {
        let message = message.into();
        if let Some(existing) = self
            .queue
            .iter_mut()
            .find(|t| t.severity == severity && t.message == message)
        {
            existing.created_at = now;
            return;
        }
        if self.queue.len() >= MAX_QUEUED {
            self.queue.pop_front();
        }
        self.queue.push_back(Toast {
            severity,
            message,
            created_at: now,
        });
    }

    /// Drops the auto-dismissing messages whose time is up.
    pub fn tick(&mut self, now: Instant) {
        self.queue.retain(|toast| !toast.expired(now));
    }

    /// The message currently on screen.
    pub fn current(&self) -> Option<&Toast> {
        self.queue.front()
    }

    /// Closes the message on screen, if any.
    pub fn dismiss_current(&mut self) -> Option<Toast> {
        self.queue.pop_front()
    }

    /// Removes every queued error. Called when the user performs a new action.
    pub fn clear_errors(&mut self) {
        self.queue.retain(|t| t.severity != Severity::Error);
    }

    /// Text of the most recent error still queued.
    pub fn error(&self) -> Option<&str> {
        self.queue
            .iter()
            .rev()
            .find(|t| t.severity == Severity::Error)
            .map(|t| t.message.as_str())
    }

    /// Every queued message, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.queue.iter()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Draws the toast for `toast` right-aligned at the bottom of `area`, which
/// should be the content area directly above the footer.
pub fn render(frame: &mut Frame, toast: &Toast, queued: usize, area: Rect) {
    let width = MAX_WIDTH.min(area.width);
    let text_width = width.saturating_sub(CHROME_WIDTH).max(1);
    let lines = wrapped_line_count(&toast.message, text_width).max(1) as u16;
    let height = (lines + 2).min(area.height);
    if height == 0 || width <= CHROME_WIDTH {
        return;
    }
    let rect = Rect {
        x: area.right() - width,
        y: area.bottom() - height,
        width,
        height,
    };

    let style = toast.severity.style();
    let mut title = format!(" {} {}", toast.severity.icon(), toast.severity.title());
    if queued > 1 {
        title.push_str(&format!(" (1/{queued})"));
    }
    title.push(' ');
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            style.add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(style);

    frame.render_widget(Clear, rect);
    let paragraph = Paragraph::new(toast.message.as_str())
        .style(theme::theme().text_style())
        .block(block)
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Instant {
        Instant::now()
    }

    #[test]
    fn success_and_info_expire_after_three_seconds() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Success, "Task saved", t0);
        toasts.push(Severity::Info, "Hint", t0);

        toasts.tick(t0 + AUTO_DISMISS_AFTER - Duration::from_millis(1));
        assert_eq!(toasts.len(), 2);

        toasts.tick(t0 + AUTO_DISMISS_AFTER);
        assert!(toasts.is_empty());
    }

    #[test]
    fn errors_never_expire_on_their_own() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Error, "boom", t0);

        toasts.tick(t0 + Duration::from_secs(3600));

        assert_eq!(toasts.error(), Some("boom"));
    }

    #[test]
    fn queue_shows_the_oldest_first_and_dismisses_in_order() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Error, "first", t0);
        toasts.push(Severity::Success, "second", t0);

        assert_eq!(toasts.current().unwrap().message, "first");
        assert_eq!(toasts.dismiss_current().unwrap().message, "first");
        assert_eq!(toasts.current().unwrap().message, "second");
    }

    #[test]
    fn clear_errors_keeps_non_error_messages() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Error, "boom", t0);
        toasts.push(Severity::Success, "saved", t0);

        toasts.clear_errors();

        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts.error(), None);
        assert_eq!(toasts.current().unwrap().message, "saved");
    }

    #[test]
    fn identical_messages_are_not_duplicated_and_restart_their_timer() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Success, "saved", t0);
        let later = t0 + Duration::from_secs(2);
        toasts.push(Severity::Success, "saved", later);
        assert_eq!(toasts.len(), 1);

        toasts.tick(t0 + AUTO_DISMISS_AFTER);
        assert_eq!(toasts.len(), 1, "timer should restart from the second push");
        toasts.tick(later + AUTO_DISMISS_AFTER);
        assert!(toasts.is_empty());
    }

    #[test]
    fn queue_drops_the_oldest_past_its_limit() {
        let t0 = base();
        let mut toasts = Toasts::default();
        for i in 0..=MAX_QUEUED {
            toasts.push(Severity::Error, format!("e{i}"), t0);
        }
        assert_eq!(toasts.len(), MAX_QUEUED);
        assert_eq!(toasts.current().unwrap().message, "e1");
    }

    #[test]
    fn error_returns_the_most_recent_one() {
        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(Severity::Error, "old", t0);
        toasts.push(Severity::Error, "new", t0);
        assert_eq!(toasts.error(), Some("new"));
    }

    #[test]
    fn render_never_panics_on_tiny_areas_and_wraps_long_messages() {
        use ratatui::{Terminal, backend::TestBackend};

        let t0 = base();
        let mut toasts = Toasts::default();
        toasts.push(
            Severity::Error,
            "a very long error message that has to wrap over several lines to fit",
            t0,
        );
        let toast = toasts.current().unwrap();
        for (w, h) in [(0, 0), (1, 1), (4, 3), (12, 4), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| render(frame, toast, 1, frame.area()))
                .unwrap();
        }
    }
}
