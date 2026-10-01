// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use pmp::app::App;
use pmp::db::sqlite::SqliteRepository;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let db_path = data_dir.join("pmp").join("pmp.db");
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let repo = Arc::new(SqliteRepository::open(&db_path)?);
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "mcp").unwrap_or(false) {
        pmp::mcp::serve(repo).await?;
    } else if args.get(1).map(|s| s == "agents").unwrap_or(false) {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        pmp::agents::run(&refs[1..])?;
    } else {
        let app = App::new(repo);
        pmp::tui::run(app)?;
    }
    Ok(())
}
