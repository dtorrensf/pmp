// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};

use crate::db::repository::Repository;
use crate::domain::project::Project;
use crate::domain::task::{Status, Task};
use crate::domain::version::Version;
use crate::screens::components::confirm_dialog::ConfirmDialog;
use crate::screens::components::keys::key;
use crate::screens::components::list_navigation::move_selection;
use crate::screens::components::progress::completion_percentage;
use crate::screens::components::scroll::{
    ListScroll, TextScroll, render_scrollbar, wrapped_line_count,
};
use crate::screens::{AUTO_REFRESH_INTERVAL, Action, Component, Screen};

/// One row of the versions list: the version plus its task progress.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionRow {
    pub version: Version,
    pub done: u64,
    pub total: u64,
    pub percentage: u64,
}

impl VersionRow {
    /// Label shown in the list, e.g. `v0.1.0  3/8 (37%)  Open`.
    pub fn label(&self) -> String {
        format!(
            "{}  {}/{} ({}%)  {}",
            self.version.name,
            self.done,
            self.total,
            self.percentage,
            self.version.status.label()
        )
    }
}

pub struct ProjectDashboard {
    repo: Arc<dyn Repository>,
    project_id: i64,
    project: Option<Project>,
    /// Every task of the project; charts and the gauge aggregate over these.
    all_tasks: Vec<Task>,
    versions: Vec<VersionRow>,
    selected: usize,
    confirm_delete: Option<ConfirmDialog>,
    description_scroll: TextScroll,
    versions_scroll: ListScroll,
}

impl ProjectDashboard {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
        (key(KeyCode::Up), "Up"),
        (key(KeyCode::Down), "Down"),
        (key(KeyCode::Enter), "Open"),
        (key(KeyCode::Char('c')), "New Version"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('r')), "Cycle Status"),
        (key(KeyCode::Char('e')), "Edit Project"),
        (key(KeyCode::PageDown), "Scroll description"),
        (key(KeyCode::PageUp), "Scroll description"),
        (key(KeyCode::Esc), "Back"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            project: None,
            all_tasks: Vec::new(),
            versions: Vec::new(),
            selected: 0,
            confirm_delete: None,
            description_scroll: TextScroll::new(),
            versions_scroll: ListScroll::new(),
        }
    }

    pub fn set_project_id(&mut self, project_id: i64) {
        self.project_id = project_id;
    }

    /// Versions of the project with their progress, open ones first.
    pub fn versions(&self) -> &[VersionRow] {
        &self.versions
    }

    pub fn selected_version(&self) -> usize {
        self.selected
    }

    pub fn set_selected_version(&mut self, index: usize) {
        self.selected = index.min(self.versions.len().saturating_sub(1));
    }

    /// Every task of the project, across versions.
    pub fn all_tasks(&self) -> &[Task] {
        &self.all_tasks
    }

    pub fn is_confirming_delete(&self) -> bool {
        self.confirm_delete.is_some()
    }

    fn load(&mut self) -> anyhow::Result<()> {
        self.project = Some(self.repo.get_project(self.project_id)?);
        self.all_tasks = self.repo.list_tasks_by_project(self.project_id)?;
        let versions = self.repo.list_versions_by_project(self.project_id)?;
        self.versions = versions
            .into_iter()
            .map(|version| {
                let tasks: Vec<&Task> = self
                    .all_tasks
                    .iter()
                    .filter(|t| t.version_id == version.id.unwrap_or(-1))
                    .collect();
                let total = tasks.len() as u64;
                let done = tasks.iter().filter(|t| t.status == Status::Done).count() as u64;
                let percentage = if total == 0 {
                    0
                } else {
                    (done as f64 / total as f64 * 100.0).round() as u64
                };
                VersionRow {
                    version,
                    done,
                    total,
                    percentage,
                }
            })
            .collect();
        self.selected = self.selected.min(self.versions.len().saturating_sub(1));
        self.description_scroll.reset();
        Ok(())
    }

    fn selected_row(&self) -> Option<&VersionRow> {
        self.versions.get(self.selected)
    }
}

impl Component for ProjectDashboard {
    fn init(&mut self) -> anyhow::Result<()> {
        self.confirm_delete = None;
        self.load()
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.load()
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn auto_refresh_interval(&self) -> Option<Duration> {
        self.confirm_delete
            .is_none()
            .then_some(AUTO_REFRESH_INTERVAL)
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        else {
            return None;
        };
        let code = *code;

        if let Some(dialog) = &self.confirm_delete {
            let action = dialog.handle_key(code);
            self.confirm_delete = None;
            return action;
        }

        match code {
            KeyCode::PageUp => {
                self.description_scroll.page_up();
                return None;
            }
            KeyCode::PageDown => {
                self.description_scroll.page_down();
                return None;
            }
            _ => {}
        }

        let project_id = self.project_id;
        match code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down => {
                move_selection(&mut self.selected, self.versions.len(), code);
                None
            }
            KeyCode::Enter => self.selected_row().and_then(|row| {
                row.version.id.map(|version_id| {
                    Action::Navigate(Screen::VersionDashboard {
                        project_id,
                        version_id,
                    })
                })
            }),
            KeyCode::Char('c') => Some(Action::Navigate(Screen::VersionForm {
                project_id,
                version_id: None,
            })),
            KeyCode::Char('e') => Some(Action::Navigate(Screen::ProjectForm {
                project_id: Some(project_id),
            })),
            KeyCode::Char('d') => {
                let row = self.selected_row()?;
                let version_id = row.version.id?;
                let message = format!(
                    "Delete version {}? Its tasks will be removed. (y/N)",
                    row.version.name
                );
                self.confirm_delete = Some(ConfirmDialog::new(
                    message,
                    Action::DeleteVersion { version_id },
                ));
                None
            }
            KeyCode::Char('r') => {
                let row = self.selected_row()?;
                Some(Action::SaveVersion {
                    version_id: row.version.id,
                    project_id,
                    name: row.version.name.clone(),
                    description: row.version.description.clone(),
                    status: row.version.status.next(),
                })
            }
            KeyCode::Backspace | KeyCode::Esc => Some(Action::Navigate(Screen::ProjectList)),
            _ => None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let project_name = self
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("Project")
            .to_uppercase();
        let project_description = self
            .project
            .as_ref()
            .map(|p| p.description.as_str())
            .unwrap_or("");

        let outer_block = Block::default()
            .title(project_name.to_string())
            .borders(Borders::ALL);
        let inner = outer_block.inner(area);
        frame.render_widget(outer_block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(9),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner);

        let desc_block = Block::default().borders(Borders::BOTTOM);
        let desc_inner = desc_block.inner(chunks[0]);
        let desc_offset = self.description_scroll.sync(
            wrapped_line_count(project_description, desc_inner.width),
            desc_inner.height,
        );
        let desc = Paragraph::new(Text::from(project_description.to_string()))
            .block(desc_block)
            .wrap(Wrap { trim: true })
            .scroll((desc_offset, 0));
        frame.render_widget(desc, chunks[0]);

        let counts = status_counts(&self.all_tasks);
        let status_bars = vec![
            Bar::default()
                .value(counts[0])
                .label(Line::from("Todo"))
                .style(Status::Todo.style()),
            Bar::default()
                .value(counts[1])
                .label(Line::from("In Progress"))
                .style(Status::InProgress.style()),
            Bar::default()
                .value(counts[2])
                .label(Line::from("Done"))
                .style(Status::Done.style()),
        ];
        let status_chart = BarChart::default()
            .block(Block::default().title("Status").borders(Borders::ALL))
            .data(BarGroup::default().bars(&status_bars))
            .bar_width(10);
        frame.render_widget(status_chart, chunks[1]);

        let total = self.all_tasks.len() as u64;
        let done = counts[2];
        let percentage = completion_percentage(&self.all_tasks);
        let ratio = if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        };
        let gauge = Gauge::default()
            .block(Block::default().title("Progress").borders(Borders::ALL))
            .ratio(ratio)
            .label(format!("{done}/{total} tasks ({percentage}%)"));
        frame.render_widget(gauge, chunks[2]);

        self.render_versions(frame, chunks[3]);
    }
}

impl ProjectDashboard {
    fn render_versions(&self, frame: &mut Frame, area: Rect) {
        if let Some(dialog) = &self.confirm_delete {
            dialog.render(frame, area);
            return;
        }
        let block = Block::default().title("Versions").borders(Borders::ALL);
        if self.versions.is_empty() {
            let paragraph = Paragraph::new("No versions yet. Press c to create one.")
                .block(block)
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }
        // Rows that fit inside the top and bottom borders.
        let row_height = area.height.saturating_sub(2) as usize;
        let range =
            self.versions_scroll
                .visible_range(self.versions.len(), self.selected, row_height);
        let items: Vec<ListItem> = self.versions[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, row)| {
                let i = range.start + offset;
                let style = if i == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    row.version.status.style()
                };
                ListItem::new(Line::from(Span::styled(row.label(), style)))
            })
            .collect();
        frame.render_widget(List::new(items).block(block), area);

        // Scrollbar on the right border, aligned with the data rows.
        if row_height > 0 && area.width > 0 {
            let track = Rect::new(area.x + area.width - 1, area.y + 1, 1, row_height as u16);
            render_scrollbar(frame, track, self.versions.len(), row_height, range.start);
        }
    }
}

fn status_counts(tasks: &[Task]) -> [u64; 3] {
    let mut counts = [0u64; 3];
    for task in tasks {
        match task.status {
            Status::Todo => counts[0] += 1,
            Status::InProgress => counts[1] += 1,
            Status::Done => counts[2] += 1,
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;
    use crate::screens::test_utils::{buffer_to_string, key_event, render_to_string};
    use crate::screens::{Component, Screen};

    use super::*;

    /// Project "Alpha" with the default `v0` (5 tasks) and `v0.1.0` (1 done task).
    fn setup_with_tasks() -> (Arc<StubRepository>, ProjectDashboard, i64) {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo
            .create_project("Alpha", "First project")
            .unwrap()
            .id
            .unwrap();
        let v0 = repo.default_version_id(project_id).unwrap();
        for (name, status) in [
            ("T1", Status::Todo),
            ("T2", Status::Todo),
            ("T3", Status::Todo),
            ("T4", Status::InProgress),
            ("T5", Status::Done),
        ] {
            repo.create_task(project_id, v0, name, "", status, Priority::Low)
                .unwrap();
        }
        let v1 = repo
            .create_version(project_id, "v0.1.0", "")
            .unwrap()
            .id
            .unwrap();
        repo.create_task(project_id, v1, "R1", "", Status::Done, Priority::Low)
            .unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        (repo, dashboard, project_id)
    }

    fn render_text(dashboard: &ProjectDashboard) -> String {
        let backend = TestBackend::new(70, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn auto_refresh_interval_is_five_seconds() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        assert_eq!(
            dashboard.auto_refresh_interval(),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn auto_refresh_interval_is_none_while_confirming_delete() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.handle_event(&key_event(KeyCode::Char('d')));
        assert_eq!(dashboard.auto_refresh_interval(), None);
    }

    #[test]
    fn versions_are_listed_with_progress_label() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let labels: Vec<String> = dashboard.versions().iter().map(VersionRow::label).collect();
        assert_eq!(labels[0], "v0.1.0  1/1 (100%)  Open");
        assert_eq!(labels[1], "v0  1/5 (20%)  Open");
    }

    #[test]
    fn version_progress_counts_only_its_tasks() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let v0 = dashboard
            .versions()
            .iter()
            .find(|r| r.version.name == "v0")
            .unwrap();
        assert_eq!((v0.done, v0.total, v0.percentage), (1, 5, 20));
    }

    #[test]
    fn status_counts_aggregate_over_all_versions() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        assert_eq!(status_counts(dashboard.all_tasks()), [3, 1, 2]);
    }

    #[test]
    fn gauge_shows_done_over_total_across_versions() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        assert!(render_text(&dashboard).contains("2/6 tasks (33%)"));
    }

    #[test]
    fn gauge_shows_zero_percent_when_no_tasks() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Empty", "").unwrap().id.unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        assert!(render_text(&dashboard).contains("0/0 tasks (0%)"));
    }

    #[test]
    fn enter_opens_selected_version() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let version_id = dashboard.versions()[0].version.id.unwrap();
        let action = dashboard.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionDashboard {
                project_id,
                version_id,
            }))
        );
    }

    #[test]
    fn c_opens_new_version_form() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('c')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionForm {
                project_id,
                version_id: None,
            }))
        );
    }

    #[test]
    fn e_opens_project_edit_form() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('e')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectForm {
                project_id: Some(project_id),
            }))
        );
    }

    #[test]
    fn j_and_k_move_version_selection_and_clamp() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.handle_event(&key_event(KeyCode::Char('j')));
        assert_eq!(dashboard.selected_version(), 1);
        dashboard.handle_event(&key_event(KeyCode::Char('j')));
        assert_eq!(dashboard.selected_version(), 1);
        dashboard.handle_event(&key_event(KeyCode::Char('k')));
        dashboard.handle_event(&key_event(KeyCode::Char('k')));
        assert_eq!(dashboard.selected_version(), 0);
    }

    #[test]
    fn d_then_y_emits_delete_version() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let version_id = dashboard.versions()[0].version.id.unwrap();
        dashboard.handle_event(&key_event(KeyCode::Char('d')));
        assert!(dashboard.is_confirming_delete());
        let action = dashboard.handle_event(&key_event(KeyCode::Char('y')));
        assert_eq!(action, Some(Action::DeleteVersion { version_id }));
        assert!(!dashboard.is_confirming_delete());
    }

    #[test]
    fn d_then_other_key_cancels_delete() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.handle_event(&key_event(KeyCode::Char('d')));
        let action = dashboard.handle_event(&key_event(KeyCode::Char('n')));
        assert_eq!(action, None);
        assert!(!dashboard.is_confirming_delete());
    }

    #[test]
    fn r_cycles_selected_version_status() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let row = dashboard.versions()[0].version.clone();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('r')));
        assert_eq!(
            action,
            Some(Action::SaveVersion {
                version_id: row.id,
                project_id: row.project_id,
                name: row.name,
                description: row.description,
                status: VersionStatus::Released,
            })
        );
    }

    #[test]
    fn esc_returns_to_project_list() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(action, Some(Action::Navigate(Screen::ProjectList)));
    }

    #[test]
    fn q_key_emits_quit_action() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        assert_eq!(
            dashboard.handle_event(&key_event(KeyCode::Char('q'))),
            Some(Action::Quit)
        );
    }

    #[test]
    fn empty_project_shows_create_hint() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Empty", "").unwrap().id.unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        // The default version exists, so the list is never empty for a fresh project.
        assert_eq!(dashboard.versions().len(), 1);
        repo.delete_version(dashboard.versions()[0].version.id.unwrap())
            .unwrap();
        dashboard.refresh().unwrap();
        assert!(render_text(&dashboard).contains("No versions yet. Press c to create one."));
    }

    #[test]
    fn dashboard_with_versions_snapshot() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        insta::assert_snapshot!(render_to_string(&dashboard, 70, 30));
    }

    #[test]
    fn long_versions_list_scrolls_to_selected_version() {
        let repo = Arc::new(StubRepository::default());
        let project_id = repo.create_project("Alpha", "").unwrap().id.unwrap();
        for i in 0..12 {
            repo.create_version(project_id, &format!("Ver{i:02}"), "")
                .unwrap();
        }
        let mut dashboard = ProjectDashboard::new(repo);
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();

        let first = dashboard.versions()[0].version.name.clone();
        let last_index = dashboard.versions().len() - 1;
        let last = dashboard.versions()[last_index].version.name.clone();

        let before = render_to_string(&dashboard, 60, 24);
        assert!(before.contains(&first));
        assert!(!before.contains(&last));

        for _ in 0..last_index {
            dashboard.handle_event(&key_event(KeyCode::Down));
        }
        assert_eq!(dashboard.selected_version(), last_index);
        let after = render_to_string(&dashboard, 60, 24);
        assert!(after.contains(&last), "selection not visible:\n{after}");
        assert!(
            !after.contains(&first),
            "first version should have scrolled out:\n{after}"
        );
    }

    #[test]
    fn dashboard_keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = ProjectDashboard::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert!(keys.contains(&(KeyCode::Char('c'), "New Version")));
        assert!(keys.contains(&(KeyCode::Char('r'), "Cycle Status")));
        assert!(keys.contains(&(KeyCode::Char('e'), "Edit Project")));
        assert!(keys.contains(&(KeyCode::Char('d'), "Delete")));
        assert!(keys.contains(&(KeyCode::PageDown, "Scroll description")));
        assert!(keys.contains(&(KeyCode::PageUp, "Scroll description")));
    }

    #[test]
    fn page_keys_scroll_long_project_description() {
        let repo = Arc::new(StubRepository::default());
        let description = (0..30)
            .map(|i| format!("line{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let project_id = repo
            .create_project("Alpha", &description)
            .unwrap()
            .id
            .unwrap();
        let mut dashboard = ProjectDashboard::new(repo);
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();

        let top = render_to_string(&dashboard, 60, 24);
        assert!(top.contains("line00") && top.contains("line01"));

        dashboard.handle_event(&key_event(KeyCode::PageDown));
        let scrolled = render_to_string(&dashboard, 60, 24);
        assert!(
            !scrolled.contains("line00"),
            "description did not scroll:\n{scrolled}"
        );
        assert!(scrolled.contains("line02"));

        for _ in 0..20 {
            dashboard.handle_event(&key_event(KeyCode::PageDown));
        }
        let end = render_to_string(&dashboard, 60, 24);
        assert!(end.contains("line29"));
    }

    #[test]
    fn status_counts_is_empty_for_no_tasks() {
        assert_eq!(status_counts(&[]), [0, 0, 0]);
    }
}
