// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Agent profiles: the `[agents.<name>]` entries of `config.toml`.
//!
//! A profile is what the ACP client needs to start an agent: the binary, its
//! arguments and extra environment variables. Parsing lives in
//! [`crate::config`] with the rest of the configuration; this module is the
//! one place that turns a profile name chosen by the user (or by a future CLI
//! flag) into a profile, with an actionable error when the name does not
//! exist.
//!
//! Profiles are validation errors when missing: a typo in a name is a user
//! mistake, not a transient failure, so retrying is pointless.

use crate::config::{AgentProfile, Config};
use crate::domain::error::AppError;

/// The sorted names of every configured profile, for menus and help text.
pub fn names(config: &Config) -> Vec<&str> {
    config.agents.keys().map(String::as_str).collect()
}

/// Looks up one profile by name.
///
/// An unknown name or an empty configuration is a
/// [`Validation`](AppError::Validation) error that lists what is available:
/// the message is meant to be read by the user, not logged.
pub fn resolve<'a>(config: &'a Config, name: &str) -> Result<&'a AgentProfile, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "an agent profile name is required".to_string(),
        ));
    }
    if config.agents.is_empty() {
        return Err(AppError::Validation(format!(
            "no agent profiles are configured; add [agents.{name}] with a command to config.toml"
        )));
    }
    config.agents.get(name).ok_or_else(|| {
        AppError::Validation(format!(
            "unknown agent profile '{name}'; configured profiles: {}",
            names(config).join(", ")
        ))
    })
}

/// The default profile: the only one configured, or `claude` when several
/// exist. Used by flows that can run without asking which agent to use.
pub fn default_name(config: &Config) -> Option<&str> {
    if config.agents.len() == 1 {
        return config.agents.keys().next().map(String::as_str);
    }
    config
        .agents
        .get_key_value("claude")
        .map(|(name, _)| name.as_str())
}
