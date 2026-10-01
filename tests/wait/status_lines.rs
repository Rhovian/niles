use super::support::*;

#[test]
fn does_not_redeliver_consumed_wake_and_delivers_next() {
    let workspace = temp_workspace("niles-wait-cursor");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: first\n");
    let args = ["auth-fix", "--interval", "50ms", "--timeout", "0"];

    let first = run_wait(&workspace, &args);
    assert_command_success("first wait", &first);
    assert_eq!(stdout_of(&first), "done: first\n");
    assert_eq!(cursor(&worker_dir), "12\n");

    let second = run_wait(&workspace, &args);
    assert!(!second.status.success());
    assert_eq!(second.status.code(), Some(22));
    assert!(stdout_of(&second).is_empty());
    assert!(stderr_of(&second).contains("timeout"));

    append_status(&worker_dir.join("status.log"), b"done: second\n");

    let third = run_wait(&workspace, &args);
    assert_command_success("third wait", &third);
    assert_eq!(stdout_of(&third), "done: second\n");
    assert_eq!(cursor(&worker_dir), "25\n");
}

#[test]
fn skips_non_actionable_lines_without_persisting_past_an_undelivered_wake() {
    let workspace = temp_workspace("niles-wait-working");
    let worker_dir = worker_with_status(
        &workspace,
        "auth-fix",
        b"working: one\nworking: two\nneeds-decision: pick a lane\nworking: three\n",
    );

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );

    assert_command_success("wait past working lines", &output);
    assert_eq!(stdout_of(&output), "needs-decision: pick a lane\n");
    // Stops just past the decision line, not at EOF: the trailing `working:` stays unscanned.
    assert_eq!(cursor(&worker_dir), "54\n");
}

#[test]
fn leaves_an_unterminated_trailing_line_for_the_next_poll() {
    let workspace = temp_workspace("niles-wait-partial");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: comp");

    let partial = run_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );
    assert_eq!(partial.status.code(), Some(22));
    // The cursor file exists because it doubles as the lock, but holds no advanced position.
    let recorded = cursor(&worker_dir);
    assert!(
        recorded.trim().is_empty() || recorded.trim() == "0",
        "a half-written line must not advance the cursor, found {recorded:?}"
    );

    append_status(&worker_dir.join("status.log"), b"lete\n");

    let whole = run_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );
    assert_command_success("wait after line completed", &whole);
    assert_eq!(stdout_of(&whole), "done: complete\n");
}

#[test]
fn escapes_control_characters_instead_of_emitting_them() {
    let workspace = temp_workspace("niles-wait-control");
    worker_with_status(
        &workspace,
        "auth-fix",
        b"done: shipped\x1b[2J\x1b[H\x07 and cleared your screen\n",
    );

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );

    assert_command_success("wait with control characters", &output);
    let stdout = stdout_of(&output);
    assert!(
        !stdout.contains('\x1b'),
        "raw escape reached stdout: {stdout:?}"
    );
    assert!(
        !stdout.contains('\x07'),
        "raw bell reached stdout: {stdout:?}"
    );
    assert!(stdout.contains("shipped"));
    assert!(stdout.contains("and cleared your screen"));
}

#[test]
fn non_utf8_bytes_do_not_desynchronise_the_cursor() {
    let workspace = temp_workspace("niles-wait-binary");
    let mut status = b"working: \xff\xfe garbage\n".to_vec();
    status.extend_from_slice(b"done: recovered\n");
    let worker_dir = worker_with_status(&workspace, "auth-fix", &status);
    let total = fs::metadata(worker_dir.join("status.log")).unwrap().len();

    let output = run_wait(
        &workspace,
        &["auth-fix", "--interval", "50ms", "--timeout", "0"],
    );

    assert_command_success("wait past non-utf8", &output);
    assert_eq!(stdout_of(&output), "done: recovered\n");
    // Offsets come from raw bytes, so lossy decoding of the garbage line cannot shift them.
    assert_eq!(cursor(&worker_dir), format!("{total}\n"));
}

#[test]
fn a_truncated_log_rescans_from_the_start() {
    let workspace = temp_workspace("niles-wait-truncated");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: first\n");
    let args = ["auth-fix", "--interval", "50ms", "--timeout", "0"];

    assert_command_success("first wait", &run_wait(&workspace, &args));
    assert_eq!(cursor(&worker_dir), "12\n");

    // Rewrite the log shorter than the recorded offset of 12.
    fs::write(worker_dir.join("status.log"), b"done: new\n").unwrap();

    let output = run_wait(&workspace, &args);
    assert_command_success("wait after truncation", &output);
    assert_eq!(stdout_of(&output), "done: new\n");
    assert_eq!(cursor(&worker_dir), "10\n");
}

#[test]
fn concurrent_waits_deliver_the_line_to_exactly_one() {
    let workspace = temp_workspace("niles-wait-concurrent");
    worker_with_status(&workspace, "auth-fix", b"working: still running\n");
    let args = ["auth-fix", "--interval", "50ms", "--timeout", "5s"];

    let first = spawn_wait(&workspace, &args);
    let second = spawn_wait(&workspace, &args);
    settle();

    append_status(
        &workspace.join(".niles/worker/auth-fix/status.log"),
        b"done: only once\n",
    );

    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    // Neither wait is rejected for the other's existence; exactly one is handed the line.
    let winners = [&first, &second]
        .into_iter()
        .filter(|output| stdout_of(output).contains("done: only once"))
        .count();
    assert_eq!(
        winners,
        1,
        "first: {:?}/{:?}\nsecond: {:?}/{:?}",
        stdout_of(&first),
        stderr_of(&first),
        stdout_of(&second),
        stderr_of(&second)
    );
    let losers = [&first, &second]
        .into_iter()
        .filter(|output| output.status.code() == Some(22))
        .count();
    assert_eq!(losers, 1, "the wait that lost the race should time out");
}
