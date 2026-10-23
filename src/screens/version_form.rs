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
use crate::domain::version::VersionStatus;
use crate::screens::components::fields::{render_field, render_readonly_field};
use crate::screens::components::keys::{ctrl, key};
use crate::screens::{Action, Component, Screen};

/// Fields in tab order.
const NAME: u8 = 0;
const DESCRIPTION: u8 = 1;
const STATUS: u8 = 2;

pub struct VersionForm {
    repo: Arc<dyn Repository>,
    project_id: i64,
    version_id: Option<i64>,
    name: String,
    description: TextArea<'static>,
    status: VersionStatus,
    active_field: u8,
    return_to: Screen,
}

impl VersionForm {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (ctrl('s'), "Submit"),
        (key(KeyCode::Tab), "Next field"),
        (key(KeyCode::Esc), "Cancel"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            version_id: None,
            name: String::new(),
            description: TextArea::default(),
            status: VersionStatus::Open,
            active_field: NAME,
            return_to: Screen::ProjectList,
        }
    }

    pub fn set_project_id(&mut self, project_id: i64) {
        self.project_id = project_id;
    }

    pub fn set_version_id(&mut self, version_id: Option<i64>) {
        self.version_id = version_id;
    }

    pub fn set_return_to(&mut self, screen: &Screen) {
        self.return_to = screen.clone();
    }

    pub fn return_to(&self) -> &Screen {
        &self.return_to
    }
}

impl Component for VersionForm {
    fn init(&mut self) -> anyhow::Result<()> {
        if let Some(id) = self.version_id {
            let version = self.repo.get_version(id)?;
            self.project_id = version.project_id;
            self.name = version.name;
            let mut textarea = TextArea::default();
            textarea.insert_str(&version.description);
            self.description = textarea;
            self.status = version.status;
        } else {
            self.name = self.repo.next_version_name(self.project_id)?;
            self.description = TextArea::default();
            self.status = VersionStatus::Open;
        }
        self.active_field = NAME;
        Ok(())
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        else {
            return None;
        };

        if *code == KeyCode::Char('s') && modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Action::SaveVersion {
                version_id: self.version_id,
                project_id: self.project_id,
                name: self.name.clone(),
                description: self.description.lines().join("\n"),
                status: self.status,
            });
        }

        match code {
            KeyCode::Esc => Some(Action::Navigate(self.return_to.clone())),
            KeyCode::Tab => {
                self.active_field = (self.active_field + 1) % 3;
                None
            }
            KeyCode::Backspace => {
                match self.active_field {
                    NAME => {
                        self.name.pop();
                    }
                    DESCRIPTION => {
                        self.description.delete_char();
                    }
                    _ => {}
                }
                None
            }
            KeyCode::Left | KeyCode::Right if self.active_field == STATUS => {
                self.status = self.status.next();
                None
            }
            KeyCode::Left if self.active_field == DESCRIPTION => {
                self.description
                    .move_cursor(ratatui_textarea::CursorMove::Back);
                None
            }
            KeyCode::Right if self.active_field == DESCRIPTION => {
                self.description
                    .move_cursor(ratatui_textarea::CursorMove::Forward);
                None
            }
            KeyCode::Up if self.active_field == DESCRIPTION => {
                self.description
                    .move_cursor(ratatui_textarea::CursorMove::Up);
                None
            }
            KeyCode::Down if self.active_field == DESCRIPTION => {
                self.description
                    .move_cursor(ratatui_textarea::CursorMove::Down);
                None
            }
            KeyCode::Enter if self.active_field == DESCRIPTION => {
                self.description.insert_newline();
                None
            }
            KeyCode::Char(' ') if self.active_field == STATUS => {
                self.status = self.status.next();
                None
            }
            KeyCode::Char(c) => {
                match self.active_field {
                    NAME => self.name.push(*c),
                    DESCRIPTION => self.description.insert_char(*c),
                    _ => {}
                }
                None
            }
            _ => None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        let title = if self.version_id.is_some() {
            "Edit Version"
        } else {
            "Create Version"
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(7),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner);

        render_field(
            frame,
            chunks[0],
            "Name",
            &self.name,
            self.active_field == NAME,
            false,
        );

        let desc_style = if self.active_field == DESCRIPTION {
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

        render_readonly_field(
            frame,
            chunks[2],
            "Status",
            self.status.label(),
            self.active_field == STATUS,
        );

        if self.active_field == NAME {
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

    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::screens::test_utils::{
        buffer_to_string, char_event, ctrl_s_event, key_event, render_to_string,
    };
    use crate::screens::{Action, Component, Screen};

    use super::*;

    fn repo_with_project() -> (Arc<StubRepository>, i64) {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        (repo, project_id)
    }

    fn form_for_create() -> VersionForm {
        let (repo, project_id) = repo_with_project();
        let mut form = VersionForm::new(repo);
        form.set_project_id(project_id);
        form.init().unwrap();
        form
    }

    fn form_for_edit() -> (VersionForm, Arc<StubRepository>) {
        let (repo, project_id) = repo_with_project();
        let version = repo
            .create_version(project_id, "v1.0.0", "Description")
            .unwrap();
        let mut form = VersionForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_version_id(version.id);
        form.init().unwrap();
        (form, repo)
    }

    #[test]
    fn init_suggests_next_name_when_create() {
        let (repo, project_id) = repo_with_project();
        repo.create_version(project_id, "v0.1.0", "").unwrap();
        let mut form = VersionForm::new(repo);
        form.set_project_id(project_id);
        form.init().unwrap();
        assert_eq!(form.name, "v0.2.0");
        assert_eq!(form.status, VersionStatus::Open);
        assert_eq!(form.active_field, NAME);
    }

    #[test]
    fn init_loads_on_edit() {
        let (form, _repo) = form_for_edit();
        assert_eq!(form.name, "v1.0.0");
        assert_eq!(form.description.lines().join("\n"), "Description");
        assert_eq!(form.status, VersionStatus::Open);
        assert!(form.version_id.is_some());
    }

    #[test]
    fn init_clears_state_on_create_after_edit() {
        let (repo, project_id) = repo_with_project();
        let mut form = VersionForm::new(repo);
        form.set_project_id(project_id);
        form.init().unwrap();
        form.handle_event(&char_event('X'));
        form.init().unwrap();
        assert_eq!(form.name, "v0.1.0");
    }

    #[test]
    fn tab_rotates_through_all_three_fields() {
        let mut form = form_for_create();
        form.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(form.active_field, DESCRIPTION);
        form.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(form.active_field, STATUS);
        form.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(form.active_field, NAME);
    }

    #[test]
    fn arrows_toggle_status_when_status_field_active() {
        let mut form = form_for_create();
        form.active_field = STATUS;
        form.handle_event(&key_event(KeyCode::Right));
        assert_eq!(form.status, VersionStatus::Released);
        form.handle_event(&key_event(KeyCode::Left));
        assert_eq!(form.status, VersionStatus::Open);
    }

    #[test]
    fn typing_is_ignored_on_status_field() {
        let mut form = form_for_create();
        form.active_field = STATUS;
        let before = form.name.clone();
        form.handle_event(&char_event('z'));
        assert_eq!(form.name, before);
    }

    #[test]
    fn submit_emits_save_version_for_create() {
        let mut form = form_for_create();
        let project_id = form.project_id;
        let action = form.handle_event(&ctrl_s_event());
        assert_eq!(
            action,
            Some(Action::SaveVersion {
                version_id: None,
                project_id,
                name: "v0.1.0".to_string(),
                description: String::new(),
                status: VersionStatus::Open,
            })
        );
    }

    #[test]
    fn submit_emits_save_version_with_status_on_edit() {
        let (mut form, _repo) = form_for_edit();
        form.active_field = STATUS;
        form.handle_event(&key_event(KeyCode::Right));
        let action = form.handle_event(&ctrl_s_event());
        let Some(Action::SaveVersion {
            version_id, status, ..
        }) = action
        else {
            panic!("expected SaveVersion");
        };
        assert!(version_id.is_some());
        assert_eq!(status, VersionStatus::Released);
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
    fn renders_fields_and_title() {
        let form = form_for_create();
        let text = render_to_string(&form, 60, 20);
        assert!(text.contains("Create Version"));
        assert!(text.contains("Name"));
        assert!(text.contains("Description"));
        assert!(text.contains("Status"));
        assert!(text.contains("v0.1.0"));
    }

    #[test]
    fn edit_mode_renders_edit_title() {
        let (form, _repo) = form_for_edit();
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Edit Version"));
        assert!(text.contains("v1.0.0"));
    }

    #[test]
    fn version_form_default_snapshot() {
        let form = form_for_create();
        insta::assert_snapshot!(render_to_string(&form, 60, 20));
    }

    #[test]
    fn version_form_with_text_snapshot() {
        let mut form = form_for_create();
        form.handle_event(&char_event('!'));
        form.handle_event(&key_event(KeyCode::Tab));
        for c in "First cut".chars() {
            form.handle_event(&char_event(c));
        }
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&key_event(KeyCode::Right));
        insta::assert_snapshot!(render_to_string(&form, 60, 20));
    }

    #[test]
    fn keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = VersionForm::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![
                (KeyCode::Char('s'), "Submit"),
                (KeyCode::Tab, "Next field"),
                (KeyCode::Esc, "Cancel"),
            ]
        );
    }
}
