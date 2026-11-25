// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod acp;
pub mod app;
pub mod application;
pub mod config;
pub mod db;
pub mod doctor;
pub mod domain;
pub mod logging;
pub mod paths;
pub mod screens;
pub mod tui;
pub mod update;

pub use db::repository;

pub mod mcp;

#[cfg(test)]
mod tests {
    #[test]
    #[allow(unused_imports)]
    fn lib_reexports_public_modules() {
        use crate::{
            acp, app, application, db, doctor, domain, logging, mcp, paths, repository, screens,
            tui,
        };
    }
}
