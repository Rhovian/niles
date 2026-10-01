use super::support::*;

#[test]
fn workers_lists_live_workers_with_task_age_and_last_status() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-list");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_task(
        &workspace,
        "auth-fix",
        "working: running tests\ndone: ready for review\n",
        Some("auth"),
    );
    write_worker_fixture(
        &workspace,
        "reviewer",
        "working: reading diff\nblocked: needs clarification\n",
    );
    let archive = workspace.join(".niles/worker/archive/old-worker-20000101T000000000000000Z");
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("status.log"), "done: archived\n").unwrap();

    let output = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_WINDOWS", "niles-auth-fix\t0")
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("workers[2]{id,agent,task,age,window,wake,last_status}:"));
    assert!(stdout.lines().any(|line| {
        line.contains("auth-fix,codex,auth,")
            && line.contains(",live,")
            && line.contains("done: ready for review")
    }));
    assert!(stdout.lines().any(|line| {
        line.contains("reviewer,codex,-,")
            && line.contains(",window-dead,")
            && line.contains("blocked: needs clarification")
    }));
    assert!(!stdout.contains("old-worker"));
}

/// Two workers reporting `done:` render identically unless the listing says which line the lead
/// still owes itself a `niles wait` for. That is how a finished worker sat unnoticed for twenty
/// minutes next to two that had already been collected.
#[test]
fn workers_marks_a_worker_whose_wake_has_not_been_collected() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-pending-wake");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    // Collected: a wait consumed the `done:` and left its cursor past the end of the log.
    let collected = write_worker_fixture(&workspace, "collected", "done: ready for review\n");
    fs::write(collected.join("status.cursor"), "23\n").unwrap();
    // Waiting: the same line, and no wait has ever run against it.
    write_worker_fixture(&workspace, "waiting", "done: ready for review\n");
    // Working: the only undelivered line wakes nobody, so nothing is owed.
    let working = write_worker_fixture(
        &workspace,
        "working",
        "done: first pass\nworking: second pass\n",
    );
    fs::write(working.join("status.cursor"), "17\n").unwrap();

    let output = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env(
            "TMUX_WINDOWS",
            "niles-collected\t0\nniles-waiting\t0\nniles-working\t0",
        )
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = |id: &str| {
        stdout
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{id},")))
            .unwrap_or_else(|| panic!("no row for {id}:\n{stdout}"))
            .to_owned()
    };

    // The same `last_status` on both, and only the uncollected one is marked.
    assert!(
        row("collected").contains(",-,done: ready for review"),
        "{stdout}"
    );
    assert!(
        row("waiting").contains(",pending,done: ready for review"),
        "{stdout}"
    );
    assert!(
        row("working").contains(",-,working: second pass"),
        "{stdout}"
    );
}

#[test]
fn workers_reports_unknown_when_tmux_window_query_fails() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-workers-list-unknown");
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture(&workspace, "auth-fix", "working: checking window\n");

    let output = Command::new(niles)
        .arg("workers")
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", niles_home(&workspace))
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_LIST_WINDOWS_FAIL", "1")
        .output()
        .unwrap();

    assert_command_success("workers", &output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unknown:tmux list-windows failed for session niles"));
    assert!(stdout.contains("server unreachable retry later"));
    assert!(!stdout.lines().any(|line| line.starts_with("retry later")));
    assert!(!stdout.contains("window-dead"));
}
