use super::support::*;

#[test]
fn missing_agent_binary_leaves_no_worker_state() {
    let env = TestEnv::with_manifest("niles-worker-missing-binary", "claude");
    fs::remove_file(env.bin.join("claude")).unwrap();
    let output = env
        .spawn_command("research", &["missing-cli", "Check", "setup"])
        .env("PATH", &env.bin)
        .output()
        .unwrap();

    assert_failure_contains(
        "missing agent binary",
        &output,
        "agent binary claude not found on PATH",
    );
    assert!(!state_dir(&env.root).join("worker/missing-cli").exists());
}

#[test]
fn leftover_worker_json_file_is_inert() {
    let workspace = temp_workspace("niles-worker-json-inert");
    let home = niles_home(&workspace);
    fs::create_dir_all(state_dir(&workspace).join("worker")).unwrap();
    fs::write(
        state_dir(&workspace).join("worker/auth-fix.json"),
        r#"{"id":"auth-fix"}"#,
    )
    .unwrap();

    let workers = niles_bare(&workspace, &home)
        .arg("workers")
        .output()
        .unwrap();
    assert_command_success("workers ignores leftover json", &workers);
    let stdout = stdout_of(&workers);
    assert_eq!(stdout, "{\"workers\":[]}\n");

    let peek = niles_bare(&workspace, &home)
        .args(["peek", "auth-fix"])
        .output()
        .unwrap();
    assert_failure_contains("peek leftover json", &peek, "unknown worker id 'auth-fix'");
}

#[test]
fn spawn_window_failure_cleans_partial_worker_and_allows_respawn() {
    let env = TestEnv::with_tmux(
        "niles-worker-failed-spawn",
        r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
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
    write_spawn_manifest(&env.root, "claude");

    let failed = env
        .spawn_command("research", &["auth-fix", "Fix", "auth"])
        .env("TMUX_FAIL_NEW_WINDOW", "1")
        .output()
        .unwrap();
    assert_failure_contains("failed spawn", &failed, "failed to launch worker auth-fix");
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("create window failed: index 1 in use"));
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());

    let peek = niles_bare(&env.root, &env.home)
        .args(["peek", "auth-fix"])
        .output()
        .unwrap();
    assert_failure_contains("peek failed spawn", &peek, "unknown worker id 'auth-fix'");

    let respawn = env
        .spawn_command("research", &["auth-fix", "Fix", "auth"])
        .output()
        .unwrap();
    assert_command_success("respawn", &respawn);
    assert!(state_dir(&env.root).join("worker/auth-fix").is_dir());
    assert!(
        state_dir(&env.root)
            .join("worker/auth-fix/meta.json")
            .is_file()
    );

    let log = env.tmux_log();
    assert!(log.contains("new-window -d -t =niles-test-session:"));
    assert!(log.contains(": -n niles-auth-fix"));
}

#[test]
fn spawn_meta_write_failure_kills_window_and_cleans_location() {
    let env = TestEnv::with_tmux(
        "niles-worker-meta-write-failed-spawn",
        r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  list-windows) exit 0 ;;
  new-window) mkdir -p "$META_PATH"; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    );
    write_spawn_manifest(&env.root, "claude");

    let failed = env
        .spawn_command("research", &["auth-fix", "Fix", "auth"])
        .env(
            "META_PATH",
            state_dir(&env.root).join("worker/auth-fix/meta.json"),
        )
        .output()
        .unwrap();
    assert_failure_contains(
        "spawn with meta write failure",
        &failed,
        "failed to finish launching worker auth-fix",
    );
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("cleaned up launched worker"));
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());

    let log = env.tmux_log();
    assert!(log.contains("new-window -d -t =niles-test-session:"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains("set-option -w -t "));
    assert!(log.contains(" @niles-worker-id auth-fix"));
    assert!(log.contains("kill-window -t "));
    assert!(log.contains(":=niles-auth-fix"));
}
