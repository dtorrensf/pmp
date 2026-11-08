// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use anyhow::Context;

use pmp::app::App;
use pmp::db::sqlite::SqliteRepository;

const HELP: &str = include_str!("../help.txt");

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("-v" | "--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help") => println!("{}", HELP),
        Some("mcp") => pmp::mcp::serve(open_repo()?).await?,
        Some(arg) => {
            eprintln!("error: unknown argument '{}'\n\n{}", arg, HELP);
            std::process::exit(1);
        }
        None => {
            let mut app = App::new(open_repo()?);
            app.set_pending_update(pmp::update::spawn_check(env!("CARGO_PKG_VERSION")));
            pmp::tui::run(app)?
        }
    }
    Ok(())
}

fn open_repo() -> anyhow::Result<Arc<SqliteRepository>> {
    let data_dir = match dirs::data_dir() {
        Some(dir) => dir,
        None => std::env::current_dir().context("locate data directory")?,
    };
    let db_path = data_dir.join("pmp").join("pmp.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Arc::new(SqliteRepository::open(&db_path)?))
}
