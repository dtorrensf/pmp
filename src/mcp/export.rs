// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Markdown export of a version summary, shared by the MCP tool and the TUI.
//!
//! The generated document is self-contained: prose, Mermaid pie charts for
//! status and priority, a dependency flowchart (when small enough) and a task
//! table plus per-task notes, all derived from the frozen `CompactSnapshot`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::task::{Priority, Status};
use crate::domain::version_summary::VersionSummary;

const MAX_FLOWCHART_TASKS: usize = 20;

/// Maximum length of the version-name portion of an export file name.
const MAX_EXPORT_STEM_LEN: usize = 80;

/// Writes the Markdown export for `summary_id` to the data dir exports folder
/// and returns the path written. The destination is always chosen by the
/// server: clients cannot ask for an arbitrary path.
pub fn export_version_summary_markdown(
    repo: &dyn Repository,
    summary_id: i64,
) -> Result<PathBuf, AppError> {
    let base = exports_dir()?;
    export_version_summary_markdown_to(repo, summary_id, &base)
}

fn exports_dir() -> Result<PathBuf, AppError> {
    Ok(dirs::data_dir()
        .ok_or_else(|| AppError::Validation("no data directory available".to_string()))?
        .join("pmp")
        .join("exports"))
}

/// Shared by tests: writes the export under `exports_dir` instead of the data
/// dir, applying the same sanitization and confinement checks.
pub(crate) fn export_version_summary_markdown_to(
    repo: &dyn Repository,
    summary_id: i64,
    exports_dir: &Path,
) -> Result<PathBuf, AppError> {
    let summary = repo.get_version_summary(summary_id)?;
    let snapshot: CompactSnapshot = serde_json::from_str(&summary.snapshot_json)
        .map_err(|e| AppError::Validation(format!("snapshot parse error: {e}")))?;
    let version = repo.get_version(summary.version_id)?;
    let project = repo.get_project(version.project_id)?;

    let markdown = render_markdown(&project.name, &version.name, &summary, &snapshot);
    std::fs::create_dir_all(exports_dir)
        .map_err(|e| AppError::Validation(format!("cannot create export directory: {e}")))?;
    let base = exports_dir
        .canonicalize()
        .map_err(|e| AppError::Validation(format!("cannot resolve export directory: {e}")))?;
    let target = export_path(&base, &version.name, summary_id);
    ensure_within_dir(&base, &target)?;
    std::fs::write(&target, markdown)
        .map_err(|e| AppError::Validation(format!("cannot write export file: {e}")))?;
    Ok(target)
}

/// Builds a file name inside `base` from a sanitized version name. The result
/// never contains separators or `..`, because all non-alphanumeric characters
/// are collapsed to `-`.
fn export_path(base: &Path, version_name: &str, summary_id: i64) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = sanitize_file_component(version_name);
    base.join(format!("{stem}-{summary_id}-{timestamp}.md"))
}

/// Turns an arbitrary version name into a safe single path component.
fn sanitize_file_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(MAX_EXPORT_STEM_LEN));
    let mut last_separator = false;
    for ch in value.chars() {
        if out.len() >= MAX_EXPORT_STEM_LEN {
            break;
        }
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if last_separator {
                continue;
            }
            last_separator = true;
        } else {
            last_separator = false;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '_');
    if trimmed.is_empty() {
        "version".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Rejects `target` when its parent resolves outside the canonical `base`.
fn ensure_within_dir(base: &Path, target: &Path) -> Result<(), AppError> {
    let parent = target.parent().unwrap_or(base);
    let parent = parent
        .canonicalize()
        .map_err(|e| AppError::Validation(format!("cannot resolve export path: {e}")))?;
    if !parent.starts_with(base) {
        return Err(AppError::Validation(
            "export path escapes the exports directory".to_string(),
        ));
    }
    Ok(())
}

fn render_markdown(
    project_name: &str,
    version_name: &str,
    summary: &VersionSummary,
    snapshot: &CompactSnapshot,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {project_name} — {version_name}\n\n"));
    out.push_str(&format!("Generated at: {}\n\n", summary.generated_at));

    out.push_str("## Summary\n\n");
    out.push_str(&summary.summary);
    out.push_str("\n\n");

    out.push_str("```mermaid\npie title Tasks by status\n");
    out.push_str(&format!(
        "    \"todo\" : {}\n",
        count_status(snapshot, Status::Todo)
    ));
    out.push_str(&format!(
        "    \"in_progress\" : {}\n",
        count_status(snapshot, Status::InProgress)
    ));
    out.push_str(&format!(
        "    \"done\" : {}\n",
        count_status(snapshot, Status::Done)
    ));
    out.push_str("```\n\n");

    out.push_str("```mermaid\npie title Tasks by priority\n");
    out.push_str(&format!(
        "    \"Low\" : {}\n",
        count_priority(snapshot, Priority::Low)
    ));
    out.push_str(&format!(
        "    \"Medium\" : {}\n",
        count_priority(snapshot, Priority::Medium)
    ));
    out.push_str(&format!(
        "    \"High\" : {}\n",
        count_priority(snapshot, Priority::High)
    ));
    out.push_str("```\n\n");

    if snapshot.tasks.len() <= MAX_FLOWCHART_TASKS {
        out.push_str("```mermaid\nflowchart LR\n");
        for task in &snapshot.tasks {
            if let Some(id) = task.id {
                out.push_str(&format!("    t{id}[\"{}\"]\n", escape_mermaid(&task.name)));
            }
        }
        for dep in &snapshot.dependencies {
            out.push_str(&format!(
                "    t{} --> t{}\n",
                dep.task_id, dep.depends_on_id
            ));
        }
        out.push_str("```\n\n");
    } else {
        out.push_str(&format!(
            "> Dependency flowchart omitted: more than {MAX_FLOWCHART_TASKS} tasks.\n\n"
        ));
    }

    out.push_str("## Tasks\n\n");
    out.push_str("| Name | Status | Priority | Created | Completed |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for task in &snapshot.tasks {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            task.name,
            task.status.as_str(),
            task.priority.label(),
            task.created_at,
            task.completed_at.as_deref().unwrap_or("-"),
        ));
    }
    out.push('\n');

    out.push_str("## Notes\n\n");
    for task in &snapshot.tasks {
        let Some(task_id) = task.id else { continue };
        let notes: Vec<_> = snapshot
            .notes
            .iter()
            .filter(|n| n.task_id == task_id)
            .collect();
        if notes.is_empty() {
            continue;
        }
        out.push_str(&format!("### {}\n\n", task.name));
        for note in notes {
            out.push_str(&format!("- {}\n", note.content));
        }
        out.push('\n');
    }

    out
}

fn count_status(snapshot: &CompactSnapshot, status: Status) -> usize {
    snapshot.tasks.iter().filter(|t| t.status == status).count()
}

fn count_priority(snapshot: &CompactSnapshot, priority: Priority) -> usize {
    snapshot
        .tasks
        .iter()
        .filter(|t| t.priority == priority)
        .count()
}

fn escape_mermaid(value: &str) -> String {
    value.replace('"', "'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::*;
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    fn compacted(repo: &StubRepository, version_id: i64) -> VersionSummary {
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.create_version_summary(version_id, "Prose summary", &snapshot)
            .unwrap()
    }

    #[test]
    fn export_path_stays_inside_exports_dir() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let path = export_path(&base, "v1.0.0", 42);
        assert_eq!(path.parent().unwrap(), base);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("v1-0-0-42-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
    }

    #[test]
    fn sanitize_file_component_neutralizes_traversal_and_separators() {
        assert_eq!(sanitize_file_component("../../.config/x"), "config-x");
        assert_eq!(sanitize_file_component("..\\..\\windows"), "windows");
        assert_eq!(sanitize_file_component("v1.0.0"), "v1-0-0");
        assert_eq!(sanitize_file_component("/"), "version");
        assert_eq!(sanitize_file_component("  "), "version");
    }

    #[test]
    fn ensure_within_dir_rejects_paths_outside_the_exports_dir() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let outside = dir.path().parent().unwrap().join("outside.md");
        let err = ensure_within_dir(&base, &outside).unwrap_err();
        assert!(matches!(err, AppError::Validation(message) if message.contains("escapes")));
        assert!(ensure_within_dir(&base, &base.join("inside.md")).is_ok());
    }

    #[test]
    fn export_with_malicious_version_name_stays_inside_exports_dir() {
        let repo = StubRepository::default();
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo
            .create_version(pid, "../../../.config/evil", "")
            .unwrap()
            .id
            .unwrap();
        repo.create_task(pid, v, "A", "", Status::Done, Priority::High)
            .unwrap();
        let summary = compacted(&repo, v);
        let dir = tempfile::tempdir().unwrap();
        let path =
            export_version_summary_markdown_to(&repo, summary.id.unwrap(), dir.path()).unwrap();
        let base = dir.path().canonicalize().unwrap();
        assert_eq!(path.parent().unwrap(), base);
        assert!(path.exists());
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.contains(".."), "{name}");
        assert!(!name.contains('/'), "{name}");
    }

    #[test]
    fn render_markdown_includes_prose_and_mermaid_blocks() {
        let repo = StubRepository::default();
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        let a = repo
            .create_task(pid, v, "A", "", Status::Done, Priority::High)
            .unwrap();
        repo.add_task_note(a.id.unwrap(), "a note").unwrap();
        repo.create_task(pid, v, "B", "", Status::Todo, Priority::Low)
            .unwrap();

        let summary = compacted(&repo, v);
        let snapshot: CompactSnapshot = serde_json::from_str(&summary.snapshot_json).unwrap();
        let markdown = render_markdown("P", "v1.0.0", &summary, &snapshot);

        assert!(markdown.contains("# P — v1.0.0"));
        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("Prose summary"));
        assert!(markdown.contains("pie title Tasks by status"));
        assert!(markdown.contains("pie title Tasks by priority"));
        assert!(markdown.contains("flowchart LR"));
        assert!(markdown.contains("a note"));
    }

    #[test]
    fn render_markdown_omits_flowchart_for_many_tasks() {
        let repo = StubRepository::default();
        let pid = repo.create_project("P", "").unwrap().id.unwrap();
        let v = repo.create_version(pid, "v1.0.0", "").unwrap().id.unwrap();
        for i in 0..(MAX_FLOWCHART_TASKS + 1) {
            repo.create_task(pid, v, &format!("T{i}"), "", Status::Done, Priority::Low)
                .unwrap();
        }
        let summary = compacted(&repo, v);
        let snapshot: CompactSnapshot = serde_json::from_str(&summary.snapshot_json).unwrap();
        let markdown = render_markdown("P", "v1.0.0", &summary, &snapshot);

        assert!(!markdown.contains("flowchart LR"));
        assert!(markdown.contains("Dependency flowchart omitted"));
    }
}
