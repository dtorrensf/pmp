// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
mod args;
mod mcp;
mod plan;
mod renderers;
mod template;

use std::path::PathBuf;

pub use args::{IndividualTarget, InstallArgs, Scope, Target};
pub use plan::{InstallPlan, execute_plan, plan, plans, run};
pub use renderers::{
    AntigravityRenderer, ClaudeRenderer, CodexRenderer, CopilotRenderer, CursorRenderer,
    OpenCodeRenderer, RenderContext, TargetRenderer, renderer_for,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOp {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAction {
    Write(WriteOp),
    Guidance(String),
}

#[cfg(test)]
mod tests {
    use super::plan::write_op;
    use super::template::markdown_frontmatter;
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_install_args_defaults() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "claude"]).unwrap();
        assert_eq!(args.target, Target::Claude);
        assert_eq!(args.scope, Scope::Project);
        assert_eq!(args.project_dir, std::env::current_dir().unwrap());
        assert!(!args.dry_run);
    }

    #[test]
    fn parse_install_args_explicit() {
        let args = InstallArgs::parse_args(&[
            "agents",
            "install",
            "--target",
            "claude",
            "--scope",
            "global",
            "--project-dir",
            "/tmp/foo",
            "--dry-run",
        ])
        .unwrap();
        assert_eq!(args.target, Target::Claude);
        assert_eq!(args.scope, Scope::Global);
        assert_eq!(args.project_dir, PathBuf::from("/tmp/foo"));
        assert!(args.dry_run);
    }

    fn project_ctx(project_dir: &TempDir) -> RenderContext {
        project_ctx_for(project_dir, Target::Claude)
    }

    fn project_ctx_for(project_dir: &TempDir, target: Target) -> RenderContext {
        RenderContext {
            target,
            scope: Scope::Project,
            project_dir: project_dir.path().to_path_buf(),
            home_dir: project_dir.path().join("home").to_path_buf(),
            dry_run: false,
        }
    }

    fn global_ctx(home_dir: &TempDir) -> RenderContext {
        global_ctx_for(home_dir, Target::Claude)
    }

    fn global_ctx_for(home_dir: &TempDir, target: Target) -> RenderContext {
        RenderContext {
            target,
            scope: Scope::Global,
            project_dir: home_dir.path().join("project").to_path_buf(),
            home_dir: home_dir.path().to_path_buf(),
            dry_run: false,
        }
    }

    #[test]
    fn plan_rejects_meta_target_all_instead_of_panicking() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = project_ctx(&tmp);
        ctx.target = Target::All;

        let err = plan(&ctx).expect_err("plan(All) must return an error, not panic");
        assert!(
            err.to_string().contains("meta-target"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn markdown_frontmatter_includes_tools_line_when_present() {
        let out = markdown_frontmatter("pmp-query", "Reads things.", Some("Read, Grep"), false);
        assert_eq!(
            out,
            "---\nname: pmp-query\ndescription: Reads things.\ntools: Read, Grep\n---\n\n"
        );
    }

    #[test]
    fn markdown_frontmatter_omits_tools_line_when_absent() {
        let out = markdown_frontmatter("pmp-plan", "Plans things.", None, false);
        assert_eq!(
            out,
            "---\nname: pmp-plan\ndescription: Plans things.\n---\n\n"
        );
    }

    #[test]
    fn markdown_frontmatter_emits_readonly_flag() {
        let out = markdown_frontmatter("pmp-query", "Reads things.", None, true);
        assert_eq!(
            out,
            "---\nname: pmp-query\ndescription: Reads things.\nreadonly: true\n---\n\n"
        );
    }

    #[test]
    fn toml_escape_allows_adversarial_body_in_multiline_string() {
        let body = r#"Quote: "hello" and path: C:\Users\test"#;
        let escaped = template::toml_escape(body);
        let wrapped = format!("developer_instructions = \"\"\"{escaped}\"\"\"\n");
        let parsed: toml::Value =
            toml::from_str(&wrapped).expect("wrapped text must be valid TOML");
        assert_eq!(
            parsed["developer_instructions"]
                .as_str()
                .expect("must be a string"),
            body
        );
        assert_eq!(
            wrapped,
            "developer_instructions = \"\"\"Quote: \\\"hello\\\" and path: C:\\\\Users\\\\test\"\"\"\n"
        );
    }

    #[test]
    fn all_renderers_emit_four_agent_files() {
        const PLAIN_NAMES: [&str; 4] = [
            "pmp-orchestrator.md",
            "pmp-query.md",
            "pmp-plan.md",
            "pmp-implement.md",
        ];
        const COPILOT_NAMES: [&str; 4] = [
            "pmp-orchestrator.agent.md",
            "pmp-query.agent.md",
            "pmp-plan.agent.md",
            "pmp-implement.agent.md",
        ];
        const TOML_NAMES: [&str; 4] = [
            "pmp-orchestrator.toml",
            "pmp-query.toml",
            "pmp-plan.toml",
            "pmp-implement.toml",
        ];

        for &target in Target::individual_targets() {
            // (agents dir, expected file names, assert frontmatter delimiter)
            let (dir, expected_names, check_frontmatter) = match target {
                Target::Claude => (".claude/agents", PLAIN_NAMES, true),
                Target::OpenCode => (".opencode/agents", PLAIN_NAMES, true),
                Target::Cursor => (".cursor/agents", PLAIN_NAMES, true),
                Target::Copilot => (".github/agents", COPILOT_NAMES, false),
                Target::Codex => (".codex/agents", TOML_NAMES, false),
                Target::Antigravity => (".agents/agents", PLAIN_NAMES, true),
                Target::All => unreachable!("individual_targets() never yields All"),
            };

            let tmp = TempDir::new().unwrap();
            let ctx = project_ctx_for(&tmp, target);
            let plan = plan(&ctx).unwrap();

            assert_eq!(
                plan.writes.len(),
                4,
                "{target:?} should emit four agent files"
            );
            let names: Vec<String> = plan
                .writes
                .iter()
                .map(|op| op.path.file_name().unwrap().to_string_lossy().to_string())
                .collect();
            for expected in expected_names {
                assert!(
                    names.contains(&expected.to_string()),
                    "{target:?} missing {expected}"
                );
            }

            for op in &plan.writes {
                assert!(
                    op.path.starts_with(ctx.project_dir.join(dir)),
                    "{target:?} wrote outside {dir}: {}",
                    op.path.display()
                );
                if check_frontmatter {
                    assert!(
                        op.content.starts_with("---\n"),
                        "{target:?} file {} missing frontmatter delimiter",
                        op.path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn claude_query_agent_has_read_only_tools() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx(&tmp);
        let renderer = ClaudeRenderer;
        let ops = renderer.render(&ctx).unwrap();

        let query = ops
            .iter()
            .find(|op| op.path.ends_with("pmp-query.md"))
            .expect("pmp-query.md missing");
        assert!(
            query.content.contains("tools: Read, Grep, Glob, WebFetch"),
            "expected read-only tools in pmp-query frontmatter, got:\n{}",
            query.content
        );

        for op in ops.iter().filter(|op| !op.path.ends_with("pmp-query.md")) {
            assert!(
                !op.content.contains("tools:"),
                "expected no tools line in {}, got:\n{}",
                op.path.display(),
                op.content
            );
        }
    }

    #[test]
    fn mcp_merge_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx(&tmp);
        let renderer = ClaudeRenderer;

        let original = r#"{"existing": {"nested": true}}"#;
        let mcp_path = ctx.project_dir.join(".mcp.json");
        std::fs::create_dir_all(ctx.project_dir.join(".claude/agents")).unwrap();
        std::fs::write(&mcp_path, original).unwrap();

        let action = renderer.mcp_action(&ctx).unwrap();
        let first = match action {
            McpAction::Write(op) => op,
            _ => panic!("expected a write action for project scope"),
        };
        std::fs::write(&first.path, &first.content).unwrap();
        let first_bytes = std::fs::read(&first.path).unwrap();

        let action2 = renderer.mcp_action(&ctx).unwrap();
        let second = match action2 {
            McpAction::Write(op) => op,
            _ => panic!("expected a write action for project scope"),
        };
        std::fs::write(&second.path, &second.content).unwrap();
        let second_bytes = std::fs::read(&second.path).unwrap();

        assert_eq!(first_bytes, second_bytes);

        let merged: serde_json::Value = serde_json::from_slice(&second_bytes).unwrap();
        assert!(merged["existing"]["nested"].as_bool().unwrap());
        assert!(merged["mcpServers"]["pmp"]["command"].as_str().unwrap() == "pmp");
    }

    #[test]
    fn dry_run_writes_nothing_for_any_target() {
        for &target in Target::individual_targets() {
            // (path that must not be created, pre-existing MCP file that must stay untouched)
            let (absent, mcp_file) = match target {
                Target::Claude => (".claude/agents", Some(".mcp.json")),
                Target::OpenCode => (".opencode", None),
                Target::Cursor => (".cursor", None),
                Target::Copilot => (".github", None),
                Target::Codex => (".codex/agents", Some(".codex/config.toml")),
                Target::Antigravity => (".agents/agents", Some(".agents/mcp_config.json")),
                Target::All => unreachable!("individual_targets() never yields All"),
            };

            let tmp = TempDir::new().unwrap();
            let mut ctx = project_ctx_for(&tmp, target);
            ctx.dry_run = true;

            let preexisting = mcp_file.map(|rel| {
                let path = ctx.project_dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                let content = match target {
                    Target::Codex => "[placeholder]\n",
                    _ => "{}\n",
                };
                std::fs::write(&path, content).unwrap();
                (path, content.as_bytes())
            });

            let plan = plan(&ctx).unwrap();
            execute_plan(&ctx, &plan).unwrap();

            assert!(
                !ctx.project_dir.join(absent).exists(),
                "dry run should not create {absent} for {target:?}"
            );
            if let Some((path, expected)) = preexisting {
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    expected,
                    "dry run modified {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn global_scope_prints_mcp_guidance_and_skips_writes_when_dry_run() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = global_ctx(&tmp);
        ctx.dry_run = true;

        let plan = plan(&ctx).unwrap();
        assert_eq!(
            plan.mcp,
            McpAction::Guidance("claude mcp add -s user pmp -- pmp mcp".to_string())
        );

        execute_plan(&ctx, &plan).unwrap();
        assert!(!ctx.home_dir.join(".claude/agents").exists());
    }

    #[test]
    fn full_install_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx(&tmp);

        let plan = plan(&ctx).unwrap();
        execute_plan(&ctx, &plan).unwrap();
        let first_mcp = std::fs::read(ctx.project_dir.join(".mcp.json")).unwrap();
        let first_agent =
            std::fs::read(ctx.project_dir.join(".claude/agents/pmp-orchestrator.md")).unwrap();

        execute_plan(&ctx, &plan).unwrap();
        let second_mcp = std::fs::read(ctx.project_dir.join(".mcp.json")).unwrap();
        let second_agent =
            std::fs::read(ctx.project_dir.join(".claude/agents/pmp-orchestrator.md")).unwrap();

        assert_eq!(first_mcp, second_mcp);
        assert_eq!(first_agent, second_agent);
    }

    #[test]
    fn parse_opencode_target() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "opencode"]).unwrap();
        assert_eq!(args.target, Target::OpenCode);
    }

    #[test]
    fn parse_cursor_target() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "cursor"]).unwrap();
        assert_eq!(args.target, Target::Cursor);
    }

    #[test]
    fn parse_copilot_target() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "copilot"]).unwrap();
        assert_eq!(args.target, Target::Copilot);
    }

    #[test]
    fn parse_all_target() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "all"]).unwrap();
        assert_eq!(args.target, Target::All);
    }

    /// Golden test: the OpenCode renderer must produce bytes identical to the
    /// committed `.opencode/agents/*.md` files. We use direct byte comparison
    /// rather than insta snapshots because the committed files themselves are the
    /// canonical golden output; maintaining a parallel snapshot file would only
    /// duplicate them and could drift independently.
    #[test]
    fn opencode_rendered_agents_match_committed_files() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ctx = RenderContext {
            target: Target::OpenCode,
            scope: Scope::Project,
            project_dir: repo_root.clone(),
            home_dir: repo_root.join("home"),
            dry_run: false,
        };
        let renderer = OpenCodeRenderer;
        let ops = renderer.render(&ctx).unwrap();

        assert_eq!(ops.len(), 4, "expected four OpenCode agent files");
        for op in &ops {
            let committed = std::fs::read(&op.path)
                .unwrap_or_else(|e| panic!("failed to read committed {}: {e}", op.path.display()));
            assert_eq!(
                op.content.as_bytes(),
                committed.as_slice(),
                "rendered content for {} does not match committed file",
                op.path.display()
            );
        }
    }

    #[test]
    fn opencode_orchestrator_is_primary_others_are_subagents() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::OpenCode);
        let plan = plan(&ctx).unwrap();

        for op in &plan.writes {
            let name = op.path.file_name().unwrap().to_string_lossy().to_string();
            if name == "pmp-orchestrator.md" {
                assert!(
                    op.content.contains("mode: primary"),
                    "expected primary mode for orchestrator, got:\n{}",
                    op.content
                );
            } else {
                assert!(
                    op.content.contains("mode: subagent"),
                    "expected subagent mode for {name}, got:\n{}",
                    op.content
                );
            }
        }
    }

    #[test]
    fn opencode_renderer_includes_permission_blocks() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::OpenCode);
        let plan = plan(&ctx).unwrap();

        for op in &plan.writes {
            assert!(
                op.content.contains("permission:"),
                "missing permission block in {}",
                op.path.display()
            );
            assert!(
                op.content.contains("read: allow"),
                "missing read permission in {}",
                op.path.display()
            );
        }

        let query = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-query.md"))
            .expect("pmp-query.md missing");
        assert!(
            query.content.contains("edit: deny"),
            "pmp-query should deny edit tools, got:\n{}",
            query.content
        );

        let implement = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-implement.md"))
            .expect("pmp-implement.md missing");
        assert!(
            implement.content.contains("edit: allow"),
            "pmp-implement should allow edit tools, got:\n{}",
            implement.content
        );
    }

    #[test]
    fn opencode_mcp_merge_is_idempotent_and_preserves_default_agent() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::OpenCode);

        let original = r#"{"$schema":"https://opencode.ai/config.json","default_agent":"pmp-orchestrator","subagent_depth":1}"#;
        let mcp_path = ctx.project_dir.join("opencode.json");
        std::fs::write(&mcp_path, original).unwrap();

        let first_plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &first_plan.mcp {
            write_op(op).unwrap();
        }
        let first_bytes = std::fs::read(&mcp_path).unwrap();

        let second_plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &second_plan.mcp {
            write_op(op).unwrap();
        }
        let second_bytes = std::fs::read(&mcp_path).unwrap();

        assert_eq!(first_bytes, second_bytes);

        let merged: serde_json::Value = serde_json::from_slice(&second_bytes).unwrap();
        assert_eq!(
            merged["default_agent"].as_str().unwrap(),
            "pmp-orchestrator"
        );
        assert_eq!(merged["mcp"]["pmp"]["type"].as_str().unwrap(), "local");
        assert!(merged["mcp"]["pmp"]["enabled"].as_bool().unwrap());
        let command = merged["mcp"]["pmp"]["command"].as_array().unwrap();
        assert_eq!(command[0].as_str().unwrap(), "pmp");
        assert_eq!(command[1].as_str().unwrap(), "mcp");
    }

    #[test]
    fn cursor_query_agent_is_readonly() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Cursor);
        let plan = plan(&ctx).unwrap();

        let query = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-query.md"))
            .expect("pmp-query.md missing");
        assert!(
            query.content.contains("readonly: true"),
            "expected readonly flag in pmp-query, got:\n{}",
            query.content
        );

        for op in plan
            .writes
            .iter()
            .filter(|op| !op.path.ends_with("pmp-query.md"))
        {
            assert!(
                !op.content.contains("readonly:"),
                "unexpected readonly flag in {}, got:\n{}",
                op.path.display(),
                op.content
            );
        }
    }

    #[test]
    fn cursor_mcp_merge_uses_mcp_servers_root_key() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Cursor);

        let mcp_path = ctx.project_dir.join(".cursor/mcp.json");
        let plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &plan.mcp {
            write_op(op).unwrap();
        }

        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(
            merged.get("mcpServers").is_some(),
            "expected mcpServers root key"
        );
        assert_eq!(
            merged["mcpServers"]["pmp"]["command"].as_str().unwrap(),
            "pmp"
        );
        let args = merged["mcpServers"]["pmp"]["args"].as_array().unwrap();
        assert_eq!(args[0].as_str().unwrap(), "mcp");
    }

    #[test]
    fn copilot_query_agent_omits_edit_tools() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Copilot);
        let plan = plan(&ctx).unwrap();

        let query = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-query.agent.md"))
            .expect("pmp-query.agent.md missing");
        assert!(
            !query.content.contains("edit_file"),
            "pmp-query should not include edit_file, got:\n{}",
            query.content
        );

        let implement = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-implement.agent.md"))
            .expect("pmp-implement.agent.md missing");
        assert!(
            implement.content.contains("edit_file"),
            "pmp-implement should include edit_file, got:\n{}",
            implement.content
        );
    }

    #[test]
    fn copilot_mcp_merge_uses_servers_root_key() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Copilot);

        let mcp_path = ctx.project_dir.join(".vscode/mcp.json");
        let plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &plan.mcp {
            write_op(op).unwrap();
        }

        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(merged.get("servers").is_some(), "expected servers root key");
        assert!(
            merged.get("mcpServers").is_none(),
            "copilot must not use mcpServers root key"
        );
        assert_eq!(merged["servers"]["pmp"]["type"].as_str().unwrap(), "stdio");
        assert_eq!(merged["servers"]["pmp"]["command"].as_str().unwrap(), "pmp");
        let args = merged["servers"]["pmp"]["args"].as_array().unwrap();
        assert_eq!(args[0].as_str().unwrap(), "mcp");
    }

    #[test]
    fn target_all_produces_plans_for_all_six_targets() {
        let tmp = TempDir::new().unwrap();
        let ctx = RenderContext {
            target: Target::All,
            scope: Scope::Project,
            project_dir: tmp.path().to_path_buf(),
            home_dir: tmp.path().join("home").to_path_buf(),
            dry_run: false,
        };
        let all_plans = plans(&ctx).unwrap();
        assert_eq!(all_plans.len(), 6);

        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".claude/agents"))
        }));
        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".opencode/agents"))
        }));
        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".cursor/agents"))
        }));
        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".github/agents"))
        }));
        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".codex/agents"))
        }));
        assert!(all_plans.iter().any(|p| {
            p.writes
                .iter()
                .any(|op| op.path.to_string_lossy().contains(".agents/agents"))
        }));
    }

    #[test]
    fn new_targets_use_correct_global_agent_paths() {
        let tmp = TempDir::new().unwrap();

        let opencode_ctx = global_ctx_for(&tmp, Target::OpenCode);
        let opencode_plan = plan(&opencode_ctx).unwrap();
        for op in &opencode_plan.writes {
            assert!(
                op.path
                    .starts_with(opencode_ctx.home_dir.join(".config/opencode/agents")),
                "expected OpenCode global agent under .config/opencode/agents, got {}",
                op.path.display()
            );
        }

        let cursor_ctx = global_ctx_for(&tmp, Target::Cursor);
        let cursor_plan = plan(&cursor_ctx).unwrap();
        for op in &cursor_plan.writes {
            assert!(
                op.path
                    .starts_with(cursor_ctx.home_dir.join(".cursor/agents")),
                "expected Cursor global agent under .cursor/agents, got {}",
                op.path.display()
            );
        }

        let copilot_ctx = global_ctx_for(&tmp, Target::Copilot);
        let copilot_plan = plan(&copilot_ctx).unwrap();
        for op in &copilot_plan.writes {
            assert!(
                op.path
                    .starts_with(copilot_ctx.home_dir.join(".vscode/agents")),
                "expected Copilot global agent under .vscode/agents, got {}",
                op.path.display()
            );
        }

        let codex_ctx = global_ctx_for(&tmp, Target::Codex);
        let codex_plan = plan(&codex_ctx).unwrap();
        for op in &codex_plan.writes {
            assert!(
                op.path
                    .starts_with(codex_ctx.home_dir.join(".codex/agents")),
                "expected Codex global agent under .codex/agents, got {}",
                op.path.display()
            );
        }

        let antigravity_ctx = global_ctx_for(&tmp, Target::Antigravity);
        let antigravity_plan = plan(&antigravity_ctx).unwrap();
        for op in &antigravity_plan.writes {
            assert!(
                op.path
                    .starts_with(antigravity_ctx.home_dir.join(".gemini/config/agents")),
                "expected Antigravity global agent under .gemini/config/agents, got {}",
                op.path.display()
            );
        }
    }

    #[test]
    fn parse_codex_target() {
        let args = InstallArgs::parse_args(&["agents", "install", "--target", "codex"]).unwrap();
        assert_eq!(args.target, Target::Codex);
    }

    #[test]
    fn parse_antigravity_target() {
        let args =
            InstallArgs::parse_args(&["agents", "install", "--target", "antigravity"]).unwrap();
        assert_eq!(args.target, Target::Antigravity);
    }

    #[test]
    fn antigravity_orchestrator_is_main_agent_others_are_subagents() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Antigravity);
        let plan = plan(&ctx).unwrap();

        for op in &plan.writes {
            let name = op.path.file_name().unwrap().to_string_lossy().to_string();
            if name == "pmp-orchestrator.md" {
                assert!(
                    op.content.contains("mainAgent: true"),
                    "expected mainAgent: true for orchestrator, got:\n{}",
                    op.content
                );
                assert!(
                    !op.content.contains("subagent:"),
                    "orchestrator must not be a subagent"
                );
            } else {
                assert!(
                    op.content.contains("subagent: true"),
                    "expected subagent: true for {name}, got:\n{}",
                    op.content
                );
                assert!(
                    !op.content.contains("mainAgent:"),
                    "subagent must not be mainAgent"
                );
            }
        }
    }

    #[test]
    fn antigravity_agents_use_antigravity_tool_names() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Antigravity);
        let plan = plan(&ctx).unwrap();

        for op in &plan.writes {
            assert!(
                op.content
                    .contains("tools: view_file, replace_file_content, run_command, manage_task"),
                "expected Antigravity tools in {}, got:\n{}",
                op.path.display(),
                op.content
            );
        }
    }

    #[test]
    fn codex_query_agent_has_sandbox_readonly() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Codex);
        let plan = plan(&ctx).unwrap();

        let query = plan
            .writes
            .iter()
            .find(|op| op.path.ends_with("pmp-query.toml"))
            .expect("pmp-query.toml missing");
        assert!(
            query.content.contains("sandbox_mode = \"read-only\""),
            "expected sandbox_mode for pmp-query, got:\n{}",
            query.content
        );

        for op in plan
            .writes
            .iter()
            .filter(|op| !op.path.ends_with("pmp-query.toml"))
        {
            assert!(
                !op.content.contains("sandbox_mode"),
                "unexpected sandbox_mode in {}, got:\n{}",
                op.path.display(),
                op.content
            );
        }
    }

    #[test]
    fn codex_mcp_append_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Codex);

        let mcp_path = ctx.project_dir.join(".codex/config.toml");
        std::fs::create_dir_all(ctx.project_dir.join(".codex/agents")).unwrap();

        let first_plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &first_plan.mcp {
            write_op(op).unwrap();
        }
        let first_bytes = std::fs::read(&mcp_path).unwrap();

        let second_plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &second_plan.mcp {
            write_op(op).unwrap();
        }
        let second_bytes = std::fs::read(&mcp_path).unwrap();

        assert_eq!(first_bytes, second_bytes);

        let text = std::str::from_utf8(&second_bytes).unwrap();
        let count = text.matches("[mcp_servers.pmp]").count();
        assert_eq!(count, 1, "mcp_servers.pmp block must appear exactly once");
    }

    #[test]
    fn codex_mcp_preserves_existing_sections() {
        let tmp = TempDir::new().unwrap();
        let ctx = project_ctx_for(&tmp, Target::Codex);

        let mcp_path = ctx.project_dir.join(".codex/config.toml");
        std::fs::create_dir_all(ctx.project_dir.join(".codex")).unwrap();
        let original = "[existing]\nkey = \"value\"\n";
        std::fs::write(&mcp_path, original).unwrap();

        let plan = plan(&ctx).unwrap();
        if let McpAction::Write(op) = &plan.mcp {
            write_op(op).unwrap();
        }

        let merged = std::fs::read_to_string(&mcp_path).unwrap();
        assert!(
            merged.contains("[existing]"),
            "existing section was removed"
        );
        assert!(
            merged.contains("key = \"value\""),
            "existing key was removed"
        );
        assert!(merged.contains("[mcp_servers.pmp]"), "pmp section missing");
        assert!(merged.contains("command = \"pmp\""), "pmp command missing");
        assert_eq!(
            merged.matches("[mcp_servers.pmp]").count(),
            1,
            "pmp section must appear exactly once"
        );
    }
}
