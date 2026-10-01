// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod app;
pub mod db;
pub mod domain;
pub mod screens;
pub mod tui;

pub use db::repository;

pub mod agents;
pub mod mcp;

#[cfg(test)]
mod tests {
    #[test]
    #[allow(unused_imports)]
    fn lib_reexports_public_modules() {
        use crate::{app, db, domain, mcp, repository, screens, tui};
    }
}
