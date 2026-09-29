use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

pub fn render_footer(frame: &mut Frame, area: Rect, keys: &[(KeyEvent, &str)]) {
    let mut grouped: Vec<(Vec<String>, &str)> = Vec::new();
    for (event, label) in keys {
        if let Some(existing) = grouped.iter_mut().find(|(_, l)| l == label) {
            existing.0.push(key_label(event));
        } else {
            grouped.push((vec![key_label(event)], *label));
        }
    }

    let text = grouped
        .into_iter()
        .map(|(events, label)| format!("{}: {}", events.join("/"), label))
        .collect::<Vec<_>>()
        .join("  ");
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
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
            .draw(|frame| render_footer(frame, frame.area(), keys))
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
}
