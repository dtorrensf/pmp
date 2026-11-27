// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::db::repository::{NoteRepository, ProjectRepository, RepositoryPolicy, TaskRepository};
use crate::db::stub::StubRepository;
use crate::domain::snapshot::SNAPSHOT_SCHEMA;
use crate::domain::task::{Priority, Status};

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn seeded() -> (StubRepository, i64) {
    let repo = StubRepository::default();
    let project = repo.create_project("P", "d", None).unwrap();
    let pid = project.id.unwrap();
    let version = repo.default_version_id(pid).unwrap();
    let task = repo
        .create_task(pid, version, "A", "", Status::Done, Priority::High)
        .unwrap();
    repo.add_task_note(task.id.unwrap(), "note").unwrap();
    (repo, pid)
}

fn exported_file(dir: &Path) -> PathBuf {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries.pop().unwrap()
}

#[test]
fn export_writes_a_versioned_snapshot_to_the_exports_dir() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();

    let outcome = run_with(&repo, dir.path(), &args(&["export"]));

    assert_eq!(outcome.code, 0, "{outcome:?}");
    assert!(outcome.stderr.is_empty());
    assert!(outcome.stdout.contains("snapshot exported"), "{outcome:?}");
    let file = exported_file(dir.path());
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("pmp-snapshot-"), "{name}");
    assert!(name.ends_with(".json"), "{name}");

    let json = std::fs::read_to_string(&file).unwrap();
    let snapshot = use_case::parse(&json).unwrap();
    assert_eq!(snapshot.schema, SNAPSHOT_SCHEMA);
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.notes.len(), 1);
}

#[test]
fn export_honours_an_explicit_output_path() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("nested").join("backup.json");

    let outcome = run_with(
        &repo,
        &dir.path().join("unused"),
        &args(&["export", "--output", target.to_str().unwrap()]),
    );

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let json = std::fs::read_to_string(&target).unwrap();
    assert_eq!(use_case::parse(&json).unwrap().tasks.len(), 1);
}

#[test]
fn export_rejects_an_output_without_a_path() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let outcome = run_with(&repo, dir.path(), &args(&["export", "--output"]));
    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("--output needs a path"),
        "{outcome:?}"
    );
}

#[test]
fn import_restores_the_rows_into_an_empty_store() {
    let (source, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    run_with(&source, dir.path(), &args(&["export"]));
    let file = exported_file(dir.path());
    let target = StubRepository::default();

    let outcome = run_with(
        &target,
        dir.path(),
        &args(&["import", file.to_str().unwrap()]),
    );

    assert_eq!(outcome.code, 0, "{outcome:?}");
    assert_eq!(target.list_projects().unwrap().len(), 1);
    assert_eq!(target.list_task_notes(1).unwrap()[0].content, "note");
}

#[test]
fn import_refuses_a_non_empty_store_without_replace() {
    let (source, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    run_with(&source, dir.path(), &args(&["export"]));
    let file = exported_file(dir.path());
    let target = StubRepository::default();
    target.create_project("other", "", None).unwrap();

    let outcome = run_with(
        &target,
        dir.path(),
        &args(&["import", file.to_str().unwrap()]),
    );

    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.contains("replace"), "{outcome:?}");
    assert_eq!(target.list_projects().unwrap().len(), 1);
}

#[test]
fn import_with_replace_overwrites_the_existing_rows() {
    let (source, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    run_with(&source, dir.path(), &args(&["export"]));
    let file = exported_file(dir.path());
    let target = StubRepository::default();
    target.create_project("outdated", "", None).unwrap();

    let outcome = run_with(
        &target,
        dir.path(),
        &args(&["import", file.to_str().unwrap(), "--replace"]),
    );

    assert_eq!(outcome.code, 0, "{outcome:?}");
    let names: Vec<String> = target
        .list_projects()
        .unwrap()
        .into_iter()
        .map(|project| project.name)
        .collect();
    assert_eq!(names, vec!["P".to_string()]);
}

#[test]
fn import_reports_a_missing_or_unreadable_file() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let outcome = run_with(&repo, dir.path(), &args(&["import"]));
    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("missing snapshot path"),
        "{outcome:?}"
    );

    let outcome = run_with(
        &repo,
        dir.path(),
        &args(&["import", dir.path().join("nope.json").to_str().unwrap()]),
    );
    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.contains("cannot read"), "{outcome:?}");
}

#[test]
fn import_rejects_an_invalid_document_without_writing() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.json");
    std::fs::write(&file, "{\"schema\":\"other\"}").unwrap();

    let outcome = run_with(
        &repo,
        dir.path(),
        &args(&["import", file.to_str().unwrap(), "--replace"]),
    );

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("invalid snapshot file"),
        "{outcome:?}"
    );
    assert_eq!(repo.list_projects().unwrap().len(), 1);
}

#[test]
fn help_and_unknown_subcommands_are_reported() {
    let (repo, _) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let help = run_with(&repo, dir.path(), &args(&["--help"]));
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("pmp snapshot"));

    let unknown = run_with(&repo, dir.path(), &args(&["teleport"]));
    assert_eq!(unknown.code, 1);
    assert!(
        unknown.stderr.contains("unknown snapshot command"),
        "{unknown:?}"
    );
}
