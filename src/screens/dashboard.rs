// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Text},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::db::repository::Repository;
use crate::domain::project::Project;
use crate::domain::task::{Status, Task};
use crate::screens::components::keys::key;
use crate::screens::task_list_view::{TaskListMode, TaskListView};
use crate::screens::{Action, Component, Screen};

pub struct ProjectDashboard {
    repo: Arc<dyn Repository>,
    project_id: i64,
    project: Option<Project>,
    view: TaskListView,
}

/// Interval at which the project dashboard is automatically refreshed when idle.
pub const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

impl ProjectDashboard {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
        (key(KeyCode::Up), "Up"),
        (key(KeyCode::Down), "Down"),
        (key(KeyCode::Enter), "Edit"),
        (key(KeyCode::Tab), "Cycle"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('f')), "Filter"),
        (key(KeyCode::Char('c')), "New Task"),
        (key(KeyCode::Char('e')), "Edit Project"),
        (key(KeyCode::Char('r')), "Refresh"),
        (key(KeyCode::Esc), "Back"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            project: None,
            view: TaskListView::new(),
        }
    }

    pub fn set_project_id(&mut self, project_id: i64) {
        self.project_id = project_id;
    }

    pub fn set_selected_task(&mut self, index: usize) {
        self.view.set_selected(index);
    }

    pub fn selected_task(&self) -> usize {
        self.view.selected()
    }

    /// Read-only access to the cached task list (refreshed by `init`).
    pub fn tasks(&self) -> &[Task] {
        self.view.tasks()
    }
}

impl Component for ProjectDashboard {
    fn init(&mut self) -> anyhow::Result<()> {
        self.project = Some(self.repo.get_project(self.project_id)?);
        let tasks = self.repo.list_tasks_by_project(self.project_id)?;
        self.view.set_tasks(tasks);
        self.view.set_mode(TaskListMode::Browsing);
        Ok(())
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.project = Some(self.repo.get_project(self.project_id)?);
        let tasks = self.repo.list_tasks_by_project(self.project_id)?;
        self.view.set_tasks(tasks);
        Ok(())
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn auto_refresh_interval(&self) -> Option<Duration> {
        self.view.is_browsing().then_some(AUTO_REFRESH_INTERVAL)
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            let code = *code;
            if code == KeyCode::Char('q') {
                return Some(Action::Quit);
            }
            if self.view.is_browsing() {
                match code {
                    KeyCode::Char('r') => return Some(Action::Refresh),
                    KeyCode::Char('c') => {
                        return Some(Action::Navigate(Screen::TaskForm {
                            project_id: self.project_id,
                            task_id: None,
                        }));
                    }
                    KeyCode::Char('e') => {
                        return Some(Action::Navigate(Screen::ProjectForm {
                            project_id: Some(self.project_id),
                        }));
                    }
                    KeyCode::Backspace | KeyCode::Esc => {
                        return Some(Action::Navigate(Screen::ProjectList));
                    }
                    _ => {}
                }
            }
            self.view.handle_key(code, self.project_id, &*self.repo)
        } else {
            None
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

        let desc = Paragraph::new(Text::from(project_description.to_string()))
            .block(Block::default().borders(Borders::BOTTOM))
            .wrap(Wrap { trim: true });
        frame.render_widget(desc, chunks[0]);

        // Charts aggregate over ALL project tasks, not the filtered list.
        let counts = status_counts(self.view.all_tasks());
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

        let total = self.view.all_tasks().len() as u64;
        let done = counts[2];
        let ratio = if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        };
        let percentage = (ratio * 100.0).round() as u64;
        let gauge = Gauge::default()
            .block(Block::default().title("Progress").borders(Borders::ALL))
            .ratio(ratio)
            .label(format!("{done}/{total} tasks ({percentage}%)"));
        frame.render_widget(gauge, chunks[2]);

        self.view.render(frame, chunks[3]);
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
    use crate::screens::test_utils::{buffer_to_string, key_event, render_to_string};
    use crate::screens::{Component, Screen};

    use super::*;

    fn setup_with_tasks() -> (Arc<StubRepository>, ProjectDashboard, i64) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First project").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(project_id, "T1", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "T2", "", Status::Todo, Priority::Medium)
            .unwrap();
        repo.create_task(project_id, "T3", "", Status::Todo, Priority::High)
            .unwrap();
        repo.create_task(project_id, "T4", "", Status::InProgress, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "T5", "", Status::Done, Priority::Medium)
            .unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        (repo, dashboard, project_id)
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
    fn auto_refresh_interval_is_none_while_filtering() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();

        dashboard.handle_event(&key_event(KeyCode::Char('f')));

        assert_eq!(dashboard.auto_refresh_interval(), None);
    }

    #[test]
    fn auto_refresh_interval_is_none_while_confirm_delete() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();

        dashboard.handle_event(&key_event(KeyCode::Char('d')));

        assert_eq!(dashboard.auto_refresh_interval(), None);
    }

    #[test]
    fn status_counts_aggregate_correctly() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let counts = status_counts(dashboard.tasks());
        assert_eq!(counts, [3, 1, 1]);
    }

    #[test]
    fn gauge_shows_done_over_total_with_percentage() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("1/5 tasks (20%)"));
    }

    #[test]
    fn gauge_shows_zero_percent_when_no_tasks() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "No tasks").unwrap();
        let project_id = project.id.unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("0/0 tasks (0%)"));
    }

    #[test]
    fn gauge_shows_one_hundred_percent_when_all_done() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Done", "All done").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(project_id, "D1", "", Status::Done, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "D2", "", Status::Done, Priority::Medium)
            .unwrap();
        let mut dashboard = ProjectDashboard::new(repo.clone());
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("2/2 tasks (100%)"));
    }

    #[test]
    fn q_key_emits_quit_action() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('q')));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn t_key_navigates_to_task_form() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('c')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                project_id,
                task_id: None,
            }))
        );
    }

    #[test]
    fn enter_navigates_to_task_form_edit() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let task_id = dashboard.tasks()[dashboard.selected_task()].id.unwrap();
        let action = dashboard.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                project_id,
                task_id: Some(task_id),
            }))
        );
    }

    #[test]
    fn j_moves_selection_down() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.handle_event(&key_event(KeyCode::Char('j')));
        assert_eq!(dashboard.selected_task(), 1);
    }

    #[test]
    fn down_key_clamps_at_last_task() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.handle_event(&key_event(KeyCode::Down));
        assert_eq!(dashboard.selected_task(), 1);
        let last = dashboard.tasks().len() - 1;
        dashboard.set_selected_task(last);
        dashboard.handle_event(&key_event(KeyCode::Down));
        assert_eq!(dashboard.selected_task(), last);
    }

    #[test]
    fn k_and_up_move_selection_up_and_clamp_at_top() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        dashboard.set_selected_task(1);
        dashboard.handle_event(&key_event(KeyCode::Char('k')));
        assert_eq!(dashboard.selected_task(), 0);
        dashboard.handle_event(&key_event(KeyCode::Up));
        assert_eq!(dashboard.selected_task(), 0);
    }

    #[test]
    fn tab_cycles_selected_task_status() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let task = dashboard.tasks()[dashboard.selected_task()].clone();
        let action = dashboard.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(
            action,
            Some(Action::UpdateTaskStatus {
                task_id: task.id.unwrap(),
                new_status: task.status.next(),
            })
        );
    }

    #[test]
    fn d_then_y_emits_delete_task() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let task_id = dashboard.tasks()[dashboard.selected_task()].id.unwrap();
        assert_eq!(dashboard.handle_event(&key_event(KeyCode::Char('d'))), None);
        let action = dashboard.handle_event(&key_event(KeyCode::Char('y')));
        assert_eq!(action, Some(Action::DeleteTask { task_id }));
    }

    #[test]
    fn d_then_other_key_cancels_delete() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let task_id = dashboard.tasks()[dashboard.selected_task()].id.unwrap();
        dashboard.handle_event(&key_event(KeyCode::Char('d')));
        assert_eq!(dashboard.handle_event(&key_event(KeyCode::Char('n'))), None);
        // back to browsing: 'y' no longer emits a delete action
        assert_eq!(dashboard.handle_event(&key_event(KeyCode::Char('y'))), None);
        // and 'd' re-enters the confirm flow, proving the cancel happened
        dashboard.handle_event(&key_event(KeyCode::Char('d')));
        assert_eq!(
            dashboard.handle_event(&key_event(KeyCode::Char('y'))),
            Some(Action::DeleteTask { task_id })
        );
    }

    #[test]
    fn f_filters_dashboard_task_list() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        assert_eq!(dashboard.handle_event(&key_event(KeyCode::Char('f'))), None);
        dashboard.handle_event(&key_event(KeyCode::Right)); // status -> Todo
        dashboard.handle_event(&key_event(KeyCode::Enter)); // apply filter
        assert_eq!(dashboard.tasks().len(), 3);
        assert!(
            dashboard
                .tasks()
                .iter()
                .all(|task| task.status == Status::Todo)
        );
    }

    /// Status chart + Progress gauge lines: everything from the Status block
    /// title up to (but excluding) the Tasks block title.
    fn chart_lines(rendered: &str) -> Vec<String> {
        rendered
            .lines()
            .skip_while(|line| !line.contains("Status"))
            .take_while(|line| !line.contains("Tasks"))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn charts_keep_full_project_values_when_filter_applied() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let before = render_to_string(&dashboard, 60, 24);

        // Apply a status filter (Status: Todo) through the filter popup.
        dashboard.handle_event(&key_event(KeyCode::Char('f')));
        dashboard.handle_event(&key_event(KeyCode::Right));
        dashboard.handle_event(&key_event(KeyCode::Enter));

        // Sanity: the task list itself is filtered and the render reflects it.
        assert_eq!(dashboard.tasks().len(), 3);
        let after = render_to_string(&dashboard, 60, 24);
        assert!(after.contains("Tasks (Status: Todo)"));

        // The charts aggregate over ALL project tasks, so they must render
        // exactly as before the filter was applied.
        assert_eq!(chart_lines(&before), chart_lines(&after));
        assert!(after.contains("1/5 tasks"));
    }

    #[test]
    fn back_navigates_to_entry() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Backspace));
        assert_eq!(action, Some(Action::Navigate(Screen::ProjectList)));
    }

    #[test]
    fn empty_project_shows_zero_counts() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let project_id = project.id.unwrap();
        let mut dashboard = ProjectDashboard::new(repo);
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("0/0 tasks"));
        assert!(!text.contains("First project"));
    }

    #[test]
    fn dashboard_renders_all_widgets() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dashboard.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("ALPHA"));
        assert!(text.contains("First project"));
        assert!(text.contains("Status"));
        assert!(text.contains("Progress"));
        assert!(text.contains("Tasks"));
        assert!(text.contains("Name"));
        assert!(text.contains("Priority"));
        assert!(!text.contains("Priority by Status"));
    }

    #[test]
    fn dashboard_empty_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "No tasks here").unwrap();
        let project_id = project.id.unwrap();
        let mut dashboard = ProjectDashboard::new(repo);
        dashboard.set_project_id(project_id);
        dashboard.init().unwrap();
        insta::assert_snapshot!(render_to_string(&dashboard, 60, 24));
    }

    #[test]
    fn dashboard_with_tasks_snapshot() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        insta::assert_snapshot!(render_to_string(&dashboard, 60, 24));
    }

    #[test]
    fn dashboard_keybindings_match_spec() {
        let (_repo, dashboard, _id) = setup_with_tasks();
        let keys: Vec<(KeyCode, &str)> = dashboard
            .keybindings()
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![
                (KeyCode::Char('j'), "Down"),
                (KeyCode::Char('k'), "Up"),
                (KeyCode::Up, "Up"),
                (KeyCode::Down, "Down"),
                (KeyCode::Enter, "Edit"),
                (KeyCode::Tab, "Cycle"),
                (KeyCode::Char('d'), "Delete"),
                (KeyCode::Char('f'), "Filter"),
                (KeyCode::Char('c'), "New Task"),
                (KeyCode::Char('e'), "Edit Project"),
                (KeyCode::Char('r'), "Refresh"),
                (KeyCode::Esc, "Back"),
                (KeyCode::Char('q'), "Quit"),
            ]
        );
    }

    #[test]
    fn e_key_navigates_to_project_form_edit() {
        let (_repo, mut dashboard, project_id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('e')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectForm {
                project_id: Some(project_id)
            }))
        );
    }

    #[test]
    fn r_key_emits_refresh_action() {
        let (_repo, mut dashboard, _id) = setup_with_tasks();
        let action = dashboard.handle_event(&key_event(KeyCode::Char('r')));
        assert_eq!(action, Some(Action::Refresh));
    }
}
