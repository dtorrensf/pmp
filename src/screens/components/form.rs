// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Key handling and layout shared by the project, version and task forms.
//! Each form keeps its own fields; these helpers hold the behaviour they have
//! in common: unwrapping key presses, Ctrl+S submit, Tab field cycling, and
//! editing a single-line or multi-line text field.

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear},
};
use ratatui_textarea::{CursorMove, TextArea};

use crate::screens::components::theme;

/// The code and modifiers of a key press, or `None` for any other event.
pub fn key_press(event: &Event) -> Option<(KeyCode, KeyModifiers)> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some((key.code, key.modifiers)),
        _ => None,
    }
}

/// Ctrl+S, the save shortcut shared by every form.
pub fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    code == KeyCode::Char('s') && modifiers.contains(KeyModifiers::CONTROL)
}

/// The field after `current` when Tab cycles through `count` fields.
pub fn next_field(current: u8, count: u8) -> u8 {
    (current + 1) % count
}

/// A text field a key can edit.
pub enum TextTarget<'a> {
    /// Single-line input such as a name or a new note.
    Line(&'a mut String),
    /// Multi-line description.
    Area(&'a mut TextArea<'static>),
}

/// Applies a text-editing key to `target`. Returns `true` when the key was
/// used. Single-line fields take characters and Backspace; multi-line fields
/// also take Enter, the arrow keys and their cursor movement.
pub fn handle_text_keys(target: TextTarget<'_>, code: KeyCode) -> bool {
    match (target, code) {
        (TextTarget::Line(text), KeyCode::Char(c)) => {
            text.push(c);
            true
        }
        (TextTarget::Line(text), KeyCode::Backspace) => {
            text.pop();
            true
        }
        (TextTarget::Line(_), _) => false,
        (TextTarget::Area(area), KeyCode::Char(c)) => {
            area.insert_char(c);
            true
        }
        (TextTarget::Area(area), KeyCode::Backspace) => {
            area.delete_char();
            true
        }
        (TextTarget::Area(area), KeyCode::Enter) => {
            area.insert_newline();
            true
        }
        (TextTarget::Area(area), KeyCode::Left) => {
            area.move_cursor(CursorMove::Back);
            true
        }
        (TextTarget::Area(area), KeyCode::Right) => {
            area.move_cursor(CursorMove::Forward);
            true
        }
        (TextTarget::Area(area), KeyCode::Up) => {
            area.move_cursor(CursorMove::Up);
            true
        }
        (TextTarget::Area(area), KeyCode::Down) => {
            area.move_cursor(CursorMove::Down);
            true
        }
        (TextTarget::Area(_), _) => false,
    }
}

/// Clears `area`, draws the bordered form frame titled `title` and returns
/// its inner rectangle, where the form lays out its fields.
pub fn render_form_frame(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme::theme().border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEvent, KeyEventState};
    use ratatui_textarea::TextArea;

    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
    }

    #[test]
    fn key_press_ignores_release_and_non_key_events() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Release,
            state: KeyEventState::empty(),
        });
        assert_eq!(key_press(&release), None);
        assert_eq!(key_press(&Event::FocusGained), None);
        assert_eq!(
            key_press(&press(KeyCode::Tab, KeyModifiers::empty())),
            Some((KeyCode::Tab, KeyModifiers::empty()))
        );
    }

    #[test]
    fn submit_requires_ctrl_s() {
        assert!(is_submit(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!is_submit(KeyCode::Char('s'), KeyModifiers::empty()));
        assert!(!is_submit(KeyCode::Char('x'), KeyModifiers::CONTROL));
    }

    #[test]
    fn next_field_wraps_around() {
        assert_eq!(next_field(0, 3), 1);
        assert_eq!(next_field(2, 3), 0);
        assert_eq!(next_field(5, 6), 0);
    }

    #[test]
    fn line_target_takes_chars_and_backspace_only() {
        let mut text = String::from("ab");
        assert!(handle_text_keys(
            TextTarget::Line(&mut text),
            KeyCode::Char('c')
        ));
        assert!(handle_text_keys(
            TextTarget::Line(&mut text),
            KeyCode::Backspace
        ));
        assert_eq!(text, "ab");
        assert!(!handle_text_keys(
            TextTarget::Line(&mut text),
            KeyCode::Enter
        ));
        assert!(!handle_text_keys(
            TextTarget::Line(&mut text),
            KeyCode::Left
        ));
        assert_eq!(text, "ab");
    }

    #[test]
    fn area_target_takes_text_newlines_and_cursor_moves() {
        let mut area = TextArea::default();
        handle_text_keys(TextTarget::Area(&mut area), KeyCode::Char('x'));
        handle_text_keys(TextTarget::Area(&mut area), KeyCode::Enter);
        handle_text_keys(TextTarget::Area(&mut area), KeyCode::Char('y'));
        handle_text_keys(TextTarget::Area(&mut area), KeyCode::Up);
        handle_text_keys(TextTarget::Area(&mut area), KeyCode::Backspace);
        assert_eq!(area.lines(), ["", "y"]);
        assert!(!handle_text_keys(TextTarget::Area(&mut area), KeyCode::Tab));
    }
}
