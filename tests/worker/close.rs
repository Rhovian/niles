use super::support::*;

/// A worker whose agent exited keeps its window so the pane stays readable. That window is
/// still there to clean up — treating it as already gone left it behind forever.
#[test]
fn worker_close_kills_a_window_whose_agent_has_exited() {
    let env = TestEnv::new("niles-close-exited");
    write_worker(
        &env.root,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"closed: agent exited (status 3)",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_WINDOWS", "niles-auth-fix\t1")
        .env(
            "TMUX_CAPTURE",
            "Do you trust the contents of this directory?",
        )
        .output()
        .unwrap();
    assert_command_success("close after agent exit", &close);

    let log = env.tmux_log();
    assert!(
        log.contains("kill-window -t =niles:=niles-auth-fix"),
        "the window was left behind:\n{log}"
    );
    // Its output is the only record of why the agent died, so it is captured before the kill.
    assert!(log.contains("capture-pane"), "{log}");
    let archived = latest_archive_dir(&env.root, "auth-fix");
    let pane = fs::read_to_string(archived.join("final-pane.txt")).unwrap();
    assert!(pane.contains("Do you trust"), "{pane}");
}

#[test]
fn worker_close_tears_down_worker() {
    let env = TestEnv::new("niles-close");
    let worker_dir = write_worker(
        &env.root,
        "auth-fix",
        "niles:niles-auth-fix",
        None,
        b"status",
    );
    fs::write(worker_dir.join("report.md"), "durable report\n").unwrap();

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_WINDOWS", "niles-auth-fix\t0")
        .env("TMUX_CAPTURE", "final pane")
        .output()
        .unwrap();
    assert_command_success("close", &close);
    let close_stdout = String::from_utf8_lossy(&close.stdout);
    assert!(close_stdout.contains("pane:"));
    assert!(close_stdout.contains("archive:"));
    assert!(close_stdout.contains("closed window: niles-auth-fix"));
    assert!(close_stdout.contains("closed: auth-fix"));

    let log = env.tmux_log();
    assert!(log.contains("capture-pane -p -t =niles:=niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t =niles:=niles-auth-fix"));
    assert!(!worker_dir.exists());
    let archive_dir = latest_archive_dir(&env.root, "auth-fix");
    assert_eq!(
        fs::read_to_string(archive_dir.join("final-pane.txt")).unwrap(),
        "final pane\n"
    );
    assert_eq!(
        fs::read_to_string(archive_dir.join("report.md")).unwrap(),
        "durable report\n"
    );
}

#[test]
fn worker_close_targets_recorded_session_not_ambient() {
    let env = TestEnv::new("niles-close-recorded");

    write_worker(
        &env.root,
        "auth-fix",
        "home:niles-auth-fix",
        None,
        b"working: close requested",
    );

    let close = env
        .niles(&env.root, &["close", "auth-fix"])
        .env("TMUX_WINDOWS", "niles-auth-fix\t0")
        .env("TMUX", "/tmp/ambient-tmux")
        .output()
        .unwrap();
    assert_command_success("recorded-target close", &close);

    let log = env.tmux_log();
    assert!(!log.contains("display-message"));
    assert!(log.contains("list-windows -t =home -F #{window_name}\t#{pane_dead}"));
    assert!(log.contains("capture-pane -p -t =home:=niles-auth-fix -S -2000"));
    assert!(log.contains("kill-window -t =home:=niles-auth-fix"));
}

#[test]
fn worker_close_by_task_closes_matching_workers_only() {
    let env = TestEnv::new("niles-close-task");

    for (id, task, status) in [
        ("auth-one", "auth", b"working: one".as_slice()),
        ("auth-two", "auth", b"working: two".as_slice()),
        ("docs-one", "docs", b"working: docs".as_slice()),
    ] {
        write_worker(
            &env.root,
            id,
            &format!("niles:niles-{id}"),
            Some(task),
            status,
        );
    }

    let close = env
        .niles(&env.root, &["close", "--task", "auth"])
        .env("TMUX_CAPTURE", "pane")
        .output()
        .unwrap();

    assert_command_success("close --task", &close);
    let stdout = String::from_utf8_lossy(&close.stdout);
    assert!(stdout.contains("workers[2]{id,status,archive}:"));
    assert!(stdout.contains("auth-one,closed,"));
    assert!(stdout.contains("auth-two,closed,"));
    assert!(!stdout.contains("docs-one,closed,"));

    assert!(!env.root.join(".niles/worker/auth-one").exists());
    assert!(!env.root.join(".niles/worker/auth-two").exists());
    assert!(env.root.join(".niles/worker/docs-one").exists());

    assert_archived_with_closed_sentinel(&env.root, "auth-one");
    assert_archived_with_closed_sentinel(&env.root, "auth-two");
}

#[test]
fn worker_close_all_is_scoped_to_invoking_workspace() {
    let env = TestEnv::new("niles-close-scope");
    let workspace_a = env.root.join("workspace-a");
    let workspace_b = env.root.join("workspace-b");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();
    for (workspace, id, label) in [
        (&workspace_a, "alpha", "task-a"),
        (&workspace_b, "bravo", "task-b"),
    ] {
        let spawn = env
            .niles(
                workspace,
                &["spawn", id, "--task", label, "--agent", "claude", "Fix"],
            )
            .output()
            .unwrap();
        assert_command_success("scoped close spawn", &spawn);
    }

    let close = env
        .niles(&workspace_a, &["close", "--all"])
        .env("TMUX_CAPTURE", "pane")
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

    let close_foreign_task = env
        .niles(&workspace_a, &["close", "--task", "task-b"])
        .output()
        .unwrap();
    assert_failure_contains(
        "foreign task close",
        &close_foreign_task,
        "no live workers with task label task-b",
    );
    assert!(workspace_b.join(".niles/worker/bravo").exists());
}

#[test]
fn worker_close_zero_match_behaviors_are_distinct() {
    let workspace = temp_workspace("niles-close-zero");

    let close_all = niles_bare(&workspace, &niles_home(&workspace))
        .args(["close", "--all"])
        .output()
        .unwrap();
    assert_command_success("empty close --all", &close_all);
    assert_eq!(
        String::from_utf8_lossy(&close_all.stdout),
        "no live workers\n"
    );

    write_worker(
        &workspace,
        "docs-one",
        "niles:niles-docs-one",
        Some("docs"),
        b"working: docs",
    );

    let close_task = niles_bare(&workspace, &niles_home(&workspace))
        .args(["close", "--task", "missing"])
        .output()
        .unwrap();
    assert_failure_contains(
        "missing task close",
        &close_task,
        "no live workers with task label missing",
    );
    assert!(String::from_utf8_lossy(&close_task.stdout).is_empty());
}

#[test]
fn worker_close_selection_reports_partial_failures_without_aborting_rest() {
    for (suffix, args, task, summary) in [
        (
            "task",
            &["close", "--task", "auth"][..],
            Some("auth"),
            "close --task auth failed for 1 worker(s): bad-meta",
        ),
        (
            "all",
            &["close", "--all"][..],
            None,
            "close --all failed for 1 worker(s): bad-meta",
        ),
    ] {
        let env = TestEnv::new(&format!("niles-close-{suffix}-partial"));
        write_corrupt_worker_fixture(&env.root, "bad-meta");
        write_worker(
            &env.root,
            "good-worker",
            "niles:niles-good-worker",
            task,
            b"working: close me",
        );

        let close = env
            .niles(&env.root, args)
            .env("TMUX_CAPTURE", "pane")
            .output()
            .unwrap();

        assert_failure_contains("partial close", &close, summary);
        let stdout = String::from_utf8_lossy(&close.stdout);
        assert!(stdout.contains("workers[2]{id,status,archive}:"));
        assert!(stdout.contains("bad-meta,failed,-"));
        assert!(stdout.contains("good-worker,closed,"));
        let stderr = String::from_utf8_lossy(&close.stderr);
        assert!(stderr.contains("worker bad-meta close failed"));
        assert!(env.root.join(".niles/worker/bad-meta").exists());
        assert!(!env.root.join(".niles/worker/good-worker").exists());
        assert!(latest_archive_dir(&env.root, "good-worker").exists());
    }
}

#[test]
fn worker_close_unknown_id_errors() {
    let workspace = temp_workspace("niles-close-missing");
    let home = niles_home(&workspace);

    let close = niles_bare(&workspace, &home)
        .args(["close", "missing"])
        .output()
        .unwrap();
    assert_failure_contains("close unknown worker", &close, "no live worker 'missing'");
}
