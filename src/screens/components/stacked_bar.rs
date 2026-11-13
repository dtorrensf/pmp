// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! A full-width bar split into Todo / In Progress / Done segments, plus the
//! legend that names them with their counts.
//!
//! Every task chart in the app uses this module, so the same status always
//! has the same colour and the same proportion everywhere.

use ratatui::text::{Line, Span};

use crate::domain::task::{Status, Task};
use crate::screens::components::glyphs;
use crate::screens::components::theme::{self, status_style};

/// The statuses in the order they are drawn, left to right.
pub const STATUS_ORDER: [Status; 3] = [Status::Todo, Status::InProgress, Status::Done];

/// Tasks per status, in `STATUS_ORDER`: `[todo, in_progress, done]`.
pub fn status_counts<'a>(tasks: impl IntoIterator<Item = &'a Task>) -> [u64; 3] {
    let mut counts = [0u64; 3];
    for task in tasks {
        match task.status {
            Status::Todo => counts[0] += 1,
            Status::InProgress => counts[1] += 1,
            Status::Done => counts[2] += 1,
        }
    }
    counts
}

/// Splits `width` cells among the three status counts, proportionally, using
/// the largest-remainder method so the segments always add up to `width`.
/// Returns zeros when there are no tasks.
pub fn bar_segments(counts: [u64; 3], width: usize) -> [usize; 3] {
    let total: u64 = counts.iter().sum();
    if total == 0 || width == 0 {
        return [0; 3];
    }
    let width = width as u64;
    let mut cells = [0u64; 3];
    let mut remainders = [0u64; 3];
    for i in 0..3 {
        let exact = counts[i] * width;
        cells[i] = exact / total;
        remainders[i] = exact % total;
    }
    let mut order = [0usize, 1, 2];
    order.sort_by_key(|&i| std::cmp::Reverse(remainders[i]));
    let missing = width - cells.iter().sum::<u64>();
    for &i in order.iter().take(missing as usize) {
        cells[i] += 1;
    }
    cells.map(|c| c as usize)
}

/// The bar as styled spans, exactly `width` cells long. Without tasks the
/// whole bar is muted empty cells.
pub fn stacked_bar_spans(counts: [u64; 3], width: usize) -> Vec<Span<'static>> {
    let glyphs = glyphs::glyphs();
    if counts.iter().sum::<u64>() == 0 {
        return vec![Span::styled(
            glyphs.bar_empty.repeat(width),
            theme::theme().muted_style(),
        )];
    }
    let cells = bar_segments(counts, width);
    STATUS_ORDER
        .iter()
        .zip(cells)
        .filter(|(_, n)| *n > 0)
        .map(|(status, n)| Span::styled(glyphs.bar_full.repeat(n), status_style(*status)))
        .collect()
}

/// The stacked bar as a single line.
pub fn stacked_bar(counts: [u64; 3], width: usize) -> Line<'static> {
    Line::from(stacked_bar_spans(counts, width))
}

/// Legend under the bar: one coloured square and count per status, e.g.
/// `█ Todo 3   █ In Progress 1   █ Done 2`.
pub fn legend(counts: [u64; 3]) -> Line<'static> {
    let glyphs = glyphs::glyphs();
    let mut spans = Vec::new();
    for (i, (status, count)) in STATUS_ORDER.iter().zip(counts).enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(glyphs.bar_full, status_style(*status)));
        spans.push(Span::raw(format!(" {} {count}", status_label(*status))));
    }
    Line::from(spans)
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Todo => "Todo",
        Status::InProgress => "In Progress",
        Status::Done => "Done",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn segments_always_fill_the_width() {
        // 1/3 each: the leftover cell goes to the first segment on a tie.
        assert_eq!(bar_segments([1, 1, 1], 10), [4, 3, 3]);
        assert_eq!(bar_segments([1, 2, 4], 10).iter().sum::<usize>(), 10);
        assert_eq!(bar_segments([7, 0, 3], 10).iter().sum::<usize>(), 10);
        assert_eq!(bar_segments([3, 1, 1], 10), [6, 2, 2]);
        assert_eq!(bar_segments([0, 0, 4], 10), [0, 0, 10]);
    }

    #[test]
    fn segments_are_zero_without_tasks_or_width() {
        assert_eq!(bar_segments([0, 0, 0], 10), [0, 0, 0]);
        assert_eq!(bar_segments([1, 2, 3], 0), [0, 0, 0]);
    }

    #[test]
    fn empty_bar_is_muted_empty_cells() {
        let spans = stacked_bar_spans([0, 0, 0], 6);
        assert_eq!(text(&spans), "░░░░░░");
    }

    #[test]
    fn bar_has_exactly_the_requested_width() {
        for width in [0, 1, 2, 7, 40] {
            let spans = stacked_bar_spans([3, 1, 2], width);
            assert_eq!(text(&spans).chars().count(), width);
        }
    }

    #[test]
    fn legend_names_every_status_with_its_count() {
        let line = legend([3, 1, 2]);
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "█ Todo 3   █ In Progress 1   █ Done 2");
    }
}
