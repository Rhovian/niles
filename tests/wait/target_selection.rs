use super::support::*;

#[test]
fn waiting_on_several_workers_prefixes_the_winning_id() {
    let workspace = temp_workspace("niles-wait-fleet");
    worker_with_status(&workspace, "alpha", b"working: nothing yet\n");
    worker_with_status(&workspace, "beta", b"done: beta finished\n");

    let output = run_wait(
        &workspace,
        &["alpha", "beta", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("fleet wait", &output);
    assert_eq!(stdout_of(&output), "beta: done: beta finished\n");
}

#[test]
fn corrupt_cursor_fails_loudly_and_names_the_file() {
    let workspace = temp_workspace("niles-wait-corrupt");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    fs::write(worker_dir.join("status.cursor"), "not-a-number\n").unwrap();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("invalid wake cursor"), "stderr: {stderr}");
    // The operator has to be told which file to delete, or the worker is wedged.
    assert!(stderr.contains("status.cursor"), "stderr: {stderr}");
    assert!(stderr.contains("remove it to resume"), "stderr: {stderr}");
}

#[test]
fn unknown_id_errors_without_closed_backstop() {
    let workspace = temp_workspace("niles-wait-unknown");

    let output = run_wait(
        &workspace,
        &["missing", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    assert!(stdout_of(&output).is_empty());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("unknown worker id 'missing'"));
    assert!(!stderr.contains("worker 'missing' closed"));
}

#[test]
fn returns_closed_backstop_when_the_directory_is_removed_mid_wait() {
    let workspace = temp_workspace("niles-wait-removed");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"working: still running\n");

    let waiter = spawn_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "5"],
    );
    // The removal has to land after target resolution, or this is the unknown-id path instead.
    settle();
    remove_dir_all_eventually(&worker_dir);

    let output = waiter.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(
        stdout_of(&output),
        "closed: worker 'auth-fix' directory removed\n"
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("worker 'auth-fix' closed"));
    assert!(!stderr.contains("timeout"));
}

#[test]
fn a_symlinked_cursor_path_is_refused_rather_than_followed() {
    let workspace = temp_workspace("niles-wait-symlink");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    let outside = workspace.join("outside.txt");
    fs::write(&outside, "untouched\n").unwrap();
    std::os::unix::fs::symlink(&outside, worker_dir.join("status.cursor")).unwrap();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "untouched\n",
        "the cursor write followed a symlink out of the worker directory"
    );
}

#[test]
fn rejects_both_worker_and_task_selectors() {
    let workspace = temp_workspace("niles-wait-selectors");

    let output = run_wait(
        &workspace,
        &["auth-fix", "--task", "auth", "--timeout", "0"],
    );

    assert!(!output.status.success());
    let combined = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        combined.contains("cannot be used with") || combined.contains("use either"),
        "output: {combined}"
    );
}

#[test]
fn caps_an_enormous_status_line_instead_of_flooding_the_manager() {
    let workspace = temp_workspace("niles-wait-huge");
    let mut status = b"done: ".to_vec();
    status.extend(std::iter::repeat_n(b'x', 64 * 1024));
    status.push(b'\n');
    worker_with_status(&workspace, "auth-fix", &status);

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "0.05", "--timeout", "0"],
    );

    assert_command_success("wait with an enormous line", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("done: xxx"));
    assert!(stdout.contains("(truncated)"), "stdout was not capped");
    assert!(stdout.len() < 8 * 1024, "stdout was {} bytes", stdout.len());
}

#[test]
fn task_label_waits_on_every_live_worker_carrying_it() {
    let workspace = temp_workspace("niles-wait-task");
    // The fabricated workers need real tmux windows, or `wait`'s window-gone check (commit
    // 82c8795) would report them gone before their status logs are ever read.
    let server = TmuxServer::start(&workspace, "niles");
    server.new_window("niles-alpha");
    server.new_window("niles-beta");
    server.new_window("niles-gamma");
    write_task_worker(&workspace, "alpha", "auth", b"working: nothing yet\n");
    write_task_worker(&workspace, "beta", "auth", b"blocked: needs a decision\n");
    write_task_worker(&workspace, "gamma", "other", b"done: unrelated task\n");

    let bin = workspace.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let output = niles_in(
        &server,
        &workspace,
        &bin,
        &[
            "wait",
            "--task",
            "auth",
            "--interval",
            "0.05",
            "--timeout",
            "0",
        ],
    )
    .output()
    .unwrap();

    assert_command_success("wait --task", &output);
    // gamma carries a different label, so its wake must not satisfy this wait.
    assert_eq!(stdout_of(&output), "beta: blocked: needs a decision\n");
}
