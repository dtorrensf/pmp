// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Claude,
    OpenCode,
    Cursor,
    Copilot,
    Codex,
    Antigravity,
    All,
}

impl Target {
    pub fn individual_targets() -> &'static [Target] {
        &[
            Target::Claude,
            Target::OpenCode,
            Target::Cursor,
            Target::Copilot,
            Target::Codex,
            Target::Antigravity,
        ]
    }
}

/// A target that can actually render files, i.e. every [`Target`] except the
/// `All` meta-target. Keeps `renderer_for` total: no panic arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndividualTarget {
    Claude,
    OpenCode,
    Cursor,
    Copilot,
    Codex,
    Antigravity,
}

impl From<IndividualTarget> for Target {
    fn from(target: IndividualTarget) -> Self {
        match target {
            IndividualTarget::Claude => Target::Claude,
            IndividualTarget::OpenCode => Target::OpenCode,
            IndividualTarget::Cursor => Target::Cursor,
            IndividualTarget::Copilot => Target::Copilot,
            IndividualTarget::Codex => Target::Codex,
            IndividualTarget::Antigravity => Target::Antigravity,
        }
    }
}

impl TryFrom<Target> for IndividualTarget {
    type Error = anyhow::Error;

    fn try_from(target: Target) -> Result<Self> {
        match target {
            Target::Claude => Ok(IndividualTarget::Claude),
            Target::OpenCode => Ok(IndividualTarget::OpenCode),
            Target::Cursor => Ok(IndividualTarget::Cursor),
            Target::Copilot => Ok(IndividualTarget::Copilot),
            Target::Codex => Ok(IndividualTarget::Codex),
            Target::Antigravity => Ok(IndividualTarget::Antigravity),
            Target::All => Err(anyhow!(
                "All is a meta-target and has no dedicated renderer"
            )),
        }
    }
}

impl FromStr for Target {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "claude" => Ok(Target::Claude),
            "opencode" => Ok(Target::OpenCode),
            "cursor" => Ok(Target::Cursor),
            "copilot" => Ok(Target::Copilot),
            "codex" => Ok(Target::Codex),
            "antigravity" => Ok(Target::Antigravity),
            "all" => Ok(Target::All),
            _ => Err(anyhow!("unsupported target: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    Global,
}

impl FromStr for Scope {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "project" => Ok(Scope::Project),
            "global" => Ok(Scope::Global),
            _ => Err(anyhow!("unsupported scope: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallArgs {
    pub target: Target,
    pub scope: Scope,
    pub project_dir: PathBuf,
    pub dry_run: bool,
}

impl InstallArgs {
    pub fn parse_args(args: &[&str]) -> Result<Self> {
        let mut target = None;
        let mut scope = Scope::Project;
        let mut project_dir = std::env::current_dir()?;
        let mut dry_run = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "agents" | "install" => {}
                "--target" => {
                    i += 1;
                    target = Some(args[i].parse()?);
                }
                "--scope" => {
                    i += 1;
                    scope = args[i].parse()?;
                }
                "--project-dir" => {
                    i += 1;
                    project_dir = PathBuf::from(args[i]);
                }
                "--dry-run" => dry_run = true,
                other => return Err(anyhow!("unknown argument: {other}")),
            }
            i += 1;
        }

        let target = target.ok_or_else(|| anyhow!("--target is required"))?;
        Ok(Self {
            target,
            scope,
            project_dir,
            dry_run,
        })
    }
}
