use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{ExecutableCommand, cursor};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::flag;

use crate::app::App;
use crate::screens::Action;
use crate::screens::keybinding_footer::render_footer;

/// Polling timeout for each iteration of the TUI event loop.
const EVENT_TIMEOUT: Duration = Duration::from_millis(100);

/// Number of consecutive event-loop timeouts that make up an auto-refresh interval.
fn auto_refresh_ticks(interval: Duration) -> usize {
    interval.as_millis().div_ceil(EVENT_TIMEOUT.as_millis()) as usize
}

pub fn run(mut app: App) -> anyhow::Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    stdout
        .execute(EnterAlternateScreen)
        .context("enter alternate screen")?;
    stdout.execute(cursor::Hide).context("hide cursor")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    let terminate = setup_signal_flag().context("setup signal flag")?;
    let mut events = CrosstermEvents;

    let result = run_loop(&mut app, &mut terminal, &mut events, &terminate);

    disable_raw_mode().context("disable raw mode")?;
    terminal
        .backend_mut()
        .execute(LeaveAlternateScreen)
        .context("leave alternate screen")?;
    terminal
        .backend_mut()
        .execute(cursor::Show)
        .context("show cursor")?;
    terminal.show_cursor().context("show terminal cursor")?;

    result
}

fn setup_signal_flag() -> anyhow::Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, stop.clone())?;
    flag::register(SIGINT, stop.clone())?;
    Ok(stop)
}

fn run_loop<B, E>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    events: &mut E,
    stop: &AtomicBool,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    E: EventSource,
{
    let mut refresh_tick = 0usize;

    while !stop.load(Ordering::Relaxed) {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            let keys = app.active_component().keybindings();
            app.active_component().render(frame, chunks[0]);
            render_footer(frame, chunks[1], keys);
            if let Some(error) = &app.error {
                render_error(frame, error, area);
            }
        })?;

        let event = events.next_event(EVENT_TIMEOUT)?;
        let is_timeout = event.is_none();
        match event {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    break;
                }
                let mut next = app.active_component().handle_event(&Event::Key(key));
                while let Some(action) = next {
                    if matches!(action, Action::Quit) {
                        return Ok(());
                    }
                    next = app.dispatch(action);
                }
            }
            Some(Event::Resize(_, _)) => {}
            _ => {}
        }

        match app.active_component().auto_refresh_interval() {
            Some(interval) if is_timeout => {
                refresh_tick += 1;
                if refresh_tick >= auto_refresh_ticks(interval) {
                    app.dispatch(Action::Refresh);
                    refresh_tick = 0;
                }
            }
            Some(_) => {}
            None => refresh_tick = 0,
        }
    }
    Ok(())
}

fn render_error(frame: &mut Frame, error: &str, area: Rect) {
    let error_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area)[1];
    let paragraph = Paragraph::new(error)
        .style(Style::default().fg(Color::Red))
        .block(Block::default().title("Error").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, error_area);
}

trait EventSource {
    fn next_event(&mut self, timeout: Duration) -> anyhow::Result<Option<Event>>;
}

struct CrosstermEvents;

impl EventSource for CrosstermEvents {
    fn next_event(&mut self, timeout: Duration) -> anyhow::Result<Option<Event>> {
        if event::poll(timeout).context("poll events")? {
            Ok(Some(event::read().context("read event")?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::repository::Repository;
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::dashboard::AUTO_REFRESH_INTERVAL;
    use crate::screens::{Action, Screen};

    use super::*;

    fn key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
    }

    fn ctrl_c_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        })
    }

    fn char_event(c: char) -> Event {
        key_event(KeyCode::Char(c))
    }

    /// Test event source that replays a scripted sequence of entries, where
    /// `None` represents an event-loop timeout.
    struct VecEventSource {
        events: Vec<Option<Event>>,
        index: usize,
        stop: Option<Arc<AtomicBool>>,
    }

    impl VecEventSource {
        fn new(events: Vec<Event>) -> Self {
            Self {
                events: events.into_iter().map(Some).collect(),
                index: 0,
                stop: None,
            }
        }

        fn with_stop(events: Vec<Event>, stop: Arc<AtomicBool>) -> Self {
            Self {
                events: events.into_iter().map(Some).collect(),
                index: 0,
                stop: Some(stop),
            }
        }

        fn with_timeouts(events: Vec<Option<Event>>, stop: Arc<AtomicBool>) -> Self {
            Self {
                events,
                index: 0,
                stop: Some(stop),
            }
        }
    }

    impl EventSource for VecEventSource {
        fn next_event(&mut self, _timeout: Duration) -> anyhow::Result<Option<Event>> {
            if self.index < self.events.len() {
                let event = self.events[self.index].clone();
                self.index += 1;
                Ok(event)
            } else {
                if let Some(stop) = &self.stop {
                    stop.store(true, Ordering::Relaxed);
                }
                Ok(None)
            }
        }
    }

    fn setup_app_with_project() -> (Arc<StubRepository>, App) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First project").unwrap();
        let mut app = App::new(repo.clone());
        app.dispatch(Action::Navigate(Screen::ProjectDashboard {
            project_id: project.id.unwrap(),
        }));
        app.dispatch(Action::Navigate(Screen::ProjectList));
        (repo, app)
    }

    fn run_with_events(app: &mut App, events: Vec<Event>) -> anyhow::Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut events = VecEventSource::with_stop(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend)?;
        run_loop(app, &mut terminal, &mut events, &stop)
    }

    #[test]
    fn run_loop_breaks_on_quit_action() {
        let (_repo, mut app) = setup_app_with_project();
        let events = vec![char_event('q'), key_event(KeyCode::Enter)];

        let result = run_with_events(&mut app, events);

        assert!(result.is_ok());
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn event_loop_navigates_entry_to_dashboard_on_enter() {
        let (_repo, mut app) = setup_app_with_project();
        assert_eq!(app.screen, Screen::ProjectList);

        run_with_events(&mut app, vec![key_event(KeyCode::Enter)]).unwrap();

        assert!(matches!(app.screen, Screen::ProjectDashboard { .. }));
    }

    #[test]
    fn event_loop_ctrl_c_exits() {
        let (_repo, mut app) = setup_app_with_project();
        let result = run_with_events(&mut app, vec![ctrl_c_event()]);
        assert!(result.is_ok());
    }

    #[test]
    fn dashboard_auto_refreshes_after_timeouts() {
        let (repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));

        repo.create_task(1, "Auto", "", Status::Todo, Priority::Low)
            .unwrap();

        let events: Vec<Option<Event>> =
            std::iter::repeat_n(None, auto_refresh_ticks(AUTO_REFRESH_INTERVAL))
                .chain(std::iter::once(Some(char_event('q'))))
                .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        run_loop(&mut app, &mut terminal, &mut source, &stop).unwrap();

        let text = render_app_to_string(&mut app);
        assert!(
            text.contains("Auto"),
            "dashboard should auto-refresh and show the new task"
        );
    }

    #[test]
    fn dashboard_auto_refresh_pauses_while_task_list_dialog_is_open() {
        let (repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        repo.create_task(1, "Dialog task", "", Status::Todo, Priority::Low)
            .unwrap();

        let open_filter_events: Vec<Option<Event>> = std::iter::once(Some(char_event('f')))
            .chain(std::iter::repeat_n(
                None,
                auto_refresh_ticks(AUTO_REFRESH_INTERVAL),
            ))
            .chain(std::iter::once(Some(char_event('q'))))
            .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(open_filter_events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        run_loop(&mut app, &mut terminal, &mut source, &stop).unwrap();

        let text = render_app_to_string(&mut app);
        assert!(text.contains("Filter"));
        assert!(!text.contains("Dialog task"));

        let close_filter_events: Vec<Option<Event>> =
            std::iter::once(Some(key_event(KeyCode::Esc)))
                .chain(std::iter::repeat_n(
                    None,
                    auto_refresh_ticks(AUTO_REFRESH_INTERVAL),
                ))
                .chain(std::iter::once(Some(char_event('q'))))
                .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(close_filter_events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        run_loop(&mut app, &mut terminal, &mut source, &stop).unwrap();

        assert!(render_app_to_string(&mut app).contains("Dialog task"));
    }

    #[test]
    fn non_dashboard_does_not_auto_refresh_after_timeouts() {
        let (repo, mut app) = setup_app_with_project();
        assert_eq!(app.screen, Screen::ProjectList);

        repo.create_project("Beta", "").unwrap();

        let events: Vec<Option<Event>> =
            std::iter::repeat_n(None, auto_refresh_ticks(AUTO_REFRESH_INTERVAL))
                .chain(std::iter::once(Some(char_event('q'))))
                .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        run_loop(&mut app, &mut terminal, &mut source, &stop).unwrap();

        let text = render_app_to_string(&mut app);
        assert!(
            text.contains("Alpha"),
            "project list should still show the originally loaded project"
        );
        assert!(
            !text.contains("Beta"),
            "project list must not auto-refresh on a non-dashboard screen"
        );
    }

    #[test]
    fn sigterm_flag_breaks_loop() {
        let (_repo, mut app) = setup_app_with_project();
        let stop = Arc::new(AtomicBool::new(true));
        let mut events = VecEventSource::new(vec![]);
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let result = run_loop(&mut app, &mut terminal, &mut events, &stop);

        assert!(result.is_ok());
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn sigterm_signal_sets_stop_flag() {
        let stop = setup_signal_flag().unwrap();
        assert!(!stop.load(Ordering::Relaxed));

        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            signal_hook::low_level::raise(SIGTERM).unwrap();
        });

        let start = std::time::Instant::now();
        while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(stop.load(Ordering::Relaxed));
    }

    #[test]
    fn full_navigation_entry_to_dashboard_to_form_and_back() {
        let (_repo, mut app) = setup_app_with_project();
        let events = vec![
            key_event(KeyCode::Enter),
            char_event('t'),
            key_event(KeyCode::Esc),
            key_event(KeyCode::Esc),
        ];

        run_with_events(&mut app, events).unwrap();

        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn entry_screen_snapshot() {
        let (_repo, mut app) = setup_app_with_project();
        insta::assert_snapshot!(render_app_to_string(&mut app));
    }

    #[test]
    fn dashboard_screen_snapshot() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        insta::assert_snapshot!(render_app_to_string(&mut app));
    }

    #[test]
    fn task_form_screen_snapshot() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id: 1,
            task_id: None,
        }));
        insta::assert_snapshot!(render_app_to_string(&mut app));
    }

    #[test]
    fn entry_footer_shows_context_keys() {
        let (_repo, mut app) = setup_app_with_project();
        let text = render_app_to_string(&mut app);
        let last_line = text.lines().last().unwrap();
        assert!(last_line.contains("Enter: Open"));
        assert!(last_line.contains("c: Create"));
        assert!(last_line.contains("q: Quit"));
    }

    #[test]
    fn dashboard_footer_shows_context_keys() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        let text = render_app_to_string(&mut app);
        let last_line = text.lines().last().unwrap();
        assert!(last_line.contains("j/Down: Down"));
        assert!(last_line.contains("Enter: Edit"));
        assert!(last_line.contains("Tab: Cycle"));
        assert!(last_line.contains("d: Delete"));
        assert!(last_line.contains("f: Filter"));
    }

    #[test]
    fn task_form_footer_shows_context_keys() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            project_id: 1,
            task_id: None,
        }));
        let text = render_app_to_string(&mut app);
        let last_line = text.lines().last().unwrap();
        assert!(last_line.contains("Ctrl+s: Submit"));
        assert!(last_line.contains("Esc: Cancel"));
    }

    #[test]
    fn layout_reserves_bottom_row_for_footer() {
        let (_repo, mut app) = setup_app_with_project();
        let text = render_app_to_string(&mut app);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 24);
        assert!(lines[0].contains("Projects"));
        assert!(lines.last().unwrap().contains("Enter: Open"));
        assert!(!lines[lines.len() - 2].contains("Enter: Open"));
    }

    fn render_app_to_string(app: &mut App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(1)])
                    .split(area);
                let keys = app.active_component().keybindings();
                app.active_component().render(frame, chunks[0]);
                render_footer(frame, chunks[1], keys);
                if let Some(error) = &app.error {
                    render_error(frame, error, area);
                }
            })
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let area = buffer.area();
        let mut lines = Vec::new();
        for y in area.y..area.y + area.height {
            let mut line = String::new();
            for x in area.x..area.x + area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }
}
