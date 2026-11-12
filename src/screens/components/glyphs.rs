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

use std::sync::OnceLock;

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
    /// Warning marker.
    pub warning: &'static str,
    /// Filled cell of a progress bar.
    pub bar_full: &'static str,
    /// Empty cell of a progress bar.
    pub bar_empty: &'static str,
    /// Half-filled cell of a progress bar.
    pub bar_half: &'static str,
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
            warning: "⚠",
            bar_full: "█",
            bar_empty: "░",
            bar_half: "▌",
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
            warning: "!",
            bar_full: "#",
            bar_empty: ".",
            bar_half: "=",
        }
    }

    /// Every glyph of the set, in a fixed order.
    #[cfg(test)]
    fn all(&self) -> [&'static str; 14] {
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
            self.warning,
            self.bar_full,
            self.bar_empty,
            self.bar_half,
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

/// `true` when the ASCII glyph set is requested through `PMP_ASCII`.
pub fn ascii_requested() -> bool {
    ascii_requested_in(&|key| std::env::var(key).ok())
}

/// `true` when the given lookup yields a non-empty `PMP_ASCII` value.
fn ascii_requested_in(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    lookup(ASCII_ENV).is_some_and(|value| !value.is_empty())
}

thread_local! {
    /// Test-only override of the glyph set, so a render can be checked with
    /// the ASCII fallback without touching the process environment.
    static OVERRIDE: std::cell::RefCell<Option<Glyphs>> = const { std::cell::RefCell::new(None) };
}

/// The glyph set in use, resolved from the environment on first use.
pub fn glyphs() -> Glyphs {
    if let Some(glyphs) = OVERRIDE.with(|override_| *override_.borrow()) {
        return glyphs;
    }
    static GLYPHS: OnceLock<Glyphs> = OnceLock::new();
    *GLYPHS.get_or_init(|| {
        if ascii_requested() {
            Glyphs::ascii()
        } else {
            Glyphs::unicode()
        }
    })
}

/// Runs `body` with `glyphs` forced as the glyph set of the current thread.
///
/// This is how the tests exercise the ASCII fallback end to end.
#[cfg(test)]
pub fn with_glyphs<T>(glyphs: Glyphs, body: impl FnOnce() -> T) -> T {
    let previous = OVERRIDE.with(|override_| override_.replace(Some(glyphs)));
    let result = body();
    OVERRIDE.with(|override_| *override_.borrow_mut() = previous);
    result
}

/// Icon of a task status, using the glyph set in use.
pub fn status_icon(status: Status) -> &'static str {
    glyphs().status(status)
}

/// Icon of a task priority, using the glyph set in use.
pub fn priority_icon(priority: Priority) -> &'static str {
    glyphs().priority(priority)
}

/// Icon of a version status, using the glyph set in use.
pub fn version_status_icon(status: VersionStatus) -> &'static str {
    glyphs().version_status(status)
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
    fn ascii_set_is_used_when_requested() {
        let glyphs = with_glyphs(Glyphs::ascii(), glyphs);

        assert_eq!(glyphs, Glyphs::ascii());
        assert_eq!(glyphs.status(Status::Todo), "o");
    }

    #[test]
    fn override_restores_the_previous_set() {
        with_glyphs(Glyphs::ascii(), || {
            assert_eq!(glyphs(), Glyphs::ascii());
        });

        assert_ne!(glyphs(), Glyphs::ascii());
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
