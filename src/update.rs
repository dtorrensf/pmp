// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Background check for newer pmp releases on GitHub.
//!
//! The check runs at most once every [`CHECK_INTERVAL_SECS`] and never breaks
//! the application: any network, parsing or IO error simply means "no update".

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Command shown to the user to install the newer version.
pub const UPDATE_COMMAND: &str = "cargo install --git https://github.com/dtorrensf/pmp";

/// Minimum time between two calls to the GitHub API.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/dtorrensf/pmp/releases/latest";
const TAG_PREFIX: &str = "pmp-v";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Persisted result of the last update check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCheckState {
    pub last_check_unix: u64,
    pub latest_version: Option<String>,
}

/// Parses a release tag such as `pmp-v0.4.0` into a semver version.
pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    let version = tag.strip_prefix(TAG_PREFIX)?;
    semver::Version::parse(version).ok()
}

/// Returns `true` when `latest` is a strictly newer version than `current`.
/// Unparseable input is never considered newer.
pub fn newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

/// Extracts the `tag_name` field from the GitHub "latest release" JSON body.
pub fn latest_version_from_json(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value.get("tag_name")?.as_str().map(str::to_string)
}

/// Returns `true` when the last check happened less than `interval` ago.
pub fn is_fresh(last_check_unix: u64, now: u64, interval: u64) -> bool {
    now.saturating_sub(last_check_unix) < interval
}

/// Extracts the latest published version (without the tag prefix) from the
/// GitHub "latest release" JSON body.
pub fn latest_version_from_release_body(body: &str) -> Option<String> {
    let tag = latest_version_from_json(body)?;
    Some(parse_tag(&tag)?.to_string())
}

/// Queries `url` (GitHub "latest release" endpoint) for the latest published
/// version, whether or not it is newer than `current` (the comparison happens
/// when the result is shown). Any network, status or parsing error yields `None`.
pub fn fetch_latest_version(url: &str, current: &str) -> Option<String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(format!("pmp/{current}"))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let body = response.body_mut().read_to_string().ok()?;
    latest_version_from_release_body(&body)
}

/// Queries GitHub for the latest published version.
pub fn check_for_update(current: &str) -> Option<String> {
    fetch_latest_version(LATEST_RELEASE_URL, current)
}

/// Starts [`run_update_check`] on a detached background thread and returns the
/// slot that receives the newer version, if any.
pub fn spawn_check(current: &'static str) -> Arc<OnceLock<String>> {
    spawn_with(current, run_update_check)
}

/// Runs `job` on a detached background thread with a fresh slot. Split out so
/// tests can drive the threading without touching the network.
fn spawn_with<F>(current: &'static str, job: F) -> Arc<OnceLock<String>>
where
    F: FnOnce(&str, &Arc<OnceLock<String>>) + Send + 'static,
{
    let slot = Arc::new(OnceLock::new());
    let thread_slot = slot.clone();
    std::thread::spawn(move || job(current, &thread_slot));
    slot
}

/// Runs the update check against the real clock, data directory and GitHub,
/// publishing a newer version (if any) into `slot`.
pub fn run_update_check(current: &str, slot: &Arc<OnceLock<String>>) {
    run_update_check_with(current, slot, state_path().as_deref(), now_unix(), || {
        check_for_update(current)
    });
}

/// Core of the update check with every external dependency injected:
/// `state_file` is the cache location (`None` disables caching), `now` the
/// current unix time and `fetch` the network lookup. Reuses the cached result
/// while it is fresh; otherwise calls `fetch`, persists the outcome and
/// publishes `latest` into `slot` only when it is newer than `current`.
pub fn run_update_check_with<F>(
    current: &str,
    slot: &Arc<OnceLock<String>>,
    state_file: Option<&Path>,
    now: u64,
    fetch: F,
) where
    F: FnOnce() -> Option<String>,
{
    let latest = match state_file.and_then(load_state) {
        Some(state) if is_fresh(state.last_check_unix, now, CHECK_INTERVAL_SECS) => {
            state.latest_version
        }
        _ => {
            let latest = fetch();
            if let Some(path) = state_file {
                save_state(
                    path,
                    &UpdateCheckState {
                        last_check_unix: now,
                        latest_version: latest.clone(),
                    },
                );
            }
            latest
        }
    };

    if let Some(version) = latest.filter(|v| newer(current, v)) {
        let _ = slot.set(version);
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn state_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("pmp").join("update-check.json"))
}

fn load_state(path: &Path) -> Option<UpdateCheckState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_state(path: &Path, state: &UpdateCheckState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(state) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_strips_prefix_and_parses_semver() {
        assert_eq!(parse_tag("pmp-v0.4.0"), Some(semver::Version::new(0, 4, 0)));
    }

    #[test]
    fn parse_tag_rejects_missing_prefix_or_invalid_semver() {
        assert_eq!(parse_tag("v0.4.0"), None);
        assert_eq!(parse_tag("pmp-v0.4"), None);
        assert_eq!(parse_tag("pmp-vfoo"), None);
        assert_eq!(parse_tag(""), None);
    }

    #[test]
    fn newer_detects_strictly_greater_versions() {
        assert!(newer("0.3.0", "0.4.0"));
        assert!(newer("0.3.0", "1.0.0"));
        assert!(newer("0.3.0", "0.3.1"));
        assert!(!newer("0.3.0", "0.3.0"));
        assert!(!newer("0.4.0", "0.3.0"));
    }

    #[test]
    fn newer_is_false_for_unparseable_input() {
        assert!(!newer("0.3.0", "garbage"));
        assert!(!newer("garbage", "0.4.0"));
    }

    #[test]
    fn latest_version_from_json_extracts_tag_name() {
        let body = r#"{"tag_name":"pmp-v0.4.0","name":"0.4.0","draft":false}"#;
        assert_eq!(
            latest_version_from_json(body),
            Some("pmp-v0.4.0".to_string())
        );
    }

    #[test]
    fn latest_version_from_json_returns_none_on_bad_payload() {
        assert_eq!(latest_version_from_json("not json"), None);
        assert_eq!(latest_version_from_json(r#"{"name":"x"}"#), None);
        assert_eq!(latest_version_from_json(r#"{"tag_name":42}"#), None);
    }

    #[test]
    fn latest_version_from_release_body_returns_published_version() {
        let body = r#"{"tag_name":"pmp-v0.3.0"}"#;
        assert_eq!(
            latest_version_from_release_body(body),
            Some("0.3.0".to_string())
        );
        assert_eq!(
            latest_version_from_release_body(r#"{"tag_name":"x"}"#),
            None
        );
    }

    #[test]
    fn is_fresh_true_within_interval() {
        assert!(is_fresh(1_000, 1_000, 100));
        assert!(is_fresh(1_000, 1_099, 100));
    }

    #[test]
    fn is_fresh_false_once_interval_elapsed() {
        assert!(!is_fresh(1_000, 1_100, 100));
        assert!(!is_fresh(1_000, 5_000, 100));
    }

    #[test]
    fn is_fresh_treats_future_timestamps_as_fresh() {
        assert!(is_fresh(2_000, 1_000, 100));
    }

    #[test]
    fn update_check_state_round_trips_through_json() {
        let state = UpdateCheckState {
            last_check_unix: 42,
            latest_version: Some("0.4.0".to_string()),
        };
        let text = serde_json::to_string(&state).unwrap();
        let back: UpdateCheckState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn update_command_points_to_repository() {
        assert_eq!(
            UPDATE_COMMAND,
            "cargo install --git https://github.com/dtorrensf/pmp"
        );
    }

    use std::cell::Cell;
    use std::net::TcpListener;

    /// Registers the "latest release" endpoint on the mock server. It only
    /// matches requests carrying the headers that `fetch_latest_version` must
    /// send, so a missing `Accept` or `User-Agent` makes the test fail.
    fn mock_latest_release(
        server: &mut mockito::ServerGuard,
        status: usize,
        body: &str,
    ) -> mockito::Mock {
        server
            .mock("GET", "/latest")
            .match_header("accept", "application/vnd.github+json")
            .match_header("user-agent", "pmp/0.3.0")
            .with_status(status)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    /// URL on a local port with nothing listening (connection refused).
    fn closed_port_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}/latest")
    }

    #[test]
    fn fetch_latest_version_parses_successful_response() {
        let mut server = mockito::Server::new();
        let mock = mock_latest_release(&mut server, 200, r#"{"tag_name":"pmp-v0.4.0"}"#);

        let url = format!("{}/latest", server.url());

        assert_eq!(
            fetch_latest_version(&url, "0.3.0"),
            Some("0.4.0".to_string())
        );
        mock.assert();
    }

    #[test]
    fn fetch_latest_version_returns_published_version_even_if_not_newer() {
        let mut server = mockito::Server::new();
        let mock = mock_latest_release(&mut server, 200, r#"{"tag_name":"pmp-v0.3.0"}"#);

        let url = format!("{}/latest", server.url());

        assert_eq!(
            fetch_latest_version(&url, "0.3.0"),
            Some("0.3.0".to_string())
        );
        mock.assert();
    }

    #[test]
    fn fetch_latest_version_returns_none_on_http_error_status() {
        let mut server = mockito::Server::new();
        let mock = mock_latest_release(&mut server, 500, r#"{"tag_name":"pmp-v9.0.0"}"#);

        let url = format!("{}/latest", server.url());

        assert_eq!(fetch_latest_version(&url, "0.3.0"), None);
        mock.assert();
    }

    #[test]
    fn fetch_latest_version_returns_none_on_invalid_payload() {
        let mut server = mockito::Server::new();
        let mock = mock_latest_release(&mut server, 200, "<html>not json</html>");

        let url = format!("{}/latest", server.url());

        assert_eq!(fetch_latest_version(&url, "0.3.0"), None);
        mock.assert();
    }

    #[test]
    fn fetch_latest_version_returns_none_on_unreachable_host() {
        assert_eq!(fetch_latest_version(&closed_port_url(), "0.3.0"), None);
    }

    #[test]
    fn spawn_with_runs_job_on_background_thread_and_fills_slot() {
        let slot = spawn_with("0.2.0", |current, slot| {
            let _ = slot.set(format!("from {current}"));
        });
        let start = std::time::Instant::now();
        while slot.get().is_none() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(slot.get().map(String::as_str), Some("from 0.2.0"));
    }

    #[test]
    fn now_unix_returns_a_plausible_timestamp() {
        assert!(now_unix() > 1_700_000_000);
    }

    #[test]
    fn state_path_is_inside_pmp_data_dir() {
        if let Some(path) = state_path() {
            assert!(path.ends_with("pmp/update-check.json"));
        }
    }

    #[test]
    fn load_state_returns_none_when_file_is_missing_or_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        assert_eq!(load_state(&missing), None);

        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{not json").unwrap();
        assert_eq!(load_state(&corrupt), None);
    }

    #[test]
    fn save_state_creates_parent_dirs_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("update-check.json");
        let state = UpdateCheckState {
            last_check_unix: 7,
            latest_version: None,
        };

        save_state(&path, &state);

        assert_eq!(load_state(&path), Some(state));
    }

    fn empty_slot() -> Arc<OnceLock<String>> {
        Arc::new(OnceLock::new())
    }

    #[test]
    fn run_update_check_uses_fresh_cache_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        save_state(
            &path,
            &UpdateCheckState {
                last_check_unix: 1_000,
                latest_version: Some("0.4.0".to_string()),
            },
        );
        let slot = empty_slot();

        run_update_check_with("0.3.0", &slot, Some(&path), 1_100, || {
            panic!("fresh cache must not hit the network")
        });

        assert_eq!(slot.get().map(String::as_str), Some("0.4.0"));
    }

    #[test]
    fn run_update_check_fresh_cache_without_newer_version_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        save_state(
            &path,
            &UpdateCheckState {
                last_check_unix: 1_000,
                latest_version: Some("0.3.0".to_string()),
            },
        );
        let slot = empty_slot();

        run_update_check_with("0.3.0", &slot, Some(&path), 1_100, || {
            panic!("fresh cache must not hit the network")
        });

        assert_eq!(slot.get(), None);
    }

    #[test]
    fn run_update_check_fresh_failed_check_is_not_retried_within_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        save_state(
            &path,
            &UpdateCheckState {
                last_check_unix: 1_000,
                latest_version: None,
            },
        );
        let slot = empty_slot();

        run_update_check_with("0.3.0", &slot, Some(&path), 1_100, || {
            panic!("a failed check inside the interval must not be retried")
        });

        assert_eq!(slot.get(), None);
    }

    #[test]
    fn run_update_check_stale_cache_fetches_persists_and_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        save_state(
            &path,
            &UpdateCheckState {
                last_check_unix: 0,
                latest_version: Some("0.1.0".to_string()),
            },
        );
        let slot = empty_slot();
        let calls = Cell::new(0);

        run_update_check_with("0.3.0", &slot, Some(&path), CHECK_INTERVAL_SECS + 1, || {
            calls.set(calls.get() + 1);
            Some("0.5.0".to_string())
        });

        assert_eq!(calls.get(), 1);
        assert_eq!(slot.get().map(String::as_str), Some("0.5.0"));
        assert_eq!(
            load_state(&path),
            Some(UpdateCheckState {
                last_check_unix: CHECK_INTERVAL_SECS + 1,
                latest_version: Some("0.5.0".to_string()),
            })
        );
    }

    #[test]
    fn run_update_check_without_cache_file_fetches_and_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmp").join("update-check.json");
        let slot = empty_slot();

        run_update_check_with("0.3.0", &slot, Some(&path), 500, || {
            Some("0.3.0".to_string())
        });

        assert_eq!(slot.get(), None, "same version is not an update");
        assert_eq!(
            load_state(&path),
            Some(UpdateCheckState {
                last_check_unix: 500,
                latest_version: Some("0.3.0".to_string()),
            })
        );
    }

    #[test]
    fn run_update_check_corrupt_cache_is_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        std::fs::write(&path, "garbage").unwrap();
        let slot = empty_slot();
        let calls = Cell::new(0);

        run_update_check_with("0.3.0", &slot, Some(&path), 500, || {
            calls.set(calls.get() + 1);
            Some("0.4.0".to_string())
        });

        assert_eq!(calls.get(), 1);
        assert_eq!(slot.get().map(String::as_str), Some("0.4.0"));
    }

    #[test]
    fn run_update_check_failed_fetch_persists_none_and_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        let slot = empty_slot();

        run_update_check_with("0.3.0", &slot, Some(&path), 500, || None);

        assert_eq!(slot.get(), None);
        assert_eq!(
            load_state(&path),
            Some(UpdateCheckState {
                last_check_unix: 500,
                latest_version: None,
            })
        );
    }

    #[test]
    fn run_update_check_without_state_file_still_fetches_and_publishes() {
        let slot = empty_slot();
        let calls = Cell::new(0);

        run_update_check_with("0.3.0", &slot, None, 500, || {
            calls.set(calls.get() + 1);
            Some("0.4.0".to_string())
        });

        assert_eq!(calls.get(), 1);
        assert_eq!(slot.get().map(String::as_str), Some("0.4.0"));
    }
}
