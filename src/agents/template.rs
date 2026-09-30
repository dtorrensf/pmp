// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Result, anyhow};

pub struct AgentSource {
    pub stem: &'static str,
    pub source: &'static str,
}

pub const AGENT_SOURCES: [AgentSource; 4] = [
    AgentSource {
        stem: "pmp-orchestrator",
        source: include_str!("../../agents/pmp-orchestrator.md"),
    },
    AgentSource {
        stem: "pmp-query",
        source: include_str!("../../agents/pmp-query.md"),
    },
    AgentSource {
        stem: "pmp-plan",
        source: include_str!("../../agents/pmp-plan.md"),
    },
    AgentSource {
        stem: "pmp-implement",
        source: include_str!("../../agents/pmp-implement.md"),
    },
];

pub struct Template {
    pub name: String,
    pub description: String,
    pub body: String,
}

impl Template {
    pub fn parse(source: &str) -> Result<Self> {
        let source = source
            .strip_prefix("---\n")
            .ok_or_else(|| anyhow!("template must open with frontmatter delimiter"))?;
        let (front, rest) = source
            .split_once("\n---\n")
            .ok_or_else(|| anyhow!("template must close frontmatter delimiter"))?;

        let mut name = None;
        let mut description = None;
        for line in front.lines() {
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "name" => name = Some(value.to_string()),
                    "description" => description = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        let name = name.ok_or_else(|| anyhow!("template missing name"))?;
        let description = description.ok_or_else(|| anyhow!("template missing description"))?;
        let body = rest.trim_start_matches('\n').to_string();

        Ok(Self {
            name,
            description,
            body,
        })
    }
}

pub fn markdown_frontmatter(
    name: &str,
    description: &str,
    tools: Option<&str>,
    readonly: bool,
) -> String {
    let mut extras = Vec::new();
    if let Some(t) = tools {
        extras.push(format!("tools: {t}"));
    }
    if readonly {
        extras.push("readonly: true".to_string());
    }
    markdown_frontmatter_with_extras(name, description, &extras)
}

pub fn markdown_frontmatter_with_extras(
    name: &str,
    description: &str,
    extras: &[String],
) -> String {
    let mut out = format!("---\nname: {name}\ndescription: {description}\n");
    for extra in extras {
        out.push_str(extra);
        out.push('\n');
    }
    out.push_str("---\n\n");
    out
}

/// Escape a string so it can be safely placed inside a TOML multi-line basic
/// string (`"""..."""`). Backslashes and double quotes are escaped; other
/// control characters (except newlines) are escaped as `\uXXXX` or named
/// escapes.
pub fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push('\n'),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
