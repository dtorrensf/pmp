// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[cfg(test)]
mod priority_param_tests {
    use super::*;

    fn schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap()
    }

    #[test]
    fn create_task_priority_schema_is_plain_string_enum() {
        let schema = schema_for::<CreateTaskParams>();
        let prop = &schema["properties"]["priority"];
        let refd = prop.get("$ref").and_then(|r| r.as_str());
        let def = match refd {
            Some(r) => &schema["$defs"][r.trim_start_matches("#/$defs/")],
            None => prop,
        };
        assert_eq!(def["type"], "string", "priority must be a string: {def}");
        assert_eq!(
            def["enum"],
            serde_json::json!(["Low", "Medium", "High"]),
            "priority must enumerate the three levels: {def}"
        );
        assert!(
            !prop.to_string().contains("null"),
            "priority must not be nullable: {prop}"
        );
        let required = schema["required"].as_array().unwrap();
        assert!(!required.iter().any(|r| r == "priority"));
    }

    #[test]
    fn task_output_priority_schema_matches_its_integer_wire_format() {
        let schema = schema_for::<Task>();
        let prop = &schema["properties"]["priority"];
        let refd = prop.get("$ref").and_then(|r| r.as_str());
        let def = match refd {
            Some(r) => &schema["$defs"][r.trim_start_matches("#/$defs/")],
            None => prop,
        };
        assert_eq!(
            def["type"], "integer",
            "output priority must be numeric: {def}"
        );
        assert_eq!(def["enum"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn create_task_params_accept_named_priority() {
        let params: CreateTaskParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b",
            "priority": "High"
        }))
        .unwrap();
        assert_eq!(Priority::from(params.priority), Priority::High);
    }

    #[test]
    fn create_task_params_default_to_medium_when_omitted() {
        let params: CreateTaskParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b"
        }))
        .unwrap();
        assert_eq!(Priority::from(params.priority), Priority::Medium);
    }

    #[test]
    fn create_task_params_reject_unknown_priority() {
        let err = serde_json::from_value::<CreateTaskParams>(serde_json::json!({
            "project_id": 1, "version_id": 1, "name": "n",
            "context": "c", "requirements": "r", "benefits": "b",
            "priority": "Urgent"
        }))
        .err()
        .expect("unknown priority must be rejected");
        assert!(err.to_string().contains("Urgent"), "{err}");
    }

    #[test]
    fn list_tasks_params_accept_named_priority() {
        let params: ListTasksParams = serde_json::from_value(serde_json::json!({
            "project_id": 1, "priority": "High"
        }))
        .unwrap();
        assert_eq!(params.priority.map(Priority::from), Some(Priority::High));
    }

    #[tokio::test]
    async fn repository_operations_time_out() {
        let repo: Arc<dyn Repository> = Arc::new(crate::db::stub::StubRepository::default());
        let result = execute_repository_with_timeout(
            repo,
            |_| {
                std::thread::sleep(Duration::from_millis(200));
                Ok(())
            },
            Duration::from_millis(10),
        )
        .await;
        let err = result.expect_err("a blocked operation must fail with a timeout");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    }
}

#[cfg(test)]
mod instructions_tests {
    use super::*;

    #[test]
    fn server_instructions_describe_status_workflow() {
        let repo = Arc::new(crate::db::stub::StubRepository::default());
        let info = PmpServer::new(repo).get_info();
        let text = info.instructions.expect("instructions should be set");
        for needle in ["update_task_status", "in_progress", "done", "add_task_note"] {
            assert!(text.contains(needle), "instructions must mention {needle}");
        }
    }
}
