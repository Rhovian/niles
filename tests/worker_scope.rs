#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn by_id_commands_do_not_reach_worker_in_another_workspace() {
    let env = TestEnv::new("niles-worker-scope-foreign-live");
    let workspace_a = env.root.join("workspace-a");
    let workspace_b = env.root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    write_spawn_manifest(&workspace_b, "claude");
    let spawn = env
        .niles(
            &workspace_b,
            &["spawn", "--skill", "research", "shared", "Fix"],
        )
        .output()
        .unwrap();
    assert_command_success("spawn scoped worker", &spawn);
    let worker_dir = state_dir(&workspace_b).join("worker/shared");
    fs::write(worker_dir.join("report.md"), "workspace B report\n").unwrap();
    let status = worker_dir.join("status.log");

    let run =
        |workspace: &std::path::Path, args: &[&str]| env.niles(workspace, args).output().unwrap();

    let foreign_workers = run(&workspace_a, &["workers"]);
    assert_command_success("foreign workers", &foreign_workers);
    let foreign_workers_stdout = stdout_of(&foreign_workers);
    assert_eq!(foreign_workers_stdout, "{\"workers\":[]}\n");

    let owner_workers = run(&workspace_b, &["workers"]);
    assert_command_success("owner workers", &owner_workers);
    let owner_workers_json: serde_json::Value =
        serde_json::from_slice(&owner_workers.stdout).unwrap();
    let owner_workers = owner_workers_json["workers"].as_array().unwrap();
    assert_eq!(owner_workers.len(), 1);
    assert_eq!(owner_workers[0]["id"], "shared");
    let tmux_before = env.tmux_log();

    let peek = run(&workspace_a, &["peek", "shared"]);
    assert_failure_contains("foreign peek", &peek, "unknown worker id 'shared'");
    assert!(stdout_of(&peek).is_empty());

    let send = run(&workspace_a, &["send", "shared", "continue"]);
    assert_failure_contains("foreign send", &send, "unknown worker id 'shared'");

    let wait = run(
        &workspace_a,
        &["wait", "--interval", "10ms", "--timeout", "0"],
    );
    assert_eq!(wait.status.code(), Some(22));
    assert!(stdout_of(&wait).is_empty());

    let report = run(&workspace_a, &["report", "shared"]);
    assert_failure_contains(
        "foreign report",
        &report,
        "no report found for worker 'shared'",
    );
    assert!(stdout_of(&report).is_empty());

    let close = run(&workspace_a, &["close", "shared"]);
    assert_failure_contains("foreign close", &close, "no live worker 'shared'");

    assert_eq!(env.tmux_log(), tmux_before);
    let status_body = fs::read_to_string(&status).unwrap();
    assert!(!status_body.contains("closed: shared"));
    // A command run from another workspace must not have touched this worker's wake state.
    assert!(!worker_dir.join("status.cursor").exists());
    assert!(!state_dir(&workspace_b).join("worker/archive").exists());
    assert!(worker_dir.exists());
    let close = run(&workspace_b, &["close", "shared"]);
    assert_command_success("close worker in owner workspace", &close);

    let local_report = run(&workspace_b, &["report", "shared"]);
    assert_command_success("local archived report", &local_report);
    assert_eq!(stdout_of(&local_report), "workspace B report\n");

    let foreign_report = run(&workspace_a, &["report", "shared"]);
    assert_failure_contains(
        "foreign archived report",
        &foreign_report,
        "no report found for worker 'shared'",
    );
    assert!(stdout_of(&foreign_report).is_empty());
}
