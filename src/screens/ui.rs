// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The explicit UI context every render receives.
//!
//! Before 1.0.0 the theme, the glyph set and the frame clock were resolved
//! through process-wide `OnceLock`s and thread-local overrides. That hid the
//! dependencies of each render, made isolated scenarios impossible to run in
//! parallel and coupled the tests to the environment of the process.
//!
//! [`UiContext`] replaces all of it with one value resolved exactly once in the
//! composition root ([`crate::tui::run`]) and passed down explicitly: the event
//! loop builds one per frame (adding the frame time), and every screen and
//! helper names it in its signature. Tests build the context they want, so no
//! global state leaks between them.
//!
//! The context is `Copy`: cloning it around is free, and a context with no
//! frame time renders every animation in its final state, which is what
//! snapshots use.

use std::time::Duration;

use ratatui::style::{Color, Style};

use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

use super::components::glyphs::Glyphs;
use super::components::motion::Motion;
use super::components::theme::Theme;

/// Everything a render needs to look right: palette, glyph set and the motion
/// clock. Built once from the environment, then copied per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiContext {
    theme: Theme,
    glyphs: Glyphs,
    motion: Motion,
}

impl Default for UiContext {
    /// The deterministic context used by snapshots and unit tests: default
    /// palette, Unicode glyphs, no frame time (every animation settled).
    fn default() -> Self {
        Self::new(Theme::default_theme(), Glyphs::unicode(), false)
    }
}

impl UiContext {
    /// Builds a context from already resolved parts.
    pub const fn new(theme: Theme, glyphs: Glyphs, reduced_motion: bool) -> Self {
        Self {
            theme,
            glyphs,
            motion: Motion::still().with_reduced(reduced_motion),
        }
    }

    /// The same context with another palette.
    pub const fn with_theme(self, theme: Theme) -> Self {
        Self { theme, ..self }
    }

    /// The same context with another glyph set.
    pub const fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
    }

    /// The same context with the reduced-motion preference replaced.
    pub const fn with_reduced_motion(self, reduced: bool) -> Self {
        Self {
            motion: self.motion.with_reduced(reduced),
            ..self
        }
    }

    /// The same context at `frame_time`. The event loop calls this once per
    /// iteration, after measuring the elapsed time since it started.
    pub const fn with_frame_time(self, frame_time: Duration) -> Self {
        Self {
            motion: self.motion.at(frame_time),
            ..self
        }
    }

    /// The palette in use.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The glyph set in use.
    pub fn glyphs(&self) -> Glyphs {
        self.glyphs
    }

    /// The motion clock and preference in use.
    pub fn motion(&self) -> &Motion {
        &self.motion
    }

    /// Time of the frame being drawn, if this context is animating.
    pub fn frame_time(&self) -> Option<Duration> {
        self.motion.frame_time()
    }

    /// `true` when animations play in this frame.
    pub fn animating(&self) -> bool {
        self.motion.animating()
    }

    /// The canonical style of a task status.
    pub fn status_style(&self, status: Status) -> Style {
        self.theme.status_style(status)
    }

    /// The canonical style of a whole row carrying a task status.
    pub fn status_row_style(&self, status: Status) -> Style {
        self.theme.status_row_style(status)
    }

    /// The canonical style of a task priority.
    pub fn priority_style(&self, priority: Priority) -> Style {
        self.theme.priority_style(priority)
    }

    /// The canonical style of a version status.
    pub fn version_status_style(&self, status: VersionStatus) -> Style {
        self.theme.version_status_style(status)
    }

    /// Style of the default text.
    pub fn text_style(&self) -> Style {
        self.theme.text_style()
    }

    /// Style of secondary text.
    pub fn muted_style(&self) -> Style {
        self.theme.muted_style()
    }

    /// Style of the default border.
    pub fn border_style(&self) -> Style {
        self.theme.border_style()
    }

    /// Style of the focused element's border, pulsing while animating.
    pub fn border_focus_style(&self) -> Style {
        self.theme.border_focus_style(&self.motion)
    }

    /// Accent style, pulsing while animating when the color is RGB.
    pub fn pulse_style(&self, color: Color) -> Style {
        self.theme.pulse_style(color, &self.motion)
    }

    /// Style of the accent color.
    pub fn accent_style(&self) -> Style {
        self.theme.accent_style()
    }

    /// Style of successful operations.
    pub fn success_style(&self) -> Style {
        self.theme.success_style()
    }

    /// Style of warnings.
    pub fn warning_style(&self) -> Style {
        self.theme.warning_style()
    }

    /// Style of errors and destructive actions.
    pub fn danger_style(&self) -> Style {
        self.theme.danger_style()
    }

    /// Style of a table header.
    pub fn header_style(&self) -> Style {
        self.theme.header_style()
    }

    /// Style of the selected row, pulsing while animating.
    pub fn selection_style(&self) -> Style {
        self.theme.selection_style(&self.motion)
    }

    /// Icon of a task status.
    pub fn status_icon(&self, status: Status) -> &'static str {
        self.glyphs.status(status)
    }

    /// Icon of a task priority.
    pub fn priority_icon(&self, priority: Priority) -> &'static str {
        self.glyphs.priority(priority)
    }

    /// Icon of a version status.
    pub fn version_status_icon(&self, status: VersionStatus) -> &'static str {
        self.glyphs.version_status(status)
    }

    /// Glyph of the spinner for tasks in progress.
    pub fn spinner_glyph(&self) -> &'static str {
        self.motion.spinner_glyph(self.glyphs)
    }

    /// Progress of a toast entrance (0.0..=1.0).
    pub fn entrance_progress(&self, age: Duration) -> f64 {
        self.motion.entrance_progress(age)
    }

    /// `true` while a celebration started at `since` is still playing.
    pub fn celebrating(&self, since: Option<Duration>) -> bool {
        self.motion.celebrating(since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::Status;
    use crate::screens::components::theme;

    #[test]
    fn default_context_is_deterministic_and_still() {
        let ui = UiContext::default();
        assert!(!ui.animating());
        assert_eq!(ui.frame_time(), None);
        assert_eq!(ui.glyphs(), Glyphs::unicode());
        assert_eq!(ui.theme(), &Theme::default_theme());
    }

    #[test]
    fn from_lookup_resolves_each_source_once() {
        let ui = UiContext::new(
            Theme::from_env_with(|key| (key == theme::NO_COLOR_ENV).then(|| "1".to_string())),
            Glyphs::from_lookup(&|key| (key == "PMP_ASCII").then(|| "1".to_string())),
            true,
        );
        assert!(!ui.theme().colors_enabled);
        assert!(ui.glyphs().is_ascii());
        assert!(!ui.animating());
    }

    #[test]
    fn builders_keep_the_other_parts() {
        let ui = UiContext::default()
            .with_glyphs(Glyphs::ascii())
            .with_reduced_motion(true)
            .with_frame_time(Duration::from_millis(5));
        assert_eq!(ui.glyphs(), Glyphs::ascii());
        assert!(!ui.animating());
        assert_eq!(ui.frame_time(), Some(Duration::from_millis(5)));
        assert_eq!(ui.theme(), &Theme::default_theme());
    }

    #[test]
    fn animating_context_pulses_the_selection() {
        let ui = UiContext::default()
            .with_theme(Theme {
                selection_bg: Color::Rgb(0, 0, 0),
                ..Theme::default_theme()
            })
            .with_frame_time(Duration::from_millis(600));
        assert!(ui.animating());
        assert_ne!(ui.selection_style().bg, Some(Color::Rgb(0, 0, 0)));
    }

    #[test]
    fn glyph_helpers_come_from_the_context() {
        let ui = UiContext::default().with_glyphs(Glyphs::ascii());
        assert_eq!(ui.status_icon(Status::Todo), "o");
        assert_eq!(ui.spinner_glyph(), "|");
    }
}
