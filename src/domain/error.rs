// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error taxonomy shared by the TUI, the MCP server and, from 1.0.0, the ACP
//! client.
//!
//! Every failure carries a stable [`ErrorCode`] so a client can react
//! programmatically instead of matching the message text. Codes never change
//! meaning; new failures get a new code.
//!
//! | code | retryable | blockers | meaning |
//! |---|---|---|---|
//! | `not_found` | no | no | The record does not exist (or is not visible). |
//! | `validation` | no | no | Input rejected: blank name, oversized text, unknown relationship. |
//! | `duplicate_name` | no | no | A record with that name already exists here. |
//! | `dependency_blocked` | no | yes | The task has incomplete dependencies; `blockers` lists them. |
//! | `dependency_cycle` | no | no | The dependency edge would close a cycle. |
//! | `confirmation_required` | no | yes | A destructive change would remove active work or dependents; `blockers` lists them. Retry with explicit confirmation. |
//! | `invalid_transition` | no | no | The state change is not allowed (a task with edges cannot move version). |
//! | `no_ready_task` | no | no | No todo task without incomplete dependencies matches the request (M3 `claim_next_task`). |
//! | `db_busy` | yes | no | The store is locked; retrying later may succeed. |
//! | `agent_unavailable` | yes | no | The configured agent binary could not be started. |
//! | `agent_failed` | no | no | The agent started but the session failed. |
//! | `internal` | no | no | Unexpected failure. Its details are logged, never sent to clients. |
//!
//! [`AppError::user_message`] is the text meant for the user: it never includes
//! SQLite or filesystem details. Use [`AppError::info`] to carry the code, the
//! message, `retryable` and `blockers` over a wire.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable, serializable identifier of an [`AppError`] kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Validation,
    DuplicateName,
    DependencyBlocked,
    DependencyCycle,
    ConfirmationRequired,
    InvalidTransition,
    NoReadyTask,
    DbBusy,
    AgentUnavailable,
    AgentFailed,
    Internal,
}

impl ErrorCode {
    /// The wire form of the code, as it appears in `serde` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Validation => "validation",
            Self::DuplicateName => "duplicate_name",
            Self::DependencyBlocked => "dependency_blocked",
            Self::DependencyCycle => "dependency_cycle",
            Self::ConfirmationRequired => "confirmation_required",
            Self::InvalidTransition => "invalid_transition",
            Self::NoReadyTask => "no_ready_task",
            Self::DbBusy => "db_busy",
            Self::AgentUnavailable => "agent_unavailable",
            Self::AgentFailed => "agent_failed",
            Self::Internal => "internal",
        }
    }

    /// Whether retrying the same operation later may succeed.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::DbBusy | Self::AgentUnavailable)
    }
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    DuplicateName(String),
    #[error("{message}")]
    DependencyBlocked { message: String, blockers: Vec<i64> },
    #[error("dependency cycle")]
    DependencyCycle,
    #[error("{message}")]
    ConfirmationRequired { message: String, blockers: Vec<i64> },
    #[error("{0}")]
    InvalidTransition(String),
    #[error("{0}")]
    NoReadyTask(String),
    #[error("database is busy; retry the operation")]
    DbBusy,
    #[error("{0}")]
    AgentUnavailable(String),
    #[error("{0}")]
    AgentFailed(String),
    #[error("db error: {0}")]
    DbError(#[source] rusqlite::Error),
    #[error("internal error")]
    Internal,
}

/// Wire form of a failure: what MCP sends as error data and what the 1.5.0
/// core service can return as a JSON error body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<i64>,
}

impl AppError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound => ErrorCode::NotFound,
            Self::Validation(_) => ErrorCode::Validation,
            Self::DuplicateName(_) => ErrorCode::DuplicateName,
            Self::DependencyBlocked { .. } => ErrorCode::DependencyBlocked,
            Self::DependencyCycle => ErrorCode::DependencyCycle,
            Self::ConfirmationRequired { .. } => ErrorCode::ConfirmationRequired,
            Self::InvalidTransition(_) => ErrorCode::InvalidTransition,
            Self::NoReadyTask(_) => ErrorCode::NoReadyTask,
            Self::DbBusy => ErrorCode::DbBusy,
            Self::AgentUnavailable(_) => ErrorCode::AgentUnavailable,
            Self::AgentFailed(_) => ErrorCode::AgentFailed,
            Self::DbError(_) | Self::Internal => ErrorCode::Internal,
        }
    }

    /// Whether retrying the same operation later may succeed.
    pub fn is_retryable(&self) -> bool {
        self.code().is_retryable()
    }

    /// Ids of the tasks that block this one. Empty for every other error.
    pub fn blockers(&self) -> &[i64] {
        match self {
            Self::DependencyBlocked { blockers, .. }
            | Self::ConfirmationRequired { blockers, .. } => blockers,
            _ => &[],
        }
    }

    /// Text for the user. Storage and filesystem details are logged instead of
    /// shown: they are not actionable and may expose local paths or SQL.
    pub fn user_message(&self) -> String {
        match self {
            Self::DbError(source) => {
                tracing::error!(error = %source, "database error");
                "internal error".to_string()
            }
            Self::Internal => {
                tracing::error!("internal error");
                "internal error".to_string()
            }
            other => other.to_string(),
        }
    }

    /// Whether a SQLite failure is transient lock contention that a later retry
    /// may overcome. Kept next to the conversion so both stay in sync.
    pub fn is_lock(err: &rusqlite::Error) -> bool {
        matches!(
            err,
            rusqlite::Error::SqliteFailure(failure, _)
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }

    /// The complete, serializable form of the failure.
    pub fn info(&self) -> ErrorInfo {
        ErrorInfo {
            code: self.code(),
            message: self.user_message(),
            retryable: self.is_retryable(),
            blockers: self.blockers().to_vec(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        if Self::is_lock(&err) {
            Self::DbBusy
        } else {
            Self::DbError(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorCode, ErrorInfo};

    fn blocked() -> AppError {
        AppError::DependencyBlocked {
            message: "cannot mark task as done: it has incomplete dependencies".to_string(),
            blockers: vec![4, 7],
        }
    }

    #[test]
    fn every_variant_maps_to_its_stable_code() {
        let cases: Vec<(AppError, ErrorCode)> = vec![
            (AppError::NotFound, ErrorCode::NotFound),
            (AppError::Validation("x".into()), ErrorCode::Validation),
            (
                AppError::DuplicateName("x".into()),
                ErrorCode::DuplicateName,
            ),
            (blocked(), ErrorCode::DependencyBlocked),
            (AppError::DependencyCycle, ErrorCode::DependencyCycle),
            (
                AppError::InvalidTransition("x".into()),
                ErrorCode::InvalidTransition,
            ),
            (
                AppError::ConfirmationRequired {
                    message: "x".into(),
                    blockers: vec![],
                },
                ErrorCode::ConfirmationRequired,
            ),
            (AppError::DbBusy, ErrorCode::DbBusy),
            (
                AppError::AgentUnavailable("x".into()),
                ErrorCode::AgentUnavailable,
            ),
            (AppError::AgentFailed("x".into()), ErrorCode::AgentFailed),
            (AppError::NoReadyTask("x".into()), ErrorCode::NoReadyTask),
            (AppError::Internal, ErrorCode::Internal),
            (
                AppError::DbError(rusqlite::Error::InvalidQuery),
                ErrorCode::Internal,
            ),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error:?}");
            assert_eq!(
                error.code().as_str(),
                serde_json::to_value(code).unwrap().as_str().unwrap()
            );
        }
    }

    #[test]
    fn only_busy_store_and_missing_agent_are_retryable() {
        assert!(AppError::DbBusy.is_retryable());
        assert!(AppError::AgentUnavailable("x".into()).is_retryable());
        for error in [
            AppError::NotFound,
            AppError::Validation("x".into()),
            AppError::DuplicateName("x".into()),
            blocked(),
            AppError::DependencyCycle,
            AppError::InvalidTransition("x".into()),
            AppError::NoReadyTask("x".into()),
            AppError::AgentFailed("x".into()),
            AppError::Internal,
        ] {
            assert!(!error.is_retryable(), "{error:?}");
        }
    }

    #[test]
    fn blocked_errors_carry_the_blocker_ids() {
        assert_eq!(blocked().blockers(), [4, 7]);
        assert!(AppError::Validation("x".into()).blockers().is_empty());
    }

    #[test]
    fn storage_details_never_reach_the_user_message() {
        let error = AppError::DbError(rusqlite::Error::InvalidQuery);
        assert_eq!(error.user_message(), "internal error");
        assert_eq!(AppError::Internal.user_message(), "internal error");
    }

    #[test]
    fn validation_messages_reach_the_user_verbatim() {
        assert_eq!(
            AppError::Validation("project name cannot be empty".into()).user_message(),
            "project name cannot be empty"
        );
        assert_eq!(
            blocked().user_message(),
            "cannot mark task as done: it has incomplete dependencies"
        );
    }

    #[test]
    fn busy_store_errors_are_detected_on_conversion() {
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        );
        assert!(matches!(AppError::from(busy), AppError::DbBusy));
        assert!(matches!(
            AppError::from(rusqlite::Error::InvalidQuery),
            AppError::DbError(_)
        ));
    }

    #[test]
    fn info_serializes_code_retryable_and_blockers() {
        let info = blocked().info();
        assert_eq!(
            info,
            ErrorInfo {
                code: ErrorCode::DependencyBlocked,
                message: "cannot mark task as done: it has incomplete dependencies".to_string(),
                retryable: false,
                blockers: vec![4, 7],
            }
        );
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["code"], "dependency_blocked");
        assert_eq!(json["retryable"], false);
        assert_eq!(json["blockers"], serde_json::json!([4, 7]));

        let quiet = AppError::NotFound.info();
        let json = serde_json::to_value(&quiet).unwrap();
        assert_eq!(json["code"], "not_found");
        assert!(json.get("blockers").is_none(), "empty blockers are omitted");
    }
}
