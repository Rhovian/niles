use super::support::*;

#[test]
fn respawn_after_successful_close_from_same_cwd_gets_fresh_worker_dir() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-respawn-same-cwd");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    let first = Command::new(niles)
        .args(["spawn", "reviewer", "--agent", "claude", "FIRST"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("first spawn", &first);

    let worker_dir = workspace.join(".niles/worker/reviewer");
    fs::write(worker_dir.join("report.md"), "first report\n").unwrap();
    fs::write(worker_dir.join("status.log"), "done: first\n").unwrap();

    let close = Command::new(niles)
        .args(["close", "reviewer"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "first pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
        .args(["spawn", "reviewer", "--agent", "claude", "SECOND"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-worker-double-close");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture(&workspace, "auth-fix", "working: close requested");
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("first close", &close);

    let second = Command::new(niles)
        .args(["close", "auth-fix"])
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
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE_EMPTY", "1")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
        .args(["close", "auth-fix"])
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
    let workspace = temp_workspace("niles-close-wait");
    let home = niles_home(&workspace);

    // The waited-on worker needs a real tmux window, or `wait`'s window-gone check
    // (commit 82c8795) would report it gone before the close ever lands.
    let server = TmuxServer::start(&workspace, "niles");
    server.new_window("niles-auth-fix");

    write_worker_fixture(&workspace, "auth-fix", "working: close requested");

    let waiter = Command::new(niles)
        .args(["wait", "auth-fix", "--interval", "0.05", "--timeout", "5"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .env("TMUX", server.tmux_env())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));

    // No stub tmux in PATH: `close` must reach the real tmux server so it can see and kill the
    // worker's window.
    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX", server.tmux_env())
        .output()
        .unwrap();
    assert_command_success("close", &close);

    let started = Instant::now();
    let output = waiter.wait_with_output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(10));

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
