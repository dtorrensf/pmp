// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

use crate::domain::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VersionStatus {
    Backlog,
    Open,
    Released,
}

impl From<VersionStatus> for &'static str {
    fn from(status: VersionStatus) -> Self {
        status.as_str()
    }
}

impl TryFrom<&str> for VersionStatus {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "backlog" => Ok(VersionStatus::Backlog),
            "open" => Ok(VersionStatus::Open),
            "released" => Ok(VersionStatus::Released),
            _ => Err(AppError::Validation(format!(
                "invalid version status: {value}"
            ))),
        }
    }
}

impl VersionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VersionStatus::Backlog => "backlog",
            VersionStatus::Open => "open",
            VersionStatus::Released => "released",
        }
    }

    /// Wrapping cycle `Backlog -> Open -> Released -> Backlog`.
    pub fn next(self) -> VersionStatus {
        match self {
            VersionStatus::Backlog => VersionStatus::Open,
            VersionStatus::Open => VersionStatus::Released,
            VersionStatus::Released => VersionStatus::Backlog,
        }
    }

    /// Reverse wrapping cycle `Backlog <- Open <- Released <- Backlog`.
    pub fn prev(self) -> VersionStatus {
        match self {
            VersionStatus::Backlog => VersionStatus::Released,
            VersionStatus::Open => VersionStatus::Backlog,
            VersionStatus::Released => VersionStatus::Open,
        }
    }

    pub fn all() -> [VersionStatus; 3] {
        [
            VersionStatus::Backlog,
            VersionStatus::Open,
            VersionStatus::Released,
        ]
    }

    /// Position used when listing versions: active (`Open`) versions first,
    /// then planned (`Backlog`) ones, then `Released` ones.
    pub fn list_rank(&self) -> u8 {
        match self {
            VersionStatus::Open => 0,
            VersionStatus::Backlog => 1,
            VersionStatus::Released => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            VersionStatus::Backlog => "Backlog",
            VersionStatus::Open => "Open",
            VersionStatus::Released => "Released",
        }
    }
}

/// Name of the version every project receives on creation (the same legacy
/// bucket the V3 migration creates for existing projects).
pub const DEFAULT_VERSION_NAME: &str = "v0";

/// Name suggested when a project has no valid semver version yet.
pub const FIRST_VERSION_NAME: &str = "v0.1.0";

/// Suggests the next version name: the highest valid semver name among
/// `existing` with its minor bumped (`v1.2.3` -> `v1.3.0`). Names that are not
/// semver (such as the legacy `v0`) are ignored; with none valid, returns
/// `v0.1.0`. The optional `v` prefix is accepted when parsing.
pub fn next_version_name<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    let highest = existing
        .into_iter()
        .filter_map(|name| semver::Version::parse(name.strip_prefix('v').unwrap_or(name)).ok())
        .max();
    match highest {
        Some(v) => format!("v{}.{}.0", v.major, v.minor + 1),
        None => FIRST_VERSION_NAME.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub id: Option<i64>,
    pub project_id: i64,
    pub name: String,
    pub description: String,
    pub status: VersionStatus,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    fn sample() -> Version {
        Version {
            id: None,
            project_id: 1,
            name: "v0.1.0".to_string(),
            description: "First release".to_string(),
            status: VersionStatus::Open,
            created_at: "2026-10-18T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn version_fields_match_spec() {
        let version = sample();
        assert!(version.id.is_none());
        assert_eq!(version.project_id, 1);
        assert_eq!(version.name, "v0.1.0");
        assert_eq!(version.description, "First release");
        assert_eq!(version.status, VersionStatus::Open);
        assert_eq!(version.created_at, "2026-10-18T00:00:00Z");
    }

    #[test]
    fn version_with_id_is_some() {
        let version = Version {
            id: Some(42),
            ..sample()
        };
        assert_eq!(version.id, Some(42));
    }

    #[test]
    fn version_serializes_and_deserializes() {
        let version = Version {
            id: Some(3),
            ..sample()
        };
        let json = serde_json::to_string(&version).unwrap();
        let parsed: Version = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, version);
    }

    #[test]
    fn version_status_serializes_to_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&VersionStatus::Backlog).unwrap(),
            "\"backlog\""
        );
        assert_eq!(
            serde_json::to_string(&VersionStatus::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&VersionStatus::Released).unwrap(),
            "\"released\""
        );
    }

    #[test]
    fn version_status_round_trips_all_variants() {
        for status in VersionStatus::all() {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: VersionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn version_status_as_str_and_try_from_round_trip() {
        for status in VersionStatus::all() {
            assert_eq!(VersionStatus::try_from(status.as_str()).unwrap(), status);
            assert_eq!(<&str>::from(status), status.as_str());
        }
    }

    #[test]
    fn version_status_try_from_rejects_invalid_values() {
        assert!(matches!(
            VersionStatus::try_from("closed"),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            VersionStatus::try_from(""),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn version_status_next_cycles_through_all_variants() {
        assert_eq!(VersionStatus::Backlog.next(), VersionStatus::Open);
        assert_eq!(VersionStatus::Open.next(), VersionStatus::Released);
        assert_eq!(VersionStatus::Released.next(), VersionStatus::Backlog);
        // The cycle returns to the starting point after every variant.
        let start = VersionStatus::Backlog;
        assert_eq!(start.next().next().next(), start);
    }

    #[test]
    fn version_status_prev_cycles_backwards_through_all_variants() {
        assert_eq!(VersionStatus::Open.prev(), VersionStatus::Backlog);
        assert_eq!(VersionStatus::Released.prev(), VersionStatus::Open);
        assert_eq!(VersionStatus::Backlog.prev(), VersionStatus::Released);
        // The cycle returns to the starting point after every variant.
        let start = VersionStatus::Backlog;
        assert_eq!(start.prev().prev().prev(), start);
    }

    #[test]
    fn version_status_prev_is_the_inverse_of_next() {
        for status in VersionStatus::all() {
            assert_eq!(status.prev().next(), status);
            assert_eq!(status.next().prev(), status);
        }
    }

    #[test]
    fn version_status_all_contains_all_variants() {
        assert_eq!(VersionStatus::all().len(), 3);
        assert!(VersionStatus::all().contains(&VersionStatus::Backlog));
        assert!(VersionStatus::all().contains(&VersionStatus::Open));
        assert!(VersionStatus::all().contains(&VersionStatus::Released));
    }

    #[test]
    fn version_status_list_rank_orders_open_backlog_released() {
        assert!(VersionStatus::Open.list_rank() < VersionStatus::Backlog.list_rank());
        assert!(VersionStatus::Backlog.list_rank() < VersionStatus::Released.list_rank());
    }

    #[test]
    fn version_status_label_returns_human_readable_name() {
        assert_eq!(VersionStatus::Backlog.label(), "Backlog");
        assert_eq!(VersionStatus::Open.label(), "Open");
        assert_eq!(VersionStatus::Released.label(), "Released");
    }

    #[test]
    fn next_version_name_defaults_when_no_semver_exists() {
        assert_eq!(next_version_name([]), "v0.1.0");
        assert_eq!(next_version_name([DEFAULT_VERSION_NAME]), "v0.1.0");
        assert_eq!(next_version_name(["release-candidate"]), "v0.1.0");
    }

    #[test]
    fn next_version_name_bumps_minor_of_highest_version() {
        assert_eq!(next_version_name(["v0.1.0"]), "v0.2.0");
        assert_eq!(next_version_name(["v0.1.0", "v1.2.3", "v0.9.0"]), "v1.3.0");
        assert_eq!(next_version_name(["1.4.0"]), "v1.5.0");
        assert_eq!(next_version_name(["v0", "v2.0.0"]), "v2.1.0");
    }

    #[test]
    fn version_status_impls_json_schema() {
        assert_eq!(VersionStatus::schema_name(), "VersionStatus");
    }
}
