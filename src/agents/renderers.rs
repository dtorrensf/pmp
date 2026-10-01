// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::args::{IndividualTarget, InstallArgs, Scope, Target};
use super::mcp::mcp_write;
use super::template::{
    AGENT_SOURCES, Template, markdown_frontmatter, markdown_frontmatter_with_extras, toml_escape,
};
use super::{McpAction, WriteOp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderContext {
    pub target: Target,
    pub scope: Scope,
    pub project_dir: PathBuf,
    pub home_dir: PathBuf,
    pub dry_run: bool,
}

impl RenderContext {
    pub fn from_args(args: &InstallArgs, home_dir: PathBuf) -> Self {
        Self {
            target: args.target,
            scope: args.scope,
            project_dir: args.project_dir.clone(),
            home_dir,
            dry_run: args.dry_run,
        }
    }

    pub fn base_dir(&self) -> &Path {
        match self.scope {
            Scope::Project => &self.project_dir,
            Scope::Global => &self.home_dir,
        }
    }
}

pub trait TargetRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>>;
    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction>;
}

fn render_agent_files<M>(
    base: &Path,
    filename_suffix: &str,
    metadata: &[M; AGENT_SOURCES.len()],
    frontmatter: impl Fn(&Template, &M) -> String,
) -> Result<Vec<WriteOp>> {
    let mut ops = Vec::with_capacity(AGENT_SOURCES.len());
    for (agent, meta) in AGENT_SOURCES.iter().zip(metadata.iter()) {
        let template = Template::parse(agent.source)?;
        let content = frontmatter(&template, meta) + &template.body;
        ops.push(WriteOp {
            path: base.join(format!("{}{filename_suffix}", agent.stem)),
            content,
        });
    }
    Ok(ops)
}

pub struct ClaudeRenderer;

impl ClaudeRenderer {
    const TOOLS: [Option<&'static str>; 4] = [None, Some("Read, Grep, Glob, WebFetch"), None, None];
}

impl TargetRenderer for ClaudeRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        render_agent_files(
            &ctx.base_dir().join(".claude/agents"),
            ".md",
            &Self::TOOLS,
            |template, tools| {
                markdown_frontmatter(&template.name, &template.description, *tools, false)
            },
        )
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        match ctx.scope {
            Scope::Project => mcp_write(
                ctx.project_dir.join(".mcp.json"),
                "mcpServers",
                serde_json::json!({
                    "command": "pmp",
                    "args": ["mcp"],
                }),
            ),
            Scope::Global => Ok(McpAction::Guidance(
                "claude mcp add -s user pmp -- pmp mcp".to_string(),
            )),
        }
    }
}

pub struct OpenCodeRenderer;

impl OpenCodeRenderer {
    const PERMISSION_ORCHESTRATOR: &'static str = r#"permission:
  read: allow
  glob: allow
  grep: allow
  edit: deny
  bash:
    "*": deny
    "cargo test*": allow
    "cargo fmt --check": allow
    "cargo check": allow
    "cargo clippy*": allow
    "cargo audit": allow
    "git *": allow
    "git commit*": ask
    "git push*": ask
    "git fetch*": ask
    "git pull*": ask
    "git merge*": ask
    "git rebase*": ask
    "git cherry-pick*": ask
    "git branch -d*": ask
    "git branch --delete*": ask
    "git branch -D*": ask
    "git worktree remove*": ask
    "git worktree prune*": ask
    "git reset*": deny
    "git restore*": deny
    "git clean*": deny
    "git status*": allow
    "git diff*": allow
  task:
    "*": deny
    "pmp-query": allow
    "pmp-plan": allow
    "pmp-implement": allow"#;

    const PERMISSION_QUERY: &'static str = r#"permission:
  read: allow
  glob: allow
  grep: allow
  edit: deny
  bash:
    "*": deny
    "git status*": allow
    "git diff*": allow
    "git log*": allow
    "git show*": allow
    "git branch --list*": allow
    "git branch --show-current": allow
    "git worktree list*": allow
  task: deny"#;

    const PERMISSION_PLAN: &'static str = r#"permission:
  read: allow
  glob: allow
  grep: allow
  edit: deny
  bash:
    "*": deny
    "git status*": allow
    "git diff*": allow
    "git log*": allow
    "git branch --list*": allow
    "git worktree list*": allow
  task: deny"#;

    const PERMISSION_IMPLEMENT: &'static str = r#"permission:
  read: allow
  glob: allow
  grep: allow
  edit: allow
  bash:
    "*": ask
    "cargo test*": allow
    "cargo fmt*": allow
    "cargo check": allow
    "cargo clippy*": allow
    "git *": allow
    "git commit*": ask
    "git push*": ask
    "git fetch*": ask
    "git pull*": ask
    "git merge*": ask
    "git rebase*": ask
    "git cherry-pick*": ask
    "git branch -d*": ask
    "git branch --delete*": ask
    "git branch -D*": ask
    "git worktree remove*": ask
    "git worktree prune*": ask
    "git reset*": deny
    "git restore*": deny
    "git clean*": deny
  task: deny"#;

    const METADATA: [(&'static str, &'static str, &'static str); 4] = [
        ("primary", "accent", Self::PERMISSION_ORCHESTRATOR),
        ("subagent", "info", Self::PERMISSION_QUERY),
        ("subagent", "warning", Self::PERMISSION_PLAN),
        ("subagent", "success", Self::PERMISSION_IMPLEMENT),
    ];

    fn render_frontmatter(description: &str, mode: &str, color: &str, permission: &str) -> String {
        format!(
            "---\ndescription: {}\nmode: {}\ncolor: {}\n{}\n---\n\n",
            description, mode, color, permission
        )
    }
}

impl TargetRenderer for OpenCodeRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        let base = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".opencode/agents"),
            Scope::Global => ctx.home_dir.join(".config/opencode/agents"),
        };
        render_agent_files(&base, ".md", &Self::METADATA, |template, meta| {
            let (mode, color, permission) = *meta;
            Self::render_frontmatter(&template.description, mode, color, permission)
        })
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        let path = match ctx.scope {
            Scope::Project => ctx.project_dir.join("opencode.json"),
            Scope::Global => ctx.home_dir.join(".config/opencode/opencode.json"),
        };
        mcp_write(
            path,
            "mcp",
            serde_json::json!({
                "type": "local",
                "command": ["pmp", "mcp"],
                "enabled": true,
            }),
        )
    }
}

pub struct CursorRenderer;

impl CursorRenderer {
    const READONLY: [bool; 4] = [false, true, false, false];
}

impl TargetRenderer for CursorRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        render_agent_files(
            &ctx.base_dir().join(".cursor/agents"),
            ".md",
            &Self::READONLY,
            |template, readonly| {
                markdown_frontmatter(&template.name, &template.description, None, *readonly)
            },
        )
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        let path = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".cursor/mcp.json"),
            Scope::Global => ctx.home_dir.join(".cursor/mcp.json"),
        };
        mcp_write(
            path,
            "mcpServers",
            serde_json::json!({
                "command": "pmp",
                "args": ["mcp"],
            }),
        )
    }
}

pub struct CopilotRenderer;

impl CopilotRenderer {
    const TOOLS: [Option<&'static str>; 4] = [
        Some("read_file, search, run_in_terminal, edit_file"),
        Some("read_file, search"),
        Some("read_file, search, run_in_terminal, edit_file"),
        Some("read_file, search, run_in_terminal, edit_file"),
    ];
}

impl TargetRenderer for CopilotRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        let base = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".github/agents"),
            Scope::Global => ctx.home_dir.join(".vscode/agents"),
        };
        render_agent_files(&base, ".agent.md", &Self::TOOLS, |template, tools| {
            markdown_frontmatter(&template.name, &template.description, *tools, false)
        })
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        let path = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".vscode/mcp.json"),
            Scope::Global => ctx.home_dir.join(".vscode/mcp.json"),
        };
        mcp_write(
            path,
            "servers",
            serde_json::json!({
                "type": "stdio",
                "command": "pmp",
                "args": ["mcp"],
            }),
        )
    }
}

pub struct CodexRenderer;

impl CodexRenderer {
    const SANDBOX: [bool; 4] = [false, true, false, false];
}

impl TargetRenderer for CodexRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        let base = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".codex/agents"),
            Scope::Global => ctx.home_dir.join(".codex/agents"),
        };
        let mut ops = Vec::with_capacity(AGENT_SOURCES.len());
        for (agent, &sandbox) in AGENT_SOURCES.iter().zip(Self::SANDBOX.iter()) {
            let template = Template::parse(agent.source)?;
            let mut content = format!(
                "name = \"{}\"\ndescription = \"{}\"\n",
                template.name, template.description
            );
            if sandbox {
                content.push_str("sandbox_mode = \"read-only\"\n");
            }
            content.push_str("developer_instructions = \"\"\"");
            content.push_str(&toml_escape(&template.body));
            content.push_str("\"\"\"\n");
            ops.push(WriteOp {
                path: base.join(format!("{}.toml", agent.stem)),
                content,
            });
        }
        Ok(ops)
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        let path = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".codex/config.toml"),
            Scope::Global => ctx.home_dir.join(".codex/config.toml"),
        };
        codex_mcp_write(path)
    }
}

fn codex_mcp_write(path: PathBuf) -> Result<McpAction> {
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let parsed: toml::Value = if existing.is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        existing.parse()?
    };
    if parsed
        .get("mcp_servers")
        .and_then(|v| v.get("pmp"))
        .is_some()
    {
        return Ok(McpAction::Write(WriteOp {
            path,
            content: existing,
        }));
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str("[mcp_servers.pmp]\ncommand = \"pmp\"\nargs = [\"mcp\"]\n");
    Ok(McpAction::Write(WriteOp { path, content }))
}

pub struct AntigravityRenderer;

impl AntigravityRenderer {
    const TOOLS: &'static str = "view_file, replace_file_content, run_command, manage_task";
    const ROLES: [Option<&'static str>; 4] = [
        Some("mainAgent: true"),
        Some("subagent: true"),
        Some("subagent: true"),
        Some("subagent: true"),
    ];
}

impl TargetRenderer for AntigravityRenderer {
    fn render(&self, ctx: &RenderContext) -> Result<Vec<WriteOp>> {
        let base = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".agents/agents"),
            Scope::Global => ctx.home_dir.join(".gemini/config/agents"),
        };
        render_agent_files(&base, ".md", &Self::ROLES, |template, role| {
            let mut extras = vec![format!("tools: {}", Self::TOOLS)];
            if let Some(r) = role {
                extras.push(r.to_string());
            }
            markdown_frontmatter_with_extras(&template.name, &template.description, &extras)
        })
    }

    fn mcp_action(&self, ctx: &RenderContext) -> Result<McpAction> {
        let path = match ctx.scope {
            Scope::Project => ctx.project_dir.join(".agents/mcp_config.json"),
            Scope::Global => ctx.home_dir.join(".gemini/config/mcp_config.json"),
        };
        mcp_write(
            path,
            "mcpServers",
            serde_json::json!({
                "command": "pmp",
                "args": ["mcp"],
            }),
        )
    }
}

pub fn renderer_for(target: IndividualTarget) -> Box<dyn TargetRenderer> {
    match target {
        IndividualTarget::Claude => Box::new(ClaudeRenderer),
        IndividualTarget::OpenCode => Box::new(OpenCodeRenderer),
        IndividualTarget::Cursor => Box::new(CursorRenderer),
        IndividualTarget::Copilot => Box::new(CopilotRenderer),
        IndividualTarget::Codex => Box::new(CodexRenderer),
        IndividualTarget::Antigravity => Box::new(AntigravityRenderer),
    }
}
