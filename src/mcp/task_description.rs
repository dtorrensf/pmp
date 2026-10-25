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

/// The three sections of a task description, as written by `compose_description`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskSections {
    pub context: String,
    pub requirements: String,
    pub benefits: String,
}

/// Inverse of `compose_description`: splits a task description back into its
/// sections so that a partial update can keep the sections it does not touch.
///
/// Sections are recognised by their exact `## Context`, `## Requirements` and
/// `## Benefits` heading lines. Text before the first heading (for example a
/// description written by hand) is treated as context.
pub fn split_description(description: &str) -> TaskSections {
    let mut preamble: Vec<&str> = Vec::new();
    let mut buckets: [Vec<&str>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut current: Option<usize> = None;

    for line in description.lines() {
        match line.trim_end() {
            "## Context" => current = Some(0),
            "## Requirements" => current = Some(1),
            "## Benefits" => current = Some(2),
            _ => match current {
                Some(index) => buckets[index].push(line),
                None => preamble.push(line),
            },
        }
    }

    let context_lines: Vec<&str> = preamble
        .into_iter()
        .chain(buckets[0].iter().copied())
        .collect();
    TaskSections {
        context: join_trimmed(&context_lines),
        requirements: join_trimmed(&buckets[1]),
        benefits: join_trimmed(&buckets[2]),
    }
}

fn join_trimmed(lines: &[&str]) -> String {
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{TaskSections, compose_description, split_description};

    #[test]
    fn split_round_trips_composed_description() {
        let description = compose_description(
            "We need to track this.",
            "- Must be visible\n- Must be editable",
            "Better organization.",
        );
        assert_eq!(
            split_description(&description),
            TaskSections {
                context: "We need to track this.".to_string(),
                requirements: "- Must be visible\n- Must be editable".to_string(),
                benefits: "Better organization.".to_string(),
            }
        );
    }

    #[test]
    fn split_missing_sections_are_empty() {
        let sections = split_description("## Requirements\n\n- One requirement");
        assert_eq!(sections.context, "");
        assert_eq!(sections.requirements, "- One requirement");
        assert_eq!(sections.benefits, "");
    }

    #[test]
    fn split_keeps_hand_written_text_as_context() {
        let sections = split_description("Plain description without headings");
        assert_eq!(sections.context, "Plain description without headings");
        assert_eq!(sections.requirements, "");
        assert_eq!(sections.benefits, "");
    }

    #[test]
    fn split_empty_description_yields_empty_sections() {
        assert_eq!(split_description(""), TaskSections::default());
    }

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
