// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Output schemas for tools whose success value is not a domain type.
//!
//! The types are only used to declare `outputSchema`; the handlers keep
//! returning the same JSON values, so the textual contract does not change.

use schemars::JsonSchema;
use serde::Serialize;

/// Root object for the tools that return a bare array in their textual JSON.
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct Items<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct TaskDependencies {
    pub task_id: i64,
    pub depends_on_ids: Vec<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct DeletedNote {
    pub note_id: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct DeletedSummary {
    pub summary_id: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct ExportedSummary {
    pub path: String,
}
