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
    fs::create_dir_all(state_dir(&workspace).join("worker")).unwrap();
    fs::write(
        state_dir(&workspace).join("worker/auth-fix.json"),
        r#"{"id":"auth-fix"}"#,
    )
    .unwrap();

    let workers = run_bare(&workspace, &["workers"]);
    assert_command_success("workers ignores leftover json", &workers);
    let stdout = stdout_of(&workers);
    assert_eq!(stdout, "{\"workers\":[]}\n");

    let peek = run_bare(&workspace, &["peek", "auth-fix"]);
    assert_failure_contains("peek leftover json", &peek, "unknown worker id 'auth-fix'");
}

#[test]
fn spawn_window_failure_cleans_partial_worker_and_allows_respawn() {
    let env = TestEnv::with_manifest("niles-worker-failed-spawn", "claude");

    let failed = env
        .spawn_command("research", &["auth-fix", "Fix", "auth"])
        .env("TMUX_FAIL_NEW_WINDOW", "1")
        .output()
        .unwrap();
    assert_failure_contains("failed spawn", &failed, "failed to launch worker auth-fix");
    let stderr = stderr_of(&failed);
    assert!(stderr.contains("create window failed: index 1 in use"));
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());

    let peek = run_bare(&env.root, &["peek", "auth-fix"]);
    assert_failure_contains("peek failed spawn", &peek, "unknown worker id 'auth-fix'");

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
    let stderr = stderr_of(&failed);
    assert!(stderr.contains("cleaned up launched worker"));
    assert!(!state_dir(&env.root).join("worker/auth-fix").exists());

    let log = env.tmux_log();
    assert!(log.contains("new-window -d -t =niles-test-session:"));
    assert!(log.contains(": -n niles-auth-fix"));
    assert!(log.contains("set-option -w -t "));
    assert!(log.contains(" @niles-worker-id auth-fix"));
    assert!(log.contains("kill-window -t "));
    assert!(log.contains(":=niles-auth-fix"));

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
}
