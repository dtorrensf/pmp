// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `pmp snapshot`: export and import a portable JSON dump of the whole store
//! (A8).
//!
//! The command never starts the TUI or the MCP server. Like `pmp config`, it
//! keeps one invocation in a value ([`Outcome`]) so the whole command can be
//! tested without spawning a process.
//!
//! Files are written atomically (temporary file, fsync, rename), so an
//! interrupted export never leaves a truncated snapshot that looks valid.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::application::snapshot as use_case;
use crate::config::Config;
use crate::db::repository::Repository;
use crate::db::sqlite::SqliteRepository;
use crate::domain::error::AppError;
use crate::domain::timestamp::now_utc;
use crate::paths::Paths;

/// Help printed by `pmp snapshot`, `pmp snapshot -h` and unknown subcommands.
pub const HELP: &str = include_str!("snapshot_cli/help.txt");

/// Exit code and streams of one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    fn ok(stdout: impl Into<String>) -> Self {
        Self {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn failure(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: format!("{}\n", message.into()),
        }
    }
}

/// Production entry point: resolves the configuration and the store, runs the
/// command and prints the outcome.
///
/// Help and unknown subcommands are answered before the store is opened, so
/// `pmp snapshot --help` never migrates or backs up the database.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            return 0;
        }
        Some("export" | "import") => {}
        Some(other) => {
            let outcome = Outcome::failure(format!("unknown snapshot command '{other}'\n\n{HELP}"));
            eprint!("{}", outcome.stderr);
            return outcome.code;
        }
    }
    let config = Config::load();
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("pmp snapshot: {error}");
            return 1;
        }
    };
    if let Err(error) = paths.ensure_data_dir() {
        eprintln!("pmp snapshot: create data directory: {error}");
        return 1;
    }
    let repo = match SqliteRepository::open_with(&paths.db_path, config.database.busy) {
        Ok(repo) => repo,
        Err(error) => {
            eprintln!("pmp snapshot: {error}");
            return 1;
        }
    };
    let exports = paths.data_dir.join("exports");
    let outcome = run_with(&repo, &exports, args);
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    outcome.code
}

/// Runs one `pmp snapshot` invocation against `repo`. Exports default to
/// `exports_dir`, which tests point at a temporary directory.
pub fn run_with(repo: &dyn Repository, exports_dir: &Path, args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => Outcome::ok(HELP),
        Some("export") => export_command(repo, exports_dir, &args[1..]),
        Some("import") => import_command(repo, &args[1..]),
        Some(other) => Outcome::failure(format!("unknown snapshot command '{other}'\n\n{HELP}")),
    }
}

fn export_command(repo: &dyn Repository, exports_dir: &Path, args: &[String]) -> Outcome {
    let mut output: Option<PathBuf> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--output" | "-o" => match rest.next() {
                Some(path) => output = Some(PathBuf::from(path)),
                None => {
                    return Outcome::failure(
                        "--output needs a path\n\nUsage: pmp snapshot export [--output <path>]",
                    );
                }
            },
            other => {
                return Outcome::failure(format!(
                    "unknown export option '{other}'\n\nUsage: pmp snapshot export [--output <path>]"
                ));
            }
        }
    }
    let target = output.unwrap_or_else(|| exports_dir.join(default_file_name()));
    let result = use_case::export(repo).and_then(|snapshot| {
        let json = use_case::render(&snapshot)?;
        write_atomically(&target, &json)?;
        Ok((snapshot.counts(), json.len()))
    });
    match result {
        Ok((counts, bytes)) => Outcome::ok(format!(
            "snapshot exported: {} ({} bytes)\n  {}\n",
            target.display(),
            bytes,
            describe(&counts)
        )),
        Err(error) => Outcome::failure(format!("cannot export the snapshot: {error}")),
    }
}

fn import_command(repo: &dyn Repository, args: &[String]) -> Outcome {
    let mut path: Option<PathBuf> = None;
    let mut replace = false;
    for arg in args {
        match arg.as_str() {
            "--replace" => replace = true,
            "-h" | "--help" => return Outcome::ok(HELP),
            option if option.starts_with('-') => {
                return Outcome::failure(format!(
                    "unknown import option '{option}'\n\nUsage: pmp snapshot import <path> [--replace]"
                ));
            }
            file => {
                if path.is_some() {
                    return Outcome::failure(
                        "import takes exactly one snapshot path\n\nUsage: pmp snapshot import <path> [--replace]",
                    );
                }
                path = Some(PathBuf::from(file));
            }
        }
    }
    let Some(path) = path else {
        return Outcome::failure(
            "missing snapshot path\n\nUsage: pmp snapshot import <path> [--replace]",
        );
    };
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) => {
            return Outcome::failure(format!(
                "cannot read the snapshot {}: {error}",
                path.display()
            ));
        }
    };
    match use_case::import(repo, &json, replace) {
        Ok(counts) => Outcome::ok(format!(
            "snapshot imported: {}\n  {}\n",
            path.display(),
            describe(&counts)
        )),
        Err(error) => Outcome::failure(format!("cannot import the snapshot: {error}")),
    }
}

fn describe(counts: &crate::domain::snapshot::SnapshotCounts) -> String {
    format!(
        "{} projects, {} versions, {} tasks, {} notes, {} dependencies, {} summaries",
        counts.projects,
        counts.versions,
        counts.tasks,
        counts.notes,
        counts.dependencies,
        counts.summaries
    )
}

/// Unique, sortable default file name: `pmp-snapshot-<YYYYMMDDHHMMSS>.json`.
fn default_file_name() -> String {
    let stamp: String = now_utc().chars().filter(|ch| ch.is_ascii_digit()).collect();
    format!("pmp-snapshot-{stamp}.json")
}

/// Writes `contents` to `path` through a temporary file in the same directory,
/// flushing before the rename, so the destination is never half written.
fn write_atomically(path: &Path, contents: &str) -> Result<(), AppError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| {
        AppError::Validation(format!(
            "cannot create the output directory {}: {error}",
            parent.display()
        ))
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "snapshot.json".to_string());
    let tmp = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|error| {
        AppError::Validation(format!("cannot write {}: {error}", tmp.display()))
    })?;
    if let Ok(file) = File::open(&tmp) {
        let _ = file.sync_all();
    }
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        AppError::Validation(format!("cannot write {}: {error}", path.display()))
    })
}

#[cfg(test)]
mod tests;
