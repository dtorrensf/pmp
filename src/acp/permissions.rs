// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Permission requests from an agent (`session/request_permission`).
//!
//! An agent asks before it runs a command or changes a file outside its
//! default policy. PMP parses the request, lets the user decide (C4 modal in
//! the TUI) and answers with the option the decision maps to. Three rules
//! shape the answer:
//!
//! * **Deny by default.** Without a handler, or when the user does not answer,
//!   the answer is `cancelled`, which never leaves the agent waiting.
//! * **Options come from the agent.** PMP selects the `optionId` the agent
//!   offered for the decision, never an id it made up.
//! * **Always is bounded.** "Allow always" stores a rule per project, agent
//!   and tool kind, up to [`MAX_RULES`]. Rules only ever allow what the user
//!   named; nothing matches a broader pattern.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

use super::protocol::{PermissionOutcome, SessionId};
use crate::domain::error::AppError;

/// Largest number of "allow always" rules kept at once.
pub const MAX_RULES: usize = 64;

/// Tool kind used when the agent does not say what the tool does.
pub const UNKNOWN_KIND: &str = "other";

/// One permission option the agent offered, as sent in the request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// A parsed `session/request_permission`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub session_id: SessionId,
    /// What the agent wants to do, in its own words.
    pub title: String,
    /// The tool category (`execute`, `edit`, `read`, ...), or
    /// [`UNKNOWN_KIND`]. Allow-always rules are keyed by this value.
    pub tool_kind: String,
    pub options: Vec<PermissionOption>,
}

/// What the user decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRequest {
    session_id: SessionId,
    tool_call: RawToolCall,
    options: Vec<PermissionOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawToolCall {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

impl PermissionRequest {
    /// Parses the params of a `session/request_permission` request. A request
    /// without options cannot be answered with a selection, so it is rejected.
    pub fn from_params(params: Value) -> Result<Self, serde_json::Error> {
        let raw: RawRequest = serde_json::from_value(params)?;
        Ok(Self {
            session_id: raw.session_id,
            title: raw
                .tool_call
                .title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "The agent asks for permission".to_string()),
            tool_kind: raw
                .tool_call
                .kind
                .filter(|kind| !kind.trim().is_empty())
                .unwrap_or_else(|| UNKNOWN_KIND.to_string()),
            options: raw.options,
        })
    }

    /// The answer for `decision`.
    ///
    /// The option kinds are tried in order, so the answer never escalates:
    /// `AllowOnce` only selects `allow_once`; `AllowAlways` falls back to
    /// `allow_once` when the agent offers no persistent option; `Deny` selects
    /// `reject_once`, then `reject_always`. When no suitable option exists the
    /// answer is `cancelled`.
    pub fn outcome(&self, decision: Decision) -> PermissionOutcome {
        let preferred: &[&str] = match decision {
            Decision::AllowOnce => &["allow_once"],
            Decision::AllowAlways => &["allow_always", "allow_once"],
            Decision::Deny => &["reject_once", "reject_always"],
        };
        preferred
            .iter()
            .find_map(|kind| self.options.iter().find(|option| option.kind == *kind))
            .map_or(PermissionOutcome::Cancelled, |option| {
                PermissionOutcome::Selected {
                    option_id: option.option_id.clone(),
                }
            })
    }
}

/// A standing "allow always" grant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    pub project_id: i64,
    pub agent: String,
    pub tool_kind: String,
}

/// The bounded set of standing grants, held by the TUI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowlist {
    rules: BTreeSet<Rule>,
}

impl Allowlist {
    /// Whether a request of this kind is already granted for the project and
    /// agent.
    pub fn allows(&self, project_id: i64, agent: &str, tool_kind: &str) -> bool {
        self.rules.contains(&Rule {
            project_id,
            agent: agent.to_string(),
            tool_kind: tool_kind.to_string(),
        })
    }

    /// Stores a grant. Storing one that already exists is a no-op; a new one
    /// past [`MAX_RULES`] is refused with a validation error the user can
    /// read.
    pub fn allow_always(&mut self, rule: Rule) -> Result<(), AppError> {
        if self.rules.contains(&rule) {
            return Ok(());
        }
        if self.rules.len() >= MAX_RULES {
            return Err(AppError::Validation(format!(
                "the allowlist holds at most {MAX_RULES} rules; allow this once instead"
            )));
        }
        self.rules.insert(rule);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn params(options: Value, kind: Option<&str>) -> Value {
        let mut tool_call = json!({ "toolCallId": "call-1", "title": "Run cargo test" });
        if let Some(kind) = kind {
            tool_call["kind"] = json!(kind);
        }
        json!({ "sessionId": "s-1", "toolCall": tool_call, "options": options })
    }

    fn standard_options() -> Value {
        json!([
            { "optionId": "ok", "name": "Allow", "kind": "allow_once" },
            { "optionId": "always", "name": "Always", "kind": "allow_always" },
            { "optionId": "no", "name": "Reject", "kind": "reject_once" },
        ])
    }

    fn selected(id: &str) -> PermissionOutcome {
        PermissionOutcome::Selected {
            option_id: id.to_string(),
        }
    }

    #[test]
    fn parses_title_kind_and_options() {
        let request =
            PermissionRequest::from_params(params(standard_options(), Some("execute"))).unwrap();
        assert_eq!(request.session_id, SessionId::from("s-1"));
        assert_eq!(request.title, "Run cargo test");
        assert_eq!(request.tool_kind, "execute");
        assert_eq!(request.options.len(), 3);
    }

    #[test]
    fn missing_kind_and_title_fall_back_to_defaults() {
        let value = json!({
            "sessionId": "s-1",
            "toolCall": { "toolCallId": "c", "title": "  " },
            "options": standard_options(),
        });
        let request = PermissionRequest::from_params(value).unwrap();
        assert_eq!(request.tool_kind, UNKNOWN_KIND);
        assert_eq!(request.title, "The agent asks for permission");
    }

    #[test]
    fn a_request_without_options_is_rejected() {
        let value = json!({ "sessionId": "s-1", "toolCall": { "toolCallId": "c" } });
        assert!(PermissionRequest::from_params(value).is_err());
    }

    #[test]
    fn decisions_select_the_option_the_agent_offered() {
        let request =
            PermissionRequest::from_params(params(standard_options(), Some("edit"))).unwrap();
        assert_eq!(request.outcome(Decision::AllowOnce), selected("ok"));
        assert_eq!(request.outcome(Decision::AllowAlways), selected("always"));
        assert_eq!(request.outcome(Decision::Deny), selected("no"));
    }

    #[test]
    fn allow_always_falls_back_to_allow_once_but_never_the_reverse() {
        let options = json!([{ "optionId": "ok", "name": "Allow", "kind": "allow_once" }]);
        let request = PermissionRequest::from_params(params(options, None)).unwrap();
        assert_eq!(request.outcome(Decision::AllowAlways), selected("ok"));

        let only_always = json!([{ "optionId": "always", "name": "A", "kind": "allow_always" }]);
        let request = PermissionRequest::from_params(params(only_always, None)).unwrap();
        assert_eq!(
            request.outcome(Decision::AllowOnce),
            PermissionOutcome::Cancelled
        );
    }

    #[test]
    fn deny_falls_back_to_reject_always_and_then_cancels() {
        let only_always_reject =
            json!([{ "optionId": "never", "name": "Never", "kind": "reject_always" }]);
        let request = PermissionRequest::from_params(params(only_always_reject, None)).unwrap();
        assert_eq!(request.outcome(Decision::Deny), selected("never"));

        let allow_only = json!([{ "optionId": "ok", "name": "Allow", "kind": "allow_once" }]);
        let request = PermissionRequest::from_params(params(allow_only, None)).unwrap();
        assert_eq!(
            request.outcome(Decision::Deny),
            PermissionOutcome::Cancelled
        );
    }

    #[test]
    fn allowlist_matches_only_the_exact_project_agent_and_kind() {
        let mut allowlist = Allowlist::default();
        allowlist
            .allow_always(Rule {
                project_id: 3,
                agent: "claude".into(),
                tool_kind: "edit".into(),
            })
            .unwrap();
        assert!(allowlist.allows(3, "claude", "edit"));
        assert!(!allowlist.allows(4, "claude", "edit"), "other project");
        assert!(!allowlist.allows(3, "codex", "edit"), "other agent");
        assert!(!allowlist.allows(3, "claude", "execute"), "other kind");
    }

    #[test]
    fn allowlist_is_bounded_and_duplicates_do_not_count() {
        let mut allowlist = Allowlist::default();
        for index in 0..MAX_RULES {
            allowlist
                .allow_always(Rule {
                    project_id: 1,
                    agent: format!("agent-{index}"),
                    tool_kind: "edit".into(),
                })
                .unwrap();
        }
        // Re-adding an existing rule is fine even when full.
        allowlist
            .allow_always(Rule {
                project_id: 1,
                agent: "agent-0".into(),
                tool_kind: "edit".into(),
            })
            .unwrap();
        assert_eq!(allowlist.len(), MAX_RULES);

        let refused = allowlist.allow_always(Rule {
            project_id: 1,
            agent: "one-too-many".into(),
            tool_kind: "edit".into(),
        });
        assert!(matches!(refused, Err(AppError::Validation(_))));
        assert_eq!(allowlist.len(), MAX_RULES);
        assert!(!allowlist.is_empty());
    }
}
