# PMP AI agent integration

PMP ships with a four-agent pipeline that uses the built-in `pmp mcp` server as the single source of truth for projects, tasks, dependencies, and status. The `pmp agents install` command renders the agents and wires the MCP server for the tool you already use.

## The four agents

| Agent | Role | Default restrictions |
|-------|------|----------------------|
| `pmp-orchestrator` | Coordinates task discovery, decomposition, implementation, and quality gates. | Delegates to the other three agents; does not edit code directly. |
| `pmp-query` | Reads PMP projects, tasks, dependencies, and repository code without making changes. | Read-only tools only; no edit, no task creation, no state changes. |
| `pmp-plan` | Converts a goal into atomic PMP tasks and persists an acyclic dependency graph. | Creates tasks and dependencies; does not edit source files. |
| `pmp-implement` | Implements exactly one assigned PMP task and reports code and test evidence. | Edits files for its one task only; does not create tasks or mark them done. |

The orchestrator decides what to do, asks `pmp-query` for facts, asks `pmp-plan` to create missing tasks and dependencies, and assigns one ready task at a time to `pmp-implement`. Each agent returns a structured report so the orchestrator can verify evidence before marking work done.

## Prerequisites

- The `pmp` binary must be on your `PATH` so every target can run `pmp mcp`.
- The MCP server is started by each tool automatically; you do not need to run `pmp mcp` yourself.

## Quick start

Install all six target configurations into the current project at once:

```bash
pmp agents install --target all --scope project
```

Run with `--dry-run` first to see every file that would be written and every MCP config that would be merged:

```bash
pmp agents install --target all --scope project --dry-run
```

## Scope: project vs global

- `--scope project` (default) writes agent files and MCP configuration into the current project directory. Commit the generated files so teammates get the same setup.
- `--scope global` writes agent files and MCP configuration into your home directory. This makes the PMP agents available in every project you open with that tool, but it only configures the tool itself; the `pmp` binary and MCP server still operate on whichever repository is open.

Global paths are resolved with `HOME` on macOS/Linux and `USERPROFILE` on Windows.

## Per-target installation details

Each target writes four agent files plus an MCP server entry for `pmp mcp`. The `pmp` server uses the `command` `pmp` and the argument `mcp` in every target.

### Claude

Files written by `pmp agents install --target claude`:

- Project scope: `.claude/agents/pmp-orchestrator.md`, `.claude/agents/pmp-query.md`, `.claude/agents/pmp-plan.md`, `.claude/agents/pmp-implement.md`
- Project MCP config: `.mcp.json`
- Global scope: `~/.claude/agents/*.md`
- Global MCP wiring: prints `claude mcp add -s user pmp -- pmp mcp`

`pmp-query.md` receives a `tools: Read, Grep, Glob, WebFetch` line in its frontmatter; the other agents do not receive a tools line.

The merged `.mcp.json` looks like this (existing keys are preserved):

```json
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
```

#### Manual install for Claude

The installer renders the agent files with Claude-specific frontmatter. The manual equivalent is to run `pmp agents install --target claude --dry-run` once, copy the rendered `.claude/agents/*.md` files into place, and merge the MCP config:

```bash
mkdir -p .claude/agents
# Create the four agent files with the rendered frontmatter (see dry-run output for exact content).
# Create or merge .mcp.json
if [ -f .mcp.json ]; then
  jq '.mcpServers.pmp = {"command":"pmp","args":["mcp"]}' .mcp.json > .mcp.json.tmp && mv .mcp.json.tmp .mcp.json
else
  cat > .mcp.json <<'EOF'
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
EOF
fi
```

### OpenCode

Files written by `pmp agents install --target opencode`:

- Project scope: `.opencode/agents/pmp-orchestrator.md`, `.opencode/agents/pmp-query.md`, `.opencode/agents/pmp-plan.md`, `.opencode/agents/pmp-implement.md`
- Project MCP config: `opencode.json`
- Global scope: `~/.config/opencode/agents/*.md`
- Global MCP config: `~/.config/opencode/opencode.json`

The orchestrator is rendered with `mode: primary`; the other three agents are rendered with `mode: subagent`. Each file includes an OpenCode `permission:` block. `pmp-query` denies edit tools; `pmp-implement` allows them.

The merged `opencode.json` looks like this (existing keys, including `default_agent`, are preserved):

```json
{
  "mcp": {
    "pmp": {
      "type": "local",
      "command": ["pmp", "mcp"],
      "enabled": true
    }
  }
}
```

#### Manual install for OpenCode

The installer renders the agent files with OpenCode-specific frontmatter. The manual equivalent is to run `pmp agents install --target opencode --dry-run` once, copy the rendered `.opencode/agents/*.md` files into place, and merge the MCP config:

```bash
mkdir -p .opencode/agents
# Create the four agent files with the rendered frontmatter (see dry-run output for exact content).
# Create or merge opencode.json
if [ -f opencode.json ]; then
  jq '.mcp.pmp = {"type":"local","command":["pmp","mcp"],"enabled":true}' opencode.json > opencode.json.tmp && mv opencode.json.tmp opencode.json
else
  cat > opencode.json <<'EOF'
{
  "mcp": {
    "pmp": {
      "type": "local",
      "command": ["pmp", "mcp"],
      "enabled": true
    }
  }
}
EOF
fi
```

### Cursor

Files written by `pmp agents install --target cursor`:

- Project scope: `.cursor/agents/pmp-orchestrator.md`, `.cursor/agents/pmp-query.md`, `.cursor/agents/pmp-plan.md`, `.cursor/agents/pmp-implement.md`
- Project MCP config: `.cursor/mcp.json`
- Global scope: `~/.cursor/agents/*.md` and `~/.cursor/mcp.json`

`pmp-query.md` receives `readonly: true` in its frontmatter; the other agents do not.

The merged `.cursor/mcp.json` uses the `mcpServers` root key:

```json
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
```

#### Manual install for Cursor

The installer renders the agent files with Cursor-specific frontmatter. The manual equivalent is to run `pmp agents install --target cursor --dry-run` once, copy the rendered `.cursor/agents/*.md` files into place, and merge the MCP config:

```bash
mkdir -p .cursor/agents
# Create the four agent files with the rendered frontmatter (see dry-run output for exact content).
# pmp-query.md needs "readonly: true" in its frontmatter.
if [ -f .cursor/mcp.json ]; then
  jq '.mcpServers.pmp = {"command":"pmp","args":["mcp"]}' .cursor/mcp.json > .cursor/mcp.json.tmp && mv .cursor/mcp.json.tmp .cursor/mcp.json
else
  cat > .cursor/mcp.json <<'EOF'
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
EOF
fi
```

### Copilot (VS Code)

Files written by `pmp agents install --target copilot`:

- Project scope: `.github/agents/pmp-orchestrator.agent.md`, `.github/agents/pmp-query.agent.md`, `.github/agents/pmp-plan.agent.md`, `.github/agents/pmp-implement.agent.md`
- Project MCP config: `.vscode/mcp.json`
- Global scope: `~/.vscode/agents/*.agent.md`
- Global MCP config: `~/.vscode/mcp.json`

Each agent file receives a `tools:` line with VS Code tool names. `pmp-query.agent.md` only lists `read_file, search`; the other agents also include `run_in_terminal, edit_file`.

The merged `.vscode/mcp.json` uses the `servers` root key (not `mcpServers`):

```json
{
  "servers": {
    "pmp": {
      "type": "stdio",
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
```

#### Manual install for Copilot

The installer renders the agent files with Copilot-specific frontmatter and the `.agent.md` suffix. The manual equivalent is to run `pmp agents install --target copilot --dry-run` once, copy the rendered `.github/agents/*.agent.md` files into place, and merge the MCP config:

```bash
mkdir -p .github/agents
# Create the four agent files with the rendered frontmatter (see dry-run output for exact content).
# pmp-query.agent.md only lists read_file, search; the others also include run_in_terminal, edit_file.
if [ -f .vscode/mcp.json ]; then
  jq '.servers.pmp = {"type":"stdio","command":"pmp","args":["mcp"]}' .vscode/mcp.json > .vscode/mcp.json.tmp && mv .vscode/mcp.json.tmp .vscode/mcp.json
else
  mkdir -p .vscode
  cat > .vscode/mcp.json <<'EOF'
{
  "servers": {
    "pmp": {
      "type": "stdio",
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
EOF
fi
```

### Codex

Files written by `pmp agents install --target codex`:

- Project scope: `.codex/agents/pmp-orchestrator.toml`, `.codex/agents/pmp-query.toml`, `.codex/agents/pmp-plan.toml`, `.codex/agents/pmp-implement.toml`
- Project MCP config: `.codex/config.toml`
- Global scope: `~/.codex/agents/*.toml` and `~/.codex/config.toml`

Codex agents are rendered as TOML with `name`, `description`, `developer_instructions`, and (for `pmp-query.toml` only) `sandbox_mode = "read-only"`. The `developer_instructions` value is the agent template body escaped for a TOML multi-line basic string.

The installer appends the following block to `.codex/config.toml`, skipping it if `[mcp_servers.pmp]` already exists so the operation stays idempotent:

```toml
[mcp_servers.pmp]
command = "pmp"
args = ["mcp"]
```

#### Manual install for Codex

```bash
mkdir -p .codex/agents
# Create one TOML file per agent. Example for pmp-orchestrator.toml:
cat > .codex/agents/pmp-orchestrator.toml <<'EOF'
name = "pmp-orchestrator"
description = "Coordinates PMP task discovery, decomposition, implementation, and quality gates for this repository."
developer_instructions = """
# paste the body of agents/pmp-orchestrator.md here, escaping \\ and " for TOML
"""
EOF
# For pmp-query.toml add: sandbox_mode = "read-only"
# Append the MCP server block only if it is not already present
if ! grep -q '^\[mcp_servers\.pmp\]' .codex/config.toml 2>/dev/null; then
  cat >> .codex/config.toml <<'EOF'

[mcp_servers.pmp]
command = "pmp"
args = ["mcp"]
EOF
fi
```

### Antigravity

Files written by `pmp agents install --target antigravity`:

- Project scope: `.agents/agents/pmp-orchestrator.md`, `.agents/agents/pmp-query.md`, `.agents/agents/pmp-plan.md`, `.agents/agents/pmp-implement.md`
- Project MCP config: `.agents/mcp_config.json`
- Global scope: `~/.gemini/config/agents/*.md` and `~/.gemini/config/mcp_config.json`

Each agent file receives the Antigravity tool line `tools: view_file, replace_file_content, run_command, manage_task`. The orchestrator receives `mainAgent: true`; the other three receive `subagent: true`.

The merged `.agents/mcp_config.json` uses the `mcpServers` root key:

```json
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
```

> **Note:** The Antigravity IDE (v2.0 / CLI) currently falls back to `AGENTS.md` discovery. The files above follow the documented layout; if your IDE version only reads `AGENTS.md`, place the orchestrator instructions at the repository root in an `AGENTS.md` file and reference the agents under `.agents/agents/`.

#### Manual install for Antigravity

The installer renders the agent files with Antigravity-specific frontmatter. The manual equivalent is to run `pmp agents install --target antigravity --dry-run` once, copy the rendered `.agents/agents/*.md` files into place, and merge the MCP config:

```bash
mkdir -p .agents/agents
# Create the four agent files with the rendered frontmatter (see dry-run output for exact content).
# Add tools: view_file, replace_file_content, run_command, manage_task and mainAgent/subagent roles.
if [ -f .agents/mcp_config.json ]; then
  jq '.mcpServers.pmp = {"command":"pmp","args":["mcp"]}' .agents/mcp_config.json > .agents/mcp_config.json.tmp && mv .agents/mcp_config.json.tmp .agents/mcp_config.json
else
  cat > .agents/mcp_config.json <<'EOF'
{
  "mcpServers": {
    "pmp": {
      "command": "pmp",
      "args": ["mcp"]
    }
  }
}
EOF
fi
```

## Installer reference

```
pmp agents install --target <claude|opencode|codex|antigravity|cursor|copilot|all>
                 [--scope project|global]
                 [--project-dir PATH]
                 [--dry-run]
```

- `--target` is required. Use `all` to install every target into the same project.
- `--scope` defaults to `project`.
- `--project-dir` defaults to the current working directory.
- `--dry-run` prints the planned files and MCP merges without writing anything.

The installer is idempotent: running it twice produces the same output and does not duplicate JSON keys or TOML sections.
