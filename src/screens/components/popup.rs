//! Shared centered-popup helpers: fixed-size and percentage-based centering
//! geometry, plus the common scaffold (`Clear` + titled `Borders::ALL` block)
//! used by the task-list filter popup and the task-form dependency picker.

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear},
};

/// Returns the `width`x`height` rect centered inside `area`, clamped to
/// `area` when the requested size exceeds it.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = width.min(area.width);
    let popup_height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

/// Returns a rect spanning `percent`% of `area`'s width and height, centered
/// inside `area`. Dimensions are truncated (not rounded) and the origin is
/// floored, replicating the exact `f32` arithmetic the dependency picker
/// popup has always used. `percent` is expected to be at most 100; larger
/// values saturate the origin at `area`'s top-left instead of underflowing.
pub fn centered_rect_percent(percent: u16, area: Rect) -> Rect {
    let ratio = percent as f32 / 100.0;
    let popup_width = (area.width as f32 * ratio) as u16;
    let popup_height = (area.height as f32 * ratio) as u16;
    Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    }
}

/// Clears `area`, draws a `Borders::ALL` block titled `title` over it and
/// returns the block's inner rect for the caller's content.
pub fn render_popup_scaffold(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    use crate::screens::test_utils::buffer_to_string;

    use super::{centered_rect, centered_rect_percent, render_popup_scaffold};

    #[test]
    fn centered_rect_centers_even_dimensions() {
        let area = Rect::new(0, 0, 80, 24);

        assert_eq!(centered_rect(52, 11, area), Rect::new(14, 6, 52, 11));
    }

    #[test]
    fn centered_rect_floors_origin_for_odd_slack() {
        let area = Rect::new(0, 0, 81, 25);

        assert_eq!(centered_rect(52, 11, area), Rect::new(14, 7, 52, 11));
    }

    #[test]
    fn centered_rect_centers_odd_popup_dimensions() {
        let area = Rect::new(0, 0, 80, 24);

        assert_eq!(centered_rect(51, 9, area), Rect::new(14, 7, 51, 9));
    }

    #[test]
    fn centered_rect_clamps_popup_to_area() {
        let area = Rect::new(0, 0, 80, 24);

        assert_eq!(centered_rect(100, 30, area), Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn centered_rect_respects_area_origin() {
        let area = Rect::new(4, 2, 20, 10);

        assert_eq!(centered_rect(10, 4, area), Rect::new(9, 5, 10, 4));
    }

    #[test]
    fn centered_rect_percent_replicates_dep_popup_math_at_60x20() {
        let area = Rect::new(0, 0, 60, 20);

        assert_eq!(centered_rect_percent(60, area), Rect::new(12, 4, 36, 12));
    }

    #[test]
    fn centered_rect_percent_replicates_dep_popup_math_at_80x24() {
        let area = Rect::new(0, 0, 80, 24);

        assert_eq!(centered_rect_percent(60, area), Rect::new(16, 5, 48, 14));
    }

    #[test]
    fn centered_rect_percent_truncates_fractional_dimensions() {
        let area = Rect::new(0, 0, 61, 25);

        assert_eq!(centered_rect_percent(60, area), Rect::new(12, 5, 36, 15));
    }

    #[test]
    fn centered_rect_percent_respects_area_origin() {
        let area = Rect::new(10, 4, 50, 20);

        assert_eq!(centered_rect_percent(60, area), Rect::new(20, 8, 30, 12));
    }

    fn render_scaffold(area: Rect, title: &str, width: u16, height: u16) -> (String, Rect) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut inner = None;
        terminal
            .draw(|frame| {
                let full = frame.area();
                for y in full.y..full.bottom() {
                    for x in full.x..full.right() {
                        frame.buffer_mut()[(x, y)].set_symbol("X");
                    }
                }
                inner = Some(render_popup_scaffold(frame, area, title));
            })
            .unwrap();
        (
            buffer_to_string(terminal.backend().buffer()),
            inner.unwrap(),
        )
    }

    #[test]
    fn render_popup_scaffold_clears_draws_titled_border_and_returns_inner() {
        let (output, inner) = render_scaffold(Rect::new(2, 1, 10, 5), "Popup", 20, 8);

        assert_eq!(inner, Rect::new(3, 2, 8, 3));
        let expected = [
            "XXXXXXXXXXXXXXXXXXXX",
            "XX┌Popup───┐XXXXXXXX",
            "XX│        │XXXXXXXX",
            "XX│        │XXXXXXXX",
            "XX│        │XXXXXXXX",
            "XX└────────┘XXXXXXXX",
            "XXXXXXXXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXXXXXXXX",
        ]
        .join("\n");
        assert_eq!(output, expected);
    }
}
