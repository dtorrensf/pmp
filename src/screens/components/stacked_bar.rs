// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! A full-width bar split into Todo / In Progress / Done segments, plus the
//! legend that names them with their counts.
//!
//! Every task chart in the app uses this module, so the same status always
//! has the same colour and the same proportion everywhere.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::domain::task::{Status, Task};
use crate::screens::components::glyphs;
use crate::screens::components::progress::percent;
use crate::screens::components::progress_bar::ProgressBar;
use crate::screens::components::theme::{self, status_style};
use unicode_width::UnicodeWidthStr;

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

/// One row per status: label, a bar of its share of all tasks, the count and
/// the percentage, e.g. `Todo         ███████░░░░░   3  ( 50%)`. Every row uses the
/// same scale (the total of tasks), so the rows compare at a glance.
/// `bar_width` is the width of the bar in cells, without the label and the
/// count; see `STATUS_ROW_OVERHEAD`.
pub fn status_rows(counts: [u64; 3], bar_width: usize) -> Vec<Line<'static>> {
    let total: u64 = counts.iter().sum();
    STATUS_ORDER
        .iter()
        .zip(counts)
        .map(|(status, count)| {
            share_row(
                None,
                status_label(*status),
                count,
                total,
                status_style(*status),
                bar_width,
            )
        })
        .collect()
}

/// A row of the shared layout: an optional icon, the label, a bar showing
/// `count` out of `total` and the count with its percentage on the right.
/// Rows built with the same `bar_width` line up column by column.
pub fn share_row(
    icon: Option<(&'static str, Style)>,
    label: &str,
    count: u64,
    total: u64,
    style: Style,
    bar_width: usize,
) -> Line<'static> {
    let ratio = if total == 0 {
        0.0
    } else {
        count as f64 / total as f64
    };
    let bar = ProgressBar::new(ratio, bar_width)
        .with_filled_style(style)
        .with_empty_style(theme::theme().muted_style());

    // The icon, when present, takes its own cell and a space before the label.
    let mut spans = Vec::new();
    let mut used = 0;
    if let Some((glyph, glyph_style)) = icon {
        spans.push(Span::styled(glyph, glyph_style));
        spans.push(Span::raw(" "));
        used = glyph.width() + 1;
    }
    let padding = STATUS_LABEL_WIDTH.saturating_sub(used + label.width());
    spans.push(Span::raw(format!("{label}{}", " ".repeat(padding))));
    spans.extend(bar.line().spans);
    spans.push(Span::raw(format!(
        "  {count:>3}  ({:>3}%)",
        percent(count, total)
    )));
    Line::from(spans)
}

/// Columns used by a status row besides its bar: the label and the count with
/// the percentage.
pub const STATUS_ROW_OVERHEAD: usize = STATUS_LABEL_WIDTH + STATUS_SUFFIX_WIDTH;

const STATUS_LABEL_WIDTH: usize = 12;
/// `"  {count:>3}  ({pct:>3}%)"`: the count and the percentage after the bar.
const STATUS_SUFFIX_WIDTH: usize = 13;

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
    fn status_rows_have_label_bar_count_and_percentage() {
        let rows = status_rows([3, 1, 2], 10);
        let plain: Vec<String> = rows
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(plain[0], "Todo        █████░░░░░    3  ( 50%)");
        assert_eq!(plain[1], "In Progress █▌░░░░░░░░    1  ( 17%)");
        assert_eq!(plain[2], "Done        ███▌░░░░░░    2  ( 33%)");
    }

    #[test]
    fn status_rows_without_tasks_are_empty() {
        let rows = status_rows([0, 0, 0], 4);
        assert_eq!(rows.len(), 3);
        let first: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(first, "Todo        ░░░░    0  (  0%)");
    }
}
