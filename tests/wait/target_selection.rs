use super::support::*;

#[test]
fn waiting_on_several_workers_prefixes_the_winning_id() {
    let workspace = temp_workspace("niles-wait-fleet");
    worker_with_status(&workspace, "alpha", b"working: nothing yet\n");
    worker_with_status(&workspace, "beta", b"done: beta finished\n");

    let output = run_wait(
        &workspace,
        &["alpha", "beta", "--interval", "50ms", "--timeout", "0"],
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
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );

    assert_failure_contains("wait with corrupt cursor", &output, "invalid wake cursor");
    let stderr = stderr_of(&output);
    // The operator has to be told which file to delete, or the worker is wedged.
    assert!(stderr.contains("status.cursor"), "stderr: {stderr}");
    assert!(stderr.contains("remove it to resume"), "stderr: {stderr}");
}

#[test]
fn unknown_id_errors_without_closed_backstop() {
    let workspace = temp_workspace("niles-wait-unknown");

    let output = run_wait(
        &workspace,
        &["missing", "--interval", "50ms", "--timeout", "0"],
    );

    assert_failure_contains(
        "wait for unknown worker",
        &output,
        "unknown worker id 'missing'",
    );
    assert!(stdout_of(&output).is_empty());
    let stderr = stderr_of(&output);
    assert!(!stderr.contains("worker 'missing' closed"));
}

#[test]
fn returns_closed_backstop_when_the_directory_is_removed_mid_wait() {
    let workspace = temp_workspace("niles-wait-removed");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"working: still running\n");

    let waiter = spawn_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "5s"],
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
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
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
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );

    assert_command_success("wait with an enormous line", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("done: xxx"));
    assert!(stdout.contains("(truncated)"), "stdout was not capped");
    assert!(stdout.len() < 8 * 1024, "stdout was {} bytes", stdout.len());
}

#[test]
fn task_label_waits_on_every_live_worker_carrying_it() {
    let lab = Lab::start("niles-wait-task");
    // The fabricated workers need real tmux windows so the window-gone check does not win.
    for (id, task, status) in [
        ("alpha", "auth", b"working: nothing yet\n".as_slice()),
        ("beta", "auth", b"blocked: needs a decision\n".as_slice()),
        ("gamma", "other", b"done: unrelated task\n".as_slice()),
    ] {
        lab.server.new_window(&format!("niles-{id}"));
        lab.worker(id, Some(task), status);
    }

    let output = lab
        .niles(&[
            "wait",
            "--task",
            "auth",
            "--interval",
            "50ms",
            "--timeout",
            "0",
        ])
        .output()
        .unwrap();

    assert_command_success("wait --task", &output);
    // gamma carries a different label, so its wake must not satisfy this wait.
    assert_eq!(stdout_of(&output), "beta: blocked: needs a decision\n");
}

#[test]
fn worker_close_wakes_waiters_with_nonzero_closed_status() {
    let lab = Lab::start("niles-close-wait");
    lab.worker_window("auth-fix", b"working: close requested");

    let waiter = lab
        .niles(&["wait", "auth-fix", "--interval", "50ms", "--timeout", "5s"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    settle();

    let close = lab.niles(&["close", "auth-fix"]).output().unwrap();
    assert_command_success("close", &close);

    let started = Instant::now();
    let output = waiter.wait_with_output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "wait did not return promptly; stdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(output.status.code(), Some(10));
    let stdout = stdout_of(&output);
    assert!(stdout.contains("closed:"), "stdout:\n{stdout}");
    let stderr = stderr_of(&output);
    assert!(stderr.contains("worker 'auth-fix' closed"), "{stderr}");
    assert!(!stderr.contains("timeout"), "{stderr}");
}
