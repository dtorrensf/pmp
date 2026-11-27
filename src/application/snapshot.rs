// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portable whole-store snapshot use cases (A8).
//!
//! Export produces a self-contained, versioned JSON document with every project,
//! version, task, note, dependency and frozen summary; import validates it and
//! writes it in one transaction. Both channels (today `pmp snapshot`, later the
//! MCP tool or the 1.5.0 core service) call these functions, so the format and
//! the safety rules only exist once.

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::snapshot::{DataSnapshot, SnapshotCounts};

/// Reads the whole store as a snapshot.
pub fn export(repo: &dyn Repository) -> Result<DataSnapshot, AppError> {
    repo.export_snapshot()
}

/// Serializes a snapshot as the pretty-printed JSON written to disk.
pub fn render(snapshot: &DataSnapshot) -> Result<String, AppError> {
    let mut json = serde_json::to_string_pretty(snapshot)
        .map_err(|e| AppError::Validation(format!("cannot serialize the snapshot: {e}")))?;
    json.push('\n');
    Ok(json)
}

/// Parses and validates a snapshot document. A file written by a newer build
/// (higher `schema_version`) is rejected instead of silently losing fields.
pub fn parse(json: &str) -> Result<DataSnapshot, AppError> {
    let snapshot: DataSnapshot = serde_json::from_str(json)
        .map_err(|e| AppError::Validation(format!("invalid snapshot file: {e}")))?;
    snapshot.validate()?;
    Ok(snapshot)
}

/// Parses `json` and imports it. Without `replace`, a store that already holds
/// projects is left untouched and the call fails with a validation error.
pub fn import(
    repo: &dyn Repository,
    json: &str,
    replace: bool,
) -> Result<SnapshotCounts, AppError> {
    let snapshot = parse(json)?;
    repo.import_snapshot(&snapshot, replace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::{
        NoteRepository, ProjectRepository, RepositoryPolicy, SnapshotRepository, TaskRepository,
    };
    use crate::db::stub::StubRepository;
    use crate::domain::snapshot::{SNAPSHOT_SCHEMA, SNAPSHOT_SCHEMA_VERSION};
    use crate::domain::task::{Priority, Status};

    fn seeded() -> StubRepository {
        let repo = StubRepository::default();
        let project = repo.create_project("P", "d", None).unwrap();
        let pid = project.id.unwrap();
        let version = repo.default_version_id(pid).unwrap();
        let task = repo
            .create_task(pid, version, "A", "", Status::Done, Priority::High)
            .unwrap();
        repo.add_task_note(task.id.unwrap(), "note").unwrap();
        repo
    }

    #[test]
    fn export_renders_a_versioned_document() {
        let repo = seeded();
        let json = render(&export(&repo).unwrap()).unwrap();
        assert!(json.contains(&format!("\"schema\": \"{SNAPSHOT_SCHEMA}\"")));
        assert!(json.contains(&format!("\"schema_version\": {SNAPSHOT_SCHEMA_VERSION}")));
        assert!(json.ends_with('\n'));
    }

    #[test]
    fn a_document_with_an_unknown_version_is_rejected() {
        let repo = seeded();
        let mut snapshot = export(&repo).unwrap();
        snapshot.schema_version = SNAPSHOT_SCHEMA_VERSION + 1;
        let json = serde_json::to_string(&snapshot).unwrap();
        let error = parse(&json).unwrap_err();
        assert!(
            matches!(error, AppError::Validation(message) if message.contains("schema version"))
        );
    }

    #[test]
    fn importing_without_replace_keeps_the_existing_store() {
        let repo = seeded();
        let snapshot = export(&repo).unwrap();
        let error = repo.import_snapshot(&snapshot, false).unwrap_err();
        assert!(matches!(error, AppError::Validation(message) if message.contains("replace")));
        assert_eq!(repo.list_projects().unwrap().len(), 1);
    }

    #[test]
    fn import_round_trips_through_json() {
        let source = seeded();
        let json = render(&export(&source).unwrap()).unwrap();
        let target = StubRepository::default();
        let counts = import(&target, &json, false).unwrap();
        assert_eq!(counts, source.export_snapshot().unwrap().counts());
        let notes = target.list_task_notes(1).unwrap();
        assert_eq!(notes[0].content, "note");
    }
}
