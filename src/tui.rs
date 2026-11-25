// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{ExecutableCommand, cursor};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::flag;

pub mod events;
pub mod sessions;

use crate::app::App;
use crate::config::AppearanceConfig;
use crate::screens::components::glyphs::Glyphs;
use crate::screens::components::help;
use crate::screens::components::popup::{centered_rect, render_popup_scaffold};
use crate::screens::components::theme::Theme;
use crate::screens::components::toast;
use crate::screens::components::top_bar;
use crate::screens::keybinding_footer::render_footer;
use crate::screens::{Action, UiContext};
use crate::update::update_command;

use self::events::{EventSource, TuiEvent, TuiEvents};
use self::sessions::AgentSessions;

/// How long the loop waits for an event before producing an animation tick.
/// It also paces auto-refresh: see [`auto_refresh_ticks`].
const EVENT_TIMEOUT: Duration = Duration::from_millis(100);
/// xterm-style escape codes that save and restore the window title. Terminals
/// that do not support them ignore the sequences.
const PUSH_TITLE: &[u8] = b"\x1b[22;0t";
const POP_TITLE: &[u8] = b"\x1b[23;0t";
/// Whether the title was pushed and still needs restoring. Guards against a
/// second restore (panic hook followed by `Drop`) popping an outer title.
static TITLE_PUSHED: AtomicBool = AtomicBool::new(false);

struct CancelUpdateCheckOnDrop(Option<Arc<AtomicBool>>);

impl Drop for CancelUpdateCheckOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Number of consecutive event-loop timeouts that make up an auto-refresh interval.
fn auto_refresh_ticks(interval: Duration) -> usize {
    interval.as_millis().div_ceil(EVENT_TIMEOUT.as_millis()) as usize
}

/// Restores the terminal state that the TUI changes. Errors are deliberately
/// ignored: this also runs from `Drop` and from the panic hook, where there is
/// nowhere useful to report them.
fn restore_terminal() {
    let mut stdout = io::stdout();
    restore_terminal_on(&mut stdout);
}

fn restore_terminal_on(out: &mut impl Write) {
    let _ = disable_raw_mode();
    let _ = out.execute(LeaveAlternateScreen);
    if TITLE_PUSHED.swap(false, Ordering::Relaxed) {
        let _ = out.write_all(POP_TITLE);
    }
    let _ = out.execute(cursor::Show);
}

/// Installs the terminal changes on construction and undoes them on drop, so a
/// panic or an early error can never leave the user in raw mode with an
/// alternate screen and a hidden cursor.
struct TerminalGuard;

impl TerminalGuard {
    fn install() -> anyhow::Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = io::stdout();
        stdout
            .execute(EnterAlternateScreen)
            .context("enter alternate screen")?;
        stdout.execute(cursor::Hide).context("hide cursor")?;
        stdout.write_all(PUSH_TITLE).context("save window title")?;
        TITLE_PUSHED.store(true, Ordering::Relaxed);
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Restores the terminal before delegating to the previously installed hook,
/// so a panic still prints its message on a usable terminal.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

pub async fn run(mut app: App, appearance: &AppearanceConfig) -> anyhow::Result<()> {
    let _cancel_update_check = CancelUpdateCheckOnDrop(app.update_check_cancel_handle());
    install_panic_hook();
    let _terminal_guard = TerminalGuard::install()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    let terminate = setup_signal_flag().context("setup signal flag")?;
    // Every source —keys, ticks, the update check and the agent sessions—
    // arrives through this one stream.
    let mut events = TuiEvents::start(app.pending_update_slot(), EVENT_TIMEOUT)
        .context("start the event loop")?;
    let mut sessions = AgentSessions::new(events.sender());
    // Composition root: the resolved configuration becomes the UI context
    // once, before the first frame.
    let ui = ui_context(appearance);

    run_loop(
        &mut app,
        &mut terminal,
        &mut events,
        &terminate,
        &ui,
        &mut sessions,
    )
    .await
}

/// Builds the UI context from the resolved appearance settings. This is the
/// only place where a palette, a token override and a glyph/motion switch
/// become the values a render sees.
fn ui_context(appearance: &AppearanceConfig) -> UiContext {
    let theme = Theme::from_parts(
        appearance.palette.as_deref(),
        &appearance.tokens,
        appearance.colors_enabled,
    );
    let glyphs = if appearance.ascii {
        Glyphs::ascii()
    } else {
        Glyphs::unicode()
    };
    UiContext::new(theme, glyphs, appearance.reduced_motion)
}

fn setup_signal_flag() -> anyhow::Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, stop.clone())?;
    flag::register(SIGINT, stop.clone())?;
    Ok(stop)
}

/// Measures the time since the event loop started and builds the context of
/// each frame from it.
///
/// There is no global state to withdraw on exit: the base context resolved by
/// the composition root has no frame time, so renders made after the loop (or
/// before it) stay static. Dropping the clock simply stops producing animated
/// frames.
struct FrameClock {
    started: std::time::Instant,
}

impl FrameClock {
    fn start() -> Self {
        Self {
            started: std::time::Instant::now(),
        }
    }

    /// The context of the frame being drawn: `base` with the elapsed time
    /// since the loop started.
    fn frame(&self, base: &UiContext) -> UiContext {
        base.with_frame_time(self.started.elapsed())
    }
}

/// Runs the loop and, whatever made it stop, cancels every agent session
/// before returning. This is the one place that guarantees the interface never
/// leaves an agent running.
async fn run_loop<B, E>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    events: &mut E,
    stop: &AtomicBool,
    ui: &UiContext,
    sessions: &mut AgentSessions,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    E: EventSource,
{
    let result = drive(app, terminal, events, stop, ui).await;
    sessions.cancel_all().await;
    result
}

/// The loop itself: draw the frame, then react to exactly one event.
async fn drive<B, E>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    events: &mut E,
    stop: &AtomicBool,
    ui: &UiContext,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    E: EventSource,
{
    let mut refresh_tick = 0usize;
    let mut shown_title = String::new();
    let frame_clock = FrameClock::start();

    while !stop.load(Ordering::Relaxed) {
        let frame_ui = frame_clock.frame(ui);
        app.tick();
        app.active_component().tick(&frame_ui);
        let title = top_bar::window_title(&app.breadcrumb());
        if title != shown_title {
            events.set_title(&title);
            shown_title = title;
        }
        terminal.draw(|frame| render_app_frame(app, frame, &frame_ui))?;

        match events.next_event().await? {
            TuiEvent::Input(event) => {
                if matches!(handle_input(app, event), Flow::Quit) {
                    return Ok(());
                }
            }
            TuiEvent::Tick => {
                if let Some(interval) = app.active_component().auto_refresh_interval() {
                    refresh_tick += 1;
                    if refresh_tick >= auto_refresh_ticks(interval) {
                        app.dispatch(Action::Refresh);
                        refresh_tick = 0;
                    }
                } else {
                    refresh_tick = 0;
                }
            }
            TuiEvent::Update(version) => app.set_update_available(version),
            TuiEvent::Agent(event) => app.record_agent_event(*event),
        }
    }
    Ok(())
}

/// What a handled input asks the loop to do next.
enum Flow {
    Continue,
    Quit,
}

/// Routes one terminal event: global shortcuts first, then the active screen.
fn handle_input(app: &mut App, event: Event) -> Flow {
    // Resize, paste, focus and mouse events are redrawn but not routed.
    let Event::Key(key) = event else {
        return Flow::Continue;
    };
    if key.kind != KeyEventKind::Press {
        return Flow::Continue;
    }
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Flow::Quit;
    }
    if app.handle_update_key(key.code, key.modifiers)
        || app.handle_help_key(key.code, key.modifiers)
        || app.handle_toast_key(key.code)
    {
        return Flow::Continue;
    }
    let mut next = app.active_component().handle_event(&Event::Key(key));
    while let Some(action) = next {
        if matches!(action, Action::Quit) {
            return Flow::Quit;
        }
        next = app.dispatch(action);
    }
    Flow::Continue
}

/// Shared drawing for the whole application frame: active screen, footer
/// (with the update hint when available), update banner and error overlay.
fn render_app_frame(app: &mut App, frame: &mut Frame, ui: &UiContext) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    let (top, body, footer) = (chunks[0], chunks[1], chunks[2]);
    top_bar::render(
        frame,
        top,
        &app.breadcrumb(),
        app.current_version(),
        app.update_version(),
        ui,
    );
    let keys = app.active_component().keybindings();
    app.active_component().render(frame, body, ui);
    let help_hint = !app.screen.accepts_text_input();
    render_footer(frame, footer, keys, help_hint, ui);
    if let (true, Some(version)) = (app.is_update_banner_open(), app.update_version()) {
        render_update_banner(frame, version, app.current_version(), area, ui);
    }
    if app.is_help_open() {
        let sections = app.active_component().help_sections();
        help::render_overlay(frame, area, &sections, ui);
    }
    if let Some(toast) = app.toasts().current() {
        let queued = app.toasts().len();
        let age = app.toast_age().unwrap_or_default();
        toast::render(frame, toast, queued, body, age, ui);
    }
}

fn render_update_banner(
    frame: &mut Frame,
    version: &str,
    current_version: &str,
    area: Rect,
    ui: &UiContext,
) {
    let popup = centered_rect(64, 9, area);
    let inner = render_popup_scaffold(frame, popup, "Update available", ui);
    let command = update_command(version);
    let text = format!(
        "New version: v{version} (current: v{current_version})\n\nUpdate with:\n{command}\n\nEsc: close"
    );
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
}

#[cfg(test)]
mod tests {
    use crate::db::repository::RepositoryPolicy;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::db::repository::{ProjectRepository, TaskRepository};
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::AUTO_REFRESH_INTERVAL;
    use crate::screens::{Action, Screen};

    use super::*;

    #[test]
    fn ui_context_reflects_the_resolved_appearance() {
        use std::collections::BTreeMap;

        use ratatui::style::Color;

        let appearance = AppearanceConfig {
            palette: Some("bright".to_string()),
            colors_enabled: false,
            tokens: BTreeMap::from([("accent".to_string(), "red".to_string())]),
            ascii: true,
            reduced_motion: true,
        };

        let ui = ui_context(&appearance);

        assert!(!ui.theme().colors_enabled);
        assert_eq!(ui.theme().accent, Color::Red);
        assert_eq!(ui.glyphs(), Glyphs::ascii());
        assert!(ui.motion().is_reduced());
    }

    #[tokio::test]
    async fn restore_terminal_leaves_alternate_screen_and_shows_cursor() {
        let mut out = Vec::new();
        restore_terminal_on(&mut out);
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("\x1b[?1049l"),
            "missing LeaveAlternateScreen: {text:?}"
        );
        assert!(text.contains("\x1b[?25h"), "missing Show cursor: {text:?}");
    }

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

    /// Test event source that replays a scripted sequence of [`TuiEvent`]s.
    /// Running out of script means "the loop has nothing left to react to": it
    /// stops, exactly like a real timeout followed by a stop request.
    struct VecEventSource {
        events: Vec<TuiEvent>,
        index: usize,
        stop: Option<Arc<AtomicBool>>,
    }

    impl VecEventSource {
        fn new(events: Vec<Event>) -> Self {
            Self {
                events: events.into_iter().map(TuiEvent::Input).collect(),
                index: 0,
                stop: None,
            }
        }

        fn with_stop(events: Vec<Event>, stop: Arc<AtomicBool>) -> Self {
            Self {
                events: events.into_iter().map(TuiEvent::Input).collect(),
                index: 0,
                stop: Some(stop),
            }
        }

        /// Replays events as they are, for the ones a terminal never produces.
        fn with_events(events: Vec<TuiEvent>, stop: Arc<AtomicBool>) -> Self {
            Self {
                events,
                index: 0,
                stop: Some(stop),
            }
        }

        /// `None` entries become animation ticks.
        fn with_timeouts(events: Vec<Option<Event>>, stop: Arc<AtomicBool>) -> Self {
            Self {
                events: events
                    .into_iter()
                    .map(|event| event.map_or(TuiEvent::Tick, TuiEvent::Input))
                    .collect(),
                index: 0,
                stop: Some(stop),
            }
        }
    }

    impl EventSource for VecEventSource {
        async fn next_event(&mut self) -> anyhow::Result<TuiEvent> {
            if self.index < self.events.len() {
                let event = self.events[self.index].clone();
                self.index += 1;
                return Ok(event);
            }
            if let Some(stop) = &self.stop {
                stop.store(true, Ordering::Relaxed);
            }
            Ok(TuiEvent::Tick)
        }
    }

    /// A registry with no sessions, which is all these loop tests need.
    fn test_sessions() -> AgentSessions {
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        AgentSessions::new(sender)
    }

    fn setup_app_with_project() -> (Arc<StubRepository>, App) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "First project", None).unwrap();
        let project_id = project.id.unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "Task 1",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "Task 2",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();
        repo.create_task(
            project_id,
            repo.default_version_id(project_id).unwrap(),
            "Task 3",
            "",
            Status::Done,
            Priority::Low,
        )
        .unwrap();
        let mut app = App::new(repo.clone());
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
        app.dispatch(Action::Navigate(Screen::ProjectList));
        (repo, app)
    }

    async fn run_with_events(app: &mut App, events: Vec<Event>) -> anyhow::Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let mut events = VecEventSource::with_stop(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend)?;
        run_loop(
            app,
            &mut terminal,
            &mut events,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
    }

    #[tokio::test]
    async fn run_loop_breaks_on_quit_action() {
        let (_repo, mut app) = setup_app_with_project();
        let events = vec![char_event('q'), key_event(KeyCode::Enter)];

        let result = run_with_events(&mut app, events).await;

        assert!(result.is_ok());
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn error_toast_is_drawn_above_the_footer() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::SaveProject {
            project_id: None,
            name: "   ".to_string(),
            description: String::new(),
            working_dir: String::new(),
        });

        let text = render_app_to_string(&mut app);
        let lines: Vec<&str> = text.lines().collect();
        let footer = lines.last().unwrap();
        let toast_row = lines
            .iter()
            .position(|line| line.contains("Project name cannot be empty"))
            .expect("toast message is drawn");

        assert!(lines[toast_row - 1].contains("Error"), "{text}");
        assert!(
            toast_row < lines.len() - 1,
            "toast must not cover the footer"
        );
        assert!(footer.contains("Quit"), "footer stays visible: {footer:?}");
    }

    #[tokio::test]
    async fn question_mark_opens_help_and_swallows_keys_until_closed() {
        let (_repo, mut app) = setup_app_with_project();
        // `q` would quit, but while the overlay is open it must be swallowed.
        let events = vec![char_event('?'), char_event('q')];

        run_with_events(&mut app, events).await.unwrap();

        assert!(app.is_help_open());
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[tokio::test]
    async fn esc_closes_help_without_reaching_the_screen() {
        let (_repo, mut app) = setup_app_with_project();
        let events = vec![char_event('?'), key_event(KeyCode::Esc), char_event('q')];

        let result = run_with_events(&mut app, events).await;

        // The final `q` quits, proving the overlay closed on Esc.
        assert!(result.is_ok());
        assert!(!app.is_help_open());
    }

    #[tokio::test]
    async fn help_overlay_lists_the_active_screen_sections() {
        let (_repo, mut app) = setup_app_with_project();
        run_with_events(&mut app, vec![char_event('?')])
            .await
            .unwrap();

        let text = render_app_to_string(&mut app);

        assert!(text.contains("Shortcuts"), "{text}");
        assert!(text.contains("Navigation"), "{text}");
        assert!(text.contains("Projects"), "{text}");
    }

    #[test]
    fn tui_cleanup_cancels_background_update_check() {
        let cancel = Arc::new(AtomicBool::new(false));

        drop(CancelUpdateCheckOnDrop(Some(cancel.clone())));

        assert!(cancel.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn event_loop_navigates_entry_to_dashboard_on_enter() {
        let (_repo, mut app) = setup_app_with_project();
        assert_eq!(app.screen, Screen::ProjectList);

        run_with_events(&mut app, vec![key_event(KeyCode::Enter)])
            .await
            .unwrap();

        assert!(matches!(app.screen, Screen::ProjectDashboard { .. }));
    }

    #[tokio::test]
    async fn event_loop_ctrl_c_exits() {
        let (_repo, mut app) = setup_app_with_project();
        let result = run_with_events(&mut app, vec![ctrl_c_event()]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn the_loop_applies_the_version_the_background_check_found() {
        let (_repo, mut app) = setup_app_with_project();
        assert!(!app.has_update());
        let stop = Arc::new(AtomicBool::new(false));
        let mut source =
            VecEventSource::with_events(vec![TuiEvent::Update("0.9.0".to_string())], stop.clone());
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();

        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

        assert_eq!(app.update_version(), Some("0.9.0"));
    }

    #[tokio::test]
    async fn the_loop_records_the_progress_of_an_agent_session() {
        let (_repo, mut app) = setup_app_with_project();
        let session = crate::acp::SessionId::new("sess-1");
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_events(
            vec![TuiEvent::Agent(Box::new(events::AgentEvent {
                session: session.clone(),
                agent: "claude".to_string(),
                event: crate::acp::AcpEvent::Update(crate::acp::SessionUpdateNotification {
                    session_id: session.clone(),
                    update: crate::acp::SessionUpdate::AgentMessageChunk {
                        content: crate::acp::ContentBlock::text("almost there"),
                        message_id: None,
                    },
                }),
            }))],
            stop.clone(),
        );
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();

        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

        let activity = app.agent_activity().get(&session).expect("the session");
        assert_eq!(activity.agent, "claude");
        assert_eq!(activity.updates, 1);
        assert_eq!(activity.last_text.as_deref(), Some("almost there"));
    }

    #[tokio::test]
    async fn dashboard_auto_refreshes_after_timeouts() {
        let (repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id: 1,
            version_id: app.repo.default_version_id(1).unwrap(),
        }));

        repo.create_task(
            1,
            repo.default_version_id(1).unwrap(),
            "Auto",
            "",
            Status::Todo,
            Priority::Low,
        )
        .unwrap();

        let events: Vec<Option<Event>> =
            std::iter::repeat_n(None, auto_refresh_ticks(AUTO_REFRESH_INTERVAL))
                .chain(std::iter::once(Some(char_event('q'))))
                .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

        let text = render_app_to_string(&mut app);
        assert!(
            text.contains("Auto"),
            "dashboard should auto-refresh and show the new task"
        );
    }

    #[tokio::test]
    async fn dashboard_auto_refresh_pauses_while_task_list_dialog_is_open() {
        let (repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::VersionDashboard {
            project_id: 1,
            version_id: app.repo.default_version_id(1).unwrap(),
        }));
        repo.create_task(
            1,
            repo.default_version_id(1).unwrap(),
            "Dialog task",
            "",
            Status::Todo,
            Priority::Low,
        )
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
        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

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
        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

        assert!(render_app_to_string(&mut app).contains("Dialog task"));
    }

    #[tokio::test]
    async fn project_list_auto_refreshes_progress_after_timeouts() {
        let (repo, mut app) = setup_app_with_project();
        assert_eq!(app.screen, Screen::ProjectList);

        let task_id = repo
            .list_tasks_by_project(1)
            .unwrap()
            .into_iter()
            .find(|task| task.status == Status::Todo)
            .unwrap()
            .id
            .unwrap();
        repo.update_task_status(task_id, Status::Done).unwrap();

        let events: Vec<Option<Event>> =
            std::iter::repeat_n(None, auto_refresh_ticks(AUTO_REFRESH_INTERVAL))
                .chain(std::iter::once(Some(char_event('q'))))
                .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = VecEventSource::with_timeouts(events, stop.clone());
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();

        let text = render_app_to_string(&mut app);
        assert!(
            text.contains("67%") && text.contains("· 1 version"),
            "project progress should be refreshed"
        );
    }

    /// Event source that records every window title the loop asks for.
    struct TitleRecorder {
        titles: Vec<String>,
        remaining: usize,
        stop: Arc<AtomicBool>,
    }

    impl EventSource for TitleRecorder {
        async fn next_event(&mut self) -> anyhow::Result<TuiEvent> {
            if self.remaining == 0 {
                self.stop.store(true, Ordering::Relaxed);
            }
            self.remaining = self.remaining.saturating_sub(1);
            Ok(TuiEvent::Tick)
        }

        fn set_title(&mut self, title: &str) {
            self.titles.push(title.to_string());
        }
    }

    #[tokio::test]
    async fn window_title_is_set_for_the_current_context_only_when_it_changes() {
        let (_repo, mut app) = setup_app_with_project();
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = TitleRecorder {
            titles: Vec::new(),
            remaining: 3,
            stop: stop.clone(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();
        assert_eq!(source.titles, vec!["pmp — Projects".to_string()]);

        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        let stop = Arc::new(AtomicBool::new(false));
        let mut source = TitleRecorder {
            titles: Vec::new(),
            remaining: 2,
            stop: stop.clone(),
        };
        run_loop(
            &mut app,
            &mut terminal,
            &mut source,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await
        .unwrap();
        assert_eq!(source.titles, vec!["pmp — Alpha".to_string()]);
    }

    #[test]
    fn top_bar_shows_breadcrumb_on_the_first_row() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        let text = render_app_to_string_at_size(&mut app, 120, 24);
        let first = text.lines().next().unwrap();
        assert!(first.contains("PMP v"), "{first:?}");
        assert!(first.contains("Alpha"), "{first:?}");
    }

    #[tokio::test]
    async fn sigterm_flag_breaks_loop() {
        let (_repo, mut app) = setup_app_with_project();
        let stop = Arc::new(AtomicBool::new(true));
        let mut events = VecEventSource::new(vec![]);
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let result = run_loop(
            &mut app,
            &mut terminal,
            &mut events,
            &stop,
            &UiContext::default(),
            &mut test_sessions(),
        )
        .await;

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

    #[tokio::test]
    async fn full_navigation_entry_to_dashboard_to_form_and_back() {
        let (_repo, mut app) = setup_app_with_project();
        let events = vec![
            key_event(KeyCode::Enter),
            char_event('t'),
            key_event(KeyCode::Esc),
            key_event(KeyCode::Esc),
        ];

        run_with_events(&mut app, events).await.unwrap();

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
            version_id: app.repo.default_version_id(1).unwrap(),
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
        // Render wide enough to fit every context key; the footer truncates at
        // 80 columns, which would hide the trailing keys.
        let text = render_app_to_string_at_size(&mut app, 160, 24);
        let last_line = text.lines().last().unwrap();
        assert!(last_line.contains("Enter: Open"));
        assert!(last_line.contains("c: New Version"));
        assert!(last_line.contains("v: Edit Version"));
        assert!(last_line.contains("r: Cycle Status"));
        assert!(last_line.contains("d: Delete"));
    }

    #[test]
    fn task_form_footer_shows_context_keys() {
        let (_repo, mut app) = setup_app_with_project();
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(1).unwrap(),
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
        render_app_to_string_at_size(app, 80, 24)
    }

    fn render_app_to_string_at_size(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_app_frame(app, frame, &UiContext::default()))
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

    fn app_with_update(version: &str) -> (Arc<StubRepository>, App) {
        let (repo, mut app) = setup_app_with_project();
        app.set_update_available(version.to_string());
        (repo, app)
    }

    #[tokio::test]
    async fn u_opens_update_banner_and_esc_closes_it() {
        let (_repo, mut app) = app_with_update("0.4.0");

        run_with_events(&mut app, vec![char_event('u')])
            .await
            .unwrap();
        assert!(app.is_update_banner_open());
        assert!(render_app_to_string(&mut app).contains("Update available"));

        run_with_events(&mut app, vec![key_event(KeyCode::Esc)])
            .await
            .unwrap();
        assert!(!app.is_update_banner_open());
        assert!(!render_app_to_string(&mut app).contains("Update available"));
    }

    #[tokio::test]
    async fn u_does_nothing_without_update() {
        let (_repo, mut app) = setup_app_with_project();

        run_with_events(&mut app, vec![char_event('u')])
            .await
            .unwrap();

        assert!(!app.is_update_banner_open());
    }

    #[tokio::test]
    async fn u_is_text_input_inside_forms() {
        let (_repo, mut app) = app_with_update("0.4.0");
        app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id: 1 }));
        app.dispatch(Action::Navigate(Screen::TaskForm {
            version_id: app.repo.default_version_id(1).unwrap(),
            task_id: None,
        }));

        run_with_events(&mut app, vec![char_event('u')])
            .await
            .unwrap();

        assert!(!app.is_update_banner_open());
    }

    #[tokio::test]
    async fn open_update_banner_swallows_keys_except_esc() {
        let (_repo, mut app) = app_with_update("0.4.0");

        // Enter would open the dashboard if it reached the project list.
        run_with_events(&mut app, vec![char_event('u'), key_event(KeyCode::Enter)])
            .await
            .unwrap();

        assert!(app.is_update_banner_open());
        assert_eq!(app.screen, Screen::ProjectList);
    }

    #[test]
    fn update_hint_is_in_the_top_bar_not_the_footer() {
        let (_repo, mut app) = app_with_update("0.4.0");
        let text = render_app_to_string_at_size(&mut app, 120, 24);
        let first = text.lines().next().unwrap();
        let last_line = text.lines().last().unwrap();
        assert!(first.contains("⬆ New v0.4.0 (u)"), "{first:?}");
        assert!(!last_line.contains("(u)"), "{last_line:?}");
        assert!(last_line.contains("Enter: Open"));
    }

    #[test]
    fn footer_has_no_update_hint_without_update() {
        let (_repo, mut app) = setup_app_with_project();
        let text = render_app_to_string(&mut app);
        assert!(!text.lines().last().unwrap().contains("(u)"));
    }

    #[test]
    fn update_banner_snapshot() {
        let (_repo, mut app) = app_with_update("0.4.0");
        app.set_current_version("0.3.0");
        app.open_update_banner();
        insta::assert_snapshot!(render_app_to_string(&mut app));
    }

    #[test]
    fn frame_clock_animates_frames_and_leaves_the_base_context_still() {
        let base = UiContext::default();
        {
            let clock = FrameClock::start();
            let frame = clock.frame(&base);
            assert!(frame.frame_time().is_some());
            assert!(frame.animating());
        }
        // No global clock survives the loop: the base context stays static.
        assert_eq!(base.frame_time(), None);
        assert!(!base.animating());
    }
}
