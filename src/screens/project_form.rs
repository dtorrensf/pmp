// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear},
};
use ratatui_textarea::TextArea;

use crate::db::repository::Repository;
use crate::screens::components::fields::render_field;
use crate::screens::components::keys::{ctrl, key};
use crate::screens::{Action, Component, Screen};

pub struct ProjectForm {
    repo: Arc<dyn Repository>,
    project_id: Option<i64>,
    name: String,
    description: TextArea<'static>,
    active_field: u8,
    return_to: Screen,
}

impl ProjectForm {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] =
        &[(ctrl('s'), "Submit"), (key(KeyCode::Esc), "Cancel")];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: None,
            name: String::new(),
            description: TextArea::default(),
            active_field: 0,
            return_to: Screen::ProjectList,
        }
    }

    pub fn set_project_id(&mut self, project_id: Option<i64>) {
        self.project_id = project_id;
    }

    pub fn set_return_to(&mut self, screen: &Screen) {
        self.return_to = screen.clone();
    }

    pub fn return_to(&self) -> &Screen {
        &self.return_to
    }
}

impl Component for ProjectForm {
    fn init(&mut self) -> anyhow::Result<()> {
        if let Some(id) = self.project_id {
            let project = self.repo.get_project(id)?;
            self.name = project.name;
            let mut textarea = TextArea::default();
            for line in project.description.lines() {
                if !textarea.lines().join("").is_empty() {
                    textarea.insert_newline();
                }
                for c in line.chars() {
                    textarea.insert_char(c);
                }
            }
            self.description = textarea;
        } else {
            self.name.clear();
            self.description = TextArea::default();
        }
        self.active_field = 0;
        Ok(())
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            if *code == KeyCode::Char('s') && modifiers.contains(KeyModifiers::CONTROL) {
                return Some(Action::SaveProject {
                    project_id: self.project_id,
                    name: self.name.clone(),
                    description: self.description.lines().join("\n"),
                });
            }

            match code {
                KeyCode::Esc => Some(Action::Navigate(self.return_to.clone())),
                KeyCode::Tab => {
                    self.active_field = 1 - self.active_field;
                    None
                }
                KeyCode::Backspace => {
                    if self.active_field == 0 {
                        self.name.pop();
                    } else {
                        self.description.delete_char();
                    }
                    None
                }
                KeyCode::Left => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Back);
                    }
                    None
                }
                KeyCode::Right => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Forward);
                    }
                    None
                }
                KeyCode::Up => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Up);
                    }
                    None
                }
                KeyCode::Down => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Down);
                    }
                    None
                }
                KeyCode::Enter => {
                    if self.active_field == 1 {
                        self.description.insert_newline();
                    }
                    None
                }
                KeyCode::Char(c) => {
                    if self.active_field == 0 {
                        self.name.push(*c);
                    } else {
                        self.description.insert_char(*c);
                    }
                    None
                }
                _ => None,
            }
        } else {
            None
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        let title = if self.project_id.is_some() {
            "Edit Project"
        } else {
            "Create Project"
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(7),
                Constraint::Min(0),
            ])
            .split(inner);

        render_field(
            frame,
            chunks[0],
            "Name",
            &self.name,
            self.active_field == 0,
            false,
        );

        let desc_style = if self.active_field == 1 {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let desc_block = Block::default()
            .title("Description")
            .borders(Borders::ALL)
            .border_style(desc_style);
        let mut desc_textarea = self.description.clone();
        desc_textarea.set_block(desc_block);
        frame.render_widget(&desc_textarea, chunks[1]);

        if self.active_field == 0 {
            let x = chunks[0].x + 1 + self.name.chars().count() as u16;
            let y = chunks[0].y + 1;
            frame.set_cursor_position((x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::screens::test_utils::{
        buffer_to_string, char_event, ctrl_s_event, key_event, render_to_string,
    };
    use crate::screens::{Component, Screen};

    use super::*;

    fn form_for_create() -> ProjectForm {
        let repo = Arc::new(StubRepository::default());
        let mut form = ProjectForm::new(repo.clone());
        form.init().unwrap();
        form
    }

    fn form_for_edit() -> ProjectForm {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "Description").unwrap();
        let mut form = ProjectForm::new(repo.clone());
        form.set_project_id(project.id);
        form.init().unwrap();
        form
    }

    #[test]
    fn init_clears_form_state_when_create() {
        let mut form = form_for_create();
        form.handle_event(&char_event('A'));
        form.init().unwrap();
        assert!(form.name.is_empty());
        assert_eq!(form.active_field, 0);
    }

    #[test]
    fn init_loads_project_data_when_edit() {
        let mut form = form_for_edit();
        form.set_return_to(&Screen::ProjectList);
        assert_eq!(form.name, "Alpha");
        assert_eq!(form.description.lines().join("\n"), "Description");
    }

    #[test]
    fn submit_emits_save_project_action() {
        let mut form = form_for_create();
        form.handle_event(&char_event('A'));
        form.handle_event(&char_event('l'));
        form.handle_event(&char_event('p'));
        form.handle_event(&char_event('h'));
        form.handle_event(&char_event('a'));
        let action = form.handle_event(&ctrl_s_event());
        if let Some(Action::SaveProject {
            project_id,
            name,
            description,
        }) = &action
        {
            assert!(project_id.is_none());
            assert_eq!(name, "Alpha");
            assert!(description.is_empty());
        } else {
            panic!("Expected SaveProject action");
        }
    }

    #[test]
    fn submit_with_project_id_emits_save_project_with_id() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "Desc").unwrap();
        let mut form = ProjectForm::new(repo);
        form.set_project_id(project.id);
        form.init().unwrap();
        let action = form.handle_event(&ctrl_s_event());
        if let Some(Action::SaveProject {
            project_id, name, ..
        }) = &action
        {
            assert_eq!(*project_id, project.id);
            assert_eq!(name, "Alpha");
        } else {
            panic!("Expected SaveProject action");
        }
    }

    #[test]
    fn esc_returns_to_return_to_screen() {
        let mut form = form_for_create();
        form.set_return_to(&Screen::ProjectDashboard { project_id: 42 });
        let action = form.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard {
                project_id: 42
            }))
        );
    }

    #[test]
    fn tab_switches_active_field() {
        let mut form = form_for_create();
        assert_eq!(form.active_field, 0);
        form.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(form.active_field, 1);
        form.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(form.active_field, 0);
    }

    #[test]
    fn text_input_goes_to_active_field() {
        let mut form = form_for_create();
        form.handle_event(&char_event('A'));
        assert_eq!(form.name, "A");
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&char_event('B'));
        assert_eq!(form.name, "A");
        assert_eq!(form.description.lines().join("\n"), "B");
    }

    #[test]
    fn renders_name_and_description_fields() {
        let form = form_for_create();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Create Project"));
        assert!(text.contains("Name"));
        assert!(text.contains("Description"));
    }

    #[test]
    fn edit_mode_renders_edit_title() {
        let form = form_for_edit();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Edit Project"));
        assert!(text.contains("Alpha"));
    }

    #[test]
    fn project_form_default_snapshot() {
        let form = form_for_create();
        insta::assert_snapshot!(render_to_string(&form, 60, 20));
    }

    #[test]
    fn project_form_with_text_snapshot() {
        let mut form = form_for_create();
        form.handle_event(&char_event('M'));
        form.handle_event(&char_event('y'));
        form.handle_event(&char_event(' '));
        form.handle_event(&char_event('P'));
        form.handle_event(&char_event('r'));
        form.handle_event(&char_event('o'));
        form.handle_event(&char_event('j'));
        form.handle_event(&char_event('e'));
        form.handle_event(&char_event('c'));
        form.handle_event(&char_event('t'));
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&char_event('A'));
        form.handle_event(&char_event(' '));
        form.handle_event(&char_event('d'));
        form.handle_event(&char_event('e'));
        form.handle_event(&char_event('s'));
        form.handle_event(&char_event('c'));
        form.handle_event(&char_event('r'));
        form.handle_event(&char_event('i'));
        form.handle_event(&char_event('p'));
        form.handle_event(&char_event('t'));
        form.handle_event(&char_event('i'));
        form.handle_event(&char_event('o'));
        form.handle_event(&char_event('n'));
        insta::assert_snapshot!(render_to_string(&form, 60, 20));
    }

    #[test]
    fn keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = ProjectForm::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![(KeyCode::Char('s'), "Submit"), (KeyCode::Esc, "Cancel")]
        );
    }
}
