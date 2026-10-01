// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use super::Component;

pub fn key_event(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

pub fn char_event(c: char) -> Event {
    key_event(KeyCode::Char(c))
}

pub fn ctrl_s_event() -> Event {
    Event::Key(KeyEvent {
        code: KeyCode::Char('s'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

pub fn render_to_string(target: &impl Component, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| target.render(frame, frame.area()))
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

pub fn buffer_to_string(buffer: &Buffer) -> String {
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

#[cfg(test)]
mod tests {
    use ratatui::{Frame, layout::Rect, widgets::Block};

    use super::*;

    struct StubScreen;

    impl Component for StubScreen {
        fn render(&self, frame: &mut Frame, area: Rect) {
            frame.render_widget(Block::bordered().title("S"), area);
        }
    }

    #[test]
    fn key_event_is_press_with_empty_modifiers() {
        let Event::Key(event) = key_event(KeyCode::Enter) else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Enter);
        assert_eq!(event.modifiers, KeyModifiers::empty());
        assert_eq!(event.kind, KeyEventKind::Press);
        assert_eq!(event.state, KeyEventState::empty());
    }

    #[test]
    fn char_event_wraps_char_keycode() {
        let Event::Key(event) = char_event('x') else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Char('x'));
        assert_eq!(event.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn ctrl_s_event_is_char_s_with_control_modifier() {
        let Event::Key(event) = ctrl_s_event() else {
            panic!("expected a key event");
        };
        assert_eq!(event.code, KeyCode::Char('s'));
        assert_eq!(event.modifiers, KeyModifiers::CONTROL);
        assert_eq!(event.kind, KeyEventKind::Press);
    }

    #[test]
    fn buffer_to_string_trims_trailing_whitespace_per_line() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 2));
        buffer[(0, 0)].set_symbol("A");
        buffer[(1, 0)].set_symbol("B");

        let text = buffer_to_string(&buffer);

        assert_eq!(text.split('\n').collect::<Vec<_>>(), vec!["AB", ""]);
    }

    #[test]
    fn render_to_string_renders_target_at_explicit_size() {
        let output = render_to_string(&StubScreen, 20, 5);

        let lines: Vec<&str> = output.split('\n').collect();
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| line.chars().count() <= 20));
        assert!(output.contains('S'));
    }
}
