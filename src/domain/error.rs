// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("db error: {0}")]
    DbError(#[from] rusqlite::Error),
    #[error("dependency cycle")]
    DependencyCycle,
    #[error("validation: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rusqlite_error_converts_to_db_error() {
        let sqlite_err = rusqlite::Error::InvalidQuery;
        let err: AppError = sqlite_err.into();
        assert!(matches!(err, AppError::DbError(_)));
    }

    #[test]
    fn validation_error_stores_message() {
        let err = AppError::Validation("bad input".to_string());
        assert!(matches!(err, AppError::Validation(ref s) if s == "bad input"));
    }

    #[test]
    fn not_found_and_dependency_cycle_variants_exist() {
        assert!(matches!(AppError::NotFound, AppError::NotFound));
        assert!(matches!(
            AppError::DependencyCycle,
            AppError::DependencyCycle
        ));
    }
}
