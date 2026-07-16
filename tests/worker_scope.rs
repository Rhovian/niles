mod common;

use common::{assert_command_success, niles_home, temp_workspace, write_executable};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[test]
fn by_id_commands_do_not_reach_worker_in_another_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-scope-foreign-live");
    let home = niles_home(&root);
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    let (bin, tmux_log) = write_scope_test_bins(&root);
    let path = path_with_bin(&bin);

    spawn_worker(niles, &workspace_b, &home, &path, &tmux_log, "shared");
    let worker_dir = workspace_b.join(".niles/worker/shared");
    fs::write(worker_dir.join("report.md"), "workspace B report\n").unwrap();
    let status = worker_dir.join("status.log");

    fs::write(home.join("runs/index.json"), "{ invalid global index").unwrap();
    let tmux_before = fs::read_to_string(&tmux_log).unwrap();

    let peek = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args(["peek", "shared"])
        .output()
        .unwrap();
    assert_failure_contains("foreign peek", &peek, "unknown worker id 'shared'");
    assert!(String::from_utf8_lossy(&peek.stdout).is_empty());

    let send = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args(["send", "shared", "continue"])
        .output()
        .unwrap();
    assert_failure_contains("foreign send", &send, "unknown worker id 'shared'");

    let wait = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args([
            "wait",
            "--worker",
            "shared",
            "--interval",
            "0.01",
            "--timeout",
            "0",
        ])
        .output()
        .unwrap();
    assert_failure_contains("foreign wait", &wait, "unknown worker id 'shared'");

    let report = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args(["report", "shared"])
        .output()
        .unwrap();
    assert_failure_contains(
        "foreign report",
        &report,
        "no report found for worker 'shared'",
    );
    assert!(String::from_utf8_lossy(&report.stdout).is_empty());

    let close = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args(["worker-close", "shared"])
        .output()
        .unwrap();
    assert_failure_contains("foreign close", &close, "no live worker 'shared'");

    assert_eq!(fs::read_to_string(&tmux_log).unwrap(), tmux_before);
    let status_body = fs::read_to_string(&status).unwrap();
    assert!(!status_body.contains("closed: shared"));
    assert!(!worker_dir.join("status.waiter").exists());
    assert!(!worker_dir.join("status.ack").exists());
    assert!(!worker_dir.join("status.ack.log").exists());
    assert!(!workspace_b.join(".niles/worker/archive").exists());
    assert!(worker_dir.exists());
}

#[test]
fn archived_reports_are_workspace_local() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-scope-foreign-archive");
    let home = niles_home(&root);
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    let (bin, tmux_log) = write_scope_test_bins(&root);
    let path = path_with_bin(&bin);

    spawn_worker(niles, &workspace_b, &home, &path, &tmux_log, "closed");
    let worker_dir = workspace_b.join(".niles/worker/closed");
    fs::write(worker_dir.join("report.md"), "closed worker report\n").unwrap();

    let close = scoped_command(niles, &workspace_b, &home, &path, &tmux_log)
        .args(["worker-close", "closed"])
        .output()
        .unwrap();
    assert_command_success("close worker in owner workspace", &close);

    let local_report = scoped_command(niles, &workspace_b, &home, &path, &tmux_log)
        .args(["report", "closed"])
        .output()
        .unwrap();
    assert_command_success("local archived report", &local_report);
    assert_eq!(
        String::from_utf8_lossy(&local_report.stdout),
        "closed worker report\n"
    );

    let foreign_report = scoped_command(niles, &workspace_a, &home, &path, &tmux_log)
        .args(["report", "closed"])
        .output()
        .unwrap();
    assert_failure_contains(
        "foreign archived report",
        &foreign_report,
        "no report found for worker 'closed'",
    );
    assert!(String::from_utf8_lossy(&foreign_report.stdout).is_empty());
}

fn spawn_worker(niles: &str, workspace: &Path, home: &Path, path: &str, tmux_log: &Path, id: &str) {
    let spawn = scoped_command(niles, workspace, home, path, tmux_log)
        .args(["spawn", id, "--project", ".", "--agent", "claude", "Fix"])
        .output()
        .unwrap();
    assert_command_success("spawn scoped worker", &spawn);
}

fn scoped_command(
    niles: &str,
    workspace: &Path,
    home: &Path,
    path: &str,
    tmux_log: &Path,
) -> Command {
    let mut command = Command::new(niles);
    command
        .current_dir(workspace)
        .env("PATH", path)
        .env("NILES_HOME", home)
        .env("TMUX_LOG", tmux_log)
        .env_remove("TMUX");
    command
}

fn assert_failure_contains(label: &str, output: &Output, needle: &str) {
    assert!(
        !output.status.success(),
        "{label} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "{label} stderr did not contain {needle:?}\nstderr:\n{stderr}"
    );
}

fn write_scope_test_bins(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = root.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 0 ;;
  list-windows)
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
  capture-pane) printf 'pane output\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    (bin, tmux_log)
}

fn path_with_bin(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}
