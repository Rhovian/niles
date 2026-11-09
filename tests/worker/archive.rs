use super::support::*;

#[test]
fn respawn_after_successful_close_from_same_cwd_gets_fresh_worker_dir() {
    let env = TestEnv::with_manifest("niles-worker-respawn-same-cwd", "claude");

    let first = env.spawn("research", &["reviewer", "FIRST"]);
    assert_command_success("first spawn", &first);

    let worker_dir = state_dir(&env.root).join("worker/reviewer");
    fs::write(worker_dir.join("report.md"), "first report\n").unwrap();
    fs::write(worker_dir.join("status.log"), "done: first\n").unwrap();

    let close = env
        .niles(&env.root, &["close", "reviewer"])
        .env("TMUX_CAPTURE", "first pane")
        .output()
        .unwrap();
    assert_command_success("close first worker", &close);
    assert!(!worker_dir.exists());

    let archive_dir = latest_archive_dir(&env.root, "reviewer");
    assert_eq!(
        fs::read_to_string(archive_dir.join("report.md")).unwrap(),
        "first report\n"
    );

    let second = env.spawn("research", &["reviewer", "SECOND"]);
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
fn worker_close_does_not_write_or_advertise_empty_final_pane() {
    let env = TestEnv::new("niles-worker-empty-pane-close");

    let close = close_auth_fix(
        &env,
        "niles:niles-auth-fix",
        b"working: close requested",
        &[("TMUX_CAPTURE_EMPTY", "1")],
    );
    assert_command_success("close empty pane worker", &close);
    let stdout = stdout_of(&close);
    assert!(!stdout.contains("pane:"));
    let archive_dir = latest_archive_dir(&env.root, "auth-fix");
    assert!(!archive_dir.join("final-pane.txt").exists());
}

#[test]
fn worker_close_malformed_metadata_names_path_and_cause() {
    let workspace = temp_workspace("niles-worker-invalid-meta");
    write_corrupt_worker_fixture(&workspace, "auth-fix");

    let output = run_bare(&workspace, &["close", "auth-fix"]);

    assert_failure_contains("close with malformed metadata", &output, "failed to parse");
    let stderr = stderr_of(&output);
    assert!(stderr.contains("meta.json"));
    assert!(stderr.contains("missing field"), "{stderr}");
}
