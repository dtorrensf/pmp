// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Contract tests for the public MCP surface of pmp 1.0: every tool (with its
//! input JSON schema and annotations) and every resource and template is
//! pinned by a snapshot. Any change to these shapes shows up as a snapshot
//! diff, so incompatible changes to existing clients fail in CI.

use super::*;

/// Names of the tools that 1.0 clients may call. Adding a tool means
/// updating this list and the snapshot on purpose.
const TOOL_CONTRACT: &[&str] = &[
    "list_tasks",
    "list_projects",
    "create_project",
    "update_project",
    "list_versions",
    "create_version",
    "update_version",
    "move_task",
    "update_task_priority",
    "update_task",
    "create_task",
    "update_task_status",
    "get_task_dependencies",
    "get_task_dependents",
    "add_task_dependency",
    "set_task_dependencies",
    "remove_task_dependency",
    "list_task_notes",
    "add_task_note",
    "delete_task_note",
    "preview_compact_version",
    "compact_version",
    "list_version_summaries",
    "read_version_summary",
    "delete_version_summary",
    "export_version_summary",
    "get_project",
    "get_version",
    "get_task",
    "delete_project",
    "delete_version",
    "delete_task",
    "list_ready_tasks",
    "claim_next_task",
];

#[test]
fn tool_set_matches_the_contract() {
    let mut names: Vec<String> = PmpServer::tool_router()
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    let mut expected: Vec<String> = TOOL_CONTRACT.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(names, expected);
}

#[test]
fn tool_definitions_match_snapshot() {
    let mut tools = PmpServer::tool_router().list_all();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    insta::assert_json_snapshot!("tool_definitions", tools);
}

const READ_ONLY: &[&str] = &[
    "list_projects",
    "get_project",
    "list_versions",
    "get_version",
    "list_tasks",
    "list_ready_tasks",
    "get_task",
    "get_task_dependencies",
    "get_task_dependents",
    "list_task_notes",
    "preview_compact_version",
    "list_version_summaries",
    "read_version_summary",
];

const ADDITIVE: &[&str] = &[
    "create_project",
    "create_version",
    "create_task",
    "add_task_note",
    "export_version_summary",
];

const ADDITIVE_IDEMPOTENT: &[&str] = &["add_task_dependency"];

const MUTATING_IDEMPOTENT: &[&str] = &[
    "update_project",
    "update_version",
    "update_task",
    "update_task_status",
    "update_task_priority",
    "move_task",
    "set_task_dependencies",
    "remove_task_dependency",
    "delete_project",
    "delete_version",
    "delete_task",
    "delete_task_note",
    "delete_version_summary",
];

const MUTATING_NON_IDEMPOTENT: &[&str] = &["claim_next_task", "compact_version"];

fn expected_hints(name: &str) -> (bool, bool, bool) {
    let group = |group: &[&str]| group.contains(&name);
    if group(READ_ONLY) {
        (true, false, true)
    } else if group(ADDITIVE) {
        (false, false, false)
    } else if group(ADDITIVE_IDEMPOTENT) {
        (false, false, true)
    } else if group(MUTATING_IDEMPOTENT) {
        (false, true, true)
    } else if group(MUTATING_NON_IDEMPOTENT) {
        (false, true, false)
    } else {
        panic!("tool {name} is missing from the annotation groups");
    }
}

#[test]
fn every_tool_has_annotations_and_an_output_schema() {
    for tool in PmpServer::tool_router().list_all() {
        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no annotations", tool.name));
        let (read_only, destructive, idempotent) = expected_hints(&tool.name);
        assert_eq!(
            annotations.read_only_hint,
            Some(read_only),
            "readOnlyHint for {}",
            tool.name
        );
        assert_eq!(
            annotations.destructive_hint,
            Some(destructive),
            "destructiveHint for {}",
            tool.name
        );
        assert_eq!(
            annotations.idempotent_hint,
            Some(idempotent),
            "idempotentHint for {}",
            tool.name
        );
        assert_eq!(
            annotations.open_world_hint,
            Some(false),
            "openWorldHint for {}",
            tool.name
        );
        assert!(
            tool.output_schema.is_some(),
            "{} has no outputSchema",
            tool.name
        );
        assert_eq!(
            tool.output_schema.as_ref().unwrap().get("type"),
            Some(&serde_json::json!("object")),
            "{} outputSchema must be an object",
            tool.name
        );
    }
}

#[test]
fn prompt_definitions_match_snapshot() {
    let mut prompts = PmpServer::prompt_router().list_all();
    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    insta::assert_json_snapshot!("prompt_definitions", prompts);
}

#[tokio::test]
async fn resource_surface_matches_snapshot() {
    let (server, _) = server();
    let resources = server.list_resources_page(None).await.unwrap();
    let templates = server.list_resource_templates_page(None).await.unwrap();
    insta::assert_json_snapshot!(
        "resource_definitions",
        serde_json::json!({
            "resources": resources.resources,
            "resourceTemplates": templates.resource_templates,
        })
    );
}
