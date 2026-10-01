// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskNote};

pub trait Repository: Send + Sync {
    fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError>;
    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError>;
    fn get_project(&self, id: i64) -> Result<Project, AppError>;
    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError>;
    fn create_task(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError>;
    fn get_task(&self, id: i64) -> Result<Task, AppError>;
    fn update_task(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError>;
    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError>;
    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError>;
    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError>;
    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError>;
    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError>;
    fn delete_project(&self, id: i64) -> Result<(), AppError>;
    fn delete_task(&self, id: i64) -> Result<(), AppError>;
    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError>;
    fn would_create_cycle(&self, task_id: i64, candidate_id: i64) -> Result<bool, AppError>;
    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError>;
    fn add_task_note(&self, task_id: i64, content: &str) -> Result<TaskNote, AppError>;
    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Repository;

    #[test]
    fn repository_trait_is_object_safe() {
        let _: Option<Arc<dyn Repository>> = None;
    }
}
