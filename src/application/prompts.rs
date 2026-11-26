// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reusable prompt templates for MCP clients and ACP agents.
//!
//! The same text must reach an agent whether it was selected as an MCP prompt
//! or built by an ACP session. Keeping the templates in the application layer
//! means both transports render exactly the same instructions.

use super::dependencies::DependencyView;
use super::records::TaskDetail;

/// Renders the prompt for implementing one concrete task.
///
/// The task and its resolved blockers are embedded so an ACP agent can work
/// without first fetching the task, while an MCP client still gets the complete
/// context in a single prompt message.
pub fn implement_task(detail: &TaskDetail, dependencies: &DependencyView) -> String {
    let task = &detail.task;
    let id = task.id.unwrap_or_default();
    let mut text = format!(
        "Implement PMP task #{id}: {}\n\n\
         Status: {}\n\
         Priority: {}\n\n\
         ## Context\n\n{}\n\n\
         ## Requirements\n\n{}\n\n\
         ## Benefits\n\n{}\n\n",
        task.name,
        task.status.label(),
        task.priority.label(),
        section_or_empty(&detail.sections.context),
        section_or_empty(&detail.sections.requirements),
        section_or_empty(&detail.sections.benefits),
    );

    text.push_str("## Dependencies\n\n");
    if dependencies.blocked_by.is_empty() {
        text.push_str("None. This task has no blockers.\n");
    } else {
        for blocker in &dependencies.blocked_by {
            text.push_str(&format!(
                "- #{} {} — {} ({})\n",
                blocker.id,
                blocker.name,
                blocker.status.label(),
                blocker.priority.label()
            ));
        }
    }

    if !detail.recent_notes.is_empty() {
        text.push_str("\n## Recent notes\n\n");
        for note in &detail.recent_notes {
            text.push_str(&format!("- {}\n", note.content));
        }
    }

    text.push_str(
        "\n## Workflow\n\n\
         1. Work only on this task; do not use claim_next_task for a different task.\n\
         2. If the task is not already in progress, mark it in_progress through the PMP MCP server.\n\
         3. Read the relevant code and tests before changing anything.\n\
         4. Keep the change focused on the Context, Requirements and Benefits above.\n\
         5. Run the relevant tests and checks, including formatting and linting when available.\n\
         6. Record progress, decisions and verification in a task note.\n\
         7. Mark the task done only after the verification passes; otherwise leave it in progress and explain what remains.\n\n\
         Never bypass the PMP MCP server, the repository and its tests, or the user's review.",
    );
    text
}

/// Renders a planning prompt for a version.
pub fn plan_version(version_id: i64) -> String {
    format!(
        "Plan PMP version #{version_id}.\n\n\
         Work in read-only mode: do not create, edit, move, compact or delete records.\n\n\
         Steps:\n\
         1. Read the version with get_version and its tasks with list_tasks (include_description=true).\n\
         2. Inspect each task's blockers with get_task_dependencies.\n\
         3. Summarise the goal, scope, sequencing, parallel work, blockers and acceptance criteria.\n\
         4. Point out missing requirements, oversized tasks and unclear dependencies.\n\
         5. Propose an ordered plan with concrete task ids. Do not invent work that is not needed for the version goal.\n\n\
         Stop after presenting the plan unless the user explicitly asks you to apply it."
    )
}

/// Renders a release-readiness review prompt for a version.
pub fn review_version(version_id: i64) -> String {
    format!(
        "Review PMP version #{version_id} for release readiness.\n\n\
         Work in read-only mode: do not change task status, dependencies or summaries.\n\n\
         Steps:\n\
         1. Read the version with get_version and list every task with list_tasks.\n\
         2. Check incomplete or blocked work, dependency loops and tasks without verification notes.\n\
         3. Compare the finished work with each task's Requirements and Benefits.\n\
         4. Report findings first, ordered by severity, with task ids and file or test evidence when available.\n\
         5. List the checks you could verify and any checks that remain unverified.\n\n\
         Conclude with a clear release recommendation: ready, ready with caveats, or not ready."
    )
}

/// Renders a release-notes prompt for a version.
pub fn release_notes(version_id: i64) -> String {
    format!(
        "Write release notes for PMP version #{version_id}.\n\n\
         Base the notes only on the version's completed tasks and compacted summaries; never invent changes.\n\n\
         Steps:\n\
         1. Read the version with get_version.\n\
         2. Use list_version_summaries and read_version_summary when the version is compacted.\n\
         3. Use list_tasks for any remaining done tasks and their notes as supporting evidence.\n\
         4. Produce Markdown with a short summary, highlights grouped by area, fixes, breaking changes and upgrade notes when applicable.\n\
         5. Mention only user-visible impacts and link each item to a task id.\n\n\
         Do not compact, export or modify the version unless the user explicitly asks."
    )
}

fn section_or_empty(value: &str) -> &str {
    if value.trim().is_empty() {
        "(not provided)"
    } else {
        value.trim()
    }
}

#[cfg(test)]
mod tests {
    use super::super::dependencies::{DependencyView, TaskRef};
    use super::super::records::TaskDetail;
    use super::{implement_task, plan_version, release_notes, review_version};
    use crate::domain::task::{Priority, Status, Task};
    use crate::domain::task_description::TaskSections;

    fn detail() -> TaskDetail {
        TaskDetail {
            task: Task {
                id: Some(7),
                project_id: 1,
                version_id: 2,
                name: "Implement M7".to_string(),
                description: String::new(),
                status: Status::Todo,
                priority: Priority::High,
                created_at: "now".to_string(),
                completed_at: None,
            },
            sections: TaskSections {
                context: "The MCP server needs prompts.".to_string(),
                requirements: "Add annotations and output schemas.".to_string(),
                benefits: "Clients can discover the contract.".to_string(),
            },
            recent_notes: Vec::new(),
            depends_on: vec![3],
            blocks: Vec::new(),
        }
    }

    fn dependencies() -> DependencyView {
        DependencyView {
            task_id: 7,
            ready: false,
            blocked_by: vec![TaskRef {
                id: 3,
                name: "Shared use case".to_string(),
                status: Status::InProgress,
                priority: Priority::Medium,
            }],
            blocks: Vec::new(),
        }
    }

    #[test]
    fn implement_task_includes_sections_and_blockers() {
        let text = implement_task(&detail(), &dependencies());
        for needle in [
            "task #7",
            "The MCP server needs prompts.",
            "Add annotations and output schemas.",
            "Clients can discover the contract.",
            "#3 Shared use case",
            "claim_next_task",
            "in_progress",
            "task done only after",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn implement_task_marks_an_empty_section() {
        let mut detail = detail();
        detail.sections.context = "  ".to_string();
        let text = implement_task(&detail, &dependencies());
        assert!(text.contains("## Context\n\n(not provided)"));
    }

    #[test]
    fn version_prompts_name_their_version_and_are_read_only() {
        for text in [plan_version(12), review_version(12), release_notes(12)] {
            assert!(text.contains("#12"));
            assert!(text.contains("read-only") || text.contains("never invent"));
        }
    }
}
