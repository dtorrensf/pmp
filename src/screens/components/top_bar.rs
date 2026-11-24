// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! One-line bar above the active screen: the app name and version on the
//! left, the breadcrumb of the current context in the middle and the update
//! badge on the right. The same breadcrumb feeds the terminal window title.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::screens::UiContext;
use crate::screens::task_list_view::truncate_display;

/// Below this width the version is hidden so the breadcrumb keeps the room.
const VERSION_MIN_WIDTH: u16 = 100;
const SEPARATOR: &str = " › ";
const APP_NAME: &str = "PMP";

/// Joins the breadcrumb segments into the terminal window title, for example
/// `pmp — Alpha › mvp`.
pub fn window_title(breadcrumb: &[String]) -> String {
    if breadcrumb.is_empty() {
        return "pmp".to_string();
    }
    format!("pmp — {}", breadcrumb.join(SEPARATOR))
}

/// Draws the top bar in `area`. `version` is the running pmp version and
/// `update` the newer version offered by the background check, if any.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    breadcrumb: &[String],
    version: &str,
    update: Option<&str>,
    ui: &UiContext,
) {
    let brand = if area.width >= VERSION_MIN_WIDTH {
        format!("{APP_NAME} v{version}")
    } else {
        APP_NAME.to_string()
    };
    let update_text = update.map(|v| format!("⬆ New v{v} (u)"));
    let brand_width = brand.width() as u16 + 2;
    let update_width = update_text
        .as_ref()
        .map_or(0, |text| text.width() as u16 + 2);

    let parts = Layout::horizontal([
        Constraint::Length(brand_width),
        Constraint::Min(0),
        Constraint::Length(update_width),
    ])
    .split(area);

    let brand_line =
        Paragraph::new(format!(" {brand}")).style(ui.accent_style().add_modifier(Modifier::BOLD));
    frame.render_widget(brand_line, parts[0]);

    let crumb = truncate_display(&breadcrumb.join(SEPARATOR), parts[1].width as usize);
    let crumb_line = Paragraph::new(crumb)
        .style(ui.text_style().add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center);
    frame.render_widget(crumb_line, parts[1]);

    if let Some(text) = update_text {
        let badge = Paragraph::new(format!("{text} "))
            .style(
                ui.pulse_style(ui.theme().warning)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Right);
        frame.render_widget(badge, parts[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn crumbs(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    fn render_to_string(width: u16, breadcrumb: &[String], update: Option<&str>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    breadcrumb,
                    "0.5.0",
                    update,
                    &UiContext::default(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect::<String>()
    }

    #[test]
    fn window_title_joins_the_breadcrumb() {
        let title = window_title(&crumbs(&["Alpha", "mvp"]));
        assert_eq!(title, "pmp — Alpha › mvp");
    }

    #[test]
    fn window_title_without_context_is_the_app_name() {
        assert_eq!(window_title(&[]), "pmp");
    }

    #[test]
    fn wide_bar_shows_version_and_centered_breadcrumb() {
        let text = render_to_string(120, &crumbs(&["Alpha", "mvp"]), None);
        assert!(text.contains("PMP v0.5.0"), "{text}");
        let crumb_at = text.find("Alpha › mvp").expect("breadcrumb drawn");
        let column = text[..crumb_at].width();
        assert!((40..70).contains(&column), "{text}");
    }

    #[test]
    fn narrow_bar_hides_version_and_keeps_breadcrumb() {
        let text = render_to_string(80, &crumbs(&["Alpha", "mvp"]), None);
        assert!(!text.contains("v0.5.0"), "{text}");
        assert!(text.contains("Alpha › mvp"), "{text}");
    }

    #[test]
    fn long_breadcrumb_is_truncated_not_overflowing() {
        let long = "x".repeat(200);
        let text = render_to_string(60, &crumbs(&[&long]), None);
        assert!(text.contains('…'), "{text}");
        assert_eq!(text.width(), 60);
    }

    #[test]
    fn update_badge_is_drawn_on_the_right_when_available() {
        let text = render_to_string(120, &crumbs(&["Projects"]), Some("0.6.0"));
        assert!(text.trim_end().ends_with("⬆ New v0.6.0 (u)"), "{text}");
    }

    #[test]
    fn no_update_means_no_badge() {
        let text = render_to_string(120, &crumbs(&["Projects"]), None);
        assert!(!text.contains("New v"), "{text}");
    }
}
