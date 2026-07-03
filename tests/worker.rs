mod common;

use common::*;
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
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
    let home = niles_home(&workspace);

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
        .env("NILES_HOME", &home)
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
    assert!(spawn_stdout.contains("report: niles report auth-fix"));
    assert!(spawn_stdout.contains("close: niles worker-close auth-fix"));

    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains("\"window\": \"niles:niles-auth-fix\""));

    let brief = fs::read_to_string(workspace.join(".niles/worker/auth-fix/brief.md")).unwrap();
    assert!(brief.contains("Fix auth"));
    assert!(brief.contains("niles peek auth-fix"));
    assert!(brief.contains("report_file:"));
    assert!(brief.contains(".niles/worker/auth-fix/report.md"));
    assert!(brief.contains("Write substantial deliverable content"));
    assert!(brief.contains("done: <short result>; report:"));

    let launch = fs::read_to_string(workspace.join(".niles/worker/auth-fix/launch.sh")).unwrap();
    assert!(launch.contains("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false"));
    assert!(launch.contains("exec 'claude'"));

    let peek = Command::new(niles)
        .args(["peek", "auth-fix", "--lines", "7"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
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
        .env("NILES_HOME", &home)
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
fn report_prints_worker_report_file() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-report");

    let worker_dir = write_worker_fixture(&workspace, "auth-fix", "working: report ready");
    fs::write(
        worker_dir.join("report.md"),
        "# Findings\n\n- durable content\n",
    )
    .unwrap();

    let report = Command::new(niles)
        .args(["report", "auth-fix"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("report", &report);
    assert_eq!(
        String::from_utf8_lossy(&report.stdout),
        "# Findings\n\n- durable content\n"
    );
}

#[test]
fn report_errors_helpfully_when_report_file_is_absent() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-report-missing");

    let worker_dir = write_worker_fixture(&workspace, "auth-fix", "working: no report yet");
    fs::write(worker_dir.join("final-pane.txt"), "pane tail\n").unwrap();

    let report = Command::new(niles)
        .args(["report", "auth-fix"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(!report.status.success());
    assert!(String::from_utf8_lossy(&report.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&report.stderr);
    assert!(stderr.contains("no report found for worker 'auth-fix'"));
    assert!(stderr.contains(".niles/worker/auth-fix/report.md"));
    assert!(stderr.contains("final pane snapshot is available"));
}

#[test]
fn peek_defaults_deep_and_zero_lines_captures_full_history() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-peek-deep");
    let home = niles_home(&workspace);
    write_worker_fixture(&workspace, "auth-fix", "working: inspect pane");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  capture-pane) printf 'pane output\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let default_peek = Command::new(niles)
        .args(["peek", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("default peek", &default_peek);

    let full_history_peek = Command::new(niles)
        .args(["peek", "auth-fix", "--lines", "0"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("full-history peek", &full_history_peek);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t niles:niles-auth-fix -S -2000")
    );
    assert!(
        log.lines()
            .any(|line| line == "capture-pane -p -t niles:niles-auth-fix -S -")
    );
}

#[test]
fn spawn_maps_model_effort_specs_into_worker_launches_and_metadata() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-tier-test");
    let home = niles_home(&workspace);

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
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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

    let codex_spawn = Command::new(niles)
        .args([
            "spawn",
            "codex-hi",
            "--project",
            ".",
            "--agent",
            "codex:gpt-5.5:xhigh",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("codex tiered spawn", &codex_spawn);
    let codex_stdout = String::from_utf8_lossy(&codex_spawn.stdout);
    assert!(codex_stdout.contains("agent: codex:gpt-5.5:xhigh"));
    assert!(codex_stdout.contains("agent_family: codex"));
    assert!(codex_stdout.contains("model: gpt-5.5"));
    assert!(codex_stdout.contains("effort: xhigh"));

    let codex_meta =
        fs::read_to_string(workspace.join(".niles/worker/codex-hi/meta.json")).unwrap();
    assert!(codex_meta.contains(r#""agent": "codex:gpt-5.5:xhigh""#));
    assert!(codex_meta.contains(r#""agent_family": "codex""#));
    assert!(codex_meta.contains(r#""model": "gpt-5.5""#));
    assert!(codex_meta.contains(r#""effort": "xhigh""#));

    let codex_launch =
        fs::read_to_string(workspace.join(".niles/worker/codex-hi/launch.sh")).unwrap();
    assert!(codex_launch.contains("'--dangerously-bypass-approvals-and-sandbox'"));
    assert!(codex_launch.contains("'--model' 'gpt-5.5'"));
    assert!(codex_launch.contains("'--config' 'model_reasoning_effort=\"xhigh\"'"));

    let claude_spawn = Command::new(niles)
        .args([
            "spawn",
            "claude-max",
            "--project",
            ".",
            "--agent",
            "claude:opus:max",
            "Review",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("claude tiered spawn", &claude_spawn);

    let claude_meta =
        fs::read_to_string(workspace.join(".niles/worker/claude-max/meta.json")).unwrap();
    assert!(claude_meta.contains(r#""agent": "claude:opus:max""#));
    assert!(claude_meta.contains(r#""agent_family": "claude""#));
    assert!(claude_meta.contains(r#""model": "opus""#));
    assert!(claude_meta.contains(r#""effort": "max""#));

    let claude_launch =
        fs::read_to_string(workspace.join(".niles/worker/claude-max/launch.sh")).unwrap();
    assert!(claude_launch.contains("'--dangerously-skip-permissions'"));
    assert!(claude_launch.contains("'--model' 'opus'"));
    assert!(claude_launch.contains("'--effort' 'max'"));
}

#[test]
fn spawn_rejects_invalid_model_effort_specs() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-invalid-tier-test");

    let spawn = Command::new(niles)
        .args(["spawn", "bad-worker", "--agent", "claude:opus:turbo", "Fix"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    assert!(String::from_utf8_lossy(&spawn.stderr).contains("unsupported claude effort `turbo`"));
    assert!(!workspace.join(".niles/worker/bad-worker").exists());
}

#[test]
fn spawned_worker_resolves_from_invoking_project_and_unrelated_cwds() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-cross-cwd");
    let home = niles_home(&root);
    let invoker = root.join("invoker");
    let project = root.join("project");
    let unrelated = root.join("unrelated");
    fs::create_dir_all(&invoker).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&unrelated).unwrap();

    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = root.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  capture-pane) printf 'pane output\n'; exit 0 ;;
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

    let spawn = Command::new(niles)
        .arg("spawn")
        .arg("auth-fix")
        .arg("--project")
        .arg(&project)
        .arg("--agent")
        .arg("claude")
        .args(["Fix", "auth"])
        .current_dir(&invoker)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd spawn", &spawn);

    let worker_dir = project.join(".niles/worker/auth-fix");
    let status = worker_dir.join("status.log");
    assert!(worker_dir.join("meta.json").is_file());
    assert!(invoker.join(".niles/worker/auth-fix.json").is_file());
    assert!(project.join(".niles/worker/auth-fix.json").is_file());
    assert!(!invoker.join(".niles/worker/auth-fix").exists());

    for cwd in [&invoker, &project, &unrelated] {
        let peek = Command::new(niles)
            .args(["peek", "auth-fix", "--lines", "7"])
            .current_dir(cwd)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success("cross-cwd peek", &peek);
        assert_eq!(String::from_utf8_lossy(&peek.stdout), "pane output\n");
    }

    let send = Command::new(niles)
        .args(["send", "auth-fix", "continue", "please"])
        .current_dir(&unrelated)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd send", &send);

    for (index, cwd) in [&invoker, &project, &unrelated].into_iter().enumerate() {
        let mut status_file = fs::OpenOptions::new().append(true).open(&status).unwrap();
        writeln!(status_file, "done: wake {index}").unwrap();

        let wait = Command::new(niles)
            .args([
                "wait",
                "--worker",
                "auth-fix",
                "--interval",
                "0.05",
                "--timeout",
                "0",
            ])
            .current_dir(cwd)
            .env("NILES_HOME", &home)
            .output()
            .unwrap();
        assert_command_success("cross-cwd wait", &wait);
        assert_eq!(
            String::from_utf8_lossy(&wait.stdout),
            format!("done: wake {index}\n")
        );
    }

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&unrelated)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd worker-close", &close);

    assert!(!invoker.join(".niles/worker/auth-fix.json").exists());
    assert!(!project.join(".niles/worker/auth-fix.json").exists());
    assert!(worker_dir.is_dir());
    assert!(!worker_dir.join("meta.json").exists());
    assert_eq!(
        fs::read_to_string(worker_dir.join("final-pane.txt")).unwrap(),
        "pane output\n"
    );
    assert_global_index_lacks(&home, "auth-fix");
}

#[test]
fn spawn_window_failure_cleans_partial_worker_and_allows_respawn() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-failed-spawn");
    let home = niles_home(&workspace);

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
        .env("NILES_HOME", &home)
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
    assert_global_index_lacks(&home, "auth-fix");

    let peek = Command::new(niles)
        .args(["peek", "auth-fix"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
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
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("respawn", &respawn);
    assert!(workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(workspace.join(".niles/worker/auth-fix").is_dir());
    assert!(workspace.join(".niles/worker/auth-fix/meta.json").is_file());

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-window -d -t niles: -n niles-auth-fix"));
}

#[test]
fn ask_spawns_one_off_worker_window() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-ask-test");
    let home = niles_home(&workspace);

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
        .env("NILES_HOME", &home)
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

    let meta = fs::read_to_string(workspace.join(format!(".niles/worker/{id}/meta.json"))).unwrap();
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
    let home = niles_home(&workspace);

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
  capture-pane) printf 'final pane\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();

    write_worker_fixture(&workspace, "auth-fix", "status");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::write(worker_dir.join("report.md"), "durable report\n").unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
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
    assert!(close_stdout.contains("pane:"));
    assert!(close_stdout.contains("closed window: niles-auth-fix"));
    assert!(close_stdout.contains("closed: auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("capture-pane -p -t niles:niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t niles:niles-auth-fix"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(worker_dir.is_dir());
    assert!(!worker_dir.join("meta.json").exists());
    assert_eq!(
        fs::read_to_string(worker_dir.join("final-pane.txt")).unwrap(),
        "final pane\n"
    );
    assert_eq!(
        fs::read_to_string(worker_dir.join("report.md")).unwrap(),
        "durable report\n"
    );
    assert_global_index_lacks(&home, "auth-fix");
}

#[test]
fn worker_close_old_metadata_reports_schema_skew_without_raw_serde_error() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-old-meta");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(
        worker_dir.join("meta.json"),
        r#"{
  "id": "auth-fix",
  "agent": "codex",
  "window": "niles:niles-auth-fix",
  "brief": "brief.md",
  "launch": "launch.sh"
}
"#,
    )
    .unwrap();

    let output = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("worker metadata"));
    assert!(stderr.contains("meta.json"));
    assert!(stderr.contains("schema 1"));
    assert!(stderr.contains("expects 2"));
    assert!(stderr.contains("remove the worker dir and respawn"));
    assert!(!stderr.contains("missing field"));
}

#[test]
fn worker_close_wakes_waiters_with_nonzero_closed_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-wait");
    let home = niles_home(&workspace);

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

    write_worker_fixture(&workspace, "auth-fix", "working: close requested");

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
        .env("NILES_HOME", &home)
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
        .env("NILES_HOME", &home)
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
    let home = niles_home(&workspace);

    let close = Command::new(niles)
        .args(["worker-close", "missing"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
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

fn write_worker_fixture(workspace: &Path, id: &str, status_body: &str) -> PathBuf {
    let worker_root = workspace.join(".niles/worker");
    let worker_dir = worker_root.join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, status_body).unwrap();
    fs::write(
        worker_root.join(format!("{id}.json")),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "workspace": "{}",
  "worker_dir": "{}",
  "local_stores": ["{}"]
}}
"#,
            workspace.display(),
            worker_dir.display(),
            worker_root.display()
        ),
    )
    .unwrap();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
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
    worker_dir
}

fn assert_global_index_lacks(home: &Path, id: &str) {
    let path = home.join("runs/index.json");
    if path.exists() {
        let index = fs::read_to_string(&path).unwrap();
        assert!(!index.contains(id), "global index retained {id}:\n{index}");
    }
}
