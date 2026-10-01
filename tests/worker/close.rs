use super::support::*;

/// A worker whose agent exited keeps its window so the pane stays readable. That window is
/// still there to clean up — treating it as already gone left it behind forever.
#[test]
fn worker_close_kills_a_window_whose_agent_has_exited() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-exited");
    let home = niles_home(&workspace);

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  has-session) exit 0 ;;
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    printf 'niles-auth-fix\t1\n'
    exit 0
    ;;
  capture-pane) printf 'Do you trust the contents of this directory?\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();

    write_worker_fixture(&workspace, "auth-fix", "closed: agent exited (status 3)");

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", path_with_bin(&bin))
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("close after agent exit", &close);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(
        log.contains("kill-window -t =niles:=niles-auth-fix"),
        "the window was left behind:\n{log}"
    );
    // Its output is the only record of why the agent died, so it is captured before the kill.
    assert!(log.contains("capture-pane"), "{log}");
    let archived = fs::read_dir(workspace.join(".niles/worker/archive"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let pane = fs::read_to_string(archived.join("final-pane.txt")).unwrap();
    assert!(pane.contains("Do you trust"), "{pane}");
}

#[test]
fn worker_close_tears_down_worker() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close");
    let home = niles_home(&workspace);

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = workspace.join("tmux.log");
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  has-session) exit 0 ;;
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    printf 'niles-auth-fix\t0\n'
    exit 0
    ;;
  capture-pane) printf 'final pane\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&tmux).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tmux, permissions).unwrap();

    write_worker_fixture(&workspace, "auth-fix", "status");
    let worker_dir = workspace.join(".niles/worker/auth-fix");
    fs::write(worker_dir.join("report.md"), "durable report\n").unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(
        close.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(close_stdout.contains("pane:"));
    assert!(close_stdout.contains("archive:"));
    assert!(close_stdout.contains("closed window: niles-auth-fix"));
    assert!(close_stdout.contains("closed: auth-fix"));

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(log.contains("capture-pane -p -t =niles:=niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t =niles:=niles-auth-fix"));
    assert!(!workspace.join(".niles/worker/auth-fix.json").exists());
    assert!(!worker_dir.exists());
    let archive_dir = latest_archive_dir(&workspace, "auth-fix");
    assert_eq!(
        fs::read_to_string(archive_dir.join("final-pane.txt")).unwrap(),
        "final pane\n"
    );
    assert_eq!(
        fs::read_to_string(archive_dir.join("report.md")).unwrap(),
        "durable report\n"
    );
    assert_global_index_absent(&home);
}

#[test]
fn worker_close_targets_recorded_session_not_ambient() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-recorded");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_window(
        &workspace,
        "auth-fix",
        "working: close requested",
        "home:niles-auth-fix",
    );

    let close = Command::new(niles)
        .args(["close", "auth-fix"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_WINDOWS", "niles-auth-fix\t0")
        .env("TMUX", "/tmp/ambient-tmux")
        .output()
        .unwrap();
    assert_command_success("recorded-target close", &close);

    let log = fs::read_to_string(&tmux_log).unwrap();
    assert!(!log.contains("display-message"));
    assert!(log.contains("list-windows -t =home -F #{window_name}\t#{pane_dead}"));
    assert!(log.contains("capture-pane -p -t =home:=niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t =home:=niles-auth-fix"));
}

#[test]
fn worker_close_by_task_closes_matching_workers_only() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-task");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_worker_fixture_with_task(&workspace, "auth-one", "working: one", Some("auth"));
    write_worker_fixture_with_task(&workspace, "auth-two", "working: two", Some("auth"));
    write_worker_fixture_with_task(&workspace, "docs-one", "working: docs", Some("docs"));

    let close = Command::new(niles)
        .args(["close", "--task", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();

    assert_command_success("close --task", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("auth-one,closed,"));
    assert!(stdout.contains("auth-two,closed,"));
    assert!(!stdout.contains("docs-one,closed,"));

    assert!(!workspace.join(".niles/worker/auth-one").exists());
    assert!(!workspace.join(".niles/worker/auth-two").exists());
    assert!(workspace.join(".niles/worker/docs-one").exists());

    let archive_one = latest_archive_dir(&workspace, "auth-one");
    let archive_two = latest_archive_dir(&workspace, "auth-two");
    assert!(
        fs::read_to_string(archive_one.join("status.log"))
            .unwrap()
            .contains("closed: auth-one")
    );
    assert!(
        fs::read_to_string(archive_two.join("status.log"))
            .unwrap()
            .contains("closed: auth-two")
    );
}

#[test]
fn worker_close_all_is_scoped_to_invoking_workspace() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let root = temp_workspace("niles-close-scope");
    let workspace_a = root.join("workspace-a");
    let workspace_b = root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    let home = niles_home(&root);
    let (bin, tmux_log) = write_worker_test_bins(&root);
    let path = path_with_bin(&bin);

    for (workspace, id, label) in [
        (&workspace_a, "alpha", "task-a"),
        (&workspace_b, "bravo", "task-b"),
    ] {
        let spawn = Command::new(niles)
            .args(["spawn", id, "--task", label, "--agent", "claude", "Fix"])
            .current_dir(workspace)
            .env("PATH", &path)
            .env("NILES_HOME", &home)
            .env("TMUX_LOG", &tmux_log)
            .env("TMUX", "/tmp/niles-test-tmux,0,0")
            .output()
            .unwrap();
        assert_command_success("scoped close spawn", &spawn);
    }

    let close = Command::new(niles)
        .args(["close", "--all"])
        .current_dir(&workspace_a)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert_command_success("workspace-scoped close --all", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[1]{id,status,archive}:"));
    assert!(stdout.contains("alpha,closed,"));
    assert!(!stdout.contains("bravo"));

    assert!(!workspace_a.join(".niles/worker/alpha").exists());
    assert!(workspace_b.join(".niles/worker/bravo").exists());
    assert!(workspace_b.join(".niles/worker/bravo/meta.json").exists());

    let close_foreign_task = Command::new(niles)
        .args(["close", "--task", "task-b"])
        .current_dir(&workspace_a)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();
    assert!(!close_foreign_task.status.success());
    assert!(
        String::from_utf8_lossy(&close_foreign_task.stderr)
            .contains("no live workers with task label task-b")
    );
    assert!(workspace_b.join(".niles/worker/bravo").exists());
}

#[test]
fn worker_close_zero_match_behaviors_are_distinct() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-zero");

    let close_all = Command::new(niles)
        .args(["close", "--all"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert_command_success("empty close --all", &close_all);
    assert_eq!(
        String::from_utf8_lossy(&close_all.stdout),
        "no live workers\n"
    );

    write_worker_fixture_with_task(&workspace, "docs-one", "working: docs", Some("docs"));

    let close_task = Command::new(niles)
        .args(["close", "--task", "missing"])
        .current_dir(&workspace)
        .env("NILES_HOME", niles_home(&workspace))
        .output()
        .unwrap();
    assert!(!close_task.status.success());
    assert!(String::from_utf8_lossy(&close_task.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&close_task.stderr)
            .contains("no live workers with task label missing")
    );
}

#[test]
fn worker_close_by_task_reports_selection_failures_and_closes_matches() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-task-selection-failure");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_corrupt_worker_fixture(&workspace, "bad-meta");
    write_worker_fixture_with_task(&workspace, "good-worker", "working: close me", Some("auth"));

    let close = Command::new(niles)
        .args(["close", "--task", "auth"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();

    assert!(!close.status.success());
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("bad-meta,failed,-"));
    assert!(stdout.contains("good-worker,closed,"));
    let stderr = String::from_utf8_lossy(&close.stderr);
    assert!(stderr.contains("worker bad-meta close failed"));
    assert!(stderr.contains("close --task auth failed for 1 worker(s): bad-meta"));

    assert!(workspace.join(".niles/worker/bad-meta").exists());
    assert!(!workspace.join(".niles/worker/good-worker").exists());
    assert!(latest_archive_dir(&workspace, "good-worker").exists());
}

#[test]
fn worker_close_all_reports_partial_failures_without_aborting_rest() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-all-partial");
    let home = niles_home(&workspace);
    let (bin, tmux_log) = write_worker_test_bins(&workspace);
    let path = path_with_bin(&bin);

    write_corrupt_worker_fixture(&workspace, "bad-meta");
    write_worker_fixture(&workspace, "good-worker", "working: close me");

    let close = Command::new(niles)
        .args(["close", "--all"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("NILES_HOME", &home)
        .env("TMUX_LOG", &tmux_log)
        .env("TMUX_CAPTURE", "pane")
        .env("TMUX", "/tmp/niles-test-tmux,0,0")
        .output()
        .unwrap();

    assert!(!close.status.success());
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("bad-meta,failed,-"));
    assert!(stdout.contains("good-worker,closed,"));
    let stderr = String::from_utf8_lossy(&close.stderr);
    assert!(stderr.contains("worker bad-meta close failed"));
    assert!(stderr.contains("close --all failed for 1 worker(s): bad-meta"));

    assert!(workspace.join(".niles/worker/bad-meta").exists());
    assert!(!workspace.join(".niles/worker/good-worker").exists());
    assert!(latest_archive_dir(&workspace, "good-worker").exists());
}

#[test]
fn worker_close_unknown_id_errors() {
    let niles = env!("CARGO_BIN_EXE_niles");
    let workspace = temp_workspace("niles-close-missing");
    let home = niles_home(&workspace);

    let close = Command::new(niles)
        .args(["close", "missing"])
        .current_dir(&workspace)
        .env("NILES_HOME", &home)
        .output()
        .unwrap();
    assert!(!close.status.success());
    assert!(
        String::from_utf8_lossy(&close.stderr).contains("no live worker 'missing'"),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&close.stdout),
        String::from_utf8_lossy(&close.stderr)
    );
}
