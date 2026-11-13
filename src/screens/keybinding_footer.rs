// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::screens::components::theme;

/// Separator between two footer entries.
const SEPARATOR: &str = "  ";
/// Text of the overflow indicator, shown when not every shortcut fits.
const HELP_HINT_KEY: &str = "?";
const HELP_HINT_LABEL: &str = " ayuda";

/// Renders the shortcut footer. Entries that do not fit are dropped from the
/// end. When `help_hint` is set and something was dropped, a `? ayuda`
/// indicator is drawn at the right so the full list stays discoverable.
pub fn render_footer(frame: &mut Frame, area: Rect, keys: &[(KeyEvent, &str)], help_hint: bool) {
    let line = footer_line(keys, area.width as usize, help_hint);
    frame.render_widget(Paragraph::new(line), area);
}

/// Groups the keys that share a label and returns `(key labels, label)` pairs
/// in first-seen order.
pub(crate) fn group_keys<'a>(keys: &[(KeyEvent, &'a str)]) -> Vec<(Vec<String>, &'a str)> {
    let mut grouped: Vec<(Vec<String>, &str)> = Vec::new();
    for (event, label) in keys {
        if let Some(existing) = grouped.iter_mut().find(|(_, l)| l == label) {
            existing.0.push(key_label(event));
        } else {
            grouped.push((vec![key_label(event)], *label));
        }
    }
    grouped
}

/// Builds the footer line for `width` columns. Key names are accented and
/// bold, labels are muted.
pub(crate) fn footer_line(
    keys: &[(KeyEvent, &str)],
    width: usize,
    help_hint: bool,
) -> Line<'static> {
    let theme = theme::theme();
    let entries: Vec<(String, &str)> = group_keys(keys)
        .into_iter()
        .map(|(events, label)| (events.join("/"), label))
        .collect();
    let entry_width = |(key, label): &(String, &str)| key.width() + ": ".width() + label.width();
    let sep = SEPARATOR.width();
    let hint_width = HELP_HINT_KEY.width() + HELP_HINT_LABEL.width();

    let full: usize =
        entries.iter().map(entry_width).sum::<usize>() + sep * entries.len().saturating_sub(1);
    let overflow = full > width;
    let show_hint = overflow && help_hint;
    // Room for the entries: the whole line, or the line minus the indicator.
    let budget = if show_hint {
        width.saturating_sub(hint_width + sep)
    } else {
        width
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        let needed = if index == 0 {
            entry_width(entry)
        } else {
            sep + entry_width(entry)
        };
        if used + needed > budget {
            break;
        }
        if index > 0 {
            spans.push(Span::styled(SEPARATOR, theme.muted_style()));
        }
        spans.push(Span::styled(
            entry.0.clone(),
            theme
                .accent_style()
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(": {}", entry.1), theme.muted_style()));
        used += needed;
    }

    if show_hint && hint_width <= width {
        if !spans.is_empty() {
            spans.push(Span::styled(SEPARATOR, theme.muted_style()));
        }
        spans.push(Span::styled(
            HELP_HINT_KEY,
            theme
                .accent_style()
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
        spans.push(Span::styled(HELP_HINT_LABEL, theme.muted_style()));
    }
    Line::from(spans)
}

fn key_label(event: &KeyEvent) -> String {
    let prefix = if event.modifiers.contains(KeyModifiers::CONTROL) {
        "Ctrl+"
    } else if event.modifiers.contains(KeyModifiers::ALT) {
        "Alt+"
    } else {
        ""
    };
    let key = match event.code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Char(c) => c.to_string(),
        _ => format!("{:?}", event.code),
    };
    format!("{prefix}{key}")
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let area = buffer.area();
        let mut lines = Vec::new();
        for y in area.y..area.y + area.height {
            let mut line = String::new();
            for x in area.x..area.x + area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }

    fn render_to_string(keys: &[(KeyEvent, &str)], width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_footer(frame, frame.area(), keys, true))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn footer_renders_keybinding_labels() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        let text = render_to_string(keys, 80, 1);
        assert!(text.contains("Enter: Open"));
        assert!(text.contains("c: Create"));
        assert!(text.contains("q: Quit"));
    }

    #[test]
    fn footer_snapshot_at_80_cols() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        insta::assert_snapshot!(render_to_string(keys, 80, 1));
    }

    #[test]
    fn footer_truncates_when_narrow() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        let text = render_to_string(keys, 30, 1);
        assert!(!text.is_empty());
        assert!(text.chars().count() <= 30);
    }

    #[test]
    fn empty_keybindings_renders_empty_bar() {
        let keys: &[(KeyEvent, &str)] = &[];
        let text = render_to_string(keys, 80, 1);
        assert!(text.trim().is_empty());
    }

    #[test]
    fn footer_never_panics_on_extreme_sizes() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        render_to_string(keys, 1, 1);
        render_to_string(keys, 200, 1);
        render_to_string(&[], 0, 0);
    }

    #[test]
    fn footer_groups_labels() {
        let keys = &[
            (key_event(KeyCode::Char('q')), "Quit"),
            (key_event(KeyCode::Esc), "Quit"),
        ];
        let text = render_to_string(keys, 80, 1);
        assert!(text.contains("q/Esc: Quit"));
    }

    #[test]
    fn footer_uses_the_muted_theme_color_for_contrast() {
        let keys = &[(key_event(KeyCode::Char('q')), "Quit")];
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_footer(frame, frame.area(), keys, true))
            .unwrap();

        // "q: Quit": the key is accented, the label (starting at column 3) is muted.
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 0)].fg, theme::theme().muted);
        // DarkGray was too dim on light terminals.
        assert_ne!(buffer[(3, 0)].fg, ratatui::style::Color::DarkGray);
    }

    #[test]
    fn footer_accents_the_key_and_mutes_the_label() {
        let keys = &[(key_event(KeyCode::Char('q')), "Quit")];
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_footer(frame, frame.area(), keys, true))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].fg, theme::theme().accent);
        assert!(
            buffer[(0, 0)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn footer_shows_no_hint_when_everything_fits() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        let text = render_to_string(keys, 80, 1);
        assert!(!text.contains("ayuda"));
    }

    #[test]
    fn footer_overflow_shows_help_hint_and_stays_in_width() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        let text = render_to_string(keys, 30, 1);
        assert!(text.trim_end().ends_with("? ayuda"), "{text:?}");
        assert!(text.trim_end().chars().count() <= 30);
    }

    #[test]
    fn footer_overflow_without_help_hint_shows_no_indicator() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_footer(frame, frame.area(), keys, false))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(!text.contains("ayuda"));
        assert!(text.starts_with("Enter: Open"));
    }

    #[test]
    fn footer_drops_whole_entries_instead_of_cutting_them() {
        let keys = &[
            (key_event(KeyCode::Enter), "Open"),
            (key_event(KeyCode::Char('c')), "Create"),
            (key_event(KeyCode::Char('q')), "Quit"),
        ];
        // "Enter: Open  ? ayuda" fits in 20 columns, "c: Create" does not.
        let text = render_to_string(keys, 20, 1);
        assert_eq!(text.trim_end(), "Enter: Open  ? ayuda");
    }
}
