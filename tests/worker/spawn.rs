use super::support::*;

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
fn spawn_always_targets_the_invoking_workspace_and_rejects_a_project_flag() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-worker-invoking-workspace");
    let home = niles_home(&root);
    let workspace = root.join("workspace");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    // There is no longer a flag that can name a different workspace.
    let rejected = Command::new(niles)
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
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("unexpected argument '--project'"),
        "stderr:\n{}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    // The worker lands in the directory the command ran from, and nowhere else.
    let spawn = Command::new(niles)
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("workspace-local spawn", &spawn);
    assert!(workspace.join(".niles/worker/auth-fix").exists());
    assert!(!elsewhere.join(".niles").exists());
}

#[test]
fn spawn_outside_tmux_fails_with_guidance_instead_of_inventing_a_session() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-no-tmux");
    let home = niles_home(&workspace);

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
exit 0
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
exit 0
"#,
    );

    let spawn = Command::new(niles)
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env_remove("TMUX")
        .output()
        .unwrap();

    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("must run inside tmux"), "{stderr}");
    assert!(stderr.contains("tmux new -s niles"), "{stderr}");
    // Refusing is the point: no window, no session, no worker directory left behind.
    assert!(!workspace.join(".niles/worker/auth-fix").exists());
    // niles never invokes tmux here, so tmux.log is correctly absent — its absence is the proof
    // that no tmux command was issued. A file that *does* exist only matters if it shows a
    // session or window was created; any other read error is a real failure.
    let log = match fs::read_to_string(&tmux_log) {
        Ok(log) => log,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => panic!("tmux.log should be readable: {err}"),
    };
    assert!(!log.contains("new-session"), "{log}");
    assert!(!log.contains("new-window"), "{log}");
}

#[test]
fn spawn_places_the_worker_in_the_current_tmux_session() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-current-session");
    let home = niles_home(&workspace);

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'ambient\n'; exit 0 ;;
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
exit 0
"#,
    );

    let path = path_with_bin(&bin);
    let spawn = Command::new(niles)
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/ambient-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("current-session spawn", &spawn);

    // The session the operator is attached to is the session, so it is asked for by name
    // rather than resolved from a recorded pointer.
    let meta = fs::read_to_string(workspace.join(".niles/worker/auth-fix/meta.json")).unwrap();
    assert!(
        meta.contains(r#""window": "ambient:niles-auth-fix""#),
        "{meta}"
    );

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("display-message -p #S"), "{log}");
    assert!(
        log.contains("new-window -d -t =ambient: -n niles-auth-fix"),
        "{log}"
    );
    assert!(log.contains("set-option -w -t =ambient:=niles-auth-fix @niles-project"));
    assert!(log.contains("set-option -w -t =ambient:=niles-auth-fix @niles-worker-id auth-fix"));
    // No pointer file, no invented session.
    assert!(!workspace.join(".niles/sessions/tmux-session.json").exists());
    assert!(!log.contains("new-session"), "{log}");
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
fn spawn_maps_model_effort_specs_into_worker_launches_and_metadata() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-tier-test");
    let home = niles_home(&workspace);
    fs::write(
        workspace.join("niles.yaml"),
        "models: { codex: { gpt-5.7: { efforts: [xhigh] } } }\n",
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
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  has-session) exit 1 ;;
  list-windows) exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("codex"),
        r#"#!/bin/sh
exit 0
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
exit 0
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    );

    let codex_spawn = Command::new(niles)
        .args([
            "spawn",
            "codex-hi",
            "--agent",
            "codex:gpt-5.7:xhigh",
            "Fix",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("codex tiered spawn", &codex_spawn);
    let codex_stdout = String::from_utf8_lossy(&codex_spawn.stdout);
    assert!(codex_stdout.contains("agent: codex:gpt-5.7:xhigh"));
    assert!(codex_stdout.contains("agent_family: codex"));
    assert!(codex_stdout.contains("model: gpt-5.7"));
    assert!(codex_stdout.contains("effort: xhigh"));

    let codex_meta =
        fs::read_to_string(workspace.join(".niles/worker/codex-hi/meta.json")).unwrap();
    assert!(codex_meta.contains(r#""agent": "codex:gpt-5.7:xhigh""#));
    assert!(codex_meta.contains(r#""agent_family": "codex""#));
    assert!(codex_meta.contains(r#""model": "gpt-5.7""#));
    assert!(codex_meta.contains(r#""effort": "xhigh""#));

    let codex_launch =
        fs::read_to_string(workspace.join(".niles/worker/codex-hi/launch.sh")).unwrap();
    assert!(codex_launch.contains("'--dangerously-bypass-approvals-and-sandbox'"));
    assert!(codex_launch.contains("'--model' 'gpt-5.7'"));
    assert!(codex_launch.contains("'--config' 'model_reasoning_effort=\"xhigh\"'"));

    let claude_spawn = Command::new(niles)
        .args([
            "spawn",
            "claude-max",
            "--agent",
            "claude:opus:max",
            "Review",
            "auth",
        ])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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

    let off_roster = Command::new(niles)
        .args([
            "spawn",
            "future-worker",
            "--agent",
            "codex:gpt-5.7:xhigh",
            "Fix",
        ])
        .current_dir(&workspace)
        .output()
        .unwrap();

    assert!(!off_roster.status.success());
    assert!(
        String::from_utf8_lossy(&off_roster.stderr).contains("unsupported codex model `gpt-5.7`")
    );
    assert!(!workspace.join(".niles/worker/future-worker").exists());
}
