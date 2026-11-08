// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Markdown export of a version summary, shared by the MCP tool and the TUI.
//!
//! The generated document is self-contained: prose, Mermaid pie charts for
//! status and priority, a dependency flowchart (when small enough) and a task
//! table plus per-task notes, all derived from the frozen `CompactSnapshot`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::snapshot::CompactSnapshot;
use crate::domain::task::{Priority, Status};
use crate::domain::version_summary::VersionSummary;

const MAX_FLOWCHART_TASKS: usize = 20;

/// Writes the Markdown export for `summary_id` to `path` (or the default data
/// dir location when omitted) and returns the path written.
pub fn export_version_summary_markdown(
    repo: &dyn Repository,
    summary_id: i64,
    path: Option<String>,
) -> Result<PathBuf, AppError> {
    let summary = repo.get_version_summary(summary_id)?;
    let snapshot: CompactSnapshot = serde_json::from_str(&summary.snapshot_json)
        .map_err(|e| AppError::Validation(format!("snapshot parse error: {e}")))?;
    let version = repo.get_version(summary.version_id)?;
    let project = repo.get_project(version.project_id)?;

    let markdown = render_markdown(&project.name, &version.name, &summary, &snapshot);
    let target = match path {
        Some(path) => PathBuf::from(path),
        None => default_export_path(&version.name, summary_id)?,
    };

    let parent = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| AppError::Validation(format!("cannot create export directory: {e}")))?;
    std::fs::write(&target, markdown)
        .map_err(|e| AppError::Validation(format!("cannot write export file: {e}")))?;
    Ok(target)
}

fn default_export_path(version_name: &str, summary_id: i64) -> Result<PathBuf, AppError> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::Validation("no data directory available".to_string()))?
        .join("pmp")
        .join("exports");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(base.join(format!("{version_name}-{summary_id}-{timestamp}.md")))
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
    use crate::db::stub::StubRepository;
    use crate::domain::task::{Priority, Status};

    fn compacted(repo: &StubRepository, version_id: i64) -> VersionSummary {
        let snapshot = repo.build_compact_snapshot(version_id).unwrap();
        repo.create_version_summary(version_id, "Prose summary", &snapshot)
            .unwrap()
    }

    #[test]
    fn default_export_path_uses_data_dir() {
        let path = default_export_path("v1.0.0", 42).unwrap();
        let base = dirs::data_dir().unwrap().join("pmp").join("exports");
        assert_eq!(path.parent().unwrap(), base);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("v1.0.0-42-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
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
