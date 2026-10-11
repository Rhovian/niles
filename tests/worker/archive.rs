use super::support::*;

#[test]
fn respawn_after_successful_close_from_same_cwd_gets_fresh_worker_dir() {
    let env = TestEnv::new("niles-worker-respawn-same-cwd");

    let first = env
        .niles(
            &env.root,
            &[
                "spawn", "--role", "research", "reviewer", "--agent", "claude", "FIRST",
            ],
        )
        .output()
        .unwrap();
    assert_command_success("first spawn", &first);

    let worker_dir = env.root.join(".niles/worker/reviewer");
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

    let second = env
        .niles(
            &env.root,
            &[
                "spawn", "--role", "research", "reviewer", "--agent", "claude", "SECOND",
            ],
        )
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
fn worker_close_on_archived_worker_errors_and_mentions_archive() {
    let env = TestEnv::new("niles-worker-double-close");

    write_worker(
        &env.root,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"working: close requested",
    );
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_CAPTURE", "pane")
        .output()
        .unwrap();
    assert_command_success("first close", &close);

    let second = niles_bare(&env.root, &env.home)
        .args(["close", "auth-fix"])
        .output()
        .unwrap();
    assert_failure_contains("second close", &second, "no live worker 'auth-fix'");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains(".niles/worker/archive/auth-fix-"));
}

#[test]
fn worker_close_does_not_write_or_advertise_empty_final_pane() {
    let env = TestEnv::new("niles-worker-empty-pane-close");

    write_worker(
        &env.root,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"working: close requested",
    );
    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_CAPTURE_EMPTY", "1")
        .output()
        .unwrap();
    assert_command_success("close empty pane worker", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(!stdout.contains("pane:"));
    let archive_dir = latest_archive_dir(&env.root, "auth-fix");
    assert!(!archive_dir.join("final-pane.txt").exists());
}

#[test]
fn worker_close_malformed_metadata_names_path_and_cause() {
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

    let output = niles_bare(&workspace, &niles_home(&workspace))
        .args(["close", "auth-fix"])
        .output()
        .unwrap();

    assert_failure_contains("close with old metadata", &output, "failed to parse");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("meta.json"));
    assert!(stderr.contains("missing field"), "{stderr}");
}
