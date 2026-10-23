// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Create/edit form for versions. Skeleton: the full form lands in #90; for now
//! it only wires the screen to the app so navigation and dispatch work.
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{Frame, layout::Rect, widgets::Clear};

use crate::db::repository::Repository;
use crate::screens::components::keys::key;
use crate::screens::{Action, Component, Screen};

pub struct VersionForm {
    #[allow(dead_code)]
    repo: Arc<dyn Repository>,
    project_id: i64,
    version_id: Option<i64>,
    return_to: Screen,
}

impl VersionForm {
    pub const KEYBINDINGS: &[(KeyEvent, &'static str)] = &[(key(KeyCode::Esc), "Cancel")];

    pub fn new(repo: Arc<dyn Repository>) -> Self {
        Self {
            repo,
            project_id: 0,
            version_id: None,
            return_to: Screen::ProjectList,
        }
    }

    pub fn set_project_id(&mut self, project_id: i64) {
        self.project_id = project_id;
    }

    pub fn set_version_id(&mut self, version_id: Option<i64>) {
        self.version_id = version_id;
    }

    pub fn set_return_to(&mut self, screen: &Screen) {
        self.return_to = screen.clone();
    }

    pub fn return_to(&self) -> &Screen {
        &self.return_to
    }

    pub fn project_id(&self) -> i64 {
        self.project_id
    }

    pub fn version_id(&self) -> Option<i64> {
        self.version_id
    }
}

impl Component for VersionForm {
    fn keybindings(&self) -> &'static [(KeyEvent, &'static str)] {
        Self::KEYBINDINGS
    }

    fn handle_event(&mut self, event: &Event) -> Option<Action> {
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Esc, ..
            }) => Some(Action::Navigate(self.return_to.clone())),
            _ => None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
    }
}
