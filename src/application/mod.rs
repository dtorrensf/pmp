// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application layer: the use cases shared by the TUI, the MCP server and,
//! from 1.0.0, the ACP client.
//!
//! Each user-visible operation lives here exactly once, as orchestration over
//! the [`Repository`](crate::db::repository::Repository) port. There is no
//! `ratatui` and no `rmcp` in this module, so the same code can later run
//! behind the REST core service planned for 1.5.0.
//!
//! Use case inputs and outputs derive `serde` on purpose: they are the natural
//! wire contract for that service. Channels keep their own presentation, the
//! TUI turning [`AppError`](crate::domain::error::AppError) into toasts and MCP
//! into tool errors.

pub mod dependencies;
pub mod export;
pub mod notes;
pub mod pagination;
pub mod projects;
pub mod prompts;
pub mod records;
pub mod runs;
pub mod summaries;
pub mod tasks;
pub mod versions;

#[cfg(test)]
mod architecture_tests {
    /// The application layer is the seam for the 1.5.0 core service: it must
    /// stay free of presentation (`ratatui`, `crate::screens`) and transport
    /// (`rmcp`, `crate::mcp`), and must not reach back into the TUI `App`.
    #[test]
    fn application_layer_is_free_of_presentation_and_transport() {
        let files: [(&str, &str); 11] = [
            ("export.rs", include_str!("export.rs")),
            ("projects.rs", include_str!("projects.rs")),
            ("versions.rs", include_str!("versions.rs")),
            ("tasks.rs", include_str!("tasks.rs")),
            ("dependencies.rs", include_str!("dependencies.rs")),
            ("notes.rs", include_str!("notes.rs")),
            ("pagination.rs", include_str!("pagination.rs")),
            ("summaries.rs", include_str!("summaries.rs")),
            ("records.rs", include_str!("records.rs")),
            ("prompts.rs", include_str!("prompts.rs")),
            ("runs.rs", include_str!("runs.rs")),
        ];
        let forbidden = [
            "ratatui",
            "rmcp",
            "crate::screens",
            "crate::mcp",
            "crate::app",
        ];
        for (name, source) in files {
            for token in forbidden {
                assert!(
                    !source.contains(token),
                    "{name} must not reference {token}: the application layer only \
                     orchestrates the repository port"
                );
            }
        }
    }
}
