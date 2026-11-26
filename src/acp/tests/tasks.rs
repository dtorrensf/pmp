// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ACP task helper sends exactly the same implementation prompt that MCP
//! exposes as the `implement-task` prompt.

use crate::acp::send_implement_task;
use crate::acp::sim::{self, SimOptions};
use crate::db::repository::{
    DependencyRepository, ProjectRepository, RepositoryPolicy, TaskRepository,
};
use crate::db::stub::StubRepository;
use crate::domain::task::{Priority, Status};

#[tokio::test]
async fn send_implement_task_uses_the_shared_prompt_template() {
    let repo = StubRepository::default();
    let project = repo.create_project("Alpha", "", None).unwrap();
    let pid = project.id.unwrap();
    let vid = repo.default_version_id(pid).unwrap();
    let blocker = repo
        .create_task(
            pid,
            vid,
            "Shared use case",
            "",
            Status::Done,
            Priority::Medium,
        )
        .unwrap()
        .id
        .unwrap();
    let task_id = repo
        .create_task(
            pid,
            vid,
            "Implement M7",
            "## Context\n\nThe server needs prompts.\n\n## Requirements\n\nAdd MCP annotations.\n\n## Benefits\n\nClients get a stable contract.",
            Status::Todo,
            Priority::High,
        )
        .unwrap()
        .id
        .unwrap();
    repo.add_dependency(task_id, blocker).unwrap();

    let detail = crate::application::records::task_detail(&repo, task_id).unwrap();
    let dependencies = crate::application::dependencies::view(&repo, task_id).unwrap();
    let expected = crate::application::prompts::implement_task(&detail, &dependencies);

    let (client, agent) = sim::spawn(SimOptions::default()).await;
    client.initialize().await.unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let session = client.new_session(workspace.path()).await.unwrap();
    let stop = send_implement_task(&client, &session, &repo, task_id)
        .await
        .expect("the task prompt is sent");
    assert!(stop.is_end_turn());

    let prompt = agent.prompt_params().expect("session/prompt was sent");
    assert_eq!(prompt["prompt"][0]["text"], expected);
    assert_eq!(prompt["sessionId"], "sess-sim");
    client.shutdown().await;
}
