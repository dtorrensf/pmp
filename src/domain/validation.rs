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

/// Normalizes the optional `working_dir` of a project.
///
/// A blank value clears it. A non-blank value must be an absolute path and is
/// normalized lexically: `.` segments and repeated separators disappear and
/// `..` is resolved against the previous segment. Nothing is checked against
/// the filesystem, so a project can point at a checkout that is cloned later;
/// flows that actually operate on disk (ACP) require the directory to exist.
pub fn normalize_working_dir(value: Option<&str>) -> Result<Option<String>, AppError> {
    use std::path::{Component, Path, PathBuf};

    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.contains('\0') {
        return Err(AppError::Validation(
            "working directory cannot contain NUL bytes".to_string(),
        ));
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "working directory must be an absolute path".to_string(),
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(AppError::Validation(
                        "working directory escapes the filesystem root".to_string(),
                    ));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
        .to_str()
        .map(|path| Some(path.to_string()))
        .ok_or_else(|| AppError::Validation("working directory is not valid UTF-8".to_string()))
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
    fn normalizes_working_directories() {
        assert_eq!(normalize_working_dir(None).unwrap(), None);
        assert_eq!(normalize_working_dir(Some("   ")).unwrap(), None);
        assert_eq!(
            normalize_working_dir(Some("  /home/me/repo/  ")).unwrap(),
            Some("/home/me/repo".to_string())
        );
        assert_eq!(
            normalize_working_dir(Some("/home//me/./repo/")).unwrap(),
            Some("/home/me/repo".to_string())
        );
        assert_eq!(
            normalize_working_dir(Some("/home/me/repo/../other")).unwrap(),
            Some("/home/me/other".to_string())
        );
    }

    #[test]
    fn rejects_invalid_working_directories() {
        for invalid in ["repo", "./repo", "~/repo"] {
            let error = normalize_working_dir(Some(invalid)).unwrap_err();
            assert!(
                matches!(&error, AppError::Validation(message) if message.contains("absolute")),
                "{invalid}: {error:?}"
            );
        }
        let error = normalize_working_dir(Some("/..")).unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("escapes")),
            "{error:?}"
        );
        let error = normalize_working_dir(Some("/tmp/with\0nul")).unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("NUL")),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_too_many_dependencies() {
        assert!(validate_dependency_count(MAX_DEPENDENCIES).is_ok());
        assert!(validate_dependency_count(MAX_DEPENDENCIES + 1).is_err());
    }
}
