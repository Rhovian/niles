use super::support::*;

#[test]
fn pending_reports_are_one_immediate_batch_with_one_line_per_worker() {
    let workspace = temp_workspace("niles-workspace-batch");
    let first = worker_with_status(&workspace, "a", b"done: one\nblocked: two\n");
    let second = worker_with_status(&workspace, "b", b"needs-decision: choose\n");
    let quiet = worker_with_status(&workspace, "quiet", b"working: busy\n");
    let closed = worker_with_status(&workspace, "closed", b"closed: operator closed\n");
    fs::create_dir_all(state_dir(&workspace).join("worker/leftover")).unwrap();
    fs::write(
        state_dir(&workspace).join("worker/leftover/status.log"),
        b"done: ignored\n",
    )
    .unwrap();

    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("batch", &output);
    let text = stdout_of(&output);
    let mut lines: Vec<_> = text.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, ["a done: one", "b needs-decision: choose"]);
    assert_eq!(cursor(&first), "10\n");
    assert_eq!(cursor(&second), "23\n");
    assert_eq!(cursor(&quiet), "");
    assert_eq!(cursor(&closed), "");

    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("next line", &output);
    assert_eq!(stdout_of(&output), "a blocked: two\n");
    assert_eq!(
        run_wait(&workspace, &["--timeout", "0"]).status.code(),
        Some(22)
    );
}

#[test]
fn newest_wait_takes_over_without_consuming_any_line_in_the_old_wait() {
    let workspace = temp_workspace("niles-workspace-takeover");
    let dir = worker_with_status(&workspace, "a", b"working: busy\n");
    let old = spawn_wait(&workspace, &["--interval", "1h", "--timeout", "1h"]);
    wait_for_file(&dir.join("status.cursor"));
    let new = spawn_wait(&workspace, &["--interval", "10ms", "--timeout", "5s"]);
    let output = old.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(11));
    assert!(stderr_of(&output).contains("superseded"));
    assert!(stdout_of(&output).is_empty());
    assert_eq!(cursor(&dir), "");
    append_status(&dir.join("status.log"), b"done: once\n");
    let output = new.wait_with_output().unwrap();
    assert_command_success("newest wait", &output);
    assert_eq!(stdout_of(&output), "a done: once\n");
    assert_eq!(cursor(&dir), "25\n");
}

#[test]
fn killed_wait_loses_nothing_and_cannot_hold_the_slot() {
    let workspace = temp_workspace("niles-workspace-killed");
    let dir = worker_with_status(&workspace, "a", b"done: safe\n");
    let held_cursor = fs::File::create(dir.join("status.cursor")).unwrap();
    held_cursor.lock().unwrap();
    let mut old = spawn_wait(&workspace, &["--interval", "1h", "--timeout", "1h"]);
    wait_for_file(&state_dir(&workspace).join("worker/wait.lock"));
    old.kill().unwrap();
    old.wait().unwrap();
    assert_eq!(cursor(&dir), "");
    drop(held_cursor);
    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("after killed wait", &output);
    assert_eq!(stdout_of(&output), "a done: safe\n");
}

#[test]
fn stopped_stale_wait_cannot_swallow_the_new_waits_report() {
    let workspace = temp_workspace("niles-workspace-stale");
    let dir = worker_with_status(&workspace, "a", b"working: busy\n");
    let old = spawn_wait(&workspace, &["--interval", "1h", "--timeout", "1h"]);
    wait_for_file(&dir.join("status.cursor"));
    // Stop it while it cannot be inside a delivery critical section.
    let gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state_dir(&workspace).join("worker/wait.lock"))
        .unwrap();
    gate.lock().unwrap();
    unsafe {
        assert_eq!(libc::kill(old.id() as i32, libc::SIGSTOP), 0);
    }
    drop(gate);
    append_status(&dir.join("status.log"), b"done: newest\n");
    let output = run_wait(&workspace, &["--timeout", "0"]);
    let presence = fs::File::open(state_dir(&workspace).join("worker/wait.running")).unwrap();
    let presence_released = presence.try_lock().is_ok();
    unsafe {
        assert_eq!(libc::kill(old.id() as i32, libc::SIGCONT), 0);
    }
    assert!(
        presence_released,
        "a stale wait must not suppress report nudges after takeover"
    );
    assert_command_success("stale takeover", &output);
    assert_eq!(stdout_of(&output), "a done: newest\n");
    let output = old.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(11));
    assert!(stdout_of(&output).is_empty());
}

#[test]
fn failed_output_does_not_consume_a_report() {
    let workspace = temp_workspace("niles-workspace-pipe");
    let dir = worker_with_status(&workspace, "a", b"working: busy\n");
    let mut waiter = spawn_wait(&workspace, &["--interval", "10ms", "--timeout", "5s"]);
    wait_for_file(&dir.join("status.cursor"));
    drop(waiter.stdout.take());
    append_status(&dir.join("status.log"), b"done: keep\n");
    let output = waiter.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert_eq!(cursor(&dir), "");
    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("retry after broken pipe", &output);
    assert_eq!(stdout_of(&output), "a done: keep\n");
}

#[test]
fn busy_send_cursor_is_left_alone_and_other_workers_still_print() {
    let workspace = temp_workspace("niles-workspace-send-lock");
    let a = worker_with_status(&workspace, "a", b"done: taken by send\n");
    worker_with_status(&workspace, "b", b"done: pending\n");
    let path = a.join("status.cursor");
    let mut cursor_file = fs::File::create(&path).unwrap();
    cursor_file.lock().unwrap();
    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("other worker during send", &output);
    assert_eq!(stdout_of(&output), "b done: pending\n");
    assert_eq!(cursor(&a), "");
    use std::io::Write;
    writeln!(cursor_file, "19").unwrap();
    drop(cursor_file);
    let output = run_wait(&workspace, &["--timeout", "0"]);
    assert_eq!(output.status.code(), Some(22));
    assert!(stdout_of(&output).is_empty());
}

#[test]
fn workspace_lock_symlinks_are_refused_and_old_cli_forms_are_rejected() {
    for name in ["wait.lock", "wait.running"] {
        let workspace = temp_workspace("niles-workspace-symlink");
        fs::create_dir_all(state_dir(&workspace).join("worker")).unwrap();
        let outside = workspace.join("outside");
        fs::write(&outside, "untouched").unwrap();
        std::os::unix::fs::symlink(&outside, state_dir(&workspace).join("worker").join(name))
            .unwrap();
        let output = run_wait(&workspace, &["--timeout", "0"]);
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
    }
    let workspace = temp_workspace("niles-workspace-cli");
    for args in [&["a"][..], &["--all"][..]] {
        assert_eq!(run_wait(&workspace, args).status.code(), Some(2));
    }
}
