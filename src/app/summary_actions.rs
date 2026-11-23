// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Version summary use cases: exporting a compacted summary to Markdown.
use anyhow::Result;

use crate::db::repository::Repository;

use super::outcome::Outcome;

pub(super) fn export(repo: &dyn Repository, summary_id: i64) -> Result<Outcome> {
    let path = crate::mcp::export::export_version_summary_markdown(repo, summary_id)?;
    Ok(Outcome::confirmed(
        format!("Exported to {}", path.display()),
        None,
    ))
}
