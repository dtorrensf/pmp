use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::args::{InstallArgs, Target};
use super::renderers::{RenderContext, renderer_for};
use super::{McpAction, WriteOp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub writes: Vec<WriteOp>,
    pub mcp: McpAction,
}

pub fn plan(ctx: &RenderContext) -> Result<InstallPlan> {
    let renderer = renderer_for(ctx.target.try_into()?);
    let writes = renderer.render(ctx)?;
    let mcp = renderer.mcp_action(ctx)?;
    Ok(InstallPlan { writes, mcp })
}

pub fn plans(ctx: &RenderContext) -> Result<Vec<InstallPlan>> {
    match ctx.target {
        Target::All => Target::individual_targets()
            .iter()
            .map(|&target| {
                let mut single_ctx = ctx.clone();
                single_ctx.target = target;
                plan(&single_ctx)
            })
            .collect(),
        _ => Ok(vec![plan(ctx)?]),
    }
}

pub(crate) fn write_op(op: &WriteOp) -> Result<()> {
    if let Some(parent) = op.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&op.path, &op.content)?;
    Ok(())
}

pub fn execute_plan(ctx: &RenderContext, plan: &InstallPlan) -> Result<()> {
    if ctx.dry_run {
        return Ok(());
    }

    for op in &plan.writes {
        write_op(op)?;
    }

    if let McpAction::Write(op) = &plan.mcp {
        write_op(op)?;
    }

    Ok(())
}

fn resolve_home_dir() -> Result<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(key)
        .map(PathBuf::from)
        .or_else(|_| dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory")))
}

pub fn run(args: &[&str]) -> Result<()> {
    let args = InstallArgs::parse_args(args)?;
    let home_dir = resolve_home_dir()?;
    let ctx = RenderContext::from_args(&args, home_dir);
    let plans = plans(&ctx)?;

    if ctx.dry_run {
        println!("Dry-run mode. Planned writes:");
        for plan in &plans {
            for op in &plan.writes {
                println!("{} ({} bytes)", op.path.display(), op.content.len());
            }
            match &plan.mcp {
                McpAction::Write(op) => {
                    println!("{} ({} bytes)", op.path.display(), op.content.len())
                }
                McpAction::Guidance(cmd) => println!("MCP wiring (run manually): {cmd}"),
            }
        }
        return Ok(());
    }

    for plan in &plans {
        execute_plan(&ctx, plan)?;
        if let McpAction::Guidance(cmd) = &plan.mcp {
            println!("{cmd}");
        }
    }
    Ok(())
}
