// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::screens::Action;

/// Result of a use case that succeeded: an optional confirmation for the user
/// and the follow-up action the event loop should dispatch next.
///
/// Use cases return this instead of touching the toast queue, so the
/// presentation decision stays in `App`.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Outcome {
    pub(super) confirmation: Option<String>,
    pub(super) next: Option<Action>,
}

impl Outcome {
    /// Success that shows `confirmation` and asks for `next`.
    pub(super) fn confirmed(confirmation: impl Into<String>, next: Option<Action>) -> Self {
        Self {
            confirmation: Some(confirmation.into()),
            next,
        }
    }
}
