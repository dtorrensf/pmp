// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Filesystem locations, resolved once from the XDG directories.
//!
//! One [`Paths`] value is built in the composition root and passed to the
//! repository, the logger and `pmp doctor`, so every component agrees on where
//! the configuration, the database and the logs live.

use std::path::PathBuf;

use anyhow::Context;

use crate::config::CONFIG_FILE;

/// Every path PMP reads or writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `~/.config/pmp/config.toml`; `None` when no config directory is known.
    pub config_file: Option<PathBuf>,
    /// `~/.local/share/pmp`, holding the database and the logs.
    pub data_dir: PathBuf,
    /// `~/.local/share/pmp/logs`, where the structured logs are written.
    pub logs_dir: PathBuf,
    /// `~/.local/share/pmp/pmp.db`, the shared SQLite database.
    pub db_path: PathBuf,
}

impl Paths {
    /// Resolves the XDG directories for this process.
    pub fn discover() -> anyhow::Result<Self> {
        let data_dir = dirs::data_dir()
            .context("locate data directory")?
            .join("pmp");
        Ok(Self {
            config_file: dirs::config_dir().map(|dir| dir.join(CONFIG_FILE)),
            logs_dir: data_dir.join("logs"),
            db_path: data_dir.join("pmp.db"),
            data_dir,
        })
    }

    /// Creates the data directory so the database and the logs can be created.
    pub fn ensure_data_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)
    }

    /// Paths rooted at `root`, for tests and sandboxes.
    #[cfg(test)]
    pub fn in_root(root: &std::path::Path) -> Self {
        let data_dir = root.join("data").join("pmp");
        Self {
            config_file: Some(root.join("config").join(CONFIG_FILE)),
            logs_dir: data_dir.join("logs"),
            db_path: data_dir.join("pmp.db"),
            data_dir,
        }
    }
}
