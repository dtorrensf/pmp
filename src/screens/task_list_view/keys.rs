// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keyboard controllers of the task list, one per mode.
//!
//! They translate a key press into either a state change or an intent for the
//! embedding screen. The browsing controller does not touch the repository:
//! deleting needs the dependent count only to build the dialog text, so it
//! reports [`BrowsingOutcome::DeleteRequested`] and the facade (the screen
//! flow) performs that query.

use crossterm::event::KeyCode;

use crate::domain::task::{Priority, Status};
use crate::screens::components::confirm_dialog::ConfirmAnswer;
use crate::screens::{Action, Screen};

use super::modes::{FilterField, TaskListMode};
use super::state::{Selection, TaskListState};

/// What a key pressed while browsing the table produced.
pub(super) enum BrowsingOutcome {
    /// The key belongs to the embedding screen.
    Ignored,
    /// The view consumed the key and nothing else happens.
    Handled,
    /// The view wants the embedding screen to dispatch this action.
    Action(Action),
    /// The user asked to delete a task; the facade builds the confirmation.
    DeleteRequested { task_id: i64, name: String },
    /// The user opened the filter popup.
    OpenFilter,
}

/// Keys while browsing the table: move the selection, open, change status,
/// delete, filter, or change the sort order.
pub(super) fn browsing(state: &mut TaskListState, code: KeyCode) -> BrowsingOutcome {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_selection(Selection::Up);
            BrowsingOutcome::Handled
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_selection(Selection::Down);
            BrowsingOutcome::Handled
        }
        KeyCode::Char('o') => {
            state.cycle_sort();
            BrowsingOutcome::Handled
        }
        KeyCode::Char('x') => {
            state.clear_filters();
            BrowsingOutcome::Handled
        }
        KeyCode::Enter => match state.selected_task() {
            Some(task) => BrowsingOutcome::Action(Action::Navigate(Screen::TaskForm {
                version_id: task.version_id,
                task_id: task.id,
            })),
            None => BrowsingOutcome::Ignored,
        },
        KeyCode::Tab => match state.selected_task() {
            Some(task) => BrowsingOutcome::Action(Action::UpdateTaskStatus {
                task_id: task.id.unwrap_or(0),
                new_status: task.status.next(),
            }),
            None => BrowsingOutcome::Ignored,
        },
        KeyCode::Char('d') => match state.selected_task() {
            Some(task) => BrowsingOutcome::DeleteRequested {
                task_id: task.id.unwrap_or(0),
                name: task.name.clone(),
            },
            None => BrowsingOutcome::Ignored,
        },
        KeyCode::Char('f') => BrowsingOutcome::OpenFilter,
        _ => BrowsingOutcome::Ignored,
    }
}

/// Keys in the delete confirmation: `y` confirms, `n`/`Esc` cancel and any
/// other key keeps the dialog open.
pub(super) fn confirm_delete(mode: &mut TaskListMode, code: KeyCode) -> Option<Action> {
    let answer = match mode {
        TaskListMode::ConfirmDelete(dialog) => dialog.handle_key(code),
        _ => return None,
    };
    match answer {
        ConfirmAnswer::Confirmed(action) => {
            *mode = TaskListMode::Browsing;
            Some(action)
        }
        ConfirmAnswer::Cancelled => {
            *mode = TaskListMode::Browsing;
            None
        }
        ConfirmAnswer::Ignored => None,
    }
}

/// Keys in the move-to-version popup.
pub(super) fn moving(mode: &mut TaskListMode, code: KeyCode) -> Option<Action> {
    let TaskListMode::MovingTask(state) = mode else {
        return None;
    };
    let action = match code {
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
            crate::screens::components::list_navigation::move_selection(
                &mut state.selected,
                state.targets.len(),
                code,
            );
            return None;
        }
        KeyCode::Enter => state.targets.get(state.selected).and_then(|target| {
            target.id.map(|version_id| Action::MoveTask {
                task_id: state.task_id,
                version_id,
            })
        }),
        KeyCode::Esc => None,
        _ => return None,
    };
    // Without targets (only one version) the popup just closes.
    *mode = TaskListMode::Browsing;
    action
}

/// Keys in the filter popup: edit the title, cycle status and priority, then
/// apply or cancel.
pub(super) fn filtering(
    state: &mut TaskListState,
    mode: &mut TaskListMode,
    code: KeyCode,
) -> Option<Action> {
    let mut close = false;
    {
        let TaskListMode::Filtering(draft) = mode else {
            return None;
        };
        match code {
            KeyCode::Esc => close = true,
            KeyCode::Enter => {
                state.set_filters(draft.to_filters());
                close = true;
            }
            KeyCode::Tab => {
                draft.selected_field = match draft.selected_field {
                    FilterField::Title => FilterField::Status,
                    FilterField::Status => FilterField::Priority,
                    FilterField::Priority => FilterField::Title,
                };
            }
            KeyCode::Char(character) if draft.selected_field == FilterField::Title => {
                draft.filter_title.push(character);
            }
            KeyCode::Backspace if draft.selected_field == FilterField::Title => {
                draft.filter_title.pop();
            }
            KeyCode::Left => match draft.selected_field {
                FilterField::Title => {}
                FilterField::Status => {
                    draft.filter_status = draft
                        .filter_status
                        .map(Status::cycle_previous)
                        .unwrap_or(Some(Status::Done));
                }
                FilterField::Priority => {
                    draft.filter_priority = draft
                        .filter_priority
                        .map(Priority::cycle_previous)
                        .unwrap_or(Some(Priority::Low));
                }
            },
            KeyCode::Right => match draft.selected_field {
                FilterField::Title => {}
                FilterField::Status => {
                    draft.filter_status = draft
                        .filter_status
                        .map(Status::cycle_next)
                        .unwrap_or(Some(Status::Todo));
                }
                FilterField::Priority => {
                    draft.filter_priority = draft
                        .filter_priority
                        .map(Priority::cycle_next)
                        .unwrap_or(Some(Priority::High));
                }
            },
            _ => {}
        }
    }
    if close {
        *mode = TaskListMode::Browsing;
    }
    None
}
