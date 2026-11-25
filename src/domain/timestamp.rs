// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! UTC timestamps in the store's format, `YYYY-MM-DD HH:MM:SS`, the one SQLite's
//! `datetime('now')` writes. In that format text comparison orders them.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::error::AppError;

/// Current UTC time in the store's format.
pub fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_epoch(secs)
}

fn format_epoch(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Civil date of a day count since 1970-01-01 (proleptic Gregorian calendar).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Normalizes a lower time bound given by a client: `YYYY-MM-DD` means midnight
/// UTC, `YYYY-MM-DD HH:MM:SS` is used as is. Anything else is a validation error.
pub fn normalize_since(input: &str) -> Result<String, AppError> {
    let invalid = || {
        AppError::Validation(
            "updated_or_created_since must be YYYY-MM-DD or YYYY-MM-DD HH:MM:SS (UTC)".to_string(),
        )
    };
    let input = input.trim();
    let (date, time) = match input.split_once(' ') {
        Some((date, time)) => (date, Some(time)),
        None => (input, None),
    };

    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return Err(invalid());
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 || !all_digits(&parts) {
        return Err(invalid());
    }
    let month: u32 = month.parse().map_err(|_| invalid())?;
    let day: u32 = day.parse().map_err(|_| invalid())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(invalid());
    }

    let time = match time {
        None => "00:00:00".to_string(),
        Some(time) => {
            let fields: Vec<&str> = time.split(':').collect();
            if fields.len() != 3 || fields.iter().any(|f| f.len() != 2) || !all_digits(&fields) {
                return Err(invalid());
            }
            let values: Vec<u32> = fields
                .iter()
                .map(|f| f.parse().map_err(|_| invalid()))
                .collect::<Result<_, _>>()?;
            if values[0] > 23 || values[1] > 59 || values[2] > 59 {
                return Err(invalid());
            }
            time.to_string()
        }
    };
    Ok(format!("{date} {time}"))
}

fn all_digits(parts: &[&str]) -> bool {
    parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_epoch(0), "1970-01-01 00:00:00");
        // 2000 is a leap year: day 11016 is Feb 29, the next day is Mar 1.
        assert_eq!(format_epoch(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(format_epoch(951_782_400 + 86_400), "2000-03-01 00:00:00");
        assert_eq!(format_epoch(951_782_400 + 3_723), "2000-02-29 01:02:03");
    }

    #[test]
    fn now_has_the_store_format() {
        let now = now_utc();
        assert_eq!(now.len(), 19, "{now}");
        assert!(
            now.as_bytes()[10] == b' ' && now.as_bytes()[4] == b'-',
            "{now}"
        );
    }

    #[test]
    fn normalizes_accepted_bounds() {
        assert_eq!(
            normalize_since("2026-01-02").unwrap(),
            "2026-01-02 00:00:00"
        );
        assert_eq!(
            normalize_since(" 2026-01-02 10:30:00 ").unwrap(),
            "2026-01-02 10:30:00"
        );
    }

    #[test]
    fn rejects_malformed_bounds() {
        for bad in [
            "",
            "26-01-02",
            "2026-13-01",
            "2026-01-32",
            "2026-01-02T10:00:00",
            "2026-01-02 25:00:00",
            "2026-01-02 10:61:00",
            "2026-01-02 10:00",
            "2026/01/02",
            "2026-1-2",
        ] {
            assert!(
                matches!(normalize_since(bad), Err(AppError::Validation(_))),
                "{bad:?} must be rejected"
            );
        }
    }
}
