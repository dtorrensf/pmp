// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use anyhow::Context;
use pmp::app::App;
use pmp::config::Config;
use pmp::db::sqlite::SqliteRepository;
use pmp::paths::Paths;
const HELP: &str = include_str!("../help.txt");

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Composition root: files and environment are read once, then injected.
    let config = Config::load();
    match args.get(1).map(String::as_str) {
        Some("-v" | "--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => println!("{}", HELP),
        Some("doctor") => std::process::exit(pmp::doctor::run(&config)),
        Some("mcp") => {
            let paths = Paths::discover()?;
            pmp::logging::init(&config.logging, &paths.logs_dir);
            pmp::mcp::serve(open_repo(&config, &paths)?, config.mcp.timeout).await?
        }
        Some(arg) => {
            eprintln!("error: unknown argument '{}'\n\n{}", arg, HELP);
            std::process::exit(1);
        }
        None => {
            let paths = Paths::discover()?;
            pmp::logging::init(&config.logging, &paths.logs_dir);
            let mut app = App::new(open_repo(&config, &paths)?);
            app.set_pending_update(pmp::update::spawn_check(env!("CARGO_PKG_VERSION")));
            pmp::tui::run(app, &config).await?
        }
    }
    Ok(())
}

fn open_repo(config: &Config, paths: &Paths) -> anyhow::Result<Arc<SqliteRepository>> {
    paths.ensure_data_dir().context("create data directory")?;
    Ok(Arc::new(SqliteRepository::open_with(
        &paths.db_path,
        config.database.busy,
    )?))
}
