// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub created_at: String,
    /// Absolute, normalized location of the project's workspace on disk.
    /// Projects created before the field existed keep `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_fields_can_be_set() {
        let project = Project {
            id: None,
            name: "Test Project".to_string(),
            description: "A description".to_string(),
            created_at: "2026-06-27T00:00:00Z".to_string(),
            working_dir: Some("/home/me/repo".to_string()),
        };
        assert!(project.id.is_none());
        assert_eq!(project.name, "Test Project");
        assert_eq!(project.description, "A description");
        assert_eq!(project.created_at, "2026-06-27T00:00:00Z");
        assert_eq!(project.working_dir.as_deref(), Some("/home/me/repo"));
    }

    #[test]
    fn project_with_id_is_some() {
        let project = Project {
            id: Some(42),
            name: "Named".to_string(),
            description: "".to_string(),
            created_at: "now".to_string(),
            working_dir: None,
        };
        assert_eq!(project.id, Some(42));
    }
}
