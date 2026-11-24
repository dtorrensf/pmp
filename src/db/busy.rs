// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lock policy for the shared SQLite file.
//!
//! From 1.0.0 the TUI, the MCP server and the agents launched through ACP all
//! open the same `pmp.db`, so a write can find the database locked by another
//! process. SQLite already waits up to `busy_timeout` before failing with
//! `SQLITE_BUSY`; when that window is not enough, [`retry_busy`] runs the whole
//! operation again with a capped exponential backoff and only surfaces
//! [`AppError::DbBusy`] once the budget is exhausted.
//!
//! Only transient lock failures are retried. Validation, not-found or
//! constraint errors are deterministic, so retrying them would just burn the
//! caller's timeout (the MCP server caps every repository call). Reads rely on
//! `busy_timeout` alone: in WAL a reader never blocks a writer and is only
//! delayed by a checkpoint, so they get no retry layer.
//!
//! The policy is read from the environment for now; the unified `config.toml`
//! lands with A3.

use std::time::Duration;

use crate::domain::error::{AppError, ErrorCode};

/// Environment variable with the SQLite `busy_timeout` in milliseconds.
pub const BUSY_TIMEOUT_ENV: &str = "PMP_DB_BUSY_TIMEOUT_MS";
/// Environment variable with the total number of attempts per operation.
pub const RETRY_ATTEMPTS_ENV: &str = "PMP_DB_RETRY_ATTEMPTS";
/// Environment variable with the first backoff delay in milliseconds.
pub const RETRY_BASE_DELAY_ENV: &str = "PMP_DB_RETRY_BASE_DELAY_MS";
/// Environment variable with the backoff ceiling in milliseconds.
pub const RETRY_MAX_DELAY_ENV: &str = "PMP_DB_RETRY_MAX_DELAY_MS";

/// How long SQLite waits for a lock before reporting it as busy. Long enough to
/// absorb a normal TUI or MCP write, short enough that the whole retry budget
/// stays under the MCP repository timeout.
pub const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_millis(2_000);
/// Attempts per operation, counting the first one.
pub const DEFAULT_ATTEMPTS: u32 = 5;
/// Delay before the first retry; it doubles on every subsequent attempt.
pub const DEFAULT_BASE_DELAY: Duration = Duration::from_millis(10);
/// Ceiling for the backoff, so a long budget never sleeps for seconds.
pub const DEFAULT_MAX_DELAY: Duration = Duration::from_millis(250);
/// Upper bound accepted from the environment, to reject absurd budgets.
pub const MAX_ATTEMPTS: u32 = 100;

/// Locking and retry settings for a [`SqliteRepository`](super::sqlite::SqliteRepository).
///
/// The defaults suit a single-user machine where the TUI and one MCP server
/// share the file. The worst case time a write can spend inside the retry loop
/// is `busy_timeout * attempts + sum of delays`: about 10 s with the defaults,
/// comfortably under the MCP repository timeout (30 s by default), which stays
/// the upper bound for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusyOptions {
    /// `PRAGMA busy_timeout`: how long one attempt waits for the lock.
    pub busy_timeout: Duration,
    /// Total attempts per operation, at least one.
    pub max_attempts: u32,
    /// Delay before the first retry.
    pub base_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
}

impl Default for BusyOptions {
    fn default() -> Self {
        Self {
            busy_timeout: DEFAULT_BUSY_TIMEOUT,
            max_attempts: DEFAULT_ATTEMPTS,
            base_delay: DEFAULT_BASE_DELAY,
            max_delay: DEFAULT_MAX_DELAY,
        }
    }
}

impl BusyOptions {
    /// Reads the policy from the process environment, keeping the default for
    /// every value that is missing or invalid. A bad value never prevents the
    /// store from opening.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Same as [`Self::from_env`] but with an injectable lookup, so the parsing
    /// rules are testable without touching the process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let millis = |name: &str| {
            lookup(name)
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|value| *value > 0)
                .map(Duration::from_millis)
        };
        let attempts = lookup(RETRY_ATTEMPTS_ENV)
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|value| (1..=MAX_ATTEMPTS).contains(value));
        Self {
            busy_timeout: millis(BUSY_TIMEOUT_ENV).unwrap_or(defaults.busy_timeout),
            max_attempts: attempts.unwrap_or(defaults.max_attempts),
            base_delay: millis(RETRY_BASE_DELAY_ENV).unwrap_or(defaults.base_delay),
            max_delay: millis(RETRY_MAX_DELAY_ENV).unwrap_or(defaults.max_delay),
        }
    }

    /// Delay to wait after attempt `attempt` (1-based) failed: the base delay
    /// doubled each time, capped at `max_delay`.
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(16);
        let millis = self
            .base_delay
            .as_millis()
            .saturating_mul(1u128 << doublings);
        let capped = millis.min(self.max_delay.as_millis());
        Duration::from_millis(capped.try_into().unwrap_or(u64::MAX))
    }
}

/// Runs `operation`, retrying it while it fails with a transient lock error.
///
/// `operation` must be safe to run from scratch: reads and writes already
/// rolled back by SQLite (single statements and dropped transactions) qualify.
/// The closure is called again after each failed attempt, so it must re-acquire
/// the connection: no lock is held between attempts.
pub(crate) fn retry_busy<T>(
    options: &BusyOptions,
    mut operation: impl FnMut() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if error.code() == ErrorCode::DbBusy && attempt < options.max_attempts => {
                let delay = options.delay_for(attempt);
                tracing::debug!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "database is busy; retrying"
                );
                std::thread::sleep(delay);
            }
            Err(error) => return Err(error),
        }
    }
}

/// True when the error, or any error in its source chain, reports a locked or
/// busy SQLite database. Migration errors arrive wrapped, so the chain is
/// walked instead of matching only the outermost type.
pub(crate) fn is_lock_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(source) = current {
        if let Some(sqlite) = source.downcast_ref::<rusqlite::Error>()
            && AppError::is_lock(sqlite)
        {
            return true;
        }
        current = source.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;

    /// Every value provided, as if all variables were set.
    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    fn instant() -> BusyOptions {
        BusyOptions {
            busy_timeout: Duration::ZERO,
            max_attempts: 3,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    #[test]
    fn defaults_are_safe_and_bounded() {
        let options = BusyOptions::default();
        assert_eq!(options.busy_timeout, DEFAULT_BUSY_TIMEOUT);
        assert!(options.max_attempts >= 2, "one retry must be possible");
        assert!(options.base_delay > Duration::ZERO);
        assert!(options.max_delay >= options.base_delay);
    }

    #[test]
    fn lookup_overrides_every_value() {
        let options = BusyOptions::from_lookup(lookup(&[
            (BUSY_TIMEOUT_ENV, "1200"),
            (RETRY_ATTEMPTS_ENV, "9"),
            (RETRY_BASE_DELAY_ENV, "5"),
            (RETRY_MAX_DELAY_ENV, "80"),
        ]));
        assert_eq!(options.busy_timeout, Duration::from_millis(1_200));
        assert_eq!(options.max_attempts, 9);
        assert_eq!(options.base_delay, Duration::from_millis(5));
        assert_eq!(options.max_delay, Duration::from_millis(80));
    }

    #[test]
    fn invalid_values_fall_back_to_the_defaults() {
        let defaults = BusyOptions::default();
        for invalid in ["", "0", "-1", "soon", "3.5"] {
            let options = BusyOptions::from_lookup(lookup(&[
                (BUSY_TIMEOUT_ENV, invalid),
                (RETRY_ATTEMPTS_ENV, invalid),
                (RETRY_BASE_DELAY_ENV, invalid),
                (RETRY_MAX_DELAY_ENV, invalid),
            ]));
            assert_eq!(options, defaults, "value {invalid:?}");
        }
    }

    #[test]
    fn attempts_are_clamped_to_a_bounded_budget() {
        let options = BusyOptions::from_lookup(lookup(&[(RETRY_ATTEMPTS_ENV, "100000")]));
        assert_eq!(options.max_attempts, DEFAULT_ATTEMPTS);
        let options = BusyOptions::from_lookup(lookup(&[(RETRY_ATTEMPTS_ENV, "0")]));
        assert_eq!(options.max_attempts, DEFAULT_ATTEMPTS);
    }

    #[test]
    fn delay_doubles_and_caps() {
        let options = BusyOptions {
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(50),
            ..BusyOptions::default()
        };
        assert_eq!(options.delay_for(1), Duration::from_millis(10));
        assert_eq!(options.delay_for(2), Duration::from_millis(20));
        assert_eq!(options.delay_for(3), Duration::from_millis(40));
        assert_eq!(options.delay_for(4), Duration::from_millis(50));
        assert_eq!(options.delay_for(99), Duration::from_millis(50));
    }

    #[test]
    fn retries_busy_until_the_operation_succeeds() {
        let calls = RefCell::new(0);
        let result = retry_busy(&instant(), || {
            let mut calls = calls.borrow_mut();
            *calls += 1;
            if *calls < 3 {
                Err(AppError::DbBusy)
            } else {
                Ok(*calls)
            }
        });
        assert_eq!(result.unwrap(), 3);
        assert_eq!(*calls.borrow(), 3);
    }

    #[test]
    fn gives_up_after_the_attempt_budget() {
        let calls = RefCell::new(0);
        let result = retry_busy(&instant(), || {
            *calls.borrow_mut() += 1;
            Err::<(), _>(AppError::DbBusy)
        });
        assert!(matches!(result, Err(AppError::DbBusy)));
        assert_eq!(*calls.borrow(), instant().max_attempts as usize);
    }

    #[test]
    fn does_not_retry_deterministic_errors() {
        let calls = RefCell::new(0);
        let result = retry_busy(&instant(), || {
            *calls.borrow_mut() += 1;
            Err::<(), _>(AppError::NotFound)
        });
        assert!(matches!(result, Err(AppError::NotFound)));
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn lock_detection_walks_the_source_chain() {
        /// Stands in for `refinery::Error`, which wraps driver errors.
        #[derive(Debug)]
        struct Wrapped(rusqlite::Error);

        impl std::fmt::Display for Wrapped {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "wrapped")
            }
        }

        impl std::error::Error for Wrapped {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        );
        assert!(AppError::is_lock(&busy));
        assert!(is_lock_error(&Wrapped(busy)));

        let other = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            None,
        );
        assert!(!is_lock_error(&Wrapped(other)));
    }
}
