// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The destructive-action confirmation modal.
//!
//! The dialog is drawn on top of the screen that opened it, dimming the
//! background instead of erasing it, so the user keeps the context of what
//! is about to be deleted. It only answers to its own keys: `y` confirms,
//! `n` and `Esc` cancel, and every other key is ignored (the dialog stays
//! open), which keeps a stray keypress from discarding the confirmation.

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::screens::Action;
use crate::screens::components::glyphs;
use crate::screens::components::popup::{centered_rect, dim_area, render_popup_scaffold_with};
use crate::screens::components::theme;

/// Default title of the confirmation modal.
pub const DEFAULT_TITLE: &str = "Confirm";

/// Narrowest modal, in terminal columns.
const MIN_WIDTH: u16 = 30;
/// Widest modal, in terminal columns. Longer messages wrap.
const MAX_WIDTH: u16 = 60;
/// Left and right padding inside the border, in columns.
const HORIZONTAL_PADDING: u16 = 1;
/// Border (2) plus horizontal padding on both sides.
const HORIZONTAL_CHROME: u16 = 2 + 2 * HORIZONTAL_PADDING;
/// Border (2), one blank line and the actions line.
const VERTICAL_CHROME: u16 = 4;

/// The outcome of a key pressed while the dialog is open.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAnswer {
    /// `y` was pressed: run the dialog's action.
    Confirmed(Action),
    /// `n` or `Esc` was pressed: close the dialog without doing anything.
    Cancelled,
    /// Any other key: keep the dialog open.
    Ignored,
}

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    title: String,
    message: String,
    on_confirm: Action,
}

impl ConfirmDialog {
    pub fn new(message: impl Into<String>, on_confirm: Action) -> Self {
        Self {
            title: DEFAULT_TITLE.to_string(),
            message: message.into(),
            on_confirm,
        }
    }

    /// Overrides the title shown in the modal's top border.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn on_confirm(&self) -> &Action {
        &self.on_confirm
    }

    /// Answers the dialog with `code`. Only `y`, `n` and `Esc` are owned by
    /// the dialog; everything else is [`ConfirmAnswer::Ignored`].
    pub fn handle_key(&self, code: KeyCode) -> ConfirmAnswer {
        match code {
            KeyCode::Char('y') => ConfirmAnswer::Confirmed(self.on_confirm.clone()),
            KeyCode::Char('n') | KeyCode::Esc => ConfirmAnswer::Cancelled,
            _ => ConfirmAnswer::Ignored,
        }
    }

    /// Dims the background and draws the modal centered in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        dim_area(frame);

        let (width, height) = self.geometry(area);
        let modal_area = centered_rect(width, height, area);
        let theme = theme::theme();
        let danger = theme.danger_style();
        let title = Line::from(Span::styled(
            format!(" {} {} ", glyphs::glyphs().warning, self.title),
            danger.add_modifier(Modifier::BOLD),
        ));
        let inner = render_popup_scaffold_with(frame, modal_area, title, danger);

        // Horizontal padding inside the border.
        let inner = Rect {
            x: inner.x + HORIZONTAL_PADDING,
            width: inner.width.saturating_sub(2 * HORIZONTAL_PADDING),
            ..inner
        };

        let message = Paragraph::new(Text::from(self.message.clone()))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(message, inner);

        let actions = actions_line().alignment(Alignment::Center);
        let actions_area = Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        };
        frame.render_widget(Paragraph::new(actions), actions_area);
    }

    /// Width and height of the modal, adapted to the message and clamped to
    /// `area` so narrow terminals never overflow.
    fn geometry(&self, area: Rect) -> (u16, u16) {
        let message_width = self
            .message
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0) as u16;
        // The actions line must never be truncated either.
        let actions_width = actions_line().width() as u16;
        let width = message_width
            .max(actions_width)
            .saturating_add(HORIZONTAL_CHROME)
            .clamp(MIN_WIDTH, MAX_WIDTH)
            .min(area.width.max(1));
        let inner_width = width.saturating_sub(HORIZONTAL_CHROME).max(1);
        let lines = wrapped_line_count(&self.message, inner_width);
        let height = lines
            .saturating_add(VERTICAL_CHROME)
            .min(area.height.max(1));
        (width, height)
    }
}

/// The explicit actions offered by the dialog, with the accent on the
/// confirming key and the muted style on the cancelling keys.
fn actions_line() -> Line<'static> {
    let theme = theme::theme();
    Line::from(vec![
        Span::styled("[y]", theme.accent_style().add_modifier(Modifier::BOLD)),
        Span::raw(" Confirm   "),
        Span::styled("[n/Esc]", theme.muted_style()),
        Span::raw(" Cancel"),
    ])
}

/// Number of terminal lines `text` needs when word-wrapped at `width`.
/// Explicit newlines are honored and an empty string still occupies one line.
fn wrapped_line_count(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let mut lines = 0u16;
    for paragraph in text.split('\n') {
        let mut current = 0usize;
        let mut has_word = false;
        for word in paragraph.split_whitespace() {
            let word_width = word.width();
            if !has_word {
                current = word_width;
                has_word = true;
            } else if current + 1 + word_width <= width {
                current += 1 + word_width;
            } else {
                lines = lines.saturating_add(1);
                current = word_width;
            }
        }
        lines = lines.saturating_add(1);
    }
    lines.max(1)
}

/// The confirmation a screen currently shows, if any. While it is open the
/// screen sends every key to it; only `y`, `n` and `Esc` close it.
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

    /// Answers the open dialog with `code` and closes it only when the key is
    /// one the dialog owns. Returns the action to run when `code` confirms it,
    /// and `None` otherwise or when no dialog is open.
    pub fn answer(&mut self, code: KeyCode) -> Option<Action> {
        let dialog = self.0.as_ref()?;
        match dialog.handle_key(code) {
            ConfirmAnswer::Confirmed(action) => {
                self.0 = None;
                Some(action)
            }
            ConfirmAnswer::Cancelled => {
                self.0 = None;
                None
            }
            ConfirmAnswer::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        style::Modifier,
        widgets::{Block, Paragraph},
    };

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

    const DELETE_TASK: Action = Action::DeleteTask { task_id: 42 };

    #[test]
    fn y_key_emits_on_confirm_action() {
        let dialog = ConfirmDialog::new("Delete task X?", DELETE_TASK);

        assert_eq!(
            dialog.handle_key(KeyCode::Char('y')),
            ConfirmAnswer::Confirmed(DELETE_TASK)
        );
    }

    #[test]
    fn n_and_esc_cancel() {
        let dialog = ConfirmDialog::new("Delete task X?", DELETE_TASK);

        assert_eq!(
            dialog.handle_key(KeyCode::Char('n')),
            ConfirmAnswer::Cancelled
        );
        assert_eq!(dialog.handle_key(KeyCode::Esc), ConfirmAnswer::Cancelled);
    }

    #[test]
    fn any_other_key_is_ignored() {
        let dialog = ConfirmDialog::new("Delete task X?", DELETE_TASK);

        assert_eq!(dialog.handle_key(KeyCode::Enter), ConfirmAnswer::Ignored);
        assert_eq!(
            dialog.handle_key(KeyCode::Char('Y')),
            ConfirmAnswer::Ignored
        );
        assert_eq!(
            dialog.handle_key(KeyCode::Char('N')),
            ConfirmAnswer::Ignored
        );
        assert_eq!(
            dialog.handle_key(KeyCode::Char('q')),
            ConfirmAnswer::Ignored
        );
    }

    #[test]
    fn default_title_is_confirm_and_can_be_overridden() {
        let dialog = ConfirmDialog::new("Delete?", DELETE_TASK);
        assert_eq!(dialog.title(), DEFAULT_TITLE);
        assert_eq!(dialog.with_title("Delete task").title(), "Delete task");
    }

    #[test]
    fn render_confirm_dialog_snapshot() {
        let dialog = ConfirmDialog::new("Delete task T3?", Action::DeleteTask { task_id: 1 })
            .with_title("Delete task");
        insta::assert_snapshot!(render_dialog(&dialog, 60, 24));
    }

    #[test]
    fn render_dims_the_background_but_not_the_modal() {
        let dialog = ConfirmDialog::new("Delete task T3?", DELETE_TASK);
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("background"), frame.area());
                dialog.render(frame, frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // A corner far from the modal keeps its text and is dimmed.
        assert_eq!(buffer[(0, 0)].symbol(), "b");
        assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
        // The modal title is not dimmed: the dialog dims first, then draws.
        let (title_x, title_y) = (0..60)
            .flat_map(|x| (0..24).map(move |y| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "⚠")
            .expect("modal title glyph");
        assert!(!buffer[(title_x, title_y)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn narrow_areas_are_clamped_without_panicking() {
        let dialog = ConfirmDialog::new(
            "A very long message that would never fit in a tiny terminal area",
            DELETE_TASK,
        );
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dialog.render(frame, frame.area()))
            .unwrap();

        let (width, height) = dialog.geometry(Rect::new(0, 0, 10, 5));
        assert_eq!((width, height), (10, 5));
    }

    #[test]
    fn empty_messages_still_occupy_one_line() {
        assert_eq!(wrapped_line_count("", 10), 1);
        assert_eq!(wrapped_line_count("short", 10), 1);
        assert_eq!(wrapped_line_count("one two three four", 7), 3);
        assert_eq!(wrapped_line_count("line\nline", 10), 2);
    }

    #[test]
    fn answer_cancels_and_closes_with_n_or_esc() {
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut state = ConfirmState::default();
            state.open(ConfirmDialog::new("Delete?", DELETE_TASK));
            assert!(state.is_open());
            assert_eq!(state.answer(code), None);
            assert!(!state.is_open());
        }
    }

    #[test]
    fn answer_keeps_the_dialog_open_on_unowned_keys() {
        let mut state = ConfirmState::default();
        state.open(ConfirmDialog::new("Delete?", DELETE_TASK));
        assert_eq!(state.answer(KeyCode::Char('x')), None);
        assert!(state.is_open());
    }

    #[test]
    fn answer_confirms_with_y_and_closes() {
        let mut state = ConfirmState::default();
        state.open(ConfirmDialog::new("Delete?", DELETE_TASK));
        assert_eq!(state.answer(KeyCode::Char('y')), Some(DELETE_TASK));
        assert!(!state.is_open());
    }

    #[test]
    fn answer_without_dialog_is_none() {
        let mut state = ConfirmState::default();
        assert_eq!(state.answer(KeyCode::Char('y')), None);
    }

    #[test]
    fn dialog_does_not_replace_the_background() {
        let dialog = ConfirmDialog::new("Delete task T3?", DELETE_TASK);
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Block::bordered().title("Behind"), frame.area());
                dialog.render(frame, frame.area());
            })
            .unwrap();
        let output = buffer_to_string(terminal.backend().buffer());
        assert!(output.contains("Behind"), "{output}");
        assert!(output.contains("Delete task T3?"), "{output}");
        assert!(output.contains("[y] Confirm"), "{output}");
        assert!(output.contains("[n/Esc] Cancel"), "{output}");
    }
}
