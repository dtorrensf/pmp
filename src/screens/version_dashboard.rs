// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Version screen showing the tasks of one version. Skeleton: the real view
//! lands in #93 (and the task list is adapted in #94).
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{Frame, layout::Rect, widgets::Clear};

use crate::db::repository::Repository;
use crate::screens::components::keys::key;
use crate::screens::{Action, Component, Screen};

pub struct VersionDashboard {
    #[allow(dead_code)]
    repo: Arc<dyn Repository>,
    project_id: i64,
    version_id: i64,
}

impl VersionDashboard {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[(key(KeyCode::Esc), "Back")];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            version_id: 0,
        }
    }

    pub fn set_ids(&mut self, project_id: i64, version_id: i64) {
        self.project_id = project_id;
        self.version_id = version_id;
    }

    pub fn project_id(&self) -> i64 {
        self.project_id
    }

    pub fn version_id(&self) -> i64 {
        self.version_id
    }
}

impl Component for VersionDashboard {
    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Esc, ..
            }) => Some(Action::Navigate(Screen::ProjectDashboard {
                project_id: self.project_id,
            })),
            _ => None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
    }
}
