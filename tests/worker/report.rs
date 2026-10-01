use super::support::*;

#[test]
fn report_prints_worker_report_file() {
    let workspace = temp_workspace("niles-worker-report");

    let worker_dir = write_worker(
        &workspace,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"working: report ready",
    );
    fs::write(
        worker_dir.join("report.md"),
        "# Findings\n\n- durable content\n",
    )
    .unwrap();

    let report = niles_bare(&workspace, &niles_home(&workspace))
        .args(["report", "auth-fix"])
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
    let workspace = temp_workspace("niles-worker-report-missing");

    let worker_dir = write_worker(
        &workspace,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"working: no report yet",
    );
    fs::write(worker_dir.join("final-pane.txt"), "pane tail\n").unwrap();

    let report = niles_bare(&workspace, &niles_home(&workspace))
        .args(["report", "auth-fix"])
        .output()
        .unwrap();
    assert_failure_contains(
        "missing report",
        &report,
        "no report found for worker 'auth-fix'",
    );
    assert!(String::from_utf8_lossy(&report.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&report.stderr);
    assert!(stderr.contains(".niles/worker/auth-fix/report.md"));
    assert!(stderr.contains("final pane snapshot is available"));
}

#[test]
fn report_falls_back_to_most_recent_local_archive() {
    let env = TestEnv::new("niles-worker-report-archive");
    let workspace = env.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();

    for (task, report_body) in [("FIRST", "first report\n"), ("SECOND", "second report\n")] {
        let spawn = env
            .niles(
                &workspace,
                &["spawn", "reviewer", "--agent", "claude", task],
            )
            .output()
            .unwrap();
        assert_command_success("spawn archived report worker", &spawn);

        let worker_dir = workspace.join(".niles/worker/reviewer");
        fs::write(worker_dir.join("report.md"), report_body).unwrap();

        let close = env
            .niles(&workspace, &["close", "reviewer"])
            .env("TMUX_CAPTURE", task)
            .output()
            .unwrap();
        assert_command_success("close archived report worker", &close);
    }

    let report = env
        .niles(&workspace, &["report", "reviewer"])
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
    let workspace = temp_workspace("niles-worker-prefix-archive");
    let home = niles_home(&workspace);
    let archive_root = workspace.join(".niles/worker/archive");
    let worker_archive = archive_root.join("a-20260705T120000000000Z");
    let sibling_archive = archive_root.join("a-fs-20260705T130000000000Z");
    fs::create_dir_all(&worker_archive).unwrap();
    fs::create_dir_all(&sibling_archive).unwrap();
    fs::write(worker_archive.join("report.md"), "short worker report\n").unwrap();
    fs::write(sibling_archive.join("report.md"), "sibling worker report\n").unwrap();

    let report = niles_bare(&workspace, &home)
        .args(["report", "a"])
        .output()
        .unwrap();

    assert_command_success("prefix sibling archived report", &report);
    assert_eq!(
        String::from_utf8_lossy(&report.stdout),
        "short worker report\n"
    );
}
