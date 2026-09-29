use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub created_at: String,
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
        };
        assert!(project.id.is_none());
        assert_eq!(project.name, "Test Project");
        assert_eq!(project.description, "A description");
        assert_eq!(project.created_at, "2026-06-27T00:00:00Z");
    }

    #[test]
    fn project_with_id_is_some() {
        let project = Project {
            id: Some(42),
            name: "Named".to_string(),
            description: "".to_string(),
            created_at: "now".to_string(),
        };
        assert_eq!(project.id, Some(42));
    }
}
