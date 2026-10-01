use super::support::*;

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
            .args(["spawn", "reviewer", "--agent", "claude", task])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env("TMUX", "/tmp/niles-test-tmux,0,0")
            .output()
            .unwrap();
        assert_command_success("spawn archived report worker", &spawn);

        let worker_dir = workspace.join(".niles/worker/reviewer");
        fs::write(worker_dir.join("report.md"), report_body).unwrap();

        let close = Command::new(niles)
            .args(["close", "reviewer"])
            .current_dir(&workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env("TMUX_CAPTURE", task)
            .env("TMUX", "/tmp/niles-test-tmux,0,0")
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
