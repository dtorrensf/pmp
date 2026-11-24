// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use anyhow::Context;
use pmp::app::App;
use pmp::config::Config;
use pmp::db::sqlite::SqliteRepository;
const HELP: &str = include_str!("../help.txt");

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Composition root: files and environment are read once, then injected.
    let config = Config::load();
    match args.get(1).map(String::as_str) {
        Some("-v" | "--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => println!("{}", HELP),
        Some("mcp") => pmp::mcp::serve(open_repo(&config)?, config.mcp.timeout).await?,
        Some(arg) => {
            eprintln!("error: unknown argument '{}'\n\n{}", arg, HELP);
            std::process::exit(1);
        }
        None => {
            let mut app = App::new(open_repo(&config)?);
            app.set_pending_update(pmp::update::spawn_check(env!("CARGO_PKG_VERSION")));
            pmp::tui::run(app, &config.appearance)?
        }
    }
    Ok(())
}

fn open_repo(config: &Config) -> anyhow::Result<Arc<SqliteRepository>> {
    let data_dir = dirs::data_dir().context("locate data directory")?;
    let db_path = data_dir.join("pmp").join("pmp.db");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    Ok(Arc::new(SqliteRepository::open_with(
        &db_path,
        config.database.busy,
    )?))
}
