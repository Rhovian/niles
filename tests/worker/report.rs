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
    fs::write(worker_dir.join("final-pane.txt"), "pane tail\n").unwrap();

    let report = run_bare(&workspace, &["report", "auth-fix"]);
    assert_failure_contains(
        "missing report",
        &report,
        "no report found for worker 'auth-fix'",
    );
    assert!(stdout_of(&report).is_empty());
    let stderr = stderr_of(&report);
    assert!(stderr.contains("worker/auth-fix/report.md"));
    assert!(stderr.contains("final pane snapshot is available"));

    fs::write(
        worker_dir.join("report.md"),
        "# Findings\n\n- durable content\n",
    )
    .unwrap();

    let report = run_bare(&workspace, &["report", "auth-fix"]);
    assert_command_success("report", &report);
    assert_eq!(stdout_of(&report), "# Findings\n\n- durable content\n");
}

#[test]
fn report_falls_back_to_most_recent_local_archive() {
    let env = TestEnv::new("niles-worker-report-archive");
    let workspace = env.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    write_spawn_manifest(&workspace, "claude");

    for (task, report_body) in [("FIRST", "first report\n"), ("SECOND", "second report\n")] {
        let spawn = env
            .niles(
                &workspace,
                &["spawn", "--skill", "research", "reviewer", task],
            )
            .output()
            .unwrap();
        assert_command_success("spawn archived report worker", &spawn);

        let worker_dir = state_dir(&workspace).join("worker/reviewer");
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
    assert_eq!(stdout_of(&report), "second report\n");
    let stderr = stderr_of(&report);
    assert!(stderr.contains("serving archived report from"));
    assert!(stderr.contains("worker/archive/reviewer-"));
}

#[test]
fn report_skips_prefix_sibling_worker_archive() {
    let workspace = temp_workspace("niles-worker-prefix-archive");
    let archive_root = state_dir(&workspace).join("worker/archive");
    let worker_archive = archive_root.join("a-20260705T120000000000Z");
    let sibling_archive = archive_root.join("a-fs-20260705T130000000000Z");
    fs::create_dir_all(&worker_archive).unwrap();
    fs::create_dir_all(&sibling_archive).unwrap();
    fs::write(worker_archive.join("report.md"), "short worker report\n").unwrap();
    fs::write(sibling_archive.join("report.md"), "sibling worker report\n").unwrap();

    let report = run_bare(&workspace, &["report", "a"]);

    assert_command_success("prefix sibling archived report", &report);
    assert_eq!(stdout_of(&report), "short worker report\n");
}
