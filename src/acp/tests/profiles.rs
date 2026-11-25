// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Profile resolution: the names the user can choose and the errors that tell
//! them what is available when they pick one that is not configured.

use crate::acp::profiles;
use crate::config::Config;
use crate::domain::error::ErrorCode;

fn config(toml: &str) -> Config {
    Config::from_sources(Some(toml), None, |_| None)
}

fn two_profiles_or_one(toml: &str) -> Config {
    config(toml)
}

fn two_profiles() -> Config {
    config(
        r#"
[agents.claude]
command = "claude-code-acp"
args = ["--stdio"]
env = { ANTHROPIC_API_KEY = "secret" }

[agents.gemini]
command = "gemini --acp"
"#,
    )
}

#[test]
fn resolve_returns_the_profile_with_its_command_args_and_env() {
    let config = two_profiles();
    let profile = profiles::resolve(&config, "claude").expect("claude is configured");
    assert_eq!(profile.command, "claude-code-acp");
    assert_eq!(profile.args, ["--stdio"]);
    assert_eq!(
        profile.env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("secret")
    );
    assert!(
        profiles::resolve(&config, " gemini ").is_ok(),
        "names are trimmed"
    );
}

#[test]
fn resolve_rejects_an_unknown_name_and_lists_what_exists() {
    let config = two_profiles();
    let error = profiles::resolve(&config, "claud").expect_err("a typo is not a profile");
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(!error.is_retryable());
    let message = error.user_message();
    assert!(
        message.contains("unknown agent profile 'claud'"),
        "{message}"
    );
    assert!(message.contains("claude, gemini"), "{message}");
}

#[test]
fn resolve_rejects_an_empty_name() {
    let config = two_profiles();
    let error = profiles::resolve(&config, "  ").expect_err("a blank name is rejected");
    assert_eq!(error.code(), ErrorCode::Validation);
    assert!(error.user_message().contains("profile name is required"));
}

#[test]
fn resolve_explains_how_to_configure_a_profile() {
    let config = Config::default();
    let error = profiles::resolve(&config, "claude").expect_err("nothing is configured");
    assert_eq!(error.code(), ErrorCode::Validation);
    let message = error.user_message();
    assert!(
        message.contains("no agent profiles are configured"),
        "{message}"
    );
    assert!(message.contains("[agents.claude]"), "{message}");
}

#[test]
fn names_are_sorted_and_default_prefers_the_only_profile() {
    let config = two_profiles();
    assert_eq!(profiles::names(&config), ["claude", "gemini"]);
    assert_eq!(
        profiles::default_name(&config),
        Some("claude"),
        "with several profiles claude is the default"
    );

    let single = two_profiles_or_one(
        r#"
[agents.gemini]
command = "gemini --acp"
"#,
    );
    assert_eq!(profiles::default_name(&single), Some("gemini"));
    assert_eq!(profiles::default_name(&Config::default()), None);
}

#[test]
fn invalid_profiles_are_dropped_by_the_configuration_layer() {
    let config = config(
        r#"
[agents.blank]
args = ["--stdio"]

[agents.empty]
command = "   "

[agents.ok]
command = "agent"
"#,
    );
    assert_eq!(profiles::names(&config), ["ok"]);
    assert_eq!(profiles::default_name(&config), Some("ok"));
}
