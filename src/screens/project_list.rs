// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use unicode_width::UnicodeWidthStr;

use crate::application;
use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::task::Status;
use crate::screens::components::confirm_dialog::{ConfirmDialog, ConfirmState};
use crate::screens::components::empty_state::EmptyState;
use crate::screens::components::glyphs;
use crate::screens::components::help::{self, HelpSection};
use crate::screens::components::keys::key;
use crate::screens::components::list_navigation::move_selection;
use crate::screens::components::motion::Tween;
use crate::screens::components::progress::percent;
use crate::screens::components::progress_bar::ProgressBar;
use crate::screens::components::scroll::{ListScroll, list_rows, render_list_scrollbar};
use crate::screens::components::stacked_bar::status_counts;
use crate::screens::components::theme::{self, status_style};
use crate::screens::{AUTO_REFRESH_INTERVAL, Action, Component, Screen};

pub struct ProjectListScreen {
    repo: Arc<dyn Repository>,
    projects: Vec<crate::domain::project::Project>,
    /// Tasks per status for each project, see `status_counts`.
    task_counts: Vec<[u64; 3]>,
    version_counts: Vec<usize>,
    /// Eased fill of each project's progress bar, indexed like `projects`.
    bars: Vec<Tween>,
    selected: usize,
    scroll: ListScroll,
    confirm: ConfirmState,
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
            task_counts: Vec::new(),
            version_counts: Vec::new(),
            bars: Vec::new(),
            selected: 0,
            scroll: ListScroll::new(),
            confirm: ConfirmState::default(),
        }
    }

    pub fn projects(&self) -> &[crate::domain::project::Project] {
        &self.projects
    }
}

impl Component for ProjectListScreen {
    fn tick(&mut self) {
        let targets: Vec<f64> = self
            .task_counts
            .iter()
            .map(|counts| {
                let tasks: u64 = counts.iter().sum();
                percent(counts[2], tasks) as f64 / 100.0
            })
            .collect();
        for (i, target) in targets.iter().enumerate() {
            match self.bars.get_mut(i) {
                Some(bar) => bar.set(*target),
                None => self.bars.push(Tween::settled(*target)),
            }
        }
        self.bars.truncate(targets.len());
    }

    fn init(&mut self) -> anyhow::Result<()> {
        self.projects = application::projects::list(self.repo.as_ref())?;
        self.task_counts = self
            .projects
            .iter()
            .map(|project| {
                let project_id = project.id.unwrap_or(0);
                let tasks = application::tasks::list_by_project(self.repo.as_ref(), project_id)?;
                Ok(status_counts(&tasks))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.version_counts = self
            .projects
            .iter()
            .map(|project| {
                Ok(application::versions::list(self.repo.as_ref(), project.id.unwrap_or(0))?.len())
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        self.selected = self.selected.min(self.projects.len().saturating_sub(1));
        self.confirm.close();
        Ok(())
    }

    /// Projects are the subject here, so create/edit/delete go under Projects.
    fn help_sections(&self) -> Vec<HelpSection> {
        help::sections(self.keybindings(), |label| match label {
            "Down" | "Up" | "Open" => help::NAVIGATION,
            "Create" | "Edit" | "Delete" => help::PROJECTS,
            _ => help::APPLICATION,
        })
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn auto_refresh_interval(&self) -> Option<Duration> {
        (!self.confirm.is_open()).then_some(AUTO_REFRESH_INTERVAL)
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            if self.confirm.is_open() {
                return self.confirm.answer(*code);
            }
            match code {
                KeyCode::Char('q') => Some(Action::Quit),
                KeyCode::Char('r') => Some(Action::Refresh),
                KeyCode::Char('c') => {
                    Some(Action::Navigate(Screen::ProjectForm { project_id: None }))
                }
                KeyCode::Char('d') => {
                    if let Some(project) = self.projects.get(self.selected) {
                        let project_id = project.id.unwrap_or(0);
                        let message = format!(
                            "Delete project {}? This will remove all its tasks.",
                            project.name
                        );
                        self.confirm.open(
                            ConfirmDialog::new(message, Action::DeleteProject { project_id })
                                .with_title("Delete project"),
                        );
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
            }
        } else {
            None
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        // The list stays visible, dimmed, behind the confirmation modal.
        render_list(self, frame, area);
        if let Some(dialog) = self.confirm.dialog() {
            dialog.render(frame, area);
        }
    }
}

fn render_list(screen: &ProjectListScreen, frame: &mut Frame, area: Rect) {
    let block = Block::default().title("Projects").borders(Borders::ALL);
    if screen.projects.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        EmptyState::new("No projects yet")
            .with_icon(glyphs::glyphs().status_todo)
            .with_hint("Press c to create one.")
            .render(frame, inner);
        return;
    }

    // Rows that fit inside the top and bottom borders.
    let row_height = list_rows(area, 0);
    let range = screen
        .scroll
        .visible_range(screen.projects.len(), screen.selected, row_height);

    // The name column is padded to the longest name so the bars line up.
    let name_width = screen
        .projects
        .iter()
        .map(|p| p.name.width())
        .max()
        .unwrap_or(0);

    let items: Vec<ListItem> = screen.projects[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, p)| {
            let i = range.start + offset;
            let (style, marker) = if i == screen.selected {
                (theme::theme().selection_style(), glyphs::glyphs().selection)
            } else {
                (Style::default(), " ")
            };
            let counts = screen.task_counts.get(i).copied().unwrap_or([0; 3]);
            let tasks: u64 = counts.iter().sum();
            let percentage = percent(counts[2], tasks);
            let versions = screen.version_counts.get(i).copied().unwrap_or(0);
            // The label shows the real percentage; the bar eases toward it.
            let ratio = screen
                .bars
                .get(i)
                .map_or(percentage as f64 / 100.0, Tween::value);
            let bar = ProgressBar::new(ratio, PROGRESS_BAR_WIDTH)
                .with_filled_style(status_style(Status::Done))
                .with_empty_style(theme::theme().muted_style());

            let mut spans = vec![
                Span::raw(format!("{marker} {}", pad_display(&p.name, name_width))),
                Span::raw("  "),
            ];
            spans.extend(bar.line().spans);
            spans.push(Span::raw(format!(" {percentage:>3}%  ")));
            spans.push(Span::raw(format!(
                "{tasks} {} · {versions} {}",
                if tasks == 1 { "task" } else { "tasks" },
                if versions == 1 { "version" } else { "versions" },
            )));
            ListItem::new(Line::from(spans).style(style))
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);

    render_list_scrollbar(frame, area, 0, screen.projects.len(), range.start);
}

/// Width, in cells, of the progress bar of each project.
const PROGRESS_BAR_WIDTH: usize = 10;

/// Right-pads `text` with spaces to `width` terminal columns.
fn pad_display(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(fill))
}

#[cfg(test)]
mod tests {
    use crate::db::repository::*;
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
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
        assert!(text.contains("Press c to create one."), "{text}");
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
    fn delete_confirmation_avoids_the_two_column_warning_emoji() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Alpha", "").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();

        screen.handle_event(&char_event('d'));
        let text = render_to_string(&screen, 60, 20);

        // The `⚠️` emoji measures two columns and misaligns the text; the
        // plain warning sign is exactly one column wide.
        assert!(!text.contains("⚠️"), "{text}");
        assert!(text.contains('⚠'), "{text}");
        assert!(text.contains("Delete project Alpha?"), "{text}");
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

    #[test]
    fn rows_show_progress_bar_and_task_count() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let vid = repo.default_version_id(pid).unwrap();
        repo.create_task(pid, vid, "A", "", Status::Done, Priority::Low)
            .unwrap();
        repo.create_task(pid, vid, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("█████░░░░░  50%  2 tasks"), "{text}");
    }

    #[test]
    fn project_without_tasks_shows_empty_bar() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("Empty", "").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("░░░░░░░░░░   0%  0 tasks"), "{text}");
    }

    #[test]
    fn narrow_rows_do_not_panic() {
        let repo = Arc::new(StubRepository::default());
        repo.create_project("A very long project name", "").unwrap();
        let mut screen = ProjectListScreen::new(repo);
        screen.init().unwrap();
        for width in [1, 5, 12] {
            let backend = TestBackend::new(width, 4);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| screen.render(frame, frame.area()))
                .unwrap();
        }
    }
}
