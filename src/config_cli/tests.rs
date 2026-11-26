// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tests for `pmp config`, exercised against a temporary config directory.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{HELP, Outcome, run_with};

fn sandbox() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config_file = dir.path().join("pmp").join("config.toml");
    (dir, config_file)
}

fn run(config_file: &Path, args: &[&str], env: &[(&str, &str)]) -> Outcome {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let lookup = |key: &str| {
        env.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    };
    run_with(config_file, &args, &lookup)
}

fn write(config_file: &Path, text: &str) {
    std::fs::create_dir_all(config_file.parent().unwrap()).unwrap();
    std::fs::write(config_file, text).unwrap();
}

fn read(config_file: &Path) -> String {
    std::fs::read_to_string(config_file).unwrap()
}

#[test]
fn path_prints_the_resolved_file() {
    let (_dir, config_file) = sandbox();
    let outcome = run(&config_file, &["path"], &[]);
    assert_eq!(outcome.code, 0, "{outcome:?}");
    assert_eq!(outcome.stdout, format!("{}\n", config_file.display()));
}

#[test]
fn list_shows_defaults_and_their_layer() {
    let (_dir, config_file) = sandbox();
    let outcome = run(&config_file, &["list"], &[]);
    assert_eq!(outcome.code, 0, "{outcome:?}");
    for needle in [
        "appearance.palette = default (default)",
        "appearance.colors = true (default)",
        "database.busy_timeout_ms = 2000 (default)",
        "database.retry_attempts = 5 (default)",
        "mcp.timeout_secs = 30 (default)",
        "logging.level = warn (default)",
        "logging.file = pmp.log (default)",
        "agents.profiles = none configured (default)",
    ] {
        assert!(
            outcome.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            outcome.stdout
        );
    }
}

#[test]
fn list_resolves_the_config_file_and_the_environment() {
    let (_dir, config_file) = sandbox();
    write(
        &config_file,
        "[appearance]\npalette = \"bright\"\n\n[logging]\nlevel = \"debug\"\n\n\
         [agents.sim]\ncommand = \"sim-acp\"\nargs = [\"--stdio\"]\n\
         env = { API_KEY = \"super-secret\" }\n",
    );

    let outcome = run(
        &config_file,
        &["list"],
        &[("PMP_THEME", "default"), ("PMP_LOG", "info")],
    );

    assert_eq!(outcome.code, 0, "{outcome:?}");
    assert!(
        outcome
            .stdout
            .contains("appearance.palette = default (environment (PMP_THEME))"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("logging.level = info (environment (PMP_LOG))"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("agents.sim.command = sim-acp (config.toml)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("agents.sim.args = --stdio (config.toml)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("agents.sim.env = 1 variable(s) (hidden)"),
        "{}",
        outcome.stdout
    );
    assert!(
        !outcome.stdout.contains("super-secret"),
        "secrets must never be printed:\n{}",
        outcome.stdout
    );
}

#[test]
fn list_includes_the_legacy_theme_layer() {
    let (dir, config_file) = sandbox();
    write(&config_file, "[appearance]\ncolors = false\n");
    std::fs::write(
        dir.path().join("pmp").join("theme.toml"),
        "palette = \"bright\"\n[colors]\naccent = \"red\"\n",
    )
    .unwrap();

    let outcome = run(&config_file, &["list"], &[]);

    assert!(
        outcome
            .stdout
            .contains("appearance.palette = bright (theme.toml)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("appearance.tokens.accent = red (theme.toml)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("appearance.colors = false (config.toml)"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn get_returns_one_key_a_section_or_an_unset_token() {
    let (_dir, config_file) = sandbox();

    let one = run(&config_file, &["get", "mcp.timeout_secs"], &[]);
    assert_eq!(one.stdout, "mcp.timeout_secs = 30 (default)\n");

    let section = run(&config_file, &["get", "appearance"], &[]);
    assert!(section.stdout.contains("appearance.palette"));
    assert!(section.stdout.contains("appearance.colors"));
    assert!(!section.stdout.contains("database."));

    let token = run(&config_file, &["get", "appearance.tokens.accent"], &[]);
    assert_eq!(
        token.stdout,
        "appearance.tokens.accent = (not overridden) (default palette)\n"
    );

    let agents = run(&config_file, &["get", "agents"], &[]);
    assert_eq!(
        agents.stdout,
        "agents.profiles = none configured (default)\n"
    );

    let unknown = run(&config_file, &["get", "nope.key"], &[]);
    assert_eq!(unknown.code, 1);
    assert!(unknown.stderr.contains("unknown configuration key"));
}

#[test]
fn set_creates_the_file_and_preserves_comments_and_other_values() {
    let (_dir, config_file) = sandbox();
    write(
        &config_file,
        "# keep this comment\n[mcp]\ntimeout_secs = 10\n\n[logging]\nlevel = \"info\"\n",
    );

    let outcome = run(&config_file, &["set", "mcp.timeout_secs", "45"], &[]);

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let text = read(&config_file);
    assert!(text.contains("# keep this comment"), "{text}");
    assert!(text.contains("level = \"info\""), "{text}");
    assert!(text.contains("timeout_secs = 45"), "{text}");
    let effective = run(&config_file, &["get", "mcp.timeout_secs"], &[]);
    assert_eq!(effective.stdout, "mcp.timeout_secs = 45 (config.toml)\n");
}

#[test]
fn set_rejects_invalid_values_and_writes_nothing() {
    let (_dir, config_file) = sandbox();
    let cases: &[(&[&str], &str)] = &[
        (&["set", "appearance.palette", "neon"], "palette"),
        (&["set", "appearance.colors", "sometimes"], "true or false"),
        (&["set", "logging.level", "loud"], "level must be"),
        (
            &["set", "appearance.tokens.nope", "red"],
            "unknown color token",
        ),
        (
            &["set", "appearance.tokens.accent", "not-a-color"],
            "not a valid color",
        ),
        (&["set", "database.retry_attempts", "0"], "between 1 and"),
        (&["set", "mcp.timeout_secs", "-1"], "positive integer"),
        (
            &["set", "database.busy_timeout_ms", "0"],
            "positive integer",
        ),
        (
            &["set", "unknown.key", "value"],
            "unknown configuration key",
        ),
        (&["set", "agents.sim.env", "API_KEY=x"], "contains secrets"),
        (
            &["set", "agents.bad.name.command", "x"],
            "unknown agent profile key",
        ),
    ];
    for (args, message) in cases {
        let outcome = run(&config_file, args, &[]);
        assert_eq!(outcome.code, 1, "{args:?} should fail: {outcome:?}");
        assert!(
            outcome.stderr.contains(message),
            "{args:?}: expected {message:?} in {:?}",
            outcome.stderr
        );
        assert!(!config_file.exists(), "{args:?} must not create the file");
    }
}

#[test]
fn set_agent_profile_command_and_args_round_trips() {
    let (_dir, config_file) = sandbox();

    assert_eq!(
        run(&config_file, &["set", "agents.sim.command", "sim-acp"], &[]).code,
        0
    );
    let outcome = run(
        &config_file,
        &["set", "agents.sim.args", "[\"--stdio\", \"--verbose\"]"],
        &[],
    );
    assert_eq!(outcome.code, 0, "{outcome:?}");

    let text = read(&config_file);
    assert!(text.contains("[agents.sim]"), "{text}");
    assert!(text.contains("command = \"sim-acp\""), "{text}");
    assert!(
        text.contains("args = [\"--stdio\", \"--verbose\"]"),
        "{text}"
    );

    let get = run(&config_file, &["get", "agents.sim"], &[]);
    assert!(get.stdout.contains("agents.sim.command = sim-acp"));
    assert!(get.stdout.contains("agents.sim.args = --stdio --verbose"));
}

#[test]
fn reset_removes_only_the_requested_section() {
    let (_dir, config_file) = sandbox();
    write(
        &config_file,
        "[appearance]\npalette = \"bright\"\n[appearance.tokens]\naccent = \"red\"\n\n\
         [mcp]\ntimeout_secs = 12\n\n[agents.sim]\ncommand = \"sim-acp\"\n",
    );

    let tokens = run(&config_file, &["reset", "appearance.tokens"], &[]);
    assert_eq!(tokens.code, 0, "{tokens:?}");
    let text = read(&config_file);
    assert!(text.contains("palette = \"bright\""), "{text}");
    assert!(!text.contains("accent"), "{text}");

    let agents = run(&config_file, &["reset", "agents"], &[]);
    assert_eq!(agents.code, 0, "{agents:?}");
    assert!(!read(&config_file).contains("sim-acp"));

    let again = run(&config_file, &["reset", "agents"], &[]);
    assert_eq!(again.code, 0, "{again:?}");
    assert!(again.stdout.contains("nothing to reset"));

    let unknown = run(&config_file, &["reset", "everything"], &[]);
    assert_eq!(unknown.code, 1);
    assert!(unknown.stderr.contains("unknown section"));
}

#[test]
fn reset_removes_one_agent_profile() {
    let (_dir, config_file) = sandbox();
    write(
        &config_file,
        "[agents.claude]\ncommand = \"claude-acp\"\n\n\
         [agents.sim]\ncommand = \"sim-acp\"\n",
    );

    let outcome = run(&config_file, &["reset", "agents.sim"], &[]);

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let text = read(&config_file);
    assert!(text.contains("claude-acp"), "{text}");
    assert!(!text.contains("sim-acp"), "{text}");
}

#[test]
fn an_empty_log_file_disables_file_logging() {
    let (_dir, config_file) = sandbox();

    let outcome = run(&config_file, &["set", "logging.file", ""], &[]);

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let effective = run(&config_file, &["get", "logging.file"], &[]);
    assert_eq!(
        effective.stdout,
        "logging.file = (disabled) (config.toml)\n"
    );
}

#[cfg(unix)]
#[test]
fn writing_preserves_the_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, config_file) = sandbox();
    write(&config_file, "[mcp]\ntimeout_secs = 10\n");
    std::fs::set_permissions(&config_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    let outcome = run(&config_file, &["set", "mcp.timeout_secs", "45"], &[]);

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let mode = std::fs::metadata(&config_file)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "permissions must not be widened");
}

#[test]
fn a_malformed_file_is_never_rewritten() {
    let (_dir, config_file) = sandbox();
    write(&config_file, "[mcp\ntimeout_secs = 1\n");

    let outcome = run(&config_file, &["set", "mcp.timeout_secs", "45"], &[]);

    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.contains("not valid TOML"));
    assert_eq!(read(&config_file), "[mcp\ntimeout_secs = 1\n");
}

#[test]
fn unknown_subcommands_show_the_help() {
    let (_dir, config_file) = sandbox();
    let outcome = run(&config_file, &["wat"], &[]);
    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.contains("unknown config command"));
    assert!(outcome.stderr.contains("Usage: pmp config"));
}

#[test]
fn the_embedded_help_lists_every_subcommand_and_settable_key() {
    for command in ["list", "path", "get", "set", "reset"] {
        assert!(
            HELP.contains(&format!("  {command}")),
            "help must document `{command}`:\n{HELP}"
        );
    }
    for key in [
        "appearance.palette",
        "appearance.tokens.<token>",
        "database.retry_attempts",
        "mcp.timeout_secs",
        "logging.level",
        "agents.<name>.command",
    ] {
        assert!(HELP.contains(key), "help must document `{key}`:\n{HELP}");
    }
    assert!(HELP.ends_with('\n'), "help should end with a newline");
}
