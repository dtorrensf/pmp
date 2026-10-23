// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Mutex;

use crate::db::repository::Repository;
use crate::domain::error::AppError;
use crate::domain::project::Project;
use crate::domain::task::{Priority, Status, Task, TaskDependency, TaskNote};
use crate::domain::version::{self, Version, VersionStatus};

pub struct StubRepository {
    pub projects: Mutex<Vec<Project>>,
    pub tasks: Mutex<Vec<Task>>,
    pub next_id: Mutex<i64>,
    pub dependencies: Mutex<Vec<TaskDependency>>,
    pub task_notes: Mutex<Vec<TaskNote>>,
    pub versions: Mutex<Vec<Version>>,
}

impl Default for StubRepository {
    fn default() -> Self {
        Self {
            projects: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            next_id: Mutex::new(1),
            dependencies: Mutex::new(Vec::new()),
            task_notes: Mutex::new(Vec::new()),
            versions: Mutex::new(Vec::new()),
        }
    }
}

impl StubRepository {
    fn next_id(&self) -> i64 {
        let mut id = self.next_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    fn ensure_version_in_project(&self, version_id: i64, project_id: i64) -> Result<(), AppError> {
        let belongs = self
            .versions
            .lock()
            .unwrap()
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

impl Repository for StubRepository {
    fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        Ok(self.projects.lock().unwrap().clone())
    }

    fn create_project(&self, name: &str, description: &str) -> Result<Project, AppError> {
        let project = Project {
            id: Some(self.next_id()),
            name: name.to_string(),
            description: description.to_string(),
            created_at: "now".to_string(),
        };
        self.projects.lock().unwrap().push(project.clone());
        self.versions.lock().unwrap().push(Version {
            id: Some(self.next_id()),
            project_id: project.id.unwrap(),
            name: version::DEFAULT_VERSION_NAME.to_string(),
            description: String::new(),
            status: VersionStatus::Open,
            created_at: "now".to_string(),
        });
        Ok(project)
    }

    fn update_project(&self, id: i64, name: &str, description: &str) -> Result<Project, AppError> {
        let mut projects = self.projects.lock().unwrap();
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
            .unwrap()
            .iter()
            .find(|p| p.id == Some(id))
            .cloned()
            .ok_or(AppError::NotFound)
    }

    fn list_tasks_by_project(&self, project_id: i64) -> Result<Vec<Task>, AppError> {
        let mut tasks: Vec<Task> = self
            .tasks
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.project_id == project_id)
            .cloned()
            .collect();
        sort_tasks(&mut tasks);
        Ok(tasks)
    }

    fn list_versions_by_project(&self, project_id: i64) -> Result<Vec<Version>, AppError> {
        let mut versions: Vec<Version> = self
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.project_id == project_id)
            .cloned()
            .collect();
        versions.sort_by(|a, b| {
            (a.status == VersionStatus::Released)
                .cmp(&(b.status == VersionStatus::Released))
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
        version::validate_version_name(name)?;
        self.get_project(project_id)?;
        let versions = self.versions.lock().unwrap();
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
            id: Some(self.next_id()),
            project_id,
            name: name.to_string(),
            description: description.to_string(),
            status: VersionStatus::Open,
            created_at: "now".to_string(),
        };
        self.versions.lock().unwrap().push(version.clone());
        Ok(version)
    }

    fn get_version(&self, id: i64) -> Result<Version, AppError> {
        self.versions
            .lock()
            .unwrap()
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
        version::validate_version_name(name)?;
        let mut versions = self.versions.lock().unwrap();
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
            .unwrap()
            .iter()
            .filter(|t| t.version_id == id)
            .filter_map(|t| t.id)
            .collect();
        for task_id in &task_ids {
            self.delete_task(*task_id)?;
        }
        self.versions.lock().unwrap().retain(|v| v.id != Some(id));
        Ok(())
    }

    fn list_tasks_by_version(&self, version_id: i64) -> Result<Vec<Task>, AppError> {
        let mut tasks: Vec<Task> = self
            .tasks
            .lock()
            .unwrap()
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
        self.ensure_version_in_project(version_id, project_id)?;
        let task = Task {
            id: Some(self.next_id()),
            project_id,
            version_id,
            name: name.to_string(),
            description: description.to_string(),
            status,
            priority,
            created_at: "now".to_string(),
        };
        self.tasks.lock().unwrap().push(task.clone());
        Ok(task)
    }

    fn get_task(&self, id: i64) -> Result<Task, AppError> {
        self.tasks
            .lock()
            .unwrap()
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
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(id))
            .ok_or(AppError::NotFound)?;
        task.name = name.to_string();
        task.description = description.to_string();
        task.status = status;
        task.priority = priority;
        Ok(task.clone())
    }

    fn update_task_status(&self, id: i64, new_status: Status) -> Result<Task, AppError> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter_mut()
            .find(|t| t.id == Some(id))
            .ok_or(AppError::NotFound)?;
        task.status = new_status;
        Ok(task.clone())
    }

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
        let mut deps = self.dependencies.lock().unwrap();
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

    fn remove_dependency(&self, task_id: i64, depends_on_id: i64) -> Result<(), AppError> {
        self.dependencies
            .lock()
            .unwrap()
            .retain(|d| !(d.task_id == task_id && d.depends_on_id == depends_on_id));
        Ok(())
    }

    fn get_dependencies(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let mut dependencies: Vec<i64> = self
            .dependencies
            .lock()
            .unwrap()
            .iter()
            .filter(|d| d.task_id == task_id)
            .map(|d| d.depends_on_id)
            .collect();
        dependencies.sort_unstable();
        Ok(dependencies)
    }

    fn has_incomplete_dependencies(&self, task_id: i64) -> Result<bool, AppError> {
        let deps = self.get_dependencies(task_id)?;
        let tasks = self.tasks.lock().unwrap();
        Ok(deps.iter().any(|id| {
            tasks
                .iter()
                .find(|t| t.id == Some(*id))
                .map(|t| t.status != Status::Done)
                .unwrap_or(false)
        }))
    }

    fn delete_project(&self, id: i64) -> Result<(), AppError> {
        let task_ids: Vec<i64> = {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .iter()
                .filter(|t| t.project_id == id)
                .filter_map(|t| t.id)
                .collect()
        };
        let mut deps = self.dependencies.lock().unwrap();
        for tid in &task_ids {
            deps.retain(|d| d.task_id != *tid && d.depends_on_id != *tid);
        }
        drop(deps);
        let mut notes = self.task_notes.lock().unwrap();
        notes.retain(|n| !task_ids.contains(&n.task_id));
        drop(notes);
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|t| t.project_id != id);
        drop(tasks);
        self.versions.lock().unwrap().retain(|v| v.project_id != id);
        let mut projects = self.projects.lock().unwrap();
        projects.retain(|p| p.id != Some(id));
        Ok(())
    }

    fn delete_task(&self, id: i64) -> Result<(), AppError> {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|t| t.id != Some(id));
        drop(tasks);
        let mut deps = self.dependencies.lock().unwrap();
        deps.retain(|d| d.task_id != id && d.depends_on_id != id);
        drop(deps);
        let mut notes = self.task_notes.lock().unwrap();
        notes.retain(|n| n.task_id != id);
        Ok(())
    }

    fn get_dependents(&self, task_id: i64) -> Result<Vec<i64>, AppError> {
        let mut dependents: Vec<i64> = self
            .dependencies
            .lock()
            .unwrap()
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
        let deps = self.dependencies.lock().unwrap();
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

    fn list_task_notes(&self, task_id: i64) -> Result<Vec<TaskNote>, AppError> {
        let mut notes: Vec<TaskNote> = self
            .task_notes
            .lock()
            .unwrap()
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
        note.id = Some(self.next_id());
        note.created_at = "now".to_string();
        self.task_notes.lock().unwrap().push(note.clone());
        Ok(note)
    }

    fn delete_task_note(&self, note_id: i64) -> Result<(), AppError> {
        let mut notes = self.task_notes.lock().unwrap();
        let pos = notes
            .iter()
            .position(|n| n.id == Some(note_id))
            .ok_or(AppError::NotFound)?;
        notes.remove(pos);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
