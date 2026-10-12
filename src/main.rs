// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

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
        None => pmp::tui::run(App::new(open_repo()?))?,
    }
    Ok(())
}

fn open_repo() -> anyhow::Result<Arc<SqliteRepository>> {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let db_path = data_dir.join("pmp").join("pmp.db");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    Ok(Arc::new(SqliteRepository::open(&db_path)?))
}
