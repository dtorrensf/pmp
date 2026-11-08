// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::Text,
    widgets::{Clear, Paragraph, Wrap},
};

use crate::screens::Action;

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    message: String,
    on_confirm: Action,
}

impl ConfirmDialog {
    pub fn new(message: impl Into<String>, on_confirm: Action) -> Self {
        Self {
            message: message.into(),
            on_confirm,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn on_confirm(&self) -> &Action {
        &self.on_confirm
    }

    pub fn handle_key(&self, code: KeyCode) -> Option<Action> {
        match code {
            KeyCode::Char('y') => Some(self.on_confirm.clone()),
            _ => None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        let paragraph = Paragraph::new(Text::from(self.message.clone()))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }
}

/// The confirmation a screen currently shows, if any. While it is open the
/// screen sends every key to it, and the first key closes it.
#[derive(Debug, Default, Clone)]
pub struct ConfirmState(Option<ConfirmDialog>);

impl ConfirmState {
    /// Opens `dialog`, replacing any dialog already open.
    pub fn open(&mut self, dialog: ConfirmDialog) {
        self.0 = Some(dialog);
    }

    pub fn is_open(&self) -> bool {
        self.0.is_some()
    }

    pub fn dialog(&self) -> Option<&ConfirmDialog> {
        self.0.as_ref()
    }

    pub fn close(&mut self) {
        self.0 = None;
    }

    /// Answers the open dialog with `code` and closes it. Returns the action
    /// to run when `code` confirms it, and `None` otherwise or when no dialog
    /// is open.
    pub fn answer(&mut self, code: KeyCode) -> Option<Action> {
        self.0.take()?.handle_key(code)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use ratatui::{Terminal, backend::TestBackend};

    use crate::screens::Action;
    use crate::screens::test_utils::buffer_to_string;

    use super::*;

    fn render_dialog(dialog: &ConfirmDialog, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dialog.render(frame, frame.area()))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn y_key_emits_on_confirm_action() {
        let action = Action::DeleteTask { task_id: 42 };
        let dialog = ConfirmDialog::new("Delete task X? (y/N)", action.clone());

        assert_eq!(dialog.handle_key(KeyCode::Char('y')), Some(action));
    }

    #[test]
    fn any_other_key_returns_none() {
        let dialog = ConfirmDialog::new("Delete task X? (y/N)", Action::DeleteTask { task_id: 42 });

        assert_eq!(dialog.handle_key(KeyCode::Char('n')), None);
        assert_eq!(dialog.handle_key(KeyCode::Enter), None);
        assert_eq!(dialog.handle_key(KeyCode::Esc), None);
        assert_eq!(dialog.handle_key(KeyCode::Char('Y')), None);
    }

    #[test]
    fn render_confirm_dialog_snapshot() {
        let dialog = ConfirmDialog::new("Delete task T3? (y/N)", Action::DeleteTask { task_id: 1 });
        insta::assert_snapshot!(render_dialog(&dialog, 60, 24));
    }

    #[test]
    fn answer_closes_the_dialog_on_any_key() {
        let action = Action::DeleteTask { task_id: 7 };
        let mut state = ConfirmState::default();
        state.open(ConfirmDialog::new("Delete?", action.clone()));
        assert!(state.is_open());
        assert_eq!(state.answer(KeyCode::Char('n')), None);
        assert!(!state.is_open());
    }

    #[test]
    fn answer_confirms_with_y_and_closes() {
        let action = Action::DeleteTask { task_id: 7 };
        let mut state = ConfirmState::default();
        state.open(ConfirmDialog::new("Delete?", action.clone()));
        assert_eq!(state.answer(KeyCode::Char('y')), Some(action));
        assert!(!state.is_open());
    }

    #[test]
    fn answer_without_dialog_is_none() {
        let mut state = ConfirmState::default();
        assert_eq!(state.answer(KeyCode::Char('y')), None);
    }
}
