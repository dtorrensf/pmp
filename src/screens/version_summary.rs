// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Read-only view of a compacted version summary: the frozen prose plus a
//! small ASCII distribution chart parsed from the snapshot.
use std::cell::Cell;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::application;
use crate::db::repository::Repository;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::task::{Priority, Status};
use crate::domain::version::Version;
use crate::domain::version_summary::VersionSummary;
use crate::screens::components::help::{self, HelpSection};
use crate::screens::components::keys::key;
use crate::screens::components::scroll::{clamp_text_scroll, wrapped_line_count};
use crate::screens::components::stacked_bar::{share_row, status_rows};
use crate::screens::{Action, Component, Screen, UiContext};

/// Width, in cells, of the bars of the distribution chart.
const CHART_BAR_WIDTH: usize = 20;

pub struct VersionSummaryScreen {
    repo: Arc<dyn Repository>,
    project_id: i64,
    version_id: i64,
    summary_id: i64,
    version: Option<Version>,
    summary: Option<VersionSummary>,
    snapshot: Option<CompactSnapshot>,
    scroll: Cell<u16>,
}

impl VersionSummaryScreen {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[
        (key(KeyCode::Char('j')), "Down"),
        (key(KeyCode::Char('k')), "Up"),
        (key(KeyCode::Esc), "Back"),
        (key(KeyCode::Char('q')), "Quit"),
    ];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            version_id: 0,
            summary_id: 0,
            version: None,
            summary: None,
            snapshot: None,
            scroll: Cell::new(0),
        }
    }

    pub fn set_ids(&mut self, project_id: i64, version_id: i64, summary_id: i64) {
        self.project_id = project_id;
        self.version_id = version_id;
        self.summary_id = summary_id;
    }

    fn load(&mut self) -> anyhow::Result<()> {
        let summary = application::summaries::get(self.repo.as_ref(), self.summary_id)?;
        let version = application::versions::get(self.repo.as_ref(), summary.version_id)?;
        self.project_id = version.project_id;
        self.version_id = version.id.unwrap_or(self.version_id);
        // Intentional fallback: a snapshot that cannot be parsed only loses the
        // optional chart. The summary prose still renders, so the user sees
        // the content instead of an error for a missing decoration.
        self.snapshot = serde_json::from_str::<CompactSnapshot>(&summary.snapshot_json).ok();
        self.version = Some(version);
        self.summary = Some(summary);
        self.scroll.set(0);
        Ok(())
    }
}

impl Component for VersionSummaryScreen {
    fn init(&mut self) -> anyhow::Result<()> {
        self.load()
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        self.load()
    }

    fn help_sections(&self) -> Vec<HelpSection> {
        help::sections(self.keybindings(), |label| match label {
            "Down" | "Up" => help::NAVIGATION,
            _ => help::APPLICATION,
        })
    }

    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
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
        match code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Esc => Some(Action::Navigate(Screen::VersionDashboard {
                project_id: self.project_id,
                version_id: self.version_id,
            })),
            KeyCode::Char('j') => {
                self.scroll.set(self.scroll.get().saturating_add(1));
                None
            }
            KeyCode::Char('k') => {
                self.scroll.set(self.scroll.get().saturating_sub(1));
                None
            }
            _ => None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, ui: &UiContext) {
        let title = match &self.version {
            Some(v) => format!("{} · Summary", v.name),
            None => "Summary".to_string(),
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chart = self
            .snapshot
            .as_ref()
            .map(|snapshot| render_chart(snapshot, ui))
            .unwrap_or_default();
        let chart_height = chart.len() as u16 + 2;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(chart_height)])
            .split(inner);

        // Intentional fallback: no summary means no prose, not an error.
        let prose = self
            .summary
            .as_ref()
            .map(|s| s.summary.as_str())
            .unwrap_or_default();
        let max_scroll = clamp_text_scroll(
            self.scroll.get(),
            wrapped_line_count(prose, chunks[0].width),
            chunks[0].height,
        );
        self.scroll.set(max_scroll);
        frame.render_widget(
            Paragraph::new(prose)
                .wrap(Wrap { trim: true })
                .scroll((max_scroll, 0)),
            chunks[0],
        );

        let chart_block = Block::default().title("Distribution").borders(Borders::ALL);
        let chart_inner = chart_block.inner(chunks[1]);
        frame.render_widget(chart_block, chunks[1]);
        frame.render_widget(Paragraph::new(Text::from(chart)), chart_inner);
    }
}

fn render_chart(snapshot: &CompactSnapshot, ui: &UiContext) -> Vec<Line<'static>> {
    let total = snapshot.tasks.len();
    let counts = [Status::Todo, Status::InProgress, Status::Done]
        .map(|status| count_status(snapshot, status));

    // Status rows and priority rows share one scale and one layout: a full
    // bar is every task of the version.
    let mut lines = status_rows(counts.map(|c| c as u64), CHART_BAR_WIDTH, ui);
    for (label, priority) in [
        ("high", Priority::High),
        ("medium", Priority::Medium),
        ("low", Priority::Low),
    ] {
        lines.push(share_row(
            Some((ui.priority_icon(priority), ui.priority_style(priority))),
            label,
            count_priority(snapshot, priority) as u64,
            total as u64,
            ui.priority_style(priority),
            CHART_BAR_WIDTH,
            ui,
        ));
    }
    lines
}

fn count_status(snapshot: &CompactSnapshot, status: Status) -> usize {
    snapshot.tasks.iter().filter(|t| t.status == status).count()
}

fn count_priority(snapshot: &CompactSnapshot, priority: Priority) -> usize {
    snapshot
        .tasks
        .iter()
        .filter(|t| t.priority == priority)
        .count()
}

#[cfg(test)]
mod tests {
    use crate::db::repository::*;
    use crossterm::event::KeyCode;

    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};
    use crate::screens::test_utils::{key_event, render_to_string};

    use super::*;

    fn screen_with_summary() -> (VersionSummaryScreen, Arc<StubRepository>, i64, i64) {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let version = repo.create_version(pid, "v1.0.0", "").unwrap();
        let vid = version.id.unwrap();
        let task = repo
            .create_task(pid, vid, "A", "", Status::Done, Priority::High)
            .unwrap();
        repo.add_task_note(task.id.unwrap(), "a note").unwrap();
        let snapshot = repo.build_compact_snapshot(vid).unwrap();
        let summary = repo
            .create_version_summary(vid, "Release summary prose", &snapshot)
            .unwrap();
        let summary_id = summary.id.unwrap();

        let mut screen = VersionSummaryScreen::new(repo.clone());
        screen.set_ids(pid, vid, summary_id);
        screen.init().unwrap();
        (screen, repo, pid, vid)
    }

    #[test]
    fn init_loads_summary_and_version() {
        let (screen, _repo, pid, vid) = screen_with_summary();
        assert_eq!(screen.project_id, pid);
        assert_eq!(screen.version_id, vid);
        assert_eq!(screen.version.as_ref().unwrap().name, "v1.0.0");
        assert_eq!(
            screen.summary.as_ref().unwrap().summary,
            "Release summary prose"
        );
        let ui = UiContext::default();
        let chart = render_chart(screen.snapshot.as_ref().unwrap(), &ui);
        assert_eq!(chart.len(), 6);
    }

    #[test]
    fn esc_returns_to_version_dashboard() {
        let (mut screen, _repo, pid, vid) = screen_with_summary();
        let action = screen.handle_event(&key_event(KeyCode::Esc));
        assert_eq!(
            action,
            Some(Action::Navigate(Screen::VersionDashboard {
                project_id: pid,
                version_id: vid,
            }))
        );
    }

    #[test]
    fn scroll_moves_with_j_and_k() {
        let (mut screen, _repo, _pid, _vid) = screen_with_summary();
        screen.handle_event(&key_event(KeyCode::Char('j')));
        screen.handle_event(&key_event(KeyCode::Char('j')));
        assert_eq!(screen.scroll.get(), 2);
        screen.handle_event(&key_event(KeyCode::Char('k')));
        assert_eq!(screen.scroll.get(), 1);
    }

    #[test]
    fn renders_prose_and_distribution() {
        let (screen, _repo, _pid, _vid) = screen_with_summary();
        let text = render_to_string(&screen, 60, 20);
        assert!(text.contains("v1.0.0 · Summary"));
        assert!(text.contains("Release summary prose"));
        assert!(text.contains("Distribution"));
        assert!(text.contains("Done"));
        assert!(text.contains("high"));
    }

    #[test]
    fn version_summary_default_snapshot() {
        let (screen, _repo, _pid, _vid) = screen_with_summary();
        insta::assert_snapshot!(render_to_string(&screen, 60, 20));
    }

    #[test]
    fn version_summary_scroll_snapshot() {
        let repo = Arc::new(StubRepository::default());
        let project = repo.create_project("Alpha", "").unwrap();
        let pid = project.id.unwrap();
        let version = repo.create_version(pid, "v1.0.0", "").unwrap();
        let vid = version.id.unwrap();
        let task = repo
            .create_task(pid, vid, "A", "", Status::Done, Priority::High)
            .unwrap();
        repo.add_task_note(task.id.unwrap(), "note").unwrap();
        let snapshot = repo.build_compact_snapshot(vid).unwrap();
        let prose = (1..=20)
            .map(|i| format!("Summary line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let summary = repo.create_version_summary(vid, &prose, &snapshot).unwrap();
        let summary_id = summary.id.unwrap();

        let mut screen = VersionSummaryScreen::new(repo.clone());
        screen.set_ids(pid, vid, summary_id);
        screen.init().unwrap();
        for _ in 0..4 {
            screen.handle_event(&key_event(KeyCode::Char('j')));
        }
        insta::assert_snapshot!(render_to_string(&screen, 60, 20));
    }

    #[test]
    fn keybindings_match_spec() {
        let keys: Vec<(KeyCode, &str)> = VersionSummaryScreen::KEYBINDINGS
            .iter()
            .map(|(event, label)| (event.code, *label))
            .collect();
        assert_eq!(
            keys,
            vec![
                (KeyCode::Char('j'), "Down"),
                (KeyCode::Char('k'), "Up"),
                (KeyCode::Esc, "Back"),
                (KeyCode::Char('q'), "Quit"),
            ]
        );
    }
}
