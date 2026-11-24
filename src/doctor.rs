// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `pmp doctor`: a one-shot report of the environment PMP runs in.
//!
//! The command answers the questions that usually explain a broken start: where
//! the configuration, the database and the logs live, whether they are
//! writable, what SQLite reports for the shared file and which agent binaries
//! the configured profiles can actually launch. It only reads: it neither
//! migrates the database nor creates logs.
//!
//! The report is assembled into a [`Report`] and rendered separately, so the
//! formatting and the individual statuses are tested without touching the real
//! user directories.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::config::Config;
use crate::logging;
use crate::paths::Paths;

/// Severity of one diagnostic line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

/// One `label: detail` line with its severity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub status: Status,
    pub label: String,
    pub detail: String,
}

impl Check {
    pub fn ok(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(Status::Ok, label, detail)
    }

    pub fn warn(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(Status::Warn, label, detail)
    }

    pub fn fail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(Status::Fail, label, detail)
    }

    fn new(status: Status, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status,
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// A titled group of checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub checks: Vec<Check>,
}

impl Section {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            checks: Vec::new(),
        }
    }

    pub fn with(mut self, check: Check) -> Self {
        self.checks.push(check);
        self
    }
}

/// The whole diagnostic, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub version: String,
    pub sections: Vec<Section>,
}

impl Report {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            sections: Vec::new(),
        }
    }

    pub fn push(&mut self, section: Section) {
        self.sections.push(section);
    }

    /// `(ok, warnings, failures)` across every section.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for check in self.sections.iter().flat_map(|section| &section.checks) {
            match check.status {
                Status::Ok => counts.0 += 1,
                Status::Warn => counts.1 += 1,
                Status::Fail => counts.2 += 1,
            }
        }
        counts
    }

    pub fn has_failures(&self) -> bool {
        self.counts().2 > 0
    }

    /// Human-readable text: a header, one indented block per section and a
    /// summary. This is the only place that knows the layout.
    pub fn render(&self) -> String {
        let mut out = format!("pmp doctor {}\n", self.version);
        for section in &self.sections {
            out.push_str(&format!("\n{}\n", section.title));
            for check in &section.checks {
                out.push_str(&format!(
                    "  {:<6}{}: {}\n",
                    check.status.tag(),
                    check.label,
                    check.detail
                ));
            }
        }
        let (ok, warn, fail) = self.counts();
        out.push_str(&format!("\nsummary: {ok} ok, {warn} warn, {fail} fail\n"));
        out
    }
}

/// Prints the report and returns the process exit code: `1` when a check
/// failed, `0` otherwise, `2` when even the paths cannot be resolved.
pub fn run(config: &Config) -> i32 {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            println!("pmp doctor: cannot resolve the PMP directories: {error}");
            return 2;
        }
    };
    let report = report_with(config, &paths);
    print!("{}", report.render());
    if report.has_failures() { 1 } else { 0 }
}

/// Builds the report for an explicit [`Paths`] value. Taking the paths as a
/// parameter keeps the whole collection testable in a sandbox.
pub fn report_with(config: &Config, paths: &Paths) -> Report {
    let mut report = Report::new(env!("CARGO_PKG_VERSION"));
    report.push(environment());
    report.push(configuration(paths));
    report.push(database(config, paths));
    report.push(permissions(paths));
    report.push(agents(config));
    report.push(logging(config, paths));
    report
}

fn environment() -> Section {
    Section::new("environment")
        .with(Check::ok("version", env!("CARGO_PKG_VERSION")))
        .with(Check::ok(
            "platform",
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        ))
        .with(match std::env::current_exe() {
            Ok(path) => Check::ok("executable", path.display().to_string()),
            Err(error) => Check::warn("executable", format!("cannot resolve: {error}")),
        })
}

fn configuration(paths: &Paths) -> Section {
    let config_file = match &paths.config_file {
        Some(path) if path.is_file() => Check::ok("config", path.display().to_string()),
        Some(path) => Check::warn(
            "config",
            format!("{} (not found; defaults apply)", path.display()),
        ),
        None => Check::warn("config", "no config directory; defaults apply"),
    };
    Section::new("configuration")
        .with(config_file)
        .with(path_check("data", &paths.data_dir))
        .with(path_check("logs", &paths.logs_dir))
        .with(Check::ok("database", paths.db_path.display().to_string()))
}

fn path_check(label: &str, path: &Path) -> Check {
    if path.is_dir() {
        Check::ok(label, path.display().to_string())
    } else {
        Check::warn(
            label,
            format!("{} (missing; created on first start)", path.display()),
        )
    }
}

fn database(config: &Config, paths: &Paths) -> Section {
    let policy = &config.database.busy;
    let mut section = Section::new("database");

    if !paths.db_path.exists() {
        section = section.with(Check::warn(
            "pmp.db",
            format!(
                "{} (missing; created on first start)",
                paths.db_path.display()
            ),
        ));
    } else {
        match database_facts(&paths.db_path) {
            Ok(facts) => {
                section = section.with(Check::ok(
                    "pmp.db",
                    format!("{} ({})", paths.db_path.display(), human_size(facts.bytes)),
                ));
                let journal = if facts.journal_mode.eq_ignore_ascii_case("wal") {
                    Status::Ok
                } else {
                    Status::Warn
                };
                section = section.with(Check::new(
                    journal,
                    "journal mode",
                    format!("{} (expected wal)", facts.journal_mode),
                ));
                section = section.with(match facts.schema_version {
                    Some(version) => Check::ok("schema version", version.to_string()),
                    None => Check::warn("schema version", "no migrations recorded"),
                });
            }
            Err(error) => {
                section = section.with(Check::fail(
                    "pmp.db",
                    format!("{}: {error}", paths.db_path.display()),
                ));
            }
        }
    }

    section.with(Check::new(
        Status::Ok,
        "busy timeout",
        format!(
            "{} ms (configured), {} attempts, {}–{} ms backoff",
            policy.busy_timeout.as_millis(),
            policy.max_attempts,
            policy.base_delay.as_millis(),
            policy.max_delay.as_millis()
        ),
    ))
}

fn permissions(paths: &Paths) -> Section {
    let mut section = Section::new("permissions");
    section = section.with(probe("data directory", &paths.data_dir));
    section = section.with(probe("logs directory", &paths.logs_dir));
    if let Some(dir) = paths.config_file.as_deref().and_then(Path::parent) {
        section = section.with(probe("config directory", dir));
    }
    section
}

/// Verifies writability by creating and removing a probe file; this is the only
/// check with a side effect, and it only touches the directory it tests.
fn probe(label: &str, dir: &Path) -> Check {
    match std::fs::create_dir_all(dir).and_then(|()| {
        let probe = dir.join(format!(".pmp-doctor-{}", std::process::id()));
        std::fs::write(&probe, b"")?;
        std::fs::remove_file(&probe)
    }) {
        Ok(()) => Check::ok(label, "writable"),
        Err(error) => Check::fail(label, format!("not writable: {error}")),
    }
}

fn agents(config: &Config) -> Section {
    let mut section = Section::new("agents");
    if config.agents.is_empty() {
        return section.with(Check::ok("profiles", "none configured"));
    }
    let path = std::env::var_os("PATH");
    for (name, profile) in &config.agents {
        section = section.with(match resolve_command(&profile.command, path.as_deref()) {
            Some(binary) => Check::ok(name, binary.display().to_string()),
            None => Check::fail(name, format!("{} not found in PATH", profile.command)),
        });
    }
    section
}

/// Resolves `command` to an executable: an explicit path must exist as one, a
/// bare name is searched in `path` (the process `PATH` in production).
pub fn resolve_command(command: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let explicit = Path::new(command);
    if explicit.components().count() > 1 {
        return is_executable(explicit).then(|| explicit.to_path_buf());
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn logging(config: &Config, paths: &Paths) -> Section {
    let mut section = Section::new("logging").with(Check::ok(
        "level",
        format!("{} (override with PMP_LOG)", config.logging.level),
    ));
    section = section.with(
        match logging::destination(&config.logging, &paths.logs_dir) {
            None => Check::warn("file", "disabled ([logging] file is empty)"),
            Some(path) if path.exists() => Check::ok("file", path.display().to_string()),
            Some(path) => Check::warn(
                "file",
                format!("{} (missing; created on first start)", path.display()),
            ),
        },
    );
    section
}

/// What SQLite reports for the shared database, read without migrating it.
struct DatabaseFacts {
    journal_mode: String,
    schema_version: Option<i64>,
    bytes: u64,
}

fn database_facts(path: &Path) -> Result<DatabaseFacts, String> {
    let bytes = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let has_history: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master \
             WHERE type = 'table' AND name = 'refinery_schema_history')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let schema_version = if has_history {
        connection
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    Ok(DatabaseFacts {
        journal_mode,
        schema_version,
        bytes,
    })
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::busy::BusyOptions;
    use crate::db::sqlite::SqliteRepository;

    #[test]
    fn renders_sections_and_a_summary() {
        let mut report = Report::new("1.2.3");
        report.push(
            Section::new("environment")
                .with(Check::ok("version", "1.2.3"))
                .with(Check::warn("config", "not found")),
        );
        report.push(Section::new("database").with(Check::fail("pmp.db", "unreadable")));

        assert_eq!(
            report.render(),
            "pmp doctor 1.2.3\n\
             \n\
             environment\n\
             \x20 ok    version: 1.2.3\n\
             \x20 warn  config: not found\n\
             \n\
             database\n\
             \x20 fail  pmp.db: unreadable\n\
             \n\
             summary: 1 ok, 1 warn, 1 fail\n"
        );
        assert!(report.has_failures());
        assert_eq!(report.counts(), (1, 1, 1));
    }

    #[test]
    fn a_missing_database_is_only_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_root(dir.path());

        let report = report_with(&Config::default(), &paths);
        let text = report.render();

        assert!(text.starts_with("pmp doctor "), "{text}");
        assert!(text.contains("warn  pmp.db: "), "{text}");
        assert!(text.contains("missing; created on first start"), "{text}");
        assert!(text.contains("ok    profiles: none configured"), "{text}");
        assert!(!report.has_failures(), "{text}");
    }

    #[test]
    fn a_migrated_database_reports_wal_and_the_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_root(dir.path());
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        let repository =
            SqliteRepository::open_with(&paths.db_path, BusyOptions::default()).expect("db opens");
        drop(repository);

        let report = report_with(&Config::default(), &paths);

        let database = report
            .sections
            .iter()
            .find(|section| section.title == "database")
            .expect("database section");
        let journal = database
            .checks
            .iter()
            .find(|check| check.label == "journal mode")
            .expect("journal mode check");
        assert_eq!(journal.status, Status::Ok, "{journal:?}");
        assert!(journal.detail.starts_with("wal"), "{journal:?}");

        let schema = database
            .checks
            .iter()
            .find(|check| check.label == "schema version")
            .expect("schema version check");
        assert_eq!(schema.status, Status::Ok, "{schema:?}");
        assert!(schema.detail.parse::<i64>().is_ok(), "{schema:?}");
    }

    #[test]
    fn an_unwritable_directory_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_root(dir.path());
        std::fs::create_dir_all(paths.data_dir.parent().unwrap()).unwrap();
        std::fs::write(&paths.data_dir, b"not a directory").unwrap();

        let report = report_with(&Config::default(), &paths);

        assert!(report.has_failures());
        let text = report.render();
        assert!(text.contains("fail  data directory"), "{text}");
        assert!(text.contains("fail  logs directory"), "{text}");
    }

    #[test]
    fn resolves_bare_names_in_the_search_path() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("agent");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(resolve_command("agent", Some(&path)), Some(binary.clone()));
        assert_eq!(resolve_command("missing", Some(&path)), None);

        let explicit = binary.display().to_string();
        assert_eq!(resolve_command(&explicit, None), Some(binary));
        assert_eq!(resolve_command("/nope/missing", None), None);
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
