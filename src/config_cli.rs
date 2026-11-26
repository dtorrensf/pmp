// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `pmp config`: inspect and edit `config.toml` without starting the app.
//!
//! Reads go through the same resolution as the composition root
//! ([`Config::from_sources`]), so `get` and `list` show the values the TUI,
//! the MCP server and the repository will actually use. Writes use
//! [`toml_edit`], which keeps comments and the formatting of everything the
//! command does not touch.
//!
//! The command never starts the TUI, the MCP server or touches the database,
//! and it refuses to read or write the `env` table of an agent profile: those
//! values are credentials and belong to the user's editor, not to a CLI that
//! may end up in a log or a shell history.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use toml::Value as TomlValue;
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::config::{self, Config, is_valid_log_level};
use crate::db::busy::{
    BUSY_TIMEOUT_ENV, MAX_ATTEMPTS, RETRY_ATTEMPTS_ENV, RETRY_BASE_DELAY_ENV, RETRY_MAX_DELAY_ENV,
};
use crate::screens::components::glyphs::ASCII_ENV;
use crate::screens::components::motion::REDUCED_MOTION_ENV;
use crate::screens::components::theme::{self, NO_COLOR_ENV, THEME_ENV, ThemeFile};

/// Help printed by `pmp config`, `pmp config -h` and unknown subcommands.
///
/// Kept in a text file next to this module and embedded at compile time, the
/// same way `src/main.rs` embeds `help.txt`, so the wording is edited as prose
/// without Rust escaping.
pub const HELP: &str = include_str!("config_cli/help.txt");

/// Exit code and streams of one invocation. Keeping them in a value makes the
/// whole command testable without spawning a process.
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

/// Production entry point: resolves `config.toml` and prints the outcome.
pub fn run(args: &[String]) -> i32 {
    let Some(config_file) = dirs::config_dir().map(|dir| dir.join(config::CONFIG_FILE)) else {
        eprintln!("pmp config: no config directory is known; set XDG_CONFIG_HOME or HOME");
        return 2;
    };
    let lookup = |key: &str| std::env::var(key).ok();
    let outcome = run_with(&config_file, args, &lookup);
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    outcome.code
}

/// Runs one `pmp config` invocation against `config_file`, reading environment
/// overrides through `lookup`.
pub fn run_with(
    config_file: &Path,
    args: &[String],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Outcome {
    match args.first().map(String::as_str) {
        None | Some("list") => list(config_file, lookup),
        Some("path") => Outcome::ok(format!("{}\n", config_file.display())),
        Some("get") => get(config_file, args.get(1).map(String::as_str), lookup),
        Some("set") => set(config_file, &args[1..]),
        Some("reset") => reset(config_file, args.get(1).map(String::as_str)),
        Some("-h" | "--help" | "help") => Outcome::ok(HELP),
        Some(other) => Outcome::failure(format!("unknown config command '{other}'\n\n{HELP}")),
    }
}

/// Which layer produced an effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Default,
    Config,
    Theme,
    Environment(&'static str),
}

impl Source {
    fn label(self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::Config => "config.toml".to_string(),
            Self::Theme => "theme.toml".to_string(),
            Self::Environment(variable) => format!("environment ({variable})"),
        }
    }
}

/// One effective configuration value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    key: String,
    value: String,
    source: Source,
}

/// Everything read from disk, kept together so provenance does not re-read.
struct Snapshot {
    config: Config,
    raw: Option<TomlValue>,
    theme: Option<ThemeFile>,
}

fn load(config_file: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Snapshot, String> {
    let config_text = read_optional(config_file)?;
    let theme_path = theme_path(config_file);
    let theme_text = match &theme_path {
        Some(path) => read_optional(path)?,
        None => None,
    };
    let config = Config::from_sources(config_text.as_deref(), theme_text.as_deref(), lookup);
    let raw = config_text
        .as_deref()
        .and_then(|text| text.parse::<TomlValue>().ok());
    let theme = theme_text.as_deref().and_then(theme::parse_theme_file);
    Ok(Snapshot { config, raw, theme })
}

fn theme_path(config_file: &Path) -> Option<PathBuf> {
    config_file.parent().map(|dir| dir.join("theme.toml"))
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn list(config_file: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Outcome {
    let snapshot = match load(config_file, lookup) {
        Ok(snapshot) => snapshot,
        Err(error) => return Outcome::failure(error),
    };
    let entries = entries(&snapshot, lookup);
    let mut out = String::new();
    let mut section = "";
    for entry in &entries {
        let current = entry.key.split('.').next().unwrap_or("");
        if current != section {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(current);
            out.push('\n');
            section = current;
        }
        out.push_str(&format!(
            "  {} = {} ({})\n",
            entry.key,
            entry.value,
            entry.source.label()
        ));
    }
    Outcome::ok(out)
}

fn get(
    config_file: &Path,
    query: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Outcome {
    let snapshot = match load(config_file, lookup) {
        Ok(snapshot) => snapshot,
        Err(error) => return Outcome::failure(error),
    };
    let entries = entries(&snapshot, lookup);
    let Some(query) = query.filter(|query| !query.trim().is_empty()) else {
        let entries: Vec<&Entry> = entries.iter().collect();
        return Outcome::ok(render_entries(&entries));
    };
    let query = query.trim();
    let selected: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.key == query || entry.key.starts_with(&format!("{query}.")))
        .collect();
    if selected.is_empty() {
        if let Some(token) = query.strip_prefix("appearance.tokens.")
            && theme::is_valid_token(token)
        {
            return Outcome::ok(format!("{query} = (not overridden) (default palette)\n"));
        }
        return Outcome::failure(format!(
            "unknown configuration key or section '{query}'; run 'pmp config list'"
        ));
    }
    Outcome::ok(render_entries(&selected))
}

fn render_entries(entries: &[&Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&format!(
            "{} = {} ({})\n",
            entry.key,
            entry.value,
            entry.source.label()
        ));
    }
    out
}

/// The effective values, in a stable order: appearance, database, MCP server,
/// logging and agent profiles.
fn entries(snapshot: &Snapshot, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<Entry> {
    let config = &snapshot.config;
    let raw = snapshot.raw.as_ref();
    let mut entries = Vec::new();

    entries.push(Entry {
        key: "appearance.palette".to_string(),
        value: config
            .appearance
            .palette
            .clone()
            .unwrap_or_else(|| theme::DEFAULT_PALETTE.to_string()),
        source: if non_empty(lookup(THEME_ENV)) {
            Source::Environment(THEME_ENV)
        } else if has(raw, &["appearance", "palette"]) {
            Source::Config
        } else if snapshot
            .theme
            .as_ref()
            .is_some_and(|theme| theme.palette.is_some())
        {
            Source::Theme
        } else {
            Source::Default
        },
    });
    entries.push(Entry {
        key: "appearance.colors".to_string(),
        value: config.appearance.colors_enabled.to_string(),
        source: if theme::no_color_requested(&|key| lookup(key)).is_some() {
            Source::Environment(NO_COLOR_ENV)
        } else if has(raw, &["appearance", "colors"]) {
            Source::Config
        } else {
            Source::Default
        },
    });
    entries.push(Entry {
        key: "appearance.ascii".to_string(),
        value: config.appearance.ascii.to_string(),
        source: if non_empty(lookup(ASCII_ENV)) {
            Source::Environment(ASCII_ENV)
        } else if has(raw, &["appearance", "ascii"]) {
            Source::Config
        } else {
            Source::Default
        },
    });
    entries.push(Entry {
        key: "appearance.reduced_motion".to_string(),
        value: config.appearance.reduced_motion.to_string(),
        source: if non_empty(lookup(REDUCED_MOTION_ENV)) {
            Source::Environment(REDUCED_MOTION_ENV)
        } else if has(raw, &["appearance", "reduced_motion"]) {
            Source::Config
        } else {
            Source::Default
        },
    });

    let mut tokens: BTreeSet<&str> = config
        .appearance
        .tokens
        .keys()
        .map(String::as_str)
        .collect();
    if let Some(theme) = &snapshot.theme {
        tokens.extend(theme.colors.keys().map(String::as_str));
    }
    for token in tokens {
        entries.push(Entry {
            key: format!("appearance.tokens.{token}"),
            value: config
                .appearance
                .tokens
                .get(token)
                .cloned()
                .unwrap_or_default(),
            source: if has(raw, &["appearance", "tokens", token]) {
                Source::Config
            } else {
                Source::Theme
            },
        });
    }

    let busy = &config.database.busy;
    entries.push(Entry {
        key: "database.busy_timeout_ms".to_string(),
        value: busy.busy_timeout.as_millis().to_string(),
        source: millis_source(raw, "busy_timeout_ms", BUSY_TIMEOUT_ENV, lookup),
    });
    entries.push(Entry {
        key: "database.retry_attempts".to_string(),
        value: busy.max_attempts.to_string(),
        source: if lookup(RETRY_ATTEMPTS_ENV).is_some_and(|value| {
            value
                .trim()
                .parse::<u32>()
                .is_ok_and(|attempts| (1..=MAX_ATTEMPTS).contains(&attempts))
        }) {
            Source::Environment(RETRY_ATTEMPTS_ENV)
        } else if has(raw, &["database", "retry_attempts"]) {
            Source::Config
        } else {
            Source::Default
        },
    });
    entries.push(Entry {
        key: "database.retry_base_delay_ms".to_string(),
        value: busy.base_delay.as_millis().to_string(),
        source: millis_source(raw, "retry_base_delay_ms", RETRY_BASE_DELAY_ENV, lookup),
    });
    entries.push(Entry {
        key: "database.retry_max_delay_ms".to_string(),
        value: busy.max_delay.as_millis().to_string(),
        source: millis_source(raw, "retry_max_delay_ms", RETRY_MAX_DELAY_ENV, lookup),
    });

    entries.push(Entry {
        key: "mcp.timeout_secs".to_string(),
        value: config.mcp.timeout.as_secs().to_string(),
        source: if lookup(config::MCP_TIMEOUT_ENV).is_some_and(positive_integer) {
            Source::Environment(config::MCP_TIMEOUT_ENV)
        } else if has(raw, &["mcp", "timeout_secs"]) {
            Source::Config
        } else {
            Source::Default
        },
    });

    entries.push(Entry {
        key: "logging.level".to_string(),
        value: config.logging.level.clone(),
        source: if lookup(config::LOG_LEVEL_ENV)
            .is_some_and(|level| !level.trim().is_empty() && is_valid_log_level(&level))
        {
            Source::Environment(config::LOG_LEVEL_ENV)
        } else if has(raw, &["logging", "level"]) {
            Source::Config
        } else {
            Source::Default
        },
    });
    let file = match config.logging.file.as_deref() {
        Some("") => "(disabled)".to_string(),
        Some(file) => file.to_string(),
        None => "pmp.log".to_string(),
    };
    entries.push(Entry {
        key: "logging.file".to_string(),
        value: file,
        source: if non_empty(lookup(config::LOG_FILE_ENV)) {
            Source::Environment(config::LOG_FILE_ENV)
        } else if has(raw, &["logging", "file"]) {
            Source::Config
        } else {
            Source::Default
        },
    });

    for (name, profile) in &config.agents {
        entries.push(Entry {
            key: format!("agents.{name}.command"),
            value: profile.command.clone(),
            source: Source::Config,
        });
        entries.push(Entry {
            key: format!("agents.{name}.args"),
            value: if profile.args.is_empty() {
                "(none)".to_string()
            } else {
                profile.args.join(" ")
            },
            source: if has(raw, &["agents", name, "args"]) {
                Source::Config
            } else {
                Source::Default
            },
        });
        entries.push(Entry {
            key: format!("agents.{name}.env"),
            value: if profile.env.is_empty() {
                "(none)".to_string()
            } else {
                format!("{} variable(s) (hidden)", profile.env.len())
            },
            source: if has(raw, &["agents", name, "env"]) {
                Source::Config
            } else {
                Source::Default
            },
        });
    }
    if config.agents.is_empty() {
        entries.push(Entry {
            key: "agents.profiles".to_string(),
            value: "none configured".to_string(),
            source: Source::Default,
        });
    }

    entries
}

fn millis_source(
    raw: Option<&TomlValue>,
    key: &str,
    variable: &'static str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Source {
    if lookup(variable).is_some_and(positive_integer) {
        Source::Environment(variable)
    } else if has(raw, &["database", key]) {
        Source::Config
    } else {
        Source::Default
    }
}

fn positive_integer(value: String) -> bool {
    value.trim().parse::<u64>().is_ok_and(|number| number > 0)
}

fn non_empty(value: Option<String>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

fn has(raw: Option<&TomlValue>, path: &[&str]) -> bool {
    let mut current = match raw {
        Some(value) => value,
        None => return false,
    };
    for key in path {
        match current.get(*key) {
            Some(value) => current = value,
            None => return false,
        }
    }
    true
}

/// One value accepted by `pmp config set`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Setting {
    Palette,
    Colors,
    Ascii,
    ReducedMotion,
    Token(String),
    BusyTimeout,
    RetryAttempts,
    RetryBaseDelay,
    RetryMaxDelay,
    McpTimeout,
    LogLevel,
    LogFile,
    AgentCommand(String),
    AgentArgs(String),
}

fn parse_setting(key: &str) -> Result<Setting, String> {
    let setting = match key {
        "appearance.palette" => Setting::Palette,
        "appearance.colors" => Setting::Colors,
        "appearance.ascii" => Setting::Ascii,
        "appearance.reduced_motion" => Setting::ReducedMotion,
        "database.busy_timeout_ms" => Setting::BusyTimeout,
        "database.retry_attempts" => Setting::RetryAttempts,
        "database.retry_base_delay_ms" => Setting::RetryBaseDelay,
        "database.retry_max_delay_ms" => Setting::RetryMaxDelay,
        "mcp.timeout_secs" => Setting::McpTimeout,
        "logging.level" => Setting::LogLevel,
        "logging.file" => Setting::LogFile,
        _ => {
            if let Some(token) = key.strip_prefix("appearance.tokens.") {
                if !theme::is_valid_token(token) {
                    return Err(format!(
                        "unknown color token '{token}'; expected one of: {}",
                        theme::TOKEN_NAMES.join(", ")
                    ));
                }
                Setting::Token(token.to_string())
            } else if let Some(rest) = key.strip_prefix("agents.") {
                let (name, field) = rest.split_once('.').ok_or_else(|| {
                    "expected agents.<name>.command or agents.<name>.args".to_string()
                })?;
                if !simple_agent_name(name) {
                    return Err(
                        "agent profile names may only use letters, digits, '-' and '_'".to_string(),
                    );
                }
                match field {
                    "command" => Setting::AgentCommand(name.to_string()),
                    "args" => Setting::AgentArgs(name.to_string()),
                    "env" => {
                        return Err(format!(
                            "agents.{name}.env contains secrets and is not editable here; \
                             edit config.toml directly"
                        ));
                    }
                    other => {
                        return Err(format!(
                            "unknown agent profile key '{other}'; expected command or args"
                        ));
                    }
                }
            } else {
                return Err(format!(
                    "unknown configuration key '{key}'; run 'pmp config --help'"
                ));
            }
        }
    };
    Ok(setting)
}

fn simple_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn set(config_file: &Path, args: &[String]) -> Outcome {
    let [key, raw_value] = args else {
        return Outcome::failure("usage: pmp config set <key> <value>");
    };
    let setting = match parse_setting(key) {
        Ok(setting) => setting,
        Err(error) => return Outcome::failure(error),
    };
    let mut document = match load_document(config_file) {
        Ok(document) => document,
        Err(error) => return Outcome::failure(error),
    };
    let display = match apply(&mut document, &setting, raw_value) {
        Ok(display) => display,
        Err(error) => return Outcome::failure(error),
    };
    if let Err(error) = write(config_file, &document.to_string()) {
        return Outcome::failure(error);
    }
    Outcome::ok(format!(
        "set {key} = {display} in {}\n",
        config_file.display()
    ))
}

fn reset(config_file: &Path, section: Option<&str>) -> Outcome {
    let Some(section) = section else {
        return Outcome::failure("usage: pmp config reset <section>");
    };
    let mut document = match load_document(config_file) {
        Ok(document) => document,
        Err(error) => return Outcome::failure(error),
    };
    let removed = match remove_section(&mut document, section) {
        Ok(removed) => removed,
        Err(error) => return Outcome::failure(error),
    };
    if !removed {
        return Outcome::ok(format!(
            "nothing to reset: '{section}' is not present in {}\n",
            config_file.display()
        ));
    }
    if let Err(error) = write(config_file, &document.to_string()) {
        return Outcome::failure(error);
    }
    Outcome::ok(format!("reset '{section}' in {}\n", config_file.display()))
}

fn load_document(config_file: &Path) -> Result<DocumentMut, String> {
    match read_optional(config_file)? {
        Some(text) => text.parse::<DocumentMut>().map_err(|error| {
            format!(
                "{} is not valid TOML ({error}); fix it before editing it with pmp config",
                config_file.display()
            )
        }),
        None => Ok(DocumentMut::new()),
    }
}

fn apply(document: &mut DocumentMut, setting: &Setting, raw: &str) -> Result<String, String> {
    match setting {
        Setting::Palette => {
            let palette = raw.trim().to_ascii_lowercase();
            if !theme::is_valid_palette(&palette) {
                return Err("palette must be 'default' or 'bright'".to_string());
            }
            child(document, &["appearance"])?.insert("palette", value(palette.clone()));
            Ok(palette)
        }
        Setting::Colors => {
            let flag = parse_bool(raw)?;
            child(document, &["appearance"])?.insert("colors", value(flag));
            Ok(flag.to_string())
        }
        Setting::Ascii => {
            let flag = parse_bool(raw)?;
            child(document, &["appearance"])?.insert("ascii", value(flag));
            Ok(flag.to_string())
        }
        Setting::ReducedMotion => {
            let flag = parse_bool(raw)?;
            child(document, &["appearance"])?.insert("reduced_motion", value(flag));
            Ok(flag.to_string())
        }
        Setting::Token(name) => {
            let color = raw.trim();
            if !theme::is_valid_color(color) {
                return Err(format!(
                    "'{color}' is not a valid color name or #rrggbb value"
                ));
            }
            child(document, &["appearance", "tokens"])?.insert(name, value(color));
            Ok(color.to_string())
        }
        Setting::BusyTimeout => {
            let millis = parse_positive(raw, "busy_timeout_ms")?;
            child(document, &["database"])?.insert("busy_timeout_ms", value(millis));
            Ok(millis.to_string())
        }
        Setting::RetryAttempts => {
            let attempts = raw
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|attempts| (1..=MAX_ATTEMPTS).contains(attempts))
                .ok_or_else(|| format!("retry_attempts must be between 1 and {MAX_ATTEMPTS}"))?;
            child(document, &["database"])?.insert("retry_attempts", value(i64::from(attempts)));
            Ok(attempts.to_string())
        }
        Setting::RetryBaseDelay => {
            let millis = parse_positive(raw, "retry_base_delay_ms")?;
            child(document, &["database"])?.insert("retry_base_delay_ms", value(millis));
            Ok(millis.to_string())
        }
        Setting::RetryMaxDelay => {
            let millis = parse_positive(raw, "retry_max_delay_ms")?;
            child(document, &["database"])?.insert("retry_max_delay_ms", value(millis));
            Ok(millis.to_string())
        }
        Setting::McpTimeout => {
            let secs = parse_positive(raw, "timeout_secs")?;
            child(document, &["mcp"])?.insert("timeout_secs", value(secs));
            Ok(secs.to_string())
        }
        Setting::LogLevel => {
            let level = raw.trim().to_ascii_lowercase();
            if !is_valid_log_level(&level) {
                return Err("level must be trace, debug, info, warn or error".to_string());
            }
            child(document, &["logging"])?.insert("level", value(level.clone()));
            Ok(level)
        }
        Setting::LogFile => {
            let file = raw.trim();
            child(document, &["logging"])?.insert("file", value(file));
            Ok(if file.is_empty() {
                "\"\" (disabled)".to_string()
            } else {
                file.to_string()
            })
        }
        Setting::AgentCommand(name) => {
            let command = raw.trim();
            if command.is_empty() {
                return Err("agent command cannot be empty".to_string());
            }
            let profile = child(document, &["agents", name])?;
            profile.insert("command", value(command));
            Ok(command.to_string())
        }
        Setting::AgentArgs(name) => {
            let parsed: TomlValue = toml::from_str(&format!("args = {raw}")).map_err(|error| {
                format!("args must be a TOML array of strings, e.g. '[\"--stdio\"]': {error}")
            })?;
            let args: Vec<String> = parsed
                .get("args")
                .and_then(TomlValue::as_array)
                .ok_or_else(|| "args must be a TOML array of strings".to_string())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "every arg must be a string".to_string())
                })
                .collect::<Result<_, _>>()?;
            let mut array = Array::new();
            for arg in &args {
                array.push(arg.as_str());
            }
            let profile = child(document, &["agents", name])?;
            profile.insert("args", value(array));
            Ok(format!("{args:?}"))
        }
    }
}

/// Navigates to (and creates) the table at `path`, refusing to turn an inline
/// table or a scalar into a table behind the user's back.
///
/// Intermediate tables are created implicit, so setting `agents.claude.command`
/// writes `[agents.claude]` without an empty `[agents]` header.
fn child<'a>(document: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut Table, String> {
    let mut table = document.as_table_mut();
    for (index, key) in path.iter().enumerate() {
        if !table.contains_key(key) {
            let mut created = Table::new();
            created.set_implicit(index + 1 < path.len());
            table.insert(key, Item::Table(created));
        }
        let item = table
            .get_mut(key)
            .ok_or_else(|| format!("cannot access '{key}' in config.toml"))?;
        table = match item {
            Item::Table(table) => table,
            _ => {
                return Err(format!(
                    "'{key}' is not a regular table in config.toml; edit it manually"
                ));
            }
        };
    }
    table.set_implicit(false);
    Ok(table)
}

fn remove_section(document: &mut DocumentMut, section: &str) -> Result<bool, String> {
    match section {
        "appearance" | "database" | "mcp" | "logging" | "agents" => {
            Ok(document.as_table_mut().remove(section).is_some())
        }
        "appearance.tokens" => {
            let Some(item) = document.as_table_mut().get_mut("appearance") else {
                return Ok(false);
            };
            let Item::Table(appearance) = item else {
                return Err(
                    "'appearance' is not a regular table in config.toml; edit it manually"
                        .to_string(),
                );
            };
            Ok(appearance.remove("tokens").is_some())
        }
        _ => {
            if let Some(name) = section.strip_prefix("agents.") {
                if !simple_agent_name(name) {
                    return Err(
                        "agent profile names may only use letters, digits, '-' and '_'".to_string(),
                    );
                }
                let Some(item) = document.as_table_mut().get_mut("agents") else {
                    return Ok(false);
                };
                let Item::Table(agents) = item else {
                    return Err(
                        "'agents' is not a regular table in config.toml; edit it manually"
                            .to_string(),
                    );
                };
                Ok(agents.remove(name).is_some())
            } else {
                Err(format!(
                    "unknown section '{section}'; expected appearance, appearance.tokens, \
                     database, mcp, logging, agents or agents.<name>"
                ))
            }
        }
    }
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err("value must be true or false".to_string()),
    }
}

fn parse_positive(raw: &str, key: &str) -> Result<i64, String> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("{key} must be a positive integer"))
}

fn write(config_file: &Path, text: &str) -> Result<(), String> {
    // Follow a symlinked config (dotfiles setups) instead of replacing the
    // link with a regular file.
    let target = std::fs::canonicalize(config_file).unwrap_or_else(|_| config_file.to_path_buf());
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    // Replacing the file must not widen its permissions: it can contain agent
    // credentials, so keep whatever mode the user already had (0600, 0640...).
    let permissions = std::fs::metadata(&target)
        .ok()
        .map(|metadata| metadata.permissions());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = target.with_extension(format!("toml.tmp-{stamp}"));
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    if let Some(permissions) = permissions {
        let _ = std::fs::set_permissions(&temporary, permissions);
    }
    std::fs::rename(&temporary, &target).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot replace {}: {error}", target.display())
    })
}

#[cfg(test)]
mod tests;
