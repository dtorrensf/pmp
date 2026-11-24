// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shared glyph set used across the app.
//!
//! Every icon lives here so the whole UI speaks the same visual language.
//! Two sets are provided: a Unicode set (the default) and an ASCII
//! fallback for terminals that cannot render the Unicode glyphs. The
//! fallback is selected with the `PMP_ASCII` environment variable.
//!
//! Every glyph is a single display column wide, which is asserted by the
//! tests in this module. Glyphs whose width is ambiguous (such as the
//! `⚠️` emoji, two columns because of its variation selector) are avoided
//! on purpose: they break the alignment of the surrounding text.

use crate::domain::task::{Priority, Status};
use crate::domain::version::VersionStatus;

/// Environment variable that switches to the ASCII glyph set.
pub const ASCII_ENV: &str = "PMP_ASCII";

/// The icons used by the app.
///
/// All fields are single-column glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    /// Icon of `Status::Todo`.
    pub status_todo: &'static str,
    /// Icon of `Status::InProgress`.
    pub status_in_progress: &'static str,
    /// Icon of `Status::Done`.
    pub status_done: &'static str,
    /// Icon of `Priority::High`.
    pub priority_high: &'static str,
    /// Icon of `Priority::Medium`.
    pub priority_medium: &'static str,
    /// Icon of `Priority::Low`.
    pub priority_low: &'static str,
    /// Icon of `VersionStatus::Backlog`.
    pub version_backlog: &'static str,
    /// Icon of `VersionStatus::Open`.
    pub version_open: &'static str,
    /// Icon of `VersionStatus::Released`.
    pub version_released: &'static str,
    /// Marker of the selected row.
    pub selection: &'static str,
    /// Marker of the number of tasks a task depends on.
    pub dependency: &'static str,
    /// Warning marker.
    pub warning: &'static str,
    /// Marker of an informational message.
    pub info: &'static str,
    /// Marker of a successful operation.
    pub success: &'static str,
    /// Marker of a failed operation.
    pub error: &'static str,
    /// Filled cell of a progress bar.
    pub bar_full: &'static str,
    /// Empty cell of a progress bar.
    pub bar_empty: &'static str,
    /// Half-filled cell of a progress bar.
    pub bar_half: &'static str,
    /// Ascending sort direction, shown next to the ID sort.
    pub sort_asc: &'static str,
    /// Descending sort direction, shown next to the ID sort.
    pub sort_desc: &'static str,
    /// `true` for the ASCII fallback, so callers that mix glyphs and text (the
    /// braille spinner) can pick a matching set without inventing one.
    ascii: bool,
}

impl Glyphs {
    /// The Unicode glyph set, used by default.
    pub const fn unicode() -> Glyphs {
        Glyphs {
            status_todo: "○",
            status_in_progress: "◐",
            status_done: "✔",
            priority_high: "▲",
            priority_medium: "●",
            priority_low: "▼",
            version_backlog: "○",
            version_open: "◐",
            version_released: "✔",
            selection: "▸",
            dependency: "↳",
            warning: "⚠",
            info: "»",
            success: "✓",
            error: "✗",
            bar_full: "█",
            bar_empty: "░",
            bar_half: "▌",
            sort_asc: "↑",
            sort_desc: "↓",
            ascii: false,
        }
    }

    /// The ASCII fallback, for terminals without Unicode support.
    pub const fn ascii() -> Glyphs {
        Glyphs {
            status_todo: "o",
            status_in_progress: "*",
            status_done: "x",
            priority_high: "^",
            priority_medium: "-",
            priority_low: "v",
            version_backlog: "o",
            version_open: "*",
            version_released: "x",
            selection: ">",
            dependency: ">",
            warning: "!",
            info: ">",
            success: "+",
            error: "x",
            bar_full: "#",
            bar_empty: ".",
            bar_half: "=",
            sort_asc: "^",
            sort_desc: "v",
            ascii: true,
        }
    }

    /// `true` for the ASCII fallback set.
    pub const fn is_ascii(&self) -> bool {
        self.ascii
    }

    /// The glyph set requested by the environment: ASCII when `PMP_ASCII` is
    /// set to a non-empty value, Unicode otherwise. Resolved once by the
    /// composition root.
    pub fn from_env() -> Glyphs {
        Glyphs::from_lookup(&|key| std::env::var(key).ok())
    }

    /// Same as [`Self::from_env`] with an injectable lookup, for tests.
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Glyphs {
        if ascii_requested_in(lookup) {
            Glyphs::ascii()
        } else {
            Glyphs::unicode()
        }
    }

    /// Every glyph of the set, in a fixed order.
    #[cfg(test)]
    fn all(&self) -> [&'static str; 20] {
        [
            self.status_todo,
            self.status_in_progress,
            self.status_done,
            self.priority_high,
            self.priority_medium,
            self.priority_low,
            self.version_backlog,
            self.version_open,
            self.version_released,
            self.selection,
            self.dependency,
            self.warning,
            self.info,
            self.success,
            self.error,
            self.bar_full,
            self.bar_empty,
            self.bar_half,
            self.sort_asc,
            self.sort_desc,
        ]
    }

    /// Icon of a task status.
    pub fn status(&self, status: Status) -> &'static str {
        match status {
            Status::Todo => self.status_todo,
            Status::InProgress => self.status_in_progress,
            Status::Done => self.status_done,
        }
    }

    /// Icon of a task priority.
    pub fn priority(&self, priority: Priority) -> &'static str {
        match priority {
            Priority::High => self.priority_high,
            Priority::Medium => self.priority_medium,
            Priority::Low => self.priority_low,
        }
    }

    /// Icon of a version status.
    pub fn version_status(&self, status: VersionStatus) -> &'static str {
        match status {
            VersionStatus::Backlog => self.version_backlog,
            VersionStatus::Open => self.version_open,
            VersionStatus::Released => self.version_released,
        }
    }
}

/// `true` when the given lookup yields a non-empty `PMP_ASCII` value.
fn ascii_requested_in(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    lookup(ASCII_ENV).is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn unicode_glyphs_are_all_one_column_wide() {
        for glyph in Glyphs::unicode().all() {
            assert_eq!(
                glyph.width(),
                1,
                "glyph {glyph:?} must be exactly one column wide"
            );
        }
    }

    #[test]
    fn ascii_glyphs_are_all_one_column_wide() {
        for glyph in Glyphs::ascii().all() {
            assert_eq!(
                glyph.width(),
                1,
                "glyph {glyph:?} must be exactly one column wide"
            );
        }
    }

    #[test]
    fn unicode_glyphs_are_not_ascii() {
        for glyph in Glyphs::unicode().all() {
            assert!(
                !glyph.is_ascii(),
                "glyph {glyph:?} belongs to the Unicode set"
            );
        }
    }

    #[test]
    fn ascii_glyphs_are_ascii() {
        for glyph in Glyphs::ascii().all() {
            assert!(glyph.is_ascii(), "glyph {glyph:?} belongs to the ASCII set");
        }
    }

    #[test]
    fn status_icons_match_the_documented_set() {
        let glyphs = Glyphs::unicode();

        assert_eq!(glyphs.status(Status::Todo), "○");
        assert_eq!(glyphs.status(Status::InProgress), "◐");
        assert_eq!(glyphs.status(Status::Done), "✔");
    }

    #[test]
    fn priority_icons_match_the_documented_set() {
        let glyphs = Glyphs::unicode();

        assert_eq!(glyphs.priority(Priority::High), "▲");
        assert_eq!(glyphs.priority(Priority::Medium), "●");
        assert_eq!(glyphs.priority(Priority::Low), "▼");
    }

    #[test]
    fn selection_marker_is_a_single_column() {
        assert_eq!(Glyphs::unicode().selection, "▸");
        assert_eq!(Glyphs::ascii().selection, ">");
    }

    #[test]
    fn warning_glyph_avoids_the_two_column_emoji() {
        let glyphs = Glyphs::unicode();

        assert_eq!(glyphs.warning.width(), 1);
        // The emoji form carries a variation selector and measures two
        // columns, which misaligns the text around it.
        assert!("⚠️".width() > glyphs.warning.width());
    }

    #[test]
    fn bar_glyphs_are_distinct() {
        let glyphs = Glyphs::unicode();

        assert_eq!(glyphs.bar_full, "█");
        assert_eq!(glyphs.bar_empty, "░");
        assert_eq!(glyphs.bar_half, "▌");
    }

    #[test]
    fn version_status_icons_are_defined() {
        let glyphs = Glyphs::unicode();

        assert_eq!(glyphs.version_status(VersionStatus::Backlog), "○");
        assert_eq!(glyphs.version_status(VersionStatus::Open), "◐");
        assert_eq!(glyphs.version_status(VersionStatus::Released), "✔");
    }

    #[test]
    fn ascii_set_is_selected_from_the_lookup() {
        let glyphs = Glyphs::from_lookup(&|key| (key == ASCII_ENV).then(|| "1".to_string()));

        assert_eq!(glyphs, Glyphs::ascii());
        assert!(glyphs.is_ascii());
        assert_eq!(glyphs.status(Status::Todo), "o");
        assert!(!Glyphs::unicode().is_ascii());
    }

    #[test]
    fn ascii_requested_ignores_an_empty_value() {
        assert!(!ascii_requested_in(
            &|key| (key == ASCII_ENV).then(String::new)
        ));
        assert!(ascii_requested_in(
            &|key| (key == ASCII_ENV).then(|| "1".to_string())
        ));
        assert!(!ascii_requested_in(&|_| None));
    }
}
