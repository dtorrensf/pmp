// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Mutex;

use crate::db::repository::{
    DependencyRepository, NoteRepository, ProjectRepository, SummaryRepository, TaskRepository,
    VersionRepository, check_dependency_set, check_done_transition, check_task_move, unique_ids,
};
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskDependency, TaskNote};
use crate::domain::validation;
use crate::domain::version::{self, Version, VersionStatus};
use crate::domain::version_summary::VersionSummary;

/// Entity kinds that own an id sequence in the stub. SQLite allocates ids per
/// table (rowid), so the stub keeps one counter per entity instead of sharing a
/// single sequence, which would assign ids that diverge from production.
#[derive(Clone, Copy)]
enum IdKind {
    Project,
    Version,
    Task,
    Note,
    Summary,
}

struct IdCounters {
    project: i64,
    version: i64,
    task: i64,
    note: i64,
    summary: i64,
}

impl Default for IdCounters {
    fn default() -> Self {
        Self {
            project: 1,
            version: 1,
            task: 1,
            note: 1,
            summary: 1,
        }
    }
}

impl IdCounters {
    fn next(&mut self, kind: IdKind) -> i64 {
        let slot = match kind {
            IdKind::Project => &mut self.project,
            IdKind::Version => &mut self.version,
            IdKind::Task => &mut self.task,
            IdKind::Note => &mut self.note,
            IdKind::Summary => &mut self.summary,
        };
        let current = *slot;
        *slot += 1;
        current
    }
}

pub struct StubRepository {
    pub projects: Mutex<Vec<Project>>,
    pub tasks: Mutex<Vec<Task>>,
    next_ids: Mutex<IdCounters>,
    pub dependencies: Mutex<Vec<TaskDependency>>,
    pub task_notes: Mutex<Vec<TaskNote>>,
    pub versions: Mutex<Vec<Version>>,
    pub version_summaries: Mutex<Vec<VersionSummary>>,
}

impl Default for StubRepository {
    fn default() -> Self {
        Self {
            projects: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            next_ids: Mutex::new(IdCounters::default()),
            dependencies: Mutex::new(Vec::new()),
            task_notes: Mutex::new(Vec::new()),
            versions: Mutex::new(Vec::new()),
            version_summaries: Mutex::new(Vec::new()),
        }
    }
}

impl StubRepository {
    fn next_id(&self, kind: IdKind) -> i64 {
        let mut ids = self.next_ids.lock().unwrap_or_else(|e| e.into_inner());
        ids.next(kind)
    }

    fn ensure_version_in_project(&self, version_id: i64, project_id: i64) -> Result<(), AppError> {
        let belongs = self
            .versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|v| v.id == Some(version_id) && v.project_id == project_id);
        if belongs {
            Ok(())
        } else {
            Err(AppError::Validation(
                "version does not belong to the project".to_string(),
            ))
        }
    }
}

/// Orders tasks: not-done first, then priority descending, then newest first.
fn sort_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| {
        let done_order = (a.status == Status::Done).cmp(&(b.status == Status::Done));
        if done_order != std::cmp::Ordering::Equal {
            return done_order;
        }
        let priority_order = i64::from(b.priority).cmp(&i64::from(a.priority));
        if priority_order != std::cmp::Ordering::Equal {
            return priority_order;
        }
        b.created_at.cmp(&a.created_at)
    });
}

impl ProjectRepository for StubRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        Ok(self
            .projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone())
    }

    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        let project = Project {
            id: Some(self.next_id(IdKind::Project)),
            name: name.to_string(),
            description: description.to_string(),
            created_at: "now".to_string(),
        };
        self.projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(project.clone());
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Version {
                id: Some(self.next_id(IdKind::Version)),
                project_id: project.id.unwrap(),
                name: version::DEFAULT_VERSION_NAME.to_string(),
                description: String::new(),
                status: VersionStatus::Backlog,
                created_at: "now".to_string(),
            });
        Ok(project)
    }

    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError> {
        validation::validate_name("project", name)?;
        validation::validate_text("project", "description", description)?;
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        let project = projects
            .iter_mut()
            .find(|p| p.id == Some(id))
            .ok_or(AppError::NotFound)?;
        project.name = name.to_string();
        project.description = description.to_string();
        Ok(project.clone())
    }

    fn get_project(&self, id: i64) -> Result<Project, AppError> {
        self.projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|p| p.id == Some(id))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    fn delete_project(&self, id: i64) -> Result<(), AppError> {
        let task_ids: Vec<i64> = {
            let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            tasks
                .iter()
                .filter(|t| t.project_id == id)
                .filter_map(|t| t.id)
                .collect()
        };
        let mut deps = self.dependencies.lock().unwrap_or_else(|e| e.into_inner());
        for tid in &task_ids {
            deps.retain(|d| d.task_id != *tid && d.depends_on_id != *tid);
        }
        drop(deps);
        let mut notes = self.task_notes.lock().unwrap_or_else(|e| e.into_inner());
        notes.retain(|n| !task_ids.contains(&n.task_id));
        drop(notes);
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|t| t.project_id != id);
        drop(tasks);
        let version_ids: Vec<i64> = {
            let mut versions = self.versions.lock().unwrap_or_else(|e| e.into_inner());
            let ids = versions
                .iter()
                .filter(|v| v.project_id == id)
                .filter_map(|v| v.id)
                .collect();
            versions.retain(|v| v.project_id != id);
            ids
        };
        let mut summaries = self
            .version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        summaries.retain(|s| !version_ids.contains(&s.version_id));
        drop(summaries);
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        projects.retain(|p| p.id != Some(id));
        Ok(())
    }
}

impl VersionRepository for StubRepository {
    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError> {
        let mut versions: Vec<Version> = self
            .versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|v| v.project_id == project_id)
            .cloned()
            .collect();
        versions.sort_by(|a, b| {
            a.status
                .list_rank()
                .cmp(&b.status.list_rank())
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(versions)
    }

    fn create_version(
        &self,
        project_id: i64,
        name: &str,
        description: &str,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        self.get_project(project_id)?;
        let versions = self.versions.lock().unwrap_or_else(|e| e.into_inner());
        if versions
            .iter()
            .any(|v| v.project_id == project_id && v.name == name)
        {
            return Err(AppError::Validation(format!(
                "version already exists: {name}"
            )));
        }
        drop(versions);
        let version = Version {
            id: Some(self.next_id(IdKind::Version)),
            project_id,
            name: name.to_string(),
            description: description.to_string(),
            status: VersionStatus::Backlog,
            created_at: "now".to_string(),
        };
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(version.clone());
        Ok(version)
    }

    fn get_version(&self, id: i64) -> Result<Version, AppError> {
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|v| v.id == Some(id))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    fn update_version(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: VersionStatus,
    ) -> Result<Version, AppError> {
        validation::validate_name("version", name)?;
        validation::validate_text("version", "description", description)?;
        let mut versions = self.versions.lock().unwrap_or_else(|e| e.into_inner());
        let project_id = versions
            .iter()
            .find(|v| v.id == Some(id))
            .map(|v| v.project_id)
            .ok_or(AppError::NotFound)?;
        if versions
            .iter()
            .any(|v| v.id != Some(id) && v.project_id == project_id && v.name == name)
        {
            return Err(AppError::Validation(format!(
                "version already exists: {name}"
            )));
        }
        let version = versions
            .iter_mut()
            .find(|v| v.id == Some(id))
            .ok_or(AppError::NotFound)?;
        version.name = name.to_string();
        version.description = description.to_string();
        version.status = status;
        Ok(version.clone())
    }

    fn delete_version(&self, id: i64) -> Result<(), AppError> {
        let task_ids: Vec<i64> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|t| t.version_id == id)
            .filter_map(|t| t.id)
            .collect();
        for task_id in &task_ids {
            self.delete_task(*task_id)?;
        }
        self.version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|s| s.version_id != id);
        self.versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|v| v.id != Some(id));
        Ok(())
    }
}

impl TaskRepository for StubRepository {
    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError> {
        let mut tasks: Vec<Task> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|t| t.project_id == project_id)
            .cloned()
            .collect();
        sort_tasks(&mut tasks);
        Ok(tasks)
    }

    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError> {
        let mut tasks: Vec<Task> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|t| t.version_id == version_id)
            .cloned()
            .collect();
        sort_tasks(&mut tasks);
        Ok(tasks)
    }

    fn create_task(
        &self,
        project_id: i64,
        version_id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError> {
        validation::validate_name("task", name)?;
        validation::validate_text("task", "description", description)?;
        self.ensure_version_in_project(version_id, project_id)?;
        let task = Task {
            id: Some(self.next_id(IdKind::Task)),
            project_id,
            version_id,
            name: name.to_string(),
            description: description.to_string(),
            status,
            priority,
            created_at: "now".to_string(),
            completed_at: None,
        };
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(task.clone());
        Ok(task)
    }

    fn get_task(&self, id: i64) -> Result<Task, AppError> {
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|t| t.id == Some(id))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    fn update_task(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: Status,
        priority: Priority,
    ) -> Result<Task, AppError> {
        validation::validate_name("task", name)?;
        validation::validate_text("task", "description", description)?;
        let current = self.get_task(id)?;
        check_done_transition(current.status, status, || {
            self.has_incomplete_dependencies(id)
        })?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(id))
            .ok_or(AppError::NotFound)?;
        task.name = name.to_string();
        task.description = description.to_string();
        task.status = status;
        task.priority = priority;
        task.completed_at = match status {
            Status::Done => task
                .completed_at
                .clone()
                .or_else(|| Some("now".to_string())),
            _ => None,
        };
        Ok(task.clone())
    }

    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError> {
        let current = self.get_task(id)?;
        check_done_transition(current.status, new_status, || {
            self.has_incomplete_dependencies(id)
        })?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(id))
            .ok_or(AppError::NotFound)?;
        task.status = new_status;
        task.completed_at = match new_status {
            Status::Done => Some(
                task.completed_at
                    .clone()
                    .unwrap_or_else(|| "now".to_string()),
            ),
            _ => None,
        };
        Ok(task.clone())
    }

    fn update_task_priority(&self, id: i64, priority: Priority) -> Result<Task, AppError> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(id))
            .ok_or(AppError::NotFound)?;
        task.priority = priority;
        Ok(task.clone())
    }

    fn move_task(&self, task_id: i64, version_id: i64) -> Result<Task, AppError> {
        let task = self.get_task(task_id)?;
        let target = self.get_version(version_id)?;
        if task.version_id == version_id {
            return Ok(task);
        }
        check_task_move(
            &task,
            &target,
            &self.get_dependencies(task_id)?,
            &self.get_dependents(task_id)?,
        )?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(task_id))
            .ok_or(AppError::NotFound)?;
        task.version_id = version_id;
        Ok(task.clone())
    }

    fn delete_task(&self, id: i64) -> Result<(), AppError> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|t| t.id != Some(id));
        drop(tasks);
        let mut deps = self.dependencies.lock().unwrap_or_else(|e| e.into_inner());
        deps.retain(|d| d.task_id != id && d.depends_on_id != id);
        drop(deps);
        let mut notes = self.task_notes.lock().unwrap_or_else(|e| e.into_inner());
        notes.retain(|n| n.task_id != id);
        Ok(())
    }

    fn delete_done_tasks_by_version(&self, version_id: i64) -> Result<(), AppError> {
        let task_ids: Vec<i64> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|t| t.version_id == version_id && t.status == Status::Done)
            .filter_map(|t| t.id)
            .collect();
        for id in &task_ids {
            self.delete_task(*id)?;
        }
        Ok(())
    }
}

impl DependencyRepository for StubRepository {
    fn add_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        let task = self.get_task(task_id)?;
        let depends_on = self.get_task(depends_on_id)?;
        if task.version_id != depends_on.version_id {
            return Err(AppError::Validation(
                "dependencies must be within the same version".to_string(),
            ));
        }
        if self.would_create_cycle(task_id, depends_on_id)? {
            return Err(AppError::DependencyCycle);
        }
        let mut deps = self.dependencies.lock().unwrap_or_else(|e| e.into_inner());
        if !deps
            .iter()
            .any(|d| d.task_id == task_id && d.depends_on_id == depends_on_id)
        {
            deps.push(TaskDependency {
                task_id,
                depends_on_id,
            });
        }
        Ok(())
    }

    fn set_dependencies(&self, task_id: i64, depends_on_ids: &[i64]) -> Result<(), AppError> {
        validation::validate_dependency_count(depends_on_ids.len())?;
        let task = self.get_task(task_id)?;
        let unique = unique_ids(depends_on_ids);
        for &dep in &unique {
            let depends_on = self.get_task(dep)?;
            if depends_on.version_id != task.version_id {
                return Err(AppError::Validation(
                    "dependencies must be within the same version".to_string(),
                ));
            }
        }
        let mut graph: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
        for d in self
            .dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            graph.entry(d.task_id).or_default().push(d.depends_on_id);
        }
        check_dependency_set(&graph, task_id, &unique)?;

        let mut deps = self.dependencies.lock().unwrap_or_else(|e| e.into_inner());
        deps.retain(|d| d.task_id != task_id);
        deps.extend(unique.iter().map(|&depends_on_id| TaskDependency {
            task_id,
            depends_on_id,
        }));
        Ok(())
    }

    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        self.dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|d| !(d.task_id == task_id && d.depends_on_id == depends_on_id));
        Ok(())
    }

    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let mut dependencies: Vec<i64> = self
            .dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|d| d.task_id == task_id)
            .map(|d| d.depends_on_id)
            .collect();
        dependencies.sort_unstable();
        Ok(dependencies)
    }

    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError> {
        let deps = self.get_dependencies(task_id)?;
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        Ok(deps.iter().any(|id| {
            tasks
                .iter()
                .find(|t| t.id == Some(*id))
                .map(|t| t.status != Status::Done)
                .unwrap_or(false)
        }))
    }

    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let mut dependents: Vec<i64> = self
            .dependencies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|d| d.depends_on_id == task_id)
            .map(|d| d.task_id)
            .collect();
        dependents.sort_unstable();
        Ok(dependents)
    }

    fn would_create_cycle(&self, task_id: i64, candidate_id: i64) -> Result<bool, AppError> {
        if task_id == candidate_id {
            return Ok(true);
        }
        let deps = self.dependencies.lock().unwrap_or_else(|e| e.into_inner());
        let mut graph: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
        for d in deps.iter() {
            graph.entry(d.task_id).or_default().push(d.depends_on_id);
        }
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![candidate_id];
        while let Some(current) = stack.pop() {
            if current == task_id {
                return Ok(true);
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(next) = graph.get(&current) {
                stack.extend(next);
            }
        }
        Ok(false)
    }
}

impl NoteRepository for StubRepository {
    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError> {
        let mut notes: Vec<TaskNote> = self
            .task_notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|n| n.task_id == task_id)
            .cloned()
            .collect();
        notes.sort_by(|a, b| {
            let created_order = b.created_at.cmp(&a.created_at);
            if created_order != std::cmp::Ordering::Equal {
                return created_order;
            }
            b.id.cmp(&a.id)
        });
        Ok(notes)
    }

    fn add_task_note(&self, task_id: i64, content: &str) -> Result<TaskNote, AppError> {
        let mut note = TaskNote::new(task_id, content)?;
        note.id = Some(self.next_id(IdKind::Note));
        note.created_at = "now".to_string();
        self.task_notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(note.clone());
        Ok(note)
    }

    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError> {
        let mut notes = self.task_notes.lock().unwrap_or_else(|e| e.into_inner());
        let pos = notes
            .iter()
            .position(|n| n.id == Some(note_id))
            .ok_or(AppError::NotFound)?;
        notes.remove(pos);
        Ok(())
    }
}

impl SummaryRepository for StubRepository {
    fn list_version_summaries(&self, version_id: i64) -> Result<Vec<VersionSummary>, AppError> {
        let mut summaries: Vec<VersionSummary> = self
            .version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|s| s.version_id == version_id)
            .cloned()
            .collect();
        summaries.sort_by(|a, b| {
            b.generated_at
                .cmp(&a.generated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(summaries)
    }

    fn get_version_summary(&self, id: i64) -> Result<VersionSummary, AppError> {
        self.version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|s| s.id == Some(id))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    fn create_version_summary(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError> {
        validation::validate_text("version summary", "summary", summary)?;
        self.get_version(version_id)?;
        let mut summaries = self
            .version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if summaries.iter().any(|s| s.version_id == version_id) {
            return Err(AppError::Validation(
                "version already has a summary".to_string(),
            ));
        }
        let summary = VersionSummary {
            id: Some(self.next_id(IdKind::Summary)),
            version_id,
            summary: summary.to_string(),
            snapshot_json: snapshot_json.to_string(),
            generated_at: "now".to_string(),
            created_at: "now".to_string(),
        };
        summaries.push(summary.clone());
        Ok(summary)
    }

    fn delete_version_summary(&self, id: i64) -> Result<(), AppError> {
        let mut summaries = self
            .version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pos = summaries
            .iter()
            .position(|s| s.id == Some(id))
            .ok_or(AppError::NotFound)?;
        summaries.remove(pos);
        Ok(())
    }

    fn compact_version(
        &self,
        version_id: i64,
        summary: &str,
        snapshot_json: &str,
    ) -> Result<VersionSummary, AppError> {
        validation::validate_text("version summary", "summary", summary)?;
        self.get_version(version_id)?;
        self.version_summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|s| s.version_id != version_id);
        let created = self.create_version_summary(version_id, summary, snapshot_json)?;
        self.delete_done_tasks_by_version(version_id)?;
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::RepositoryPolicy;

    fn repository_with_tasks() -> (StubRepository, [i64; 3]) {
        let repo = StubRepository::default();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let ids = [
            repo.create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "A",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap()
            .id
            .unwrap(),
            repo.create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "B",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap()
            .id
            .unwrap(),
            repo.create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "C",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap()
            .id
            .unwrap(),
        ];
        (repo, ids)
    }

    #[test]
    fn add_dependency_rejects_self_loop() {
        let (repo, [task_id, _, _]) = repository_with_tasks();

        assert!(matches!(
            repo.add_dependency(task_id, task_id),
            Err(AppError::DependencyCycle)
        ));
    }

    #[test]
    fn add_dependency_rejects_indirect_cycle() {
        let (repo, [a_id, b_id, c_id]) = repository_with_tasks();
        repo.add_dependency(a_id, b_id).unwrap();
        repo.add_dependency(b_id, c_id).unwrap();

        assert!(matches!(
            repo.add_dependency(c_id, a_id),
            Err(AppError::DependencyCycle)
        ));
    }

    #[test]
    fn dependency_queries_are_sorted() {
        let (repo, [a_id, b_id, c_id]) = repository_with_tasks();
        repo.add_dependency(a_id, c_id).unwrap();
        repo.add_dependency(a_id, b_id).unwrap();

        assert_eq!(repo.get_dependencies(a_id).unwrap(), vec![b_id, c_id]);
        assert_eq!(repo.get_dependents(b_id).unwrap(), vec![a_id]);
    }

    #[test]
    fn stub_task_notes_crud() {
        let repo = StubRepository::default();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        let note = repo.add_task_note(task_id, "note content").unwrap();
        assert_eq!(note.task_id, task_id);
        assert_eq!(note.content, "note content");
        assert!(note.id.is_some());

        let notes = repo.list_task_notes(task_id).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "note content");

        repo.delete_task_note(note.id.unwrap()).unwrap();
        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }

    #[test]
    fn stub_delete_task_cascades_notes() {
        let repo = StubRepository::default();
        let project = repo.create_project("Alpha", "").unwrap();
        let project_id = project.id.unwrap();
        let task = repo
            .create_task(
                project_id,
                repo.default_version_id(project_id).unwrap(),
                "T1",
                "",
                Status::Todo,
                Priority::Medium,
            )
            .unwrap();
        let task_id = task.id.unwrap();

        repo.add_task_note(task_id, "cascade me").unwrap();
        repo.delete_task(task_id).unwrap();

        assert!(repo.list_task_notes(task_id).unwrap().is_empty());
    }

    #[test]
    fn stub_delete_task_note_not_found() {
        let repo = StubRepository::default();
        assert!(matches!(
            repo.delete_task_note(999),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn poisoned_mutex_is_recovered_instead_of_panicking() {
        let repo = StubRepository::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = repo.projects.lock().unwrap_or_else(|e| e.into_inner());
            panic!("poison the projects mutex");
        }));
        assert!(repo.list_projects().is_ok());
    }
}
