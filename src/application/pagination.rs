// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offset pagination shared by the list use cases (M2).
//!
//! Every paged list answers with the same metadata (`total`, `limit`,
//! `offset`, `has_more`) so clients can page through any collection the same
//! way.

use serde::{Deserialize, Serialize};

/// Default and maximum page size for list use cases.
pub const DEFAULT_PAGE_SIZE: u32 = 100;
pub const MAX_PAGE_SIZE: u32 = 500;

/// Metadata that accompanies every page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageMeta {
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
    pub has_more: bool,
}

/// Applies the default and the bounds to a requested page size.
pub fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

/// Selects the window of `items` given by `limit` and `offset`, with its metadata.
pub fn window<T>(items: Vec<T>, limit: u32, offset: u32) -> (Vec<T>, PageMeta) {
    let total = items.len() as u64;
    let page: Vec<T> = items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let meta = meta(total, page.len(), limit, offset);
    (page, meta)
}

/// Builds the metadata for a page of `returned` items starting at `offset`.
pub fn meta(total: u64, returned: usize, limit: u32, offset: u32) -> PageMeta {
    PageMeta {
        total,
        limit,
        offset,
        has_more: (u64::from(offset) + returned as u64) < total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_applies_default_and_bounds() {
        assert_eq!(clamp_limit(None), DEFAULT_PAGE_SIZE);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(10_000)), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(Some(25)), 25);
    }

    #[test]
    fn window_slices_and_reports_has_more() {
        let (page, meta) = window(vec![1, 2, 3, 4, 5], 2, 0);
        assert_eq!(page, vec![1, 2]);
        assert_eq!(meta.total, 5);
        assert!(meta.has_more);

        let (page, meta) = window(vec![1, 2, 3, 4, 5], 2, 4);
        assert_eq!(page, vec![5]);
        assert!(!meta.has_more);
    }

    #[test]
    fn offset_past_the_end_is_an_empty_last_page() {
        let (page, meta) = window(vec![1, 2], 10, 7);
        assert!(page.is_empty());
        assert_eq!(meta.total, 2);
        assert!(!meta.has_more);
    }
}
