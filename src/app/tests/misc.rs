// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::repository::{ProjectRepository, RepositoryPolicy};
use crate::db::stub::StubRepository;
use crate::screens::{Action, Screen};
use crossterm::event::{KeyCode, KeyModifiers};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use super::*;

#[test]
fn app_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<App>();
}

#[test]
fn update_is_hidden_by_default() {
    let app = App::new(Arc::new(StubRepository::default()));
    assert!(!app.has_update());
    assert!(!app.is_update_banner_open());
}

#[test]
fn poll_update_copies_slot_value() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    let slot = Arc::new(OnceLock::new());
    app.set_pending_update((slot.clone(), Arc::new(AtomicBool::new(false))));
    app.poll_update();
    assert!(!app.has_update());
    slot.set("0.4.0".to_string()).unwrap();
    app.poll_update();
    assert_eq!(app.update_version(), Some("0.4.0"));
}

#[test]
fn open_update_banner_requires_an_update() {
    let mut app = App::new(Arc::new(StubRepository::default()));
    app.open_update_banner();
    assert!(!app.is_update_banner_open());
    app.set_update_available("0.4.0".to_string());
    app.open_update_banner();
    assert!(app.is_update_banner_open());
    app.close_update_banner();
    assert!(!app.is_update_banner_open());
}

#[test]
fn question_mark_opens_help_only_outside_text_forms() {
    let repo = Arc::new(StubRepository::default());
    let project_id = repo.create_project("P", "").unwrap().id.unwrap();
    let mut app = App::new(repo);
    assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::SHIFT));
    assert!(app.is_help_open());
    // Esc closes it; `?` closes it too.
    assert!(app.handle_help_key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.is_help_open());
    assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert!(!app.is_help_open());
    app.dispatch(Action::Navigate(Screen::ProjectForm { project_id: None }));
    assert!(!app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert!(!app.is_help_open());
    app.dispatch(Action::Navigate(Screen::ProjectDashboard { project_id }));
    assert!(app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE));
}

#[test]
fn help_swallows_keys_with_ctrl_modifiers_too() {
    let repo = Arc::new(StubRepository::default());
    let mut app = App::new(repo);
    app.handle_help_key(KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(app.handle_help_key(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(app.is_help_open());
}

#[test]
fn accepts_text_input_only_on_forms() {
    let repo = Arc::new(StubRepository::default());
    let project_id = repo.create_project("P", "").unwrap().id.unwrap();
    let mut app = App::new(repo);
    assert!(!app.screen.accepts_text_input());
    app.dispatch(Action::Navigate(Screen::ProjectForm { project_id: None }));
    assert!(app.screen.accepts_text_input());
    app.dispatch(Action::Navigate(Screen::TaskForm {
        version_id: app.repo.default_version_id(project_id).unwrap(),
        task_id: None,
    }));
    assert!(app.screen.accepts_text_input());
}
