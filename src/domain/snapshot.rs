// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

use crate::domain::task::{Task, TaskDependency, TaskNote};
use crate::domain::version::Version;
use crate::domain::version_summary::VersionSummary;

/// Frozen point-in-time representation of a version, built before compaction.
///
/// `previous_summary` carries the summary of the prior compaction when it
/// exists; because it embeds its own `snapshot_json`, recompacting a version
/// preserves the full history recursively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactSnapshot {
    pub version: Version,
    pub previous_summary: Option<VersionSummary>,
    pub tasks: Vec<Task>,
    pub notes: Vec<TaskNote>,
    pub dependencies: Vec<TaskDependency>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::{Priority, Status};
    use crate::domain::version::VersionStatus;

    #[test]
    fn snapshot_serializes_and_deserializes() {
        let snapshot = CompactSnapshot {
            version: Version {
                id: Some(1),
                project_id: 1,
                name: "v1.0.0".to_string(),
                description: "".to_string(),
                status: VersionStatus::Released,
                created_at: "now".to_string(),
            },
            previous_summary: None,
            tasks: vec![Task {
                id: Some(2),
                project_id: 1,
                version_id: 1,
                name: "T".to_string(),
                description: "".to_string(),
                status: Status::Done,
                priority: Priority::High,
                created_at: "now".to_string(),
                completed_at: Some("now".to_string()),
            }],
            notes: vec![],
            dependencies: vec![],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: CompactSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snapshot);
    }
}
