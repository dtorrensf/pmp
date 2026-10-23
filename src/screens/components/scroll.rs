// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reusable scrolling helpers.
//!
//! Two kinds of scrollable areas are covered:
//!
//! - **Selectable lists** (task table, project list, dependency picker): a
//!   [`ListScroll`] keeps the viewport offset across frames so the selected
//!   row stays visible. Screens keep `selected` as the source of truth, slice
//!   their rows to [`ListScroll::visible_range`], and style the selected row
//!   themselves. Stateful ratatui widgets are not used.
//! - **Plain text blocks** (descriptions, notes, dependency panel): the
//!   caller stores a `u16` scroll value, clamps it with [`clamp_text_scroll`]
//!   against the wrapped line count from [`wrapped_line_count`], and passes it
//!   to `Paragraph::scroll`.
//!
//! [`ListScroll`] uses a [`Cell`] so that rendering can take `&self`, which is
//! the signature of `Component::render`.

use std::cell::Cell;
use std::ops::Range;

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};

/// Viewport offset for a selectable list, persisted across frames.
#[derive(Debug, Default)]
pub struct ListScroll {
    offset: Cell<usize>,
}

impl ListScroll {
    /// Creates a scroll state positioned at the first item.
    pub const fn new() -> Self {
        Self {
            offset: Cell::new(0),
        }
    }

    /// Index of the first visible item.
    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    /// Returns to the first item, e.g. after the list contents change.
    pub fn reset(&self) {
        self.offset.set(0);
    }

    /// Computes the range of item indices to render so that `selected` is
    /// visible in a viewport of `height` rows over `total` items.
    ///
    /// The offset moves the minimum amount needed: moving the selection
    /// inside the current view does not scroll it, and the view only moves
    /// when the selection leaves it. The offset is clamped so the view never
    /// shows empty rows past the end of the list. `selected` is clamped to
    /// `total - 1`. When `total` or `height` is zero the range is empty.
    pub fn visible_range(&self, total: usize, selected: usize, height: usize) -> Range<usize> {
        if total == 0 || height == 0 {
            self.offset.set(0);
            return 0..0;
        }

        let selected = selected.min(total - 1);
        let max_offset = total.saturating_sub(height);
        let mut offset = self.offset.get().min(max_offset);

        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }

        self.offset.set(offset);
        offset..(offset + height).min(total)
    }
}

/// Scroll position for a plain text block, persisted across frames.
///
/// Keys call [`TextScroll::page_up`] / [`TextScroll::page_down`], which step
/// by the last rendered viewport height. Rendering calls
/// [`TextScroll::sync`], which records the viewport, clamps the stored offset
/// to the content and returns it for `Paragraph::scroll`. Clamping on render
/// keeps the offset from running past the end while the user keeps pressing
/// `PageDown`.
#[derive(Debug, Default)]
pub struct TextScroll {
    offset: Cell<u16>,
    viewport: Cell<u16>,
}

impl TextScroll {
    /// Creates a scroll state at the top of the text.
    pub const fn new() -> Self {
        Self {
            offset: Cell::new(0),
            viewport: Cell::new(0),
        }
    }

    /// Current (last synced) scroll offset in rows.
    pub fn offset(&self) -> u16 {
        self.offset.get()
    }

    /// Scrolls up by one viewport height, stopping at the top.
    pub fn page_up(&self) {
        let step = self.viewport.get().max(1);
        self.offset.set(self.offset.get().saturating_sub(step));
    }

    /// Scrolls down by one viewport height. The offset is clamped on the
    /// next [`TextScroll::sync`].
    pub fn page_down(&self) {
        let step = self.viewport.get().max(1);
        self.offset.set(self.offset.get().saturating_add(step));
    }

    /// Returns to the top, e.g. when the displayed content is replaced.
    pub fn reset(&self) {
        self.offset.set(0);
    }

    /// Records the viewport height, clamps the offset to `total_lines`, and
    /// returns the offset to pass to `Paragraph::scroll`.
    pub fn sync(&self, total_lines: usize, viewport_height: u16) -> u16 {
        self.viewport.set(viewport_height);
        let clamped = clamp_text_scroll(self.offset.get(), total_lines, viewport_height);
        self.offset.set(clamped);
        clamped
    }
}

/// Clamps a text scroll position so the viewport never scrolls past the last
/// line. `total_lines` is the number of rendered (wrapped) lines and
/// `viewport_height` the number of visible rows.
pub fn clamp_text_scroll(scroll: u16, total_lines: usize, viewport_height: u16) -> u16 {
    let max_scroll = total_lines.saturating_sub(viewport_height as usize);
    scroll.min(u16::try_from(max_scroll).unwrap_or(u16::MAX))
}

/// Estimates how many rows `text` occupies when wrapped to `width` columns,
/// using greedy word wrapping similar to `Paragraph` with `Wrap`. Each
/// logical line is at least one row; words longer than `width` are split.
/// Widths are counted in `char`s, which is exact for ASCII and close enough
/// for the Latin text used in the UI. `width == 0` yields one row per line.
pub fn wrapped_line_count(text: &str, width: u16) -> usize {
    let width = width as usize;
    text.lines().map(|line| wrap_line_rows(line, width)).sum()
}

fn wrap_line_rows(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut rows = 1;
    let mut col = 0usize;
    for word in line.split_whitespace() {
        let len = word.chars().count();
        let needed = if col == 0 { len } else { col + 1 + len };
        if needed <= width {
            col = needed;
            continue;
        }
        // Word does not fit on the current row: start a new one.
        if col > 0 {
            rows += 1;
        }
        // Split words longer than the row width across several rows.
        let full_rows = len.saturating_sub(1) / width;
        rows += full_rows;
        col = len - full_rows * width;
    }
    rows
}

/// Renders a vertical scrollbar on the right edge of `area` when the content
/// does not fit. `total` is the number of items or rows, `viewport` the
/// number of visible ones and `offset` the first visible index.
pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    viewport: usize,
    offset: usize,
) {
    if total <= viewport || area.width == 0 || area.height == 0 {
        return;
    }
    let max_offset = total - viewport;
    let mut state = ScrollbarState::new(max_offset).position(offset.min(max_offset));
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    frame.render_stateful_widget(scrollbar, area, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_yields_empty_range_and_zero_offset() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(0, 0, 5), 0..0);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn zero_height_yields_empty_range() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(10, 3, 0), 0..0);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn list_shorter_than_viewport_never_scrolls() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(3, 2, 10), 0..3);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn list_equal_to_viewport_never_scrolls() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(5, 4, 5), 0..5);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn moving_down_scrolls_only_when_selection_leaves_view() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(20, 0, 5), 0..5);
        assert_eq!(scroll.visible_range(20, 4, 5), 0..5);
        assert_eq!(scroll.visible_range(20, 5, 5), 1..6);
        assert_eq!(scroll.visible_range(20, 6, 5), 2..7);
    }

    #[test]
    fn moving_up_inside_view_does_not_scroll() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(20, 17, 5), 13..18);
        assert_eq!(scroll.visible_range(20, 15, 5), 13..18);
        assert_eq!(scroll.visible_range(20, 14, 5), 13..18);
        // Leaving the top edge scrolls up by one.
        assert_eq!(scroll.visible_range(20, 12, 5), 12..17);
    }

    #[test]
    fn jumping_to_last_item_clamps_to_end() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(20, 19, 5), 15..20);
        assert_eq!(scroll.offset(), 15);
    }

    #[test]
    fn jumping_back_to_top_scrolls_to_zero() {
        let scroll = ListScroll::new();
        scroll.visible_range(20, 19, 5);
        assert_eq!(scroll.visible_range(20, 0, 5), 0..5);
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn selection_past_end_is_clamped() {
        let scroll = ListScroll::new();
        assert_eq!(scroll.visible_range(20, 500, 5), 15..20);
    }

    #[test]
    fn offset_is_preserved_across_frames_when_selection_unchanged() {
        let scroll = ListScroll::new();
        scroll.visible_range(20, 10, 5);
        let first = scroll.offset();
        for _ in 0..3 {
            assert_eq!(scroll.visible_range(20, 10, 5).start, first);
        }
    }

    #[test]
    fn shrinking_list_clamps_stale_offset() {
        let scroll = ListScroll::new();
        scroll.visible_range(20, 19, 5);
        assert_eq!(scroll.offset(), 15);
        // Only 8 items remain: the view must not show empty rows.
        assert_eq!(scroll.visible_range(8, 7, 5), 3..8);
    }

    #[test]
    fn growing_viewport_clamps_offset_to_end() {
        let scroll = ListScroll::new();
        scroll.visible_range(20, 19, 5);
        // Viewport of 10 over 20 items: max offset is 10.
        assert_eq!(scroll.visible_range(20, 19, 10), 10..20);
    }

    #[test]
    fn reset_returns_to_first_item() {
        let scroll = ListScroll::new();
        scroll.visible_range(20, 19, 5);
        scroll.reset();
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn clamp_text_scroll_limits_to_last_page() {
        assert_eq!(clamp_text_scroll(0, 10, 4), 0);
        assert_eq!(clamp_text_scroll(3, 10, 4), 3);
        assert_eq!(clamp_text_scroll(50, 10, 4), 6);
    }

    #[test]
    fn clamp_text_scroll_handles_short_and_empty_content() {
        assert_eq!(clamp_text_scroll(5, 2, 4), 0);
        assert_eq!(clamp_text_scroll(5, 0, 0), 0);
        assert_eq!(clamp_text_scroll(0, 0, 3), 0);
    }

    #[test]
    fn wrapped_line_count_counts_logical_lines() {
        assert_eq!(wrapped_line_count("", 10), 0);
        assert_eq!(wrapped_line_count("one", 10), 1);
        assert_eq!(wrapped_line_count("one\ntwo\nthree", 10), 3);
        assert_eq!(wrapped_line_count("one\n\nthree", 10), 3);
    }

    #[test]
    fn wrapped_line_count_wraps_on_word_boundaries() {
        // "aaa bbb" fits in 7 columns; "ccc" must wrap.
        assert_eq!(wrapped_line_count("aaa bbb ccc", 7), 2);
        assert_eq!(wrapped_line_count("aaa bbb ccc", 11), 1);
        assert_eq!(wrapped_line_count("aaa bbb ccc", 3), 3);
    }

    #[test]
    fn wrapped_line_count_splits_long_words() {
        assert_eq!(wrapped_line_count("abcdefghij", 4), 3);
        assert_eq!(wrapped_line_count("abcd", 4), 1);
    }

    #[test]
    fn wrapped_line_count_zero_width_is_one_row_per_line() {
        assert_eq!(wrapped_line_count("one\ntwo", 0), 2);
    }

    #[test]
    fn text_scroll_pages_by_viewport_height() {
        let scroll = TextScroll::new();
        scroll.sync(100, 10);
        scroll.page_down();
        assert_eq!(scroll.offset(), 10);
        scroll.page_down();
        assert_eq!(scroll.offset(), 20);
        scroll.page_up();
        assert_eq!(scroll.offset(), 10);
        scroll.page_up();
        scroll.page_up();
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn text_scroll_clamps_to_end_on_sync() {
        let scroll = TextScroll::new();
        scroll.sync(25, 10);
        for _ in 0..5 {
            scroll.page_down();
        }
        // Max offset for 25 lines in 10 rows is 15.
        assert_eq!(scroll.sync(25, 10), 15);
        assert_eq!(scroll.offset(), 15);
        // Content shrinks: offset follows.
        assert_eq!(scroll.sync(5, 10), 0);
    }

    #[test]
    fn text_scroll_without_sync_pages_by_one_row() {
        let scroll = TextScroll::new();
        scroll.page_down();
        assert_eq!(scroll.offset(), 1);
    }

    #[test]
    fn text_scroll_reset_returns_to_top() {
        let scroll = TextScroll::new();
        scroll.sync(100, 10);
        scroll.page_down();
        scroll.reset();
        assert_eq!(scroll.offset(), 0);
    }

    #[test]
    fn scrollbar_is_skipped_when_content_fits() {
        // Must not panic and must draw nothing when total <= viewport.
        let backend = ratatui::backend::TestBackend::new(4, 4);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_scrollbar(frame, frame.area(), 3, 4, 0))
            .unwrap();
        let rendered = terminal.backend().buffer().clone();
        assert!(rendered.content().iter().all(|c| c.symbol() == " "));
    }
}
