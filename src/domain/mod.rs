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
