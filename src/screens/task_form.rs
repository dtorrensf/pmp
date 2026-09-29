use std::collections::HashSet;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;

use crate::db::repository::Repository;
use crate::domain::task::{Priority, Status, Task, TaskNote};
use crate::screens::components::fields::{render_field, render_readonly_field};
use crate::screens::components::keys::{ctrl, key};
use crate::screens::components::popup::{centered_rect_percent, render_popup_scaffold};
use crate::screens::{Action, Component, Screen};

pub struct TaskForm {
    repo: Arc<dyn Repository>,
    project_id: i64,
    task_id: Option<i64>,
    name: String,
    description: TextArea<'static>,
    status: Status,
    priority: Priority,
    available_tasks: Vec<Task>,
    selected_deps: HashSet<i64>,
    notes: Vec<TaskNote>,
    new_note: String,
    active_field: u8,
    error: Option<String>,
    return_to: Screen,
    show_dep_popup: bool,
    dep_filter: String,
    popup_selected: usize,
    dep_original_selection: HashSet<i64>,
    popup_filtered_tasks: Vec<Task>,
}

impl TaskForm {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] =
        &[(ctrl('s'), "Submit"), (key(KeyCode::Esc), "Cancel")];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            task_id: None,
            name: String::new(),
            description: TextArea::default(),
            status: Status::Todo,
            priority: Priority::Medium,
            available_tasks: Vec::new(),
            selected_deps: HashSet::new(),
            notes: Vec::new(),
            new_note: String::new(),
            active_field: 0,
            error: None,
            return_to: Screen::ProjectDashboard { project_id: 0 },
            show_dep_popup: false,
            dep_filter: String::new(),
            popup_selected: 0,
            dep_original_selection: HashSet::new(),
            popup_filtered_tasks: Vec::new(),
        }
    }

    pub fn set_project_id(&mut self, project_id: i64) {
        self.project_id = project_id;
    }

    pub fn set_task_id(&mut self, task_id: Option<i64>) {
        self.task_id = task_id;
    }

    pub fn set_return_to(&mut self, screen: &Screen) {
        self.return_to = screen.clone();
    }

    pub fn return_to(&self) -> &Screen {
        &self.return_to
    }

    pub fn notes(&self) -> &[TaskNote] {
        &self.notes
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &TextArea<'static> {
        &self.description
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    pub fn new_note(&self) -> &str {
        &self.new_note
    }

    pub fn active_field(&self) -> u8 {
        self.active_field
    }

    pub fn show_dep_popup(&self) -> bool {
        self.show_dep_popup
    }

    pub fn selected_deps(&self) -> &HashSet<i64> {
        &self.selected_deps
    }

    fn handle_popup_event(&mut self, code: &KeyCode) -> Option<Action> {
        match code {
            KeyCode::Esc => {
                self.selected_deps = self.dep_original_selection.clone();
                self.show_dep_popup = false;
            }
            KeyCode::Enter => {
                self.show_dep_popup = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.popup_selected > 0 {
                    self.popup_selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.popup_selected + 1 < self.popup_filtered_tasks.len() {
                    self.popup_selected += 1;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(task) = self.popup_filtered_tasks.get(self.popup_selected)
                    && let Some(task_id) = task.id
                {
                    if self.selected_deps.contains(&task_id) {
                        self.selected_deps.remove(&task_id);
                    } else {
                        self.selected_deps.insert(task_id);
                    }
                }
            }
            KeyCode::Backspace => {
                self.dep_filter.pop();
                self.recompute_filtered_tasks();
            }
            KeyCode::Char(c) => {
                self.dep_filter.push(*c);
                self.recompute_filtered_tasks();
            }
            _ => {}
        }
        None
    }

    fn recompute_filtered_tasks(&mut self) {
        let filter_lower = self.dep_filter.to_lowercase();
        self.popup_filtered_tasks = self
            .available_tasks
            .iter()
            .filter(|t| {
                if let Some(id) = t.id
                    && let Some(task_id) = self.task_id
                {
                    if task_id == id {
                        return false;
                    }
                    if self.repo.would_create_cycle(task_id, id).unwrap_or(true) {
                        return false;
                    }
                }
                t.name.to_lowercase().contains(&filter_lower)
            })
            .cloned()
            .collect();
        if self.popup_selected >= self.popup_filtered_tasks.len() {
            self.popup_selected = self.popup_filtered_tasks.len().saturating_sub(1);
        }
    }
}

impl Component for TaskForm {
    fn init(&mut self) -> anyhow::Result<()> {
        self.available_tasks = self
            .repo
            .list_tasks_by_project(self.project_id)?
            .into_iter()
            .filter(|t| t.id != self.task_id)
            .collect();
        if let Some(id) = self.task_id {
            let task = self.repo.get_task(id)?;
            self.name = task.name;
            self.description = TextArea::default();
            self.description.insert_str(&task.description);
            self.status = task.status;
            self.priority = task.priority;
            self.selected_deps = self.repo.get_dependencies(id)?.into_iter().collect();
            self.notes = self.repo.list_task_notes(id)?;
        } else {
            self.name.clear();
            self.description = TextArea::default();
            self.status = Status::Todo;
            self.priority = Priority::Medium;
            self.selected_deps.clear();
            self.notes.clear();
        }
        self.new_note.clear();
        self.active_field = 0;
        self.error = None;
        self.show_dep_popup = false;
        self.dep_filter.clear();
        self.popup_selected = 0;
        self.dep_original_selection.clear();
        self.recompute_filtered_tasks();
        Ok(())
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.available_tasks = self
            .repo
            .list_tasks_by_project(self.project_id)?
            .into_iter()
            .filter(|t| t.id != self.task_id)
            .collect();
        if let Some(id) = self.task_id {
            self.selected_deps = self.repo.get_dependencies(id)?.into_iter().collect();
            self.notes = self.repo.list_task_notes(id)?;
        }
        self.recompute_filtered_tasks();
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
                if self.name.trim().is_empty() {
                    self.error = Some("Task name cannot be empty".to_string());
                    return None;
                }
                return Some(Action::SaveTask {
                    task_id: self.task_id,
                    project_id: self.project_id,
                    name: self.name.clone(),
                    description: self.description.lines().join("\n"),
                    status: self.status,
                    priority: self.priority,
                    depends_on_ids: self.selected_deps.iter().copied().collect(),
                });
            }

            if self.show_dep_popup {
                return self.handle_popup_event(code);
            }

            match code {
                KeyCode::Esc => Some(Action::Navigate(self.return_to.clone())),
                KeyCode::Tab => {
                    self.active_field = (self.active_field + 1) % 6;
                    None
                }
                KeyCode::Backspace => {
                    match self.active_field {
                        0 => {
                            self.name.pop();
                        }
                        1 => {
                            self.description.delete_char();
                        }
                        5 => {
                            self.new_note.pop();
                        }
                        _ => {}
                    }
                    None
                }
                KeyCode::Left => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Back);
                    } else if self.active_field == 2 {
                        self.status = self.status.previous();
                    } else if self.active_field == 3 {
                        self.priority = self.priority.previous();
                    }
                    None
                }
                KeyCode::Right => {
                    if self.active_field == 1 {
                        self.description
                            .move_cursor(ratatui_textarea::CursorMove::Forward);
                    } else if self.active_field == 2 {
                        self.status = self.status.next();
                    } else if self.active_field == 3 {
                        self.priority = self.priority.next();
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
                    } else if self.active_field == 4 {
                        self.dep_original_selection = self.selected_deps.clone();
                        self.dep_filter.clear();
                        self.popup_selected = 0;
                        self.recompute_filtered_tasks();
                        self.show_dep_popup = true;
                    } else if self.active_field == 5
                        && let Some(task_id) = self.task_id
                        && !self.new_note.trim().is_empty()
                    {
                        return Some(Action::AddTaskNote {
                            task_id,
                            content: self.new_note.clone(),
                        });
                    }
                    None
                }
                KeyCode::Char(c) => {
                    match self.active_field {
                        0 => self.name.push(*c),
                        1 => {
                            self.description.insert_char(*c);
                        }
                        2 => {
                            if *c == 'h' {
                                self.status = self.status.previous();
                            } else if *c == 'l' {
                                self.status = self.status.next();
                            }
                        }
                        3 => {
                            if *c == 'h' {
                                self.priority = self.priority.previous();
                            } else if *c == 'l' {
                                self.priority = self.priority.next();
                            }
                        }
                        5 => {
                            self.new_note.push(*c);
                        }
                        _ => {}
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
        let title = if self.task_id.is_some() {
            "Edit Task"
        } else {
            "Create Task"
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut constraints = vec![
            Constraint::Length(3), // Name
            Constraint::Min(5),    // Description
            Constraint::Length(3), // Status
            Constraint::Length(3), // Priority
            Constraint::Min(2),    // Dependencies
            Constraint::Min(7),    // Notes (list + new-note input)
        ];
        if self.error.is_some() {
            constraints.push(Constraint::Length(3));
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        render_field(
            frame,
            chunks[0],
            "Name",
            &self.name,
            self.active_field == 0,
            self.error.is_some() && self.active_field == 0,
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

        render_readonly_field(
            frame,
            chunks[2],
            "Status",
            self.status.label(),
            self.active_field == 2,
        );
        render_readonly_field(
            frame,
            chunks[3],
            "Priority",
            self.priority.label(),
            self.active_field == 3,
        );

        render_dependencies(self, frame, chunks[4]);
        let notes_chunks = render_notes(self, frame, chunks[5]);

        if let Some(error) = &self.error {
            let error_para = Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: true });
            frame.render_widget(error_para, chunks[6]);
        }

        if self.active_field == 0 {
            let x = chunks[0].x + 1 + self.name.chars().count() as u16;
            let y = chunks[0].y + 1;
            frame.set_cursor_position((x, y));
        } else if self.active_field == 5 {
            let x = notes_chunks[1].x + 1 + self.new_note.chars().count() as u16;
            let y = notes_chunks[1].y + 1;
            frame.set_cursor_position((x, y));
        }

        if self.show_dep_popup {
            render_dep_popup(self, frame, area);
        }
    }
}

fn render_notes(form: &TaskForm, frame: &mut Frame, area: Rect) -> Vec<Rect> {
    let notes_style = if form.active_field == 5 {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let count = form.notes.len();
    let title = format!("Notes ({count})");
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(notes_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let notes_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(inner);

    if form.notes.is_empty() {
        let placeholder =
            Paragraph::new("No notes yet").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, notes_chunks[0]);
    } else {
        let lines: Vec<ratatui::text::Line> = form
            .notes
            .iter()
            .map(|note| {
                ratatui::text::Line::from(format!("{} - {}", note.created_at, note.content))
            })
            .collect();
        let list = Paragraph::new(ratatui::text::Text::from(lines)).wrap(Wrap { trim: true });
        frame.render_widget(list, notes_chunks[0]);
    }

    render_field(
        frame,
        notes_chunks[1],
        "New note",
        &form.new_note,
        form.active_field == 5,
        false,
    );

    notes_chunks.to_vec()
}

fn render_dependencies(form: &TaskForm, frame: &mut Frame, area: Rect) {
    let dep_style = if form.active_field == 4 {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let count = form.selected_deps.len();
    let title = format!("Dependencies ({count} selected)");
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(dep_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if form.selected_deps.is_empty() {
        let mut lines = vec![ratatui::text::Line::styled(
            "No dependencies",
            Style::default().fg(Color::DarkGray),
        )];
        if form.active_field == 4 {
            lines.push(ratatui::text::Line::from("[Enter: manage dependencies]"));
        }
        let para = Paragraph::new(ratatui::text::Text::from(lines));
        frame.render_widget(para, inner);
        return;
    }

    let selected_tasks: Vec<&Task> = form
        .available_tasks
        .iter()
        .filter(|t| t.id.is_some_and(|id| form.selected_deps.contains(&id)))
        .collect();

    let mut lines: Vec<ratatui::text::Line> = selected_tasks
        .iter()
        .map(|task| {
            let name = ratatui::text::Span::from(task.name.clone());
            let status = ratatui::text::Span::styled(
                format!(" [{}]", task.status.label()),
                task.status.style(),
            );
            ratatui::text::Line::from(vec![name, status])
        })
        .collect();

    if form.active_field == 4 {
        lines.push(ratatui::text::Line::from("[Enter: manage dependencies]"));
    }

    let para = Paragraph::new(ratatui::text::Text::from(lines));
    frame.render_widget(para, inner);
}

fn render_dep_popup(form: &TaskForm, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect_percent(60, area);
    let inner = render_popup_scaffold(frame, popup_area, "Manage Dependencies");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    let filter_block = Block::default().title("Filter").borders(Borders::ALL);
    let filter_text = if form.dep_filter.is_empty() {
        "Type to filter tasks..."
    } else {
        form.dep_filter.as_str()
    };
    let filter_para = Paragraph::new(filter_text).block(filter_block);
    frame.render_widget(filter_para, chunks[0]);

    if form.popup_filtered_tasks.is_empty() {
        let placeholder =
            Paragraph::new("No matching tasks").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(placeholder, chunks[1]);
        return;
    }

    let lines: Vec<ratatui::text::Line> = form
        .popup_filtered_tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let checked = task
                .id
                .map(|id| form.selected_deps.contains(&id))
                .unwrap_or(false);
            let marker = if checked { "[x]" } else { "[ ]" };
            let name = ratatui::text::Span::from(format!("{} {}", marker, task.name));
            let status = ratatui::text::Span::styled(
                format!(" [{}]", task.status.label()),
                task.status.style(),
            );
            let mut line = ratatui::text::Line::from(vec![name, status]);
            if i == form.popup_selected {
                line = line.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            line
        })
        .collect();

    let list_para = Paragraph::new(ratatui::text::Text::from(lines));
    frame.render_widget(list_para, chunks[1]);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::test_utils::{
        buffer_to_string, char_event, ctrl_s_event, key_event, render_to_string,
    };
    use crate::screens::{Component, Screen};
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    use super::*;

    fn setup() -> (Arc<StubRepository>, TaskForm, i64) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.init().unwrap();
        (repo, form, project_id)
    }

    #[test]
    fn defaults_are_set_on_init() {
        let (_repo, form, _id) = setup();
        assert_eq!(form.status, Status::Todo);
        assert_eq!(form.priority, Priority::Medium);
        assert!(form.name.is_empty());
        assert!(form.description.is_empty());
        assert!(form.selected_deps.is_empty());
        assert_eq!(form.active_field, 0);
        assert!(form.error.is_none());
    }

    #[test]
    fn empty_name_shows_validation_error() {
        let (_repo, mut form, _id) = setup();
        let action = form.handle_event(&ctrl_s_event());
        assert_eq!(action, None);
        assert_eq!(form.error, Some("Task name cannot be empty".to_string()));
    }

    #[test]
    fn valid_submit_emits_save_task_action() {
        let (_repo, mut form, project_id) = setup();
        form.handle_event(&char_event('T'));
        form.handle_event(&char_event('a'));
        form.handle_event(&char_event('s'));
        form.handle_event(&char_event('k'));
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&char_event('D'));
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&key_event(KeyCode::Right)); // status Todo -> InProgress
        form.handle_event(&key_event(KeyCode::Tab));
        form.handle_event(&key_event(KeyCode::Right)); // priority Medium -> High
        let action = form.handle_event(&ctrl_s_event());
        assert_eq!(
            action,
            Some(Action::SaveTask {
                task_id: None,
                project_id,
                name: "Task".to_string(),
                description: "D".to_string(),
                status: Status::InProgress,
                priority: Priority::High,
                depends_on_ids: vec![],
            })
        );
    }

    #[test]
    fn edit_mode_pre_populates_from_repo() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(
                project_id,
                "Auth",
                "secret",
                Status::InProgress,
                Priority::High,
            )
            .unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(task.id);
        form.init().unwrap();

        assert_eq!(form.name, "Auth");
        assert_eq!(form.description.lines(), ["secret"]);
        assert_eq!(form.status, Status::InProgress);
        assert_eq!(form.priority, Priority::High);
    }

    #[test]
    fn enter_in_edit_emits_save_with_task_id() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(task.id);
        form.init().unwrap();
        form.handle_event(&char_event('V'));
        let action = form.handle_event(&ctrl_s_event());
        assert_eq!(
            action,
            Some(Action::SaveTask {
                task_id: task.id,
                project_id,
                name: "AuthV".to_string(),
                description: "".to_string(),
                status: Status::Todo,
                priority: Priority::Medium,
                depends_on_ids: vec![],
            })
        );
    }

    #[test]
    fn title_shows_edit_when_task_id_set() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(task.id);
        form.init().unwrap();

        let backend = TestBackend::new(60, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Edit Task"));
    }

    #[test]
    fn task_form_edit_mode_snapshot() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(
                project_id,
                "Auth",
                "desc",
                Status::InProgress,
                Priority::High,
            )
            .unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(task.id);
        form.init().unwrap();
        insta::assert_snapshot!(render_to_string(&form, 60, 28));
    }

    #[test]
    fn back_emits_navigate_without_saving() {
        let (_repo, mut form, project_id) = setup();
        form.set_return_to(&Screen::ProjectDashboard { project_id });
        form.handle_event(&char_event('x'));
        let action = form.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::ProjectDashboard { project_id }))
        );
    }

    #[test]
    fn esc_returns_to_return_to_screen() {
        let (_repo, mut form, project_id) = setup();
        form.set_return_to(&Screen::TaskList { project_id });
        let action = form.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::TaskList { project_id }))
        );
    }

    #[test]
    fn form_renders_all_fields() {
        let (_repo, form, _id) = setup();
        let backend = TestBackend::new(60, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Create Task"));
        assert!(text.contains("Name"));
        assert!(text.contains("Description"));
        assert!(text.contains("Status"));
        assert!(text.contains("Priority"));
        assert!(text.contains("Dependencies"));
    }

    #[test]
    fn error_is_rendered() {
        let (_repo, mut form, _id) = setup();
        form.handle_event(&ctrl_s_event());
        let backend = TestBackend::new(60, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_string(&buffer);
        assert!(text.contains("Task name cannot be empty"));
    }

    #[test]
    fn task_form_default_snapshot() {
        let (_repo, form, _id) = setup();
        insta::assert_snapshot!(render_to_string(&form, 60, 28));
    }

    #[test]
    fn task_form_with_error_snapshot() {
        let (_repo, mut form, _id) = setup();
        form.handle_event(&ctrl_s_event());
        insta::assert_snapshot!(render_to_string(&form, 60, 28));
    }

    #[test]
    fn keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = TaskForm::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![(KeyCode::Char('s'), "Submit"), (KeyCode::Esc, "Cancel")]
        );
    }

    #[test]
    fn popup_state_resets_on_init() {
        let (_repo, form, _id) = setup();
        assert!(!form.show_dep_popup);
        assert!(form.dep_filter.is_empty());
        assert_eq!(form.popup_selected, 0);
        assert!(form.dep_original_selection.is_empty());
        assert!(form.popup_filtered_tasks.is_empty());
    }

    #[test]
    fn refresh_after_add_task_note_preserves_form_state() {
        let (repo, mut form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Task", "desc", Status::Todo, Priority::Medium)
            .unwrap();
        let task_id = task.id.unwrap();
        form.set_task_id(Some(task_id));
        form.init().unwrap();

        // Simulate user edits and transient UI state.
        form.name.push_str(" edited");
        form.description = TextArea::default();
        form.description.insert_str("desc more");
        form.status = Status::InProgress;
        form.priority = Priority::High;
        form.active_field = 4;
        form.new_note.push_str("draft note");

        // Add a dependency and open the popup.
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        repo.add_dependency(task_id, dep_id).unwrap();
        form.refresh().unwrap();
        form.dep_original_selection = form.selected_deps.clone();
        form.dep_filter.push('D');
        form.recompute_filtered_tasks();
        form.show_dep_popup = true;

        // Add a note to the repo (the side effect of AddTaskNote).
        repo.add_task_note(task_id, "My note").unwrap();

        // Refresh must reload only data, leaving the transient state intact.
        form.refresh().unwrap();

        assert_eq!(form.notes.len(), 1);
        assert_eq!(form.notes[0].content, "My note");
        assert_eq!(form.name, "Task edited");
        assert_eq!(form.description.lines(), ["desc more"]);
        assert_eq!(form.status, Status::InProgress);
        assert_eq!(form.priority, Priority::High);
        assert_eq!(form.active_field, 4);
        assert_eq!(form.new_note, "draft note");
        assert!(form.show_dep_popup);
        assert_eq!(form.dep_filter, "D");
        assert!(form.selected_deps.contains(&dep_id));
        assert!(
            form.popup_filtered_tasks
                .iter()
                .any(|t| t.id == Some(dep_id))
        );
    }

    #[test]
    fn popup_opens_on_enter_at_field_4() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        let action = form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(action, None);
        assert!(form.show_dep_popup);
        assert_eq!(form.popup_filtered_tasks.len(), 1);
        assert_eq!(form.popup_filtered_tasks[0].name, "Dep");
    }

    #[test]
    fn popup_dependency_list_uses_canonical_status_style() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "Dep", "", Status::InProgress, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));

        let backend = TestBackend::new(60, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();

        let badge_style = find_status_badge_style(buffer, "In Progress");
        assert_eq!(badge_style.fg, Some(Color::Blue));
    }

    #[test]
    fn popup_escape_restores_original_selection() {
        let (repo, mut form, project_id) = setup();
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        form.handle_event(&char_event(' '));
        assert!(form.selected_deps.contains(&dep_id));
        let action = form.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(action, None);
        assert!(!form.show_dep_popup);
        assert!(!form.selected_deps.contains(&dep_id));
    }

    #[test]
    fn popup_enter_confirms_selection() {
        let (repo, mut form, project_id) = setup();
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        form.handle_event(&char_event(' '));
        assert!(form.selected_deps.contains(&dep_id));
        let action = form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(action, None);
        assert!(!form.show_dep_popup);
        assert!(form.selected_deps.contains(&dep_id));
    }

    #[test]
    fn popup_space_toggles_selection() {
        let (repo, mut form, project_id) = setup();
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        form.handle_event(&char_event(' '));
        assert!(form.selected_deps.contains(&dep_id));
        form.handle_event(&char_event(' '));
        assert!(!form.selected_deps.contains(&dep_id));
    }

    #[test]
    fn popup_jk_navigation() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "First", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "Second", "", Status::Todo, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(form.popup_selected, 0);
        form.handle_event(&char_event('j'));
        assert_eq!(form.popup_selected, 1);
        form.handle_event(&char_event('k'));
        assert_eq!(form.popup_selected, 0);
        form.handle_event(&key_event(KeyCode::Down));
        assert_eq!(form.popup_selected, 1);
        form.handle_event(&key_event(KeyCode::Up));
        assert_eq!(form.popup_selected, 0);
        // Clamp at top.
        form.handle_event(&char_event('k'));
        assert_eq!(form.popup_selected, 0);
        // Clamp at bottom.
        form.handle_event(&char_event('j'));
        form.handle_event(&char_event('j'));
        assert_eq!(form.popup_selected, 1);
    }

    #[test]
    fn popup_filter_narrows_list() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "Auth", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "API", "", Status::Todo, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        form.handle_event(&char_event('a'));
        form.handle_event(&char_event('u'));
        assert_eq!(form.popup_filtered_tasks.len(), 1);
        assert_eq!(form.popup_filtered_tasks[0].name, "Auth");
    }

    #[test]
    fn popup_filter_empty_state() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "Auth", "", Status::Todo, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        form.handle_event(&char_event('z'));
        assert!(form.popup_filtered_tasks.is_empty());
        let text = render_to_string(&form, 60, 28);
        assert!(text.contains("No matching tasks"));
    }

    #[test]
    fn popup_backspace_removes_char_and_refilters() {
        let (repo, mut form, project_id) = setup();
        repo.create_task(project_id, "Auth", "", Status::Todo, Priority::Low)
            .unwrap();
        repo.create_task(project_id, "API", "", Status::Todo, Priority::Low)
            .unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        // Type 'a' then 'p' — filter narrows to "API"
        form.handle_event(&char_event('a'));
        form.handle_event(&char_event('p'));
        assert_eq!(form.dep_filter, "ap");
        assert_eq!(form.popup_filtered_tasks.len(), 1);
        assert_eq!(form.popup_filtered_tasks[0].name, "API");
        // Backspace removes 'p' — filter narrows to "Auth" and "API" both
        form.handle_event(&key_event(KeyCode::Backspace));
        assert_eq!(form.dep_filter, "a");
        assert_eq!(form.popup_filtered_tasks.len(), 2);
        // Second backspace removes 'a' — filter is empty, all tasks shown
        form.handle_event(&key_event(KeyCode::Backspace));
        assert_eq!(form.dep_filter, "");
        assert_eq!(form.popup_filtered_tasks.len(), 2);
    }

    #[test]
    fn compact_dependencies_renders_selected_with_status() {
        let (repo, mut form, project_id) = setup();
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Done, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.selected_deps.insert(dep_id);
        let text = render_to_string(&form, 60, 28);
        assert!(text.contains("Dep"));
        assert!(text.contains("Done"));
        assert!(text.contains("[Enter: manage dependencies]"));
    }

    #[test]
    fn compact_dependencies_use_canonical_status_style() {
        let (repo, mut form, project_id) = setup();
        let dep = repo
            .create_task(project_id, "Dep", "", Status::Todo, Priority::Low)
            .unwrap();
        let dep_id = dep.id.unwrap();
        form.init().unwrap();
        form.active_field = 4;
        form.selected_deps.insert(dep_id);

        let backend = TestBackend::new(60, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();

        let badge_style = find_status_badge_style(buffer, "Todo");
        assert_eq!(badge_style.fg, Some(Color::Yellow));
    }

    fn find_status_badge_style(buffer: &Buffer, label: &str) -> ratatui::style::Style {
        let area = buffer.area();
        let pattern = format!("[{label}]");
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let mut matched = true;
                for (i, ch) in pattern.chars().enumerate() {
                    let cx = x + i as u16;
                    if cx >= area.x + area.width || buffer[(cx, y)].symbol() != ch.to_string() {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    return buffer[(x, y)].style();
                }
            }
        }
        panic!("status badge [{label}] not found in buffer");
    }

    #[test]
    fn compact_dependencies_renders_no_dependencies_placeholder() {
        let (_repo, form, _id) = setup();
        let text = render_to_string(&form, 60, 28);
        assert!(text.contains("No dependencies"));
    }

    #[test]
    fn notes_list_renders_existing_notes() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let task_id = task.id.unwrap();
        repo.add_task_note(task_id, "First note").unwrap();
        repo.add_task_note(task_id, "Second note").unwrap();

        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(Some(task_id));
        form.init().unwrap();

        let text = render_to_string(&form, 60, 28);
        assert!(text.contains("Notes"));
        assert!(text.contains("First note"));
        assert!(text.contains("Second note"));
    }

    #[test]
    fn empty_notes_renders_placeholder() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(task.id);
        form.init().unwrap();

        let text = render_to_string(&form, 60, 28);
        assert!(text.contains("No notes yet"));
    }

    #[test]
    fn enter_in_notes_field_emits_add_task_note() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let task_id = task.id.unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(Some(task_id));
        form.init().unwrap();
        form.active_field = 5;
        form.handle_event(&char_event('H'));
        form.handle_event(&char_event('i'));
        let action = form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::AddTaskNote {
                task_id,
                content: "Hi".to_string(),
            })
        );
    }

    #[test]
    fn empty_note_does_not_emit_action() {
        let (repo, _form, project_id) = setup();
        let task = repo
            .create_task(project_id, "Auth", "", Status::Todo, Priority::Medium)
            .unwrap();
        let task_id = task.id.unwrap();
        let mut form = TaskForm::new(repo.clone());
        form.set_project_id(project_id);
        form.set_task_id(Some(task_id));
        form.init().unwrap();
        form.active_field = 5;
        let action = form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(action, None);
    }

    #[test]
    fn creating_task_cannot_add_note() {
        let (_repo, mut form, _project_id) = setup();
        form.active_field = 5;
        form.handle_event(&char_event('H'));
        let action = form.handle_event(&key_event(KeyCode::Enter));
        assert_eq!(action, None);
        assert_eq!(form.new_note, "H");
    }

    #[test]
    fn cycle_creating_task_excluded_from_popup() {
        let (repo, mut form, project_id) = setup();
        let a = repo
            .create_task(project_id, "A", "", Status::Todo, Priority::Low)
            .unwrap();
        let b = repo
            .create_task(project_id, "B", "", Status::Todo, Priority::Low)
            .unwrap();
        let a_id = a.id.unwrap();
        let b_id = b.id.unwrap();
        repo.add_dependency(b_id, a_id).unwrap();
        form.set_task_id(Some(a_id));
        form.init().unwrap();
        form.active_field = 4;
        form.handle_event(&key_event(KeyCode::Enter));
        assert!(!form.popup_filtered_tasks.iter().any(|t| t.id == Some(b_id)));
        // Task A itself is also excluded from available_tasks during init, so the popup is empty.
        assert!(form.popup_filtered_tasks.is_empty());
    }
}
