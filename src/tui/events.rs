// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The single event stream the TUI loop consumes.
//!
//! The loop used to block on `crossterm`'s keyboard poll and only advanced on
//! its own timeout, so anything asynchronous — an agent streaming progress —
//! could only be noticed between key presses. Now every source pushes into one
//! channel:
//!
//! * terminal events, read on a dedicated thread because `crossterm` blocks,
//! * animation and auto-refresh ticks, produced by the wait deadline itself,
//! * the background update check, reported once when its slot fills, and
//! * agent sessions, forwarded by the bridges in
//!   [`sessions`](super::sessions).
//!
//! The loop receives exactly one [`TuiEvent`] per iteration, in arrival order,
//! and never asks "did the timeout fire?": [`TuiEvent::Tick`] says so.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyEvent};
use crossterm::terminal::SetTitle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::acp::{AcpEvent, Decision, PermissionRequest, SessionId, StopReason};

/// How long the input thread blocks inside `crossterm` before checking whether
/// the TUI is closing. It only bounds how long the thread takes to stop; an
/// event is forwarded the moment it arrives.
pub const INPUT_POLL: Duration = Duration::from_millis(25);

/// One thing the loop reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum TuiEvent {
    /// A terminal event: a key press, a resize, a paste.
    Input(Event),
    /// The deadline passed with nothing to report: advance animations and
    /// timers.
    Tick,
    /// The background check found a newer release.
    Update(String),
    /// Streamed progress from one agent session. Boxed because it is much
    /// larger than the other variants and it travels through a channel.
    Agent(Box<AgentEvent>),
    /// The prompt turn of one agent session ended, successfully or not. The
    /// loop records the closing note and shuts the session down with it.
    AgentFinished(Box<AgentFinished>),
    /// An agent asks permission for an operation. The loop answers from the
    /// allowlist or asks the user; the agent waits until it does.
    Permission(Box<PermissionAsk>),
}

impl TuiEvent {
    /// The key event this input carries, if it is one.
    pub fn key(&self) -> Option<KeyEvent> {
        match self {
            Self::Input(Event::Key(key)) => Some(*key),
            _ => None,
        }
    }

    /// Whether this iteration had nothing to report.
    pub fn is_tick(&self) -> bool {
        matches!(self, Self::Tick)
    }
}

/// Progress streamed by one agent session.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEvent {
    /// Session the update belongs to.
    pub session: SessionId,
    /// Profile name the session was launched with.
    pub agent: String,
    pub event: AcpEvent,
}

/// How a prompt turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRunOutcome {
    /// The agent answered with a stop reason (`end_turn`, `cancelled`, ...).
    Finished(StopReason),
    /// The request failed: the connection broke, timed out or the agent
    /// rejected it. The message is already the user-facing one.
    Failed(String),
}

/// A permission request waiting for an answer.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionAsk {
    pub session: SessionId,
    /// Profile name the session was launched with.
    pub agent: String,
    pub task_id: i64,
    pub project_id: i64,
    pub request: PermissionRequest,
    pub reply: PermissionReply,
}

/// The one-shot answer channel of a [`PermissionAsk`]. `Some(decision)` answers
/// the request and `None` cancels it. It is shared behind an `Arc` so the event
/// stays cloneable; the first call to [`PermissionReply::send`] wins.
#[derive(Clone)]
pub struct PermissionReply(Arc<StdMutex<Option<oneshot::Sender<Option<Decision>>>>>);

impl PermissionReply {
    /// A reply and the receiver the agent-side handler awaits.
    pub fn new() -> (Self, oneshot::Receiver<Option<Decision>>) {
        let (sender, receiver) = oneshot::channel();
        (Self(Arc::new(StdMutex::new(Some(sender)))), receiver)
    }

    /// Sends the answer. Returns `false` when it was already sent or the
    /// handler is gone.
    pub fn send(&self, answer: Option<Decision>) -> bool {
        let sender = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        sender.is_some_and(|sender| sender.send(answer).is_ok())
    }
}

impl std::fmt::Debug for PermissionReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PermissionReply")
            .finish_non_exhaustive()
    }
}

impl PartialEq for PermissionReply {
    /// Two replies are equal when they answer the same request.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The end of one agent turn, delivered once by the session bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFinished {
    pub session: SessionId,
    pub agent: String,
    pub task_id: i64,
    pub outcome: AgentRunOutcome,
    /// Agent message text streamed during the turn, capped by the bridge.
    pub transcript: String,
}

/// Where the loop gets its events from. Tests implement it with a script so
/// the loop stays deterministic; production uses [`TuiEvents`].
///
/// Crate-private on purpose: it exists so the loop can be driven without a
/// terminal, not as something a caller implements.
pub(crate) trait EventSource {
    /// The next event to react to. Never blocks longer than the configured
    /// deadline: it answers [`TuiEvent::Tick`] when the deadline passes.
    async fn next_event(&mut self) -> anyhow::Result<TuiEvent>;

    /// Sets the terminal window title. Does nothing by default, so a scripted
    /// source never writes escape codes to a real terminal.
    fn set_title(&mut self, _title: &str) {}
}

/// The real event source: one channel fed by the terminal input thread, the
/// update check and every agent bridge.
pub struct TuiEvents {
    sender: UnboundedSender<TuiEvent>,
    receiver: UnboundedReceiver<TuiEvent>,
    update: UpdateProbe,
    deadline: Duration,
    input: Option<InputThread>,
    /// Set by the input thread when the terminal stops cooperating, so the
    /// loop ends with an error instead of drawing forever without input.
    input_failed: Option<Arc<AtomicBool>>,
}

impl TuiEvents {
    /// Starts the production source: the shared channel plus the terminal
    /// input thread.
    pub fn start(
        update: Option<Arc<OnceLock<String>>>,
        deadline: Duration,
    ) -> anyhow::Result<Self> {
        let mut events = Self::detached(update, deadline);
        let input = InputThread::start(events.sender.clone())?;
        events.input_failed = Some(Arc::clone(&input.failed));
        events.input = Some(input);
        Ok(events)
    }

    /// The hub without terminal input. Used by tests and by any transport that
    /// feeds its own events through [`TuiEvents::sender`].
    pub fn detached(update: Option<Arc<OnceLock<String>>>, deadline: Duration) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver,
            update: UpdateProbe::new(update),
            deadline,
            input: None,
            input_failed: None,
        }
    }

    /// A producer handle: agent bridges, background jobs and tests push their
    /// events here.
    pub fn sender(&self) -> UnboundedSender<TuiEvent> {
        self.sender.clone()
    }
}

impl EventSource for TuiEvents {
    async fn next_event(&mut self) -> anyhow::Result<TuiEvent> {
        // A filled update slot wakes the loop on the next iteration instead of
        // waiting for the next timeout.
        if let Some(version) = self.update.take() {
            return Ok(TuiEvent::Update(version));
        }
        if self
            .input_failed
            .as_ref()
            .is_some_and(|failed| failed.load(Ordering::Relaxed))
        {
            anyhow::bail!("the terminal stopped delivering input");
        }
        match timeout(self.deadline, self.receiver.recv()).await {
            Ok(Some(event)) => Ok(event),
            // The hub holds a sender of its own, so the channel cannot close
            // while the loop is running.
            Ok(None) => anyhow::bail!("the TUI event channel closed unexpectedly"),
            Err(_) => Ok(TuiEvent::Tick),
        }
    }

    fn set_title(&mut self, title: &str) {
        // Terminals that do not support the sequence ignore it.
        let _ = std::io::stdout().execute(SetTitle(title));
    }
}

/// Reports the background update check once, as an event, instead of having
/// the loop poll a shared slot on every frame.
struct UpdateProbe {
    slot: Option<Arc<OnceLock<String>>>,
    reported: bool,
}

impl UpdateProbe {
    fn new(slot: Option<Arc<OnceLock<String>>>) -> Self {
        Self {
            slot,
            reported: false,
        }
    }

    /// The version the check published, the first time it is available.
    fn take(&mut self) -> Option<String> {
        if self.reported {
            return None;
        }
        let version = self.slot.as_ref()?.get()?.clone();
        self.reported = true;
        Some(version)
    }
}

/// Anything that blocks for at most `timeout` waiting for a terminal event.
/// The real one is `crossterm`; tests script it.
trait Input {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<Option<Event>>;
}

/// The real terminal, through `crossterm`.
struct Crossterm;

impl Input for Crossterm {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<Option<Event>> {
        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }
}

/// Reads terminal events on its own thread.
///
/// `crossterm` blocks, so this cannot live on the runtime: an agent streaming
/// updates would queue behind a key read. The thread stops on drop, which
/// happens before the terminal is restored (see `tui::run`).
struct InputThread {
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl InputThread {
    fn start(sender: UnboundedSender<TuiEvent>) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_failed = Arc::clone(&failed);
        let handle = std::thread::Builder::new()
            .name("pmp-input".to_string())
            .spawn(move || pump(Crossterm, &sender, &thread_stop, &thread_failed))?;
        Ok(Self {
            stop,
            failed,
            handle: Some(handle),
        })
    }
}

impl Drop for InputThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Forwards terminal events until asked to stop or until the loop is gone.
/// A terminal that stops cooperating sets `failed`, which ends the loop with
/// an error instead of leaving a TUI that can never receive a key.
fn pump(
    mut input: impl Input,
    sender: &UnboundedSender<TuiEvent>,
    stop: &AtomicBool,
    failed: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        match input.poll(INPUT_POLL) {
            Ok(Some(event)) => {
                if sender.send(TuiEvent::Input(event)).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "cannot read a terminal event");
                failed.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resize() -> Event {
        Event::Resize(80, 24)
    }

    #[tokio::test]
    async fn the_update_check_becomes_one_event() {
        let slot = Arc::new(OnceLock::new());
        let mut events = TuiEvents::detached(Some(Arc::clone(&slot)), Duration::from_millis(50));
        slot.set("0.7.0".to_string()).unwrap();
        assert_eq!(
            events.next_event().await.unwrap(),
            TuiEvent::Update("0.7.0".to_string())
        );
        // Reported once: the next wait is a normal tick.
        assert_eq!(events.next_event().await.unwrap(), TuiEvent::Tick);
    }

    #[tokio::test]
    async fn an_empty_update_slot_does_not_emit_anything() {
        let mut events =
            TuiEvents::detached(Some(Arc::new(OnceLock::new())), Duration::from_millis(10));
        assert_eq!(events.next_event().await.unwrap(), TuiEvent::Tick);
    }

    #[tokio::test]
    async fn the_deadline_produces_ticks() {
        let mut events = TuiEvents::detached(None, Duration::from_millis(10));
        let started = std::time::Instant::now();
        assert_eq!(events.next_event().await.unwrap(), TuiEvent::Tick);
        assert!(started.elapsed() >= Duration::from_millis(10));
    }

    #[tokio::test]
    async fn producers_are_served_in_arrival_order() {
        let mut events = TuiEvents::detached(None, Duration::from_secs(5));
        let sender = events.sender();
        sender.send(TuiEvent::Input(resize())).unwrap();
        sender
            .send(TuiEvent::Agent(Box::new(AgentEvent {
                session: SessionId::new("sess-1"),
                agent: "claude".to_string(),
                event: AcpEvent::Closed {
                    reason: "done".to_string(),
                },
            })))
            .unwrap();

        assert_eq!(
            events.next_event().await.unwrap(),
            TuiEvent::Input(resize())
        );
        match events.next_event().await.unwrap() {
            TuiEvent::Agent(event) => {
                assert_eq!(event.session.as_str(), "sess-1");
                assert_eq!(event.agent, "claude");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn key_events_are_distinguishable_from_other_inputs() {
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(TuiEvent::Input(Event::Key(key)).key(), Some(key));
        assert_eq!(TuiEvent::Input(resize()).key(), None);
        assert_eq!(TuiEvent::Tick.key(), None);
        assert!(TuiEvent::Tick.is_tick());
        assert!(!TuiEvent::Update("1.0.0".to_string()).is_tick());
    }

    /// Input that replays a script and then fails, which stops the pump.
    struct Scripted {
        events: std::vec::IntoIter<Event>,
    }

    impl Scripted {
        fn new(events: Vec<Event>) -> Self {
            Self {
                events: events.into_iter(),
            }
        }
    }

    impl Input for Scripted {
        fn poll(&mut self, _timeout: Duration) -> std::io::Result<Option<Event>> {
            match self.events.next() {
                Some(event) => Ok(Some(event)),
                None => Err(std::io::Error::other("the terminal went away")),
            }
        }
    }

    #[test]
    fn the_pump_forwards_input_until_the_terminal_fails() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let script = Scripted::new(vec![resize(), resize()]);
        let stop = AtomicBool::new(false);
        let failed = AtomicBool::new(false);

        pump(script, &sender, &stop, &failed);

        assert_eq!(receiver.try_recv().unwrap(), TuiEvent::Input(resize()));
        assert_eq!(receiver.try_recv().unwrap(), TuiEvent::Input(resize()));
        assert!(failed.load(Ordering::Relaxed), "the failure is reported");
    }

    #[test]
    fn the_pump_stops_when_the_loop_is_gone() {
        let (sender, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        let stop = AtomicBool::new(false);
        let failed = AtomicBool::new(false);
        // One event is enough: the failed send returns instead of looping.
        pump(Scripted::new(vec![resize()]), &sender, &stop, &failed);
    }

    #[test]
    fn the_pump_honours_the_stop_flag() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let stop = AtomicBool::new(true);
        let failed = AtomicBool::new(false);
        pump(Scripted::new(vec![resize()]), &sender, &stop, &failed);
        assert!(receiver.try_recv().is_err());
        assert!(!failed.load(Ordering::Relaxed), "stopping is not a failure");
    }

    #[tokio::test]
    async fn a_dead_terminal_ends_the_loop_with_an_error() {
        let mut events = TuiEvents::detached(None, Duration::from_millis(10));
        let failed = Arc::new(AtomicBool::new(false));
        events.input_failed = Some(Arc::clone(&failed));
        assert_eq!(events.next_event().await.unwrap(), TuiEvent::Tick);

        failed.store(true, Ordering::Relaxed);
        let error = events.next_event().await.expect_err("the loop must stop");
        assert!(error.to_string().contains("stopped delivering input"));
    }
}
