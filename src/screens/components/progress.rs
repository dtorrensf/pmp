// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::domain::task::{Status, Task};

/// Returns the completion percentage for a slice of tasks.
///
/// The percentage is calculated as `done / total * 100` and rounded to the
/// nearest integer. If there are no tasks, `0` is returned.
pub fn completion_percentage(tasks: &[Task]) -> u64 {
    let total = tasks.len() as u64;
    if total == 0 {
        return 0;
    }
    let done = tasks
        .iter()
        .filter(|task| task.status == Status::Done)
        .count() as u64;
    (done as f64 / total as f64 * 100.0).round() as u64
}

#[cfg(test)]
mod tests {
    use crate::domain::task::{Priority, Status, Task};

    use super::*;

    fn task(status: Status) -> Task {
        Task {
            id: None,
            project_id: 1,
            version_id: 1,
            name: "T".to_string(),
            description: "".to_string(),
            status,
            priority: Priority::Low,
            created_at: "now".to_string(),
        }
    }

    #[test]
    fn empty_task_list_is_zero_percent() {
        assert_eq!(completion_percentage(&[]), 0);
    }

    #[test]
    fn all_todo_is_zero_percent() {
        let tasks = vec![task(Status::Todo), task(Status::Todo)];
        assert_eq!(completion_percentage(&tasks), 0);
    }

    #[test]
    fn one_done_out_of_three_is_thirty_three_percent() {
        let tasks = vec![task(Status::Todo), task(Status::Todo), task(Status::Done)];
        assert_eq!(completion_percentage(&tasks), 33);
    }

    #[test]
    fn half_done_is_fifty_percent() {
        let tasks = vec![task(Status::Done), task(Status::Todo)];
        assert_eq!(completion_percentage(&tasks), 50);
    }

    #[test]
    fn all_done_is_one_hundred_percent() {
        let tasks = vec![task(Status::Done), task(Status::Done)];
        assert_eq!(completion_percentage(&tasks), 100);
    }

    #[test]
    fn rounds_to_nearest_integer() {
        let tasks = vec![task(Status::Done), task(Status::Todo), task(Status::Todo)];
        assert_eq!(completion_percentage(&tasks), 33);

        let tasks = vec![task(Status::Done), task(Status::Done), task(Status::Todo)];
        assert_eq!(completion_percentage(&tasks), 67);
    }
}
