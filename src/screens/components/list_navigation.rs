// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crossterm::event::KeyCode;

/// Moves a list selection with either arrow keys or Vim-style `k`/`j` keys.
/// Returns whether `code` was handled as a navigation key.
pub fn move_selection(selected: &mut usize, item_count: usize, code: KeyCode) -> bool {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            *selected = selected.saturating_sub(1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            *selected = if item_count == 0 {
                0
            } else {
                selected.saturating_add(1).min(item_count - 1)
            };
            true
        }
        _ => false,
    }
}
