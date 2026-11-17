// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Daily completion activity: how many tasks were finished on each of the
//! last days, for the sparkline on the project dashboard.
//!
//! Completion timestamps are written by SQLite's `datetime('now')`, so they
//! are UTC `YYYY-MM-DD HH:MM:SS` strings. Days are counted in UTC too.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::task::{Status, Task};

/// Number of days shown in the sparkline.
pub const ACTIVITY_DAYS: usize = 14;

const SECONDS_PER_DAY: i64 = 86_400;

/// Days elapsed since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The day, counted from 1970-01-01, of a `YYYY-MM-DD[ HH:MM:SS]` timestamp.
pub fn epoch_day_of(timestamp: &str) -> Option<i64> {
    let date = timestamp.get(..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// The current UTC day, counted from 1970-01-01.
pub fn today_epoch_day() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    seconds.div_euclid(SECONDS_PER_DAY)
}

/// Tasks completed on each of the last [`ACTIVITY_DAYS`] days, oldest first:
/// the last entry is `today`. Tasks finished before the window, or with an
/// unreadable completion date, are not counted.
pub fn completions_per_day(tasks: &[Task], today: i64) -> [u64; ACTIVITY_DAYS] {
    let mut counts = [0u64; ACTIVITY_DAYS];
    for task in tasks.iter().filter(|task| task.status == Status::Done) {
        let Some(day) = task.completed_at.as_deref().and_then(epoch_day_of) else {
            continue;
        };
        let age = today - day;
        if (0..ACTIVITY_DAYS as i64).contains(&age) {
            counts[ACTIVITY_DAYS - 1 - age as usize] += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::task::Priority;

    fn done_at(timestamp: Option<&str>) -> Task {
        Task {
            id: Some(1),
            project_id: 1,
            version_id: 1,
            name: "T".into(),
            description: String::new(),
            status: Status::Done,
            priority: Priority::Medium,
            created_at: "2026-01-01 00:00:00".into(),
            completed_at: timestamp.map(str::to_string),
        }
    }

    #[test]
    fn epoch_day_matches_known_dates() {
        assert_eq!(epoch_day_of("1970-01-01 00:00:00"), Some(0));
        assert_eq!(epoch_day_of("2000-03-01"), Some(11_017));
        assert_eq!(epoch_day_of("2026-11-15 20:53:49"), Some(20_772));
    }

    #[test]
    fn epoch_day_rejects_malformed_timestamps() {
        assert_eq!(epoch_day_of("now"), None);
        assert_eq!(epoch_day_of("2026-13-01"), None);
        assert_eq!(epoch_day_of("2026-11"), None);
        assert_eq!(epoch_day_of(""), None);
    }

    #[test]
    fn today_is_the_last_bucket_and_older_days_come_first() {
        let today = 20_772;
        let tasks = [
            done_at(Some("2026-11-15 08:00:00")), // today
            done_at(Some("2026-11-15 09:00:00")), // today
            done_at(Some("2026-11-14 09:00:00")), // yesterday
            done_at(Some("2026-11-02 09:00:00")), // 13 days ago: first bucket
        ];
        let counts = completions_per_day(&tasks, today);
        assert_eq!(counts[ACTIVITY_DAYS - 1], 2);
        assert_eq!(counts[ACTIVITY_DAYS - 2], 1);
        assert_eq!(counts[0], 1);
        assert_eq!(counts.iter().sum::<u64>(), 4);
    }

    #[test]
    fn tasks_outside_the_window_or_not_done_are_ignored() {
        let today = 20_772;
        let mut open = done_at(Some("2026-11-15 08:00:00"));
        open.status = Status::Todo;
        let tasks = [
            done_at(Some("2026-10-01 08:00:00")), // too old
            done_at(Some("2026-11-16 08:00:00")), // in the future
            done_at(None),                        // no completion date
            open,
        ];
        assert_eq!(completions_per_day(&tasks, today), [0; ACTIVITY_DAYS]);
    }
}
