// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Semantic visual tokens for the whole application.
//!
//! This module is the single source of truth for the app's aesthetics:
//! every screen asks the theme for a style instead of hardcoding colors.
//! The default palette uses ANSI colors so the terminal's own palette
//! applies, which keeps the app readable on both light and dark terminals.
//!
//! Two environment variables tune it:
//!
//! - `NO_COLOR`: when set to a non-empty value, colors are disabled while
//!   shapes and bold text are kept (see <https://no-color.org>).
//! - `PMP_THEME`: selects one of the built-in palettes (`default`, `bright`).
//!   Unknown names fall back to `default`.
//!
//! The mapping from domain enums (`Status`, `Priority`, `VersionStatus`) to
//! styles lives here, so the domain layer stays free of any `ratatui`
//! dependency.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};

use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

/// Environment variable that disables colors when set to a non-empty value.
pub const NO_COLOR_ENV: &str = "NO_COLOR";

/// Environment variable that selects the built-in palette.
pub const THEME_ENV: &str = "PMP_THEME";

/// Name of the default palette.
pub const DEFAULT_PALETTE: &str = "default";

/// Name of the high-contrast (bright ANSI) palette.
pub const BRIGHT_PALETTE: &str = "bright";

/// The semantic tokens used across the app.
///
/// Colors are always stored as concrete values so they can be inspected and
/// tested; whether they are actually applied is decided by
/// [`Theme::colors_enabled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Colors are applied to the terminal. When `false` the theme only
    /// carries shape information (bold, dim) and no foreground colors.
    pub colors_enabled: bool,
    /// Terminal background, inherited from the terminal by default.
    pub bg: Color,
    /// Background of raised surfaces (popups, cards).
    pub surface: Color,
    /// Color of inactive borders.
    pub border: Color,
    /// Color of the focused element's border.
    pub border_focus: Color,
    /// Default text color.
    pub text: Color,
    /// Secondary text: hints, help lines, disabled content.
    pub muted: Color,
    /// Primary accent: focus, active version, highlights.
    pub accent: Color,
    /// Successful operations and completed work.
    pub success: Color,
    /// Warnings and pending attention.
    pub warning: Color,
    /// Errors and destructive actions.
    pub danger: Color,
    /// Background of the selected row.
    pub selection_bg: Color,
    /// Foreground of the selected row.
    pub selection_fg: Color,
    /// Color of `Status::Todo`.
    pub status_todo: Color,
    /// Color of `Status::InProgress`.
    pub status_in_progress: Color,
    /// Color of `Status::Done`.
    pub status_done: Color,
    /// Color of `Priority::High`.
    pub priority_high: Color,
    /// Color of `Priority::Medium`.
    pub priority_medium: Color,
    /// Color of `Priority::Low`.
    pub priority_low: Color,
    /// Color of `VersionStatus::Backlog`.
    pub version_backlog: Color,
    /// Color of `VersionStatus::Open`.
    pub version_open: Color,
    /// Color of `VersionStatus::Released`.
    pub version_released: Color,
}

impl Theme {
    /// The default palette, with colors enabled.
    pub const fn default_theme() -> Theme {
        Theme {
            colors_enabled: true,
            bg: Color::Reset,
            surface: Color::Reset,
            border: Color::DarkGray,
            border_focus: Color::Cyan,
            text: Color::Reset,
            muted: Color::Gray,
            accent: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            selection_bg: Color::DarkGray,
            selection_fg: Color::Reset,
            status_todo: Color::Cyan,
            status_in_progress: Color::Yellow,
            status_done: Color::Green,
            priority_high: Color::Red,
            priority_medium: Color::Magenta,
            priority_low: Color::Blue,
            version_backlog: Color::Magenta,
            version_open: Color::Blue,
            version_released: Color::Green,
        }
    }

    /// The bright palette: the same semantic mapping using the bright ANSI
    /// variants, for terminals where the regular ones are too dim.
    pub const fn bright() -> Theme {
        Theme {
            colors_enabled: true,
            status_todo: Color::LightCyan,
            status_in_progress: Color::LightYellow,
            status_done: Color::LightGreen,
            priority_high: Color::LightRed,
            priority_medium: Color::LightMagenta,
            priority_low: Color::LightBlue,
            version_backlog: Color::LightMagenta,
            version_open: Color::LightBlue,
            version_released: Color::LightGreen,
            ..Theme::default_theme()
        }
    }

    /// Builds the theme from the given palette name and color preference.
    ///
    /// An unknown `palette` falls back to [`DEFAULT_PALETTE`]. When
    /// `no_color` is `true` the returned theme keeps every token (so the
    /// mapping stays inspectable) but reports that colors must not be
    /// applied.
    pub fn new(palette: Option<&str>, no_color: bool) -> Theme {
        let mut theme = match palette {
            Some(BRIGHT_PALETTE) => Theme::bright(),
            Some(other) if other == DEFAULT_PALETTE => Theme::default_theme(),
            Some(other) => {
                tracing::warn!(palette = other, "unknown palette, using default");
                Theme::default_theme()
            }
            None => Theme::default_theme(),
        };
        theme.colors_enabled = !no_color;
        theme
    }

    /// Builds the theme from the process environment.
    pub fn from_env() -> Theme {
        Theme::from_env_with(|key| std::env::var(key).ok())
    }

    /// Builds the theme from an arbitrary environment lookup.
    ///
    /// Taking the lookup as a parameter keeps the resolution logic testable
    /// without touching the process-wide environment.
    pub fn from_env_with(lookup: impl Fn(&str) -> Option<String>) -> Theme {
        Theme::new(lookup(THEME_ENV).as_deref(), no_color_in(&lookup))
    }

    /// Foreground style for `color`, honoring the color preference.
    fn fg(&self, color: Color) -> Style {
        if self.colors_enabled {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }

    /// Style of the default text.
    pub fn text_style(&self) -> Style {
        self.fg(self.text)
    }

    /// Style of secondary text: hints, help lines and disabled content.
    pub fn muted_style(&self) -> Style {
        self.fg(self.muted)
    }

    /// Style of the default border.
    pub fn border_style(&self) -> Style {
        self.fg(self.border)
    }

    /// Style of the focused element's border.
    pub fn border_focus_style(&self) -> Style {
        self.fg(self.border_focus)
    }

    /// Style of the accent color.
    pub fn accent_style(&self) -> Style {
        self.fg(self.accent)
    }

    /// Style of successful operations.
    pub fn success_style(&self) -> Style {
        self.fg(self.success)
    }

    /// Style of warnings.
    pub fn warning_style(&self) -> Style {
        self.fg(self.warning)
    }

    /// Style of errors and destructive actions.
    pub fn danger_style(&self) -> Style {
        self.fg(self.danger)
    }

    /// Style of a table header: the terminal's own foreground, in bold, so it
    /// stays visible on light backgrounds where a hardcoded white fails.
    pub fn header_style(&self) -> Style {
        let style = if self.colors_enabled {
            Style::default().fg(self.text)
        } else {
            Style::default()
        };
        style.add_modifier(Modifier::BOLD)
    }

    /// Style of the selected row.
    pub fn selection_style(&self) -> Style {
        if self.colors_enabled {
            Style::default().bg(self.selection_bg).fg(self.selection_fg)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        }
    }

    /// Style of a task status.
    pub fn status_style(&self, status: Status) -> Style {
        let color = match status {
            Status::Todo => self.status_todo,
            Status::InProgress => self.status_in_progress,
            Status::Done => self.status_done,
        };
        self.fg(color)
    }

    /// Style of a whole row carrying a task status: completed tasks are
    /// dimmed so the active ones stand out.
    pub fn status_row_style(&self, status: Status) -> Style {
        match status {
            Status::Done => self.muted_style(),
            _ => Style::default(),
        }
    }

    /// Style of a task priority.
    pub fn priority_style(&self, priority: Priority) -> Style {
        let color = match priority {
            Priority::High => self.priority_high,
            Priority::Medium => self.priority_medium,
            Priority::Low => self.priority_low,
        };
        self.fg(color)
    }

    /// Style of a version status.
    pub fn version_status_style(&self, status: VersionStatus) -> Style {
        let color = match status {
            VersionStatus::Backlog => self.version_backlog,
            VersionStatus::Open => self.version_open,
            VersionStatus::Released => self.version_released,
        };
        self.fg(color)
    }
}

/// `true` when `NO_COLOR` is set to a non-empty value.
pub fn no_color_requested() -> bool {
    no_color_in(&|key| std::env::var(key).ok())
}

/// `true` when the given lookup yields a non-empty `NO_COLOR` value.
fn no_color_in(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    lookup(NO_COLOR_ENV).is_some_and(|value| !value.is_empty())
}

/// The process-wide theme, resolved from the environment on first use.
pub fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::from_env)
}

/// Style of a task status, using the process-wide theme.
pub fn status_style(status: Status) -> Style {
    theme().status_style(status)
}

/// Style of a whole row carrying a task status, using the process-wide theme.
pub fn status_row_style(status: Status) -> Style {
    theme().status_row_style(status)
}

/// Style of a task priority, using the process-wide theme.
pub fn priority_style(priority: Priority) -> Style {
    theme().priority_style(priority)
}

/// Style of a version status, using the process-wide theme.
pub fn version_status_style(status: VersionStatus) -> Style {
    theme().version_status_style(status)
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::*;

    #[test]
    fn default_palette_uses_terminal_friendly_colors() {
        let theme = Theme::default_theme();

        assert!(theme.colors_enabled);
        assert_eq!(theme.text, Color::Reset);
        assert_eq!(theme.bg, Color::Reset);
        // The header must not be hardcoded white: it is invisible on light
        // terminals.
        assert_ne!(theme.header_style().fg, Some(Color::White));
    }

    #[test]
    fn header_style_is_bold() {
        assert!(
            Theme::default_theme()
                .header_style()
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn status_and_priority_colors_do_not_collide() {
        let theme = Theme::default_theme();

        assert_ne!(theme.status_todo, theme.priority_medium);
        assert_ne!(theme.status_done, theme.priority_low);
        assert_ne!(theme.status_in_progress, theme.priority_high);
    }

    #[test]
    fn status_styles_map_each_variant() {
        let theme = Theme::default_theme();

        assert_eq!(theme.status_style(Status::Todo).fg, Some(theme.status_todo));
        assert_eq!(
            theme.status_style(Status::InProgress).fg,
            Some(theme.status_in_progress)
        );
        assert_eq!(theme.status_style(Status::Done).fg, Some(theme.status_done));
    }

    #[test]
    fn done_rows_are_muted_and_others_are_unstyled() {
        let theme = Theme::default_theme();

        assert_eq!(theme.status_row_style(Status::Done).fg, Some(theme.muted));
        assert_eq!(theme.status_row_style(Status::Todo).fg, None);
        assert_eq!(theme.status_row_style(Status::InProgress).fg, None);
    }

    #[test]
    fn priority_styles_map_each_variant() {
        let theme = Theme::default_theme();

        assert_eq!(
            theme.priority_style(Priority::High).fg,
            Some(theme.priority_high)
        );
        assert_eq!(
            theme.priority_style(Priority::Medium).fg,
            Some(theme.priority_medium)
        );
        assert_eq!(
            theme.priority_style(Priority::Low).fg,
            Some(theme.priority_low)
        );
    }

    #[test]
    fn version_status_styles_map_each_variant() {
        let theme = Theme::default_theme();

        assert_eq!(
            theme.version_status_style(VersionStatus::Backlog).fg,
            Some(theme.version_backlog)
        );
        assert_eq!(
            theme.version_status_style(VersionStatus::Open).fg,
            Some(theme.version_open)
        );
        assert_eq!(
            theme.version_status_style(VersionStatus::Released).fg,
            Some(theme.version_released)
        );
    }

    #[test]
    fn no_color_disables_colors_but_keeps_bold() {
        let theme = Theme::new(Some(DEFAULT_PALETTE), true);

        assert!(!theme.colors_enabled);
        assert_eq!(theme.status_style(Status::Todo).fg, None);
        assert_eq!(theme.priority_style(Priority::High).fg, None);
        assert_eq!(theme.version_status_style(VersionStatus::Open).fg, None);
        assert_eq!(theme.danger_style().fg, None);
        assert_eq!(theme.muted_style().fg, None);
        // Bold survives: shapes and emphasis are kept.
        assert!(theme.header_style().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn no_color_keeps_selection_visible_through_reverse_video() {
        let theme = Theme::new(Some(DEFAULT_PALETTE), true);

        assert!(
            theme
                .selection_style()
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn unknown_palette_falls_back_to_default() {
        let theme = Theme::new(Some("does-not-exist"), false);

        assert_eq!(theme.status_todo, Theme::default_theme().status_todo);
        assert_eq!(theme.priority_high, Theme::default_theme().priority_high);
    }

    #[test]
    fn bright_palette_uses_bright_ansi_variants() {
        let theme = Theme::new(Some(BRIGHT_PALETTE), false);

        assert_eq!(theme.status_done, Color::LightGreen);
        assert_eq!(theme.priority_high, Color::LightRed);
        assert_eq!(theme.version_open, Color::LightBlue);
    }

    #[test]
    fn selection_style_uses_background_when_colors_are_enabled() {
        let theme = Theme::default_theme();

        assert_eq!(theme.selection_style().bg, Some(theme.selection_bg));
    }

    #[test]
    fn env_lookup_selects_the_bright_palette() {
        let theme = Theme::from_env_with(|key| match key {
            THEME_ENV => Some(BRIGHT_PALETTE.to_string()),
            _ => None,
        });

        assert_eq!(theme.status_done, Color::LightGreen);
        assert!(theme.colors_enabled);
    }

    #[test]
    fn env_lookup_disables_colors_with_no_color() {
        let theme = Theme::from_env_with(|key| match key {
            NO_COLOR_ENV => Some("1".to_string()),
            _ => None,
        });

        assert!(!theme.colors_enabled);
        assert_eq!(theme.status_style(Status::Todo).fg, None);
    }

    #[test]
    fn empty_no_color_value_keeps_colors() {
        let theme = Theme::from_env_with(|key| match key {
            NO_COLOR_ENV => Some(String::new()),
            _ => None,
        });

        assert!(theme.colors_enabled);
    }

    #[test]
    fn absent_env_uses_the_default_palette() {
        let theme = Theme::from_env_with(|_| None);

        assert_eq!(theme.status_todo, Theme::default_theme().status_todo);
        assert!(theme.colors_enabled);
    }
}
