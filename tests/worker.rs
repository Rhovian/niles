mod common;

use common::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn spawn_enforces_known_agent_cli_min_version() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-version-spawn-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '0.1.0 (Claude Code)\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let spawn = Command::new(niles)
        .args([
            "spawn",
            "blocked-worker",
            "--agent",
            "claude",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("claude CLI 0.1.0 is below the supported minimum"));
    assert!(!workspace.join(".niles/worker/blocked-worker.json").exists());
}

#[test]
fn auth_spawn_peek_and_send_use_tmux_worker_metadata() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  capture-pane) printf 'pane output\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let spawn = Command::new(niles)
        .args([
            "spawn",
            "auth-fix",
            "--project",
            ".",
            "--agent",
            "claude",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        spawn.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&spawn.stdout),
        String::from_utf8_lossy(&spawn.stderr)
    );
    let spawn_stdout = String::from_utf8_lossy(&spawn.stdout);
    assert!(spawn_stdout.contains("spawned: auth-fix"));
    assert!(spawn_stdout.contains("window: niles-auth-fix"));
    assert!(spawn_stdout.contains("peek: niles peek auth-fix"));
    assert!(spawn_stdout.contains("close: niles worker-close auth-fix"));

    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix.json")).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains("\"window\": \"niles:niles-auth-fix\""));

    let brief = fs::read_to_string(workspace.join(".niles/worker/auth-fix/brief.md")).unwrap();
    assert!(brief.contains("Fix auth"));
    assert!(brief.contains("niles peek auth-fix"));

    let launch = fs::read_to_string(workspace.join(".niles/worker/auth-fix/launch.sh")).unwrap();
    assert!(launch.contains("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false"));
    assert!(launch.contains("exec 'claude'"));

    let peek = Command::new(niles)
        .args(["peek", "auth-fix", "--lines", "7"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(peek.status.success());
    assert_eq!(String::from_utf8_lossy(&peek.stdout), "pane output\n");

    let send = Command::new(niles)
        .args(["send", "auth-fix", "continue", "please"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(send.status.success());
    assert!(String::from_utf8_lossy(&send.stdout).contains("sent: auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-session -d -s niles"));
    assert!(log.contains("new-window -d -t niles: -n niles-auth-fix"));
    assert!(log.contains("capture-pane -p -t niles:niles-auth-fix -S -7"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix -l continue please"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix C-m"));
}

#[test]
fn spawn_window_failure_cleans_partial_worker_and_allows_respawn() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-failed-spawn");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  new-window)
    if [ "${TMUX_FAIL_NEW_WINDOW:-}" = 1 ]; then
      printf 'create window failed: index 1 in use\n' >&2
      exit 1
    fi
    exit 0
    ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let failed = Command::new(niles)
        .args([
            "spawn",
            "auth-fix",
            "--project",
            ".",
            "--agent",
            "claude",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_FAIL_NEW_WINDOW", "1")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("failed to launch worker auth-fix"));
    assert!(stderr.contains("create window failed: index 1 in use"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(!workspace.join(".niles/worker/auth-fix").exists());

    let peek = Command::new(niles)
        .args(["peek", "auth-fix"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!peek.status.success());
    assert!(String::from_utf8_lossy(&peek.stderr).contains("unknown worker id 'auth-fix'"));

    let respawn = Command::new(niles)
        .args([
            "spawn",
            "auth-fix",
            "--project",
            ".",
            "--agent",
            "claude",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("respawn", &respawn);
    assert!(workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(workspace.join(".niles/worker/auth-fix").is_dir());

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-window -d -t niles: -n niles-auth-fix"));
}

#[test]
fn ask_spawns_one_off_worker_window() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-ask-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let ask = Command::new(niles)
        .args(["ask", "-a", "claude", "Fix", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("ask", &ask);

    let stdout = String::from_utf8_lossy(&ask.stdout);
    let id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("spawned: "))
        .expect("ask output should include spawned worker id");
    assert!(id.starts_with("ask-claude-"));
    assert!(stdout.contains(&format!("window: niles-{id}")));
    assert!(stdout.contains(&format!("peek: niles peek {id}")));

    let meta = fs::read_to_string(workspace.join(format!(".niles/worker/{id}.json"))).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains(&format!("\"window\": \"niles:niles-{id}\"")));

    let brief = fs::read_to_string(workspace.join(format!(".niles/worker/{id}/brief.md"))).unwrap();
    assert!(brief.contains("Fix auth"));
    assert!(brief.contains(&format!("niles peek {id}")));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-session -d -s niles"));
    assert!(log.contains(&format!("new-window -d -t niles: -n niles-{id}")));
    assert!(!workspace.join(".niles/runs").exists());
}

#[test]
fn worker_close_tears_down_worker() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();

    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, "status").unwrap();
    fs::write(
        workspace.join(".niles/worker/auth-fix.json"),
        format!(
            r#"{{
  "id": "auth-fix",
  "agent": "codex",
  "project": "{}",
  "window": "niles:niles-auth-fix",
  "brief": "{}",
  "launch": "{}",
  "status": "{}"
}}
"#,
            workspace.display(),
            brief.display(),
            launch.display(),
            status.display()
        ),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        close.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(close_stdout.contains("closed window: niles-auth-fix"));
    assert!(close_stdout.contains("closed: auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("kill-window -t niles:niles-auth-fix"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
}

#[test]
fn worker_close_wakes_waiters_with_nonzero_closed_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-wait");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();

    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, "working: close requested").unwrap();
    fs::write(
        workspace.join(".niles/worker/auth-fix.json"),
        format!(
            r#"{{
  "id": "auth-fix",
  "agent": "codex",
  "project": "{}",
  "window": "niles:niles-auth-fix",
  "brief": "{}",
  "launch": "{}",
  "status": "{}"
}}
"#,
            workspace.display(),
            brief.display(),
            launch.display(),
            status.display()
        ),
    )
    .unwrap();

    let waiter = Command::new(niles)
        .args([
            "wait",
            "--worker",
            "auth-fix",
            "--interval",
            "0.05",
            "--timeout",
            "5",
        ])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("worker-close", &close);

    let started = Instant::now();
    let output = waiter.wait_with_output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("closed:"),
        "stdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("worker 'auth-fix' closed"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("timeout"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn worker_close_unknown_id_errors() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-missing");

    let close = Command::new(niles)
        .args(["worker-close", "missing"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!close.status.success());
    assert!(
        String::from_utf8_lossy(&close.stderr).contains("unknown worker id 'missing'"),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
}

#[test]
fn peek_and_send_run_step_require_recorded_window() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-step-window-test");

    let task = write_task(
        &workspace,
        r#"
goal: "Prepare an interactive step"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "needs window"
"#,
    );

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(
        prepare.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&prepare.stdout),
        String::from_utf8_lossy(&prepare.stderr)
    );

    let peek = Command::new(niles)
        .args(["peek", "--run", "latest", "--index", "1", "--lines", "5"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!peek.status.success());
    let peek_stderr = String::from_utf8_lossy(&peek.stderr);
    assert!(peek_stderr.contains("step 1 in run"));
    assert!(peek_stderr.contains("has no recorded window"));

    let send = Command::new(niles)
        .args(["send", "--run", "latest", "--index", "1", "continue"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!send.status.success());
    let send_stderr = String::from_utf8_lossy(&send.stderr);
    assert!(send_stderr.contains("step 1 in run"));
    assert!(send_stderr.contains("has no recorded window"));
}
