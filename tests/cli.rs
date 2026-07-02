use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn prepare_run(niles: &str, workspace: &Path, task: &Path) -> Output {
    let output = Command::new(niles)
        .arg("run")
        .arg(task)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert_command_success("run", &output);
    output
}

fn exec_step_output(niles: &str, workspace: &Path, index: usize) -> Output {
    Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg(index.to_string())
        .current_dir(workspace)
        .output()
        .unwrap()
}

fn drive_exec_steps(
    niles: &str,
    workspace: &Path,
    steps: impl IntoIterator<Item = usize>,
) -> Vec<Output> {
    let mut outputs = Vec::new();
    for index in steps {
        let output = exec_step_output(niles, workspace, index);
        assert_command_success(&format!("exec-step {index}"), &output);
        outputs.push(output);
    }
    outputs
}

fn assert_command_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} stdout:\n{}\n{label} stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn write_workspace_manifest(
    workspace: &Path,
    manager: &str,
    planner: &str,
    implementer: &str,
    reviewer: &str,
    validation_command: &str,
) {
    fs::create_dir_all(workspace.join(".niles")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        format!(
            "manager: {manager}\nplanner: {planner}\nimplementer: {implementer}\nreviewer: {reviewer}\nvalidation_command: {validation_command}\n"
        ),
    )
    .unwrap();
}

#[test]
fn run_executes_steps_and_persists_state() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Exercise test runner"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "hello test"
  - command: pwd
commands:
  pwd: pwd
"#,
    )
    .unwrap();

    let output = prepare_run(niles, &workspace, &task);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: created"));
    assert!(stdout.contains("next: niles step "));
    assert!(stdout.contains("exec-step: niles exec-step "));
    assert!(!stdout.contains("hello test"));

    let steps = drive_exec_steps(niles, &workspace, 1..=2);
    assert!(String::from_utf8_lossy(&steps[0].stdout).contains("hello test"));
    assert!(String::from_utf8_lossy(&steps[1].stdout).contains("status: completed"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("steps[2]{index,kind,label,status,exit}:"));
    assert!(status_stdout.contains("1,agent,echo,completed,0"));
    assert!(status_stdout.contains("help[4]:"));
    assert!(status_stdout.contains("Run `niles status "));
    assert!(status_stdout.contains("--json`"));

    let status_json = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status_json.status.success());

    let status_json_stdout = String::from_utf8_lossy(&status_json.stdout);
    assert!(status_json_stdout.contains("\"status\": \"completed\""));
    assert!(status_json_stdout.contains("001-echo.stdout.txt"));

    let show = Command::new(niles)
        .arg("show")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(show.status.success());

    let show_stdout = String::from_utf8_lossy(&show.stdout);
    assert!(show_stdout.contains("status: completed"));
    assert!(show_stdout.contains("1. agent echo completed"));
    assert!(show_stdout.contains("2. command pwd completed"));

    let log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(log.status.success());
    let log_stdout = String::from_utf8_lossy(&log.stdout);
    assert!(log_stdout.contains("hello test"));
    assert!(log_stdout.contains("Niles handoff context: "));

    let alias = Command::new(niles)
        .arg("l")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(alias.status.success());
    assert!(String::from_utf8_lossy(&alias.stdout).contains("niles-test-"));
}

#[test]
fn run_captures_git_diff_after_each_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-diff-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    assert!(
        Command::new("git")
            .arg("init")
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(workspace.join("tracked.txt"), "before\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Niles Test",
                "-c",
                "user.email=niles@example.invalid",
                "commit",
                "-m",
                "initial",
            ])
            .current_dir(&workspace)
            .output()
            .unwrap()
            .status
            .success()
    );

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Capture diff"
steps:
  - command: edit
commands:
  edit: printf 'after\n' > tracked.txt
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);
    drive_exec_steps(niles, &workspace, [1]);

    let status = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("001-edit.diff"));

    let diff = Command::new(niles)
        .arg("diff")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(diff.status.success());

    let diff_stdout = String::from_utf8_lossy(&diff.stdout);
    assert!(diff_stdout.contains("-before"));
    assert!(diff_stdout.contains("+after"));
}

#[test]
fn run_uses_project_config_defaults() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-config-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    fs::write(
        workspace.join("niles.yaml"),
        r#"
agents:
  echo:
    binary: /bin/echo
commands:
  marker: printf 'from config\n'
"#,
    )
    .unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Use project config"
steps:
  - agent: echo
    task: "configured agent"
  - command: marker
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);
    drive_exec_steps(niles, &workspace, 1..=2);

    let first_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(first_log.status.success());
    let first_log_stdout = String::from_utf8_lossy(&first_log.stdout);
    assert!(first_log_stdout.contains("configured agent"));
    assert!(first_log_stdout.contains("Niles handoff context: "));

    let second_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(second_log.status.success());
    assert_eq!(String::from_utf8_lossy(&second_log.stdout), "from config\n");
}

#[test]
fn run_enforces_known_agent_cli_min_version_and_allows_override() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-version-run-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.1.0\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Gate codex"
steps:
  - agent: codex
    task: "hello"
"#,
    )
    .unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let blocked = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    let blocked_stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(blocked_stderr.contains("codex CLI 0.1.0 is below the supported minimum"));
    assert!(blocked_stderr.contains("--allow-cli-mismatch"));

    let allowed = Command::new(niles)
        .args(["run", "--allow-cli-mismatch"])
        .arg(&task)
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("run --allow-cli-mismatch", &allowed);
    assert!(String::from_utf8_lossy(&allowed.stdout).contains("status: created"));
    assert!(String::from_utf8_lossy(&allowed.stderr).contains("CLI mismatch override is enabled"));
}

#[test]
fn spawn_enforces_known_agent_cli_min_version() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-version-spawn-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
fn analyze_reports_version_gate_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-version-analyze-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
case "$1" in
  --version) printf 'codex-cli 0.1.0\n'; exit 0 ;;
  --help) printf 'codex help\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let analyze = Command::new(niles)
        .args(["analyze", "--agent", "codex"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_command_success("analyze", &analyze);

    let stdout = String::from_utf8_lossy(&analyze.stdout);
    assert!(stdout.contains("version_gate: codex fail 0.1.0"));
    assert!(stdout.contains("wrote .niles/capabilities/codex.json"));

    let manifest = fs::read_to_string(workspace.join(".niles/capabilities/codex.json")).unwrap();
    assert!(manifest.contains(r#""version_gate""#));
    assert!(manifest.contains(r#""status": "fail""#));
}

#[test]
fn bare_niles_errors_when_stdin_is_not_interactive() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-session-noninteractive-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();
    write_workspace_manifest(&workspace, "claude", "claude", "codex", "claude", "test");

    let output = Command::new(niles)
        .args(["--goal", "Fix the startup flow"])
        .current_dir(&workspace)
        .env("TMUX", "/tmp/niles-test-tmux")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(workspace.join(".niles/worker").is_dir());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stdin is not interactive"));
    assert!(stderr.contains("choose the manager agent"));
}

#[test]
fn auth_spawn_peek_and_send_use_tmux_worker_metadata() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-worker-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
    assert!(log.contains("new-window -d -t niles -n niles-auth-fix"));
    assert!(log.contains("capture-pane -p -t niles:niles-auth-fix -S -7"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix -l continue please"));
    assert!(log.contains("send-keys -t niles:niles-auth-fix C-m"));
}

#[test]
fn ask_spawns_one_off_worker_window() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-ask-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
    assert!(log.contains(&format!("new-window -d -t niles -n niles-{id}")));
    assert!(!workspace.join(".niles/runs").exists());
}

#[test]
fn worker_close_tears_down_worker() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-worker-close-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
    let workspace = std::env::temp_dir().join(format!(
        "niles-worker-close-wait-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
    let workspace = std::env::temp_dir().join(format!(
        "niles-worker-close-missing-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

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
    let workspace = std::env::temp_dir().join(format!(
        "niles-step-window-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Prepare an interactive step"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "needs window"
"#,
    )
    .unwrap();

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
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

#[test]
fn wait_ignores_existing_status_lines_and_prints_next_wake() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-wait-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    let status_log = run_dir.join("status.log");
    fs::write(
        &status_log,
        "done: old baseline wake\nworking: already running\n",
    )
    .unwrap();

    let child = Command::new(niles)
        .args(["wait", "test-run", "--interval", "0.05", "--timeout", "5"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let writer_done = Arc::new(AtomicBool::new(false));
    let writer_status = status_log.clone();
    let writer_done_clone = Arc::clone(&writer_done);
    let writer = thread::spawn(move || {
        while !writer_done_clone.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
            let mut status = fs::OpenOptions::new()
                .append(true)
                .open(&writer_status)
                .unwrap();
            writeln!(status, "done: new appended wake").unwrap();
        }
    });

    let output = child.wait_with_output().unwrap();
    writer_done.store(true, Ordering::SeqCst);
    writer.join().unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: new appended wake\n"
    );
}

#[test]
fn wait_index_returns_already_emitted_step_wake() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-wait-index-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let run_dir = workspace.join(".niles/runs/test-run");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("status.log"),
        "done: step 1 finished\nworking: step 2 running\ndone: step 2 finished\n",
    )
    .unwrap();

    let started = Instant::now();
    let output = Command::new(niles)
        .args([
            "wait",
            "test-run",
            "--index",
            "2",
            "--interval",
            "0.05",
            "--timeout",
            "4",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_command_success("wait --index", &output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "done: step 2 finished\n"
    );
}

#[test]
fn agent_steps_receive_context_artifacts() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-context-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    fs::write(
        workspace.join("agent.sh"),
        r#"#!/bin/sh
printf 'PROMPT\n%s\n' "$1"
context=$(printf '%s\n' "$1" | sed -n 's/^Niles handoff context: //p' | head -n 1)
if [ -n "$context" ]; then
  printf 'CONTEXT\n'
  cat "$context"
fi
"#,
    )
    .unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Use context handoffs"
agents:
  inspector:
    binary: sh
    args: ["agent.sh"]
steps:
  - agent: inspector
    role: planner
    task: "planner says inspect auth flow"
  - agent: inspector
    role: implementer
    task: "implement from planner output"
  - command: validate
    role: validation
  - agent: inspector
    role: reviewer
    task: "review with validation output"
commands:
  validate: printf 'validation ok\n'
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);
    let steps = drive_exec_steps(niles, &workspace, 1..=4);
    let run_stdout = steps
        .iter()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(run_stdout.contains("context: .niles/runs/"));

    let implementer_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(implementer_log.status.success());
    let implementer_stdout = String::from_utf8_lossy(&implementer_log.stdout);
    assert!(implementer_stdout.contains("Niles handoff context: "));
    assert!(implementer_stdout.contains("# Niles Step Context"));
    assert!(implementer_stdout.contains("role: implementer"));
    assert!(implementer_stdout.contains("## Prior Agent Output"));
    assert!(implementer_stdout.contains("planner says inspect auth flow"));

    let reviewer_log = Command::new(niles)
        .arg("log")
        .arg("--step")
        .arg("4")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(reviewer_log.status.success());
    let reviewer_stdout = String::from_utf8_lossy(&reviewer_log.stdout);
    assert!(reviewer_stdout.contains("role: reviewer"));
    assert!(reviewer_stdout.contains("## Validation Output"));
    assert!(reviewer_stdout.contains("validation ok"));
    assert!(reviewer_stdout.contains("## Latest Diff"));

    let show = Command::new(niles)
        .arg("show")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("context .niles/runs/"));

    let status_json = Command::new(niles)
        .arg("status")
        .arg("--json")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status_json.status.success());
    assert!(String::from_utf8_lossy(&status_json.stdout).contains("\"context\": \".niles/runs/"));
}

#[test]
fn run_prints_actionable_failure_summary() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-failure-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Fail usefully"
steps:
  - command: fail
commands:
  fail:
    run: "for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo tail-line-$i >&2; done; exit 7"
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);
    let output = exec_step_output(niles, &workspace, 1);

    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status: failed"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failure:"));
    assert!(stderr.contains("step: 1 command fail"));
    assert!(stderr.contains("exit: 7"));
    assert!(stderr.contains("stderr: .niles/runs/"));
    assert!(stderr.contains("diff: .niles/runs/"));
    assert!(stderr.contains("stderr tail:"));
    assert!(stderr.contains("  tail-line-2"));
    assert!(stderr.contains("  tail-line-13"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());

    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: failed"));
    assert!(status_stdout.contains("1,command,fail,failed,7"));
    assert!(status_stdout.contains("--stderr`"));
}

#[test]
fn resume_continues_from_first_incomplete_step() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-resume-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Resume failed run"
steps:
  - command: first
  - command: gate
  - command: last
commands:
  first: printf 'first\n' >> trace.txt
  gate: test -f allow
  last: printf 'last\n' >> trace.txt
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);
    let first = exec_step_output(niles, &workspace, 1);
    assert_command_success("exec-step 1", &first);

    let failed = exec_step_output(niles, &workspace, 2);
    assert!(
        !failed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(String::from_utf8_lossy(&failed.stdout).contains("status: failed"));

    fs::write(workspace.join("allow"), "").unwrap();

    let resumed = Command::new(niles)
        .arg("resume")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );

    let resumed_stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(resumed_stdout.contains("resume: "));
    assert!(resumed_stdout.contains("from_step: 2"));
    assert!(resumed_stdout.contains("next: niles exec-step "));
    assert!(resumed_stdout.contains("exec-step: niles exec-step "));
    assert!(resumed_stdout.contains("wait: niles wait "));

    drive_exec_steps(niles, &workspace, 2..=3);

    assert_eq!(
        fs::read_to_string(workspace.join("trace.txt")).unwrap(),
        "first\nlast\n"
    );

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("1,command,first,completed,0"));
    assert!(status_stdout.contains("2,command,gate,completed,0"));
    assert!(status_stdout.contains("3,command,last,completed,0"));
}

#[test]
fn manifest_command_is_removed() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-manifest-removed-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let output = Command::new(niles)
        .args(["manifest", "Ship", "workflow"])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand"));
    assert!(stderr.contains("manifest"));
}

#[test]
fn status_shows_running_and_pending_steps() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-running-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Show running state"
steps:
  - command: slow
    role: validation
  - command: fast
    role: validation
commands:
  slow: sleep 1
  fast: printf 'done\n'
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);

    let child = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut saw_running = false;
    for _ in 0..20 {
        let status = Command::new(niles)
            .arg("status")
            .current_dir(&workspace)
            .output();

        if let Ok(status) = status
            && status.status.success()
        {
            let stdout = String::from_utf8_lossy(&status.stdout);
            if stdout.contains("1,validation,command,slow,running,-")
                && stdout.contains("2,validation,command,fast,pending,-")
            {
                saw_running = true;
                break;
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(saw_running, "status never showed running/pending steps");
}

#[test]
fn watch_streams_run_state_until_completion() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-watch-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Watch running state"
steps:
  - command: slow
    role: validation
  - command: fast
    role: validation
commands:
  slow: sleep 3
  fast: printf 'done\n'
"#,
    )
    .unwrap();

    prepare_run(niles, &workspace, &task);

    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut run_started = false;
    for _ in 0..30 {
        let status = Command::new(niles)
            .arg("status")
            .current_dir(&workspace)
            .output();

        if let Ok(status) = status
            && status.status.success()
        {
            let stdout = String::from_utf8_lossy(&status.stdout);
            if stdout.contains("1,validation,command,slow,running,-")
                && stdout.contains("2,validation,command,fast,pending,-")
            {
                run_started = true;
                break;
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
    assert!(run_started, "run never reached running state");

    let watch = Command::new(niles)
        .args(["watch", "--interval", "0.05", "--no-clear"])
        .current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let output = step1.wait_with_output().unwrap();
    assert_command_success("exec-step 1", &output);

    let step2 = exec_step_output(niles, &workspace, 2);
    assert_command_success("exec-step 2", &step2);

    let watch = watch.wait_with_output().unwrap();
    assert_command_success("watch", &watch);

    let stdout = String::from_utf8_lossy(&watch.stdout);
    assert!(stdout.contains("status: running"));
    assert!(stdout.contains("1,validation,command,slow,running,-"));
    assert!(stdout.contains("2,validation,command,fast,pending,-"));
    assert!(stdout.contains("status: completed"));
    assert!(stdout.contains("2,validation,command,fast,completed,0"));
}

#[test]
fn prepare_then_exec_step_drives_run() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-step-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Exercise manager stepping"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "stepped hello"
  - command: pwd
commands:
  pwd: pwd
"#,
    )
    .unwrap();

    // prepare: create the run without executing it.
    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        prepare.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&prepare.stdout),
        String::from_utf8_lossy(&prepare.stderr)
    );
    let prepare_stdout = String::from_utf8_lossy(&prepare.stdout);
    assert!(prepare_stdout.contains("status: created"));
    assert!(prepare_stdout.contains("1 agent echo"));
    assert!(prepare_stdout.contains("next: niles step "));
    // prepare must not execute the agent.
    assert!(!prepare_stdout.contains("stepped hello"));

    // status shows created with all steps pending.
    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: created"));
    assert!(status_stdout.contains("1,agent,echo,pending,-"));

    // exec-step 1: runs the agent, records state, appends a done wake.
    let step1 = Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg("1")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        step1.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&step1.stdout),
        String::from_utf8_lossy(&step1.stderr)
    );
    let step1_stdout = String::from_utf8_lossy(&step1.stdout);
    assert!(step1_stdout.contains("stepped hello"));
    assert!(step1_stdout.contains("step 1: completed"));
    // run is still running with step 2 pending; not yet complete.
    assert!(!step1_stdout.contains("status: completed"));

    // exec-step 2: runs the command and completes the run.
    let step2 = Command::new(niles)
        .arg("exec-step")
        .arg("latest")
        .arg("2")
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        step2.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&step2.stdout),
        String::from_utf8_lossy(&step2.stderr)
    );
    assert!(String::from_utf8_lossy(&step2.stdout).contains("status: completed"));

    // the run status log carries the wake lines `niles wait` reads.
    let runs_dir = workspace.join(".niles").join("runs");
    let run_dir = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .max()
        .unwrap();
    let status_log = fs::read_to_string(run_dir.join("status.log")).unwrap();
    assert!(status_log.contains("done: step 1 "));
    assert!(status_log.contains("done: step 2 "));

    // final status is completed with both steps recorded.
    let final_status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let final_stdout = String::from_utf8_lossy(&final_status.stdout);
    assert!(final_stdout.contains("status: completed"));
    assert!(final_stdout.contains("1,agent,echo,completed,0"));
    assert!(final_stdout.contains("2,command,pwd,completed,0"));
}

#[test]
fn step_guards_block_out_of_order_and_command_steps() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-step-guard-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Guard checks"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "first"
  - command: pwd
commands:
  pwd: pwd
"#,
    )
    .unwrap();

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(prepare.status.success());

    // Ordering guard: step 2 cannot launch while step 1 is still pending.
    let out_of_order = Command::new(niles)
        .args(["step", "--index", "2"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!out_of_order.status.success());
    assert!(
        String::from_utf8_lossy(&out_of_order.stderr).contains("prior steps must complete first")
    );

    // Complete step 1 (captured), then step 2 is a command -> directed to exec-step.
    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(step1.status.success());

    let command_step = Command::new(niles)
        .args(["step", "--index", "2"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!command_step.status.success());
    assert!(
        String::from_utf8_lossy(&command_step.stderr)
            .contains("run it captured with `niles exec-step")
    );
}

#[test]
fn step_close_marks_step_completed() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-step-close-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "Close checks"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "only step"
"#,
    )
    .unwrap();

    let prepare = Command::new(niles)
        .arg("run")
        .arg(&task)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(prepare.status.success());

    // step-close finalizes the step and reports completion. Window teardown is
    // best-effort (no live window in the test env) and must not fail the call.
    let close = Command::new(niles)
        .args(["step-close", "--index", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        close.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(close_stdout.contains("step 1: completed"));
    assert!(close_stdout.contains("status: completed"));

    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: completed"));
    assert!(status_stdout.contains("1,agent,echo,completed,0"));
}

#[test]
fn step_add_appends_to_run_and_reopens() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = std::env::temp_dir().join(format!(
        "niles-step-add-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let task = workspace.join("task.yaml");
    fs::write(
        &task,
        r#"
goal: "step-add"
agents:
  echo:
    binary: /bin/echo
steps:
  - agent: echo
    task: "first"
    role: implementer
commands:
  check:
    run: "true"
"#,
    )
    .unwrap();

    let run = String::from_utf8(
        Command::new(niles)
            .arg("run")
            .arg(&task)
            .current_dir(&workspace)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(run.contains("status: created"));

    // Complete the only step so the run reaches a terminal state.
    let step1 = Command::new(niles)
        .args(["exec-step", "latest", "1"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(step1.status.success());
    assert!(String::from_utf8_lossy(&step1.stdout).contains("status: completed"));

    // Append a review cycle: a reviewer agent step and a validation command step.
    let add_review = Command::new(niles)
        .args([
            "step-add",
            "latest",
            "--agent",
            "echo",
            "--role",
            "reviewer",
            "review it",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(
        add_review.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&add_review.stderr)
    );
    assert!(
        String::from_utf8_lossy(&add_review.stdout).contains("added: step 2 reviewer agent echo")
    );

    let add_check = Command::new(niles)
        .args([
            "step-add",
            "latest",
            "--command",
            "check",
            "--role",
            "validation",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(add_check.status.success());
    assert!(
        String::from_utf8_lossy(&add_check.stdout)
            .contains("added: step 3 validation command check")
    );

    // The run reopened to running with the two new pending steps.
    let status = Command::new(niles)
        .arg("status")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("status: running"));
    assert!(status_stdout.contains("1,implementer,agent,echo,completed,0"));
    assert!(status_stdout.contains("2,reviewer,agent,echo,pending,-"));
    assert!(status_stdout.contains("3,validation,command,check,pending,-"));

    // The appended steps are persisted to the task spec for step/exec-step.
    let task_body = fs::read_to_string(&task).unwrap();
    assert!(task_body.contains("role: reviewer"));
    assert!(task_body.contains("role: validation"));
}
