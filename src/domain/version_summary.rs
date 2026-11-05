// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// Frozen summary of a compacted version.
///
/// A version may hold at most one summary (`UNIQUE(version_id)`): once it is
/// compacted, its tasks live only inside `snapshot_json` and the human readable
/// prose is kept in `summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VersionSummary {
    pub id: Option<i64>,
    pub version_id: i64,
    pub summary: String,
    pub snapshot_json: String,
    pub generated_at: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VersionSummary {
        VersionSummary {
            id: None,
            version_id: 1,
            summary: "First release summary".to_string(),
            snapshot_json: "{\"tasks\":[]}".to_string(),
            generated_at: "2026-10-18T00:00:00Z".to_string(),
            created_at: "2026-10-18T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn version_summary_fields_match_spec() {
        let summary = sample();
        assert!(summary.id.is_none());
        assert_eq!(summary.version_id, 1);
        assert_eq!(summary.summary, "First release summary");
        assert_eq!(summary.snapshot_json, "{\"tasks\":[]}");
        assert_eq!(summary.generated_at, "2026-10-18T00:00:00Z");
        assert_eq!(summary.created_at, "2026-10-18T00:00:00Z");
    }

    #[test]
    fn version_summary_with_id_is_some() {
        let summary = VersionSummary {
            id: Some(42),
            ..sample()
        };
        assert_eq!(summary.id, Some(42));
    }

    #[test]
    fn version_summary_serializes_and_deserializes() {
        let summary = VersionSummary {
            id: Some(3),
            ..sample()
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: VersionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, summary);
    }
}
