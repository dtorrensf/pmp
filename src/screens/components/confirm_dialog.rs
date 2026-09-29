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
}
