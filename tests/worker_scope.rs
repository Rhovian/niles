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
    let spawn = env
        .niles(
            &workspace_b,
            &["spawn", "shared", "--agent", "claude", "Fix"],
        )
        .output()
        .unwrap();
    assert_command_success("spawn scoped worker", &spawn);
    let worker_dir = workspace_b.join(".niles/worker/shared");
    fs::write(worker_dir.join("report.md"), "workspace B report\n").unwrap();
    let status = worker_dir.join("status.log");

    let foreign_workers = env.niles(&workspace_a, &["workers"]).output().unwrap();
    assert_command_success("foreign workers", &foreign_workers);
    let foreign_workers_stdout = String::from_utf8_lossy(&foreign_workers.stdout);
    assert!(
        foreign_workers_stdout.contains("workers[0]{id,agent,task,age,window,wake,last_status}:")
    );
    assert!(!foreign_workers_stdout.contains("shared"));

    let owner_workers = env.niles(&workspace_b, &["workers"]).output().unwrap();
    assert_command_success("owner workers", &owner_workers);
    let owner_workers_stdout = String::from_utf8_lossy(&owner_workers.stdout);
    assert!(
        owner_workers_stdout.contains("workers[1]{id,agent,task,age,window,wake,last_status}:")
    );
    assert!(owner_workers_stdout.contains("\n  shared,"));
    let tmux_before = env.tmux_log();

    let peek = env
        .niles(&workspace_a, &["peek", "shared"])
        .output()
        .unwrap();
    assert_failure_contains("foreign peek", &peek, "unknown worker id 'shared'");
    assert!(String::from_utf8_lossy(&peek.stdout).is_empty());

    let send = env
        .niles(&workspace_a, &["send", "shared", "continue"])
        .output()
        .unwrap();
    assert_failure_contains("foreign send", &send, "unknown worker id 'shared'");

    let wait = env
        .niles(
            &workspace_a,
            &["wait", "shared", "--interval", "10ms", "--timeout", "0"],
        )
        .output()
        .unwrap();
    assert_failure_contains("foreign wait", &wait, "unknown worker id 'shared'");

    let report = env
        .niles(&workspace_a, &["report", "shared"])
        .output()
        .unwrap();
    assert_failure_contains(
        "foreign report",
        &report,
        "no report found for worker 'shared'",
    );
    assert!(String::from_utf8_lossy(&report.stdout).is_empty());

    let close = env
        .niles(&workspace_a, &["close", "shared"])
        .output()
        .unwrap();
    assert_failure_contains("foreign close", &close, "no live worker 'shared'");

    assert_eq!(env.tmux_log(), tmux_before);
    let status_body = fs::read_to_string(&status).unwrap();
    assert!(!status_body.contains("closed: shared"));
    // A command run from another workspace must not have touched this worker's wake state.
    assert!(!worker_dir.join("status.cursor").exists());
    assert!(!workspace_b.join(".niles/worker/archive").exists());
    assert!(worker_dir.exists());
}

#[test]
fn archived_reports_are_workspace_local() {
    let env = TestEnv::new("niles-worker-scope-foreign-archive");
    let workspace_a = env.root.join("workspace-a");
    let workspace_b = env.root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    let spawn = env
        .niles(
            &workspace_b,
            &["spawn", "closed", "--agent", "claude", "Fix"],
        )
        .output()
        .unwrap();
    assert_command_success("spawn scoped worker", &spawn);
    let worker_dir = workspace_b.join(".niles/worker/closed");
    fs::write(worker_dir.join("report.md"), "closed worker report\n").unwrap();

    let close = env
        .niles(&workspace_b, &["close", "closed"])
        .output()
        .unwrap();
    assert_command_success("close worker in owner workspace", &close);

    let local_report = env
        .niles(&workspace_b, &["report", "closed"])
        .output()
        .unwrap();
    assert_command_success("local archived report", &local_report);
    assert_eq!(
        String::from_utf8_lossy(&local_report.stdout),
        "closed worker report\n"
    );

    let foreign_report = env
        .niles(&workspace_a, &["report", "closed"])
        .output()
        .unwrap();
    assert_failure_contains(
        "foreign archived report",
        &foreign_report,
        "no report found for worker 'closed'",
    );
    assert!(String::from_utf8_lossy(&foreign_report.stdout).is_empty());
}
