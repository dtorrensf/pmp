// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use super::{McpAction, WriteOp};

fn merge_mcp_json(
    path: &Path,
    root_key: &str,
    server_name: &str,
    server_config: serde_json::Value,
) -> Result<WriteOp> {
    let mut root: serde_json::Value = if path.exists() {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text)?
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be an object", path.display()))?;
    let servers = root_obj
        .entry(root_key)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| anyhow!("{root_key} must be an object"))?;
    servers_obj.insert(server_name.to_string(), server_config);

    let content = serde_json::to_string_pretty(&root)? + "\n";
    Ok(WriteOp {
        path: path.to_path_buf(),
        content,
    })
}

pub fn mcp_write(
    path: PathBuf,
    root_key: &str,
    server_config: serde_json::Value,
) -> Result<McpAction> {
    Ok(McpAction::Write(merge_mcp_json(
        &path,
        root_key,
        "pmp",
        server_config,
    )?))
}
