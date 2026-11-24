// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unified, injectable configuration.
//!
//! Every setting lives in one optional file, `~/.config/pmp/config.toml`
//! (`$XDG_CONFIG_HOME/pmp/config.toml` when set):
//!
//! ```toml
//! [appearance]
//! palette = "bright"   # default | bright
//! colors = true        # false keeps shapes and bold only
//! ascii = false        # ASCII glyph fallback
//! reduced_motion = false
//!
//! [appearance.tokens]  # per-token color overrides
//! accent = "#7aa2f7"
//!
//! [database]
//! busy_timeout_ms = 2000
//! retry_attempts = 5
//! retry_base_delay_ms = 10
//! retry_max_delay_ms = 250
//!
//! [mcp]
//! timeout_secs = 30
//!
//! [logging]
//! level = "warn"       # trace | debug | info | warn | error
//! file = "pmp.log"
//!
//! [agents.claude]
//! command = "claude-code-acp"
//! args = ["--stdio"]
//! env = { ANTHROPIC_API_KEY = "..." }
//! ```
//!
//! Precedence, lowest to highest:
//!
//! 1. Built-in defaults.
//! 2. The legacy `~/.config/pmp/theme.toml` (palette and color overrides).
//! 3. `config.toml`.
//! 4. The environment (`NO_COLOR`, `PMP_THEME`, `PMP_ASCII`,
//!    `PMP_REDUCED_MOTION`, `PMP_DB_*`, `PMP_MCP_TIMEOUT_SECS`).
//!
//! [`Config::load`] reads the files and the environment once, at startup. The
//! TUI, the MCP server and the repository receive the resolved values, so no
//! render or query reads the environment behind the caller's back.
//!
//! A missing file is normal. A malformed file or an invalid value is logged
//! and replaced by the previous layer, never fatal: PMP always starts.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::db::busy::BusyOptions;
use crate::screens::components::glyphs;
use crate::screens::components::motion;
use crate::screens::components::theme;

/// Config file path, relative to the config directory (`~/.config` on Linux).
pub const CONFIG_FILE: &str = "pmp/config.toml";

/// Default deadline for one MCP repository operation, in seconds.
pub const DEFAULT_MCP_TIMEOUT: Duration = Duration::from_secs(30);

/// Environment variable that overrides the MCP timeout, in seconds.
pub const MCP_TIMEOUT_ENV: &str = "PMP_MCP_TIMEOUT_SECS";

/// Logging level used when the file does not set a valid one.
pub const DEFAULT_LOG_LEVEL: &str = "warn";

const LOG_LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];

/// Resolved configuration for the whole process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub appearance: AppearanceConfig,
    pub database: DatabaseConfig,
    pub mcp: McpConfig,
    pub logging: LoggingConfig,
    /// Agent profiles usable by the ACP client, keyed by profile name.
    pub agents: BTreeMap<String, AgentProfile>,
}

impl Config {
    /// Reads `config.toml`, the legacy `theme.toml` and the environment once.
    pub fn load() -> Config {
        let lookup = |key: &str| std::env::var(key).ok();
        match dirs::config_dir() {
            Some(dir) => Config::from_dir(&dir, lookup),
            None => Config::from_sources(None, None, lookup),
        }
    }

    /// Reads both files from `config_dir` (`$XDG_CONFIG_HOME` in production)
    /// and applies the environment on top. A missing file is not an error.
    pub fn from_dir(config_dir: &Path, lookup: impl Fn(&str) -> Option<String>) -> Config {
        let config_file = read_file(config_dir.join(CONFIG_FILE));
        let theme_file = read_file(config_dir.join(theme::THEME_FILE));
        Config::from_sources(config_file.as_deref(), theme_file.as_deref(), lookup)
    }

    /// Resolves the configuration from file contents and an environment
    /// lookup. Taking both as parameters keeps the precedence rules testable
    /// without touching the process environment or the filesystem.
    pub fn from_sources(
        config_file: Option<&str>,
        theme_file: Option<&str>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Config {
        let raw = config_file.map(parse_config_file).unwrap_or_default();
        let legacy = theme_file
            .and_then(theme::parse_theme_file)
            .unwrap_or_default();
        Config {
            appearance: AppearanceConfig::resolve(raw.appearance, &legacy, &lookup),
            database: DatabaseConfig::resolve(raw.database, &lookup),
            mcp: McpConfig::resolve(raw.mcp, &lookup),
            logging: LoggingConfig::resolve(raw.logging),
            agents: agent_profiles(raw.agents),
        }
    }
}

/// Visual preferences: base palette, per-token overrides and the glyph/motion
/// switches. The UI context is built from these values at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceConfig {
    /// Base palette name; `None` keeps the default one.
    pub palette: Option<String>,
    /// Whether colors are applied. `NO_COLOR` wins over this value.
    pub colors_enabled: bool,
    /// Per-token color overrides (same names as the legacy `theme.toml`).
    pub tokens: BTreeMap<String, String>,
    /// Use the ASCII glyph fallback.
    pub ascii: bool,
    /// Turn every animation off.
    pub reduced_motion: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            palette: None,
            colors_enabled: true,
            tokens: BTreeMap::new(),
            ascii: false,
            reduced_motion: false,
        }
    }
}

impl AppearanceConfig {
    fn resolve(
        raw: RawAppearance,
        legacy: &theme::ThemeFile,
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> AppearanceConfig {
        // The legacy file is the oldest layer; config.toml overrides it.
        let mut tokens = legacy.colors.clone();
        tokens.extend(raw.tokens);

        let colors_enabled = match theme::no_color_requested(lookup) {
            Some(true) => false,
            _ => raw.colors.unwrap_or(true),
        };
        let palette = lookup(theme::THEME_ENV)
            .or(raw.palette)
            .or_else(|| legacy.palette.clone());
        let ascii = glyphs::ascii_requested(lookup).unwrap_or(raw.ascii.unwrap_or(false));
        let reduced_motion =
            motion::reduced_requested(lookup).unwrap_or(raw.reduced_motion.unwrap_or(false));
        AppearanceConfig {
            palette,
            colors_enabled,
            tokens,
            ascii,
            reduced_motion,
        }
    }
}

/// SQLite lock policy. See [`BusyOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DatabaseConfig {
    pub busy: BusyOptions,
}

impl DatabaseConfig {
    fn resolve(raw: RawDatabase, lookup: &impl Fn(&str) -> Option<String>) -> DatabaseConfig {
        let defaults = BusyOptions::default();
        let busy = BusyOptions {
            busy_timeout: millis(
                raw.busy_timeout_ms,
                "database.busy_timeout_ms",
                defaults.busy_timeout,
            ),
            max_attempts: attempts(raw.retry_attempts, defaults.max_attempts),
            base_delay: millis(
                raw.retry_base_delay_ms,
                "database.retry_base_delay_ms",
                defaults.base_delay,
            ),
            max_delay: millis(
                raw.retry_max_delay_ms,
                "database.retry_max_delay_ms",
                defaults.max_delay,
            ),
        };
        DatabaseConfig {
            busy: busy.with_env_overrides(lookup),
        }
    }
}

/// MCP server settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpConfig {
    /// Upper bound for one repository operation.
    pub timeout: Duration,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_MCP_TIMEOUT,
        }
    }
}

impl McpConfig {
    fn resolve(raw: RawMcp, lookup: &impl Fn(&str) -> Option<String>) -> McpConfig {
        let mut timeout = match raw.timeout_secs {
            Some(0) => {
                tracing::warn!("invalid mcp.timeout_secs = 0; using the default");
                DEFAULT_MCP_TIMEOUT
            }
            Some(secs) => Duration::from_secs(secs),
            None => DEFAULT_MCP_TIMEOUT,
        };
        if let Some(value) = lookup(MCP_TIMEOUT_ENV) {
            match value.trim().parse::<u64>() {
                Ok(secs) if secs > 0 => timeout = Duration::from_secs(secs),
                _ => tracing::warn!(
                    variable = MCP_TIMEOUT_ENV,
                    value,
                    "invalid value; keeping the configured timeout"
                ),
            }
        }
        McpConfig { timeout }
    }
}

/// Logging settings, consumed by the `pmp doctor` command and the file logger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingConfig {
    /// One of `trace`, `debug`, `info`, `warn` or `error`.
    pub level: String,
    /// Log file path, relative to the current directory; `None` logs to stderr.
    pub file: Option<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LOG_LEVEL.to_string(),
            file: None,
        }
    }
}

impl LoggingConfig {
    fn resolve(raw: RawLogging) -> LoggingConfig {
        let level = match raw.level {
            None => DEFAULT_LOG_LEVEL.to_string(),
            Some(level) => {
                let level = level.trim().to_ascii_lowercase();
                if LOG_LEVELS.contains(&level.as_str()) {
                    level
                } else {
                    tracing::warn!(level, "invalid logging.level; using the default");
                    DEFAULT_LOG_LEVEL.to_string()
                }
            }
        };
        let file = raw
            .file
            .map(|file| file.trim().to_string())
            .filter(|file| !file.is_empty());
        LoggingConfig { level, file }
    }
}

/// One agent PMP can launch through ACP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfile {
    /// Binary or command to execute.
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Extra environment variables for the agent process.
    pub env: BTreeMap<String, String>,
}

/// `None` for a missing file, the parsed value for a valid one; a malformed
/// file is logged and ignored as a whole.
fn read_file(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref();
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read configuration file");
            None
        }
    }
}

fn parse_config_file(text: &str) -> RawConfig {
    match toml::from_str(text) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::warn!(%error, "ignoring malformed configuration file");
            RawConfig::default()
        }
    }
}

/// A positive number of milliseconds, or `fallback` (with a warning).
fn millis(value: Option<u64>, key: &str, fallback: Duration) -> Duration {
    match value {
        None => fallback,
        Some(0) => {
            tracing::warn!(key, "value must be greater than zero; using the default");
            fallback
        }
        Some(millis) => Duration::from_millis(millis),
    }
}

/// An attempt budget in `1..=MAX_ATTEMPTS`, or `fallback`.
fn attempts(value: Option<u32>, fallback: u32) -> u32 {
    match value {
        None => fallback,
        Some(attempts) if (1..=crate::db::busy::MAX_ATTEMPTS).contains(&attempts) => attempts,
        Some(attempts) => {
            tracing::warn!(
                key = "database.retry_attempts",
                attempts,
                max = crate::db::busy::MAX_ATTEMPTS,
                "value out of range; using the default"
            );
            fallback
        }
    }
}

/// Valid profiles only: a blank name or a missing command is logged and the
/// profile is skipped, so one bad entry does not discard the others.
fn agent_profiles(raw: BTreeMap<String, RawAgent>) -> BTreeMap<String, AgentProfile> {
    let mut profiles = BTreeMap::new();
    for (name, raw) in raw {
        let name = name.trim();
        if name.is_empty() {
            tracing::warn!("ignoring agent profile with an empty name");
            continue;
        }
        let command = raw
            .command
            .map(|command| command.trim().to_string())
            .filter(|command| !command.is_empty());
        let Some(command) = command else {
            tracing::warn!(profile = name, "agent profile has no command; ignoring it");
            continue;
        };
        profiles.insert(
            name.to_string(),
            AgentProfile {
                command,
                args: raw.args,
                env: raw.env,
            },
        );
    }
    profiles
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    appearance: RawAppearance,
    database: RawDatabase,
    mcp: RawMcp,
    logging: RawLogging,
    agents: BTreeMap<String, RawAgent>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawAppearance {
    palette: Option<String>,
    colors: Option<bool>,
    ascii: Option<bool>,
    reduced_motion: Option<bool>,
    tokens: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawDatabase {
    busy_timeout_ms: Option<u64>,
    retry_attempts: Option<u32>,
    retry_base_delay_ms: Option<u64>,
    retry_max_delay_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawMcp {
    timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawLogging {
    level: Option<String>,
    file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawAgent {
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::busy::{
        BUSY_TIMEOUT_ENV, DEFAULT_ATTEMPTS, DEFAULT_BUSY_TIMEOUT, RETRY_ATTEMPTS_ENV,
        RETRY_BASE_DELAY_ENV, RETRY_MAX_DELAY_ENV,
    };
    use crate::screens::components::glyphs::ASCII_ENV;
    use crate::screens::components::motion::REDUCED_MOTION_ENV;
    use crate::screens::components::theme::{BRIGHT_PALETTE, NO_COLOR_ENV, THEME_ENV};

    /// Environment lookup built from the given pairs.
    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn reads_real_files_and_tolerates_missing_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pmp")).unwrap();
        std::fs::write(
            dir.path().join("pmp/config.toml"),
            "[appearance]\nascii = true\n",
        )
        .unwrap();

        let config = Config::from_dir(dir.path(), no_env);

        assert!(config.appearance.ascii);
        assert_eq!(config.mcp.timeout, DEFAULT_MCP_TIMEOUT);

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(Config::from_dir(empty.path(), no_env), Config::default());
    }

    #[test]
    fn defaults_when_nothing_is_configured() {
        let config = Config::from_sources(None, None, no_env);

        assert_eq!(config.appearance, AppearanceConfig::default());
        assert_eq!(config.database.busy, BusyOptions::default());
        assert_eq!(config.mcp.timeout, DEFAULT_MCP_TIMEOUT);
        assert_eq!(config.logging.level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.logging.file, None);
        assert!(config.agents.is_empty());
    }

    #[test]
    fn every_section_is_parsed() {
        let file = r##"
            [appearance]
            palette = "bright"
            colors = false
            ascii = true
            reduced_motion = true

            [appearance.tokens]
            accent = "#7aa2f7"

            [database]
            busy_timeout_ms = 1500
            retry_attempts = 4
            retry_base_delay_ms = 7
            retry_max_delay_ms = 90

            [mcp]
            timeout_secs = 12

            [logging]
            level = "DEBUG"
            file = " pmp.log "

            [agents.claude]
            command = "claude-code-acp"
            args = ["--stdio"]
            env = { KEY = "value" }
        "##;
        let config = Config::from_sources(Some(file), None, no_env);

        assert_eq!(config.appearance.palette.as_deref(), Some(BRIGHT_PALETTE));
        assert!(!config.appearance.colors_enabled);
        assert!(config.appearance.ascii);
        assert!(config.appearance.reduced_motion);
        assert_eq!(
            config.appearance.tokens.get("accent").map(String::as_str),
            Some("#7aa2f7")
        );
        assert_eq!(
            config.database.busy.busy_timeout,
            Duration::from_millis(1_500)
        );
        assert_eq!(config.database.busy.max_attempts, 4);
        assert_eq!(config.database.busy.base_delay, Duration::from_millis(7));
        assert_eq!(config.database.busy.max_delay, Duration::from_millis(90));
        assert_eq!(config.mcp.timeout, Duration::from_secs(12));
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.file.as_deref(), Some("pmp.log"));

        let claude = &config.agents["claude"];
        assert_eq!(claude.command, "claude-code-acp");
        assert_eq!(claude.args, vec!["--stdio".to_string()]);
        assert_eq!(claude.env.get("KEY").map(String::as_str), Some("value"));
    }

    #[test]
    fn legacy_theme_file_is_merged_below_config() {
        let theme_file =
            "palette = \"bright\"\n[colors]\naccent = \"red\"\nmuted = \"dark gray\"\n";
        let config_file =
            "[appearance]\npalette = \"default\"\n[appearance.tokens]\naccent = \"blue\"\n";

        let config = Config::from_sources(Some(config_file), Some(theme_file), no_env);

        assert_eq!(config.appearance.palette.as_deref(), Some("default"));
        // config.toml wins for the shared token, legacy tokens survive.
        assert_eq!(
            config.appearance.tokens.get("accent").map(String::as_str),
            Some("blue")
        );
        assert_eq!(
            config.appearance.tokens.get("muted").map(String::as_str),
            Some("dark gray")
        );
    }

    #[test]
    fn environment_wins_over_both_files() {
        let theme_file = "palette = \"bright\"\n";
        let config_file = r#"
            [appearance]
            palette = "bright"
            colors = false
            ascii = false
            reduced_motion = true

            [database]
            busy_timeout_ms = 1500
            retry_attempts = 4

            [mcp]
            timeout_secs = 12
        "#;
        let env = lookup(&[
            (THEME_ENV, "default"),
            (NO_COLOR_ENV, ""),
            (ASCII_ENV, "1"),
            (REDUCED_MOTION_ENV, "0"),
            (BUSY_TIMEOUT_ENV, "800"),
            (RETRY_ATTEMPTS_ENV, "9"),
            (RETRY_BASE_DELAY_ENV, "5"),
            (RETRY_MAX_DELAY_ENV, "80"),
            (MCP_TIMEOUT_ENV, "45"),
        ]);

        let config = Config::from_sources(Some(config_file), Some(theme_file), env);

        assert_eq!(config.appearance.palette.as_deref(), Some("default"));
        // An empty NO_COLOR does not override the file.
        assert!(!config.appearance.colors_enabled);
        assert!(config.appearance.ascii);
        // "0" explicitly disables reduced motion.
        assert!(!config.appearance.reduced_motion);
        assert_eq!(
            config.database.busy.busy_timeout,
            Duration::from_millis(800)
        );
        assert_eq!(config.database.busy.max_attempts, 9);
        assert_eq!(config.database.busy.base_delay, Duration::from_millis(5));
        assert_eq!(config.database.busy.max_delay, Duration::from_millis(80));
        assert_eq!(config.mcp.timeout, Duration::from_secs(45));
    }

    #[test]
    fn no_color_disables_colors_on_top_of_the_file() {
        let config_file = "[appearance]\ncolors = true\n";
        let env = lookup(&[(NO_COLOR_ENV, "1")]);

        let config = Config::from_sources(Some(config_file), None, env);

        assert!(!config.appearance.colors_enabled);
    }

    #[test]
    fn malformed_config_file_keeps_the_defaults() {
        let config = Config::from_sources(Some("[database\nbusy_timeout_ms ="), None, no_env);

        assert_eq!(config, Config::default());
    }

    #[test]
    fn invalid_values_fall_back_instead_of_aborting() {
        let file = r#"
            [database]
            busy_timeout_ms = 0
            retry_attempts = 100000
            retry_base_delay_ms = 0

            [mcp]
            timeout_secs = 0

            [logging]
            level = "loud"
            file = "   "

            [agents.bad]
            args = ["x"]

            [agents.good]
            command = " agent "
        "#;
        let env = lookup(&[
            (BUSY_TIMEOUT_ENV, "soon"),
            (RETRY_ATTEMPTS_ENV, "0"),
            (MCP_TIMEOUT_ENV, "-3"),
        ]);

        let config = Config::from_sources(Some(file), None, env);

        assert_eq!(config.database.busy.busy_timeout, DEFAULT_BUSY_TIMEOUT);
        assert_eq!(config.database.busy.max_attempts, DEFAULT_ATTEMPTS);
        assert_eq!(config.mcp.timeout, DEFAULT_MCP_TIMEOUT);
        assert_eq!(config.logging.level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.logging.file, None);
        assert_eq!(
            config.agents.keys().collect::<Vec<_>>(),
            vec![&"good".to_string()],
            "the profile without a command is skipped"
        );
        assert_eq!(config.agents["good"].command, "agent");
    }

    #[test]
    fn theme_file_still_works_without_config_toml() {
        let theme_file = "palette = \"bright\"\n[colors]\nstatus_done = \"red\"\n";

        let config = Config::from_sources(None, Some(theme_file), no_env);

        assert_eq!(config.appearance.palette.as_deref(), Some(BRIGHT_PALETTE));
        assert_eq!(
            config
                .appearance
                .tokens
                .get("status_done")
                .map(String::as_str),
            Some("red")
        );
    }
}
