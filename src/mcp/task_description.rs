// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
pub fn compose_description(context: &str, requirements: &str, benefits: &str) -> String {
    let mut parts = Vec::new();

    if !context.trim().is_empty() {
        parts.push(format!("## Context\n\n{}", context.trim()));
    }
    if !requirements.trim().is_empty() {
        parts.push(format!("## Requirements\n\n{}", requirements.trim()));
    }
    if !benefits.trim().is_empty() {
        parts.push(format!("## Benefits\n\n{}", benefits.trim()));
    }

    if parts.is_empty() {
        return String::new();
    }

    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::compose_description;

    #[test]
    fn composes_all_sections() {
        let description = compose_description(
            "We need to track this.",
            "- Must be visible\n- Must be editable",
            "Better organization.",
        );
        assert_eq!(
            description,
            "## Context\n\nWe need to track this.\n\n## Requirements\n\n- Must be visible\n- Must be editable\n\n## Benefits\n\nBetter organization."
        );
    }

    #[test]
    fn skips_empty_sections() {
        let description = compose_description("", "- One requirement", "");
        assert_eq!(description, "## Requirements\n\n- One requirement");
    }

    #[test]
    fn trims_whitespace() {
        let description = compose_description("  context  ", "  req  ", "  benefit  ");
        assert_eq!(
            description,
            "## Context\n\ncontext\n\n## Requirements\n\nreq\n\n## Benefits\n\nbenefit"
        );
    }

    #[test]
    fn all_empty_returns_empty_string() {
        let description = compose_description("", "", "");
        assert!(description.is_empty());
    }
}
