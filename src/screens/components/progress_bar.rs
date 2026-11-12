// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! A horizontal progress bar with sub-cell precision.
//!
//! The bar is drawn with the shared glyph set: `█` for the filled part,
//! `░` for the empty part and `▌` for a half-filled cell, so a ratio such
//! as 33% is not rounded to a coarse step.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::screens::components::glyphs;

/// A bar of `width` cells showing `ratio` (0.0..=1.0) as filled.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressBar {
    ratio: f64,
    width: usize,
    filled_style: Style,
    empty_style: Style,
}

impl ProgressBar {
    /// A bar of `width` cells showing `ratio` (0.0..=1.0).
    ///
    /// The ratio is clamped to the valid range, so out-of-range values never
    /// overflow the bar.
    pub fn new(ratio: f64, width: usize) -> ProgressBar {
        ProgressBar {
            ratio: ratio.clamp(0.0, 1.0),
            width,
            filled_style: Style::default(),
            empty_style: Style::default(),
        }
    }

    /// Sets the style of the filled cells.
    pub fn with_filled_style(mut self, style: Style) -> ProgressBar {
        self.filled_style = style;
        self
    }

    /// Sets the style of the empty cells.
    pub fn with_empty_style(mut self, style: Style) -> ProgressBar {
        self.empty_style = style;
        self
    }

    /// The ratio shown by the bar, clamped to 0.0..=1.0.
    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// The width of the bar, in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of half cells the bar is filled with.
    ///
    /// The ratio is rounded to the nearest half cell, so a bar is never off
    /// by more than a quarter of a cell.
    fn half_units(&self) -> usize {
        (self.ratio * self.width as f64 * 2.0).round() as usize
    }

    /// Number of whole cells that are filled.
    pub fn filled_cells(&self) -> usize {
        self.half_units() / 2
    }

    /// `true` when the bar ends with a half-filled cell.
    pub fn has_half_cell(&self) -> bool {
        self.half_units() % 2 == 1
    }

    /// The bar as a line of styled spans.
    pub fn line(&self) -> Line<'static> {
        let glyphs = glyphs::glyphs();
        let filled = self.filled_cells();
        let half = self.has_half_cell();
        let filled_len = filled + usize::from(half);
        let empty_len = self.width.saturating_sub(filled_len);

        let mut spans = Vec::new();
        if filled > 0 {
            spans.push(Span::styled(
                glyphs.bar_full.repeat(filled),
                self.filled_style,
            ));
        }
        if half {
            spans.push(Span::styled(glyphs.bar_half.to_string(), self.filled_style));
        }
        if empty_len > 0 {
            spans.push(Span::styled(
                glyphs.bar_empty.repeat(empty_len),
                self.empty_style,
            ));
        }
        Line::from(spans)
    }

    /// The bar as plain text, without styles.
    pub fn text(&self) -> String {
        let glyphs = glyphs::glyphs();
        let filled = self.filled_cells();
        let half = self.has_half_cell();
        let filled_len = filled + usize::from(half);
        let empty_len = self.width.saturating_sub(filled_len);

        let mut text = String::new();
        text.push_str(&glyphs.bar_full.repeat(filled));
        if half {
            text.push_str(glyphs.bar_half);
        }
        text.push_str(&glyphs.bar_empty.repeat(empty_len));
        text
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn empty_bar_is_all_empty_cells() {
        let bar = ProgressBar::new(0.0, 10);

        assert_eq!(bar.text(), "░░░░░░░░░░");
    }

    #[test]
    fn full_bar_is_all_full_cells() {
        let bar = ProgressBar::new(1.0, 10);

        assert_eq!(bar.text(), "██████████");
    }

    #[test]
    fn half_ratio_fills_half_the_cells() {
        let bar = ProgressBar::new(0.5, 10);

        assert_eq!(bar.filled_cells(), 5);
        assert!(!bar.has_half_cell());
        assert_eq!(bar.text(), "█████░░░░░");
    }

    #[test]
    fn third_ratio_keeps_sub_cell_precision() {
        let bar = ProgressBar::new(1.0 / 3.0, 10);

        assert_eq!(bar.filled_cells(), 3);
        assert!(bar.has_half_cell());
        assert_eq!(bar.text(), "███▌░░░░░░");
    }

    #[test]
    fn quarter_ratio_fills_exactly_two_and_a_half_cells() {
        let bar = ProgressBar::new(0.25, 10);

        assert_eq!(bar.filled_cells(), 2);
        assert!(bar.has_half_cell());
        assert_eq!(bar.text(), "██▌░░░░░░░");
    }

    #[test]
    fn ratio_rounds_to_the_nearest_half_cell() {
        // 20% of 10 cells is exactly two cells.
        let bar = ProgressBar::new(0.2, 10);
        assert_eq!(bar.text(), "██░░░░░░░░");

        // 15% of 10 cells is one and a half cells.
        let bar = ProgressBar::new(0.15, 10);
        assert_eq!(bar.text(), "█▌░░░░░░░░");
    }

    #[test]
    fn ratio_is_clamped_to_the_valid_range() {
        assert_eq!(ProgressBar::new(2.0, 10).ratio(), 1.0);
        assert_eq!(ProgressBar::new(-1.0, 10).ratio(), 0.0);
        assert_eq!(ProgressBar::new(2.0, 10).text(), "██████████");
        assert_eq!(ProgressBar::new(-1.0, 10).text(), "░░░░░░░░░░");
    }

    #[test]
    fn bar_never_exceeds_its_width() {
        for ratio in [0.0, 0.15, 0.33, 0.5, 0.99, 1.0] {
            let bar = ProgressBar::new(ratio, 8);
            assert_eq!(
                bar.text().chars().count(),
                8,
                "ratio {ratio} produced a bar of the wrong width"
            );
        }
    }

    #[test]
    fn zero_width_bar_produces_no_cells() {
        let bar = ProgressBar::new(0.5, 0);

        assert_eq!(bar.text(), "");
        assert!(!bar.has_half_cell());
    }

    #[test]
    fn line_carries_the_filled_and_empty_styles() {
        let bar = ProgressBar::new(0.5, 4)
            .with_filled_style(ratatui::style::Style::default().fg(Color::Green))
            .with_empty_style(ratatui::style::Style::default().fg(Color::Gray));

        let spans = bar.line().spans;

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "██");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[1].content, "░░");
        assert_eq!(spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn bar_follows_the_glyph_set_in_use() {
        use crate::screens::components::glyphs::{self, Glyphs};

        let text = glyphs::with_glyphs(Glyphs::ascii(), || ProgressBar::new(0.5, 4).text());

        assert_eq!(text, "##..");
    }
}
