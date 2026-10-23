// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

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
use crate::screens::components::list_navigation::move_selection;
use crate::screens::components::progress::completion_percentage;
use crate::screens::components::scroll::{ListScroll, render_scrollbar};
use crate::screens::{AUTO_REFRESH_INTERVAL, Action, Component, Screen};

pub enum ProjectListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
}

pub struct ProjectListScreen {
    repo: Arc<dyn Repository>,
    projects: Vec<crate::domain::project::Project>,
    progress: Vec<u64>,
    selected: usize,
    scroll: ListScroll,
    mode: ProjectListMode,
}

impl ProjectListScreen {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
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
            progress: Vec::new(),
            selected: 0,
            scroll: ListScroll::new(),
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
        self.progress = self
            .projects
            .iter()
            .map(|project| {
                let project_id = project.id.unwrap_or(0);
                let tasks = self
                    .repo
                    .list_tasks_by_project(project_id)
                    .unwrap_or_default();
                completion_percentage(&tasks)
            })
            .collect();
        self.selected = self.selected.min(self.projects.len().saturating_sub(1));
        self.mode = ProjectListMode::Browsing;
        Ok(())
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn auto_refresh_interval(&self) -> Option<Duration> {
        matches!(&self.mode, ProjectListMode::Browsing).then_some(AUTO_REFRESH_INTERVAL)
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
                    KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k') => {
                        move_selection(&mut self.selected, self.projects.len(), *code);
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

    // Rows that fit inside the top and bottom borders.
    let row_height = area.height.saturating_sub(2) as usize;
    let range = screen
        .scroll
        .visible_range(screen.projects.len(), screen.selected, row_height);

    let items: Vec<ListItem> = screen.projects[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, p)| {
            let i = range.start + offset;
            let style = if i == screen.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let progress = screen.progress.get(i).copied().unwrap_or(0);
            let label = format!("{} ({}%)", p.name, progress);
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);

    // Scrollbar on the right border, aligned with the data rows.
    if row_height > 0 && area.width > 0 {
        let track = Rect::new(area.x + area.width - 1, area.y + 1, 1, row_height as u16);
        render_scrollbar(frame, track, screen.projects.len(), row_height, range.start);
    }
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
                (KeyCode::Char('j'), "Down"),
                (KeyCode::Char('k'), "Up"),
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
    fn j_and_k_navigate_projects_and_clamp_at_list_boundaries() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Alpha", "").unwrap();
        repo.create_project("Beta", "").unwrap();
        repo.create_project("Gamma", "").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();

        screen.handle_event(&char_event('k'));
        assert_eq!(screen.selected, 0);

        screen.handle_event(&char_event('j'));
        assert_eq!(screen.selected, 1);
        screen.handle_event(&key_event(KeyCode::Down));
        assert_eq!(screen.selected, 2);
        screen.handle_event(&char_event('j'));
        assert_eq!(screen.selected, 2);

        screen.handle_event(&char_event('k'));
        assert_eq!(screen.selected, 1);
    }

    #[test]
    fn auto_refresh_pauses_while_confirm_delete_is_open() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Alpha", "").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();

        assert_eq!(screen.auto_refresh_interval(), Some(AUTO_REFRESH_INTERVAL));
        screen.handle_event(&char_event('d'));
        assert_eq!(screen.auto_refresh_interval(), None);
    }

    #[test]
    fn selected_project_stays_visible_when_list_overflows() {
        let repo = Arc::new(StubRepository::default());
        for i in 0..30 {
            repo.create_project(&format!("Proj{i:02}"), "").unwrap();
        }
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();

        // Height 12 => 10 rows between the borders.
        let before = render_to_string(&screen, 40, 12);
        assert!(before.contains("Proj00"));
        assert!(!before.contains("Proj12"));

        for _ in 0..14 {
            screen.handle_event(&char_event('j'));
        }
        let after = render_to_string(&screen, 40, 12);
        assert_eq!(screen.selected, 14);
        assert!(after.contains("Proj14"), "selection not visible:\n{after}");
        assert!(
            !after.contains("Proj00"),
            "top row should have scrolled out:\n{after}"
        );
    }

    #[test]
    fn project_list_empty_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let screen = ProjectListScreen::new(repo);
        insta::assert_snapshot!(render_to_string(&screen, 60, 20));
    }
}
