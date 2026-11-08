// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Input rules shared by every entry point (TUI, MCP, repositories).

use crate::domain::error::AppError;

/// Rejects a blank `name` for the record kind `entity`, e.g. `"project"`
/// yields `project name cannot be empty`.
pub fn validate_name(entity: &str, name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(format!(
            "{entity} name cannot be empty"
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
}
