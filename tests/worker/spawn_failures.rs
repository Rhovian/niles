use super::support::*;

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
    assert!(stdout.contains("workers[0]{id,agent,task,age,window,wake,last_status}:"));
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
  display-message) printf 'niles-test-session\n'; exit 0 ;;
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
exit 0
"#,
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    );

    let failed = Command::new(niles)
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_FAIL_NEW_WINDOW", "1")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("respawn", &respawn);
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(workspace.join(".niles/worker/auth-fix").is_dir());
    assert!(workspace.join(".niles/worker/auth-fix/meta.json").is_file());

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("new-window -d -t =niles-test-session:"));
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
  display-message) printf 'niles-test-session\n'; exit 0 ;;
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
exit 0
"#,
    );

    let failed = Command::new(niles)
        .args(["spawn", "auth-fix", "--agent", "claude", "Fix", "auth"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env(
            "META_PATH",
            workspace.join(".niles/worker/auth-fix/meta.json"),
        )
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
    assert!(log.contains("new-window -d -t =niles-test-session:"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains("set-option -w -t "));
    assert!(log.contains(" @niles-worker-id auth-fix"));
    assert!(log.contains("kill-window -t "));
    assert!(log.contains(":=niles-auth-fix"));
}
