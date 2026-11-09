// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Input rules shared by every entry point (TUI, MCP, repositories).

use crate::domain::error::AppError;

/// Maximum size of a name, in bytes. Names are short identifiers shown in the
/// TUI and in every MCP response.
pub const MAX_NAME_LEN: usize = 200;

/// Maximum size of a free-text field (description, note, summary), in bytes.
/// Large enough for detailed notes, small enough that a single MCP call cannot
/// exhaust the process memory or the client context window.
pub const MAX_TEXT_LEN: usize = 100_000;

/// Maximum number of dependencies accepted in one call.
pub const MAX_DEPENDENCIES: usize = 500;

/// Rejects a blank `name` for the record kind `entity`, e.g. `"project"`
/// yields `project name cannot be empty`. Also bounds its size.
pub fn validate_name(entity: &str, name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(format!(
            "{entity} name cannot be empty"
        )));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(AppError::Validation(format!(
            "{entity} name cannot exceed {MAX_NAME_LEN} bytes"
        )));
    }
    Ok(())
}

/// Rejects a free-text `value` larger than [`MAX_TEXT_LEN`].
pub fn validate_text(entity: &str, field: &str, value: &str) -> Result<(), AppError> {
    if value.len() > MAX_TEXT_LEN {
        return Err(AppError::Validation(format!(
            "{entity} {field} cannot exceed {MAX_TEXT_LEN} bytes"
        )));
    }
    Ok(())
}

/// Rejects a dependency list larger than [`MAX_DEPENDENCIES`].
pub fn validate_dependency_count(count: usize) -> Result<(), AppError> {
    if count > MAX_DEPENDENCIES {
        return Err(AppError::Validation(format!(
            "a task cannot have more than {MAX_DEPENDENCIES} dependencies"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_names_with_entity_in_message() {
        for blank in ["", "   "] {
            match validate_name("version", blank) {
                Err(AppError::Validation(message)) => {
                    assert_eq!(message, "version name cannot be empty");
                }
                other => panic!("expected validation error, got {other:?}"),
            }
        }
        assert!(validate_name("task", "Write docs").is_ok());
    }

    #[test]
    fn rejects_names_longer_than_the_limit() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        let err = validate_name("project", &long).unwrap_err();
        assert!(matches!(err, AppError::Validation(message) if message.contains("cannot exceed")));
        assert!(validate_name("project", &"a".repeat(MAX_NAME_LEN)).is_ok());
    }

    #[test]
    fn rejects_text_longer_than_the_limit() {
        let long = "a".repeat(MAX_TEXT_LEN + 1);
        let err = validate_text("task", "description", &long).unwrap_err();
        assert!(matches!(err, AppError::Validation(message) if message.contains("cannot exceed")));
        assert!(validate_text("task", "description", &"a".repeat(MAX_TEXT_LEN)).is_ok());
    }

    #[test]
    fn rejects_too_many_dependencies() {
        assert!(validate_dependency_count(MAX_DEPENDENCIES).is_ok());
        assert!(validate_dependency_count(MAX_DEPENDENCIES + 1).is_err());
    }
}
