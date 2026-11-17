// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod error;
pub mod policy;
pub mod project;
pub mod snapshot;
pub mod task;
pub mod validation;
pub mod version;
pub mod version_summary;

#[cfg(test)]
mod tests {
    /// The domain layer must stay free of UI dependencies: styles live in
    /// `crate::screens::components::theme`. This guard fails if a `ratatui`
    /// import creeps back into the domain.
    #[test]
    fn domain_does_not_depend_on_ratatui() {
        let files = [
            "task.rs",
            "version.rs",
            "project.rs",
            "snapshot.rs",
            "version_summary.rs",
            "policy.rs",
            "validation.rs",
            "error.rs",
        ];
        let offenders: Vec<&str> = files
            .into_iter()
            .filter(|name| {
                let source = match *name {
                    "task.rs" => include_str!("task.rs"),
                    "version.rs" => include_str!("version.rs"),
                    "project.rs" => include_str!("project.rs"),
                    "snapshot.rs" => include_str!("snapshot.rs"),
                    "version_summary.rs" => include_str!("version_summary.rs"),
                    "policy.rs" => include_str!("policy.rs"),
                    "validation.rs" => include_str!("validation.rs"),
                    _ => include_str!("error.rs"),
                };
                source.contains("ratatui")
            })
            .collect();

        assert!(
            offenders.is_empty(),
            "domain files must not import ratatui: found it in {offenders:?}"
        );
    }
}

/// Every glyph of the app's icon set must come from the shared glyph module.
///
/// Screens used to hardcode their own icons, which made the UI inconsistent
/// and broke the ASCII fallback. This guard keeps them in sync.
#[cfg(test)]
mod glyph_guard {
    /// Glyphs that the shared set owns, so a screen may not spell them out.
    const RESERVED: &[char] = &[
        '○', '◐', '✔', '▲', '●', '▼', '▸', '⚠', '█', '░', '▌', '▏', '■', '□', '◆', '◇', '▪', '▫',
        '◌', '∅',
    ];

    #[test]
    fn screens_do_not_hardcode_glyphs() {
        let sources = [
            include_str!("../screens/project_list.rs"),
            include_str!("../screens/task_list_view.rs"),
            include_str!("../screens/dashboard.rs"),
            include_str!("../screens/task_form.rs"),
            include_str!("../screens/version_summary.rs"),
            include_str!("../screens/version_dashboard.rs"),
            include_str!("../screens/project_form.rs"),
            include_str!("../screens/version_form.rs"),
            include_str!("../screens/components/fields.rs"),
        ];

        let offenders: Vec<char> = sources
            .iter()
            .flat_map(|source| {
                let code = production_code(source);
                RESERVED
                    .iter()
                    .filter(move |glyph| code.contains(**glyph))
                    .copied()
            })
            .collect();

        assert!(
            offenders.is_empty(),
            "these glyphs must come from the shared glyph set: {offenders:?}"
        );
    }

    /// The part of a source file that renders: comments and the test module
    /// are excluded, since neither draws anything.
    fn production_code(source: &str) -> String {
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        code.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
