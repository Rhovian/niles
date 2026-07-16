mod common;

use common::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

fn capability_manifest_containing(workspace: &Path, needle: &str) -> String {
    let dir = workspace.join(".niles/capabilities");
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let body = fs::read_to_string(&path).unwrap();
        if body.contains(needle) {
            return body;
        }
    }
    panic!(
        "no capability manifest under {} contained {needle}",
        dir.display()
    );
}

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
fn spawn_rejects_reserved_archive_worker_id() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-reserved-archive");

    let spawn = Command::new(niles)
        .args(["spawn", "archive", "--agent", "claude", "Fix", "auth"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("worker id 'archive' is reserved"));
    assert!(!workspace.join(".niles/worker/archive").exists());
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
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
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
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
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
            "--task",
            "auth",
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
    assert!(spawn_stdout.contains("task: auth"));
    assert!(spawn_stdout.contains("peek: niles peek auth-fix"));
    assert!(spawn_stdout.contains("report: niles report auth-fix"));
    assert!(spawn_stdout.contains("close: niles worker-close auth-fix"));
    assert!(spawn_stdout.contains("close_task: niles worker-close --task auth"));
    assert!(spawn_stdout.contains("workers: niles workers"));

    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains("\"agent\": \"claude\""));
    assert!(meta.contains("\"task_label\": \"auth\""));
    assert!(meta.contains("\"created_at\":"));
    let meta_json: serde_json::Value = serde_json::from_str(&meta).unwrap();
    let window = meta_json["window"].as_str().unwrap();
    let project = meta_json["project"].as_str().unwrap();
    assert!(window.starts_with("niles-niles-worker-test-"));
    assert!(window.ends_with(":niles-auth-fix"));

    let brief = fs::read_to_string(workspace.join(".niles/worker/auth-fix/brief.md")).unwrap();
    assert!(brief.contains("task_label: auth"));
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
    assert!(log.contains("new-session -d -s niles-niles-worker-test-"));
    assert!(log.contains("new-window -d -t niles-niles-worker-test-"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains(&format!(
        "set-option -w -t {window} @niles-project {project}"
    )));
    assert!(log.contains(&format!(
        "set-option -w -t {window} @niles-worker-id auth-fix"
    )));
    assert!(log.contains(&format!("capture-pane -p -t {window} -S -7")));
    assert!(log.contains(&format!("send-keys -t {window} -l continue please")));
    assert!(log.contains(&format!("send-keys -t {window} C-m")));
}

#[test]
fn spawn_pins_worker_to_manager_session_and_tags_window_not_ambient() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-pinned-session");
    let home = niles_home(&workspace);

    let session_dir = workspace.join(".niles/sessions/session-1");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(workspace.join(".niles/sessions/latest"), "session-1").unwrap();
    fs::write(
        session_dir.join("session.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "session-1",
  "agent": "codex",
  "created_at": "2026-07-06T00:00:00Z",
  "workspace": "{}",
  "brief": "{}",
  "window": "home:niles-manager"
}}"#,
            workspace.display(),
            session_dir.join("manager.md").display()
        ),
    )
    .unwrap();

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'ambient\n'; exit 0 ;;
  has-session)
    if [ "$3" = home ]; then exit 0; fi
    exit 1
    ;;
  list-windows) exit 0 ;;
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

    let path = path_with_bin(&bin);
    let spawn = Command::new(niles)
        .args([
            "spawn",
            "auth-fix",
            "--project",
            ".",
            "--agent",
            "claude",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/ambient-tmux")
        .output()
        .unwrap();
    assert_command_success("pinned manager-session spawn", &spawn);

    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains(r#""window": "home:niles-auth-fix""#));
    let pointer = fs::read_to_string(workspace.join(".niles/sessions/tmux-session.json")).unwrap();
    assert!(pointer.contains(r#""session": "home""#));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("display-message"));
    assert!(log.contains("has-session -t home"));
    assert!(log.contains("new-window -d -t home: -n niles-auth-fix"));
    assert!(log.contains("set-option -w -t home:niles-auth-fix @niles-project"));
    assert!(log.contains("set-option -w -t home:niles-auth-fix @niles-worker-id auth-fix"));
}

#[test]
fn spawn_rejects_reserved_archive_task_label() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-reserved-task-label");

    let spawn = Command::new(niles)
        .args([
            "spawn", "auth-fix", "--task", "archive", "--agent", "claude", "Fix", "auth",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("task label 'archive' is reserved"));
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
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
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
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
fn spawn_falls_back_to_static_model_validation_without_manifest() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-static-model-fallback-test");

    let spawn = Command::new(niles)
        .args(["spawn", "bad-worker", "--agent", "codex:omega:xhigh", "Fix"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    assert!(String::from_utf8_lossy(&spawn.stderr).contains("unsupported codex model `omega`"));
    assert!(!workspace.join(".niles/worker/bad-worker").exists());
}

#[test]
fn spawn_uses_fresh_capability_manifest_for_accepted_and_rejected_models() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-capability-model-test");
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
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
esac
model=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$model" in
  omega) printf 'accepted %s\n' "$model"; exit 0 ;;
  gpt-bad) printf 'unknown model %s\n' "$model" >&2; exit 8 ;;
  *) printf 'accepted %s\n' "$model"; exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let accepted_analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:omega:xhigh"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("accepted model analyze", &accepted_analyze);

    let accepted_spawn = Command::new(niles)
        .args([
            "spawn",
            "accepted-worker",
            "--project",
            ".",
            "--agent",
            "codex:omega:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("accepted model spawn", &accepted_spawn);
    let stdout = String::from_utf8_lossy(&accepted_spawn.stdout);
    assert!(stdout.contains("model: omega"));
    let launch =
        fs::read_to_string(workspace.join(".niles/worker/accepted-worker/launch.sh")).unwrap();
    assert!(launch.contains("'--model' 'omega'"));

    let rejected_analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:gpt-bad:xhigh"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("rejected model analyze", &rejected_analyze);

    let rejected_spawn = Command::new(niles)
        .args([
            "spawn",
            "rejected-worker",
            "--project",
            ".",
            "--agent",
            "codex:gpt-bad:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!rejected_spawn.status.success());
    let stderr = String::from_utf8_lossy(&rejected_spawn.stderr);
    assert!(stderr.contains("model `gpt-bad` was rejected by codex CLI 0.144.1"));
    assert!(!workspace.join(".niles/worker/rejected-worker").exists());
}

#[test]
fn analyze_uses_project_configured_agent_binary_for_model_probes() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-configured-analyze-test");
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
printf 'default codex should not be probed\n' >&2
exit 12
"#,
    );
    let custom_codex = bin.join("codex-custom");
    write_executable(
        &custom_codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'custom codex help\n'; exit 0 ;;
esac
printf 'custom accepted\n'
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            "agents:\n  codex:\n    binary: {}\n",
            custom_codex.display()
        ),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:omega:xhigh"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("configured binary analyze", &analyze);

    let manifest = capability_manifest_containing(
        &workspace,
        &format!(r#""binary": "{}""#, custom_codex.display()),
    );
    assert!(manifest.contains(&format!(r#""binary": "{}""#, custom_codex.display())));
    assert!(manifest.contains(r#""model": "omega""#));
    assert!(manifest.contains(r#""effort": "xhigh""#));

    let accepted_spawn = Command::new(niles)
        .args([
            "spawn",
            "configured-worker",
            "--project",
            ".",
            "--agent",
            "codex:omega:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("configured binary accepted spawn", &accepted_spawn);

    let unprobed_effort_spawn = Command::new(niles)
        .args([
            "spawn",
            "configured-low-worker",
            "--project",
            ".",
            "--agent",
            "codex:omega:low",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!unprobed_effort_spawn.status.success());
    let stderr = String::from_utf8_lossy(&unprobed_effort_spawn.stderr);
    assert!(stderr.contains("unsupported codex model `omega`"));
    assert!(
        !workspace
            .join(".niles/worker/configured-low-worker")
            .exists()
    );
}

#[test]
fn analyze_sends_probe_prompt_to_configured_stdin_agents() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-stdin-analyze-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let stdin_codex = bin.join("codex-stdin");
    write_executable(
        &stdin_codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'stdin codex help\n'; exit 0 ;;
esac
model=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    *) shift ;;
  esac
done
prompt=$(cat)
if [ -z "$prompt" ]; then
  printf 'missing stdin prompt for %s\n' "$model" >&2
  exit 5
fi
printf 'accepted %s with stdin\n' "$model"
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            "agents:\n  codex:\n    binary: {}\n    prompt: stdin\n",
            stdin_codex.display()
        ),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:omega:xhigh"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("stdin configured analyze", &analyze);
    assert!(
        String::from_utf8_lossy(&analyze.stdout)
            .contains("model_probe: codex:omega:xhigh accepted")
    );

    let manifest = capability_manifest_containing(
        &workspace,
        &format!(r#""binary": "{}""#, stdin_codex.display()),
    );
    assert!(manifest.contains(r#""accepted_models""#));
    assert!(manifest.contains(r#""model": "omega""#));
    assert!(!manifest.contains("missing stdin prompt"));
}

#[test]
fn default_analyze_writes_manifest_for_exact_configured_role_binary() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-default-analyze-mixed-binary-test");
    let home = niles_home(&workspace);
    write_workspace_manifest(
        &workspace,
        "claude",
        "codex:omega:xhigh",
        "codex",
        "claude",
        "test",
    );

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
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'default codex help\n'; exit 0 ;;
esac
printf 'default codex accepted\n'
"#,
    );
    let custom_codex = bin.join("codex-custom");
    write_executable(
        &custom_codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'custom codex help\n'; exit 0 ;;
esac
printf 'custom codex accepted\n'
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            "agents:\n  \"codex:omega:xhigh\":\n    binary: {}\n",
            custom_codex.display()
        ),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .arg("analyze")
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("default analyze mixed binaries", &analyze);
    assert!(workspace.join(".niles/capabilities/codex.json").is_file());
    let custom_manifest = capability_manifest_containing(
        &workspace,
        &format!(r#""binary": "{}""#, custom_codex.display()),
    );
    assert!(custom_manifest.contains(r#""model": "omega""#));
    assert!(custom_manifest.contains(r#""effort": "xhigh""#));

    let spawn = Command::new(niles)
        .args([
            "spawn",
            "configured-role-worker",
            "--project",
            ".",
            "--agent",
            "codex:omega:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("configured role binary spawn", &spawn);
}

#[test]
fn binary_specific_capability_manifest_paths_resist_slug_collisions() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-binary-slug-collision-test");
    let home = niles_home(&workspace);
    write_workspace_manifest(
        &workspace,
        "claude",
        "codex:omega:xhigh",
        "codex:theta:xhigh",
        "claude",
        "test",
    );

    let bin = workspace.join("bin");
    fs::create_dir_all(bin.join("a")).unwrap();
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
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'default codex help\n'; exit 0 ;;
esac
printf 'default codex accepted\n'
"#,
    );

    let slash_binary = bin.join("a/b");
    write_executable(
        &slash_binary,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'slash codex help\n'; exit 0 ;;
esac
model=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$model" in
  omega) printf 'slash accepted omega\n'; exit 0 ;;
  *) printf 'slash rejected %s\n' "$model" >&2; exit 7 ;;
esac
"#,
    );
    let dash_binary = bin.join("a-b");
    write_executable(
        &dash_binary,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'dash codex help\n'; exit 0 ;;
esac
model=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$model" in
  theta) printf 'dash accepted theta\n'; exit 0 ;;
  *) printf 'dash rejected %s\n' "$model" >&2; exit 8 ;;
esac
"#,
    );
    fs::write(
        workspace.join("niles.yaml"),
        format!(
            "agents:\n  \"codex:omega:xhigh\":\n    binary: {}\n  \"codex:theta:xhigh\":\n    binary: {}\n",
            slash_binary.display(),
            dash_binary.display()
        ),
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .arg("analyze")
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("default analyze slug-colliding binaries", &analyze);

    let slash_manifest = capability_manifest_containing(
        &workspace,
        &format!(r#""binary": "{}""#, slash_binary.display()),
    );
    assert!(slash_manifest.contains(r#""model": "omega""#));
    assert!(!slash_manifest.contains(r#""model": "theta""#));
    let dash_manifest = capability_manifest_containing(
        &workspace,
        &format!(r#""binary": "{}""#, dash_binary.display()),
    );
    assert!(dash_manifest.contains(r#""model": "theta""#));
    assert!(!dash_manifest.contains(r#""model": "omega""#));

    for (id, agent) in [
        ("slug-omega", "codex:omega:xhigh"),
        ("slug-theta", "codex:theta:xhigh"),
    ] {
        let spawn = Command::new(niles)
            .args(["spawn", id, "--project", ".", "--agent", agent, "Fix"])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success(&format!("spawn {agent}"), &spawn);
    }
}

#[test]
fn capability_model_probe_matching_includes_effort() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-effort-probe-test");
    let home = niles_home(&workspace);
    write_workspace_manifest(
        &workspace,
        "claude",
        "codex:omega:xhigh",
        "codex:omega:low",
        "claude",
        "test",
    );

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
        &bin.join("claude"),
        r#"#!/bin/sh
case "$1" in
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
esac
model=""
effort=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    --config)
      case "$2" in
        *model_reasoning_effort=\"xhigh\"*) effort="xhigh" ;;
        *model_reasoning_effort=\"low\"*) effort="low" ;;
      esac
      shift 2
      ;;
    *) shift ;;
  esac
done
case "$model:$effort" in
  omega:xhigh) printf 'accepted omega xhigh\n'; exit 0 ;;
  omega:low) printf 'rejected omega low\n' >&2; exit 6 ;;
  *) printf 'accepted %s %s\n' "$model" "$effort"; exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .arg("analyze")
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("effort-specific analyze", &analyze);

    let stdout = String::from_utf8_lossy(&analyze.stdout);
    assert!(stdout.contains("model_probe: codex:omega:xhigh accepted"));
    assert!(stdout.contains("model_probe: codex:omega:low not accepted"));

    let accepted_spawn = Command::new(niles)
        .args([
            "spawn",
            "omega-xhigh",
            "--project",
            ".",
            "--agent",
            "codex:omega:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("accepted effort spawn", &accepted_spawn);

    let rejected_spawn = Command::new(niles)
        .args([
            "spawn",
            "omega-low",
            "--project",
            ".",
            "--agent",
            "codex:omega:low",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!rejected_spawn.status.success());
    let stderr = String::from_utf8_lossy(&rejected_spawn.stderr);
    assert!(stderr.contains("model `omega` was rejected by codex CLI 0.144.1"));
    assert!(!workspace.join(".niles/worker/omega-low").exists());
}

#[test]
fn spawn_warns_and_falls_back_when_capability_manifest_is_stale() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-stale-capability-test");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    write_executable(
        &codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.144.1\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
esac
printf 'accepted\n'
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex:omega:xhigh"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("stale manifest seed analyze", &analyze);

    write_executable(
        &codex,
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.200.0\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
esac
printf 'accepted\n'
"#,
    );

    let spawn = Command::new(niles)
        .args([
            "spawn",
            "stale-worker",
            "--agent",
            "codex:omega:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("warning: capability manifest"));
    assert!(stderr.contains("current CLI is 0.200.0"));
    assert!(stderr.contains("unsupported codex model `omega`"));
    assert!(!workspace.join(".niles/worker/stale-worker").exists());
}

#[test]
fn spawn_rejects_cross_workspace_project_without_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-cross-cwd");
    let home = niles_home(&root);
    let invoker = root.join("invoker");
    let project = root.join("project");
    let foreign_link = root.join("foreign-link");
    fs::create_dir_all(&invoker).unwrap();
    fs::create_dir_all(&project).unwrap();
    symlink(&project, &foreign_link).unwrap();

    for (id, project_arg) in [
        ("absolute-foreign", project.as_os_str()),
        ("relative-foreign", std::ffi::OsStr::new("../project")),
        ("symlink-foreign", foreign_link.as_os_str()),
    ] {
        let spawn = Command::new(niles)
            .arg("spawn")
            .arg(id)
            .arg("--project")
            .arg(project_arg)
            .arg("--agent")
            .arg("claude")
            .args(["Fix", "auth"])
            .current_dir(&invoker)
            .env("NILES_HOME", &home)
            .env_remove("TMUX")
            .output()
            .unwrap();

        assert!(!spawn.status.success(), "{id} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&spawn.stderr)
                .contains("spawn --project must be the current workspace; cd there and spawn"),
            "{id} stderr:\n{}",
            String::from_utf8_lossy(&spawn.stderr)
        );
        assert!(!invoker.join(".niles/worker").join(id).exists());
        assert!(!project.join(".niles/worker").join(id).exists());
    }
    assert!(!home.join("runs/index.json").exists());
}

#[test]
fn spawn_accepts_project_symlink_to_current_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-current-symlink");
    let home = niles_home(&root);
    let workspace = root.join("workspace");
    let current_link = root.join("current-link");
    fs::create_dir_all(&workspace).unwrap();
    symlink(&workspace, &current_link).unwrap();
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    let spawn = Command::new(niles)
        .arg("spawn")
        .arg("auth-fix")
        .arg("--project")
        .arg(&current_link)
        .arg("--agent")
        .arg("claude")
        .arg("Fix")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert_command_success("symlink-to-current spawn", &spawn);
    assert!(workspace.join(".niles/worker/auth-fix").is_dir());
    assert!(workspace.join(".niles/worker/auth-fix/meta.json").is_file());
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(meta.contains(&workspace.display().to_string()));
    assert!(!meta.contains(&current_link.display().to_string()));
}

#[test]
fn leftover_worker_json_file_is_inert() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-json-inert");
    let home = niles_home(&workspace);
    fs::create_dir_all(workspace.join(".niles/worker")).unwrap();
    fs::write(
        workspace.join(".niles/worker/auth-fix.json"),
        r#"{"id":"auth-fix"}"#,
    )
    .unwrap();

    let workers = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("workers ignores leftover json", &workers);
    let stdout = String::from_utf8_lossy(&workers.stdout);
    assert!(stdout.contains("workers[0]{id,agent,task,age,window,last_status}:"));
    assert!(!stdout.contains("auth-fix"));

    let peek = Command::new(niles)
        .args(["peek", "auth-fix"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert!(!peek.status.success());
    assert!(String::from_utf8_lossy(&peek.stderr).contains("unknown worker id 'auth-fix'"));
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
  --version) printf '2.1.206 (Claude Code)\n'; exit 0 ;;
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
    assert_global_index_absent(&home);

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
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(workspace.join(".niles/worker/auth-fix").is_dir());
    assert!(workspace.join(".niles/worker/auth-fix/meta.json").is_file());

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-window -d -t niles-niles-worker-failed-spawn-"));
    assert!(log.contains(": -n niles-auth-fix"));
}

#[test]
fn spawn_meta_write_failure_kills_window_and_cleans_location() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-meta-write-failed-spawn");
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
  new-window) mkdir -p "$META_PATH"; exit 0 ;;
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
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env(
            "META_PATH",
            workspace.join(".niles/worker/auth-fix/meta.json"),
        )
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("failed to finish launching worker auth-fix"));
    assert!(stderr.contains("cleaned up launched worker"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
    assert_global_index_absent(&home);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-window -d -t niles-niles-worker-meta-write-failed-spawn-"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains("set-option -w -t "));
    assert!(log.contains(" @niles-worker-id auth-fix"));
    assert!(log.contains("kill-window -t "));
    assert!(log.contains(":niles-auth-fix"));
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
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    printf 'niles-auth-fix\n'
    exit 0
    ;;
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
    assert!(close_stdout.contains("archive:"));
    assert!(close_stdout.contains("closed window: niles-auth-fix"));
    assert!(close_stdout.contains("closed: auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("capture-pane -p -t niles:niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t niles:niles-auth-fix"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(!worker_dir.exists());
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert_eq!(
        fs::read_to_string(archive_dir.join("final-pane.txt")).unwrap(),
        "final pane\n"
    );
    assert_eq!(
        fs::read_to_string(archive_dir.join("report.md")).unwrap(),
        "durable report\n"
    );
    assert_global_index_absent(&home);
}

#[test]
fn worker_close_targets_recorded_session_not_ambient() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-recorded");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_WINDOWS", "niles-auth-fix")
        .env("TMUX", "/tmp/ambient-tmux")
        .output()
        .unwrap();
    assert_command_success("recorded-target worker-close", &close);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("display-message"));
    assert!(log.contains("list-windows -t home -F #{window_name}"));
    assert!(log.contains("capture-pane -p -t home:niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t home:niles-auth-fix"));
}

#[test]
fn worker_close_recovers_renamed_orphan_by_matching_tags() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-recovered");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!("new:niles-renamed\t{}\tauth-fix", workspace.display());
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("recovered orphan worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:old:niles-auth-fix->new:niles-renamed")
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("capture-pane -p -t new:niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t new:niles-renamed"));
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains("closed: auth-fix")
    );
}

#[test]
fn worker_close_ignores_same_id_tag_from_other_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-other-workspace");
    let other_workspace = temp_workspace("niles-worker-close-other-project");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!(
        "other:niles-auth-fix\t{}\tauth-fix",
        other_workspace.display()
    );
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("cross-workspace tag worker-close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: orphan-gone"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
    fs::remove_dir_all(other_workspace).unwrap();
}

#[test]
fn worker_close_multiple_tag_matches_reaps_without_kill() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-multiple-tags");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let tagged = format!(
        "one:niles-auth-fix\t{}\tauth-fix\ntwo:niles-auth-fix\t{}\tauth-fix",
        workspace.display(),
        workspace.display()
    );
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("multiple tagged orphan worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: unknown:multiple tmux windows carry worker tags"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_recovers_window_missing_by_matching_tags() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-window-missing-recovered");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let tagged = format!("other:niles-renamed\t{}\tauth-fix", workspace.display());
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", tagged)
        .output()
        .unwrap();
    assert_command_success("window-missing recovered worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains("window state: orphan-recovered:home:niles-auth-fix->other:niles-renamed")
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("list-windows -t home -F #{window_name}"));
    assert!(log.contains("capture-pane -p -t other:niles-renamed -S -2000"));
    assert!(log.contains("kill-window -t other:niles-renamed"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_window_missing_without_tag_is_window_dead() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-window-missing-dead");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("window-missing dead worker-close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: window-dead"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reports_legacy_candidate_without_auto_kill() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-legacy-candidate");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_TAGGED_WINDOWS", "other:niles-auth-fix\t\t")
        .output()
        .unwrap();
    assert_command_success("legacy candidate worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-legacy-candidate:other:niles-auth-fix"));
    assert!(stdout.contains("manual_close: tmux kill-window -t other:niles-auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reaps_unparseable_meta_window_as_unknown() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-invalid-window");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("invalid-window worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(
        stdout.contains(
            "window state: unknown:worker auth-fix metadata has invalid tmux window target"
        )
    );

    assert!(!tmux_log.exists());
    assert_archived_with_closed_sentinel(&workspace, "auth-fix");
}

#[test]
fn worker_close_reaps_session_gone_orphan_without_tmux_error() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-gone");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "old");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "old:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("gone orphan worker-close", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("window state: orphan-gone"));
    assert!(!stdout.contains("can't find session"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("kill-window"));
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains("closed: auth-fix")
    );
}

#[test]
fn worker_close_reaps_current_schema_legacy_missing_session_meta() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-aquila");
    let home = niles_home(&workspace);
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_orphan_recovery_tmux(&bin, "aquila");
    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "aquila:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .output()
        .unwrap();
    assert_command_success("back-compat missing session close", &close);
    assert!(String::from_utf8_lossy(&close.stdout).contains("window state: orphan-gone"));
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
    assert!(
        latest_archive_dir(&workspace, "auth-fix")
            .join("meta.json")
            .is_file()
    );
}

#[test]
fn workers_lists_live_workers_with_task_age_and_last_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-list");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_task(
        &workspace,
        "auth-fix",
        "working: running tests\ndone: ready for review\n",
        Some("auth"),
    );
    write_worker_fixture(
        &workspace,
        "reviewer",
        "working: reading diff\nblocked: needs clarification\n",
    );
    let archive = workspace.join(".niles/worker/archive/old-worker-20000101T000000000000000Z");
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("status.log"), "done: archived\n").unwrap();

    let output = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_WINDOWS", "niles-auth-fix")
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("workers[2]{id,agent,task,age,window,last_status}:"));
    assert!(stdout.lines().any(|line| {
        line.contains("auth-fix,codex,auth,")
            && line.contains(",live,")
            && line.contains("done: ready for review")
    }));
    assert!(stdout.lines().any(|line| {
        line.contains("reviewer,codex,-,")
            && line.contains(",window-dead,")
            && line.contains("blocked: needs clarification")
    }));
    assert!(!stdout.contains("old-worker"));
}

#[test]
fn workers_usage_view_sums_live_worker_usage_by_task() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-usage");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);
    let codex_home = workspace.join("codex-home");
    let claude_home = workspace.join("claude-home");
    let pending_workspace = workspace.join("pending-workspace");
    fs::create_dir_all(&pending_workspace).unwrap();

    write_codex_usage_rollout(
        &codex_home,
        &workspace,
        "session-codex",
        "2026-07-06T00:00:01Z",
        (10, 3, 5, 2, 18),
    );
    write_claude_usage_transcript(
        &claude_home,
        &workspace,
        "00000000-0000-4000-8000-000000000052",
        (7, 8, 9, 6),
    );

    write_usage_worker_fixture(
        &workspace,
        "auth-codex",
        "tokledger",
        "codex",
        Some(("codex", None, None)),
        json!({
            "strategy": "codex_cwd_time",
            "cwd": workspace.display().to_string(),
            "launched_at": "2026-07-06T00:00:00Z",
            "niles_prompt_count": 1
        }),
    );
    write_usage_worker_fixture(
        &workspace,
        "auth-claude",
        "tokledger",
        "claude:sonnet:med",
        Some(("claude", Some("sonnet"), Some("medium"))),
        json!({
            "strategy": "claude_session",
            "session_id": "00000000-0000-4000-8000-000000000052",
            "cwd": workspace.display().to_string(),
            "launched_at": "2026-07-06T00:00:00Z",
            "niles_prompt_count": 1
        }),
    );
    write_usage_worker_fixture(
        &workspace,
        "auth-pending",
        "pending-task",
        "codex",
        Some(("codex", None, None)),
        json!({
            "strategy": "codex_cwd_time",
            "cwd": pending_workspace.display().to_string(),
            "launched_at": "2026-07-06T00:00:00Z",
            "niles_prompt_count": 1
        }),
    );

    let output = Command::new(niles)
        .args(["workers", "--usage"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env("CODEX_HOME", &codex_home)
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .output()
        .unwrap();

    assert_command_success("workers --usage", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "workers[3]{id,agent,task,age,wall,turns,total,input,cache_create,cache_read,cached,output,reasoning,usage}:"
    ));
    assert!(stdout.contains(
        "task_usage[2]{task,workers,total,input,cache_create,cache_read,cached,output,reasoning,wall}:"
    ));

    let codex = stdout
        .lines()
        .find(|line| line.starts_with("  auth-codex,"))
        .unwrap();
    assert!(codex.ends_with(",1,18,10,-,-,3,5,2,available"));

    let claude = stdout
        .lines()
        .find(|line| line.starts_with("  auth-claude,"))
        .unwrap();
    assert!(claude.ends_with(",1,30,7,8,9,-,6,-,available"));

    let pending = stdout
        .lines()
        .find(|line| line.starts_with("  auth-pending,"))
        .unwrap();
    assert!(pending.ends_with(",-,-,-,-,-,-,-,-,pending"));

    let rollup = stdout
        .lines()
        .find(|line| line.starts_with("  tokledger,"))
        .unwrap();
    assert!(rollup.starts_with("  tokledger,2,48,17,8,9,3,11,2,"));
}

#[test]
fn workers_reports_unknown_when_tmux_window_query_fails() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-list-unknown");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture(&workspace, "auth-fix", "working: checking window\n");

    let output = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_LIST_WINDOWS_FAIL", "1")
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unknown:tmux list-windows failed for session niles"));
    assert!(stdout.contains("server unreachable retry later"));
    assert!(!stdout.lines().any(|line| line.starts_with("retry later")));
    assert!(!stdout.contains("window-dead"));
}

#[test]
fn worker_close_by_task_closes_matching_workers_only() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-task");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_task(&workspace, "auth-one", "working: one", Some("auth"));
    write_worker_fixture_with_task(&workspace, "auth-two", "working: two", Some("auth"));
    write_worker_fixture_with_task(&workspace, "docs-one", "working: docs", Some("docs"));

    let close = Command::new(niles)
        .args(["worker-close", "--task", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert_command_success("worker-close --task", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("auth-one,closed,"));
    assert!(stdout.contains("auth-two,closed,"));
    assert!(!stdout.contains("docs-one,closed,"));

    assert!(!workspace.join(".niles/worker/auth-one").exists());
    assert!(!workspace.join(".niles/worker/auth-two").exists());
    assert!(workspace.join(".niles/worker/docs-one").exists());

    let archive_one = latest_archive_dir(&workspace, "auth-one");
    let archive_two = latest_archive_dir(&workspace, "auth-two");
    assert!(
        fs::read_to_string(archive_one.join("status.log"))
            .unwrap()
            .contains("closed: auth-one")
    );
    assert!(
        fs::read_to_string(archive_two.join("status.log"))
            .unwrap()
            .contains("closed: auth-two")
    );
}

#[test]
fn worker_close_all_is_scoped_to_invoking_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-close-scope");
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    let home = niles_home(&root);
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    for (workspace, id, label) in [
        (&workspace_a, "alpha", "task-a"),
        (&workspace_b, "bravo", "task-b"),
    ] {
        let spawn = Command::new(niles)
            .args([
                "spawn",
                id,
                "--task",
                label,
                "--project",
                ".",
                "--agent",
                "claude",
                "Fix",
            ])
            .current_dir(workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success("scoped close spawn", &spawn);
    }

    let close = Command::new(niles)
        .args(["worker-close", "--all"])
        .current_dir(&workspace_a)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("workspace-scoped worker-close --all", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[1]{id,status,archive}:"));
    assert!(stdout.contains("alpha,closed,"));
    assert!(!stdout.contains("bravo"));

    assert!(!workspace_a.join(".niles/worker/alpha").exists());
    assert!(workspace_b.join(".niles/worker/bravo").exists());
    assert!(workspace_b.join(".niles/worker/bravo/meta.json").exists());

    let close_foreign_task = Command::new(niles)
        .args(["worker-close", "--task", "task-b"])
        .current_dir(&workspace_a)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!close_foreign_task.status.success());
    assert!(
        String::from_utf8_lossy(&close_foreign_task.stderr)
            .contains("no live workers with task label task-b")
    );
    assert!(workspace_b.join(".niles/worker/bravo").exists());
}

#[test]
fn worker_close_zero_match_behaviors_are_distinct() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-zero");

    let close_all = Command::new(niles)
        .args(["worker-close", "--all"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("empty worker-close --all", &close_all);
    assert_eq!(
        String::from_utf8_lossy(&close_all.stdout),
        "no live workers\n"
    );

    write_worker_fixture_with_task(&workspace, "docs-one", "working: docs", Some("docs"));

    let close_task = Command::new(niles)
        .args(["worker-close", "--task", "missing"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(!close_task.status.success());
    assert!(String::from_utf8_lossy(&close_task.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&close_task.stderr)
            .contains("no live workers with task label missing")
    );
}

#[test]
fn worker_close_by_task_reports_selection_failures_and_closes_matches() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-task-selection-failure");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_corrupt_worker_fixture(&workspace, "bad-meta");
    write_worker_fixture_with_task(&workspace, "good-worker", "working: close me", Some("auth"));

    let close = Command::new(niles)
        .args(["worker-close", "--task", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert!(!close.status.success());
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("bad-meta,failed,-"));
    assert!(stdout.contains("good-worker,closed,"));
    let stderr = String::from_utf8_lossy(&close.stderr);
    assert!(stderr.contains("worker bad-meta close failed"));
    assert!(stderr.contains("worker-close --task auth failed for 1 worker(s): bad-meta"));

    assert!(workspace.join(".niles/worker/bad-meta").exists());
    assert!(!workspace.join(".niles/worker/good-worker").exists());
    assert!(latest_archive_dir(&workspace, "good-worker").exists());
}

#[test]
fn worker_close_all_reports_partial_failures_without_aborting_rest() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-close-all-partial");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_corrupt_worker_fixture(&workspace, "bad-meta");
    write_worker_fixture(&workspace, "good-worker", "working: close me");

    let close = Command::new(niles)
        .args(["worker-close", "--all"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert!(!close.status.success());
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("bad-meta,failed,-"));
    assert!(stdout.contains("good-worker,closed,"));
    let stderr = String::from_utf8_lossy(&close.stderr);
    assert!(stderr.contains("worker bad-meta close failed"));
    assert!(stderr.contains("worker-close --all failed for 1 worker(s): bad-meta"));

    assert!(workspace.join(".niles/worker/bad-meta").exists());
    assert!(!workspace.join(".niles/worker/good-worker").exists());
    assert!(latest_archive_dir(&workspace, "good-worker").exists());
}

#[test]
fn respawn_after_successful_close_from_same_cwd_gets_fresh_worker_dir() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-respawn-same-cwd");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    let first = Command::new(niles)
        .args([
            "spawn",
            "reviewer",
            "--project",
            ".",
            "--agent",
            "claude",
            "FIRST",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("first spawn", &first);

    let worker_dir = workspace.join(".niles/worker/reviewer");
    fs::write(worker_dir.join("report.md"), "first report\n").unwrap();
    fs::write(worker_dir.join("status.log"), "done: first\n").unwrap();

    let close = Command::new(niles)
        .args(["worker-close", "reviewer"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "first pane")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("close first worker", &close);
    assert!(!worker_dir.exists());

    let archive_dir = latest_archive_dir(&workspace, "reviewer");
    assert_eq!(
        fs::read_to_string(archive_dir.join("report.md")).unwrap(),
        "first report\n"
    );

    let second = Command::new(niles)
        .args([
            "spawn",
            "reviewer",
            "--project",
            ".",
            "--agent",
            "claude",
            "SECOND",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("respawn after close", &second);
    assert_eq!(
        fs::read_to_string(worker_dir.join("status.log")).unwrap(),
        ""
    );
    assert!(!worker_dir.join("report.md").exists());
    assert!(!worker_dir.join("final-pane.txt").exists());
    assert!(
        fs::read_to_string(worker_dir.join("brief.md"))
            .unwrap()
            .contains("SECOND")
    );
}

#[test]
fn report_falls_back_to_most_recent_local_archive() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-report-archive");
    let home = niles_home(&root);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    for (task, report_body) in [("FIRST", "first report\n"), ("SECOND", "second report\n")] {
        let spawn = Command::new(niles)
            .args([
                "spawn",
                "reviewer",
                "--project",
                ".",
                "--agent",
                "claude",
                task,
            ])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success("spawn archived report worker", &spawn);

        let worker_dir = workspace.join(".niles/worker/reviewer");
        fs::write(worker_dir.join("report.md"), report_body).unwrap();

        let close = Command::new(niles)
            .args(["worker-close", "reviewer"])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env("TMUX_CAPTURE", task)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_command_success("close archived report worker", &close);
    }

    let report = Command::new(niles)
        .args(["report", "reviewer"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("local archived report", &report);
    assert_eq!(String::from_utf8_lossy(&report.stdout), "second report\n");
    let stderr = String::from_utf8_lossy(&report.stderr);
    assert!(stderr.contains("serving archived report from"));
    assert!(stderr.contains(".niles/worker/archive/reviewer-"));
}

#[test]
fn report_skips_prefix_sibling_worker_archive() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-prefix-archive");
    let home = niles_home(&workspace);
    let archive_root = workspace.join(".niles/worker/archive");
    let worker_archive = archive_root.join("a-20260705T120000000000Z");
    let sibling_archive = archive_root.join("a-fs-20260705T130000000000Z");
    fs::create_dir_all(&worker_archive).unwrap();
    fs::create_dir_all(&sibling_archive).unwrap();
    fs::write(worker_archive.join("report.md"), "short worker report\n").unwrap();
    fs::write(sibling_archive.join("report.md"), "sibling worker report\n").unwrap();

    let report = Command::new(niles)
        .args(["report", "a"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();

    assert_command_success("prefix sibling archived report", &report);
    assert_eq!(
        String::from_utf8_lossy(&report.stdout),
        "short worker report\n"
    );
}

#[test]
fn worker_close_on_archived_worker_errors_and_mentions_archive() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-double-close");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture(&workspace, "auth-fix", "working: close requested");
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("first close", &close);

    let second = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert!(!second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("no live worker 'auth-fix'"));
    assert!(stderr.contains(".niles/worker/archive/auth-fix-"));
}

#[test]
fn worker_close_does_not_write_or_advertise_empty_final_pane() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-empty-pane-close");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture(&workspace, "auth-fix", "working: close requested");
    let close = Command::new(niles)
        .args(["worker-close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE_EMPTY", "1")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("close empty pane worker", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(!stdout.contains("pane:"));
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert!(!archive_dir.join("final-pane.txt").exists());
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
        String::from_utf8_lossy(&close.stderr).contains("no live worker 'missing'"),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
}

fn write_worker_fixture(workspace: &Path, id: &str, status_body: &str) -> PathBuf {
    write_worker_fixture_with_task(workspace, id, status_body, None)
}

fn write_worker_fixture_with_window(
    workspace: &Path,
    id: &str,
    status_body: &str,
    window: &str,
) -> PathBuf {
    write_worker_fixture_with_task_and_window(workspace, id, status_body, None, window)
}

fn write_usage_worker_fixture(
    workspace: &Path,
    id: &str,
    task_label: &str,
    agent: &str,
    agent_tier: Option<(&str, Option<&str>, Option<&str>)>,
    usage_attribution: serde_json::Value,
) -> PathBuf {
    let worker_root = workspace.join(".niles/worker");
    let worker_dir = worker_root.join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, "working: measuring usage\n").unwrap();

    let mut meta = json!({
        "niles_schema": 2,
        "id": id,
        "agent": agent,
        "usage_attribution": usage_attribution,
        "task_label": task_label,
        "created_at": "2026-07-06T00:00:00Z",
        "project": workspace.display().to_string(),
        "window": format!("niles:niles-{id}"),
        "brief": brief.display().to_string(),
        "launch": launch.display().to_string(),
        "status": status.display().to_string()
    });
    if let Some((family, model, effort)) = agent_tier {
        let object = meta.as_object_mut().unwrap();
        object.insert("agent_family".to_owned(), json!(family));
        if let Some(model) = model {
            object.insert("model".to_owned(), json!(model));
        }
        if let Some(effort) = effort {
            object.insert("effort".to_owned(), json!(effort));
        }
    }
    fs::write(
        worker_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
    worker_dir
}

fn write_codex_usage_rollout(
    codex_home: &Path,
    workspace: &Path,
    session_id: &str,
    timestamp: &str,
    tokens: (u64, u64, u64, u64, u64),
) {
    let sessions = codex_home.join("sessions/2026/07/06");
    fs::create_dir_all(&sessions).unwrap();
    let (input, cached, output, reasoning, total) = tokens;
    let body = [
        serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {
                "session_id": session_id,
                "cwd": workspace.display().to_string(),
                "timestamp": timestamp
            }
        }))
        .unwrap(),
        serde_json::to_string(&json!({
            "type": "event_msg",
            "payload": {
                "type": "user_message"
            }
        }))
        .unwrap(),
        serde_json::to_string(&json!({
            "type": "event_msg",
            "payload": {
                "type": "agent_message"
            }
        }))
        .unwrap(),
        serde_json::to_string(&json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "output_tokens": output,
                        "reasoning_output_tokens": reasoning,
                        "total_tokens": total
                    }
                }
            }
        }))
        .unwrap(),
    ]
    .join("\n");
    fs::write(
        sessions.join(format!("rollout-{session_id}.jsonl")),
        format!("{body}\n"),
    )
    .unwrap();
}

fn write_claude_usage_transcript(
    claude_home: &Path,
    workspace: &Path,
    session_id: &str,
    tokens: (u64, u64, u64, u64),
) {
    let project_dir = claude_home
        .join("projects")
        .join(claude_project_slug(workspace));
    fs::create_dir_all(&project_dir).unwrap();
    let (input, cache_create, cache_read, output) = tokens;
    let body = [
        serde_json::to_string(&json!({
            "type": "user",
            "sessionId": session_id,
            "message": {
                "role": "user"
            }
        }))
        .unwrap(),
        serde_json::to_string(&json!({
            "type": "assistant",
            "sessionId": session_id,
            "uuid": "line-1",
            "message": {
                "id": "msg-1",
                "role": "assistant",
                "usage": {
                    "input_tokens": input,
                    "cache_creation_input_tokens": cache_create,
                    "cache_read_input_tokens": cache_read,
                    "output_tokens": output
                }
            }
        }))
        .unwrap(),
    ]
    .join("\n");
    fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        format!("{body}\n"),
    )
    .unwrap();
}

fn claude_project_slug(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|ch| if matches!(ch, '/' | '\\') { '-' } else { ch })
        .collect()
}

fn write_worker_fixture_with_task(
    workspace: &Path,
    id: &str,
    status_body: &str,
    task_label: Option<&str>,
) -> PathBuf {
    write_worker_fixture_with_task_and_window(
        workspace,
        id,
        status_body,
        task_label,
        &format!("niles:niles-{id}"),
    )
}

fn write_worker_fixture_with_task_and_window(
    workspace: &Path,
    id: &str,
    status_body: &str,
    task_label: Option<&str>,
    window: &str,
) -> PathBuf {
    let worker_root = workspace.join(".niles/worker");
    let worker_dir = worker_root.join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, status_body).unwrap();
    let task_label_field = task_label
        .map(|label| format!(",\n  \"task_label\": \"{label}\""))
        .unwrap_or_default();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "agent": "codex",
  "project": "{}",
  "window": "{window}",
  "brief": "{}",
  "launch": "{}",
  "status": "{}"{task_label_field}
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

fn write_corrupt_worker_fixture(workspace: &Path, id: &str) -> PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: bad metadata\n").unwrap();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "id": "{id}",
  "agent": "codex"
}}
"#
        ),
    )
    .unwrap();
    worker_dir
}

fn write_orphan_recovery_tmux(bin: &Path, missing_session: &str) {
    write_executable(
        &bin.join("tmux"),
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  list-windows)
    if [ "$2" = "-a" ]; then
      if [ -n "${{TMUX_TAGGED_WINDOWS:-}}" ]; then
        printf '%s\n' "$TMUX_TAGGED_WINDOWS"
      fi
      exit 0
    fi
    if [ "$3" = "{missing_session}" ]; then
      printf "can't find session: {missing_session}\n" >&2
      exit 1
    fi
    exit 0
    ;;
  capture-pane) printf 'final pane\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#
        ),
    );
}

fn write_worker_test_bins(root: &Path) -> (PathBuf, PathBuf) {
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
    if [ "${TMUX_LIST_WINDOWS_FAIL:-}" = 1 ]; then
      printf 'server unreachable\nretry later\n' >&2
      exit 1
    fi
    if [ "$2" = "-a" ]; then
      if [ -n "${TMUX_TAGGED_WINDOWS:-}" ]; then
        printf '%s\n' "$TMUX_TAGGED_WINDOWS"
      fi
      exit 0
    fi
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
  capture-pane)
    if [ "${TMUX_CAPTURE_EMPTY:-}" = 1 ]; then
      exit 0
    fi
    printf '%s\n' "${TMUX_CAPTURE:-pane output}"
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

fn latest_archive_dir(workspace: &Path, id: &str) -> PathBuf {
    let archive_root = workspace.join(".niles/worker/archive");
    let prefix = format!("{id}-");
    let mut archives = fs::read_dir(&archive_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect::<Vec<_>>();
    archives.sort();
    archives.pop().expect("expected worker archive")
}

fn assert_archived_with_closed_sentinel(workspace: &Path, id: &str) {
    assert!(!workspace.join(".niles/worker").join(id).exists());
    let archive_dir = latest_archive_dir(workspace, id);
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains(&format!("closed: {id}"))
    );
}

fn assert_global_index_absent(home: &Path) {
    let path = home.join("runs/index.json");
    assert!(!path.exists(), "global index should not exist: {path:?}");
}
