use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::db::repository::Repository;
use crate::screens::components::confirm_dialog::ConfirmDialog;
use crate::screens::components::keys::key;
use crate::screens::{Action, Component, Screen};

pub enum ProjectListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
}

pub struct ProjectListScreen {
    repo: Arc<dyn Repository>,
    projects: Vec<crate::domain::project::Project>,
    selected: usize,
    mode: ProjectListMode,
}

impl ProjectListScreen {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Enter), "Open"),
        (key(KeyCode::Char('c')), "Create"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('e')), "Edit"),
        (key(KeyCode::Char('r')), "Refresh"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            projects: Vec::new(),
            selected: 0,
            mode: ProjectListMode::Browsing,
        }
    }

    pub fn projects(&self) -> &[crate::domain::project::Project] {
        &self.projects
    }
}

impl Component for ProjectListScreen {
    fn init(&mut self) -> anyhow::Result<()> {
        self.projects = self.repo.list_projects()?;
        self.selected = self.selected.min(self.projects.len().saturating_sub(1));
        self.mode = ProjectListMode::Browsing;
        Ok(())
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match &self.mode {
                ProjectListMode::Browsing => match code {
                    KeyCode::Char('q') => Some(Action::Quit),
                    KeyCode::Char('r') => Some(Action::Refresh),
                    KeyCode::Char('c') => {
                        Some(Action::Navigate(Screen::ProjectForm { project_id: None }))
                    }
                    KeyCode::Char('d') => {
                        if let Some(project) = self.projects.get(self.selected) {
                            let project_id = project.id.unwrap_or(0);
                            let message = format!(
                                "Delete project {}? This will remove all its tasks ⚠️. (y/N)",
                                project.name
                            );
                            self.mode = ProjectListMode::ConfirmDelete(ConfirmDialog::new(
                                message,
                                Action::DeleteProject { project_id },
                            ));
                        }
                        None
                    }
                    KeyCode::Char('e') => self
                        .projects
                        .get(self.selected)
                        .map(|p| Action::Navigate(Screen::ProjectForm { project_id: p.id })),
                    KeyCode::Enter => self.projects.get(self.selected).map(|p| {
                        Action::Navigate(Screen::ProjectDashboard {
                            project_id: p.id.unwrap_or(0),
                        })
                    }),
                    KeyCode::Up => {
                        if self.selected > 0 {
                            self.selected -= 1;
                        }
                        None
                    }
                    KeyCode::Down => {
                        if self.selected + 1 < self.projects.len() {
                            self.selected += 1;
                        }
                        None
                    }
                    _ => None,
                },
                ProjectListMode::ConfirmDelete(dialog) => {
                    let action = dialog.handle_key(*code);
                    self.mode = ProjectListMode::Browsing;
                    action
                }
            }
        } else {
            None
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        match &self.mode {
            ProjectListMode::Browsing => render_list(self, frame, area),
            ProjectListMode::ConfirmDelete(dialog) => dialog.render(frame, area),
        }
    }
}

fn render_list(screen: &ProjectListScreen, frame: &mut Frame, area: Rect) {
    let block = Block::default().title("Projects").borders(Borders::ALL);
    if screen.projects.is_empty() {
        let paragraph = Paragraph::new("No projects yet. Create one? (press 'c')")
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = screen
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == screen.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(p.name.clone(), style)))
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::screens::test_utils::{buffer_to_string, char_event, key_event, render_to_string};
    use crate::screens::{Component, Screen};

    use super::*;

    #[test]
    fn empty_state_shows_create_hint() {
        let repo = Arc::new(StubRepository::default());
        let screen = ProjectListScreen::new(repo);
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("No projects yet"));
        assert!(text.contains("press 'c'"));
    }

    #[test]
    fn enter_on_project_navigates_to_dashboard() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard {
                project_id: project.id.unwrap()
            }))
        );
    }

    #[test]
    fn q_key_emits_quit_action() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Alpha", "First project").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let action = screen.handle_event(&char_event('q'));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn e_key_navigates_to_project_form_edit() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let action = screen.handle_event(&char_event('e'));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectForm {
                project_id: project.id
            }))
        );
    }

    #[test]
    fn c_key_navigates_to_project_form() {
        let repo = Arc::new(StubRepository::default());
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Char('c')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectForm { project_id: None }))
        );
    }

    #[test]
    fn keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = ProjectListScreen::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![
                (KeyCode::Enter, "Open"),
                (KeyCode::Char('c'), "Create"),
                (KeyCode::Char('d'), "Delete"),
                (KeyCode::Char('e'), "Edit"),
                (KeyCode::Char('r'), "Refresh"),
                (KeyCode::Char('q'), "Quit"),
            ]
        );
    }

    #[test]
    fn r_key_emits_refresh_action() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Alpha", "First").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let action = screen.handle_event(&char_event('r'));
        assert_eq!(action, Some(Action::Refresh));
    }

    #[test]
    fn project_list_empty_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let screen = ProjectListScreen::new(repo);
        insta::assert_snapshot!(render_to_string(&screen, 60, 20));
    }
}
