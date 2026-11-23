// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Modal states of the task list.
//!
//! Each variant owns what its popup needs, so the key controllers and the
//! renderers never peek at fields of another mode.

use crate::domain::task::{Priority, Status};
use crate::domain::version::Version;
use crate::screens::components::confirm_dialog::ConfirmDialog;

use super::state::Filters;

#[derive(Debug, Clone)]
pub enum TaskListMode {
    Browsing,
    ConfirmDelete(ConfirmDialog),
    Filtering(FilterState),
    MovingTask(MoveTaskState),
}

/// State of the "move task to version" popup: the task being moved, the other
/// versions of its project and the highlighted target.
#[derive(Debug, Clone)]
pub struct MoveTaskState {
    pub task_id: i64,
    pub targets: Vec<Version>,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Title,
    Status,
    Priority,
}

/// Draft filter being edited in the popup. It only becomes the list's real
/// filter when the user confirms with Enter.
#[derive(Debug, Clone)]
pub struct FilterState {
    pub filter_title: String,
    pub filter_status: Option<Status>,
    pub filter_priority: Option<Priority>,
    pub selected_field: FilterField,
}

impl FilterState {
    /// Draft pre-filled with the filters currently applied.
    pub fn from_filters(filters: &Filters) -> Self {
        Self {
            filter_title: filters.title.clone().unwrap_or_default(),
            filter_status: filters.status,
            filter_priority: filters.priority,
            selected_field: FilterField::Status,
        }
    }

    /// The filters to apply when the popup is confirmed. A blank title or one
    /// made only of spaces removes the title filter.
    pub fn to_filters(&self) -> Filters {
        Filters {
            title: (!self.filter_title.trim().is_empty())
                .then(|| self.filter_title.trim().to_lowercase()),
            status: self.filter_status,
            priority: self.filter_priority,
        }
    }
}
