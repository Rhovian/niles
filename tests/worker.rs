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
    assert!(meta.contains("\"window\": \"niles:niles-auth-fix\""));
    assert!(meta.contains("\"created_at\":"));

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
    assert!(log.contains("new-session -d -s niles"));
    assert!(log.contains("new-window -d -t niles: -n niles-auth-fix"));
    assert!(log.contains("capture-pane -p -t niles:niles-auth-fix -S -7"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix -l continue please"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix C-m"));
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
    assert!(stderr.contains("model `gpt-bad` was rejected by codex CLI 0.142.4"));
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
  --help) printf 'claude help\n'; exit 0 ;;
esac
printf 'claude ok\n'
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
    assert!(stderr.contains("model `omega` was rejected by codex CLI 0.142.4"));
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
  --version) printf 'codex-cli 0.142.4\n'; exit 0 ;;
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
    assert!(!worker_dir.exists());
    let archive_dir = latest_archive_dir(&project, "auth-fix");
    assert_eq!(
        fs::read_to_string(archive_dir.join("final-pane.txt")).unwrap(),
        "pane output\n"
    );
    assert_global_index_lacks_live_worker(&home, "auth-fix");
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
    assert_global_index_lacks_live_worker(&home, "auth-fix");
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
    assert!(stdout.contains("window-unknown:tmux list-windows failed for session niles"));
    assert!(stdout.contains("server unreachable"));
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
fn respawn_after_cross_cwd_close_does_not_inherit_archived_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-respawn-cross-cwd");
    let home = niles_home(&root);
    let invoker = root.join("invoker");
    let project = root.join("project");
    let unrelated = root.join("unrelated");
    fs::create_dir_all(&invoker).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    let first = Command::new(niles)
        .arg("spawn")
        .arg("job1")
        .arg("--project")
        .arg(&project)
        .args(["--agent", "claude", "FIRST"])
        .current_dir(&invoker)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd first spawn", &first);

    let worker_dir = project.join(".niles/worker/job1");
    fs::write(worker_dir.join("report.md"), "first worker report\n").unwrap();
    fs::write(
        worker_dir.join("status.log"),
        "working: first\ndone: first result\n",
    )
    .unwrap();

    let close = Command::new(niles)
        .args(["worker-close", "job1"])
        .current_dir(&unrelated)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "first pane")
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd close", &close);
    assert!(!worker_dir.exists());

    let second = Command::new(niles)
        .arg("spawn")
        .arg("job1")
        .arg("--project")
        .arg(&project)
        .args(["--agent", "claude", "SECOND"])
        .current_dir(&invoker)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert_command_success("cross-cwd respawn", &second);
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
fn report_falls_back_to_most_recent_archive_from_unrelated_cwd() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-report-archive");
    let home = niles_home(&root);
    let workspace = root.join("workspace");
    let unrelated = root.join("unrelated");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
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
        .current_dir(&unrelated)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert_command_success("archived report from unrelated cwd", &report);
    assert_eq!(String::from_utf8_lossy(&report.stdout), "second report\n");
    let stderr = String::from_utf8_lossy(&report.stderr);
    assert!(stderr.contains("serving archived report from"));
    assert!(stderr.contains(".niles/worker/archive/reviewer-"));
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
    write_worker_fixture_with_task(workspace, id, status_body, None)
}

fn write_worker_fixture_with_task(
    workspace: &Path,
    id: &str,
    status_body: &str,
    task_label: Option<&str>,
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
  "window": "niles:niles-{id}",
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
      printf 'server unreachable\n' >&2
      exit 1
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
  --version) printf '2.1.197 (Claude Code)\n'; exit 0 ;;
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

fn assert_global_index_lacks(home: &Path, id: &str) {
    let path = home.join("runs/index.json");
    if path.exists() {
        let index = fs::read_to_string(&path).unwrap();
        assert!(!index.contains(id), "global index retained {id}:\n{index}");
    }
}

fn assert_global_index_lacks_live_worker(home: &Path, id: &str) {
    let path = home.join("runs/index.json");
    if !path.exists() {
        return;
    }
    let index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        index
            .get("workers")
            .and_then(|workers| workers.get(id))
            .is_none(),
        "global index retained live worker {id}:\n{index:#}"
    );
}
