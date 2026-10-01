// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `KeyEvent` constructors for declaring `KEYBINDINGS` tables without the
//! modifiers/kind/state boilerplate. The produced events are bit-identical to
//! the literals the screens used to write by hand: `kind` is
//! [`KeyEventKind::Press`] and `state` is [`KeyEventState::empty`].

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

/// A [`KeyEventKind::Press`] event for `code` with no modifiers.
pub const fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// A [`KeyEventKind::Press`] `Ctrl+<char>` event.
pub const fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{ctrl, key};

    #[test]
    fn key_builds_press_event_with_empty_modifiers_and_state() {
        let event = key(KeyCode::Char('c'));
        assert_eq!(
            event,
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            }
        );
    }

    #[test]
    fn key_preserves_non_char_codes() {
        let event = key(KeyCode::Enter);
        assert_eq!(event.code, KeyCode::Enter);
        assert_eq!(event.modifiers, KeyModifiers::empty());
        assert_eq!(event.kind, KeyEventKind::Press);
        assert_eq!(event.state, KeyEventState::empty());
    }

    #[test]
    fn ctrl_builds_press_event_with_control_modifier() {
        let event = ctrl('s');
        assert_eq!(
            event,
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            }
        );
    }

    #[test]
    fn constructors_are_usable_in_const_tables() {
        const TABLE: &[(KeyEvent, &str)] = &[(key(KeyCode::Esc), "Back"), (ctrl('s'), "Submit")];
        assert_eq!(TABLE[0].0.code, KeyCode::Esc);
        assert_eq!(TABLE[0].0.modifiers, KeyModifiers::empty());
        assert_eq!(TABLE[1].0.code, KeyCode::Char('s'));
        assert_eq!(TABLE[1].0.modifiers, KeyModifiers::CONTROL);
    }
}
