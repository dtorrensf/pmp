// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders},
};

use crate::db::repository::Repository;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task};
use crate::screens::components::keys::key;
use crate::screens::task_list_view::TaskListView;
pub use crate::screens::task_list_view::{FilterField, FilterState, TaskListMode};
use crate::screens::{Action, Component, Screen};

pub struct TaskListScreen {
    repo: Arc<dyn Repository>,
    project_id: i64,
    project: Option<Project>,
    view: TaskListView,
}

impl TaskListScreen {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
        (key(KeyCode::Up), "Up"),
        (key(KeyCode::Down), "Down"),
        (key(KeyCode::Enter), "Edit"),
        (key(KeyCode::Tab), "Cycle"),
        (key(KeyCode::Char('d')), "Delete"),
        (key(KeyCode::Char('r')), "Refresh"),
        (key(KeyCode::Char('f')), "Filter"),
        (key(KeyCode::Char('c')), "New"),
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

    pub fn selected_task(&self) -> usize {
        self.view.selected()
    }

    pub fn tasks(&self) -> &[Task] {
        self.view.tasks()
    }

    pub fn filter_status(&self) -> Option<Status> {
        self.view.filter_status()
    }

    pub fn filter_priority(&self) -> Option<Priority> {
        self.view.filter_priority()
    }
}

impl Component for TaskListScreen {
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
                    KeyCode::Esc | KeyCode::Backspace => {
                        return Some(Action::Navigate(Screen::ProjectDashboard {
                            project_id: self.project_id,
                        }));
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

        let block = Block::default()
            .title(project_name.to_string())
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        self.view.render(frame, inner);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::components::confirm_dialog::ConfirmDialog;
    use crate::screens::test_utils::{buffer_to_string, key_event, render_to_string};

    use super::*;

    fn setup_with_tasks() -> (Arc<StubRepository>, TaskListScreen, i64) {
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
        let mut screen = TaskListScreen::new(repo.clone());
        screen.set_project_id(project_id);
        screen.init().unwrap();
        (repo, screen, project_id)
    }

    #[test]
    fn task_list_keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = TaskListScreen::KEYBINDINGS
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
                (KeyCode::Char('r'), "Refresh"),
                (KeyCode::Char('f'), "Filter"),
                (KeyCode::Char('c'), "New"),
                (KeyCode::Esc, "Back"),
                (KeyCode::Char('q'), "Quit"),
            ]
        );
    }

    #[test]
    fn init_loads_project_and_tasks() {
        let (_repo, screen, _id) = setup_with_tasks();
        assert_eq!(screen.tasks().len(), 5);
        assert!(screen.project.is_some());
        assert_eq!(screen.project.unwrap().name, "Alpha");
    }

    #[test]
    fn init_clamps_selection_when_task_list_shrinks() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(project_id, "Only", "", Status::Todo, Priority::Low)
            .unwrap();
        let mut screen = TaskListScreen::new(repo);
        screen.set_project_id(project_id);
        screen.view.set_selected(5);
        screen.init().unwrap();
        assert_eq!(screen.selected_task(), 0);
    }

    #[test]
    fn init_on_empty_project_renders_no_tasks_message() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let project_id = project.id.unwrap();
        let mut screen = TaskListScreen::new(repo);
        screen.set_project_id(project_id);
        screen.init().unwrap();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("No tasks. Press c to create one."));
    }

    #[test]
    fn render_shows_project_name_border() {
        let (_repo, screen, _id) = setup_with_tasks();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("ALPHA"));
    }

    #[test]
    fn empty_list_renders_no_tasks_message() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let mut screen = TaskListScreen::new(repo);
        screen.project = Some(project);

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("No tasks. Press c to create one."));
    }

    #[test]
    fn confirm_delete_without_dependents_renders_simple_message() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Delete task Leaf? (y/N)",
                Action::DeleteTask { task_id: 1 },
            )));

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("? (y/N)"));
        assert!(!text.contains("dependency links"));
    }

    #[test]
    fn confirm_delete_with_dependents_renders_links_warning() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Deleting will remove 3 dependency links. Continue? (y/N)",
                Action::DeleteTask { task_id: 1 },
            )));

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Deleting will remove 3 dependency links"));
    }

    #[test]
    fn list_rows_render_name_status_priority_columns() {
        let (_repo, screen, _id) = setup_with_tasks();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Tasks"));
        assert!(text.contains("Name"));
        assert!(text.contains("Status"));
        assert!(text.contains("Priority"));
        assert!(text.contains("Done"));
        assert!(text.contains("Todo"));
    }

    #[test]
    fn q_key_emits_quit_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('q')));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn t_key_navigates_to_new_task_form() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('c')));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                project_id,
                task_id: None,
            }))
        );
    }

    #[test]
    fn esc_navigates_to_dashboard() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
    }

    #[test]
    fn backspace_navigates_to_dashboard() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Backspace));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
    }

    #[test]
    fn up_clamps_at_top_bound() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.view.set_selected(1);
        screen.handle_event(&key_event(KeyCode::Up));
        assert_eq!(screen.selected_task(), 0);
        screen.handle_event(&key_event(KeyCode::Up));
        assert_eq!(screen.selected_task(), 0);
    }

    #[test]
    fn down_clamps_at_bottom_bound() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let last = screen.tasks().len() - 1;
        screen.view.set_selected(last);
        screen.handle_event(&key_event(KeyCode::Down));
        assert_eq!(screen.selected_task(), last);
    }

    #[test]
    fn enter_on_selected_navigates_to_edit_form() {
        let (_repo, mut screen, project_id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskForm {
                project_id,
                task_id: Some(task_id),
            }))
        );
    }

    #[test]
    fn tab_cycles_todo_to_inprogress() {
        let (_repo, mut screen, _project_id) = setup_with_tasks();
        screen.view.set_selected(0);
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(
            action,
            Some(Action::UpdateTaskStatus {
                task_id,
                new_status: Status::InProgress,
            })
        );
    }

    #[test]
    fn tab_cycles_done_to_todo() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let done_index = screen
            .tasks()
            .iter()
            .position(|t| t.status == Status::Done)
            .unwrap();
        screen.view.set_selected(done_index);
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(
            action,
            Some(Action::UpdateTaskStatus {
                task_id,
                new_status: Status::Todo,
            })
        );
    }

    #[test]
    fn tab_noop_when_empty_list() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "").unwrap();
        let project_id = project.id.unwrap();
        let mut screen = TaskListScreen::new(repo);
        screen.project = Some(project);
        screen.project_id = project_id;
        let action = screen.handle_event(&key_event(KeyCode::Tab));
        assert_eq!(action, None);
    }

    #[test]
    fn d_enters_confirm_mode_with_dependents_count() {
        let (repo, mut screen, _id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        let dep = repo
            .create_task(screen.project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.add_dependency(dep.id.unwrap(), task_id).unwrap();

        let action = screen.handle_event(&key_event(KeyCode::Char('d')));
        assert_eq!(action, None);
        match screen.view.mode() {
            TaskListMode::ConfirmDelete(dialog) => {
                assert_eq!(dialog.on_confirm(), &Action::DeleteTask { task_id });
                assert!(dialog.message().contains("1 dependency link"));
            }
            other => panic!("expected confirm mode, got {:?}", other),
        }
    }

    #[test]
    fn y_confirms_delete_and_emits_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let task_id = screen.tasks()[screen.selected_task()].id.unwrap();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        assert!(matches!(screen.view.mode(), TaskListMode::ConfirmDelete(_)));
        let action = screen.handle_event(&key_event(KeyCode::Char('y')));
        assert_eq!(action, Some(Action::DeleteTask { task_id }));
    }

    #[test]
    fn n_cancels_confirm_mode() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('d')));
        let action = screen.handle_event(&key_event(KeyCode::Char('n')));
        assert_eq!(action, None);
        assert!(matches!(screen.view.mode(), TaskListMode::Browsing));
    }

    #[test]
    fn task_list_with_tasks_snapshot() {
        let (_repo, screen, _id) = setup_with_tasks();
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn task_list_empty_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Empty", "No tasks here").unwrap();
        let mut screen = TaskListScreen::new(repo);
        screen.project = Some(project);
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn task_list_confirm_delete_leaf_snapshot() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                format!("Delete task {}? (y/N)", screen.tasks()[0].name),
                Action::DeleteTask { task_id: 1 },
            )));
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn task_list_confirm_delete_with_deps_snapshot() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen
            .view
            .set_mode(TaskListMode::ConfirmDelete(ConfirmDialog::new(
                "Deleting will remove 1 dependency links. Continue? (y/N)",
                Action::DeleteTask { task_id: 1 },
            )));
        insta::assert_snapshot!(render_to_string(&screen, 60, 24));
    }

    #[test]
    fn r_key_emits_refresh_action() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let action = screen.handle_event(&key_event(KeyCode::Char('r')));
        assert_eq!(action, Some(Action::Refresh));
    }

    #[test]
    fn f_key_enters_filtering_mode() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        assert!(matches!(screen.view.mode(), TaskListMode::Filtering(_)));
    }

    #[test]
    fn filter_enter_without_changes_keeps_all_tasks() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        let initial_count = screen.tasks().len();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(screen.tasks().len(), initial_count);
    }

    #[test]
    fn filter_by_status_shows_only_requested() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered: Vec<Status> = screen.tasks().iter().map(|t| t.status).collect();
        assert!(filtered.iter().all(|s| *s == Status::Todo));
        assert!(filtered.len() < 5);
    }

    #[test]
    fn filter_by_priority_shows_only_requested() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Tab));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered: Vec<Priority> = screen.tasks().iter().map(|t| t.priority).collect();
        assert!(filtered.iter().all(|p| *p == Priority::High));
    }

    #[test]
    fn filter_by_both_shows_intersection() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Tab));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        for task in screen.tasks() {
            assert_eq!(task.status, Status::Todo);
            assert_eq!(task.priority, Priority::High);
        }
    }

    #[test]
    fn filter_esc_cancels_and_keeps_previous_filter() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        let filtered_count = screen.tasks().len();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(screen.tasks().len(), filtered_count);
    }

    #[test]
    fn filter_tab_switches_selected_field() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Tab));
        match screen.view.mode() {
            TaskListMode::Filtering(state) => {
                assert_eq!(state.selected_field, FilterField::Priority);
            }
            _ => panic!("expected Filtering mode"),
        }
    }

    #[test]
    fn filter_right_cycles_status_to_any() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        screen.handle_event(&key_event(KeyCode::Char('f')));
        for _ in 0..4 {
            screen.handle_event(&key_event(KeyCode::Right));
        }
        match screen.view.mode() {
            TaskListMode::Filtering(state) => {
                assert_eq!(state.filter_status, None);
            }
            _ => panic!("expected Filtering mode"),
        }
    }

    #[test]
    fn filter_navigation_clamps_on_filtered_list() {
        let (_repo, mut screen, _id) = setup_with_tasks();
        assert!(screen.tasks().len() > 1);
        screen.handle_event(&key_event(KeyCode::Down));
        assert_eq!(screen.selected_task(), 1);
        screen.handle_event(&key_event(KeyCode::Char('f')));
        screen.handle_event(&key_event(KeyCode::Right));
        screen.handle_event(&key_event(KeyCode::Enter));
        assert!(screen.selected_task() < screen.tasks().len());
    }
}
